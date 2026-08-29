//! SDBQL Query Execution Tests
//!
//! Tests for executing SDBQL queries against real collections.
//! Covers:
//! - FOR/IN queries
//! - INSERT/UPDATE/REMOVE operations
//! - FILTER with various operators
//! - SORT and LIMIT
//! - Indexes and index usage
//! - Graph traversals

use serde_json::json;
use solidb::storage::{IndexType, StorageEngine};
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

/// Helper to execute a query
fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<serde_json::Value> {
    let query = parse(query_str).unwrap_or_else(|_| panic!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .execute(&query)
        .unwrap_or_else(|_| panic!("Query failed: {}", query_str))
}

/// Helper to execute a query and expect an error
#[allow(dead_code)]
fn execute_query_expect_err(engine: &StorageEngine, query_str: &str) -> String {
    let query = parse(query_str).unwrap_or_else(|_| panic!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor.execute(&query).unwrap_err().to_string()
}

/// Create a test engine with some seed data
fn create_seeded_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");

    // Create users collection with sample data
    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    users
        .insert(
            json!({"_key": "alice", "name": "Alice", "age": 30, "city": "Paris", "active": true}),
        )
        .unwrap();
    users
        .insert(json!({"_key": "bob", "name": "Bob", "age": 25, "city": "London", "active": true}))
        .unwrap();
    users.insert(json!({"_key": "charlie", "name": "Charlie", "age": 35, "city": "Paris", "active": false})).unwrap();
    users
        .insert(
            json!({"_key": "diana", "name": "Diana", "age": 28, "city": "Berlin", "active": true}),
        )
        .unwrap();
    users
        .insert(json!({"_key": "eve", "name": "Eve", "age": 32, "city": "London", "active": true}))
        .unwrap();

    // Create products collection
    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let products = engine.get_collection("products").unwrap();
    products
        .insert(json!({"_key": "p1", "name": "Laptop", "price": 999.99, "category": "electronics"}))
        .unwrap();
    products
        .insert(json!({"_key": "p2", "name": "Phone", "price": 599.99, "category": "electronics"}))
        .unwrap();
    products
        .insert(json!({"_key": "p3", "name": "Book", "price": 19.99, "category": "books"}))
        .unwrap();
    products
        .insert(
            json!({"_key": "p4", "name": "Headphones", "price": 149.99, "category": "electronics"}),
        )
        .unwrap();

    (engine, tmp_dir)
}

// ============================================================================
// Basic FOR/IN Queries
// ============================================================================

#[test]
fn test_for_return_all() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(&engine, "FOR doc IN users RETURN doc");
    assert_eq!(results.len(), 5, "Should return all 5 users");
}

#[test]
fn test_for_return_specific_field() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(&engine, "FOR doc IN users RETURN doc.name");
    assert_eq!(results.len(), 5);

    let names: Vec<&str> = results.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"Alice"));
    assert!(names.contains(&"Bob"));
}

#[test]
fn test_for_return_object() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR doc IN users RETURN { name: doc.name, age: doc.age }",
    );
    assert_eq!(results.len(), 5);

    let first = results
        .iter()
        .find(|v| v.get("name") == Some(&json!("Alice")))
        .unwrap();
    assert_eq!(first.get("age"), Some(&json!(30)));
}

// ============================================================================
// FILTER Operations
// ============================================================================

#[test]
fn test_filter_equals() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.city == 'Paris' RETURN doc.name",
    );
    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_filter_not_equals() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.city != 'Paris' RETURN doc.name",
    );
    assert_eq!(results.len(), 3);
}

#[test]
fn test_filter_greater_than() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.age > 30 RETURN doc.name",
    );
    assert_eq!(results.len(), 2); // Charlie (35), Eve (32)
}

#[test]
fn test_filter_less_than_or_equal() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.age <= 28 RETURN doc.name",
    );
    assert_eq!(results.len(), 2); // Bob (25), Diana (28)
}

#[test]
fn test_filter_boolean() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.active == true RETURN doc.name",
    );
    assert_eq!(results.len(), 4); // All except Charlie
}

#[test]
fn test_filter_and() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.city == 'Paris' AND doc.active == true RETURN doc.name",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));
}

#[test]
fn test_filter_or() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.city == 'Paris' OR doc.city == 'Berlin' RETURN doc.name",
    );
    assert_eq!(results.len(), 3); // Alice, Charlie, Diana
}

