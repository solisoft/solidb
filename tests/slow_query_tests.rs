//! Slow Query Logging Tests
//!
//! Tests for the slow query logging feature that logs queries
//! exceeding the SLOW_QUERY_THRESHOLD_MS to _slow_queries collection.

use axum::{body::Body, http::Request};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

fn create_test_app() -> (axum::Router, TempDir, String) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");

    let script_stats = Arc::new(ScriptStats::default());

    let router = create_router(
        engine,
        None, // ClusterManager
        None, // SyncLog
        None, // ShardCoordinator
        None, // QueueWorker
        script_stats,
        None, // StreamManager
        None, // BlobRebalanceWorker
        0,    // port
    );

    let token = solidb::server::auth::AuthService::create_jwt_with_roles(
        "test_admin",
        Some(vec!["admin".to_string()]),
        None,
    )
    .expect("Failed to create test token");

    (router, tmp_dir, token)
}

fn auth_header(token: &str) -> String {
    format!("Bearer {}", token)
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn setup_db(app: &axum::Router, token: &str, db_name: &str) {
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(token))
                .body(Body::from(json!({ "name": db_name }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
}

async fn setup_collection(app: &axum::Router, token: &str, db_name: &str, coll_name: &str) {
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/_api/database/{}/collection", db_name))
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(token))
                .body(Body::from(json!({ "name": coll_name }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
}

async fn execute_query(app: &axum::Router, token: &str, db_name: &str, query: &str) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/_api/database/{}/cursor", db_name))
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(token))
                .body(Body::from(json!({ "query": query }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    response_json(response).await
}

async fn get_slow_queries(app: &axum::Router, token: &str, db_name: &str) -> Vec<Value> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/_api/database/{}/cursor", db_name))
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(token))
                .body(Body::from(
                    json!({ "query": "FOR sq IN _slow_queries RETURN sq" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let json = response_json(response).await;
    json["result"].as_array().cloned().unwrap_or_default()
}

// ============================================================================
// Slow Query Logging Tests
// ============================================================================

#[tokio::test]
async fn test_fast_query_not_logged() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, &token, "fastdb").await;

    // Execute a simple fast query
    let result = execute_query(&app, &token, "fastdb", "RETURN 1 + 1").await;
    assert_eq!(result["result"][0], 2.0);

    // Wait for async logging to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Check _slow_queries - should be empty or not exist
    let slow_queries = get_slow_queries(&app, &token, "fastdb").await;
    assert!(
        slow_queries.is_empty(),
        "Fast query should not be logged to _slow_queries"
    );
}

#[tokio::test]
async fn test_slow_query_logged() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, &token, "slowdb").await;
    setup_collection(&app, &token, "slowdb", "data").await;

    // Insert many documents to create a slow query scenario
    // Note: The threshold is 100ms, so we need a query that takes longer
    let insert_query = r#"
        FOR i IN 1..10000
            INSERT { value: i, data: REPEAT("x", 100) } INTO data
    "#;

    let result = execute_query(&app, &token, "slowdb", insert_query).await;
    let exec_time = result["executionTimeMs"].as_f64().unwrap_or(0.0);
    println!("Insert query execution time: {}ms", exec_time);

    // Wait for async logging to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Only check for slow query log if the query actually took > 100ms
    if exec_time >= 100.0 {
        let slow_queries = get_slow_queries(&app, &token, "slowdb").await;
        assert!(
            !slow_queries.is_empty(),
            "Slow query ({}ms) should be logged to _slow_queries",
            exec_time
        );

        // Verify slow query document structure
        let sq = &slow_queries[0];
        assert!(sq.get("query").is_some(), "Should have query field");
        assert!(
            sq.get("execution_time_ms").is_some(),
            "Should have execution_time_ms field"
        );
        assert!(sq.get("timestamp").is_some(), "Should have timestamp field");
        assert!(
            sq.get("results_count").is_some(),
            "Should have results_count field"
        );
        assert!(
            sq.get("documents_inserted").is_some(),
            "Should have documents_inserted field"
        );
    } else {
        println!(
            "Query was faster than threshold ({}ms < 100ms), skipping assertion",
            exec_time
        );
    }
}

#[tokio::test]
async fn test_slow_query_document_structure() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, &token, "structdb").await;
    setup_collection(&app, &token, "structdb", "items").await;

    // Insert enough documents to make a scan slow
    for batch in 0..20 {
        let query = format!(
            "FOR i IN {}..{} INSERT {{ idx: i, payload: REPEAT('data', 50) }} INTO items",
            batch * 500,
            (batch + 1) * 500 - 1
        );
        execute_query(&app, &token, "structdb", &query).await;
    }

    // Now run a full scan query that should be slow
    let scan_query = "FOR item IN items FILTER item.payload != null RETURN item";
    let result = execute_query(&app, &token, "structdb", scan_query).await;
    let exec_time = result["executionTimeMs"].as_f64().unwrap_or(0.0);
    println!("Scan query execution time: {}ms", exec_time);

    // Wait for async logging
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    if exec_time >= 100.0 {
        let slow_queries = get_slow_queries(&app, &token, "structdb").await;

        // Find the scan query in slow_queries
        let scan_log = slow_queries.iter().find(|sq| {
            sq.get("query")
                .and_then(|q| q.as_str())
                .map(|q| q.contains("FILTER item.payload"))
                .unwrap_or(false)
        });

        if let Some(sq) = scan_log {
            // Verify document structure
            assert!(sq["query"].is_string());
            assert!(sq["execution_time_ms"].is_f64());
            assert!(sq["timestamp"].is_string());
            assert!(sq["results_count"].is_u64() || sq["results_count"].is_i64());
            assert!(sq["documents_inserted"].is_u64() || sq["documents_inserted"].is_i64());
            assert!(sq["documents_updated"].is_u64() || sq["documents_updated"].is_i64());
            assert!(sq["documents_removed"].is_u64() || sq["documents_removed"].is_i64());

            println!("Slow query logged: {:?}", sq);
        }
    }
}

#[tokio::test]
async fn test_slow_queries_collection_created_automatically() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, &token, "autodb").await;
    setup_collection(&app, &token, "autodb", "test").await;

    // Run a query that may or may not be slow
    let query = "FOR i IN 1..5000 INSERT { x: i } INTO test";
    let result = execute_query(&app, &token, "autodb", query).await;
    let exec_time = result["executionTimeMs"].as_f64().unwrap_or(0.0);
    println!("Insert query execution time: {}ms", exec_time);

    // Wait for potential async logging
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Try to query the _slow_queries collection
    let slow_queries = get_slow_queries(&app, &token, "autodb").await;

    // If query was slow (>= 100ms), collection should exist and have entries
    // If query was fast (< 100ms), collection might not exist (empty result)
    if exec_time >= 100.0 {
        assert!(
            !slow_queries.is_empty(),
            "Slow query ({}ms) should have created _slow_queries collection with entry",
            exec_time
        );
    } else {
        // Fast query - collection might or might not exist
        println!(
            "Query was fast ({}ms < 100ms), _slow_queries may not exist. Found {} entries.",
            exec_time,
            slow_queries.len()
        );
    }
}

#[tokio::test]
async fn test_multiple_slow_queries_logged() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, &token, "multidb").await;
    setup_collection(&app, &token, "multidb", "docs").await;

    // Run multiple potentially slow queries
    let queries = vec![
        "FOR i IN 1..3000 INSERT { n: i } INTO docs",
        "FOR i IN 1..3000 INSERT { n: i + 3000 } INTO docs",
        "FOR d IN docs FILTER d.n > 1000 RETURN d",
    ];

    let mut slow_count: usize = 0;
    for query in &queries {
        let result = execute_query(&app, &token, "multidb", query).await;
        let exec_time = result["executionTimeMs"].as_f64().unwrap_or(0.0);
        if exec_time >= 100.0 {
            slow_count += 1;
        }
        println!("Query execution time: {}ms", exec_time);
    }

    // Wait for async logging
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    let slow_queries = get_slow_queries(&app, &token, "multidb").await;
    println!(
        "Expected {} slow queries, found {}",
        slow_count,
        slow_queries.len()
    );

    // The number of logged slow queries should match queries that exceeded threshold
    // (with some tolerance for timing variations)
    assert!(
        slow_queries.len() >= slow_count.saturating_sub(1),
        "Should log slow queries"
    );
}
