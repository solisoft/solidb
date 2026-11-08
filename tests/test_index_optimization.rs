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

    // No row reaches the subquery: its loops are planned from the shape of
    // their FILTER, outer values standing in as placeholders, and say so.
    let none = "FOR d IN indexed FILTER d.a == 99";
    let e = explain(&format!(
        "{none} LET o = (FOR x IN indexed FILTER x.b == d.b RETURN 1) RETURN o"
    ));
    assert_eq!(e.collections.len(), 2);
    let o = &e.collections[1];
    assert_eq!(
        (
            o.subquery.as_deref(),
            o.access_type.as_str(),
            o.index_used.as_deref()
        ),
        (Some("LET o"), "index_lookup", Some("idx_b"))
    );
    assert_eq!(o.estimated, Some(true));
    assert_eq!(e.collections[0].estimated, None);
    assert!(
        e.warnings
            .iter()
            .any(|w| w.contains("LET o: no row reached the subquery")),
        "{:?}",
        e.warnings
    );
    // A Soli `includes`: the primary key.
    let e = explain(&format!(
        "{none} LET owner = (FOR u IN indexed FILTER u._key == d.owner_id LIMIT 1 RETURN u) \
         RETURN owner"
    ));
    assert_eq!(e.collections[1].index_used.as_deref(), Some("primary key"));
    // `IN` an outer list, an unindexed field, and a subquery inside one.
    let e = explain(&format!(
        "{none} LET o = (FOR x IN indexed FILTER x.a IN d.list \
           LET y = (FOR z IN plain FILTER z.b == x.b RETURN 1) RETURN y) RETURN o"
    ));
    let got: Vec<_> = e
        .collections
        .iter()
        .skip(1)
        .map(|c| {
            (
                c.subquery.clone(),
                c.name.clone(),
                c.access_type.clone(),
                c.estimated,
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (
                Some("LET o".into()),
                "indexed".into(),
                "index_lookup".into(),
                Some(true)
            ),
            (
                Some("LET o > LET y".into()),
                "plain".into(),
                "full_scan".into(),
                Some(true)
            ),
        ]
    );
    assert_eq!(e.collections[1].index_used.as_deref(), Some("idx_a"));
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

/// Whether a sparse index holds every document is cached between writes:
/// each write path that can leave a document out of it must be seen, through
/// whichever handle it arrives, before the next ascending read trusts it.
#[test]
fn test_sparse_sort_completeness_follows_writes() {
    let (engine, _tmp) = create_sparse_engine();
    let executor = QueryExecutor::with_database(&engine, "sparsedb".to_string());
    let sorted = || {
        let q = parse("FOR d IN complete SORT d.f ASC LIMIT 3 RETURN d.f").unwrap();
        let ex = QueryExecutor::with_database(&engine, "sparsedb".to_string());
        let rows = ex.execute(&q).unwrap();
        (rows, ex.collection_scans() == 0)
    };
    let run = |q: &str| executor.execute(&parse(q).unwrap()).unwrap();
    let expect_null_first = |label: &str| {
        let (rows, indexed) = sorted();
        assert_eq!(rows[0], serde_json::Value::Null, "{label}");
        assert!(
            !indexed,
            "{label}: a row the index lacks must send it to the scan"
        );
    };
    let expect_indexed = |label: &str| {
        let (rows, indexed) = sorted();
        assert_ne!(rows[0], serde_json::Value::Null, "{label}");
        assert!(indexed, "{label}");
    };

    expect_indexed("start");
    expect_indexed("cached");

    run("INSERT {_key: \"n1\"} INTO complete");
    expect_null_first("insert without f");
    run("REMOVE \"n1\" IN complete");
    expect_indexed("removed");

    let any_key = run("FOR d IN complete LIMIT 1 RETURN d._key")[0].clone();
    let key = any_key.as_str().unwrap().to_string();
    run(&format!("UPDATE \"{key}\" WITH {{f: null}} IN complete"));
    expect_null_first("update to null");
    run(&format!("UPDATE \"{key}\" WITH {{f: 4}} IN complete"));
    expect_indexed("update back");

    run(&format!("REPLACE \"{key}\" WITH {{seq: -1}} IN complete"));
    expect_null_first("replace without f");
    run(&format!(
        "REPLACE \"{key}\" WITH {{seq: -1, f: 4}} IN complete"
    ));
    expect_indexed("replace back");

    let db = engine.get_database("sparsedb").unwrap();
    db.get_collection("complete")
        .unwrap()
        .insert_batch(vec![json!({"_key": "b1"})])
        .unwrap();
    expect_null_first("insert_batch");
    run("REMOVE \"b1\" IN complete");
    expect_indexed("batch row removed");

    // A write through the engine's own handle for the collection.
    engine
        .get_collection("sparsedb:complete")
        .unwrap()
        .insert(json!({"_key": "e1"}))
        .unwrap();
    expect_null_first("engine handle");
    run("REMOVE \"e1\" IN complete");
    expect_indexed("engine row removed");

    // Rebuilding keeps the answer right.
    db.get_collection("complete")
        .unwrap()
        .rebuild_all_indexes()
        .unwrap();
    expect_indexed("rebuilt");
}

/// When an `OR` is scanned because a branch has no index, EXPLAIN names the
/// branch and the field an index would need; a union it serves says nothing.
#[test]
fn test_explain_names_the_unindexed_or_branch() {
    let (engine, _tmp) = create_planner_engine();
    let executor = QueryExecutor::with_database(&engine, "plandb".to_string());
    let warnings = |filter: &str| {
        let q = parse(&format!("FOR d IN indexed FILTER {filter} RETURN d.seq")).unwrap();
        let e = executor.explain(&q).unwrap();
        let ors: Vec<String> = e
            .warnings
            .into_iter()
            .filter(|w| w.contains("OR scanned"))
            .collect();
        (e.collections[0].access_type.clone(), ors)
    };

    let (access, ors) = warnings("d.a == 1 OR d.seq == 2");
    assert_eq!(access, "full_scan");
    assert_eq!(ors.len(), 1, "{ors:?}");
    assert_eq!(
        ors[0],
        "optimizer: OR scanned, no index serves the branch `d.seq == 2` \
         (an index on `seq` would serve it)"
    );
    assert!(
        !ors[0].contains("d.a"),
        "the served branch is not blamed: {ors:?}"
    );

    let (_, ors) = warnings("d.a == 1 OR LIKE(d.object, \"%x%\", true)");
    assert_eq!(ors.len(), 1, "{ors:?}");
    assert_eq!(
        ors[0],
        "optimizer: OR scanned, no index serves the branch `LIKE(d.object, \"%x%\", true)` \
         (no index can serve this condition)"
    );

    let (access, ors) = warnings("d.a == 1 OR d.b == 2");
    assert_eq!(access, "index_lookup");
    assert!(ors.is_empty(), "{ors:?}");
}

// ===================== Trigram indexes for LIKE "%term%" =====================

/// The same rows in `plain` and `trig` (trigram indexes on `name`, `city`
/// and `n`), with case and Unicode pairs LIKE's case-insensitive form treats
/// as equal, escapes, numbers, arrays, objects, nulls and short texts.
fn create_trigram_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_database("trgdb".to_string()).unwrap();
    let db = engine.get_database("trgdb").unwrap();
    let names = [
        json!("Straße Nord"),
        json!("STRAẞE SÜD"),
        json!("ſtraße alt"),
        json!("Kelvin \u{212A} unit"),
        json!("ΟΔΟΣ Αθηνών"),
        json!("οδος ς"),
        json!("µ-meter"),
        json!("μmeter"),
        json!("Hello World"),
        json!("hello_world"),
        json!("50% off"),
        json!("a\\b path"),
        json!("x"),
        json!(""),
        json!(null),
        json!(12345),
        json!(["arr", "value"]),
        json!({"k": "objval"}),
    ];
    let cities = ["Paris", "paris-nord", "Lyon", "PARISIEN", "Marseille"];
    for coll in ["plain", "trig"] {
        db.create_collection(coll.to_string(), None).unwrap();
        let c = db.get_collection(coll).unwrap();
        for i in 0..90i64 {
            let mut doc = json!({ "seq": i, "n": i * 1117, "city": cities[(i % 5) as usize] });
            if i % 19 != 0 {
                doc["name"] = names[(i % names.len() as i64) as usize].clone();
            }
            c.insert(doc).unwrap();
        }
    }
    let trig = db.get_collection("trig").unwrap();
    for (name, field) in [("trg_name", "name"), ("trg_city", "city"), ("trg_n", "n")] {
        trig.create_index(
            name.to_string(),
            vec![field.to_string()],
            IndexType::Trigram,
            false,
        )
        .unwrap();
    }
    (engine, tmp_dir)
}