#[test]
fn test_filter_like() {
    let (engine, _tmp) = create_seeded_engine();

    // LIKE with % wildcard
    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.name LIKE 'A%' RETURN doc.name",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));
}

#[test]
fn test_filter_in_array() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.city IN ['Paris', 'Berlin'] RETURN doc.name",
    );
    assert_eq!(results.len(), 3);
}

#[test]
fn test_key_fast_path_point_lookup() {
    let (engine, _tmp) = create_seeded_engine();

    // Hit: existing key resolves to exactly one document via the primary-key
    // fast-path (no secondary index on `_key` exists).
    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc._key == 'alice' RETURN doc",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name"), Some(&json!("Alice")));

    // Hit with LIMIT 1: still returns the document.
    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc._key == 'alice' LIMIT 1 RETURN doc",
    );
    assert_eq!(results.len(), 1);

    // Miss: non-existent key returns an empty result (not an error).
    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc._key == 'zzz_nonexistent' RETURN doc",
    );
    assert_eq!(results.len(), 0);

    // Non-string literal: cannot match any document key (keys are strings).
    let results = execute_query(&engine, "FOR doc IN users FILTER doc._key == 42 RETURN doc");
    assert_eq!(results.len(), 0);
}

// ============================================================================
// SORT Operations
// ============================================================================

#[test]
fn test_sort_ascending() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(&engine, "FOR doc IN users SORT doc.age ASC RETURN doc.name");
    assert_eq!(results.len(), 5);
    assert_eq!(results[0], json!("Bob")); // Age 25 (youngest)
    assert_eq!(results[4], json!("Charlie")); // Age 35 (oldest)
}

#[test]
fn test_sort_descending() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR doc IN users SORT doc.age DESC RETURN doc.name",
    );
    assert_eq!(results.len(), 5);
    assert_eq!(results[0], json!("Charlie")); // Age 35 (oldest)
    assert_eq!(results[4], json!("Bob")); // Age 25 (youngest)
}

#[test]
fn test_sort_multiple_fields() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(&engine,
        "FOR doc IN users SORT doc.city ASC, doc.age DESC RETURN { city: doc.city, name: doc.name }");
    assert_eq!(results.len(), 5);

    // First Berlin (Diana), then London (Eve, Bob), then Paris (Charlie, Alice)
    assert_eq!(results[0].get("city"), Some(&json!("Berlin")));
}

// ============================================================================
// LIMIT Operations
// ============================================================================

#[test]
fn test_limit_count() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(&engine, "FOR doc IN users LIMIT 3 RETURN doc.name");
    assert_eq!(results.len(), 3);
}

#[test]
fn test_limit_offset_count() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR doc IN users SORT doc.age ASC LIMIT 1, 2 RETURN doc.name",
    );
    assert_eq!(results.len(), 2);
    // Skip Bob (youngest), get next 2
}

#[test]
fn test_sort_and_limit() {
    let (engine, _tmp) = create_seeded_engine();

    // Top 3 oldest users
    let results = execute_query(
        &engine,
        "FOR doc IN users SORT doc.age DESC LIMIT 3 RETURN doc.name",
    );
    assert_eq!(results.len(), 3);
    assert_eq!(results[0], json!("Charlie")); // 35
    assert_eq!(results[1], json!("Eve")); // 32
    assert_eq!(results[2], json!("Alice")); // 30
}

// ============================================================================
// LET Bindings
// ============================================================================

#[test]
fn test_let_simple() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "LET threshold = 30 FOR doc IN users FILTER doc.age >= threshold RETURN doc.name",
    );
    assert_eq!(results.len(), 3); // Alice (30), Charlie (35), Eve (32)
}

#[test]
fn test_let_computed() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR doc IN users LET fullInfo = CONCAT(doc.name, ' from ', doc.city) RETURN fullInfo",
    );
    assert_eq!(results.len(), 5);
    assert!(results.contains(&json!("Alice from Paris")));
}

// ============================================================================
// INSERT Operations
// ============================================================================

#[test]
fn test_insert_new_document() {
    let (engine, _tmp) = create_seeded_engine();

    // Insert new user
    execute_query(
        &engine,
        "INSERT { _key: 'frank', name: 'Frank', age: 40, city: 'Tokyo', active: true } INTO users",
    );

    // Verify insertion
    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc._key == 'frank' RETURN doc",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name"), Some(&json!("Frank")));
}

