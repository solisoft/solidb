use solidb::storage::{StorageEngine, IndexType};
use tempfile::TempDir;

#[test]
fn test_fulltext_index_registration_fix() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");

    storage.create_collection("messages".to_string(), None).unwrap();
    let collection = storage.get_collection("messages").unwrap();

    // 1. Simulate the BUG: creating a regular index with "Fulltext" type
    collection.create_index(
        "broken_ft_index".to_string(),
        vec!["text".to_string()],
        IndexType::Fulltext,
        false
    ).unwrap();

    // Verify FULLTEXT search FAILS (returns None) because it can't find the index
    // Note: fulltext_search returns Option<Vec<...>>. None means "no index found".
    let search_result_broken = collection.fulltext_search("text", "hello", 1);
    assert!(search_result_broken.is_none(), "Search should return None when index is missing/broken");

    // 2. Simulate the FIX: calling create_fulltext_index
    collection.create_fulltext_index(
        "correct_ft_index".to_string(),
        vec!["text".to_string()],
        None // min_length
    ).unwrap();
    
    // Insert a doc to search for
    collection.insert(serde_json::json!({
        "text": "hello world"
    })).unwrap();
    
    // Verify FULLTEXT search SUCCEEDS (returns Some)
    let search_result_fixed = collection.fulltext_search("text", "hello", 1);
    assert!(search_result_fixed.is_some(), "Search should return Some(...) when correct index exists");
    
    // Check we got results
    let matches = search_result_fixed.unwrap();
    assert!(!matches.is_empty(), "Should find the document we just inserted");

    // Cleanup
    collection.drop_index("broken_ft_index").unwrap();
}
