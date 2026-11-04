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

    // The FILTER is still read from the `company` prefix of
    // `company_archived` (no scan); the sort runs over those rows.
    let query = "FOR p IN parcels FILTER p.company == 'acme' SORT p.seq DESC LIMIT 3 RETURN p.seq";
    let (rows, scans) = run_sorted(&engine, query);
    assert_eq!(rows, vec![json!(298), json!(296), json!(294)]);
    assert_eq!(scans, 0);

    // The sort field followed by another field: ties would come out in that
    // field's order, not in the scan's.
    add_index(&engine, "company_seq_status", &["company", "seq", "status"]);
    let (rows, scans) = run_sorted(&engine, query);
    assert_eq!(rows, vec![json!(298), json!(296), json!(294)]);
    assert_eq!(scans, 0);

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

    // Read in index order the rows would come out as 900, 902, 901. The
    // FILTER is read from the `company` prefix and the rows sorted after.
    add_index(&engine, "company_created", &["company", "created"]);
    let (rows, scans) = run_sorted(&engine, query);
    assert_eq!(rows, expected);
    assert_eq!(scans, 0);
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
    // Not counted from the index alone. The first three still read their
    // rows from the `company` prefix of `company_archived` and check the
    // rest per row; the last has nothing an index can read.
    for (query, expected, scanned) in [
        (
            "FOR p IN parcels FILTER p.company == 'acme' AND p.created > 100 \
             COLLECT WITH COUNT INTO n RETURN n",
            99,
            false,
        ),
        (
            "FOR p IN parcels FILTER p.company == 'acme' AND p.company == 'globex' \
             COLLECT WITH COUNT INTO n RETURN n",
            0,
            false,
        ),
        (
            "FOR p IN parcels FILTER p.company == 'acme' AND p.created == 4 \
             COLLECT WITH COUNT INTO n RETURN n",
            1,
            false,
        ),
        (
            "FOR p IN parcels FILTER p.archived_at == null AND p.missing == null \
             COLLECT WITH COUNT INTO n RETURN n",
            240,
            true,
        ),
    ] {
        let (rows, scans) = run_count(&engine, query);
        assert_eq!(rows, vec![json!(expected)], "{query}");
        assert_eq!(scans > 0, scanned, "{query}");
    }
}

// ============ `==` / `IN` prefixes and a range, from a composite index ============

/// A job queue in the shape Soli's claim reads: states in and out of the
/// claimed set, a `run_at` that is a date string on most rows, a number on
/// some and missing on others, and some rows with no state at all.
fn create_jobs_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_database("jobdb".to_string()).unwrap();
    let db = engine.get_database("jobdb").unwrap();
    db.create_collection("jobs".to_string(), None).unwrap();
    let jobs = db.get_collection("jobs").unwrap();
    let states = ["pending", "scheduled", "failed", "done", "running"];
    for i in 0..400i64 {
        let mut doc = json!({ "seq": i, "priority": i % 4 });
        if i % 13 != 0 {
            doc["state"] = json!(states[(i % 5) as usize]);
        }
        if i % 11 == 0 {
            doc["run_at"] = json!(i);
        } else if i % 7 != 0 {
            doc["run_at"] = json!(format!("2026-11-{:02}T00:00:00", i % 28 + 1));
        }
        jobs.insert(doc).unwrap();
    }
    (engine, tmp_dir)
}

fn add_jobs_index(engine: &StorageEngine, name: &str, fields: &[&str], ty: IndexType) {
    engine
        .get_database("jobdb")
        .unwrap()
        .get_collection("jobs")
        .unwrap()
        .create_index(
            name.to_string(),
            fields.iter().map(|f| f.to_string()).collect(),
            ty,
            false,
        )
        .unwrap();
}

/// Rows, index named by EXPLAIN, and full collection scans taken.
fn run_jobs(engine: &StorageEngine, sdbql: &str) -> (Vec<serde_json::Value>, Option<String>, u64) {
    let executor = QueryExecutor::with_database(engine, "jobdb".to_string());
    let query = parse(sdbql).unwrap();
    let rows = executor.execute(&query).unwrap();
    let scans = executor.collection_scans();
    let used = executor.explain(&query).unwrap().collections[0]
        .index_used
        .clone();
    (rows, used, scans)
}

const CLAIMED: &str = "[\"pending\", \"scheduled\", \"failed\"]";

