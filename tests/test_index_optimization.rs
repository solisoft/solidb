//! Index Optimization Tests
//!
//! Verifies that the Query Planner correctly utilizes indexes for optimization.

use serde_json::json;
use solidb::parse;
use solidb::sdbql::QueryExecutor;
use solidb::storage::{IndexType, StorageEngine};
use tempfile::TempDir;

fn create_seeded_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");

    engine.create_database("testdb".to_string()).unwrap();
    let db = engine.get_database("testdb").unwrap();

    db.create_collection("users".to_string(), None).unwrap();
    let users = db.get_collection("users").unwrap();

    // Insert 100 users
    for i in 0..100 {
        users
            .insert(json!({
                "name": format!("User{}", i),
                "age": i,
                "city": if i % 2 == 0 { "Paris" } else { "London" }
            }))
            .unwrap();
    }

    (engine, tmp_dir)
}

#[test]
fn test_index_usage_equality() {
    let (engine, _tmp) = create_seeded_engine();
    let executor = QueryExecutor::with_database(&engine, "testdb".to_string());

    let query_str = "FOR u IN users FILTER u.age == 50 RETURN u";
    let query = parse(query_str).unwrap();

    // 1. Before Index: Should be Full Scan
    let explain = executor.explain(&query).unwrap();
    let access = &explain.collections[0];
    assert_eq!(access.name, "users");
    assert_eq!(access.access_type, "full_scan");
    assert!(access.index_used.is_none());

    // 2. Create Index on 'age'
    let db = engine.get_database("testdb").unwrap();
    let users = db.get_collection("users").unwrap();
    users
        .create_index(
            "age_idx".to_string(),
            vec!["age".to_string()],
            IndexType::Hash,
            false,
        )
        .unwrap();

    // 3. After Index: Should be Index Lookup
    let explain = executor.explain(&query).unwrap();
    let access = &explain.collections[0];
    assert_eq!(
        access.access_type, "index_lookup",
        "Query should use index after creation"
    );
    assert!(access.index_used.is_some());
    assert!(access.index_used.as_ref().unwrap().contains("age"));
}

#[test]
fn test_index_usage_composite() {
    let (engine, _tmp) = create_seeded_engine();
    let executor = QueryExecutor::with_database(&engine, "testdb".to_string());

    // Query filtering on two fields
    let query_str = "FOR u IN users FILTER u.city == 'Paris' AND u.age == 10 RETURN u";
    let query = parse(query_str).unwrap();

    // 1. Before Index
    let explain = executor.explain(&query).unwrap();
    assert_eq!(explain.collections[0].access_type, "full_scan");

    // 2. Create Composite Index on [city, age]
    let db = engine.get_database("testdb").unwrap();
    let users = db.get_collection("users").unwrap();
    users
        .create_index(
            "city_age_idx".to_string(),
            vec!["city".to_string(), "age".to_string()],
            IndexType::Hash,
            false,
        )
        .unwrap();

    // 3. After Index
    let explain = executor.explain(&query).unwrap();
    let access = &explain.collections[0];

    // Note: SoliDB optimizer needs to be smart enough to pick composite index
    // If it doesn't support composite optimization yet, this test will fail (or show full_scan).
    // Assuming standard behavior for verifying optimization.
    assert_eq!(
        access.access_type, "index_lookup",
        "Should use composite index"
    );
}

/// Seed for the null-in-composite tests: `archived_at` is a date on some
/// rows, an explicit null on others, and missing on the rest.
fn create_archive_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_database("arcdb".to_string()).unwrap();
    let db = engine.get_database("arcdb").unwrap();
    db.create_collection("parcels".to_string(), None).unwrap();
    let parcels = db.get_collection("parcels").unwrap();
    for i in 0..120 {
        let company = if i % 2 == 0 { "acme" } else { "globex" };
        let doc = match i % 3 {
            0 => json!({ "seq": i, "company": company, "archived_at": "2026-01-01" }),
            1 => json!({ "seq": i, "company": company, "archived_at": null }),
            _ => json!({ "seq": i, "company": company }),
        };
        parcels.insert(doc).unwrap();
    }
    (engine, tmp_dir)
}

