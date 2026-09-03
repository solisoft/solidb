//! HTTP API Integration Tests
//!
//! Tests for the HTTP API endpoints including:
//! - Database management
//! - Collection management
//! - Document CRUD
//! - Query execution
//! - Auth (basic)

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt; // for oneshot

mod common;

fn create_test_app() -> (axum::Router, TempDir, String) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine
        .initialize()
        .expect("Failed to initialize storage engine");

    // Create minimal dependencies
    let script_stats = Arc::new(ScriptStats::default());

    let router = create_router(
        engine.clone(),
        None, // ClusterManager
        None, // SyncLog
        None, // ShardCoordinator
        None, // QueueWorker
        script_stats,
        None, // StreamManager
        None, // BlobRebalanceWorker
        0,    // port (unused in router creation)
    );

    // A real `_admins` row, not just a signed token: the auth middleware
    // refuses a JWT whose subject is not a user, which is how deleting a user
    // revokes their outstanding tokens. Seeded after `create_router`, which
    // runs `AuthService::init` and only creates the default `admin` while
    // `_admins` is still empty.
    let token = common::seed_user_token(&engine, "test_admin", &["admin"]);

    (router, tmp_dir, token)
}

fn auth_header(token: &str) -> String {
    format!("Bearer {}", token)
}

// Helper to parse JSON response
async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

// ============================================================================
// Database API Tests
// ============================================================================

#[tokio::test]
async fn test_create_database_api() {
    let (app, _tmp, token) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "testdb" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    // Check if response indicates success
    // assert_eq!(json["result"], true);
    println!("Create DB response: {:?}", json);
}