fn range_queries() -> Vec<String> {
    let f = |filter: &str| format!("FOR j IN jobs FILTER {filter} RETURN j.seq");
    vec![
        // Soli's claim, made deterministic on ties.
        format!(
            "FOR j IN jobs FILTER j.state IN {CLAIMED} AND j.run_at <= \"2026-11-14T00:00:00\" \
             SORT j.priority DESC, j.run_at ASC, j.seq ASC LIMIT 20 RETURN j.seq"
        ),
        f(&format!("j.state IN {CLAIMED} AND j.run_at <= \"2026-11-14T00:00:00\"")),
        f("j.state == \"pending\" AND j.run_at < \"2026-11-10\""),
        f(&format!("j.state IN {CLAIMED} AND j.run_at > \"2026-11-20T00:00:00\"")),
        f(&format!("j.state IN {CLAIMED} AND j.run_at >= \"2026-11-20T00:00:00\"")),
        f(&format!(
            "j.state IN {CLAIMED} AND j.run_at >= \"2026-11-05\" AND j.run_at <= \"2026-11-15\""
        )),
        // The field on the right.
        f(&format!("\"2026-11-14\" >= j.run_at AND j.state IN {CLAIMED}")),
        // A numeric bound: missing run_at (null) and numbers below it.
        f(&format!("j.state IN {CLAIMED} AND j.run_at <= 150")),
        f(&format!("j.state IN {CLAIMED} AND j.run_at <= null")),
        f(&format!("j.state IN {CLAIMED} AND j.run_at >= 200")),
        // Repeated IN keys return each row once.
        f("j.state IN [\"pending\", \"pending\", \"failed\"] AND j.run_at <= \"2026-11-14\""),
        // Two INs and a range on the third field.
        f(&format!(
            "j.state IN {CLAIMED} AND j.run_at IN [\"2026-11-03T00:00:00\", \"2026-11-09T00:00:00\"] \
             AND j.priority > 1"
        )),
        // Every field equal.
        f("j.state == \"done\" AND j.run_at == \"2026-11-04T00:00:00\" AND j.priority == 3"),
        f("j.state IN [] AND j.run_at <= \"2026-11-14\""),
        // A residual conjunct outside the index.
        f(&format!(
            "j.state IN {CLAIMED} AND j.run_at <= \"2026-11-14\" AND j.seq % 2 == 0"
        )),
    ]
}

fn sorted_if_unsorted(sdbql: &str, mut rows: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    if !sdbql.contains("SORT") {
        rows.sort_by_key(|v| v.as_i64());
    }
    rows
}

/// With `[state, run_at, priority]` beside single-field indexes on each
/// field (what Soli creates for `_jobs`), every query reads the composite
/// index, scans nothing, and returns exactly what the scan returned.
#[test]
fn test_composite_index_serves_in_and_range() {
    let (engine, _tmp) = create_jobs_engine();
    let queries = range_queries();
    let expected: Vec<_> = queries
        .iter()
        .map(|q| sorted_if_unsorted(q, run_jobs(&engine, q).0))
        .collect();
    assert!(expected[1].len() > 20, "the seed exercises the claim");

    for f in ["state", "run_at", "priority"] {
        add_jobs_index(&engine, &format!("idx_{f}"), &[f], IndexType::Hash);
    }
    add_jobs_index(
        &engine,
        "idx_jobs_claim",
        &["state", "run_at", "priority"],
        IndexType::Persistent,
    );

    for (q, want) in queries.iter().zip(&expected) {
        let (rows, used, scans) = run_jobs(&engine, q);
        assert_eq!(&sorted_if_unsorted(q, rows), want, "{q}");
        assert_eq!(used.as_deref(), Some("idx_jobs_claim"), "{q}");
        assert_eq!(scans, 0, "{q}");
    }
}

/// What the composite read cannot serve falls back, with the same rows.
#[test]
fn test_composite_range_falls_back() {
    let (engine, _tmp) = create_jobs_engine();
    let queries = [
        // Every leading value null: such rows are not in the index.
        "FOR j IN jobs FILTER j.state == null AND j.run_at == null RETURN j.seq",
        // Leading values all null and a range that admits null: a row with
        // every index field null would match and is not in the index.
        "FOR j IN jobs FILTER j.state == null AND j.run_at <= \"2026-11-14\" RETURN j.seq",
        // Not a leading field.
        "FOR j IN jobs FILTER j.run_at <= \"2026-11-14\" AND j.priority == 1 RETURN j.seq",
    ];
    let expected: Vec<_> = queries
        .iter()
        .map(|q| sorted_if_unsorted(q, run_jobs(&engine, q).0))
        .collect();
    let array_bound =
        "FOR j IN jobs FILTER j.state == \"pending\" AND j.run_at <= [\"x\"] RETURN j.seq";
    let array_expected = sorted_if_unsorted(array_bound, run_jobs(&engine, array_bound).0);
    add_jobs_index(
        &engine,
        "idx_jobs_claim",
        &["state", "run_at", "priority"],
        IndexType::Persistent,
    );
    for (q, want) in queries.iter().zip(&expected) {
        let (rows, used, _) = run_jobs(&engine, q);
        assert_eq!(&sorted_if_unsorted(q, rows), want, "{q}");
        assert_eq!(used, None, "{q}");
    }

    // An array bound is not used (the codec orders arrays as JSON text), but
    // the `state` prefix still is; the bound is checked per row.
    let (rows, used, scans) = run_jobs(&engine, array_bound);
    assert!(!array_expected.is_empty());
    assert_eq!(sorted_if_unsorted(array_bound, rows), array_expected);
    assert_eq!(used.as_deref(), Some("idx_jobs_claim"));
    assert_eq!(scans, 0);
}

