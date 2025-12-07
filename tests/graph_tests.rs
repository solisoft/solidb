//! Graph and Edge Collection Tests
//! Tests for edge collection validation and graph traversal queries

use serde_json::json;
use solidb::{parse, QueryExecutor, StorageEngine};
use tempfile::TempDir;

/// Helper to create a test storage engine
fn create_test_storage() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    (storage, temp_dir)
}

/// Setup a social network graph for testing
/// users: alice, bob, charlie, diana
/// follows: alice->bob, alice->charlie, bob->charlie, charlie->diana
fn setup_social_graph(storage: &StorageEngine) {
    // Create users collection (document collection)
    storage.create_collection("users".to_string(), None).unwrap();
    let users = storage.get_collection("users").unwrap();

    users
        .insert(json!({
            "_key": "alice",
            "name": "Alice",
            "age": 30
        }))
        .unwrap();

    users
        .insert(json!({
            "_key": "bob",
            "name": "Bob",
            "age": 25
        }))
        .unwrap();

    users
        .insert(json!({
            "_key": "charlie",
            "name": "Charlie",
            "age": 35
        }))
        .unwrap();

    users
        .insert(json!({
            "_key": "diana",
            "name": "Diana",
            "age": 28
        }))
        .unwrap();

    // Create follows collection (edge collection)
    storage
        .create_collection("follows".to_string(), Some("edge".to_string()))
        .unwrap();
    let follows = storage.get_collection("follows").unwrap();

    // Alice follows Bob and Charlie
    follows
        .insert(json!({
            "_key": "e1",
            "_from": "users/alice",
            "_to": "users/bob",
            "since": "2023-01-01"
        }))
        .unwrap();

    follows
        .insert(json!({
            "_key": "e2",
            "_from": "users/alice",
            "_to": "users/charlie",
            "since": "2023-02-15"
        }))
        .unwrap();

    // Bob follows Charlie
    follows
        .insert(json!({
            "_key": "e3",
            "_from": "users/bob",
            "_to": "users/charlie",
            "since": "2023-03-20"
        }))
        .unwrap();

    // Charlie follows Diana
    follows
        .insert(json!({
            "_key": "e4",
            "_from": "users/charlie",
            "_to": "users/diana",
            "since": "2023-04-10"
        }))
        .unwrap();
}

// ==================== Edge Collection Validation Tests ====================

#[test]
fn test_edge_collection_creation() {
    let (storage, _dir) = create_test_storage();

    // Create edge collection
    let result = storage.create_collection("edges".to_string(), Some("edge".to_string()));
    assert!(result.is_ok());

    // Verify collection type
    let collection = storage.get_collection("edges").unwrap();
    assert_eq!(collection.get_type(), "edge");
}

#[test]
fn test_edge_document_requires_from() {
    let (storage, _dir) = create_test_storage();
    storage
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let collection = storage.get_collection("edges").unwrap();

    // Insert without _from should fail
    let result = collection.insert(json!({
        "_to": "users/bob"
    }));

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("_from"));
}

#[test]
fn test_edge_document_requires_to() {
    let (storage, _dir) = create_test_storage();
    storage
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let collection = storage.get_collection("edges").unwrap();

    // Insert without _to should fail
    let result = collection.insert(json!({
        "_from": "users/alice"
    }));

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("_to"));
}

#[test]
fn test_edge_document_from_must_be_string() {
    let (storage, _dir) = create_test_storage();
    storage
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let collection = storage.get_collection("edges").unwrap();

    // Insert with non-string _from should fail
    let result = collection.insert(json!({
        "_from": 123,
        "_to": "users/bob"
    }));

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("string"));
}

#[test]
fn test_edge_document_to_must_be_string() {
    let (storage, _dir) = create_test_storage();
    storage
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let collection = storage.get_collection("edges").unwrap();

    // Insert with non-string _to should fail
    let result = collection.insert(json!({
        "_from": "users/alice",
        "_to": true
    }));

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("string"));
}

#[test]
fn test_edge_document_from_must_be_nonempty() {
    let (storage, _dir) = create_test_storage();
    storage
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let collection = storage.get_collection("edges").unwrap();

    // Insert with empty _from should fail
    let result = collection.insert(json!({
        "_from": "",
        "_to": "users/bob"
    }));

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("non-empty"));
}

