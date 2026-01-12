//! Window Function Tests for SDBQL
//!
//! Tests for SQL-like window functions: ROW_NUMBER, RANK, DENSE_RANK,
//! LAG, LEAD, FIRST_VALUE, LAST_VALUE, and running aggregates.

use serde_json::json;
use solidb::parse;
use solidb::sdbql::QueryExecutor;
use solidb::storage::StorageEngine;
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine =
        StorageEngine::new(tmp_dir.path().to_str().unwrap()).expect("Failed to create storage");
    (engine, tmp_dir)
}

fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect("Failed to parse query");
    let executor = QueryExecutor::new(engine);
    executor.execute(&query).expect("Failed to execute query")
}

fn setup_sales_data(engine: &StorageEngine) {
    engine.create_collection("sales".to_string(), None).unwrap();
    let sales = engine.get_collection("sales").unwrap();

    sales
        .insert(json!({"_key": "s1", "region": "East", "date": "2024-01-01", "amount": 100}))
        .unwrap();
    sales
        .insert(json!({"_key": "s2", "region": "East", "date": "2024-01-02", "amount": 150}))
        .unwrap();
    sales
        .insert(json!({"_key": "s3", "region": "East", "date": "2024-01-03", "amount": 200}))
        .unwrap();
    sales
        .insert(json!({"_key": "s4", "region": "West", "date": "2024-01-01", "amount": 80}))
        .unwrap();
    sales
        .insert(json!({"_key": "s5", "region": "West", "date": "2024-01-02", "amount": 120}))
        .unwrap();
}

// ============================================================================
// ROW_NUMBER Tests
// ============================================================================

#[test]
fn test_row_number_basic() {
    let (engine, _tmp) = create_test_engine();
    setup_sales_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN sales
        SORT doc.date
        RETURN {
            date: doc.date,
            row_num: ROW_NUMBER() OVER (ORDER BY doc.date)
        }
    "#,
    );

    assert_eq!(results.len(), 5);
    // Row numbers should be 1 through 5
    let row_nums: Vec<i64> = results
        .iter()
        .filter_map(|r| r["row_num"].as_i64())
        .collect();
    assert!(row_nums.contains(&1));
    assert!(row_nums.contains(&5));
}

#[test]
fn test_row_number_with_partition() {
    let (engine, _tmp) = create_test_engine();
    setup_sales_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN sales
        RETURN {
            region: doc.region,
            row_num: ROW_NUMBER() OVER (PARTITION BY doc.region ORDER BY doc.date)
        }
    "#,
    );

    // Each partition should have row numbers starting from 1
    let east_rows: Vec<_> = results
        .iter()
        .filter(|r| r["region"] == json!("East"))
        .collect();
    assert_eq!(east_rows.len(), 3);

    let west_rows: Vec<_> = results
        .iter()
        .filter(|r| r["region"] == json!("West"))
        .collect();
    assert_eq!(west_rows.len(), 2);
}

// ============================================================================
// RANK and DENSE_RANK Tests
// ============================================================================

#[test]
fn test_rank_basic() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("scores".to_string(), None)
        .unwrap();
    let scores = engine.get_collection("scores").unwrap();

    scores
        .insert(json!({"_key": "1", "name": "Alice", "score": 100}))
        .unwrap();
    scores
        .insert(json!({"_key": "2", "name": "Bob", "score": 100}))
        .unwrap();
    scores
        .insert(json!({"_key": "3", "name": "Charlie", "score": 90}))
        .unwrap();

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN scores
        RETURN {
            name: doc.name,
            rank: RANK() OVER (ORDER BY doc.score DESC)
        }
    "#,
    );

    assert_eq!(results.len(), 3);
    // Alice and Bob both have rank 1, Charlie has rank 3 (not 2)
    let charlie = results
        .iter()
        .find(|r| r["name"] == json!("Charlie"))
        .unwrap();
    assert_eq!(charlie["rank"], json!(3));
}

