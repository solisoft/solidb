//! Tests for newly added SDBQL syntax:
//! - Set operations: UNION [ALL] / INTERSECT / EXCEPT
//! - WITH RECURSIVE (recursive CTEs)
//! - RETURN DISTINCT
//! - COLLECT ... INTO ... KEEP
//! - NONE quantifier (`NONE x IN arr SATISFIES cond` and `NONE(arr, x -> cond)`)
//! - Standalone OFFSET clause (with and without LIMIT)
//!
//! Regression tests for the bugs found reviewing the above live at the bottom.

use serde_json::json;
use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<serde_json::Value> {
    let query = parse(query_str).unwrap_or_else(|_| panic!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .execute(&query)
        .unwrap_or_else(|_| panic!("Query failed: {}", query_str))
}

fn create_seeded_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    for doc in [
        json!({"_key": "alice", "name": "Alice", "age": 30, "city": "Paris"}),
        json!({"_key": "bob", "name": "Bob", "age": 25, "city": "London"}),
        json!({"_key": "carol", "name": "Carol", "age": 35, "city": "Paris"}),
        json!({"_key": "dave", "name": "Dave", "age": 28, "city": "Berlin"}),
        json!({"_key": "eve", "name": "Eve", "age": 32, "city": "London"}),
    ] {
        users.insert(doc).unwrap();
    }

    // Org chart: alice manages bob; bob manages carol and dave
    engine
        .create_collection("employees".to_string(), None)
        .unwrap();
    let employees = engine.get_collection("employees").unwrap();
    for doc in [
        json!({"_key": "alice", "manager": null}),
        json!({"_key": "bob", "manager": "alice"}),
        json!({"_key": "carol", "manager": "bob"}),
        json!({"_key": "dave", "manager": "bob"}),
        json!({"_key": "zoe", "manager": null}),
    ] {
        employees.insert(doc).unwrap();
    }

    (engine, tmp_dir)
}

fn keys(mut results: Vec<serde_json::Value>) -> Vec<String> {
    results.sort_by_key(|v| v.to_string());
    results
        .into_iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

// ============================================================================
// UNION / INTERSECT / EXCEPT
// ============================================================================

#[test]
fn test_union_dedupes() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR u IN users FILTER u.age < 26 RETURN u._key \
         UNION FOR u IN users FILTER u.city == 'London' RETURN u._key",
    );
    // {bob} U {bob, eve} = {bob, eve}
    assert_eq!(keys(results), vec!["bob".to_string(), "eve".to_string()]);
}

#[test]
fn test_union_all_keeps_duplicates() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR u IN users FILTER u.age < 26 RETURN u._key \
         UNION ALL FOR u IN users FILTER u.city == 'London' RETURN u._key",
    );
    assert_eq!(
        keys(results),
        vec!["bob".to_string(), "bob".to_string(), "eve".to_string()]
    );
}

#[test]
fn test_intersect() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR u IN users FILTER u.age < 26 RETURN u._key \
         INTERSECT FOR u IN users FILTER u.city == 'London' RETURN u._key",
    );
    assert_eq!(keys(results), vec!["bob".to_string()]);
}

#[test]
fn test_except() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR u IN users FILTER u.city == 'Paris' OR u.city == 'London' RETURN u._key \
         EXCEPT FOR u IN users FILTER u.city == 'London' RETURN u._key",
    );
    assert_eq!(
        keys(results),
        vec!["alice".to_string(), "carol".to_string()]
    );
}

#[test]
fn test_union_parenthesized_operand() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR u IN users FILTER u.age < 26 RETURN u._key \
         UNION (FOR u IN users FILTER u.city == 'Berlin' RETURN u._key)",
    );
    assert_eq!(keys(results), vec!["bob".to_string(), "dave".to_string()]);
}

// ============================================================================
// WITH RECURSIVE
// ============================================================================

#[test]
fn test_recursive_cte_hierarchy() {
    let (engine, _tmp) = create_seeded_engine();

    // Everyone in alice's reporting chain, found level by level
    let results = execute_query(
        &engine,
        "WITH RECURSIVE reports AS (\
             FOR e IN employees FILTER e._key == 'alice' RETURN e._key \
             UNION ALL \
             FOR m IN employees FILTER m.manager IN reports RETURN m._key \
         ) FOR x IN reports RETURN x",
    );
    assert_eq!(
        keys(results),
        vec![
            "alice".to_string(),
            "bob".to_string(),
            "carol".to_string(),
            "dave".to_string()
        ]
    );
}