#[test]
fn test_edge_document_to_must_be_nonempty() {
    let (storage, _dir) = create_test_storage();
    storage
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let collection = storage.get_collection("edges").unwrap();

    // Insert with empty _to should fail
    let result = collection.insert(json!({
        "_from": "users/alice",
        "_to": ""
    }));

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("non-empty"));
}

#[test]
fn test_valid_edge_document_insert() {
    let (storage, _dir) = create_test_storage();
    storage
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let collection = storage.get_collection("edges").unwrap();

    // Valid edge document
    let result = collection.insert(json!({
        "_from": "users/alice",
        "_to": "users/bob",
        "label": "follows"
    }));

    assert!(result.is_ok());
    let doc = result.unwrap();
    assert_eq!(doc.to_value()["_from"], "users/alice");
    assert_eq!(doc.to_value()["_to"], "users/bob");
}

#[test]
fn test_document_collection_allows_no_from_to() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("docs".to_string(), None).unwrap();
    let collection = storage.get_collection("docs").unwrap();

    // Regular document collection doesn't require _from/_to
    let result = collection.insert(json!({
        "name": "Test",
        "value": 42
    }));

    assert!(result.is_ok());
}

// ==================== Edge Update Validation Tests ====================

#[test]
fn test_edge_update_preserves_from_to() {
    let (storage, _dir) = create_test_storage();
    storage
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let collection = storage.get_collection("edges").unwrap();

    // Insert valid edge
    collection
        .insert(json!({
            "_key": "e1",
            "_from": "users/alice",
            "_to": "users/bob"
        }))
        .unwrap();

    // Update should preserve _from and _to
    let result = collection.update("e1", json!({ "label": "friend" }));
    assert!(result.is_ok());

    let doc = collection.get("e1").unwrap();
    assert_eq!(doc.to_value()["_from"], "users/alice");
    assert_eq!(doc.to_value()["_to"], "users/bob");
    assert_eq!(doc.to_value()["label"], "friend");
}

// ==================== Graph Traversal Tests ====================

#[test]
fn test_single_hop_traversal() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Find who Alice follows (1-hop outbound)
    let query = parse(
        r#"
        FOR user IN users
          FILTER user._key == "alice"
          FOR edge IN follows
            FILTER edge._from == user._id
            FOR friend IN users
              FILTER friend._id == edge._to
              RETURN friend.name
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Bob")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_two_hop_traversal() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Find friends-of-friends (2-hop traversal)
    // Start from Alice -> find who her friends follow
    let query = parse(
        r#"
        FOR user IN users
          FILTER user._key == "alice"
          FOR edge1 IN follows
            FILTER edge1._from == user._id
            FOR friend IN users
              FILTER friend._id == edge1._to
              FOR edge2 IN follows
                FILTER edge2._from == friend._id
                RETURN edge2._to
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Alice -> Bob -> Charlie
    // Alice -> Charlie -> Diana
    // So results should include users/charlie and users/diana
    assert!(!results.is_empty());
    assert!(results.contains(&json!("users/charlie")) || results.contains(&json!("users/diana")));
}

#[test]
fn test_traversal_with_edge_properties() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Find follows relationships with edge properties
    let query = parse(
        r#"
        FOR user IN users
          FILTER user._key == "alice"
          FOR edge IN follows
            FILTER edge._from == user._id
            FOR friend IN users
              FILTER friend._id == edge._to
              RETURN { friend: friend.name, since: edge.since }
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);

    // Check that edge properties are returned
    let bob_result = results
        .iter()
        .find(|r| r["friend"] == "Bob")
        .expect("Bob should be in results");
    assert_eq!(bob_result["since"], "2023-01-01");

    let charlie_result = results
        .iter()
        .find(|r| r["friend"] == "Charlie")
        .expect("Charlie should be in results");
    assert_eq!(charlie_result["since"], "2023-02-15");
}

