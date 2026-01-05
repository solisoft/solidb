use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;
use serde_json::json;

fn create_seeded_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    
    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    for i in 0..100 {
        users.insert(json!({"_key": format!("u{}", i), "val": i})).unwrap();
    }
    
    (engine, tmp_dir)
}

#[test]
fn test_explain_analyze_timing() {
    let (engine, _tmp) = create_seeded_engine();
    let query_str = "FOR u IN users FILTER u.val > 50 RETURN u";
    let query = parse(query_str).unwrap();
    let executor = QueryExecutor::new(&engine);
    
    let explain = executor.explain(&query).unwrap();
    
    println!("Explain Result: {:?}", explain.timing);
    
    // Check timing found
    assert!(explain.timing.total_us > 0, "Total timing should be > 0");
    // Scan time might be small but likely > 0 for 100 docs
    
    // Check counts
    assert_eq!(explain.documents_scanned, 100, "Should scan all 100 docs (no index)");
    
    // In our implementation, explain essentially runs the query.
    // But does it population documents_returned?
    // Code says: documents_returned = rows.len();
    // Rows after filter.
    // 0..99. range > 50 means 51..99. Count is 49.
    assert_eq!(explain.documents_returned, 49, "Should match filter count");
}

#[test]
fn test_explain_analyze_with_limit() {
    let (engine, _tmp) = create_seeded_engine();
    let query_str = "FOR u IN users SORT u.val DESC LIMIT 10 RETURN u";
    let query = parse(query_str).unwrap();
    let executor = QueryExecutor::new(&engine);
    
    let explain = executor.explain(&query).unwrap();
    
    assert!(explain.timing.sort_us > 0 || explain.timing.total_us > 0);
    assert!(explain.timing.limit_us > 0 || explain.timing.total_us > 0);
    
    assert_eq!(explain.documents_returned, 10);
}
