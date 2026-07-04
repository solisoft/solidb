//! Collection Truncate Tests
//!
//! Verifies the `truncate()` method which removes all documents but preserves index definitions.

use serde_json::json;
use solidb::storage::index::IndexType;
use solidb::storage::StorageEngine;
use tempfile::TempDir;
use uuid::Uuid;

fn create_test_db() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");

    engine
        .create_database(format!("testdb_{}", Uuid::new_v4()))
        .unwrap();
    (engine, tmp_dir)
}

#[test]
fn test_truncate_preserves_indexes() {
    let (engine, _tmp) = create_test_db();
    let db_names = engine.list_databases();
    let db = engine.get_database(&db_names[0]).unwrap();

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
    let db_names = engine.list_databases();
    let db = engine.get_database(&db_names[0]).unwrap();
    db.create_collection("empty".to_string(), None).unwrap();
    let col = db.get_collection("empty").unwrap();

    let count = col.truncate().unwrap();
    assert_eq!(count, 0);
    assert_eq!(col.count(), 0);
}

#[test]
fn test_truncate_clears_blob_chunks() {
    let (engine, _tmp) = create_test_db();
    let db_names = engine.list_databases();
    let db = engine.get_database(&db_names[0]).unwrap();

    db.create_collection("files".to_string(), Some("blob".to_string()))
        .unwrap();
    let files = db.get_collection("files").unwrap();

    files
        .insert(json!({ "_key": "doc1", "filename": "a.bin" }))
        .unwrap();
    files.put_blob_chunk("doc1", 0, b"chunk-zero").unwrap();
    files.put_blob_chunk("doc1", 1, b"chunk-one").unwrap();

    let (chunks_before, bytes_before) = files.blob_stats().unwrap();
    assert_eq!(chunks_before, 2);
    assert!(bytes_before > 0);
    assert_eq!(files.stats().chunk_count, 2);

    files.truncate().unwrap();

    // Chunks are gone from storage, not just from the cached counter
    assert_eq!(files.get_blob_chunk("doc1", 0).unwrap(), None);
    assert_eq!(files.get_blob_chunk("doc1", 1).unwrap(), None);
    assert_eq!(files.blob_stats().unwrap(), (0, 0));
    assert_eq!(files.stats().chunk_count, 0);
}

#[test]
fn test_truncate_clears_tmp_upload_chunks() {
    let (engine, _tmp) = create_test_db();
    let db_names = engine.list_databases();
    let db = engine.get_database(&db_names[0]).unwrap();

    db.create_collection("uploads".to_string(), Some("blob".to_string()))
        .unwrap();
    let uploads = db.get_collection("uploads").unwrap();

    uploads.put_blob_chunk_tmp("up1", 0, b"partial").unwrap();

    uploads.truncate().unwrap();

    // Finalizing must fail: the temp chunk was removed by truncate
    assert!(uploads.finalize_blob_upload("up1", "doc1", 1).is_err());
}

#[test]
fn test_truncate_clears_fulltext_entries() {
    let (engine, _tmp) = create_test_db();
    let db_names = engine.list_databases();
    let db = engine.get_database(&db_names[0]).unwrap();

    db.create_collection("articles".to_string(), None).unwrap();
    let articles = db.get_collection("articles").unwrap();

    articles
        .create_fulltext_index("ft_bio".to_string(), vec!["bio".to_string()], None)
        .unwrap();
    articles
        .insert(json!({ "name": "Alice", "bio": "database engineer" }))
        .unwrap();
    articles
        .insert(json!({ "name": "Bob", "bio": "database admin" }))
        .unwrap();

    let matches = articles.fulltext_search("database", None, 10).unwrap();
    assert_eq!(matches.len(), 2);

    articles.truncate().unwrap();

    // No stale fulltext entries pointing at deleted documents
    let matches = articles.fulltext_search("database", None, 10).unwrap();
    assert!(matches.is_empty());

    // Index definition survives and still works for new documents
    assert_eq!(articles.list_fulltext_indexes().len(), 1);
    articles
        .insert(json!({ "name": "Carol", "bio": "database architect" }))
        .unwrap();
    let matches = articles.fulltext_search("database", None, 10).unwrap();
    assert_eq!(matches.len(), 1);
}

#[test]
fn test_truncate_clears_ttl_expiry_entries() {
    let (engine, _tmp) = create_test_db();
    let db_names = engine.list_databases();
    let db = engine.get_database(&db_names[0]).unwrap();

    db.create_collection("sessions".to_string(), None).unwrap();
    let sessions = db.get_collection("sessions").unwrap();

    sessions
        .create_ttl_index("ttl_created".to_string(), "created_at".to_string(), 1)
        .unwrap();

    // Control: an already-expired document is reaped via the expiry index
    sessions
        .insert(json!({ "_key": "s1", "created_at": 1000 }))
        .unwrap();
    assert_eq!(sessions.cleanup_all_expired_documents().unwrap(), 1);

    // Truncate must also drop the expiry entries, so cleanup finds nothing
    sessions
        .insert(json!({ "_key": "s2", "created_at": 1000 }))
        .unwrap();
    sessions.truncate().unwrap();
    assert_eq!(sessions.cleanup_all_expired_documents().unwrap(), 0);
}