/// `indexHint` naming a composite index uses it, even for a FILTER that
/// fixes only its first field; `forceIndexHint` still fails when the named
/// index cannot serve the FILTER.
#[test]
fn test_index_hint_names_a_composite_index() {
    let (engine, _tmp) = create_jobs_engine();
    let hinted = [
        format!(
            "FOR j IN jobs OPTIONS {{ indexHint: \"idx_jobs_claim\", forceIndexHint: true }} \
             FILTER j.state IN {CLAIMED} AND j.run_at <= \"2026-11-14\" RETURN j.seq"
        ),
        format!(
            "FOR j IN jobs OPTIONS {{ indexHint: \"idx_jobs_claim\", forceIndexHint: true }} \
             FILTER j.state IN {CLAIMED} RETURN j.seq"
        ),
        "FOR j IN jobs OPTIONS { indexHint: \"idx_jobs_claim\" } \
         FILTER j.state == \"failed\" AND j.priority == 2 RETURN j.seq"
            .to_string(),
    ];
    // The same queries without their OPTIONS, before the index exists.
    let unhinted = |q: &str| {
        let (head, rest) = q.split_once(" OPTIONS ").unwrap();
        format!("{head} {}", rest.split_once("} ").unwrap().1)
    };
    let expected: Vec<_> = hinted
        .iter()
        .map(|q| sorted_if_unsorted(q, run_jobs(&engine, &unhinted(q)).0))
        .collect();
    add_jobs_index(
        &engine,
        "idx_jobs_claim",
        &["state", "run_at", "priority"],
        IndexType::Persistent,
    );
    for (q, want) in hinted.iter().zip(&expected) {
        let executor = QueryExecutor::with_database(&engine, "jobdb".to_string());
        let rows = executor.execute(&parse(q).unwrap()).unwrap();
        assert_eq!(&sorted_if_unsorted(q, rows), want, "{q}");
        assert_eq!(executor.collection_scans(), 0, "{q}");
    }

    let executor = QueryExecutor::with_database(&engine, "jobdb".to_string());
    let err = executor
        .execute(
            &parse(
                "FOR j IN jobs OPTIONS { indexHint: \"idx_jobs_claim\", forceIndexHint: true } \
                 FILTER j.priority == 2 RETURN j.seq",
            )
            .unwrap(),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("could not use index hint"),
        "{err}"
    );
}

// ======= Composite prefixes, `!=` / `NOT IN`, `IN` with null, `OR` unions =======

/// The same 480 rows in `plain` (no index) and `indexed`. Fields are set,
/// null, missing, empty or of another type on some rows, so a read that
/// leaves a row out shows up as a difference.
fn create_planner_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_database("plandb".to_string()).unwrap();
    let db = engine.get_database("plandb").unwrap();
    let owners = [
        json!("u1"),
        json!("u2"),
        json!("u3"),
        json!(""),
        json!(null),
    ];
    let statuses = [
        json!("Open"),
        json!("Working"),
        json!("Closed"),
        json!("Completed"),
        json!(null),
        json!(1),
    ];
    let objects = ["accounts", "contacts", "leads"];
    let opportunities = [json!("o1"), json!("o2"), json!(""), json!(null)];
    for name in ["plain", "indexed"] {
        db.create_collection(name.to_string(), None).unwrap();
        let coll = db.get_collection(name).unwrap();
        for i in 0..480i64 {
            let mut doc = json!({
                "seq": i,
                "object": objects[(i % 3) as usize],
                "sort_order": i % 7,
                "account_id": format!("a{}", i % 4),
                "a": i % 5,
                "b": i % 9,
                "amount": if i % 4 == 0 { json!(1) } else if i % 4 == 1 { json!(1.0) } else { json!(i % 3) },
            });
            if i % 11 != 0 {
                doc["owner_id"] = owners[(i % 5) as usize].clone();
            }
            if i % 13 != 0 {
                doc["status"] = statuses[(i % 6) as usize].clone();
            }
            if i % 7 != 0 {
                doc["opportunity_id"] = opportunities[(i % 4) as usize].clone();
            }
            coll.insert(doc).unwrap();
        }
    }
    let indexed = db.get_collection("indexed").unwrap();
    for (name, fields, ty) in [
        (
            "idx_object_sort",
            vec!["object", "sort_order"],
            IndexType::Hash,
        ),
        (
            "idx_owner_status",
            vec!["owner_id", "status"],
            IndexType::Persistent,
        ),
        (
            "idx_status_amount",
            vec!["status", "amount"],
            IndexType::Persistent,
        ),
        (
            "idx_opp_account",
            vec!["opportunity_id", "account_id"],
            IndexType::Persistent,
        ),
        ("idx_a", vec!["a"], IndexType::Hash),
        ("idx_b", vec!["b"], IndexType::Persistent),
    ] {
        indexed
            .create_index(
                name.to_string(),
                fields.into_iter().map(String::from).collect(),
                ty,
                false,
            )
            .unwrap();
    }
    (engine, tmp_dir)
}

/// `query` (written over `C`) on `plain` then on `indexed`: the rows of each
/// (sorted), the index EXPLAIN names on `indexed`, and its collection scans.
fn run_planner(
    engine: &StorageEngine,
    query: &str,
) -> (
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
    Option<String>,
    u64,
) {
    let run = |coll: &str| {
        let executor = QueryExecutor::with_database(engine, "plandb".to_string());
        let ast = parse(&query.replace(" C ", &format!(" {coll} "))).unwrap();
        let mut rows = executor.execute(&ast).unwrap();
        rows.sort_by_key(|v| v.as_i64());
        let used = executor
            .explain(&ast)
            .ok()
            .and_then(|e| e.collections.first().and_then(|c| c.index_used.clone()));
        (rows, used, executor.collection_scans())
    };
    let (expected, _, _) = run("plain");
    let (rows, used, scans) = run("indexed");
    (expected, rows, used, scans)
}