#[test]
fn test_recursive_cte_requires_union_all() {
    let (engine, _tmp) = create_seeded_engine();

    let query = parse("WITH RECURSIVE t AS (FOR e IN employees RETURN e._key) FOR x IN t RETURN x")
        .unwrap();
    let executor = QueryExecutor::new(&engine);
    let err = executor.execute(&query).unwrap_err().to_string();
    assert!(err.contains("UNION ALL"), "unexpected error: {}", err);
}

// ============================================================================
// RETURN DISTINCT
// ============================================================================

#[test]
fn test_return_distinct() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(&engine, "FOR u IN users RETURN DISTINCT u.city");
    assert_eq!(
        keys(results),
        vec![
            "Berlin".to_string(),
            "London".to_string(),
            "Paris".to_string()
        ]
    );

    // Without DISTINCT all five rows come back
    let results = execute_query(&engine, "FOR u IN users RETURN u.city");
    assert_eq!(results.len(), 5);
}

#[test]
fn test_return_distinct_with_sort_limit() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR u IN users SORT u.age DESC LIMIT 4 RETURN DISTINCT u.city",
    );
    // ages desc: carol(35), eve(32), alice(30), dave(28) => Paris, London, Paris, Berlin
    assert_eq!(
        keys(results),
        vec![
            "Berlin".to_string(),
            "London".to_string(),
            "Paris".to_string()
        ]
    );
}

// ============================================================================
// COLLECT ... KEEP
// ============================================================================

#[test]
fn test_collect_keep_restricts_variables() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR u IN users LET n = u.name \
         COLLECT city = u.city INTO g KEEP n SORT city LIMIT 1 \
         RETURN {city: city, names: g[*].n}",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["city"], json!("Berlin"));
    assert_eq!(results[0]["names"], json!(["Dave"]));

    // The group items must not contain other variables (u, n is kept)
    let raw = execute_query(
        &engine,
        "FOR u IN users LET n = u.name \
         COLLECT city = u.city INTO g KEEP n SORT city LIMIT 1 \
         RETURN FIRST(g)",
    );
    let item = raw[0].as_object().unwrap();
    assert!(
        !item.contains_key("u") && !item.contains_key("doc"),
        "KEEP should restrict stored variables: {:?}",
        item.keys().collect::<Vec<_>>()
    );
}

// ============================================================================
// NONE quantifier
// ============================================================================

#[test]
fn test_none_quantifier_satisfies() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(&engine, "RETURN NONE(x IN [1, 2, 3] SATISFIES x > 5)");
    assert_eq!(results[0], json!(true));

    let results = execute_query(&engine, "RETURN NONE(x IN [1, 2, 3] SATISFIES x > 2)");
    assert_eq!(results[0], json!(false));

    // Function form still works
    let results = execute_query(&engine, "RETURN NONE([1, 2, 3], x -> x > 5)");
    assert_eq!(results[0], json!(true));
}

#[test]
fn test_none_in_filter() {
    let (engine, _tmp) = create_seeded_engine();

    // Cities with no resident younger than 26
    let results = execute_query(
        &engine,
        "FOR c IN ['Paris', 'London', 'Berlin'] \
         FILTER NONE(u IN (FOR p IN users FILTER p.city == c RETURN p.age) SATISFIES u < 26) \
         SORT c \
         RETURN c",
    );
    assert_eq!(
        keys(results),
        vec!["Berlin".to_string(), "Paris".to_string()]
    );
}

// ============================================================================
// OFFSET
// ============================================================================

#[test]
fn test_limit_offset() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR u IN users SORT u.age ASC LIMIT 2 OFFSET 1 RETURN u._key",
    );
    // ages asc: bob(25), dave(28), alice(30), eve(32), carol(35); skip bob take 2
    assert_eq!(keys(results), vec!["alice".to_string(), "dave".to_string()]);
}

#[test]
fn test_standalone_offset_without_limit() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR u IN users SORT u.age ASC OFFSET 3 RETURN u._key",
    );
    assert_eq!(keys(results), vec!["carol".to_string(), "eve".to_string()]);
}

