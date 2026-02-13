//! Blob Storage Coverage Tests
//!
//! Tests for binary large object (blob) operations including:
//! - PUT blob chunks
//! - GET blob chunks
//! - DELETE blobs
//! - Large file handling

use serde_json::json;
use solidb::storage::StorageEngine;
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine
        .create_collection("files".to_string(), Some("blob".to_string()))
        .unwrap();
    (engine, tmp_dir)
}

// ============================================================================
// Basic Blob Tests
// ============================================================================

#[test]
fn test_put_single_blob_chunk() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    // Create a document to associate the blob with
    files
        .insert(json!({"_key": "file1", "name": "test.txt"}))
        .unwrap();

    // Put a blob chunk
    let data = b"Hello, World!";
    let result = files.put_blob_chunk("file1", 0, data);

    assert!(result.is_ok());
}

#[test]
fn test_get_single_blob_chunk() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "file1", "name": "test.txt"}))
        .unwrap();

    let data = b"Hello, World!";
    files.put_blob_chunk("file1", 0, data).unwrap();

    // Get the blob chunk
    let retrieved = files.get_blob_chunk("file1", 0);

    assert!(retrieved.is_ok());
    let retrieved = retrieved.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap(), data.to_vec());
}

#[test]
fn test_put_multiple_blob_chunks() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "large_file", "name": "large.bin"}))
        .unwrap();

    // Simulate a multi-chunk file
    let chunk0 = b"Chunk 0 data here";
    let chunk1 = b"Chunk 1 data here";
    let chunk2 = b"Chunk 2 data here";

    files.put_blob_chunk("large_file", 0, chunk0).unwrap();
    files.put_blob_chunk("large_file", 1, chunk1).unwrap();
    files.put_blob_chunk("large_file", 2, chunk2).unwrap();

    // Verify all chunks
    assert_eq!(
        files.get_blob_chunk("large_file", 0).unwrap().unwrap(),
        chunk0.to_vec()
    );
    assert_eq!(
        files.get_blob_chunk("large_file", 1).unwrap().unwrap(),
        chunk1.to_vec()
    );
    assert_eq!(
        files.get_blob_chunk("large_file", 2).unwrap().unwrap(),
        chunk2.to_vec()
    );
}

#[test]
fn test_overwrite_blob_chunk() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "file1", "name": "test.txt"}))
        .unwrap();

    // Initial data
    files.put_blob_chunk("file1", 0, b"Original").unwrap();
    assert_eq!(
        files.get_blob_chunk("file1", 0).unwrap().unwrap(),
        b"Original".to_vec()
    );

    // Overwrite
    files.put_blob_chunk("file1", 0, b"Updated").unwrap();
    assert_eq!(
        files.get_blob_chunk("file1", 0).unwrap().unwrap(),
        b"Updated".to_vec()
    );
}

#[test]
fn test_delete_blob_data() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "file1", "name": "test.txt"}))
        .unwrap();

    // Add blob chunks
    files.put_blob_chunk("file1", 0, b"Chunk 0").unwrap();
    files.put_blob_chunk("file1", 1, b"Chunk 1").unwrap();

    // Delete the blob data
    let result = files.delete_blob_data("file1");
    assert!(result.is_ok());

    // Chunks should be gone
    assert!(files.get_blob_chunk("file1", 0).unwrap().is_none());
    assert!(files.get_blob_chunk("file1", 1).unwrap().is_none());
}

// ============================================================================
// Binary Data Tests
// ============================================================================

#[test]
fn test_blob_with_binary_data() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "binary", "name": "data.bin"}))
        .unwrap();

    // Create binary data with all byte values
    let data: Vec<u8> = (0..=255).collect();

    files.put_blob_chunk("binary", 0, &data).unwrap();

    let retrieved = files.get_blob_chunk("binary", 0).unwrap().unwrap();
    assert_eq!(retrieved.len(), 256);
    assert_eq!(retrieved, data);
}

#[test]
fn test_blob_with_null_bytes() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "nulls", "name": "nulls.bin"}))
        .unwrap();

    // Data with embedded null bytes
    let data = b"Hello\x00World\x00!";

    files.put_blob_chunk("nulls", 0, data).unwrap();

    let retrieved = files.get_blob_chunk("nulls", 0).unwrap().unwrap();
    assert_eq!(retrieved, data.to_vec());
}

#[test]
fn test_blob_empty_chunk() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "empty", "name": "empty.bin"}))
        .unwrap();

    // Empty data
    files.put_blob_chunk("empty", 0, b"").unwrap();

    let retrieved = files.get_blob_chunk("empty", 0).unwrap().unwrap();
    assert!(retrieved.is_empty());
}

// ============================================================================
// Large Blob Tests
// ============================================================================

#[test]
fn test_large_blob_chunk() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "large", "name": "large.bin"}))
        .unwrap();

    // 1MB chunk
    let size = 1024 * 1024;
    let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();

    files.put_blob_chunk("large", 0, &data).unwrap();

    let retrieved = files.get_blob_chunk("large", 0).unwrap().unwrap();
    assert_eq!(retrieved.len(), size);
    assert_eq!(retrieved[0], 0);
    assert_eq!(retrieved[255], 255);
    assert_eq!(retrieved[256], 0);
}

#[test]
fn test_multiple_large_chunks() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "multipart", "name": "big.bin"}))
        .unwrap();

    // 100KB per chunk, 5 chunks = 500KB total
    let chunk_size = 100 * 1024;
    for i in 0..5 {
        let data: Vec<u8> = vec![i as u8; chunk_size];
        files.put_blob_chunk("multipart", i, &data).unwrap();
    }

    // Verify all chunks
    for i in 0..5 {
        let retrieved = files.get_blob_chunk("multipart", i).unwrap().unwrap();
        assert_eq!(retrieved.len(), chunk_size);
        assert!(retrieved.iter().all(|&b| b == i as u8));
    }
}

