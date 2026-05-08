//! Coverage for `src/server/handlers/blobs.rs` — user-facing upload + download
//! routes (`POST /_api/blob/{db}/{collection}` and
//! `GET /_api/blob/{db}/{collection}/{key}`). See COV-002.
//!
//! The cluster-replication routes under `/_internal/blob/*` are exercised
//! separately in `tests/blob_distribution_tests.rs`.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::auth::AuthService;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

const BOUNDARY: &str = "----CovBoundary42";
const DB: &str = "blobdb";
const COLL: &str = "files";

fn create_app() -> (TempDir, axum::Router, String) {
    let tmp_dir = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).expect("engine");
    engine.initialize().expect("initialize _system");
    let script_stats = Arc::new(ScriptStats::default());
    let router = create_router(engine, None, None, None, None, script_stats, None, None, 0);
    let token =
        AuthService::create_jwt_with_roles("admin_user", Some(vec!["admin".to_string()]), None)
            .expect("admin jwt");
    (tmp_dir, router, token)
}

fn bearer(token: &str) -> String {
    format!("Bearer {}", token)
}

/// Build a multipart body with a single `file` field. `file_name` and
/// `mime` are stamped onto the part's Content-Disposition / Content-Type
/// — matching what the handler reads via `field.file_name()` /
/// `field.content_type()`.
fn multipart_file_body(file_name: &str, mime: &str, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{}\r\n", BOUNDARY).as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n",
            file_name
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {}\r\n\r\n", mime).as_bytes());
    body.extend_from_slice(data);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{}--\r\n", BOUNDARY).as_bytes());
    body
}

async fn create_database(app: &axum::Router, token: &str, name: &str) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, bearer(token))
                .body(Body::from(json!({"name": name}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "db setup failed");
}

async fn create_collection(
    app: &axum::Router,
    token: &str,
    db: &str,
    name: &str,
    coll_type: Option<&str>,
) {
    let mut payload = json!({"name": name});
    if let Some(t) = coll_type {
        payload["type"] = json!(t);
    }
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/_api/database/{}/collection", db))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, bearer(token))
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "collection setup failed");
}

fn upload_request(token: &str, db: &str, coll: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/_api/blob/{}/{}", db, coll))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={}", BOUNDARY),
        )
        .header(header::AUTHORIZATION, bearer(token))
        .body(Body::from(body))
        .unwrap()
}