#[test]
fn test_insert_with_return() {
    let (engine, _tmp) = create_seeded_engine();

    // Insert and then query to verify (RETURN NEW may not be supported)
    execute_query(
        &engine,
        "INSERT { _key: 'grace', name: 'Grace', age: 29 } INTO users",
    );

    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc._key == 'grace' RETURN doc",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name"), Some(&json!("Grace")));
}

// ============================================================================
// UPDATE Operations
// ============================================================================

#[test]
fn test_update_document() {
    let (engine, _tmp) = create_seeded_engine();

    // Update Alice's age
    execute_query(&engine, "UPDATE 'alice' WITH { age: 31 } IN users");

    // Verify update
    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc._key == 'alice' RETURN doc.age",
    );
    assert_eq!(results[0], json!(31));
}

#[test]
fn test_update_with_expression() {
    let (engine, _tmp) = create_seeded_engine();

    // Update all Paris users to active = false
    execute_query(
        &engine,
        "FOR doc IN users FILTER doc.city == 'Paris' UPDATE doc WITH { active: false } IN users",
    );

    // Verify
    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.city == 'Paris' AND doc.active == true RETURN doc",
    );
    assert_eq!(results.len(), 0);
}

// ============================================================================
// REMOVE Operations
// ============================================================================

#[test]
fn test_remove_document() {
    let (engine, _tmp) = create_seeded_engine();

    // Remove Bob
    execute_query(&engine, "REMOVE 'bob' IN users");

    // Verify removal
    let results = execute_query(&engine, "FOR doc IN users RETURN doc.name");
    assert_eq!(results.len(), 4);
    assert!(!results.contains(&json!("Bob")));
}

// ============================================================================
// Aggregation Queries
// ============================================================================

#[test]
fn test_aggregate_count() {
    let (engine, _tmp) = create_seeded_engine();

    // Collection cardinality via a subquery (LENGTH on a string is char count).
    let results = execute_query(&engine, "RETURN LENGTH((FOR doc IN users RETURN 1))");
    assert_eq!(results[0], json!(5));
}

#[test]
fn test_aggregate_in_subquery() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "LET ages = (FOR doc IN users RETURN doc.age) RETURN AVG(ages)",
    );
    assert_eq!(results.len(), 1);
    // Average of 30, 25, 35, 28, 32 = 30
    assert_eq!(results[0], json!(30.0));
}

// ============================================================================
// Complex Queries
// ============================================================================

#[test]
fn test_complex_query_with_all_clauses() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        r#"
        LET minAge = 25
        FOR doc IN users
            FILTER doc.age >= minAge AND doc.active == true
            SORT doc.age DESC
            LIMIT 3
            RETURN { name: doc.name, age: doc.age, city: doc.city }
    "#,
    );

    assert!(results.len() <= 3);
    // All results should have age >= 25 and active = true
    for result in &results {
        let age = result.get("age").and_then(|v| v.as_i64()).unwrap();
        assert!(age >= 25);
    }
}

#[test]
fn test_nested_field_access() {
    let tmp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap();

    // Create collection with nested data
    engine
        .create_collection("profiles".to_string(), None)
        .unwrap();
    let profiles = engine.get_collection("profiles").unwrap();
    profiles
        .insert(json!({
            "_key": "p1",
            "user": {
                "name": "Alice",
                "address": {
                    "city": "Paris",
                    "country": "France"
                }
            }
        }))
        .unwrap();

    let results = execute_query(&engine, "FOR p IN profiles RETURN p.user.address.city");
    assert_eq!(results[0], json!("Paris"));
}

#[test]
fn test_array_in_document() {
    let tmp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap();

    engine
        .create_collection("orders".to_string(), None)
        .unwrap();
    let orders = engine.get_collection("orders").unwrap();
    orders
        .insert(json!({
            "_key": "o1",
            "items": ["apple", "banana", "cherry"],
            "quantities": [5, 3, 7]
        }))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR o IN orders RETURN { first_item: o.items[0], total_items: LENGTH(o.items) }",
    );
    assert_eq!(results[0].get("first_item"), Some(&json!("apple")));
    assert_eq!(results[0].get("total_items"), Some(&json!(3)));
}

// ============================================================================
// Index Tests
// ============================================================================