/// Each query returns what the scan returns, from the index named, with no
/// collection scan.
#[test]
fn test_composite_planner_serves_new_shapes() {
    let (engine, _tmp) = create_planner_engine();
    let f = |filter: &str| format!("FOR d IN C FILTER {filter} RETURN d.seq");
    let cases: Vec<(String, &str)> = vec![
        // A composite index read for its first field alone.
        (f("d.object == \"accounts\""), "idx_object_sort"),
        (
            f("d.object IN [\"accounts\", \"leads\"]"),
            "idx_object_sort",
        ),
        (
            "FOR d IN C FILTER d.object == \"leads\" SORT d.sort_order, d.seq RETURN d.seq"
                .to_string(),
            "idx_object_sort",
        ),
        // `!=` and `NOT IN` after a fixed prefix: null, missing and numeric
        // statuses are not "Completed" and must come back.
        (
            f("d.owner_id == \"u1\" AND d.status != \"Completed\""),
            "idx_owner_status",
        ),
        (
            f("d.owner_id == \"u1\" AND d.status NOT IN [\"Completed\", \"Closed\"]"),
            "idx_owner_status",
        ),
        (
            f("d.owner_id == \"u2\" AND d.status != null"),
            "idx_owner_status",
        ),
        (
            f("d.owner_id IN [\"u1\", \"u3\"] AND \"Closed\" != d.status"),
            "idx_owner_status",
        ),
        (
            f("d.owner_id == \"u1\" AND d.status >= \"Closed\" AND d.status != \"Open\""),
            "idx_owner_status",
        ),
        (
            f("d.owner_id == \"\" AND d.status != \"Open\""),
            "idx_owner_status",
        ),
        // 1 and 1.0 are equal for `!=` and for the index.
        (
            f("d.status == \"Open\" AND d.amount != 1"),
            "idx_status_amount",
        ),
        (
            f("d.status == \"Open\" AND d.amount NOT IN [0, 2]"),
            "idx_status_amount",
        ),
        // `IN` holding null, beside a field that is never null.
        (
            f("d.opportunity_id IN [null, \"\"] AND d.account_id IN [\"a1\", \"a2\"]"),
            "idx_opp_account",
        ),
        (
            f("d.owner_id IN [null, \"u1\"] AND d.status == \"Open\""),
            "idx_owner_status",
        ),
        // `OR`: the union of what each branch's index returns.
        (f("d.a == 1 OR d.b == 2"), "idx_a | idx_b"),
        (f("d.seq > 10 AND (d.a == 1 OR d.b == 2)"), "idx_a | idx_b"),
        (
            f("(d.object == \"accounts\" AND d.sort_order > 3) OR d.a IN [1, 2]"),
            "idx_object_sort | idx_a",
        ),
        (
            f("d.opportunity_id IN [\"o1\", \"o2\"] \
               OR (d.opportunity_id IN [null, \"\"] AND d.account_id IN [\"a1\"])"),
            "idx_opp_account",
        ),
    ];
    for (q, index) in &cases {
        let (expected, rows, used, scans) = run_planner(&engine, q);
        assert!(
            !expected.is_empty() || q.contains("IN []"),
            "seed covers {q}"
        );
        assert_eq!(rows, expected, "{q}");
        assert_eq!(used.as_deref(), Some(*index), "{q}");
        assert_eq!(scans, 0, "{q}");
    }
}

/// What the index cannot answer exactly is left to the scan, with the same
/// rows.
#[test]
fn test_composite_planner_falls_back() {
    let (engine, _tmp) = create_planner_engine();
    let f = |filter: &str| format!("FOR d IN C FILTER {filter} RETURN d.seq");
    for q in [
        // A combination that is all null: rows with every index field null
        // are not indexed.
        f("d.opportunity_id IN [null, \"\"]"),
        f("d.owner_id == null AND d.status != \"Closed\""),
        f("d.owner_id IN [null] AND d.status IN [null, \"Open\"]"),
        // `!=` with nothing fixed before it: null and missing values match it
        // and a single-field index does not hold them.
        f("d.status != \"Closed\""),
        // One branch has no index.
        f("d.a == 1 OR d.seq == 2"),
    ] {
        let (expected, rows, used, scans) = run_planner(&engine, &q);
        assert!(!expected.is_empty(), "seed covers {q}");
        assert_eq!(rows, expected, "{q}");
        assert_eq!(used, None, "{q}");
        assert!(scans > 0, "{q}");
    }
}