fn sorted_seqs(rows: Vec<serde_json::Value>) -> Vec<i64> {
    let mut seqs: Vec<i64> = rows.iter().map(|v| v.as_i64().unwrap()).collect();
    seqs.sort();
    seqs
}

/// `company == X AND archived_at == null` is served by the composite index,
/// and returns exactly what the full scan returns: explicit nulls AND rows
/// where the field is missing.
#[test]
fn test_composite_index_serves_equality_on_null() {
    let (engine, _tmp) = create_archive_engine();
    let executor = QueryExecutor::with_database(&engine, "arcdb".to_string());
    let query =
        parse("FOR p IN parcels FILTER p.company == 'acme' AND p.archived_at == null RETURN p.seq")
            .unwrap();

    let scanned = sorted_seqs(executor.execute(&query).unwrap());
    assert_eq!(scanned.len(), 40);
    assert_eq!(
        executor.explain(&query).unwrap().collections[0].access_type,
        "full_scan"
    );

    let db = engine.get_database("arcdb").unwrap();
    db.get_collection("parcels")
        .unwrap()
        .create_index(
            "company_archived_idx".to_string(),
            vec!["company".to_string(), "archived_at".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();

    let explain = executor.explain(&query).unwrap();
    assert_eq!(explain.collections[0].access_type, "index_lookup");
    assert_eq!(
        explain.collections[0].index_used.as_deref(),
        Some("company_archived_idx")
    );
    assert_eq!(sorted_seqs(executor.execute(&query).unwrap()), scanned);
}

/// Every index field null: those rows are never indexed, so the lookup must
/// not be used — the scan still finds them.
#[test]
fn test_composite_index_not_used_when_every_field_is_null() {
    let (engine, _tmp) = create_archive_engine();
    let executor = QueryExecutor::with_database(&engine, "arcdb".to_string());
    let db = engine.get_database("arcdb").unwrap();
    let parcels = db.get_collection("parcels").unwrap();
    parcels.insert(json!({ "seq": 500 })).unwrap();
    parcels
        .create_index(
            "company_archived_idx".to_string(),
            vec!["company".to_string(), "archived_at".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();

    let query =
        parse("FOR p IN parcels FILTER p.company == null AND p.archived_at == null RETURN p.seq")
            .unwrap();
    assert_eq!(
        executor.explain(&query).unwrap().collections[0].access_type,
        "full_scan"
    );
    assert_eq!(sorted_seqs(executor.execute(&query).unwrap()), vec![500]);
}

#[test]
fn test_index_usage_unique_constraint() {
    let (engine, _tmp) = create_seeded_engine();

    let db = engine.get_database("testdb").unwrap();
    let users = db.get_collection("users").unwrap();

    // Create Unique Index on 'name'
    users
        .create_index(
            "name_idx".to_string(),
            vec!["name".to_string()],
            IndexType::Hash,
            true,
        )
        .unwrap();

    let executor = QueryExecutor::with_database(&engine, "testdb".to_string());
    let query_str = "FOR u IN users FILTER u.name == 'User50' RETURN u";
    let query = parse(query_str).unwrap();

    let explain = executor.explain(&query).unwrap();
    assert_eq!(explain.collections[0].access_type, "index_lookup");
}

// ============================================================================
// LIMIT pushdown into index lookups
// ============================================================================

fn create_pushdown_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_database("pushdb".to_string()).unwrap();
    let db = engine.get_database("pushdb").unwrap();
    db.create_collection("events".to_string(), None).unwrap();
    let events = db.get_collection("events").unwrap();
    events
        .create_index(
            "cat_idx".to_string(),
            vec!["cat".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();
    // 300 docs: 100 per category, seq unique
    for i in 0..300 {
        events
            .insert(json!({ "cat": i % 3, "flag": i % 2, "seq": i }))
            .unwrap();
    }
    (engine, tmp_dir)
}

#[test]
fn test_limit_pushdown_eq_returns_exactly_n() {
    let (engine, _tmp) = create_pushdown_engine();
    let executor = QueryExecutor::with_database(&engine, "pushdb".to_string());

    let r = executor
        .execute(&parse("FOR e IN events FILTER e.cat == 1 LIMIT 10 RETURN e.seq").unwrap())
        .unwrap();
    assert_eq!(r.len(), 10);
    assert!(r.iter().all(|v| v.as_i64().unwrap() % 3 == 1));
}

#[test]
fn test_limit_pushdown_with_offset() {
    let (engine, _tmp) = create_pushdown_engine();
    let executor = QueryExecutor::with_database(&engine, "pushdb".to_string());

    let all = executor
        .execute(&parse("FOR e IN events FILTER e.cat == 1 RETURN e.seq").unwrap())
        .unwrap();
    assert_eq!(all.len(), 100);
    let limited = executor
        .execute(&parse("FOR e IN events FILTER e.cat == 1 LIMIT 5, 10 RETURN e.seq").unwrap())
        .unwrap();
    assert_eq!(limited.as_slice(), &all[5..15]);
}

/// `cat == 1 AND flag == 0`: only `cat` is indexed, so the flag conjunct is a
/// residual filter — the LIMIT must NOT be pushed into the index lookup or
/// the result would under-fill. 50 docs match; LIMIT 30 must return 30.
#[test]
fn test_limit_not_pushed_for_partially_covered_filter() {
    let (engine, _tmp) = create_pushdown_engine();
    let executor = QueryExecutor::with_database(&engine, "pushdb".to_string());

    let r = executor
        .execute(
            &parse("FOR e IN events FILTER e.cat == 1 AND e.flag == 0 LIMIT 30 RETURN e.seq")
                .unwrap(),
        )
        .unwrap();
    assert_eq!(r.len(), 30);
    assert!(r.iter().all(|v| {
        let s = v.as_i64().unwrap();
        s % 3 == 1 && s % 2 == 0
    }));
}

#[test]
fn test_limit_pushdown_range() {
    let (engine, _tmp) = create_pushdown_engine();
    let executor = QueryExecutor::with_database(&engine, "pushdb".to_string());

    let all = executor
        .execute(&parse("FOR e IN events FILTER e.cat >= 1 RETURN e.seq").unwrap())
        .unwrap();
    assert_eq!(all.len(), 200);
    let limited = executor
        .execute(&parse("FOR e IN events FILTER e.cat >= 1 LIMIT 10 RETURN e.seq").unwrap())
        .unwrap();
    assert_eq!(limited.as_slice(), &all[..10]);
}

/// `FILTER a FILTER b` is `FILTER a AND b`: when only the *second* FILTER has
/// an indexable conjunct, the planner must still find it. The first FILTER
/// here is the shape that used to stop it — `!=` terms and an OR containing
/// `== null`, none of which an index can serve.
#[test]
fn test_index_found_in_second_filter() {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_database("evdb".to_string()).unwrap();
    let db = engine.get_database("evdb").unwrap();
    db.create_collection("events".to_string(), None).unwrap();
    let events = db.get_collection("events").unwrap();
    for i in 0..200 {
        let date2 = match i % 5 {
            0 => json!(null),
            1 => json!(i),
            _ => json!(format!("2026-{:02}-{:02}", 1 + i % 12, 1 + i % 28)),
        };
        let parent = match i % 3 {
            0 => json!(null),
            1 => json!(""),
            _ => json!("p1"),
        };
        events
            .insert(json!({
                "seq": i, "date2": date2, "parent_key": parent,
                "atelier": i % 7 == 0, "draft": i % 11 == 0,
            }))
            .unwrap();
    }

    let q = |opts: &str| {
        format!(
            "FOR e IN events {opts} \
             FILTER e.atelier != true AND e.draft != true \
                AND (e.parent_key == null OR e.parent_key == \"\") \
             FILTER IS_STRING(e.date2) AND e.date2 >= @today \
             SORT e.seq RETURN e.seq"
        )
    };
    let binds: std::collections::HashMap<String, serde_json::Value> =
        [("today".to_string(), json!("2026-06-15"))].into();
    let executor =
        QueryExecutor::with_database_and_bind_vars(&engine, "evdb".to_string(), binds.clone());

    let scanned = executor.execute(&parse(&q("")).unwrap()).unwrap();
    assert!(!scanned.is_empty());

    events
        .create_index(
            "date2_idx".to_string(),
            vec!["date2".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();

    let explain = executor.explain(&parse(&q("")).unwrap()).unwrap();
    assert_eq!(explain.collections[0].access_type, "index_lookup");
    assert_eq!(
        explain.collections[0].index_used.as_deref(),
        Some("date2_idx")
    );

    // Same rows through the index, and the execution path really takes it:
    // a forced hint is an error when no FILTER can be served by that index.
    assert_eq!(executor.execute(&parse(&q("")).unwrap()).unwrap(), scanned);
    let forced = q(r#"OPTIONS { indexHint: "date2_idx", forceIndexHint: true }"#);
    assert_eq!(executor.execute(&parse(&forced).unwrap()).unwrap(), scanned);
}

/// `FILTER v.f == x COLLECT WITH COUNT` is counted from the index without
/// reading documents. Its answers must be the ordinary path's: a second
/// `FILTER true` keeps a query off the shortcut, so each count is checked
/// against the same query run the ordinary way.
#[test]
fn test_index_only_count_matches_the_ordinary_path() {
    let (engine, _tmp) = create_seeded_engine();
    let executor = QueryExecutor::with_database(&engine, "testdb".to_string());
    let db = engine.get_database("testdb").unwrap();
    let users = db.get_collection("users").unwrap();
    for (name, field, ty) in [
        ("city_idx", "city", IndexType::Persistent),
        ("age_idx", "age", IndexType::Hash),
    ] {
        users
            .create_index(name.to_string(), vec![field.to_string()], ty, false)
            .unwrap();
    }

    let run = |q: &str| executor.execute(&parse(q).unwrap()).unwrap();
    let both = |filter: &str, ret: &str| {
        let fast = run(&format!(
            "FOR u IN users FILTER {filter} COLLECT WITH COUNT INTO n RETURN {ret}"
        ));
        let slow = run(&format!(
            "FOR u IN users FILTER {filter} FILTER true COLLECT WITH COUNT INTO n RETURN {ret}"
        ));
        assert_eq!(fast, slow, "{filter} / {ret}");
        fast
    };

    assert_eq!(both("u.city == 'Paris'", "n"), vec![json!(50)]);
    assert_eq!(
        both("'London' == u.city", "{total: n}"),
        vec![json!({"total": 50})]
    );
    assert_eq!(both("u.city == 'Rome'", "n"), vec![json!(0)]);
    assert_eq!(both("u.age == 42", "n"), vec![json!(1)]);
    assert_eq!(both("u.age == 42.0", "n"), vec![json!(1)]);
    // No index on `name`: the ordinary path answers.
    assert_eq!(both("u.name == 'User7'", "n"), vec![json!(1)]);
    // `null` is not in the index; the ordinary path scans.
    assert_eq!(both("u.city == null", "n"), vec![json!(0)]);

    // Writes move the index entries with the documents.
    run("FOR u IN users FILTER u.age < 10 UPDATE u WITH {city: 'Rome'} IN users");
    run("FOR u IN users FILTER u.age >= 90 REMOVE u IN users");
    assert_eq!(both("u.city == 'Rome'", "n"), vec![json!(10)]);
    assert_eq!(both("u.city == 'Paris'", "n"), vec![json!(40)]);
    assert_eq!(both("u.city == 'London'", "n"), vec![json!(40)]);

    // A bind variable is a value like any other.
    let query =
        parse("FOR u IN users FILTER u.city == @c COLLECT WITH COUNT INTO n RETURN n").unwrap();
    let mut vars = std::collections::HashMap::new();
    vars.insert("c".to_string(), json!("Rome"));
    let with_vars = QueryExecutor::with_database_and_bind_vars(&engine, "testdb".to_string(), vars);
    assert_eq!(with_vars.execute(&query).unwrap(), vec![json!(10)]);
}

/// `FOR v IN c FILTER v.f == x … RETURN <fields>` copies the stored bytes of
/// the documents the index names (the driver's raw-rows path). Its bytes must
/// be what encoding the ordinary path's result gives.
#[test]
fn test_index_raw_rows_match_the_ordinary_path() {
    let (engine, _tmp) = create_seeded_engine();
    let db = engine.get_database("testdb").unwrap();
    let users = db.get_collection("users").unwrap();
    for (name, field) in [("city_idx", "city"), ("age_idx", "age")] {
        users
            .create_index(
                name.to_string(),
                vec![field.to_string()],
                IndexType::Persistent,
                false,
            )
            .unwrap();
    }
    let executor = QueryExecutor::with_database(&engine, "testdb".to_string());
    let same = |q: &str| {
        let query = parse(q).unwrap();
        let raw = executor.execute_raw_msgpack(&query).unwrap();
        let ordinary =
            rmp_serde::to_vec_named(&serde_json::Value::Array(executor.execute(&query).unwrap()))
                .unwrap();
        if let Some(raw) = &raw {
            assert_eq!(raw, &ordinary, "{q}");
        }
        raw.is_some()
    };

    for q in [
        "FOR u IN users FILTER u.city == 'Paris' RETURN u",
        "FOR u IN users FILTER u.city == 'Paris' RETURN {name: u.name, age: u.age}",
        "FOR u IN users FILTER 'London' == u.city RETURN u.name",
        "FOR u IN users FILTER u.city == 'Paris' LIMIT 5 RETURN u._key",
        "FOR u IN users FILTER u.city == 'Paris' LIMIT 45, 10 RETURN u.age",
        "FOR u IN users FILTER u.city == 'Paris' LIMIT 60, 10 RETURN u.age",
        "FOR u IN users FILTER u.city == 'Paris' LIMIT 0 RETURN u",
        "FOR u IN users FILTER u.city == 'Rome' RETURN u",
        "FOR u IN users FILTER u.age == 42 RETURN u",
        "FOR u IN users FILTER u.age == 42.0 RETURN u",
    ] {
        assert!(same(q), "the index should serve {q}");
    }
    // No index on `name`, a null, a range, `_key`: left to the ordinary path.
    for q in [
        "FOR u IN users FILTER u.name == 'User7' RETURN u",
        "FOR u IN users FILTER u.city == null RETURN u",
        "FOR u IN users FILTER u.age > 90 RETURN u",
        "FOR u IN users FILTER u._key == 'x' RETURN u",
    ] {
        assert!(!same(q), "{q} should take the ordinary path");
    }
}

/// `RETURN LENGTH(FOR doc IN c FILTER doc.f == x RETURN 1)` — how the Soli
/// ORM counts a `where` — is counted in the index too, at top level and as a
/// correlated subquery. `FILTER true` keeps the reference off the shortcut.
#[test]
fn test_index_only_length_of_subquery_matches_the_ordinary_path() {
    let (engine, _tmp) = create_seeded_engine();
    let db = engine.get_database("testdb").unwrap();
    let users = db.get_collection("users").unwrap();
    users
        .create_index(
            "city_idx".to_string(),
            vec!["city".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();
    let executor = QueryExecutor::with_database(&engine, "testdb".to_string());
    let run = |q: &str| executor.execute(&parse(q).unwrap()).unwrap();
    for (fast, slow, expected) in [
        (
            "RETURN LENGTH(FOR doc IN users FILTER doc.city == 'Paris' RETURN 1)",
            "RETURN LENGTH(FOR doc IN users FILTER doc.city == 'Paris' FILTER true RETURN 1)",
            json!([50]),
        ),
        (
            "RETURN COUNT(FOR doc IN users FILTER doc.city == 'Rome' RETURN doc)",
            "RETURN COUNT(FOR doc IN users FILTER doc.city == 'Rome' FILTER true RETURN doc)",
            json!([0]),
        ),
        (
            "FOR c IN ['Paris', 'London', 'Rome'] \
             RETURN {c, n: LENGTH(FOR u IN users FILTER u.city == c RETURN u.name)}",
            "FOR c IN ['Paris', 'London', 'Rome'] \
             RETURN {c, n: LENGTH(FOR u IN users FILTER u.city == c FILTER true RETURN u.name)}",
            json!([
                {"c": "Paris", "n": 50},
                {"c": "London", "n": 50},
                {"c": "Rome", "n": 0}
            ]),
        ),
    ] {
        let got = run(fast);
        assert_eq!(serde_json::Value::Array(got.clone()), expected, "{fast}");
        assert_eq!(got, run(slow), "{fast}");
    }
}

// ==================== FILTER … SORT … LIMIT served by an index ====================

/// 300 parcels over two companies, each archived / null / missing. Only 40
/// `created` values, so rows tie on the sort field; one row has none (it
/// sorts first).
fn create_sorted_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_database("sortdb".to_string()).unwrap();
    let db = engine.get_database("sortdb").unwrap();
    db.create_collection("parcels".to_string(), None).unwrap();
    let parcels = db.get_collection("parcels").unwrap();
    for i in 0..300 {
        let company = if i % 2 == 0 { "acme" } else { "globex" };
        let status = if i % 5 == 0 { "open" } else { "done" };
        // Not in insertion order, so the index order is the one being tested.
        let created = format!("2026-01-01T{:05}", (i * 7919) % 40);
        let mut doc = json!({ "seq": i, "company": company, "status": status, "created": created });
        match i % 3 {
            0 => doc["archived_at"] = json!("2026-02-01"),
            1 => doc["archived_at"] = json!(null),
            _ => {}
        }
        if i == 4 {
            doc.as_object_mut().unwrap().remove("created");
        }
        parcels.insert(doc).unwrap();
    }
    (engine, tmp_dir)
}

fn add_index(engine: &StorageEngine, name: &str, fields: &[&str]) {
    engine
        .get_database("sortdb")
        .unwrap()
        .get_collection("parcels")
        .unwrap()
        .create_index(
            name.to_string(),
            fields.iter().map(|f| f.to_string()).collect(),
            IndexType::Persistent,
            false,
        )
        .unwrap();
}

/// The rows and the number of full collection scans the query took.
fn run_sorted(engine: &StorageEngine, sdbql: &str) -> (Vec<serde_json::Value>, u64) {
    let executor = QueryExecutor::with_database(engine, "sortdb".to_string());
    let rows = executor.execute(&parse(sdbql).unwrap()).unwrap();
    (rows, executor.collection_scans())
}

const SORTED_QUERIES: &[&str] = &[
    "FOR p IN parcels FILTER p.company == 'acme' AND p.archived_at == null \
     SORT p.created DESC LIMIT 10 RETURN p.seq",
    "FOR p IN parcels FILTER p.company == 'acme' AND p.archived_at == null \
     SORT p.created ASC LIMIT 3, 5 RETURN p.seq",
    // Two FILTERs are one conjunction.
    "FOR p IN parcels FILTER p.company == 'globex' FILTER p.archived_at == null \
     SORT p.created DESC LIMIT 7 RETURN p.seq",
    // A condition outside the index is re-checked on each row.
    "FOR p IN parcels FILTER p.company == 'acme' AND p.archived_at == null AND p.status == 'open' \
     SORT p.created DESC LIMIT 4 RETURN p.seq",
    // LETs after the FILTER, evaluated on the rows kept.
    "FOR p IN parcels FILTER p.company == 'acme' AND p.archived_at == null LET s = p.seq * 2 \
     SORT p.created DESC LIMIT 5 RETURN s",
    // More rows asked for than exist: every match, in order.
    "FOR p IN parcels FILTER p.company == 'acme' AND p.archived_at == null \
     SORT p.created ASC LIMIT 1000 RETURN p.seq",
];

/// With `[company, archived_at, created]` the queries read the index range
/// (no collection scan) and return exactly what the scan and sort returned.
#[test]
fn test_filter_sort_limit_served_by_composite_index() {
    let (engine, _tmp) = create_sorted_engine();
    let expected: Vec<_> = SORTED_QUERIES
        .iter()
        .map(|q| run_sorted(&engine, q).0)
        .collect();
    assert_eq!(expected[5].len(), 100);
    // The row without `created` sorts first, as SORT puts null first.
    assert_eq!(expected[5][0], json!(4));
    // Ties inside the page, so their order is part of what is compared.
    let created = |q: &str| run_sorted(&engine, &q.replace("RETURN p.seq", "RETURN p.created")).0;
    let first_page = created(SORTED_QUERIES[0]);
    assert!(first_page.windows(2).any(|w| w[0] == w[1]));

    add_index(
        &engine,
        "company_archived_created",
        &["company", "archived_at", "created"],
    );
    for (query, expected) in SORTED_QUERIES.iter().zip(&expected) {
        let (rows, scans) = run_sorted(&engine, query);
        assert_eq!(&rows, expected, "{query}");
        assert_eq!(scans, 0, "{query}");
    }
}

/// An index whose prefix covers only some conditions still serves the sort;
/// the others are checked per row.
#[test]
fn test_filter_sort_limit_uses_a_shorter_prefix() {
    let (engine, _tmp) = create_sorted_engine();
    let query = SORTED_QUERIES[0];
    let (expected, _) = run_sorted(&engine, query);

    add_index(&engine, "company_created", &["company", "created"]);
    let (rows, scans) = run_sorted(&engine, query);
    assert_eq!(rows, expected);
    assert_eq!(scans, 0);
}

/// No index ends with the sort field after the condition fields, or every
/// prefix value is null: the ordinary scan and sort answer.
#[test]
fn test_filter_sort_limit_falls_back_without_a_fitting_index() {
    let (engine, _tmp) = create_sorted_engine();
    add_index(&engine, "company_archived", &["company", "archived_at"]);
    add_index(&engine, "archived_created", &["archived_at", "created"]);

    let query = "FOR p IN parcels FILTER p.company == 'acme' SORT p.seq DESC LIMIT 3 RETURN p.seq";
    let (rows, scans) = run_sorted(&engine, query);
    assert_eq!(rows, vec![json!(298), json!(296), json!(294)]);
    assert!(scans > 0);

    // The sort field followed by another field: ties would come out in that
    // field's order, not in the scan's.
    add_index(&engine, "company_seq_status", &["company", "seq", "status"]);
    let (rows, scans) = run_sorted(&engine, query);
    assert_eq!(rows, vec![json!(298), json!(296), json!(294)]);
    assert!(scans > 0);

    // `archived_at == null` alone: those rows are in `archived_created` only
    // when `created` is set, so the index cannot be trusted with it.
    let query =
        "FOR p IN parcels FILTER p.archived_at == null SORT p.created ASC LIMIT 2 RETURN p.seq";
    let (rows, scans) = run_sorted(&engine, query);
    assert_eq!(rows[0], json!(4));
    assert!(scans > 0);
}

/// Arrays and objects are ordered by the codec as JSON text, not as SORT
/// orders them: the read gives up and the scan answers.
#[test]
fn test_filter_sort_limit_falls_back_on_array_sort_values() {
    let (engine, _tmp) = create_sorted_engine();
    let parcels = engine
        .get_database("sortdb")
        .unwrap()
        .get_collection("parcels")
        .unwrap();
    for (seq, created) in [(900, json!([10])), (901, json!([9, 1])), (902, json!([2]))] {
        parcels
            .insert(json!({ "seq": seq, "company": "initech", "created": created }))
            .unwrap();
    }
    let query =
        "FOR p IN parcels FILTER p.company == 'initech' SORT p.created ASC LIMIT 3 RETURN p.seq";
    let (expected, _) = run_sorted(&engine, query);
    assert_eq!(expected, vec![json!(902), json!(901), json!(900)]);

    add_index(&engine, "company_created", &["company", "created"]);
    let (rows, scans) = run_sorted(&engine, query);
    assert_eq!(rows, expected);
    assert!(scans > 0);
}

// ==================== Counts read from a composite index ====================

/// 300 parcels over two companies; `archived_at` set, null or missing;
/// three statuses.
fn create_count_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_database("countdb".to_string()).unwrap();
    let db = engine.get_database("countdb").unwrap();
    db.create_collection("parcels".to_string(), None).unwrap();
    let parcels = db.get_collection("parcels").unwrap();
    for i in 0..300 {
        let company = if i % 2 == 0 { "acme" } else { "globex" };
        let status = ["open", "transit", "done"][i % 3];
        let mut doc = json!({ "seq": i, "company": company, "status": status, "created": i });
        match i % 5 {
            0 => doc["archived_at"] = json!("2026-02-01"),
            1 | 2 => doc["archived_at"] = json!(null),
            _ => {}
        }
        parcels.insert(doc).unwrap();
    }
    (engine, tmp_dir)
}

fn add_count_index(engine: &StorageEngine, name: &str, fields: &[&str]) {
    engine
        .get_database("countdb")
        .unwrap()
        .get_collection("parcels")
        .unwrap()
        .create_index(
            name.to_string(),
            fields.iter().map(|f| f.to_string()).collect(),
            IndexType::Persistent,
            false,
        )
        .unwrap();
}

/// The result and the number of full collection scans the query took.
fn run_count(engine: &StorageEngine, sdbql: &str) -> (Vec<serde_json::Value>, u64) {
    let executor = QueryExecutor::with_database(engine, "countdb".to_string());
    let rows = executor.execute(&parse(sdbql).unwrap()).unwrap();
    (rows, executor.collection_scans())
}

const COUNT_QUERIES: &[&str] = &[
    "FOR p IN parcels FILTER p.company == 'acme' AND p.archived_at == null \
     COLLECT WITH COUNT INTO n RETURN n",
    // The fields in the other order.
    "FOR p IN parcels FILTER p.archived_at == null AND p.company == 'globex' \
     COLLECT WITH COUNT INTO n RETURN n",
    // How the Soli ORM counts a `where`.
    "RETURN LENGTH(FOR p IN parcels FILTER p.company == 'acme' AND p.archived_at == null RETURN 1)",
    "FOR p IN parcels FILTER p.company == 'acme' AND p.archived_at == '2026-02-01' \
     COLLECT WITH COUNT INTO n RETURN n",
    // A value no document has.
    "FOR p IN parcels FILTER p.company == 'initech' AND p.archived_at == null \
     COLLECT WITH COUNT INTO n RETURN n",
];

/// With an index on exactly `[company, archived_at]`, or one that starts with
/// them, every count is read from the index and equals the scan's.
#[test]
fn test_composite_count_matches_the_scan() {
    for fields in [
        &["company", "archived_at"][..],
        &["archived_at", "company"][..],
        &["company", "archived_at", "created"][..],
    ] {
        let (engine, _tmp) = create_count_engine();
        let expected: Vec<_> = COUNT_QUERIES
            .iter()
            .map(|q| run_count(&engine, q).0)
            .collect();
        // Explicit nulls and missing fields both count as `== null`.
        assert_eq!(expected[0], vec![json!(120)]);

        add_count_index(&engine, "count_idx", fields);
        for (query, expected) in COUNT_QUERIES.iter().zip(&expected) {
            let (rows, scans) = run_count(&engine, query);
            assert_eq!(&rows, expected, "{fields:?} {query}");
            assert_eq!(scans, 0, "{fields:?} {query}");
        }
    }
}

/// Three equalities over a three-field index.
#[test]
fn test_composite_count_with_three_fields() {
    let (engine, _tmp) = create_count_engine();
    let query = "FOR p IN parcels FILTER p.company == 'acme' AND p.archived_at == null \
                 AND p.status == 'transit' COLLECT WITH COUNT INTO n RETURN n";
    let (expected, _) = run_count(&engine, query);
    add_count_index(
        &engine,
        "company_archived_status",
        &["company", "archived_at", "status"],
    );
    let (rows, scans) = run_count(&engine, query);
    assert_eq!(rows, expected);
    assert_eq!(scans, 0);
}

/// What the index cannot count exactly is left to the scan: a condition that
/// is not an equality, one field twice, an index that does not start with
/// the condition fields, or only nulls (rows with every field null are not
/// indexed).
#[test]
fn test_composite_count_falls_back_to_the_scan() {
    let (engine, _tmp) = create_count_engine();
    add_count_index(&engine, "company_archived", &["company", "archived_at"]);
    add_count_index(
        &engine,
        "status_company_archived",
        &["status", "company", "archived_at"],
    );
    for (query, expected) in [
        (
            "FOR p IN parcels FILTER p.company == 'acme' AND p.created > 100 \
             COLLECT WITH COUNT INTO n RETURN n",
            99,
        ),
        (
            "FOR p IN parcels FILTER p.company == 'acme' AND p.company == 'globex' \
             COLLECT WITH COUNT INTO n RETURN n",
            0,
        ),
        (
            "FOR p IN parcels FILTER p.company == 'acme' AND p.created == 4 \
             COLLECT WITH COUNT INTO n RETURN n",
            1,
        ),
        (
            "FOR p IN parcels FILTER p.archived_at == null AND p.missing == null \
             COLLECT WITH COUNT INTO n RETURN n",
            240,
        ),
    ] {
        let (rows, scans) = run_count(&engine, query);
        assert_eq!(rows, vec![json!(expected)], "{query}");
        assert!(scans > 0, "{query}");
    }
}
