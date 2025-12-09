use solidb::storage::engine::StorageEngine;
use solidb::sdbql::executor::QueryExecutor;
use solidb::sdbql::parser::Parser;
use solidb::storage::index::IndexType;
use tempfile::TempDir;
use std::time::Instant;

#[tokio::test]
async fn test_benchmark_sort_limit() {
    let temp_dir = TempDir::new().unwrap();
    let storage = StorageEngine::new(temp_dir.path().to_str().unwrap()).unwrap();
    
    storage.create_collection("bench".to_string(), None).unwrap();
    let collection = storage.get_collection("bench").unwrap();
    
    collection.create_index("idx1".to_string(), "val".to_string(), IndexType::Persistent, false).unwrap();
    
    println!("Inserting 1,000 documents...");
    let start_insert = Instant::now();
    for i in 0..1000 {
        collection.insert(serde_json::json!({"val": i})).unwrap();
    }
    println!("Insert took {:?}", start_insert.elapsed());
    
    // Warmup
    let query_str = "FOR d IN bench SORT d.val ASC LIMIT 1 RETURN d.val";
    let mut parser = Parser::new(query_str).expect("Failed to create parser");
    let query = parser.parse().unwrap();
    let executor = QueryExecutor::new(&storage);
    executor.execute(&query).unwrap();
    
    // Benchmark LIMIT 1 (Min)
    let start = Instant::now();
    let results = executor.execute(&query).unwrap();
    let duration = start.elapsed();
    
    println!("LIMIT 1 (Min) took {:?}", duration);
    println!("Result: {:?}", results);
    assert!(duration.as_micros() < 500, "LIMIT 1 should be sub-millisecond (found {:?})", duration);
    
    // Benchmark LIMIT 1 (Max)
    let query_str_desc = "FOR d IN bench SORT d.val DESC LIMIT 1 RETURN d.val";
    let mut parser_desc = Parser::new(query_str_desc).expect("Failed to create parser");
    let query_desc = parser_desc.parse().unwrap();
    
    let start_desc = Instant::now();
    let results_desc = executor.execute(&query_desc).unwrap();
    let duration_desc = start_desc.elapsed();
    
    println!("LIMIT 1 (Max) took {:?}", duration_desc);
    println!("Result: {:?}", results_desc);
     assert!(duration_desc.as_micros() < 500, "LIMIT 1 DESC should be sub-millisecond (found {:?})", duration_desc);
}