fn run_trigram(
    engine: &StorageEngine,
    query: &str,
) -> (
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
    Option<String>,
    u64,
) {
    let run = |coll: &str| {
        let executor = QueryExecutor::with_database(engine, "trgdb".to_string());
        let ast = parse(&query.replace(" C ", &format!(" {coll} "))).unwrap();
        let mut rows = executor.execute(&ast).unwrap();
        rows.sort_by_key(|v| v.as_i64());
        let scans = executor.collection_scans();
        let used = executor
            .explain(&ast)
            .ok()
            .and_then(|e| e.collections.first().and_then(|c| c.index_used.clone()));
        (rows, used, scans)
    };
    let (expected, _, _) = run("plain");
    let (rows, used, scans) = run("trig");
    (expected, rows, used, scans)
}

/// A trigram index returns, for every LIKE shape, exactly the rows the scan
/// returns, and is used whenever the pattern has a literal run of 3+.
#[test]
fn test_trigram_index_serves_like_contains() {
    let (engine, _tmp) = create_trigram_engine();
    let f = |filter: &str| format!("FOR d IN C FILTER {filter} RETURN d.seq");
    // (filter, index expected)
    for (q, index) in [
        (f("LIKE(d.name, \"%straße%\", true)"), Some("trg_name")),
        (f("LIKE(d.name, \"%STRASSE%\", true)"), Some("trg_name")),
        (f("LIKE(d.name, \"%strasse%\", true)"), Some("trg_name")),
        (f("LIKE(d.name, \"%traße%\")"), Some("trg_name")),
        (f("d.name LIKE \"%World%\""), Some("trg_name")),
        (f("LIKE(d.name, \"%world%\")"), Some("trg_name")),
        (f("LIKE(d.name, \"%world%\", true)"), Some("trg_name")),
        (f("LIKE(d.name, \"%hello_world%\", true)"), Some("trg_name")),
        (
            f("LIKE(d.name, \"%hello\\\\_world%\", true)"),
            Some("trg_name"),
        ),
        (f("LIKE(d.name, \"%50\\\\% off%\", true)"), Some("trg_name")),
        (f("LIKE(d.name, \"%a\\\\b p%\")"), Some("trg_name")),
        (f("LIKE(d.name, \"%kelvin k%\", true)"), Some("trg_name")),
        (f("LIKE(d.name, \"%οδοσ%\", true)"), Some("trg_name")),
        (f("LIKE(d.name, \"%ΟΔΟΣ%\", true)"), Some("trg_name")),
        (f("LIKE(d.name, \"%μmeter%\", true)"), Some("trg_name")),
        (f("LIKE(d.name, \"%µ-met%\", true)"), Some("trg_name")),
        (f("LIKE(d.name, \"%arr%\")"), Some("trg_name")),
        (f("LIKE(d.name, \"%objval%\")"), Some("trg_name")),
        (f("LIKE(d.name, \"%2345%\")"), Some("trg_name")),
        (f("LIKE(d.n, \"%234%\")"), Some("trg_n")),
        (f("LIKE(d.name, \"Hello%\")"), Some("trg_name")),
        // Soli's search box: an OR of two indexed fields is a union.
        (
            f("(LIKE(d.name, \"%hello%\", true) OR LIKE(d.city, \"%paris%\", true))"),
            Some("trg_name | trg_city"),
        ),
        // Nothing three characters long: the index cannot narrow it.
        (f("LIKE(d.name, \"%ab%\", true)"), None),
        (f("LIKE(d.name, \"%%\", true)"), None),
        (f("LIKE(d.name, \"h_l_o%\", true)"), None),
        // A trigram index never answers `==`.
        (f("d.name == \"Hello World\""), None),
    ] {
        let (expected, rows, used, scans) = run_trigram(&engine, &q);
        assert_eq!(rows, expected, "{q}");
        assert_eq!(used.as_deref(), index, "{q}");
        assert_eq!(scans == 0, index.is_some(), "{q}");
    }
    // The seed is not vacuous: the case-folding pairs do match.
    let (expected, _, _, _) = run_trigram(&engine, &f("LIKE(d.name, \"%straße%\", true)"));
    assert!(expected.len() >= 10, "{expected:?}");
}