#[test]
fn test_dense_rank_with_ties() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("scores".to_string(), None)
        .unwrap();
    let scores = engine.get_collection("scores").unwrap();

    scores
        .insert(json!({"_key": "1", "name": "Alice", "score": 100}))
        .unwrap();
    scores
        .insert(json!({"_key": "2", "name": "Bob", "score": 100}))
        .unwrap();
    scores
        .insert(json!({"_key": "3", "name": "Charlie", "score": 90}))
        .unwrap();

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN scores
        RETURN {
            name: doc.name,
            dense_rank: DENSE_RANK() OVER (ORDER BY doc.score DESC)
        }
    "#,
    );

    // Alice and Bob both have rank 1, Charlie has rank 2 (no gap)
    let charlie = results
        .iter()
        .find(|r| r["name"] == json!("Charlie"))
        .unwrap();
    assert_eq!(charlie["dense_rank"], json!(2));
}

// ============================================================================
// Running Aggregate Tests (SUM, AVG, COUNT)
// ============================================================================

#[test]
fn test_running_sum() {
    let (engine, _tmp) = create_test_engine();
    setup_sales_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN sales
        FILTER doc.region == "East"
        SORT doc.date
        RETURN {
            date: doc.date,
            amount: doc.amount,
            running_total: SUM(doc.amount) OVER (ORDER BY doc.date)
        }
    "#,
    );

    assert_eq!(results.len(), 3);
    // Running totals for East: 100, 250, 450
    let totals: Vec<f64> = results
        .iter()
        .filter_map(|r| r["running_total"].as_f64())
        .collect();
    assert!(totals.contains(&100.0));
    assert!(totals.contains(&250.0));
    assert!(totals.contains(&450.0));
}

#[test]
fn test_running_count() {
    let (engine, _tmp) = create_test_engine();
    setup_sales_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN sales
        FILTER doc.region == "East"
        SORT doc.date
        RETURN {
            date: doc.date,
            running_count: COUNT(doc._key) OVER (ORDER BY doc.date)
        }
    "#,
    );

    assert_eq!(results.len(), 3);
    // Running counts: 1, 2, 3
    let counts: Vec<i64> = results
        .iter()
        .filter_map(|r| r["running_count"].as_i64())
        .collect();
    assert_eq!(counts, vec![1, 2, 3]);
}

#[test]
fn test_running_avg() {
    let (engine, _tmp) = create_test_engine();
    setup_sales_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN sales
        FILTER doc.region == "East"
        SORT doc.date
        RETURN {
            date: doc.date,
            running_avg: AVG(doc.amount) OVER (ORDER BY doc.date)
        }
    "#,
    );

    assert_eq!(results.len(), 3);
    // Running averages: 100, 125, 150
}

// ============================================================================
// LAG and LEAD Tests
// ============================================================================

#[test]
fn test_lag_basic() {
    let (engine, _tmp) = create_test_engine();
    setup_sales_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN sales
        FILTER doc.region == "East"
        SORT doc.date
        RETURN {
            date: doc.date,
            amount: doc.amount,
            prev_amount: LAG(doc.amount) OVER (ORDER BY doc.date)
        }
    "#,
    );

    assert_eq!(results.len(), 3);
    // First row should have prev_amount = null
    let first_row = results
        .iter()
        .find(|r| r["date"] == json!("2024-01-01"))
        .unwrap();
    assert!(first_row["prev_amount"].is_null());
}

#[test]
fn test_lead_basic() {
    let (engine, _tmp) = create_test_engine();
    setup_sales_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN sales
        FILTER doc.region == "East"
        SORT doc.date
        RETURN {
            date: doc.date,
            amount: doc.amount,
            next_amount: LEAD(doc.amount) OVER (ORDER BY doc.date)
        }
    "#,
    );

    assert_eq!(results.len(), 3);
    // Last row should have next_amount = null
    let last_row = results
        .iter()
        .find(|r| r["date"] == json!("2024-01-03"))
        .unwrap();
    assert!(last_row["next_amount"].is_null());
}