// ============================================================================
// Regressions
// ============================================================================

/// A standalone OFFSET used to reach storage as a sentinel maximum count,
/// which `scan_values_range` turned into `Vec::with_capacity(i64::MAX)`.
/// Every shape below skips the SORT fast path, so they all hit that scan.
#[test]
fn test_standalone_offset_without_sort() {
    let (engine, _tmp) = create_seeded_engine();

    // Direct-scan fast path: FOR ... OFFSET n RETURN var
    let results = execute_query(&engine, "FOR u IN users OFFSET 2 RETURN u");
    assert_eq!(results.len(), 3);

    // Projection path
    let results = execute_query(&engine, "FOR u IN users OFFSET 2 RETURN u._key");
    assert_eq!(results.len(), 3);

    // Offset past the end is empty, not an error
    let results = execute_query(&engine, "FOR u IN users OFFSET 99 RETURN u._key");
    assert!(results.is_empty());

    // Inside a subquery
    let results = execute_query(
        &engine,
        "LET rest = (FOR u IN users OFFSET 4 RETURN u._key) RETURN LENGTH(rest)",
    );
    assert_eq!(results[0], json!(1));

    // Inside a set-operation operand
    let results = execute_query(
        &engine,
        "FOR u IN users FILTER u.city == 'Berlin' RETURN u._key \
         UNION ALL FOR v IN users OFFSET 4 RETURN v._key",
    );
    assert_eq!(results.len(), 2);

    // With a FILTER (index/filter pushdown path)
    let results = execute_query(
        &engine,
        "FOR u IN users FILTER u.city == 'Paris' OFFSET 1 RETURN u._key",
    );
    assert_eq!(results.len(), 1);
}

/// UNION only deduplicated the incoming side, so duplicates already present
/// in the left operand survived.
#[test]
fn test_union_dedupes_left_side_too() {
    let (engine, _tmp) = create_seeded_engine();

    // Paris and London each have two users
    let results = execute_query(
        &engine,
        "FOR u IN users RETURN u.city \
         UNION FOR v IN users FILTER v.city == 'Rome' RETURN v.city",
    );
    assert_eq!(
        keys(results),
        vec![
            "Berlin".to_string(),
            "London".to_string(),
            "Paris".to_string()
        ]
    );

    // ... and both sides at once
    let results = execute_query(
        &engine,
        "FOR u IN users RETURN u.city UNION FOR v IN users RETURN v.city",
    );
    assert_eq!(results.len(), 3);

    // UNION ALL still keeps every row
    let results = execute_query(
        &engine,
        "FOR u IN users RETURN u.city UNION ALL FOR v IN users RETURN v.city",
    );
    assert_eq!(results.len(), 10);
}

/// Set operations compare rows by value, like the UNION()/INTERSECTION()
/// array builtins — 1 and 1.0 are the same row.
#[test]
fn test_set_operations_use_value_equality() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(&engine, "RETURN 1 UNION RETURN 1.0");
    assert_eq!(results.len(), 1);

    let results = execute_query(&engine, "RETURN 1 INTERSECT RETURN 1.0");
    assert_eq!(results.len(), 1);
}

/// A recursive CTE body is a full query block: its pre-FOR LETs used to be
/// dropped, which silently produced an empty result instead of the hierarchy.
#[test]
fn test_recursive_cte_body_honours_let() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "WITH RECURSIVE reports AS (\
             LET root = 'alice' \
             FOR e IN employees FILTER e._key == root RETURN e._key \
             UNION ALL \
             LET previous = reports \
             FOR m IN employees FILTER m.manager IN previous RETURN m._key \
         ) FOR x IN reports RETURN x",
    );
    assert_eq!(
        keys(results),
        vec![
            "alice".to_string(),
            "bob".to_string(),
            "carol".to_string(),
            "dave".to_string()
        ]
    );
}

/// A CTE declared before a set operation binds for every operand, not just
/// the left one (it used to fail with "Collection 'x' not found").
#[test]
fn test_cte_visible_in_set_operation_operands() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "WITH parisians AS (FOR u IN users FILTER u.city == 'Paris' RETURN u._key) \
         FOR a IN parisians FILTER a == 'alice' RETURN a \
         UNION FOR b IN parisians FILTER b == 'carol' RETURN b",
    );
    assert_eq!(
        keys(results),
        vec!["alice".to_string(), "carol".to_string()]
    );
}

