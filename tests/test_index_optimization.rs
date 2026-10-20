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