#[test]
fn test_reverse_traversal() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Find who follows Charlie (inbound edges)
    let query = parse(
        r#"
        FOR user IN users
          FILTER user._key == "charlie"
          FOR edge IN follows
            FILTER edge._to == user._id
            FOR follower IN users
              FILTER follower._id == edge._from
              RETURN follower.name
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Alice and Bob both follow Charlie
    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Bob")));
}

#[test]
fn test_traversal_no_results() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Diana doesn't follow anyone
    let query = parse(
        r#"
        FOR user IN users
          FILTER user._key == "diana"
          FOR edge IN follows
            FILTER edge._from == user._id
            RETURN edge._to
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 0);
}

#[test]
fn test_traversal_with_filter_on_vertex() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Find Alice's friends who are older than 30
    let query = parse(
        r#"
        FOR user IN users
          FILTER user._key == "alice"
          FOR edge IN follows
            FILTER edge._from == user._id
            FOR friend IN users
              FILTER friend._id == edge._to
              FILTER friend.age > 30
              RETURN friend.name
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Only Charlie (age 35) is older than 30
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Charlie"));
}

#[test]
fn test_traversal_count_edges() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Count how many people each user follows
    let query = parse(
        r#"
        FOR user IN users
          LET followCount = LENGTH((FOR edge IN follows FILTER edge._from == user._id RETURN edge))
          RETURN { name: user.name, follows: followCount }
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 4);

    let alice = results
        .iter()
        .find(|r| r["name"] == "Alice")
        .expect("Alice should be in results");
    assert_eq!(alice["follows"], 2); // Alice follows 2 people

    let diana = results
        .iter()
        .find(|r| r["name"] == "Diana")
        .expect("Diana should be in results");
    assert_eq!(diana["follows"], 0); // Diana follows nobody
}

#[test]
fn test_mutual_follows() {
    let (storage, _dir) = create_test_storage();

    // Create a simpler graph with mutual follows
    storage.create_collection("people".to_string(), None).unwrap();
    let people = storage.get_collection("people").unwrap();

    people.insert(json!({"_key": "a", "name": "A"})).unwrap();
    people.insert(json!({"_key": "b", "name": "B"})).unwrap();

    storage
        .create_collection("knows".to_string(), Some("edge".to_string()))
        .unwrap();
    let knows = storage.get_collection("knows").unwrap();

    // A knows B
    knows
        .insert(json!({
            "_from": "people/a",
            "_to": "people/b"
        }))
        .unwrap();

    // B knows A (mutual)
    knows
        .insert(json!({
            "_from": "people/b",
            "_to": "people/a"
        }))
        .unwrap();

    // Find mutual relationships
    let query = parse(
        r#"
        FOR edge1 IN knows
          FOR edge2 IN knows
            FILTER edge1._from == edge2._to AND edge1._to == edge2._from
            RETURN { source: edge1._from, target: edge1._to }
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Should find 2 mutual edges
    assert_eq!(results.len(), 2);
}

// ==================== Complex Graph Query Tests ====================

#[test]
fn test_path_exists() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Check if path exists from Alice to Diana (via Charlie)
    let query = parse(
        r#"
        FOR user IN users
          FILTER user._key == "alice"
          FOR edge1 IN follows
            FILTER edge1._from == user._id
            FOR mid IN users
              FILTER mid._id == edge1._to
              FOR edge2 IN follows
                FILTER edge2._from == mid._id
                FOR target IN users
                  FILTER target._id == edge2._to
                  FILTER target._key == "diana"
                  RETURN { path: [user.name, mid.name, target.name] }
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Alice -> Charlie -> Diana
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["path"], json!(["Alice", "Charlie", "Diana"]));
}

#[test]
fn test_all_edges_query() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Get all edges
    let query = parse(
        r#"
        FOR edge IN follows
          RETURN { source: edge._from, target: edge._to }
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 4); // 4 edges in our graph
}

#[test]
fn test_edge_collection_scan() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    let collection = storage.get_collection("follows").unwrap();
    let all_edges = collection.scan(None);

    assert_eq!(all_edges.len(), 4);

    // Verify all edges have _from and _to
    for edge in &all_edges {
        let value = edge.to_value();
        assert!(value.get("_from").is_some());
        assert!(value.get("_to").is_some());
    }
}

// ==================== Native Graph Syntax Tests ====================