/// Writes keep a trigram index's entries current, and creation refuses what
/// it cannot index.
#[test]
fn test_trigram_index_tracks_writes() {
    let (engine, _tmp) = create_trigram_engine();
    let executor = QueryExecutor::with_database(&engine, "trgdb".to_string());
    let run = |q: &str| executor.execute(&parse(q).unwrap()).unwrap();
    let found = |term: &str| {
        let mut rows = run(&format!(
            "FOR d IN trig FILTER LIKE(d.name, \"%{term}%\", true) RETURN d._key"
        ));
        rows.sort_by_key(|v| v.to_string());
        rows
    };
    run("INSERT {_key: \"w1\", name: \"Brand New Street\"} INTO trig");
    assert_eq!(found("new str"), vec![json!("w1")]);
    run("UPDATE \"w1\" WITH {name: \"Old Lane\"} IN trig");
    assert!(found("new str").is_empty());
    assert_eq!(found("old la"), vec![json!("w1")]);
    run("REPLACE \"w1\" WITH {name: \"Old Lane East\"} IN trig");
    assert_eq!(found("lane east"), vec![json!("w1")]);
    run("REMOVE \"w1\" IN trig");
    assert!(found("old la").is_empty());

    let trig = engine
        .get_database("trgdb")
        .unwrap()
        .get_collection("trig")
        .unwrap();
    trig.insert_batch(vec![json!({"_key": "b1", "name": "Batch Avenue"})])
        .unwrap();
    assert_eq!(found("avenue"), vec![json!("b1")]);
    trig.rebuild_all_indexes().unwrap();
    assert_eq!(found("avenue"), vec![json!("b1")]);

    // A forced hint naming the trigram index is honoured.
    let hinted = run(
        "FOR d IN trig OPTIONS { indexHint: \"trg_name\", forceIndexHint: true } \
                      FILTER LIKE(d.name, \"%avenue%\", true) RETURN d._key",
    );
    assert_eq!(hinted, vec![json!("b1")]);

    assert!(trig
        .create_index(
            "bad1".into(),
            vec!["a".into(), "b".into()],
            IndexType::Trigram,
            false
        )
        .is_err());
    assert!(trig
        .create_index("bad2".into(), vec!["a".into()], IndexType::Trigram, true)
        .is_err());
}

/// A trigram every document holds narrows nothing: the read would fetch the
/// whole collection one document at a time, so the query scans instead, with
/// the same rows; a selective term on the same index is still read from it.
#[test]
fn test_trigram_index_steps_aside_when_unselective() {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_database("t".to_string()).unwrap();
    let db = engine.get_database("t").unwrap();
    db.create_collection("acc".to_string(), None).unwrap();
    let acc = db.get_collection("acc").unwrap();
    let docs: Vec<_> = (0..3000)
        .map(|i| json!({ "seq": i, "name": format!("Client {i} SARL") }))
        .collect();
    acc.insert_batch(docs).unwrap();
    acc.create_index("trg".into(), vec!["name".into()], IndexType::Trigram, false)
        .unwrap();
    let executor = QueryExecutor::with_database(&engine, "t".to_string());
    let run = |q: &str| {
        let ex = QueryExecutor::with_database(&engine, "t".to_string());
        let rows = ex.execute(&parse(q).unwrap()).unwrap();
        (rows.len(), ex.collection_scans())
    };
    let (rows, scans) = run("FOR a IN acc FILTER LIKE(a.name, \"%sarl%\", true) RETURN a.seq");
    assert_eq!((rows, scans > 0), (3000, true));
    let (rows, scans) =
        run("FOR a IN acc FILTER LIKE(a.name, \"%client 1234%\", true) RETURN a.seq");
    assert_eq!((rows, scans), (1, 0));
    let e = executor
        .explain(&parse("FOR a IN acc FILTER LIKE(a.name, \"%sarl%\", true) RETURN a").unwrap())
        .unwrap();
    assert_eq!(e.collections[0].access_type, "full_scan");
}

/// `explain_plan` (`analyze: false`) chooses the accesses EXPLAIN reports,
/// marks them estimated, and reads and writes nothing.
#[test]
fn test_explain_plan_without_running() {
    let (engine, _tmp) = create_planner_engine();
    let main_access = |q: &str| {
        let ast = parse(q).unwrap();
        let ex = QueryExecutor::with_database(&engine, "plandb".to_string());
        let ran = ex.explain(&ast).unwrap();
        let ex = QueryExecutor::with_database(&engine, "plandb".to_string());
        let plan = ex.explain_plan(&ast).unwrap();
        assert_eq!(ex.collection_scans(), 0, "{q}: a plan reads nothing");
        assert!(plan.plan_only, "{q}");
        assert_eq!(plan.timing.total_us, 0, "{q}");
        assert_eq!(plan.documents_scanned, 0, "{q}");
        assert!(
            plan.collections.iter().all(|c| c.estimated == Some(true)),
            "{q}"
        );
        (ran, plan)
    };
    for q in [
        "FOR d IN indexed FILTER d.object == \"accounts\" RETURN d",
        "FOR d IN indexed FILTER d.a == 1 OR d.b == 2 RETURN d",
        "FOR d IN indexed FILTER d.owner_id == \"u1\" AND d.status != \"Completed\" RETURN d",
        "FOR d IN indexed FILTER d.seq == 5 RETURN d",
        "FOR d IN indexed SORT d.a DESC LIMIT 5 RETURN d",
    ] {
        let (ran, plan) = main_access(q);
        assert_eq!(
            (
                plan.collections[0].access_type.as_str(),
                &plan.collections[0].index_used
            ),
            (
                ran.collections[0].access_type.as_str(),
                &ran.collections[0].index_used
            ),
            "{q}"
        );
    }

    // Subqueries are planned too, with the labels a run gives them.
    let (_, plan) = main_access(
        "FOR d IN indexed FILTER d.a == 1 \
         LET o = (FOR x IN indexed FILTER x.b == d.b RETURN 1) RETURN o",
    );
    assert_eq!(plan.collections[1].subquery.as_deref(), Some("LET o"));
    assert_eq!(plan.collections[1].index_used.as_deref(), Some("idx_b"));

    // A scan the run would make is not made.
    let ast = parse("FOR d IN plain FILTER d.seq > 3 RETURN d").unwrap();
    let ex = QueryExecutor::with_database(&engine, "plandb".to_string());
    ex.explain(&ast).unwrap();
    assert!(ex.collection_scans() > 0);
    let ex = QueryExecutor::with_database(&engine, "plandb".to_string());
    let plan = ex.explain_plan(&ast).unwrap();
    assert_eq!(ex.collection_scans(), 0);
    assert_eq!(plan.collections[0].access_type, "full_scan");

    // A write is planned, not made.
    let count = || {
        engine
            .get_database("plandb")
            .unwrap()
            .get_collection("plain")
            .unwrap()
            .count()
    };
    let before = count();
    let ex = QueryExecutor::with_database(&engine, "plandb".to_string());
    ex.explain_plan(&parse("FOR i IN 1..3 INSERT {x: i} INTO plain").unwrap())
        .unwrap();
    assert_eq!(count(), before);
}