/// Counts over a composite index's first field are read from the index.
#[test]
fn test_composite_prefix_counts() {
    let (engine, _tmp) = create_planner_engine();
    for q in [
        "FOR d IN C FILTER d.object == \"accounts\" COLLECT WITH COUNT INTO n RETURN n",
        "RETURN LENGTH(FOR d IN C FILTER d.object == \"contacts\" RETURN 1)",
        "FOR d IN C FILTER d.owner_id == \"u2\" COLLECT WITH COUNT INTO n RETURN n",
    ] {
        let (expected, rows, _, scans) = run_planner(&engine, q);
        assert!(expected[0].as_i64().unwrap() > 0, "{q}");
        assert_eq!(rows, expected, "{q}");
        assert_eq!(scans, 0, "{q}");
    }
}

/// A hint naming a composite index serves `!=` after its prefix.
#[test]
fn test_hint_serves_exclusions() {
    let (engine, _tmp) = create_planner_engine();
    let q = "FOR d IN C OPTIONS { indexHint: \"idx_owner_status\", forceIndexHint: true } \
             FILTER d.owner_id == \"u3\" AND d.status != \"Working\" RETURN d.seq";
    let executor = QueryExecutor::with_database(&engine, "plandb".to_string());
    let mut rows = executor
        .execute(&parse(&q.replace(" C ", " indexed ")).unwrap())
        .unwrap();
    rows.sort_by_key(|v| v.as_i64());
    assert_eq!(executor.collection_scans(), 0);
    let unhinted =
        "FOR d IN plain FILTER d.owner_id == \"u3\" AND d.status != \"Working\" RETURN d.seq";
    let mut expected = executor.execute(&parse(unhinted).unwrap()).unwrap();
    expected.sort_by_key(|v| v.as_i64());
    assert!(!expected.is_empty());
    assert_eq!(rows, expected);
}

/// EXPLAIN names the paths execution takes: rows read in SORT order, counts
/// read from the index, and the loops of subqueries.
#[test]
fn test_explain_reports_index_paths_and_subqueries() {
    let (engine, _tmp) = create_planner_engine();
    let executor = QueryExecutor::with_database(&engine, "plandb".to_string());
    let explain = |q: &str| executor.explain(&parse(q).unwrap()).unwrap();

    // FILTER == prefix, SORT on the next field, LIMIT: read in index order.
    let e = explain(
        "FOR d IN indexed FILTER d.object == \"leads\" SORT d.sort_order LIMIT 3 RETURN d.seq",
    );
    assert_eq!(e.collections[0].access_type, "index_sort");
    assert_eq!(
        e.collections[0].index_used.as_deref(),
        Some("idx_object_sort")
    );
    assert_eq!(
        e.sort.as_ref().unwrap().index.as_deref(),
        Some("idx_object_sort")
    );

    // A bare SORT + LIMIT over an indexed field.
    let e = explain("FOR d IN indexed SORT d.a DESC LIMIT 5 RETURN d.seq");
    assert_eq!(e.collections[0].access_type, "index_sort");
    assert_eq!(e.collections[0].index_used.as_deref(), Some("idx_a"));
    // Not when no index leads with the sort field.
    let e = explain("FOR d IN indexed SORT d.seq DESC LIMIT 5 RETURN d.seq");
    assert_ne!(e.collections[0].access_type, "index_sort");
    assert!(e.sort.as_ref().unwrap().index.is_none());

    // A count read from index entries.
    let e = explain(
        "FOR d IN indexed FILTER d.object == \"accounts\" COLLECT WITH COUNT INTO n RETURN n",
    );
    assert_eq!(e.collections[0].access_type, "index_count");

    // A correlated LET subquery, explained against the first row.
    let e = explain(
        "FOR d IN indexed FILTER d.a == 1 \
         LET o = (FOR x IN indexed FILTER x.b == d.b LIMIT 1 RETURN x.seq) RETURN o",
    );
    assert_eq!(e.collections.len(), 2);
    assert_eq!(e.collections[0].subquery, None);
    assert_eq!(e.collections[1].subquery.as_deref(), Some("LET o"));
    assert_eq!(e.collections[1].index_used.as_deref(), Some("idx_b"));
    // A primary-key read is named as one.
    let e = explain(
        "FOR d IN indexed FILTER d.a == 1 \
         LET o = (FOR x IN indexed FILTER x._key == d._key RETURN x.seq) RETURN o",
    );
    assert_eq!(e.collections[1].access_type, "index_lookup");
    assert_eq!(e.collections[1].index_used.as_deref(), Some("primary key"));
    // ... and a full scan inside one is visible too.
    let e = explain(
        "FOR d IN indexed FILTER d.a == 1 \
         LET o = (FOR x IN plain FILTER x.b == d.b RETURN 1) RETURN o",
    );
    assert_eq!(e.collections[1].access_type, "full_scan");
    assert_eq!(e.collections[1].name, "plain");

    // `LENGTH(subquery)` counted from the index, nested two deep.
    let e = explain("RETURN LENGTH(FOR d IN indexed FILTER d.a == 2 RETURN 1)");
    assert_eq!(e.collections[0].subquery.as_deref(), Some("RETURN"));
    assert_eq!(e.collections[0].access_type, "index_count");
    let e = explain(
        "LET n = FIRST(FOR d IN indexed FILTER d.a == 3 \
                 RETURN LENGTH(FOR x IN indexed FILTER x.b == d.b RETURN 1)) RETURN n",
    );
    let nested: Vec<_> = e.collections.iter().map(|c| c.subquery.clone()).collect();
    assert_eq!(
        nested,
        vec![
            Some("LET n".to_string()),
            Some("LET n > RETURN".to_string())
        ]
    );

    // No row reaches the subquery: said so, not guessed.
    let e = explain(
        "FOR d IN indexed FILTER d.a == 99 \
         LET o = (FOR x IN indexed FILTER x.b == d.b RETURN 1) RETURN o",
    );
    assert_eq!(e.collections.len(), 1);
    assert!(
        e.warnings
            .iter()
            .any(|w| w.contains("LET o: subquery not explained")),
        "{:?}",
        e.warnings
    );
}

