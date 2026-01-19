//! Graph Traversal Tests
//!
//! Comprehensive tests for graph operations including:
//! - Outbound traversals
//! - Inbound traversals
//! - Any direction traversals
//! - Multi-hop traversals
//! - Graph path queries

use serde_json::json;
use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .execute(&query)
        .expect(&format!("Query failed: {}", query_str))
}

fn create_social_graph() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");

    // Create people collection
    engine
        .create_collection("people".to_string(), None)
        .unwrap();
    let people = engine.get_collection("people").unwrap();
    people
        .insert(json!({"_key": "alice", "name": "Alice", "age": 30}))
        .unwrap();
    people
        .insert(json!({"_key": "bob", "name": "Bob", "age": 25}))
        .unwrap();
    people
        .insert(json!({"_key": "charlie", "name": "Charlie", "age": 35}))
        .unwrap();
    people
        .insert(json!({"_key": "diana", "name": "Diana", "age": 28}))
        .unwrap();
    people
        .insert(json!({"_key": "eve", "name": "Eve", "age": 32}))
        .unwrap();

    // Create follows edge collection
    engine
        .create_collection("follows".to_string(), Some("edge".to_string()))
        .unwrap();
    let follows = engine.get_collection("follows").unwrap();
    // Alice follows Bob and Charlie
    follows
        .insert(json!({"_from": "people/alice", "_to": "people/bob", "since": 2020}))
        .unwrap();
    follows
        .insert(json!({"_from": "people/alice", "_to": "people/charlie", "since": 2021}))
        .unwrap();
    // Bob follows Charlie and Diana
    follows
        .insert(json!({"_from": "people/bob", "_to": "people/charlie", "since": 2019}))
        .unwrap();
    follows
        .insert(json!({"_from": "people/bob", "_to": "people/diana", "since": 2022}))
        .unwrap();
    // Charlie follows Eve
    follows
        .insert(json!({"_from": "people/charlie", "_to": "people/eve", "since": 2018}))
        .unwrap();
    // Diana follows Alice (circular)
    follows
        .insert(json!({"_from": "people/diana", "_to": "people/alice", "since": 2023}))
        .unwrap();

    (engine, tmp_dir)
}

// ============================================================================
// Basic Edge Collection Tests
// ============================================================================

#[test]
fn test_edge_collection_exists() {
    let (engine, _tmp) = create_social_graph();

    let results = execute_query(&engine, "FOR e IN follows RETURN e");
    assert_eq!(results.len(), 6, "Should have 6 edges");
}

#[test]
fn test_edge_has_from_and_to() {
    let (engine, _tmp) = create_social_graph();

    let results = execute_query(&engine, "FOR e IN follows RETURN e");
    assert_eq!(results.len(), 6);

    for edge in &results {
        assert!(edge.get("_from").is_some(), "Edge should have _from");
        assert!(edge.get("_to").is_some(), "Edge should have _to");
    }
}

#[test]
fn test_filter_edges_by_from() {
    let (engine, _tmp) = create_social_graph();

    // Find all edges from Alice
    let results = execute_query(
        &engine,
        "FOR e IN follows FILTER e._from == 'people/alice' RETURN e._to",
    );
    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("people/bob")));
    assert!(results.contains(&json!("people/charlie")));
}

#[test]
fn test_filter_edges_by_to() {
    let (engine, _tmp) = create_social_graph();

    // Find all edges to Charlie
    let results = execute_query(
        &engine,
        "FOR e IN follows FILTER e._to == 'people/charlie' RETURN e._from",
    );
    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("people/alice")));
    assert!(results.contains(&json!("people/bob")));
}

// ============================================================================
// Outbound Traversal Tests
// ============================================================================

#[test]
fn test_outbound_depth_1() {
    let (engine, _tmp) = create_social_graph();

    // People that Alice follows (depth 1)
    let results = execute_query(
        &engine,
        "FOR v IN 1..1 OUTBOUND 'people/alice' follows RETURN v.name",
    );
    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Bob")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_outbound_depth_2() {
    let (engine, _tmp) = create_social_graph();

    // People reachable from Alice in 2 hops
    let results = execute_query(
        &engine,
        "FOR v IN 1..2 OUTBOUND 'people/alice' follows RETURN v.name",
    );

    // Alice -> Bob, Charlie (depth 1)
    // Bob -> Charlie, Diana (depth 2)
    // Charlie -> Eve (depth 2)
    // Should have multiple results including duplicates
    assert!(results.len() >= 4);
}