#[test]
fn test_parse_outbound_traversal() {
    // Test parsing of native OUTBOUND traversal syntax
    let query = parse(
        r#"
        FOR v IN OUTBOUND "users/alice" follows
          RETURN v.name
    "#,
    );
    assert!(query.is_ok(), "OUTBOUND traversal should parse");
}

#[test]
fn test_parse_inbound_traversal() {
    // Test parsing of native INBOUND traversal syntax
    let query = parse(
        r#"
        FOR v IN INBOUND "users/charlie" follows
          RETURN v.name
    "#,
    );
    assert!(query.is_ok(), "INBOUND traversal should parse");
}

#[test]
fn test_parse_any_traversal() {
    // Test parsing of native ANY traversal syntax
    let query = parse(
        r#"
        FOR v IN ANY "users/bob" follows
          RETURN v.name
    "#,
    );
    assert!(query.is_ok(), "ANY traversal should parse");
}

#[test]
fn test_parse_traversal_with_depth() {
    // Test parsing with depth range
    let query = parse(
        r#"
        FOR v IN 1..3 OUTBOUND "users/alice" follows
          RETURN v.name
    "#,
    );
    assert!(query.is_ok(), "Traversal with depth range should parse");
}

#[test]
fn test_parse_traversal_with_edge_var() {
    // Test parsing with edge variable
    let query = parse(
        r#"
        FOR v, e IN OUTBOUND "users/alice" follows
          RETURN { vertex: v.name, edge: e }
    "#,
    );
    assert!(query.is_ok(), "Traversal with edge variable should parse");
}

#[test]
fn test_parse_shortest_path() {
    // Test parsing of SHORTEST_PATH syntax
    let query = parse(
        r#"
        FOR v IN SHORTEST_PATH "users/alice" TO "users/diana" OUTBOUND follows
          RETURN v.name
    "#,
    );
    assert!(query.is_ok(), "SHORTEST_PATH should parse");
}

#[test]
fn test_execute_outbound_traversal() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Execute native OUTBOUND traversal
    let query = parse(
        r#"
        FOR v IN OUTBOUND "users/alice" follows
          RETURN v.name
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Alice follows Bob and Charlie
    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Bob")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_execute_inbound_traversal() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Execute native INBOUND traversal - who follows Charlie
    let query = parse(
        r#"
        FOR v IN INBOUND "users/charlie" follows
          RETURN v.name
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Alice and Bob follow Charlie
    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Bob")));
}

#[test]
fn test_execute_multi_hop_traversal() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Execute 2-hop traversal
    let query = parse(
        r#"
        FOR v IN 1..2 OUTBOUND "users/alice" follows
          RETURN v.name
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // 1-hop: Bob, Charlie
    // 2-hop: Charlie (from Bob), Diana (from Charlie)
    // Results should include all reachable vertices within 2 hops
    assert!(results.len() >= 2);
    assert!(results.contains(&json!("Bob")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_execute_shortest_path() {
    let (storage, _dir) = create_test_storage();
    setup_social_graph(&storage);

    // Find shortest path from Alice to Diana
    let query = parse(
        r#"
        FOR v IN SHORTEST_PATH "users/alice" TO "users/diana" OUTBOUND follows
          RETURN v.name
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Path: Alice -> Charlie -> Diana
    assert!(results.len() >= 2);
    // Diana should be in the path
    assert!(results.contains(&json!("Diana")));
}

#[test]
fn test_shortest_path_no_path() {
    let (storage, _dir) = create_test_storage();
    
    // Create isolated vertices
    storage.create_collection("vertices".to_string(), None).unwrap();
    let vertices = storage.get_collection("vertices").unwrap();
    vertices.insert(json!({"_key": "a", "name": "A"})).unwrap();
    vertices.insert(json!({"_key": "b", "name": "B"})).unwrap();
    
    storage.create_collection("edges".to_string(), Some("edge".to_string())).unwrap();
    // No edges - vertices are isolated

    let query = parse(
        r#"
        FOR v IN SHORTEST_PATH "vertices/a" TO "vertices/b" OUTBOUND edges
          RETURN v.name
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // No path exists
    assert_eq!(results.len(), 0);
}
