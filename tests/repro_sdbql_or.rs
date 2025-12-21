use serde_json::json;
use solidb::{parse, QueryExecutor, StorageEngine};
use tempfile::TempDir;

fn create_test_storage() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    (storage, temp_dir)
}

#[test]
fn test_or_short_circuit_missing_field() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("channels".to_string(), None).unwrap();
    let collection = storage.get_collection("channels").unwrap();

    // Document with type "standard" but NO members field
    collection
        .insert(json!({
            "_key": "c1",
            "type": "standard"
        }))
        .unwrap();

    // The query reported by the user (slightly adapted to focus on short-circuiting)
    // POSITION(c.members, "me") will error if c.members is Null (missing)
    let query_str = r#"
        FOR c IN channels
        FILTER c.type == "standard" OR POSITION(c.members, "me") >= 0
        RETURN c
    "#;

    let query = parse(query_str).unwrap();
    let executor = QueryExecutor::new(&storage);
    
    // This should NOT error and should return the document
    let results = executor.execute(&query).expect("Query failed, likely due to lack of short-circuiting");

    assert_eq!(results.len(), 1, "Should have returned the standard channel");
    assert_eq!(results[0]["_key"], "c1");
}

#[test]
fn test_and_short_circuit_missing_field() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("channels".to_string(), None).unwrap();
    let collection = storage.get_collection("channels").unwrap();

    // Document with type "private" but NO members field
    collection
        .insert(json!({
            "_key": "c2",
            "type": "private"
        }))
        .unwrap();

    // If AND short-circuits, FALSE AND ERROR should return FALSE
    let query_str = r#"
        FOR c IN channels
        FILTER c.type == "standard" AND POSITION(c.members, "me") >= 0
        RETURN c
    "#;

    let query = parse(query_str).unwrap();
    let executor = QueryExecutor::new(&storage);
    
    // This should NOT error and should return 0 results
    let results = executor.execute(&query).expect("Query failed, likely due to lack of short-circuiting");

    assert_eq!(results.len(), 0, "Should have returned NO channels");
}