// ================= Non-sparse indexes (`sparse: false`) =================

/// The same rows in `plain` (no index) and `dense`, whose indexes hold every
/// document: `f` is a number, a string, an array, null or missing; so is `g`.
fn create_dense_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_database("densedb".to_string()).unwrap();
    let db = engine.get_database("densedb").unwrap();
    let fs = [
        json!(1),
        json!(3),
        json!(7),
        json!("x"),
        json!(null),
        json!([3]),
    ];
    for name in ["plain", "dense"] {
        db.create_collection(name.to_string(), None).unwrap();
        let coll = db.get_collection(name).unwrap();
        for i in 0..300i64 {
            let mut doc = json!({ "seq": i });
            if i % 7 != 0 {
                doc["f"] = fs[(i % 6) as usize].clone();
            }
            if i % 4 == 1 {
                doc["g"] = json!(null);
            } else if i % 4 == 2 {
                doc["g"] = json!(format!("g{}", i % 3));
            }
            coll.insert(doc).unwrap();
        }
    }
    let dense = db.get_collection("dense").unwrap();
    dense
        .create_index_with(
            "idx_f_dense".to_string(),
            vec!["f".to_string()],
            IndexType::Persistent,
            false,
            false,
        )
        .unwrap();
    dense
        .create_index_with(
            "idx_g_f_dense".to_string(),
            vec!["g".to_string(), "f".to_string()],
            IndexType::Hash,
            false,
            false,
        )
        .unwrap();
    (engine, tmp_dir)
}

/// `query` over `C` on `plain` then `dense`: sorted rows of each, the index
/// EXPLAIN names on `dense`, and its collection scans.
fn run_dense(
    engine: &StorageEngine,
    query: &str,
) -> (
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
    Option<String>,
    u64,
) {
    let run = |coll: &str| {
        let executor = QueryExecutor::with_database(engine, "densedb".to_string());
        let ast = parse(&query.replace(" C ", &format!(" {coll} "))).unwrap();
        let mut rows = executor.execute(&ast).unwrap();
        rows.sort_by_key(|v| v.to_string());
        let used = executor
            .explain(&ast)
            .ok()
            .and_then(|e| e.collections.first().and_then(|c| c.index_used.clone()));
        (rows, used, executor.collection_scans())
    };
    let (expected, _, _) = run("plain");
    let (rows, used, scans) = run("dense");
    (expected, rows, used, scans)
}

/// A non-sparse index answers what null satisfies: `== null`, `IN [null]`,
/// `!=` and `NOT IN` with nothing fixed before, `<` / `<=`, and all-null
/// combinations of a composite index — with exactly the scan's rows.
#[test]
fn test_non_sparse_index_serves_null_shapes() {
    let (engine, _tmp) = create_dense_engine();
    let f = |filter: &str| format!("FOR d IN C FILTER {filter} RETURN d.seq");
    let cases: Vec<(String, &str)> = vec![
        (f("d.f == null"), "idx_f_dense"),
        (f("d.f IN [null, 3]"), "idx_f_dense"),
        (f("d.f != 3"), "idx_f_dense"),
        (f("d.f NOT IN [3, \"x\"]"), "idx_f_dense"),
        (f("d.f != null"), "idx_f_dense"),
        (f("d.f <= 5"), "idx_f_dense"),
        (f("d.f < 3"), "idx_f_dense"),
        (f("d.g == null AND d.f == null"), "idx_g_f_dense"),
        (f("d.g IN [null] AND d.f != 1"), "idx_g_f_dense"),
        (
            f("d.g IN [null, \"g1\"] AND d.f IN [null, 7]"),
            "idx_g_f_dense",
        ),
    ];
    for (q, index) in &cases {
        let (expected, rows, used, scans) = run_dense(&engine, q);
        assert!(!expected.is_empty(), "seed covers {q}");
        assert_eq!(rows, expected, "{q}");
        assert_eq!(used.as_deref(), Some(*index), "{q}");
        assert_eq!(scans, 0, "{q}");
    }

    // Counts of null rows are read from the index alone.
    for q in [
        "FOR d IN C FILTER d.f == null COLLECT WITH COUNT INTO n RETURN n",
        "RETURN LENGTH(FOR d IN C FILTER d.g == null AND d.f == null RETURN 1)",
    ] {
        let (expected, rows, _, scans) = run_dense(&engine, q);
        assert!(expected[0].as_i64().unwrap() > 0, "{q}");
        assert_eq!(rows, expected, "{q}");
        assert_eq!(scans, 0, "{q}");
    }

    // The driver's raw rows for `== null` are the ordinary path's bytes.
    let executor = QueryExecutor::with_database(&engine, "densedb".to_string());
    for q in [
        "FOR d IN dense FILTER d.f == null RETURN d",
        "FOR d IN dense FILTER d.f == null LIMIT 3, 4 RETURN d.seq",
    ] {
        let query = parse(q).unwrap();
        let raw = executor.execute_raw_msgpack(&query).unwrap();
        let ordinary =
            rmp_serde::to_vec_named(&serde_json::Value::Array(executor.execute(&query).unwrap()))
                .unwrap();
        assert_eq!(raw.as_ref(), Some(&ordinary), "{q}");
    }

    // An index-ordered SORT puts null and missing values first, as the
    // scan does.
    let q = "FOR d IN C SORT d.f ASC LIMIT 5 RETURN d.f";
    let (expected, rows, _, _) = run_dense(&engine, q);
    assert_eq!(rows, expected);
    assert_eq!(expected, vec![serde_json::Value::Null; 5]);
}