#[test]
fn test_outbound_depth_3() {
    let (engine, _tmp) = create_social_graph();

    // Deep traversal from Alice
    let results = execute_query(
        &engine,
        "FOR v IN 1..3 OUTBOUND 'people/alice' follows RETURN v.name",
    );

    // Should eventually reach all connected nodes (with duplicates)
    assert!(results.len() >= 4);
}

#[test]
fn test_outbound_exact_depth() {
    let (engine, _tmp) = create_social_graph();

    // Only depth 2 (not depth 1)
    let results = execute_query(
        &engine,
        "FOR v IN 2..2 OUTBOUND 'people/alice' follows RETURN v.name",
    );

    // Alice -> Bob -> (Charlie, Diana)
    // Alice -> Charlie -> Eve
    // Depth 2 only: Charlie (via Bob), Diana, Eve
    assert!(results.len() >= 2);
}

// ============================================================================
// Inbound Traversal Tests
// ============================================================================

#[test]
fn test_inbound_depth_1() {
    let (engine, _tmp) = create_social_graph();

    // Who follows Charlie (inbound)
    let results = execute_query(
        &engine,
        "FOR v IN 1..1 INBOUND 'people/charlie' follows RETURN v.name",
    );
    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Bob")));
}

#[test]
fn test_inbound_depth_2() {
    let (engine, _tmp) = create_social_graph();

    // Find followers of followers of Eve
    let results = execute_query(
        &engine,
        "FOR v IN 1..2 INBOUND 'people/eve' follows RETURN v.name",
    );

    // Eve <- Charlie <- (Alice, Bob)
    assert!(results.len() >= 2);
}

// ============================================================================
// Any Direction Traversal Tests
// ============================================================================

#[test]
fn test_any_direction_depth_1() {
    let (engine, _tmp) = create_social_graph();

    // All connections to/from Charlie
    let results = execute_query(
        &engine,
        "FOR v IN 1..1 ANY 'people/charlie' follows RETURN v.name",
    );

    // Charlie -> Eve (outbound)
    // Alice -> Charlie, Bob -> Charlie (inbound)
    assert!(results.len() >= 3);
}

// ============================================================================
// Traversal with Filters
// ============================================================================

#[test]
fn test_traversal_with_vertex_filter() {
    let (engine, _tmp) = create_social_graph();

    // People Alice follows who are older than 30
    let results = execute_query(
        &engine,
        "FOR v IN 1..1 OUTBOUND 'people/alice' follows FILTER v.age > 30 RETURN v.name",
    );

    // Bob is 25, Charlie is 35
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Charlie"));
}

#[test]
fn test_traversal_with_edge_filter() {
    let (engine, _tmp) = create_social_graph();

    // People Alice started following after 2020
    let results = execute_query(
        &engine,
        "FOR v, e IN 1..1 OUTBOUND 'people/alice' follows FILTER e.since > 2020 RETURN v.name",
    );

    // Alice -> Charlie (since 2021)
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Charlie"));
}

// ============================================================================
// Traversal Return Variables
// ============================================================================

#[test]
fn test_traversal_return_vertex() {
    let (engine, _tmp) = create_social_graph();

    let results = execute_query(
        &engine,
        "FOR v IN 1..1 OUTBOUND 'people/alice' follows RETURN v",
    );

    assert_eq!(results.len(), 2);
    for v in &results {
        assert!(v.get("name").is_some());
        assert!(v.get("age").is_some());
    }
}

#[test]
fn test_traversal_return_edge() {
    let (engine, _tmp) = create_social_graph();

    let results = execute_query(
        &engine,
        "FOR v, e IN 1..1 OUTBOUND 'people/alice' follows RETURN e",
    );

    assert_eq!(results.len(), 2);
    for e in &results {
        assert!(e.get("_from").is_some());
        assert!(e.get("_to").is_some());
        assert!(e.get("since").is_some());
    }
}

#[test]
fn test_traversal_return_both() {
    let (engine, _tmp) = create_social_graph();

    let results = execute_query(&engine,
        "FOR v, e IN 1..1 OUTBOUND 'people/alice' follows RETURN { person: v.name, since: e.since }");

    assert_eq!(results.len(), 2);
}