#[test]
fn test_create_and_use_index() {
    let (engine, _tmp) = create_seeded_engine();

    // Create persistent index on city field
    let users = engine.get_collection("users").unwrap();
    users
        .create_index(
            "city_idx".to_string(),
            vec!["city".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();

    // Query that could use the index
    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.city == 'Paris' RETURN doc.name",
    );
    assert_eq!(results.len(), 2);
}

/// Regression: with an index on the equality field, `FILTER a == x AND b != y`
/// must still apply the second conjunct. Previously the index path consumed
/// the whole FILTER clause and silently dropped any non-indexed conjuncts,
/// so uniqueness checks of the form `email == @val AND _key != @key`
/// matched the very record they were trying to exclude.
#[test]
fn test_filter_and_with_indexed_field_applies_all_conjuncts() {
    let (engine, _tmp) = create_seeded_engine();

    let users = engine.get_collection("users").unwrap();
    users
        .create_index(
            "city_idx".to_string(),
            vec!["city".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();

    // Paris has two users: alice and charlie. Excluding alice via `_key != ...`
    // must return only charlie.
    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.city == 'Paris' AND doc._key != 'alice' RETURN doc.name",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Charlie"));

    // Symmetric case: both conjuncts on indexed-ish field paths still narrow
    // correctly when the second one excludes the matched record entirely.
    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.city == 'Paris' AND doc._key == 'alice' RETURN doc.name",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));

    // Three-conjunct chain: `city == 'Paris' AND active == true AND age > 25`
    // — only Alice matches (Charlie is inactive).
    let results = execute_query(
        &engine,
        "FOR doc IN users FILTER doc.city == 'Paris' AND doc.active == true AND doc.age > 25 RETURN doc.name",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));
}

#[test]
fn test_unique_index_creation() {
    let tmp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap();

    engine
        .create_collection("emails".to_string(), None)
        .unwrap();
    let emails = engine.get_collection("emails").unwrap();

    // Create unique index
    let result = emails.create_index(
        "email_unique".to_string(),
        vec!["email".to_string()],
        IndexType::Hash,
        true,
    );
    assert!(
        result.is_ok(),
        "Should create unique index: {:?}",
        result.err()
    );

    // Insert first document
    emails
        .insert(json!({"_key": "u1", "email": "test@example.com"}))
        .unwrap();

    // Verify it exists
    let doc = emails.get("u1").unwrap();
    assert_eq!(doc.get("email"), Some(json!("test@example.com")));
}

// ============================================================================
// Edge Collection and Graph Queries
// ============================================================================

#[test]
fn test_edge_collection() {
    let tmp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap();

    // Create vertex collection
    engine
        .create_collection("people".to_string(), None)
        .unwrap();
    let people = engine.get_collection("people").unwrap();
    people
        .insert(json!({"_key": "alice", "name": "Alice"}))
        .unwrap();
    people
        .insert(json!({"_key": "bob", "name": "Bob"}))
        .unwrap();
    people
        .insert(json!({"_key": "charlie", "name": "Charlie"}))
        .unwrap();

    // Create edge collection
    engine
        .create_collection("knows".to_string(), Some("edge".to_string()))
        .unwrap();
    let knows = engine.get_collection("knows").unwrap();
    knows
        .insert(json!({"_from": "people/alice", "_to": "people/bob", "since": 2020}))
        .unwrap();
    knows
        .insert(json!({"_from": "people/bob", "_to": "people/charlie", "since": 2021}))
        .unwrap();

    // Query edges
    let results = execute_query(&engine, "FOR e IN knows RETURN e");
    assert_eq!(results.len(), 2);
}

#[test]
fn test_graph_outbound_traversal() {
    let tmp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap();

    // Setup graph
    engine
        .create_collection("vertices".to_string(), None)
        .unwrap();
    let vertices = engine.get_collection("vertices").unwrap();
    vertices.insert(json!({"_key": "a", "name": "A"})).unwrap();
    vertices.insert(json!({"_key": "b", "name": "B"})).unwrap();
    vertices.insert(json!({"_key": "c", "name": "C"})).unwrap();

    engine
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let edges = engine.get_collection("edges").unwrap();
    edges
        .insert(json!({"_from": "vertices/a", "_to": "vertices/b"}))
        .unwrap();
    edges
        .insert(json!({"_from": "vertices/a", "_to": "vertices/c"}))
        .unwrap();

    // Traverse outbound from A
    let results = execute_query(
        &engine,
        "FOR v IN 1..1 OUTBOUND 'vertices/a' edges RETURN v.name",
    );
    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("B")));
    assert!(results.contains(&json!("C")));
}