/// 3,000 tickets in `plain` and in `indexed` (single-field indexes on
/// `status`, `amount`, `owner_id`, `opportunity_id`, `account_id`, a
/// trigram on `name`): `status` is "open" for every row but the last ten,
/// `owner_id` takes three values, `opportunity_id` and `account_id` are
/// unique, and "SARL" is in two thirds of the names.
fn create_cost_engine() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_database("costdb".to_string()).unwrap();
    let db = engine.get_database("costdb").unwrap();
    for name in ["plain", "indexed"] {
        db.create_collection(name.to_string(), None).unwrap();
        let docs: Vec<_> = (0..3000i64)
            .map(|i| {
                json!({
                    "seq": i,
                    "status": if i < 2990 { "open" } else { "closed" },
                    "amount": i,
                    "owner_id": format!("u{}", i % 3),
                    "opportunity_id": format!("o{i}"),
                    "account_id": format!("a{i}"),
                    "name": if i % 3 == 0 { format!("Client {i}") } else { format!("Client {i} SARL") },
                })
            })
            .collect();
        db.get_collection(name).unwrap().insert_batch(docs).unwrap();
    }
    let indexed = db.get_collection("indexed").unwrap();
    for (name, field, ty) in [
        ("idx_status", "status", IndexType::Hash),
        ("idx_amount", "amount", IndexType::Persistent),
        ("idx_owner", "owner_id", IndexType::Hash),
        ("idx_opp", "opportunity_id", IndexType::Hash),
        ("idx_acc", "account_id", IndexType::Hash),
        ("trg_name", "name", IndexType::Trigram),
    ] {
        indexed
            .create_index(name.to_string(), vec![field.to_string()], ty, false)
            .unwrap();
    }
    (engine, tmp)
}

/// `filter` over `plain` then `indexed`: the scan's rows, the index's rows,
/// and the indexed run's EXPLAIN (index named, estimate, documents read).
fn run_cost(
    engine: &StorageEngine,
    filter: &str,
) -> (
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
    solidb::sdbql::QueryExplain,
) {
    let run = |coll: &str| {
        let ex = QueryExecutor::with_database(engine, "costdb".to_string());
        let ast = parse(&format!("FOR d IN {coll} FILTER {filter} RETURN d.seq")).unwrap();
        let mut rows = ex.execute(&ast).unwrap();
        rows.sort_by_key(|v| v.as_i64());
        (rows, ex.explain(&ast).unwrap())
    };
    let (expected, _) = run("plain");
    let (rows, explain) = run("indexed");
    (expected, rows, explain)
}

/// The planner reads the candidate with the fewest index entries, not the
/// first that applies: a selective range over a common equality (in either
/// conjunct order), a two-row union over a thousand-row `IN`, the interval
/// between a range's two sides. EXPLAIN reports what it counted.
#[test]
fn test_planner_reads_the_cheapest_candidate() {
    let (engine, _tmp) = create_cost_engine();
    for (filter, index, estimate, read) in [
        // 2,990 open rows against ten amounts.
        ("d.status == \"open\" AND d.amount >= 2985", "idx_amount", 15, 15),
        ("d.amount >= 2985 AND d.status == \"open\"", "idx_amount", 15, 15),
        // ...and the equality when it is the selective side.
        ("d.status == \"closed\" AND d.amount >= 100", "idx_status", 10, 10),
        // Two thousand team rows against the union of two unique keys.
        (
            "d.owner_id IN [\"u0\", \"u1\"] AND (d.opportunity_id == \"o5\" OR d.account_id == \"a7\")",
            "idx_opp | idx_acc",
            2,
            2,
        ),
        // Both sides of a range on one field: the interval between them,
        // not the tighter side (20 and 10 entries through 2.7.2).
        ("d.amount >= 10 AND d.amount < 20", "idx_amount", 10, 10),
        ("d.amount >= 2990 AND d.amount < 2995", "idx_amount", 5, 5),
        // A rare trigram ("123" in thirteen names) over the open rows.
        (
            "LIKE(d.name, \"%client 123%\", true) AND d.status == \"open\"",
            "trg_name",
            13,
            11,
        ),
    ] {
        let (expected, rows, e) = run_cost(&engine, filter);
        assert_eq!(rows, expected, "{filter}");
        let access = &e.collections[0];
        assert_eq!(access.index_used.as_deref(), Some(index), "{filter}");
        assert_eq!(access.estimated_rows, Some(estimate), "{filter}");
        assert_eq!(e.documents_scanned, read, "{filter}");
    }
}

/// Equal estimates keep the order the planner always used (equalities
/// before ranges, then FILTER order); a lone candidate is not counted.
#[test]
fn test_planner_ties_keep_the_old_order() {
    let (engine, _tmp) = create_cost_engine();
    for (filter, index, estimate) in [
        // One row each way: the equality, as before.
        (
            "d.amount == 7 AND d.opportunity_id == \"o7\"",
            "idx_amount",
            Some(1),
        ),
        (
            "d.opportunity_id == \"o7\" AND d.amount == 7",
            "idx_opp",
            Some(1),
        ),
        // (`<=` would not be a candidate: null rows pass it, and the
        // sparse index holds none.)
        (
            "d.amount >= 2999 AND d.account_id == \"a2999\"",
            "idx_acc",
            Some(1),
        ),
        // Nothing to compare.
        ("d.owner_id == \"u1\" AND d.seq > 5", "idx_owner", None),
    ] {
        let (expected, rows, e) = run_cost(&engine, filter);
        assert_eq!(rows, expected, "{filter}");
        assert_eq!(
            e.collections[0].index_used.as_deref(),
            Some(index),
            "{filter}"
        );
        assert_eq!(e.collections[0].estimated_rows, estimate, "{filter}");
    }
}

/// A trigram read that would step aside (its rarest trigram is in more
/// than a quarter of the collection) is never the cheapest, though it
/// counts fewer entries: "SARL" is in two thirds of the names, so the
/// status is read, and EXPLAIN's plan says so too.
#[test]
fn test_planner_passes_over_a_trigram_read_that_steps_aside() {
    let (engine, _tmp) = create_cost_engine();
    let filter = "LIKE(d.name, \"%sarl%\", true) AND d.status == \"open\"";
    let (expected, rows, e) = run_cost(&engine, filter);
    assert_eq!(rows, expected);
    assert_eq!(rows.len(), 1993);
    assert_eq!(e.collections[0].index_used.as_deref(), Some("idx_status"));
    assert_eq!(e.collections[0].estimated_rows, Some(2990));

    let ex = QueryExecutor::with_database(&engine, "costdb".to_string());
    let plan = ex
        .explain_plan(&parse(&format!("FOR d IN indexed FILTER {filter} RETURN d")).unwrap())
        .unwrap();
    assert_eq!(
        plan.collections[0].index_used.as_deref(),
        Some("idx_status")
    );
    assert_eq!(plan.collections[0].estimated_rows, Some(2990));
}