// ============================================================================
// Error Cases
// ============================================================================

#[test]
fn test_get_nonexistent_blob_chunk() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "file1", "name": "test.txt"}))
        .unwrap();

    // Try to get a chunk that doesn't exist - returns None
    let result = files.get_blob_chunk("file1", 0).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_get_blob_wrong_chunk_index() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "file1", "name": "test.txt"}))
        .unwrap();
    files.put_blob_chunk("file1", 0, b"data").unwrap();

    // Chunk 0 exists, but chunk 1 doesn't
    assert!(files.get_blob_chunk("file1", 0).unwrap().is_some());
    assert!(files.get_blob_chunk("file1", 1).unwrap().is_none());
}

#[test]
fn test_delete_nonexistent_blob() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "file1", "name": "test.txt"}))
        .unwrap();

    // Delete a blob that was never created - should succeed (no-op)
    let result = files.delete_blob_data("file1");
    assert!(result.is_ok());
}

// ============================================================================
// Multiple Documents with Blobs
// ============================================================================

#[test]
fn test_multiple_documents_with_blobs() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    // Create multiple documents with blobs
    for i in 0..5 {
        let key = format!("file{}", i);
        files
            .insert(json!({"_key": key, "name": format!("file{}.txt", i)}))
            .unwrap();
        files
            .put_blob_chunk(&key, 0, format!("Content of file {}", i).as_bytes())
            .unwrap();
    }

    // Verify each document's blob
    for i in 0..5 {
        let key = format!("file{}", i);
        let expected = format!("Content of file {}", i);
        let retrieved = files.get_blob_chunk(&key, 0).unwrap().unwrap();
        assert_eq!(String::from_utf8(retrieved).unwrap(), expected);
    }
}

#[test]
fn test_blob_isolation_between_documents() {
    let (engine, _tmp) = create_test_engine();
    let files = engine.get_collection("files").unwrap();

    files
        .insert(json!({"_key": "doc1", "name": "doc1.txt"}))
        .unwrap();
    files
        .insert(json!({"_key": "doc2", "name": "doc2.txt"}))
        .unwrap();

    files.put_blob_chunk("doc1", 0, b"Data for doc1").unwrap();
    files.put_blob_chunk("doc2", 0, b"Data for doc2").unwrap();

    // Verify blobs are isolated
    assert_eq!(
        String::from_utf8(files.get_blob_chunk("doc1", 0).unwrap().unwrap()).unwrap(),
        "Data for doc1"
    );
    assert_eq!(
        String::from_utf8(files.get_blob_chunk("doc2", 0).unwrap().unwrap()).unwrap(),
        "Data for doc2"
    );

    // Delete doc1's blob shouldn't affect doc2
    files.delete_blob_data("doc1").unwrap();
    assert!(files.get_blob_chunk("doc1", 0).unwrap().is_none());
    assert!(files.get_blob_chunk("doc2", 0).unwrap().is_some());
}

// ============================================================================
// Blob Persistence
// ============================================================================

#[test]
fn test_blob_persistence() {
    let tmp_dir = TempDir::new().unwrap();
    let path = tmp_dir.path().to_str().unwrap();

    // First session: create blob
    {
        let engine = StorageEngine::new(path).unwrap();
        engine
            .create_collection("files".to_string(), Some("blob".to_string()))
            .unwrap();
        let files = engine.get_collection("files").unwrap();

        files
            .insert(json!({"_key": "persistent", "name": "data.bin"}))
            .unwrap();
        files
            .put_blob_chunk("persistent", 0, b"Persistent data")
            .unwrap();

        engine.flush().unwrap();
    }

    // Second session: verify blob persisted
    {
        let engine = StorageEngine::new(path).unwrap();
        let files = engine.get_collection("files").unwrap();

        let retrieved = files.get_blob_chunk("persistent", 0).unwrap().unwrap();
        assert_eq!(String::from_utf8(retrieved).unwrap(), "Persistent data");
    }
}

#[test]
fn test_blob_stats() {
    let path = tempfile::tempdir().unwrap();
    let engine = StorageEngine::new(path.path()).unwrap();
    engine
        .create_collection("files".to_string(), Some("blob".to_string()))
        .unwrap();
    let files = engine.get_collection("files").unwrap();

    // Initially, stats should be zero
    let (count, bytes) = files.blob_stats().unwrap();
    assert_eq!(count, 0);
    assert_eq!(bytes, 0);

    // Add some blob chunks
    files.put_blob_chunk("file1", 0, b"Hello World").unwrap();
    files.put_blob_chunk("file1", 1, b"Chunk 2").unwrap();
    files.put_blob_chunk("file2", 0, b"Another file").unwrap();

    // Check stats
    let (count, bytes) = files.blob_stats().unwrap();
    assert_eq!(count, 3);
    assert_eq!(bytes, 11 + 7 + 12); // "Hello World" = 11, "Chunk 2" = 7, "Another file" = 12

    // Delete one blob
    files.delete_blob_data("file1").unwrap();

    let (count, bytes) = files.blob_stats().unwrap();
    assert_eq!(count, 1);
    assert_eq!(bytes, 12);
}

#[test]
fn test_blob_stats_non_blob_collection() {
    let path = tempfile::tempdir().unwrap();
    let engine = StorageEngine::new(path.path()).unwrap();
    engine.create_collection("docs".to_string(), None).unwrap();
    let docs = engine.get_collection("docs").unwrap();

    // Non-blob collection should return (0, 0)
    let (count, bytes) = docs.blob_stats().unwrap();
    assert_eq!(count, 0);
    assert_eq!(bytes, 0);
}