/// Every write path keeps the null entries of a non-sparse index current.
#[test]
fn test_non_sparse_index_tracks_writes() {
    let (engine, _tmp) = create_dense_engine();
    let executor = QueryExecutor::with_database(&engine, "densedb".to_string());
    let run = |q: &str| {
        let mut rows = executor.execute(&parse(q).unwrap()).unwrap();
        rows.sort_by_key(|v| v.to_string());
        rows
    };
    let nulls = || run("FOR d IN dense FILTER d.f == null RETURN d._key");
    let base = nulls().len();

    run("INSERT {_key: \"w1\"} INTO dense");
    run("INSERT {_key: \"w2\", f: null} INTO dense");
    assert_eq!(nulls().len(), base + 2);

    run("UPDATE \"w1\" WITH {f: 3} IN dense");
    assert_eq!(nulls().len(), base + 1);
    assert!(run("FOR d IN dense FILTER d.f == 3 RETURN d._key").contains(&json!("w1")));

    run("REPLACE \"w1\" WITH {other: 1} IN dense");
    assert_eq!(nulls().len(), base + 2);

    run("REMOVE \"w1\" IN dense");
    run("REMOVE \"w2\" IN dense");
    assert_eq!(nulls().len(), base);

    let dense = engine
        .get_database("densedb")
        .unwrap()
        .get_collection("dense")
        .unwrap();
    dense
        .insert_batch(vec![json!({"_key": "b1"}), json!({"_key": "b2", "f": 1})])
        .unwrap();
    assert_eq!(nulls().len(), base + 1);

    // The scan agrees after all of it.
    let scanned = run("FOR d IN dense FILTER NOT (d.f != null) RETURN d._key");
    assert_eq!(nulls(), scanned);

    // Rebuilding keeps the index non-sparse.
    dense.rebuild_all_indexes().unwrap();
    assert_eq!(nulls().len(), base + 1);
    let stats = dense.list_indexes();
    assert!(stats.iter().all(|s| s.sparse == Some(false)), "{stats:?}");
}

/// A unique non-sparse index lets several documents hold null (or miss the
/// field), still rejects a repeated value, and serves `== null`.
#[test]
fn test_unique_non_sparse_index_allows_many_nulls() {
    let (engine, _tmp) = create_dense_engine();
    let db = engine.get_database("densedb").unwrap();
    db.create_collection("people".to_string(), None).unwrap();
    let people = db.get_collection("people").unwrap();
    people
        .create_index_with(
            "uq_email".to_string(),
            vec!["email".to_string()],
            IndexType::Persistent,
            true,
            false,
        )
        .unwrap();
    people.insert(json!({"_key": "p1", "email": null})).unwrap();
    people.insert(json!({"_key": "p2"})).unwrap();
    people
        .insert(json!({"_key": "p3", "email": "a@b.c"}))
        .unwrap();
    assert!(people
        .insert(json!({"_key": "p4", "email": "a@b.c"}))
        .is_err());

    let executor = QueryExecutor::with_database(&engine, "densedb".to_string());
    let q = parse("FOR p IN people FILTER p.email == null SORT p._key RETURN p._key").unwrap();
    assert_eq!(
        executor.execute(&q).unwrap(),
        vec![json!("p1"), json!("p2")]
    );
    assert_eq!(executor.collection_scans(), 0);
}

// ====== Sparse indexes never drop null or missing rows from a result ======

/// `plain` (no index) and `sparse` (sparse indexes on `f` and `[g, f]`) hold
/// the same rows, with `f` null or missing on some; `complete` has an `f` on
/// every row and a sparse index on it.
fn create_sparse_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_database("sparsedb".to_string()).unwrap();
    let db = engine.get_database("sparsedb").unwrap();
    let fs = [
        json!(1),
        json!(3),
        json!(7),
        json!("x"),
        json!(null),
        json!(5),
    ];
    for name in ["plain", "sparse", "complete"] {
        db.create_collection(name.to_string(), None).unwrap();
        let coll = db.get_collection(name).unwrap();
        for i in 0..120i64 {
            let mut doc = json!({ "seq": i });
            if name == "complete" {
                doc["f"] = json!(i % 9);
            } else {
                if i % 7 != 0 {
                    doc["f"] = fs[(i % 6) as usize].clone();
                }
                if i % 3 == 0 {
                    doc["g"] = json!("g");
                }
            }
            coll.insert(doc).unwrap();
        }
    }
    for (coll, name, fields) in [
        ("sparse", "idx_f", vec!["f"]),
        ("sparse", "idx_g_f", vec!["g", "f"]),
        ("complete", "idx_complete_f", vec!["f"]),
    ] {
        db.get_collection(coll)
            .unwrap()
            .create_index(
                name.to_string(),
                fields.into_iter().map(String::from).collect(),
                IndexType::Persistent,
                false,
            )
            .unwrap();
    }
    (engine, tmp_dir)
}