/// A plan-only EXPLAIN counts nothing for a placeholder: in a correlated
/// subquery the outer value is not known, so the loop's read is named but
/// carries no `estimated_rows`; the query's own loop keeps its count.
#[test]
fn test_planned_subquery_reports_no_count_for_placeholders() {
    let (engine, _tmp) = create_cost_engine();
    let ex = QueryExecutor::with_database(&engine, "costdb".to_string());
    let plan = ex
        .explain_plan(
            &parse(
                "FOR d IN indexed FILTER d.status == \"closed\" AND d.amount >= 100 \
                 LET same = (FOR x IN indexed FILTER x.owner_id == d.owner_id \
                 AND x.amount >= 2999 RETURN x.seq) RETURN same",
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        plan.collections[0].index_used.as_deref(),
        Some("idx_status")
    );
    assert_eq!(plan.collections[0].estimated_rows, Some(10));
    assert_eq!(plan.collections[1].subquery.as_deref(), Some("LET same"));
    assert!(plan.collections[1].index_used.is_some());
    assert_eq!(plan.collections[1].estimated_rows, None);
}

/// `query` on `costdb`, explained by a run and by a plan (`analyze: false`).
fn explain_both(
    engine: &StorageEngine,
    query: &str,
) -> (
    solidb::DbResult<solidb::sdbql::QueryExplain>,
    solidb::DbResult<solidb::sdbql::QueryExplain>,
) {
    let ast = parse(query).unwrap();
    let ran = QueryExecutor::with_database(engine, "costdb".to_string()).explain(&ast);
    let plan = QueryExecutor::with_database(engine, "costdb".to_string()).explain_plan(&ast);
    (ran, plan)
}

/// EXPLAIN reads the loop's `indexHint` as the run does: a hinted index
/// that serves is named (and, not having been compared, carries no
/// count); one that cannot is named in a warning beside the planner's own
/// choice; a forced one that cannot fails EXPLAIN with the run's error.
#[test]
fn test_explain_follows_the_index_hint() {
    let (engine, _tmp) = create_cost_engine();
    let filter = "FILTER d.status == \"open\" AND d.amount >= 2985 RETURN d.seq";

    // Served: the run reads idx_status, and both EXPLAINs say so.
    let q = format!("FOR d IN indexed OPTIONS {{ indexHint: \"idx_status\" }} {filter}");
    let ex = QueryExecutor::with_database(&engine, "costdb".to_string());
    assert_eq!(ex.execute(&parse(&q).unwrap()).unwrap().len(), 5);
    let (ran, plan) = explain_both(&engine, &q);
    for e in [ran.unwrap(), plan.unwrap()] {
        assert_eq!(e.collections[0].index_used.as_deref(), Some("idx_status"));
        assert_eq!(e.collections[0].estimated_rows, None);
        assert!(
            !e.warnings.iter().any(|w| w.contains("indexHint")),
            "{:?}",
            e.warnings
        );
    }

    // Cannot serve: the planner's choice, and a warning naming the hint.
    let q = format!("FOR d IN indexed OPTIONS {{ indexHint: \"idx_owner\" }} {filter}");
    let (ran, plan) = explain_both(&engine, &q);
    for e in [ran.unwrap(), plan.unwrap()] {
        assert_eq!(e.collections[0].index_used.as_deref(), Some("idx_amount"));
        assert_eq!(e.collections[0].estimated_rows, Some(15));
        assert!(
            e.warnings
                .iter()
                .any(|w| w.contains("indexHint [idx_owner] cannot serve the FILTER")),
            "{:?}",
            e.warnings
        );
    }

    // Forced and cannot serve: the run's error, from both.
    let q = format!(
        "FOR d IN indexed OPTIONS {{ indexHint: \"idx_owner\", forceIndexHint: true }} {filter}"
    );
    let ex = QueryExecutor::with_database(&engine, "costdb".to_string());
    let run_error = ex.execute(&parse(&q).unwrap()).unwrap_err().to_string();
    assert!(run_error.contains("forceIndexHint"), "{run_error}");
    let (ran, plan) = explain_both(&engine, &q);
    assert_eq!(ran.unwrap_err().to_string(), run_error);
    assert_eq!(plan.unwrap_err().to_string(), run_error);
}

/// A forced hint in a subquery no row reaches fails only if one does: both
/// EXPLAINs warn instead of failing. A trigram hint whose read would step
/// aside is not followed, by the run or by either EXPLAIN.
#[test]
fn test_explain_hint_edge_cases() {
    let (engine, _tmp) = create_cost_engine();
    let q = "FOR d IN indexed FILTER d.status == \"nope\" \
             LET same = (FOR x IN indexed OPTIONS { indexHint: \"idx_owner\", forceIndexHint: true } \
             FILTER x.amount == d.amount RETURN 1) RETURN same";
    let ex = QueryExecutor::with_database(&engine, "costdb".to_string());
    assert!(ex.execute(&parse(q).unwrap()).unwrap().is_empty());
    let (ran, plan) = explain_both(&engine, q);
    for e in [ran.unwrap(), plan.unwrap()] {
        assert!(
            e.warnings
                .iter()
                .any(|w| w.starts_with("LET same: a row reaching it fails")
                    && w.contains("forceIndexHint")),
            "{:?}",
            e.warnings
        );
    }

    let q = "FOR d IN indexed OPTIONS { indexHint: \"trg_name\" } \
             FILTER LIKE(d.name, \"%sarl%\", true) AND d.status == \"open\" RETURN d.seq";
    let ex = QueryExecutor::with_database(&engine, "costdb".to_string());
    assert_eq!(ex.execute(&parse(q).unwrap()).unwrap().len(), 1993);
    let (ran, plan) = explain_both(&engine, q);
    for e in [ran.unwrap(), plan.unwrap()] {
        assert_eq!(e.collections[0].index_used.as_deref(), Some("idx_status"));
        assert!(
            e.warnings
                .iter()
                .any(|w| w.contains("indexHint [trg_name] cannot serve")),
            "{:?}",
            e.warnings
        );
    }

    // A rare term: the hinted trigram serves, in the run and both EXPLAINs.
    let q = "FOR d IN indexed OPTIONS { indexHint: \"trg_name\" } \
             FILTER LIKE(d.name, \"%client 123%\", true) AND d.status == \"open\" RETURN d.seq";
    let (ran, plan) = explain_both(&engine, q);
    for e in [ran.unwrap(), plan.unwrap()] {
        assert_eq!(e.collections[0].index_used.as_deref(), Some("trg_name"));
        assert_eq!(e.collections[0].estimated_rows, None);
    }
}

// ============ Two-sided ranges on a single-field index (card #723) ============

/// 600 rows with explicit keys (not in `t` order, so the tie order of the
/// scan and of the index are the same and compared) in three collections:
/// `plain` (no index), `sparse` (the default sparse index on `t`) and
/// `dense` (a non-sparse one). `t` is mostly a timestamp string with many
/// ties, and otherwise missing, null, a number, a bool, an array or an
/// object. `kind` and `grp` are for residual conditions; `grp` is indexed,
/// with four "rare" rows, two of them in the test's window.
fn create_range_engine() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_database("rangedb".to_string()).unwrap();
    let db = engine.get_database("rangedb").unwrap();
    let n = 600i64;
    let docs: Vec<_> = (0..n)
        .map(|i| {
            let mut doc = json!({
                "_key": format!("k{:04}", (i * 37) % n),
                "seq": i,
                "kind": i % 7,
                "grp": if i % 150 == 26 { "rare" } else { "common" },
            });
            let t = match i % 20 {
                0 => None,
                1 => Some(json!(null)),
                2 => Some(json!(i % 50)),
                3 => Some(json!(i % 2 == 0)),
                4 => Some(json!([i % 3])),
                5 => Some(json!({ "x": i % 3 })),
                _ => Some(json!(format!(
                    "2024-06-{:02}T{:02}",
                    (i * 11) % 28 + 1,
                    i % 3
                ))),
            };
            if let Some(t) = t {
                doc["t"] = t;
            }
            doc
        })
        .collect();
    for name in ["plain", "sparse", "dense"] {
        db.create_collection(name.to_string(), None).unwrap();
        db.get_collection(name)
            .unwrap()
            .insert_batch(docs.clone())
            .unwrap();
    }
    let sparse = db.get_collection("sparse").unwrap();
    sparse
        .create_index(
            "idx_t".to_string(),
            vec!["t".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();
    let dense = db.get_collection("dense").unwrap();
    dense
        .create_index_with(
            "idx_t_dense".to_string(),
            vec!["t".to_string()],
            IndexType::Persistent,
            false,
            false,
        )
        .unwrap();
    for coll in [&sparse, &dense] {
        coll.create_index(
            "idx_grp".to_string(),
            vec!["grp".to_string()],
            IndexType::Hash,
            false,
        )
        .unwrap();
    }
    (engine, tmp)
}

/// `query` with `C` replaced by `coll`: its rows (sorted when the query does
/// not sort), the collection scans it made, and EXPLAIN, ran and planned.
fn run_range(
    engine: &StorageEngine,
    coll: &str,
    query: &str,
) -> (
    Vec<serde_json::Value>,
    u64,
    solidb::sdbql::QueryExplain,
    solidb::sdbql::QueryExplain,
) {
    let ast = parse(&query.replace(" C ", &format!(" {coll} "))).unwrap();
    let executor = QueryExecutor::with_database(engine, "rangedb".to_string());
    let mut rows = executor.execute(&ast).unwrap();
    if !query.contains("SORT") {
        rows.sort_by_key(|v| v.as_i64());
    }
    let scans = executor.collection_scans();
    let ran = QueryExecutor::with_database(engine, "rangedb".to_string())
        .explain(&ast)
        .unwrap();
    let plan = QueryExecutor::with_database(engine, "rangedb".to_string())
        .explain_plan(&ast)
        .unwrap();
    (rows, scans, ran, plan)
}

/// Two-sided ranges, every operator in either order, the field on either
/// side, duplicated bounds, across types: each reads the interval between
/// its bounds (and no more), returns what the scan returns, and EXPLAIN —
/// ran or planned — counts the interval's entries.
#[test]
fn test_two_sided_range_reads_the_interval() {
    let (engine, _tmp) = create_range_engine();
    let filters = [
        "d.t >= \"2024-06-05\" AND d.t < \"2024-06-12\"",
        "d.t > \"2024-06-05T00\" AND d.t <= \"2024-06-12T01\"",
        "d.t < \"2024-06-12\" AND d.t >= \"2024-06-05\"",
        "d.t <= \"2024-06-12T01\" AND d.t > \"2024-06-05T00\"",
        // The field on the right-hand side.
        "\"2024-06-05\" <= d.t AND \"2024-06-12\" > d.t",
        "\"2024-06-12T01\" >= d.t AND d.t > \"2024-06-05T00\"",
        // Duplicated bounds: the tightest of each side.
        "d.t >= \"2024-06-01\" AND d.t > \"2024-06-05T00\" AND d.t < \"2024-06-20\" \
         AND d.t <= \"2024-06-12T01\"",
        // Equal bounds, one exclusive: the exclusive one.
        "d.t >= \"2024-06-05T00\" AND d.t > \"2024-06-05T00\" AND d.t < \"2024-06-09\"",
        // A point.
        "d.t >= \"2024-06-08T01\" AND d.t <= \"2024-06-08T01\"",
        // Numbers, and bounds of two types: numbers above 20, then every
        // string below the 3rd, as SDBQL ranks types.
        "d.t >= 10 AND d.t < 30",
        "d.t > 20 AND d.t < \"2024-06-03\"",
        "d.t > false AND d.t <= 5",
        // Two FILTERs are one conjunction.
        "d.t >= \"2024-06-05\" FILTER d.t < \"2024-06-12\"",
    ];
    for filter in filters {
        let q = format!("FOR d IN C FILTER {filter} RETURN d.seq");
        let (expected, _, _, _) = run_range(&engine, "plain", &q);
        assert!(!expected.is_empty(), "the seed covers {filter}");
        for (coll, index) in [("sparse", "idx_t"), ("dense", "idx_t_dense")] {
            let (rows, scans, ran, plan) = run_range(&engine, coll, &q);
            assert_eq!(rows, expected, "{coll}: {filter}");
            assert_eq!(scans, 0, "{coll}: {filter}");
            for e in [&ran, &plan] {
                let access = &e.collections[0];
                assert_eq!(access.access_type, "index_lookup", "{coll}: {filter}");
                assert_eq!(
                    access.index_used.as_deref(),
                    Some(index),
                    "{coll}: {filter}"
                );
                // Nothing but the range: the interval is the answer.
                assert_eq!(
                    access.estimated_rows,
                    Some(expected.len()),
                    "{coll}: {filter}"
                );
            }
            assert_eq!(ran.documents_scanned, expected.len(), "{coll}: {filter}");
            assert_eq!(ran.documents_returned, expected.len(), "{coll}: {filter}");
            // The FILTERs are reported, with the rows they kept.
            assert!(!ran.filters.is_empty(), "{coll}: {filter}");
            assert_eq!(
                ran.filters.last().unwrap().documents_after,
                expected.len(),
                "{coll}: {filter}"
            );
        }
    }
}

/// An empty interval reads nothing; a FILTER that admits null keeps its old
/// reads (the sparse index cannot serve it, the non-sparse one holds null).
#[test]
fn test_two_sided_range_edge_cases() {
    let (engine, _tmp) = create_range_engine();
    for filter in [
        "d.t >= \"2024-06-12\" AND d.t < \"2024-06-05\"",
        "d.t > \"2024-06-05T00\" AND d.t < \"2024-06-05T00\"",
        "d.t > 30 AND d.t <= 10",
    ] {
        for coll in ["plain", "sparse", "dense"] {
            let q = format!("FOR d IN C FILTER {filter} RETURN d.seq");
            let (rows, _, ran, _) = run_range(&engine, coll, &q);
            assert!(rows.is_empty(), "{coll}: {filter}");
            if coll != "plain" {
                assert_eq!(ran.documents_scanned, 0, "{coll}: {filter}");
                assert_eq!(
                    ran.collections[0].estimated_rows,
                    Some(0),
                    "{coll}: {filter}"
                );
            }
            let q = format!("FOR d IN C FILTER {filter} SORT d.t DESC LIMIT 3 RETURN d.seq");
            assert!(
                run_range(&engine, coll, &q).0.is_empty(),
                "{coll}: {filter}"
            );
        }
    }

    // `>= null` admits null and missing values.
    for filter in [
        "d.t >= null AND d.t < 5",
        "d.t <= 5 AND d.t >= null",
        "d.t < \"2024-06-03\" AND d.t >= false",
    ] {
        let q = format!("FOR d IN C FILTER {filter} RETURN d.seq");
        let (expected, _, _, _) = run_range(&engine, "plain", &q);
        assert!(!expected.is_empty(), "the seed covers {filter}");
        let (rows, scans, ran, _) = run_range(&engine, "dense", &q);
        assert_eq!(rows, expected, "dense: {filter}");
        assert_eq!(scans, 0, "dense: {filter}");
        assert_eq!(ran.collections[0].estimated_rows, Some(expected.len()));
        let (rows, _, _, _) = run_range(&engine, "sparse", &q);
        assert_eq!(rows, expected, "sparse: {filter}");
    }
    let q = "FOR d IN C FILTER d.t >= null AND d.t < 5 RETURN d.seq";
    let (_, scans, ran, _) = run_range(&engine, "sparse", q);
    assert!(scans > 0, "the sparse index lacks the null rows");
    assert_eq!(ran.collections[0].access_type, "full_scan");
}

/// A residual condition that rejects most rows of the window: the interval
/// is read, the condition checked on each row, and EXPLAIN reports the
/// interval's entries, the rows read and the rows the FILTER kept.
#[test]
fn test_two_sided_range_with_a_residual_condition() {
    let (engine, _tmp) = create_range_engine();
    let window = "d.t >= \"2024-06-03\" AND d.t < \"2024-06-20\"";
    let q = format!("FOR d IN C FILTER {window} RETURN d.seq");
    let (window_rows, _, _, _) = run_range(&engine, "plain", &q);
    let q = format!("FOR d IN C FILTER {window} AND d.kind == 3 RETURN d.seq");
    let (expected, _, _, _) = run_range(&engine, "plain", &q);
    assert!(expected.len() * 5 < window_rows.len());
    for coll in ["sparse", "dense"] {
        let (rows, scans, ran, plan) = run_range(&engine, coll, &q);
        assert_eq!(rows, expected, "{coll}");
        assert_eq!(scans, 0, "{coll}");
        assert_eq!(ran.collections[0].estimated_rows, Some(window_rows.len()));
        assert_eq!(plan.collections[0].estimated_rows, Some(window_rows.len()));
        assert_eq!(ran.documents_scanned, window_rows.len(), "{coll}");
        assert_eq!(ran.documents_returned, expected.len(), "{coll}");
        assert_eq!(ran.filters.len(), 1);
        assert_eq!(ran.filters[0].documents_before, window_rows.len());
        assert_eq!(ran.filters[0].documents_after, expected.len());
    }
}

/// `SORT` on the range's field with a LIMIT walks the interval in index
/// order — backwards for DESC — and stops at the LIMIT: ASC and DESC, with
/// an offset, past the end, with a residual condition, a lone bound, LETs
/// after the FILTER and after the LIMIT. Ties come out in key order, as
/// the scan's stable sort leaves them.
#[test]
fn test_range_sort_limit_reads_in_index_order() {
    let (engine, _tmp) = create_range_engine();
    let window = "d.t >= \"2024-06-05\" AND d.t < \"2024-06-12\"";
    let shapes = [
        format!("FOR d IN C FILTER {window} SORT d.t DESC LIMIT 7 RETURN [d.seq, d.t]"),
        format!("FOR d IN C FILTER {window} SORT d.t ASC LIMIT 7 RETURN [d.seq, d.t]"),
        format!("FOR d IN C FILTER {window} SORT d.t DESC LIMIT 4, 9 RETURN d.seq"),
        format!("FOR d IN C FILTER {window} SORT d.t ASC LIMIT 4, 9 RETURN d.seq"),
        format!("FOR d IN C FILTER {window} SORT d.t DESC LIMIT 1000 RETURN d.seq"),
        format!("FOR d IN C FILTER {window} AND d.kind == 3 SORT d.t DESC LIMIT 5 RETURN d.seq"),
        format!("FOR d IN C FILTER d.kind != 2 FILTER {window} SORT d.t ASC LIMIT 6 RETURN d.seq"),
        format!(
            "FOR d IN C FILTER {window} LET s = d.seq * 2 SORT d.t DESC LIMIT 5 \
             LET u = s + 1 RETURN [s, u]"
        ),
        // Bounds of two types, ASC through numbers into strings.
        "FOR d IN C FILTER d.t > 20 AND d.t < \"2024-06-03\" SORT d.t ASC LIMIT 15 \
         RETURN d.seq"
            .to_string(),
        // A lone lower bound that rejects null: newest first.
        "FOR d IN C FILTER d.t >= \"2024-06-20\" AND d.t < \"z\" SORT d.t DESC LIMIT 6 \
         RETURN d.seq"
            .to_string(),
        "FOR d IN C FILTER d.t >= \"2024-06-20\" SORT d.t ASC LIMIT 6 RETURN d.seq".to_string(),
    ];
    for q in &shapes {
        let (expected, _, _, _) = run_range(&engine, "plain", q);
        assert!(!expected.is_empty(), "the seed covers {q}");
        for (coll, index) in [("sparse", "idx_t"), ("dense", "idx_t_dense")] {
            let (rows, scans, ran, plan) = run_range(&engine, coll, q);
            assert_eq!(rows, expected, "{coll}: {q}");
            assert_eq!(scans, 0, "{coll}: {q}");
            for e in [&ran, &plan] {
                assert_eq!(e.collections[0].access_type, "index_sort", "{coll}: {q}");
                assert_eq!(
                    e.collections[0].index_used.as_deref(),
                    Some(index),
                    "{coll}: {q}"
                );
                assert_eq!(e.sort.as_ref().unwrap().index.as_deref(), Some(index));
            }
            assert_eq!(
                ran.collections[0].estimated_rows, plan.collections[0].estimated_rows,
                "{coll}: {q}"
            );
            assert_eq!(ran.documents_returned, expected.len(), "{coll}: {q}");
            assert_eq!(ran.timing.sort_us, 0, "{coll}: {q}");
        }
    }

    // Ties in the page, so their order is part of what is compared.
    let q = format!("FOR d IN C FILTER {window} SORT d.t DESC LIMIT 7 RETURN d.t");
    let (page, _, _, _) = run_range(&engine, "plain", &q);
    assert!(page.windows(2).any(|w| w[0] == w[1]), "{page:?}");

    // The walk stops at the page: 7 rows read, not the window.
    let q = format!("FOR d IN C FILTER {window} SORT d.t DESC LIMIT 7 RETURN d.seq");
    let (_, _, ran, _) = run_range(&engine, "sparse", &q);
    let window_rows = run_range(
        &engine,
        "plain",
        &format!("FOR d IN C FILTER {window} RETURN d.seq"),
    )
    .0
    .len();
    assert_eq!(ran.documents_scanned, 7);
    assert_eq!(ran.collections[0].estimated_rows, Some(window_rows));
    assert_eq!(ran.filters[0].documents_before, 7);
    assert_eq!(ran.filters[0].documents_after, 7);
}

/// Where the walk cannot be trusted the rows are read and sorted as before:
/// arrays and objects inside the walked range (the codec orders them as
/// JSON text), a sparse index and a range that admits null, a sort on
/// another field, two sort keys, an index hint, and a cheaper read of
/// another condition.
#[test]
fn test_range_sort_limit_falls_back() {
    let (engine, _tmp) = create_range_engine();
    let window = "d.t >= \"2024-06-05\" AND d.t < \"2024-06-12\"";
    for (coll, q, walked) in [
        // No upper bound: arrays and objects lie above every string. DESC
        // meets them first and gives up; ASC fills the page before them.
        (
            "sparse",
            "FOR d IN C FILTER d.t >= \"2024-06-20\" SORT d.t DESC LIMIT 6 RETURN d.seq".to_string(),
            false,
        ),
        (
            "sparse",
            "FOR d IN C FILTER d.t >= \"2024-06-20\" SORT d.t ASC LIMIT 6 RETURN d.seq".to_string(),
            true,
        ),
        // `<` alone admits null: not in the sparse index...
        (
            "sparse",
            "FOR d IN C FILTER d.t < \"2024-06-03\" SORT d.t ASC LIMIT 6 RETURN d.seq".to_string(),
            false,
        ),
        // ...but in the non-sparse one, where null sorts first.
        (
            "dense",
            "FOR d IN C FILTER d.t < \"2024-06-03\" SORT d.t ASC LIMIT 6 RETURN d.seq".to_string(),
            true,
        ),
        (
            "dense",
            "FOR d IN C FILTER d.t < \"2024-06-03\" SORT d.t DESC LIMIT 6 RETURN d.seq".to_string(),
            true,
        ),
        (
            "sparse",
            format!("FOR d IN C FILTER {window} SORT d.seq DESC LIMIT 5 RETURN d.seq"),
            false,
        ),
        (
            "sparse",
            format!("FOR d IN C FILTER {window} SORT d.t DESC, d.seq ASC LIMIT 5 RETURN d.seq"),
            false,
        ),
        (
            "sparse",
            format!(
                "FOR d IN C OPTIONS {{ indexHint: \"idx_t\" }} FILTER {window} \
                 SORT d.t DESC LIMIT 5 RETURN d.seq"
            ),
            false,
        ),
        // Four "rare" rows against the window: the equality is read.
        (
            "sparse",
            format!("FOR d IN C FILTER d.grp == \"rare\" AND {window} SORT d.t DESC LIMIT 5 RETURN d.seq"),
            false,
        ),
        // Every row "common": the window is cheaper, and walked.
        (
            "sparse",
            format!("FOR d IN C FILTER d.grp == \"common\" AND {window} SORT d.t DESC LIMIT 5 RETURN d.seq"),
            true,
        ),
    ] {
        let (expected, _, _, _) = run_range(&engine, "plain", &q);
        let (rows, _, ran, plan) = run_range(&engine, coll, &q);
        assert_eq!(rows, expected, "{coll}: {q}");
        assert_eq!(
            ran.collections[0].access_type == "index_sort",
            walked,
            "{coll}: {q}: {:?}",
            ran.collections[0]
        );
        // The plan agrees, except where only the rows tell: the walk that
        // meets an array gives up.
        if walked || !q.contains(">= \"2024-06-20\" SORT d.t DESC") {
            assert_eq!(
                plan.collections[0].access_type == "index_sort",
                walked,
                "plan: {coll}: {q}"
            );
        }
    }
}

/// An index hint naming the single-field index reads the merged interval
/// too, not one of its bounds.
#[test]
fn test_hinted_two_sided_range_reads_the_interval() {
    let (engine, _tmp) = create_range_engine();
    let window = "d.t >= \"2024-06-05\" AND d.t < \"2024-06-12\"";
    let (expected, _, _, _) = run_range(
        &engine,
        "plain",
        &format!("FOR d IN C FILTER {window} RETURN d.seq"),
    );
    let q = format!("FOR d IN C OPTIONS {{ indexHint: \"idx_t\" }} FILTER {window} RETURN d.seq");
    let (rows, scans, ran, _) = run_range(&engine, "sparse", &q);
    assert_eq!(rows, expected);
    assert_eq!(scans, 0);
    assert_eq!(ran.collections[0].index_used.as_deref(), Some("idx_t"));
    assert_eq!(ran.documents_scanned, expected.len());
}