// ============================================================================
// FIRST_VALUE and LAST_VALUE Tests
// ============================================================================

#[test]
fn test_first_value() {
    let (engine, _tmp) = create_test_engine();
    setup_sales_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN sales
        RETURN {
            region: doc.region,
            amount: doc.amount,
            first_amount: FIRST_VALUE(doc.amount) OVER (PARTITION BY doc.region ORDER BY doc.date)
        }
    "#,
    );

    // All rows in East should have first_amount = 100
    let east_rows: Vec<_> = results
        .iter()
        .filter(|r| r["region"] == json!("East"))
        .collect();
    for row in east_rows {
        assert_eq!(row["first_amount"], json!(100));
    }
}

#[test]
fn test_last_value() {
    let (engine, _tmp) = create_test_engine();
    setup_sales_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN sales
        RETURN {
            region: doc.region,
            amount: doc.amount,
            last_amount: LAST_VALUE(doc.amount) OVER (PARTITION BY doc.region ORDER BY doc.date)
        }
    "#,
    );

    // All rows in East should have last_amount = 200 (last in partition)
    let east_rows: Vec<_> = results
        .iter()
        .filter(|r| r["region"] == json!("East"))
        .collect();
    for row in east_rows {
        assert_eq!(row["last_amount"], json!(200));
    }
}

// ============================================================================
// Multiple Window Functions Tests
// ============================================================================

#[test]
fn test_multiple_window_functions() {
    let (engine, _tmp) = create_test_engine();
    setup_sales_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN sales
        RETURN {
            region: doc.region,
            date: doc.date,
            amount: doc.amount,
            row_num: ROW_NUMBER() OVER (PARTITION BY doc.region ORDER BY doc.date),
            running_total: SUM(doc.amount) OVER (PARTITION BY doc.region ORDER BY doc.date)
        }
    "#,
    );

    assert_eq!(results.len(), 5);

    // Verify East region has both row numbers and running totals
    let east_rows: Vec<_> = results
        .iter()
        .filter(|r| r["region"] == json!("East"))
        .collect();
    assert_eq!(east_rows.len(), 3);
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_window_function_empty_collection() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("empty".to_string(), None).unwrap();

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN empty
        RETURN {
            row_num: ROW_NUMBER() OVER (ORDER BY doc.date)
        }
    "#,
    );

    assert_eq!(results.len(), 0);
}

#[test]
fn test_window_function_single_row() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("single".to_string(), None)
        .unwrap();
    let single = engine.get_collection("single").unwrap();
    single.insert(json!({"_key": "1", "value": 100})).unwrap();

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN single
        RETURN {
            value: doc.value,
            row_num: ROW_NUMBER() OVER (ORDER BY doc.value),
            running_sum: SUM(doc.value) OVER (ORDER BY doc.value)
        }
    "#,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["row_num"], json!(1));
    assert_eq!(results[0]["running_sum"], json!(100.0));
}

#[test]
fn test_window_function_no_order_by() {
    let (engine, _tmp) = create_test_engine();
    setup_sales_data(&engine);

    // Window function without ORDER BY - should still work
    let results = execute_query(
        &engine,
        r#"
        FOR doc IN sales
        RETURN {
            amount: doc.amount,
            row_num: ROW_NUMBER() OVER ()
        }
    "#,
    );

    // Should have row numbers 1-5, order is undefined but all should exist
    assert_eq!(results.len(), 5);
    let row_nums: Vec<i64> = results
        .iter()
        .filter_map(|r| r["row_num"].as_i64())
        .collect();
    assert_eq!(row_nums.iter().min(), Some(&1));
    assert_eq!(row_nums.iter().max(), Some(&5));
}