fn download_request(token: &str, db: &str, coll: &str, key: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(format!("/_api/blob/{}/{}/{}", db, coll, key))
        .header(header::AUTHORIZATION, bearer(token))
        .body(Body::empty())
        .unwrap()
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// ===========================================================================
// upload_blob
// ===========================================================================

#[tokio::test]
async fn upload_blob_happy_path_returns_metadata() {
    let (_tmp, app, token) = create_app();
    create_database(&app, &token, DB).await;
    create_collection(&app, &token, DB, COLL, Some("blob")).await;

    let payload = b"hello cov-002 world";
    let body = multipart_file_body("greeting.txt", "text/plain", payload);

    let resp = app
        .clone()
        .oneshot(upload_request(&token, DB, COLL, body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let meta = body_json(resp).await;
    assert!(!meta["_key"].as_str().unwrap().is_empty());
    assert_eq!(meta["name"], "greeting.txt");
    assert_eq!(meta["type"], "text/plain");
    assert_eq!(meta["size"].as_u64().unwrap(), payload.len() as u64);
    assert!(meta["chunks"].as_u64().unwrap() >= 1);
    assert!(meta["created"].is_string());
}

#[tokio::test]
async fn upload_blob_auto_creates_blob_collection() {
    let (_tmp, app, token) = create_app();
    create_database(&app, &token, DB).await;
    // Note: NOT creating the collection up front — handler should auto-create it.

    let body = multipart_file_body("hi.bin", "application/octet-stream", b"abc");
    let resp = app
        .clone()
        .oneshot(upload_request(&token, DB, "auto_created", body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn upload_blob_rejects_non_blob_collection() {
    let (_tmp, app, token) = create_app();
    create_database(&app, &token, DB).await;
    // Pre-create a *document* collection — upload must refuse.
    create_collection(&app, &token, DB, "docs_only", None).await;

    let body = multipart_file_body("any.txt", "text/plain", b"x");
    let resp = app
        .clone()
        .oneshot(upload_request(&token, DB, "docs_only", body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upload_blob_invalid_content_type_returns_400() {
    let (_tmp, app, token) = create_app();
    create_database(&app, &token, DB).await;
    create_collection(&app, &token, DB, COLL, Some("blob")).await;

    // Wrong content-type — multipart extractor will reject this.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/_api/blob/{}/{}", DB, COLL))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, bearer(&token))
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upload_blob_missing_database_returns_404() {
    let (_tmp, app, token) = create_app();
    let body = multipart_file_body("x.txt", "text/plain", b"x");
    let resp = app
        .clone()
        .oneshot(upload_request(&token, "no_such_db", COLL, body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn upload_blob_strips_unsafe_chars_from_filename() {
    let (_tmp, app, token) = create_app();
    create_database(&app, &token, DB).await;
    create_collection(&app, &token, DB, COLL, Some("blob")).await;

    // Backslash and quote are stripped by sanitize_filename (SEC-166 family).
    let body = multipart_file_body(r"path\to\file.txt", "text/plain", b"d");
    let resp = app
        .clone()
        .oneshot(upload_request(&token, DB, COLL, body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let meta = body_json(resp).await;
    let name = meta["name"].as_str().unwrap();
    assert!(
        !name.contains('\\'),
        "expected backslashes stripped, got {name}"
    );
    assert!(!name.contains('"'), "expected quotes stripped, got {name}");
}

// ===========================================================================
// download_blob — round-trip and error paths
// ===========================================================================

async fn upload_and_get_key(app: &axum::Router, token: &str) -> (String, Vec<u8>) {
    create_database(app, token, DB).await;
    create_collection(app, token, DB, COLL, Some("blob")).await;
    let payload = b"round-trip-bytes-for-cov-002".to_vec();
    let body = multipart_file_body("doc.txt", "text/plain", &payload);
    let resp = app
        .clone()
        .oneshot(upload_request(token, DB, COLL, body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let meta = body_json(resp).await;
    let key = meta["_key"].as_str().unwrap().to_string();
    (key, payload)
}

#[tokio::test]
async fn download_blob_round_trip_bytes_and_headers() {
    let (_tmp, app, token) = create_app();
    let (key, expected) = upload_and_get_key(&app, &token).await;

    let resp = app
        .clone()
        .oneshot(download_request(&token, DB, COLL, &key))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let headers = resp.headers().clone();
    assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "text/plain");
    let cd = headers
        .get(header::CONTENT_DISPOSITION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(cd.starts_with("attachment;"), "got {cd}");
    assert!(cd.contains("doc.txt"), "got {cd}");
    let cl: u64 = headers
        .get(header::CONTENT_LENGTH)
        .unwrap()
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(cl, expected.len() as u64);

    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(bytes.as_ref(), expected.as_slice());
}

#[tokio::test]
async fn download_blob_not_found_returns_404() {
    let (_tmp, app, token) = create_app();
    create_database(&app, &token, DB).await;
    create_collection(&app, &token, DB, COLL, Some("blob")).await;

    let resp = app
        .clone()
        .oneshot(download_request(&token, DB, COLL, "no_such_key"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn download_blob_rejects_non_blob_collection() {
    let (_tmp, app, token) = create_app();
    create_database(&app, &token, DB).await;
    create_collection(&app, &token, DB, "regular", None).await;

    let resp = app
        .clone()
        .oneshot(download_request(&token, DB, "regular", "any_key"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn download_blob_missing_database_returns_404() {
    let (_tmp, app, token) = create_app();
    let resp = app
        .clone()
        .oneshot(download_request(&token, "no_db", COLL, "any"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn download_blob_missing_collection_returns_404() {
    let (_tmp, app, token) = create_app();
    create_database(&app, &token, DB).await;
    let resp = app
        .clone()
        .oneshot(download_request(&token, DB, "missing_coll", "any"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ===========================================================================
// AuthZ — both routes are protected
// ===========================================================================

#[tokio::test]
async fn upload_and_download_require_jwt() {
    let (_tmp, app, _token) = create_app();
    // Upload without auth.
    let body = multipart_file_body("x", "text/plain", b"x");
    let req = Request::builder()
        .method("POST")
        .uri(format!("/_api/blob/{}/{}", DB, COLL))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={}", BOUNDARY),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Download without auth.
    let req = Request::builder()
        .method("GET")
        .uri(format!("/_api/blob/{}/{}/somekey", DB, COLL))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