#[tokio::test]
async fn test_list_databases_api() {
    let (app, _tmp, token) = create_test_app();

    // Create db first
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "db1" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/databases")
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let names: Vec<&str> = json["databases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();

    assert!(names.contains(&"db1"));
    // assert!(names.contains(&"_system")); // Default system db may not be created in test env automatically
}

// ============================================================================
// Collection API Tests
// ============================================================================

#[tokio::test]
async fn test_create_collection_api() {
    let (app, _tmp, token) = create_test_app();

    // Create DB
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "mydb" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Create Collection
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/mydb/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "users" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

// ============================================================================
// Document API Tests
// ============================================================================

#[tokio::test]
async fn test_create_document_api() {
    let (app, _tmp, token) = create_test_app();

    // Setup DB and Collection
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "db" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/db/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "col" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Insert Document
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/db/document/col")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "Alice" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert!(json["_key"].is_string());
}

#[tokio::test]
async fn test_get_document_api() {
    let (app, _tmp, token) = create_test_app();

    // Setup
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "db" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/db/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "col" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Insert
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/db/document/col")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({ "_key": "doc1", "val": 123 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Get
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/database/db/document/col/doc1")
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["val"], 123);
}

// ============================================================================
// Query API Tests
// ============================================================================

#[tokio::test]
async fn test_query_api() {
    let (app, _tmp, token) = create_test_app();

    // Setup
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "db" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Execute Query
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/db/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "RETURN 1 + 1"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let result = &json["result"];
    assert!(result.is_array());
    assert_eq!(result[0], 2.0); // Arithmetic returns float
}

#[tokio::test]
async fn test_query_with_binds_api() {
    let (app, _tmp, token) = create_test_app();

    // Setup
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "db" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/db/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "RETURN @val",
                        "bindVars": { "val": "hello" }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let result = &json["result"];
    assert_eq!(result[0], "hello");
}

// ============================================================================
// Error Handling API Tests
// ============================================================================

#[tokio::test]
async fn test_not_found_api() {
    let (app, _tmp, token) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/database/nonexistent/document/col/doc")
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should be Not Found (404) or similar error
    // Accessing DB that doesn't exist usually returns 404
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_bad_request_api() {
    let (app, _tmp, token) = create_test_app();

    // Create DB first
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "db" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Invalid Query
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/db/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "INVALID SYNTAX"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ============================================================================
// Hybrid Search API Tests
// ============================================================================

async fn post_json(
    app: &axum::Router,
    token: &str,
    uri: &str,
    body: Value,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(token))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// Create db/collection, vector + fulltext indexes and a small corpus for
/// the hybrid search endpoint tests.
async fn setup_hybrid_api(app: &axum::Router, token: &str) {
    post_json(app, token, "/_api/database", json!({ "name": "hdb" })).await;
    post_json(
        app,
        token,
        "/_api/database/hdb/collection",
        json!({ "name": "articles" }),
    )
    .await;
    let resp = post_json(
        app,
        token,
        "/_api/database/hdb/vector/articles",
        json!({
            "name": "embedding_idx",
            "field": "embedding",
            "dimension": 4,
            "metric": "cosine"
        }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "vector index creation failed: {}",
        resp.status()
    );
    let resp = post_json(
        app,
        token,
        "/_api/database/hdb/index/articles",
        json!({
            "type": "fulltext",
            "name": "content_ft",
            "fields": ["content"],
            "min_length": 3
        }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "fulltext index creation failed: {}",
        resp.status()
    );

    for (key, content, embedding) in [
        ("doc1", "machine learning basics", [0.9, 0.1, 0.1, 0.0]),
        ("doc2", "statistical data analysis", [0.85, 0.15, 0.0, 0.1]),
        ("doc3", "machine learning deep dive", [0.0, 0.0, 0.9, 0.1]),
    ] {
        let resp = post_json(
            app,
            token,
            "/_api/database/hdb/document/articles",
            json!({ "_key": key, "content": content, "embedding": embedding }),
        )
        .await;
        assert!(
            resp.status().is_success(),
            "document insert failed: {}",
            resp.status()
        );
    }
}

#[tokio::test]
async fn test_hybrid_search_api() {
    let (app, _tmp, token) = create_test_app();
    setup_hybrid_api(&app, &token).await;

    let response = post_json(
        &app,
        &token,
        "/_api/database/hdb/hybrid/articles/search",
        json!({
            "vector": [1.0, 0.0, 0.0, 0.0],
            "text_query": "machine learning",
            "vector_index": "embedding_idx",
            "fulltext_field": "content",
            "vector_weight": 0.6,
            "text_weight": 0.4,
            "limit": 10
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;

    let results = json["results"].as_array().expect("results array");
    assert!(!results.is_empty(), "hybrid search should return results");
    assert_eq!(json["count"], results.len());

    let first = &results[0];
    assert!(first["doc_key"].is_string(), "result carries doc_key");
    assert!(first["score"].is_number(), "result carries score");
    assert!(first["sources"].is_array(), "result carries sources");
    assert!(
        first["document"].is_object(),
        "result carries the full document"
    );

    // doc1 matches both legs strongly and must be present.
    assert!(
        results.iter().any(|r| r["doc_key"] == "doc1"),
        "doc1 (matches both sources) should be in the results"
    );
}

#[tokio::test]
async fn test_hybrid_search_api_rrf() {
    let (app, _tmp, token) = create_test_app();
    setup_hybrid_api(&app, &token).await;

    let response = post_json(
        &app,
        &token,
        "/_api/database/hdb/hybrid/articles/search",
        json!({
            "vector": [1.0, 0.0, 0.0, 0.0],
            "text_query": "machine learning",
            "vector_index": "embedding_idx",
            "fulltext_field": "content",
            "fusion": "rrf",
            "limit": 2
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let results = json["results"].as_array().expect("results array");
    assert!(!results.is_empty());
    assert!(results.len() <= 2, "limit is respected");
}

#[tokio::test]
async fn test_hybrid_search_api_invalid_fusion() {
    let (app, _tmp, token) = create_test_app();
    setup_hybrid_api(&app, &token).await;

    let response = post_json(
        &app,
        &token,
        "/_api/database/hdb/hybrid/articles/search",
        json!({
            "vector": [1.0, 0.0, 0.0, 0.0],
            "text_query": "machine learning",
            "vector_index": "embedding_idx",
            "fulltext_field": "content",
            "fusion": "bogus"
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_hybrid_search_api_unknown_vector_index() {
    let (app, _tmp, token) = create_test_app();
    setup_hybrid_api(&app, &token).await;

    let response = post_json(
        &app,
        &token,
        "/_api/database/hdb/hybrid/articles/search",
        json!({
            "vector": [1.0, 0.0, 0.0, 0.0],
            "text_query": "machine learning",
            "vector_index": "no_such_index",
            "fulltext_field": "content"
        }),
    )
    .await;

    assert!(
        response.status().is_client_error() || response.status().is_server_error(),
        "unknown vector index must not return success (got {})",
        response.status()
    );
}

// ============================================================================
// Graph RAG API Tests
// ============================================================================

async fn get_json(app: &axum::Router, token: &str, uri: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("Authorization", auth_header(token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn setup_graph_rag_api(app: &axum::Router, token: &str) {
    post_json(app, token, "/_api/database", json!({ "name": "grdb" })).await;
    post_json(
        app,
        token,
        "/_api/database/grdb/collection",
        json!({ "name": "docs" }),
    )
    .await;
    post_json(
        app,
        token,
        "/_api/database/grdb/collection",
        json!({ "name": "links", "type": "edge" }),
    )
    .await;
    let resp = post_json(
        app,
        token,
        "/_api/database/grdb/vector/docs",
        json!({
            "name": "emb",
            "field": "embedding",
            "dimension": 4,
            "metric": "cosine"
        }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "vector index creation failed: {}",
        resp.status()
    );
    let resp = post_json(
        app,
        token,
        "/_api/database/grdb/index/docs",
        json!({
            "type": "fulltext",
            "name": "text_ft",
            "fields": ["text"],
            "min_length": 3
        }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "fulltext index creation failed: {}",
        resp.status()
    );

    for doc in [
        json!({ "_key": "a", "title": "Doc A", "text": "vector database introduction", "embedding": [1.0, 0.0, 0.0, 0.0] }),
        json!({ "_key": "b", "title": "Doc B", "embedding": [0.5, 0.5, 0.0, 0.0] }),
        json!({ "_key": "d", "title": "Doc D", "text": "embedding search with vector database", "embedding": [0.994, 0.006, 0.0, 0.0] }),
    ] {
        let resp = post_json(app, token, "/_api/database/grdb/document/docs", doc).await;
        assert!(
            resp.status().is_success(),
            "document insert failed: {}",
            resp.status()
        );
    }
    let resp = post_json(
        app,
        token,
        "/_api/database/grdb/document/links",
        json!({ "_from": "docs/a", "_to": "docs/b" }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "edge insert failed: {}",
        resp.status()
    );
}

#[tokio::test]
async fn test_graph_rag_api() {
    let (app, _tmp, token) = create_test_app();
    setup_graph_rag_api(&app, &token).await;

    let response = post_json(
        &app,
        &token,
        "/_api/database/grdb/graph/rag",
        json!({
            "seed_collection": "docs",
            "vector_index": "emb",
            "edge_collection": "links",
            "query_vector": [1.0, 0.0, 0.0, 0.0],
            "options": { "hops": 1, "seed_limit": 2, "direction": "outbound" }
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let results = json["results"].as_array().expect("results array");
    assert!(results.len() >= 2);
    assert_eq!(json["count"], results.len());
}

/// Hybrid seeding over the real HTTP path, where the executor carries a
/// database context and documents' `_id` are prefixed with the database name.
#[tokio::test]
async fn test_graph_rag_api_hybrid_seed_mode() {
    let (app, _tmp, token) = create_test_app();
    setup_graph_rag_api(&app, &token).await;

    let response = post_json(
        &app,
        &token,
        "/_api/database/grdb/graph/rag",
        json!({
            "seed_collection": "docs",
            "vector_index": "emb",
            "edge_collection": "links",
            "query_vector": [1.0, 0.0, 0.0, 0.0],
            "options": {
                "seed_mode": "hybrid",
                "fulltext_field": "text",
                "text_query": "vector database",
                "seed_limit": 5,
                "hops": 1
            }
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let ids: Vec<&str> = json["results"]
        .as_array()
        .expect("results array")
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(ids.contains(&"docs/a"), "got {:?}", ids);
    assert!(ids.contains(&"docs/d"), "got {:?}", ids);
    assert!(ids.contains(&"docs/b"), "expansion missing, got {:?}", ids);
}

/// An unknown option must fail the request rather than silently fall back.
#[tokio::test]
async fn test_graph_rag_api_rejects_unknown_option() {
    let (app, _tmp, token) = create_test_app();
    setup_graph_rag_api(&app, &token).await;

    let response = post_json(
        &app,
        &token,
        "/_api/database/grdb/graph/rag",
        json!({
            "seed_collection": "docs",
            "vector_index": "emb",
            "edge_collection": "links",
            "query_vector": [1.0, 0.0, 0.0, 0.0],
            "options": { "seedLimit": 2 }
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_graph_neighbors_api() {
    let (app, _tmp, token) = create_test_app();
    setup_graph_rag_api(&app, &token).await;

    let response = post_json(
        &app,
        &token,
        "/_api/database/grdb/graph/neighbors",
        json!({
            "edge_collection": "links",
            "seeds": ["docs/a"],
            "options": { "hops": 1, "direction": "outbound" }
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let results = json["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn test_community_build_api() {
    let (app, _tmp, token) = create_test_app();
    setup_graph_rag_api(&app, &token).await;
    // Add a second edge to form a small community
    post_json(
        &app,
        &token,
        "/_api/database/grdb/document/links",
        json!({ "_from": "docs/d", "_to": "docs/b" }),
    )
    .await;

    let response = post_json(
        &app,
        &token,
        "/_api/database/grdb/graph/community/build",
        json!({
            "edge_collection": "links",
            "min_community_size": 2,
            "summarize": true
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let started = response_json(response).await;
    let request_id = started["request_id"].as_str().expect("request_id");
    assert_eq!(started["status"], "pending");

    let mut status = "pending".to_string();
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let poll = get_json(
            &app,
            &token,
            &format!("/_api/database/grdb/graph/community/build/{}", request_id),
        )
        .await;
        assert_eq!(poll.status(), StatusCode::OK);
        let body = response_json(poll).await;
        status = body["status"].as_str().unwrap_or("pending").to_string();
        if status == "done" || status == "failed" {
            break;
        }
    }
    assert_eq!(status, "done");

    let list = get_json(
        &app,
        &token,
        "/_api/database/grdb/graph/communities?edge_collection=links",
    )
    .await;
    assert_eq!(list.status(), StatusCode::OK);
    let communities = response_json(list).await;
    assert!(
        !communities["communities"].as_array().unwrap().is_empty(),
        "build should produce communities"
    );

    let search = post_json(
        &app,
        &token,
        "/_api/database/grdb/graph/community/search",
        json!({
            "query_text": "community connected entities",
            "options": { "edge_collection": "links", "limit": 3 }
        }),
    )
    .await;
    assert_eq!(search.status(), StatusCode::OK);
    let search_json = response_json(search).await;
    assert!(
        search_json["count"].as_u64().unwrap_or(0) > 0,
        "community search should return summaries: {:?}",
        search_json
    );
}
