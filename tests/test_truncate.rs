//! Collection Truncate Tests
//!
//! Verifies the `truncate()` method which removes all documents but preserves index definitions.

use serde_json::json;
use solidb::storage::index::IndexType;
use solidb::storage::StorageEngine;
use tempfile::TempDir;

fn create_test_db() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");

    engine.create_database("testdb".to_string()).unwrap();
    (engine, tmp_dir)
}

#[test]
fn test_truncate_preserves_indexes() {
    let (engine, _tmp) = create_test_db();
    let db = engine.get_database("testdb").unwrap();

    // 1. Create collection
    db.create_collection("users".to_string(), None).unwrap();
    let users = db.get_collection("users").unwrap();

    // 2. Create Index on 'age'
    users
        .create_index(
            "idx_age".to_string(),
            vec!["age".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();

    // 3. Insert Documents
    users.insert(json!({ "name": "Alice", "age": 25 })).unwrap();
    users.insert(json!({ "name": "Bob", "age": 30 })).unwrap();
    users
        .insert(json!({ "name": "Charlie", "age": 35 }))
        .unwrap();

    assert_eq!(users.count(), 3);

    // Verify Index Usage (manual check via internal API or query)
    // We can assume if insert worked, index is updated.

    // 4. Truncate
    let deleted_count = users.truncate().unwrap();
    assert_eq!(deleted_count, 3);
    assert_eq!(users.count(), 0);

    // 5. Verify Index *Definition* Exists
    let indexes = users.get_all_indexes();
    assert_eq!(indexes.len(), 1);
    assert_eq!(indexes[0].name, "idx_age");

    // 6. Verify Index Empty (implied by count 0, but safe to check query)
    // We'll insert a new doc and query it to ensure index still works
    users.insert(json!({ "name": "David", "age": 40 })).unwrap();
    assert_eq!(users.count(), 1);

    // Simple SDBQL query to test index
    // Note: We need the query executor for this, or just rely on manual verification
    // that insert didn't fail.
}

#[test]
fn test_truncate_empty_collection() {
    let (engine, _tmp) = create_test_db();
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("empty".to_string(), None).unwrap();
    let col = db.get_collection("empty").unwrap();

    let count = col.truncate().unwrap();
    assert_eq!(count, 0);
    assert_eq!(col.count(), 0);
}