/// Rows of `query` (over `C`) on `plain`, then on `coll`, with the index
/// EXPLAIN reports and the collection scans on `coll`. Rows are compared in
/// order when the query sorts, else sorted.
fn run_sparse(
    engine: &StorageEngine,
    coll: &str,
    query: &str,
) -> (
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
    Option<String>,
    u64,
) {
    let run = |c: &str| {
        let executor = QueryExecutor::with_database(engine, "sparsedb".to_string());
        let ast = parse(&query.replace(" C ", &format!(" {c} "))).unwrap();
        let mut rows = executor.execute(&ast).unwrap();
        if !query.contains("SORT") {
            rows.sort_by_key(|v| v.to_string());
        }
        // Read before EXPLAIN, which runs the query its own way.
        let scans = executor.collection_scans();
        let used = executor
            .explain(&ast)
            .ok()
            .and_then(|e| e.collections.first().and_then(|c| c.index_used.clone()));
        (rows, used, scans)
    };
    let (expected, _, _) = run(if coll == "complete" {
        "complete"
    } else {
        "plain"
    });
    let (rows, used, scans) = run(coll);
    (expected, rows, used, scans)
}

/// `<`, `<=` and `>= null` accept null, which a sparse index does not hold:
/// such a range is only read from it when another term rejects null.
#[test]
fn test_sparse_index_ranges_keep_null_rows() {
    let (engine, _tmp) = create_sparse_engine();
    let f = |filter: &str| format!("FOR d IN C FILTER {filter} RETURN d.seq");
    // (query, served by idx_f)
    for (q, indexed) in [
        (f("d.f <= 5"), false),
        (f("d.f < 5"), false),
        (f("5 >= d.f"), false),
        (f("d.f <= null"), false),
        (f("d.f >= null"), false),
        (f("d.f > 2"), true),
        (f("d.f >= 1 AND d.f <= 5"), true),
        (f("d.f <= 5 AND d.f != null"), true),
        (f("d.f < 5 AND d.f IN [1, 3]"), true),
    ] {
        let (expected, rows, used, scans) = run_sparse(&engine, "sparse", &q);
        assert!(!expected.is_empty(), "seed covers {q}");
        assert_eq!(rows, expected, "{q}");
        assert_eq!(used.as_deref() == Some("idx_f"), indexed, "{q}: {used:?}");
        assert_eq!(scans == 0, indexed, "{q}");
    }
}

/// An index-ordered SORT on a sparse index returns what the scan and sort
/// return: the index is read only when no row it lacks can be in the result.
#[test]
fn test_sparse_index_sorts_keep_null_rows() {
    let (engine, _tmp) = create_sparse_engine();
    // (collection, query, served by the index)
    for (coll, q, indexed) in [
        // Null and missing values sort first: the index lacks them.
        (
            "sparse",
            "FOR d IN C SORT d.f ASC LIMIT 5 RETURN d.f",
            false,
        ),
        ("sparse", "FOR d IN C SORT d.f ASC RETURN d.f", false),
        ("sparse", "FOR d IN C SORT d.f DESC RETURN d.f", false),
        // Descending, they come last: 10 rows are all indexed ones...
        (
            "sparse",
            "FOR d IN C SORT d.f DESC LIMIT 10 RETURN d.f",
            true,
        ),
        // ...but 110 reach past them.
        (
            "sparse",
            "FOR d IN C SORT d.f DESC LIMIT 110 RETURN d.f",
            false,
        ),
        // `[g, f]` leads with `g`, null on most rows.
        (
            "sparse",
            "FOR d IN C SORT d.g ASC LIMIT 5 RETURN d.g",
            false,
        ),
        // Every row has `f`: the sparse index holds them all.
        (
            "complete",
            "FOR d IN C SORT d.f ASC LIMIT 5 RETURN d.f",
            true,
        ),
        ("complete", "FOR d IN C SORT d.f DESC RETURN d.f", true),
    ] {
        let (expected, rows, used, scans) = run_sparse(&engine, coll, q);
        assert_eq!(rows, expected, "{coll}: {q}");
        assert_eq!(scans == 0, indexed, "{coll}: {q}");
        // EXPLAIN reports the ordered read only when it happens.
        let executor = QueryExecutor::with_database(&engine, "sparsedb".to_string());
        let e = executor
            .explain(&parse(&q.replace(" C ", &format!(" {coll} "))).unwrap())
            .unwrap();
        assert_eq!(
            e.collections[0].access_type == "index_sort",
            indexed,
            "{coll}: {q}: {used:?}"
        );
    }
}