/// Non-recursive CTE bodies went through a reduced pipeline that ignored
/// SORT/LIMIT and set operations.
#[test]
fn test_cte_body_honours_sort_limit_and_set_operations() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "WITH oldest AS (FOR u IN users SORT u.age DESC LIMIT 2 RETURN u._key) \
         FOR x IN oldest RETURN x",
    );
    assert_eq!(results, vec![json!("carol"), json!("eve")]);

    let results = execute_query(
        &engine,
        "WITH both AS (\
             FOR u IN users FILTER u.city == 'Berlin' RETURN u._key \
             UNION FOR v IN users FILTER v.city == 'London' RETURN v._key\
         ) FOR x IN both RETURN x",
    );
    assert_eq!(
        keys(results),
        vec!["bob".to_string(), "dave".to_string(), "eve".to_string()]
    );
}

/// Set operations inside a subquery used to be dropped silently.
#[test]
fn test_set_operation_in_subquery() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "LET combined = (\
             FOR u IN users FILTER u.city == 'Berlin' RETURN u._key \
             UNION FOR v IN users FILTER v.city == 'London' RETURN v._key\
         ) RETURN LENGTH(combined)",
    );
    assert_eq!(results[0], json!(3));
}

/// Chained set operators follow SQL precedence: INTERSECT binds tighter than
/// UNION / EXCEPT, and same-precedence operators chain left to right. A bare
/// operand used to swallow the next operator, which nested chains to the right
/// and made `a EXCEPT b EXCEPT c` mean `a EXCEPT (b EXCEPT c)`.
#[test]
fn test_set_operation_precedence() {
    let (engine, _tmp) = create_seeded_engine();

    // Berlin UNION (London INTERSECT London) = {dave} U {bob, eve}
    let results = execute_query(
        &engine,
        "FOR u IN users FILTER u.city == 'Berlin' RETURN u._key \
         UNION FOR v IN users FILTER v.city == 'London' RETURN v._key \
         INTERSECT FOR w IN users FILTER w.city == 'London' RETURN w._key",
    );
    assert_eq!(
        keys(results),
        vec!["bob".to_string(), "dave".to_string(), "eve".to_string()]
    );

    // ((all EXCEPT Paris) EXCEPT London) = {dave}, not all EXCEPT (Paris EXCEPT London)
    let results = execute_query(
        &engine,
        "FOR u IN users RETURN u._key \
         EXCEPT FOR v IN users FILTER v.city == 'Paris' RETURN v._key \
         EXCEPT FOR w IN users FILTER w.city == 'London' RETURN w._key",
    );
    assert_eq!(keys(results), vec!["dave".to_string()]);

    // Parentheses override: Berlin UNION (London INTERSECT Paris) = {dave}
    let results = execute_query(
        &engine,
        "FOR u IN users FILTER u.city == 'Berlin' RETURN u._key \
         UNION (FOR v IN users FILTER v.city == 'London' RETURN v._key \
                INTERSECT FOR w IN users FILTER w.city == 'Paris' RETURN w._key)",
    );
    assert_eq!(keys(results), vec!["dave".to_string()]);

    // (Paris UNION London) EXCEPT London = Paris
    let results = execute_query(
        &engine,
        "(FOR u IN users FILTER u.city == 'Paris' RETURN u._key \
           UNION FOR v IN users FILTER v.city == 'London' RETURN v._key) \
         EXCEPT FOR w IN users FILTER w.city == 'London' RETURN w._key",
    );
    assert_eq!(
        keys(results),
        vec!["alice".to_string(), "carol".to_string()]
    );
}

/// KEEP naming a variable that is not in scope used to store `{}` per item.
#[test]
fn test_collect_keep_unknown_variable_errors() {
    let (engine, _tmp) = create_seeded_engine();

    let query = parse("FOR u IN users COLLECT city = u.city INTO g KEEP nosuch RETURN g").unwrap();
    let executor = QueryExecutor::new(&engine);
    let err = executor.execute(&query).unwrap_err().to_string();
    assert!(err.contains("nosuch"), "unexpected error: {}", err);
}