#[test]
fn test_pagerank_and_degree_centrality() {
    let tmp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap();

    // Setup directed graph for testing analytics
    // a -> b (1), a -> c (1), b -> c (2)   (directed)
    engine.create_collection("nodes".to_string(), None).unwrap();
    let nodes = engine.get_collection("nodes").unwrap();
    nodes.insert(json!({"_key": "a"})).unwrap();
    nodes.insert(json!({"_key": "b"})).unwrap();
    nodes.insert(json!({"_key": "c"})).unwrap();

    engine
        .create_collection("links".to_string(), Some("edge".to_string()))
        .unwrap();
    let links = engine.get_collection("links").unwrap();
    links
        .insert(json!({"_from": "nodes/a", "_to": "nodes/b", "w": 1}))
        .unwrap();
    links
        .insert(json!({"_from": "nodes/a", "_to": "nodes/c", "w": 1}))
        .unwrap();
    links
        .insert(json!({"_from": "nodes/b", "_to": "nodes/c", "w": 2}))
        .unwrap();

    let deg_results = execute_query(&engine, r#"RETURN DEGREE_CENTRALITY("links")"#);
    assert!(!deg_results.is_empty());

    let pr_results = execute_query(
        &engine,
        r#"RETURN PAGERANK("links", {limit: 5, damping: 0.85})"#,
    );
    assert!(!pr_results.is_empty());
    if let Some(first) = pr_results.first() {
        if let Some(obj) = first.as_object() {
            assert!(obj.contains_key("node"));
            assert!(obj.contains_key("score"));
        }
    }
}

#[test]
fn test_rerank_lexical_reorders_by_overlap() {
    let tmp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap();

    // Lexical rerank is pure (no LLM, no storage): most query-token overlap wins.
    let q = r#"
        LET docs = [
            {content: "the quick brown fox jumps over"},
            {content: "vector databases store embeddings for search"},
            {content: "graph traversal and vector search combined"}
        ]
        RETURN RERANK("vector search embeddings", docs)
    "#;
    let res = execute_query(&engine, q);
    let arr = res[0].as_array().unwrap();
    assert_eq!(arr.len(), 3);
    // doc[1] has 3 overlapping tokens, doc[2] has 2, doc[0] has 0.
    assert_eq!(
        arr[0]["content"],
        json!("vector databases store embeddings for search")
    );
    assert_eq!(
        arr[1]["content"],
        json!("graph traversal and vector search combined")
    );
    assert_eq!(arr[2]["content"], json!("the quick brown fox jumps over"));

    // limit truncates to the top-k after reordering.
    let q_limited = r#"
        LET docs = [
            {content: "the quick brown fox jumps over"},
            {content: "vector databases store embeddings for search"},
            {content: "graph traversal and vector search combined"}
        ]
        RETURN RERANK("vector search embeddings", docs, {limit: 1})
    "#;
    let res = execute_query(&engine, q_limited);
    let arr = res[0].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(
        arr[0]["content"],
        json!("vector databases store embeddings for search")
    );
}

#[test]
fn test_stream_output_collection_emitted_at() {
    // Simulates the output collection that streams write to (with emitted_at for prune/retention)
    let tmp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap();
    let coll_name = "_streams:test_stream";
    let _ = engine.create_collection(coll_name.to_string(), None);
    let coll = engine.get_collection(coll_name).unwrap();

    // As done in stream task process_window
    let now = "2026-07-12T12:00:00Z".to_string();
    coll.insert(serde_json::json!({
        "value": 42,
        "emitted_at": now
    }))
    .unwrap();

    assert_eq!(coll.count(), 1);
    let doc = coll.get("0").ok().or_else(|| {
        // keys may be auto
        coll.scan(None).into_iter().next()
    });
    if let Some(d) = doc {
        assert!(d.get("emitted_at").is_some());
    }
}

#[test]
fn test_cte_simple() {
    let (engine, _tmp_dir) = create_seeded_engine();

    // Simple CTE - filter active users and return them
    // Active users: Alice, Bob, Diana, Eve (Charlie is NOT active)
    let results = execute_query(
        &engine,
        "WITH active AS (FOR u IN users FILTER u.active == true RETURN u) FOR a IN active RETURN a.name",
    );
    assert_eq!(results.len(), 4);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Bob")));
    assert!(results.contains(&json!("Diana")));
    assert!(results.contains(&json!("Eve")));
    assert!(!results.contains(&json!("Charlie"))); // Charlie is not active
}

#[test]
fn test_cte_multiple() {
    let (engine, _tmp_dir) = create_seeded_engine();

    // Multiple CTEs - cross product of paris_users x london_users
    // Paris users: Alice, Charlie (2)
    // London users: Bob, Eve (2)
    // Cross product: 2 x 2 = 4
    let results = execute_query(
        &engine,
        "WITH paris_users AS (FOR u IN users FILTER u.city == 'Paris' RETURN u), london_users AS (FOR u IN users FILTER u.city == 'London' RETURN u) FOR p IN paris_users FOR l IN london_users RETURN {paris: p.name, london: l.name}",
    );
    assert_eq!(results.len(), 4);
    assert!(results.contains(&json!({"paris": "Alice", "london": "Bob"})));
    assert!(results.contains(&json!({"paris": "Alice", "london": "Eve"})));
    assert!(results.contains(&json!({"paris": "Charlie", "london": "Bob"})));
    assert!(results.contains(&json!({"paris": "Charlie", "london": "Eve"})));
}

#[test]
fn test_cte_iterate_multiple_times() {
    let (engine, _tmp_dir) = create_seeded_engine();

    // CTE can be iterated multiple times in the same query
    let results = execute_query(
        &engine,
        "WITH nums AS (FOR i IN [1, 2, 3] RETURN i) FOR a IN nums FOR b IN nums RETURN {a: a, b: b}",
    );
    // 3 x 3 = 9 combinations
    assert_eq!(results.len(), 9);
}

#[test]
fn test_cte_with_aggregation() {
    let (engine, _tmp_dir) = create_seeded_engine();

    // CTE - aggregate within the CTE itself
    let results = execute_query(
        &engine,
        "WITH active_cities AS (FOR u IN users FILTER u.active == true COLLECT city = u.city AGGREGATE count = COUNT() RETURN {city: city, count: count}) FOR c IN active_cities RETURN c",
    );
    assert_eq!(results.len(), 3);
    // London (Bob, Eve), Paris (Alice), Berlin (Diana)
    assert!(results.contains(&json!({"city": "London", "count": 2})));
    assert!(results.contains(&json!({"city": "Paris", "count": 1})));
    assert!(results.contains(&json!({"city": "Berlin", "count": 1})));
}

#[test]
fn test_cte_with_columns() {
    let (engine, _tmp_dir) = create_seeded_engine();

    // CTE with column specification
    let results = execute_query(
        &engine,
        "WITH summary(name, age) AS (FOR u IN users RETURN {name: u.name, age: u.age, extra: 'ignored'}) FOR s IN summary RETURN s.name",
    );
    assert_eq!(results.len(), 5);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Bob")));
    assert!(results.contains(&json!("Charlie")));
    assert!(results.contains(&json!("Diana")));
    assert!(results.contains(&json!("Eve")));
}

#[test]
fn test_cte_reference_earlier_cte() {
    let (engine, _tmp_dir) = create_seeded_engine();

    // Second CTE can reference the first CTE
    let results = execute_query(
        &engine,
        "WITH active_users AS (FOR u IN users FILTER u.active == true RETURN u), active_paris AS (FOR u IN active_users FILTER u.city == 'Paris' RETURN u) FOR a IN active_paris RETURN a.name",
    );
    // Alice is active and in Paris (Bob/Eve are in London, Diana is in Berlin)
    assert_eq!(results.len(), 1);
    assert!(results.contains(&json!("Alice")));
}

#[test]
fn test_cte_empty_result() {
    let (engine, _tmp_dir) = create_seeded_engine();

    // CTE with no matching results
    let results = execute_query(
        &engine,
        "WITH no_active AS (FOR u IN users FILTER u.active == false AND u.city == 'Tokyo' RETURN u) FOR n IN no_active RETURN n.name",
    );
    assert_eq!(results.len(), 0);
}

#[test]
fn test_no_cte_still_works() {
    let (engine, _tmp_dir) = create_seeded_engine();

    // Verify queries without CTE still work
    let results = execute_query(&engine, "FOR u IN users FILTER u.age > 28 RETURN u.name");
    assert_eq!(results.len(), 3);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Charlie")));
    assert!(results.contains(&json!("Eve")));
}