// ============================================================================
// Special Cases
// ============================================================================

#[test]
fn test_traversal_from_nonexistent_vertex() {
    let (engine, _tmp) = create_social_graph();

    // Traversal from non-existent vertex should return empty
    let results = execute_query(
        &engine,
        "FOR v IN 1..1 OUTBOUND 'people/nobody' follows RETURN v",
    );

    assert!(results.is_empty());
}

#[test]
fn test_traversal_no_edges() {
    let (engine, _tmp) = create_social_graph();

    // Eve has no outbound edges
    let results = execute_query(
        &engine,
        "FOR v IN 1..1 OUTBOUND 'people/eve' follows RETURN v",
    );

    assert!(results.is_empty());
}

#[test]
fn test_traversal_count() {
    let (engine, _tmp) = create_social_graph();

    // Count followers
    let results = execute_query(&engine,
        "LET followers = (FOR v IN 1..1 INBOUND 'people/charlie' follows RETURN v) RETURN LENGTH(followers)");

    assert_eq!(results[0], json!(2));
}

// ============================================================================
// Graph Pattern Queries
// ============================================================================

#[test]
fn test_mutual_follows() {
    let (engine, _tmp) = create_social_graph();

    // Find pairs where A follows B AND B follows A
    // Diana follows Alice, but Alice doesn't follow Diana
    let _results = execute_query(
        &engine,
        r#"
        FOR e1 IN follows
            FOR e2 IN follows
                FILTER e1._from == e2._to AND e1._to == e2._from
                RETURN { a: e1._from, b: e1._to }
    "#,
    );

    // In our data, there's no mutual follow relationship
    // (Diana -> Alice exists, but Alice -> Diana doesn't)
}

#[test]
fn test_friends_of_friends() {
    let (engine, _tmp) = create_social_graph();

    // Friends of friends (2nd degree connections)
    let results = execute_query(
        &engine,
        "FOR v IN 2..2 OUTBOUND 'people/alice' follows RETURN v.name",
    );

    // Alice -> Bob -> (Charlie, Diana)
    // Alice -> Charlie -> Eve
    // Depth 2: Diana, Eve, Charlie (via Bob)
    assert!(results.len() >= 2);
}

// ============================================================================
// Graph with Multiple Edge Collections
// ============================================================================

#[test]
fn test_multiple_vertex_collections() {
    let tmp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap();

    // Create two vertex collections
    engine.create_collection("users".to_string(), None).unwrap();
    engine.create_collection("posts".to_string(), None).unwrap();

    let users = engine.get_collection("users").unwrap();
    let posts = engine.get_collection("posts").unwrap();

    users
        .insert(json!({"_key": "alice", "name": "Alice"}))
        .unwrap();
    posts
        .insert(json!({"_key": "post1", "title": "Hello World"}))
        .unwrap();
    posts
        .insert(json!({"_key": "post2", "title": "Second Post"}))
        .unwrap();

    // Create authored edge
    engine
        .create_collection("authored".to_string(), Some("edge".to_string()))
        .unwrap();
    let authored = engine.get_collection("authored").unwrap();
    authored
        .insert(json!({"_from": "users/alice", "_to": "posts/post1"}))
        .unwrap();
    authored
        .insert(json!({"_from": "users/alice", "_to": "posts/post2"}))
        .unwrap();

    // Find posts by Alice
    let results = execute_query(
        &engine,
        "FOR p IN 1..1 OUTBOUND 'users/alice' authored RETURN p.title",
    );
    assert_eq!(results.len(), 2);
}

// ============================================================================
// Traversal Options
// ============================================================================

#[test]
fn test_traversal_sorted() {
    let (engine, _tmp) = create_social_graph();

    let results = execute_query(
        &engine,
        "FOR v IN 1..1 OUTBOUND 'people/alice' follows SORT v.age DESC RETURN v.name",
    );

    assert_eq!(results.len(), 2);
    // Charlie (35) should come before Bob (25)
    assert_eq!(results[0], json!("Charlie"));
    assert_eq!(results[1], json!("Bob"));
}

#[test]
fn test_traversal_limited() {
    let (engine, _tmp) = create_social_graph();

    let results = execute_query(
        &engine,
        "FOR v IN 1..2 OUTBOUND 'people/alice' follows LIMIT 3 RETURN v.name",
    );

    assert!(results.len() <= 3);
}
