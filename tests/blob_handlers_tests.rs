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
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

const BOUNDARY: &str = "----CovBoundary42";
const DB: &str = "blobdb";
const COLL: &str = "files";

fn create_app() -> (TempDir, axum::Router, String) {
    let (tmp_dir, router, token, _engine) = create_app_with_engine();
    (tmp_dir, router, token)
}

/// `create_app`, keeping the engine so a test can write storage directly —
/// the only way to lay down a blob the way a pre-range-support server did.
fn create_app_with_engine() -> (TempDir, axum::Router, String, StorageEngine) {
    let tmp_dir = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).expect("engine");
    engine.initialize().expect("initialize _system");
    let script_stats = Arc::new(ScriptStats::default());
    let router = create_router(
        engine.clone(),
        None,
        None,
        None,
        None,
        script_stats,
        None,
        None,
        0,
    );
    // A real `_admins` row, not just a signed token: the auth middleware
    // refuses a JWT whose subject is not a user, which is how deleting a user
    // revokes their outstanding tokens. Seeded after `create_router`, which
    // runs `AuthService::init` and only creates the default `admin` while
    // `_admins` is still empty.
    let token = common::seed_user_token(&engine, "admin_user", &["admin"]);
    (tmp_dir, router, token, engine)
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

// ===========================================================================
// download_blob — HTTP Range
// ===========================================================================

/// Deterministic, position-revealing bytes: a wrong offset shows up as a
/// mismatch instead of passing by accident on uniform data.
fn patterned(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn ranged_request(token: &str, key: &str, method: &str, range: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(format!("/_api/blob/{}/{}/{}", DB, COLL, key))
        .header(header::AUTHORIZATION, bearer(token));
    if let Some(r) = range {
        builder = builder.header(header::RANGE, r);
    }
    builder.body(Body::empty()).unwrap()
}

async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(resp.into_body(), 64 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec()
}

fn header_str(resp: &axum::response::Response, name: header::HeaderName) -> Option<String> {
    resp.headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
}

/// Upload `payload` through the multipart route; returns the metadata.
async fn upload_multipart(app: &axum::Router, token: &str, name: &str, payload: &[u8]) -> Value {
    create_database(app, token, DB).await;
    create_collection(app, token, DB, COLL, Some("blob")).await;
    let body = multipart_file_body(name, "audio/mpeg", payload);
    let resp = app
        .clone()
        .oneshot(upload_request(token, DB, COLL, body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await
}

/// GET `range` and check it is a 206 carrying exactly `payload[start..=end]`.
async fn assert_partial(
    app: &axum::Router,
    token: &str,
    key: &str,
    range: &str,
    payload: &[u8],
    start: usize,
    end: usize,
) {
    let resp = app
        .clone()
        .oneshot(ranged_request(token, key, "GET", Some(range)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT, "range {range}");
    assert_eq!(
        header_str(&resp, header::CONTENT_RANGE).as_deref(),
        Some(format!("bytes {}-{}/{}", start, end, payload.len()).as_str()),
        "range {range}"
    );
    assert_eq!(
        header_str(&resp, header::CONTENT_LENGTH),
        Some((end - start + 1).to_string()),
        "range {range}"
    );
    assert_eq!(
        header_str(&resp, header::ACCEPT_RANGES).as_deref(),
        Some("bytes")
    );
    let bytes = body_bytes(resp).await;
    assert_eq!(bytes.len(), end - start + 1, "range {range}");
    assert!(bytes == payload[start..=end], "range {range}: wrong bytes");
}

#[tokio::test]
async fn range_absent_serves_full_200_with_accept_ranges() {
    let (_tmp, app, token) = create_app();
    let payload = patterned(100);
    let meta = upload_multipart(&app, &token, "ep.mp3", &payload).await;
    let key = meta["_key"].as_str().unwrap();

    let resp = app
        .clone()
        .oneshot(ranged_request(&token, key, "GET", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        header_str(&resp, header::ACCEPT_RANGES).as_deref(),
        Some("bytes")
    );
    assert_eq!(
        header_str(&resp, header::CONTENT_LENGTH),
        Some("100".into())
    );
    assert_eq!(
        header_str(&resp, header::CONTENT_TYPE).as_deref(),
        Some("audio/mpeg")
    );
    assert!(resp.headers().get(header::CONTENT_RANGE).is_none());
    assert_eq!(body_bytes(resp).await, payload);
}

#[tokio::test]
async fn range_first_ten_bytes() {
    let (_tmp, app, token) = create_app();
    let payload = patterned(100);
    let meta = upload_multipart(&app, &token, "ep.mp3", &payload).await;
    let key = meta["_key"].as_str().unwrap();

    assert_partial(&app, &token, key, "bytes=0-9", &payload, 0, 9).await;

    // Same content type and disposition as the full download.
    let resp = app
        .clone()
        .oneshot(ranged_request(&token, key, "GET", Some("bytes=0-9")))
        .await
        .unwrap();
    assert_eq!(
        header_str(&resp, header::CONTENT_TYPE).as_deref(),
        Some("audio/mpeg")
    );
    let cd = header_str(&resp, header::CONTENT_DISPOSITION).unwrap();
    assert!(
        cd.starts_with("attachment;") && cd.contains("ep.mp3"),
        "{cd}"
    );
}

#[tokio::test]
async fn range_open_ended_and_suffix() {
    let (_tmp, app, token) = create_app();
    let payload = patterned(100);
    let meta = upload_multipart(&app, &token, "ep.mp3", &payload).await;
    let key = meta["_key"].as_str().unwrap();

    assert_partial(&app, &token, key, "bytes=10-", &payload, 10, 99).await;
    assert_partial(&app, &token, key, "bytes=-5", &payload, 95, 99).await;
    // Ends past the blob are clamped, a suffix longer than it is the whole.
    assert_partial(&app, &token, key, "bytes=90-1000", &payload, 90, 99).await;
    assert_partial(&app, &token, key, "bytes=-1000", &payload, 0, 99).await;
    assert_partial(&app, &token, key, "bytes=99-99", &payload, 99, 99).await;
}

#[tokio::test]
async fn range_on_multipart_upload_stored_in_uniform_chunks() {
    let (_tmp, app, token) = create_app();
    const MIB: usize = 1024 * 1024;
    let payload = patterned(2 * MIB + MIB / 2);
    let meta = upload_multipart(&app, &token, "big.mp3", &payload).await;
    let key = meta["_key"].as_str().unwrap();

    // Re-cut into 1 MiB chunks, recorded as one number.
    assert_eq!(meta["chunks"], 3);
    assert_eq!(meta["chunk_size"], MIB as u64);
    assert!(meta.get("chunk_sizes").is_none());

    // Spanning all three chunks, inside the last one, and across one edge.
    assert_partial(
        &app,
        &token,
        key,
        "bytes=1048570-2097160",
        &payload,
        MIB - 6,
        2 * MIB + 8,
    )
    .await;
    assert_partial(
        &app,
        &token,
        key,
        "bytes=2200000-2200099",
        &payload,
        2_200_000,
        2_200_099,
    )
    .await;
    assert_partial(
        &app,
        &token,
        key,
        "bytes=1048576-1048576",
        &payload,
        MIB,
        MIB,
    )
    .await;
    assert_partial(
        &app,
        &token,
        key,
        "bytes=-10",
        &payload,
        payload.len() - 10,
        payload.len() - 1,
    )
    .await;

    let resp = app
        .clone()
        .oneshot(ranged_request(&token, key, "GET", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(body_bytes(resp).await == payload);
}

/// Resumable upload with chunks of the sizes given — the client chooses, and
/// nothing forces them to match the declared `chunk_size`.
async fn upload_resumable(app: &axum::Router, token: &str, chunks: &[&[u8]]) -> Value {
    create_database(app, token, DB).await;
    create_collection(app, token, DB, COLL, Some("blob")).await;
    let total: usize = chunks.iter().map(|c| c.len()).sum();
    let chunk_size = 64 * 1024;
    assert_eq!(total.div_ceil(chunk_size), chunks.len(), "fixture shape");

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/_api/blob/{}/{}/upload", DB, COLL))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, bearer(token))
                .body(Body::from(
                    json!({
                        "file_name": "ep.m4a",
                        "mime_type": "audio/mp4",
                        "total_size": total,
                        "chunk_size": chunk_size,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let session = body_json(resp).await;
    let upload_id = session["upload_id"].as_str().unwrap().to_string();

    for (i, data) in chunks.iter().enumerate() {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/_api/blob/{}/{}/upload/{}/{}",
                        DB, COLL, upload_id, i
                    ))
                    .header(header::CONTENT_TYPE, "application/octet-stream")
                    .header(header::AUTHORIZATION, bearer(token))
                    .body(Body::from(data.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "chunk {i}");
    }

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/_api/blob/{}/{}/upload/{}/complete",
                    DB, COLL, upload_id
                ))
                .header(header::AUTHORIZATION, bearer(token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await
}

#[tokio::test]
async fn range_spanning_chunks_of_different_sizes() {
    let (_tmp, app, token) = create_app();
    let payload = patterned(200_000);
    // 70000 + 30000 + 60000 + 40000: irregular, so recorded as a list.
    let parts: Vec<&[u8]> = vec![
        &payload[0..70_000],
        &payload[70_000..100_000],
        &payload[100_000..160_000],
        &payload[160_000..200_000],
    ];
    let meta = upload_resumable(&app, &token, &parts).await;
    let key = meta["_key"].as_str().unwrap();
    assert_eq!(meta["chunks"], 4);
    assert_eq!(meta["chunk_sizes"], json!([70_000, 30_000, 60_000, 40_000]));

    // Across chunks 0..=2, across 1..=3, inside chunk 2, and on the edges.
    assert_partial(
        &app,
        &token,
        key,
        "bytes=69990-100010",
        &payload,
        69_990,
        100_010,
    )
    .await;
    assert_partial(
        &app,
        &token,
        key,
        "bytes=99999-160000",
        &payload,
        99_999,
        160_000,
    )
    .await;
    assert_partial(
        &app,
        &token,
        key,
        "bytes=110000-110099",
        &payload,
        110_000,
        110_099,
    )
    .await;
    assert_partial(
        &app,
        &token,
        key,
        "bytes=70000-99999",
        &payload,
        70_000,
        99_999,
    )
    .await;
    assert_partial(
        &app,
        &token,
        key,
        "bytes=-40001",
        &payload,
        159_999,
        199_999,
    )
    .await;

    let resp = app
        .clone()
        .oneshot(ranged_request(&token, key, "GET", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(body_bytes(resp).await == payload);
}

#[tokio::test]
async fn range_on_uniform_resumable_upload_records_one_size() {
    let (_tmp, app, token) = create_app();
    let payload = patterned(3 * 65_536 + 100);
    let parts: Vec<&[u8]> = payload.chunks(65_536).collect();
    let meta = upload_resumable(&app, &token, &parts).await;
    let key = meta["_key"].as_str().unwrap();
    assert_eq!(meta["chunk_size"], 65_536);
    assert!(meta.get("chunk_sizes").is_none());

    assert_partial(
        &app,
        &token,
        key,
        "bytes=65530-196700",
        &payload,
        65_530,
        196_700,
    )
    .await;
    assert_partial(
        &app,
        &token,
        key,
        "bytes=196608-",
        &payload,
        196_608,
        payload.len() - 1,
    )
    .await;
}

/// Write a blob the way a server without range support left it: chunks of
/// assorted sizes and a document with no layout fields.
fn store_old_style_blob(engine: &StorageEngine, key: &str, chunks: &[&[u8]], doc: Value) {
    let db = engine.get_database(DB).unwrap();
    let coll = db.get_collection(COLL).unwrap();
    for (i, data) in chunks.iter().enumerate() {
        coll.put_blob_chunk(key, i as u32, data).unwrap();
    }
    coll.insert(doc).unwrap();
}

#[tokio::test]
async fn range_on_old_blob_without_recorded_sizes() {
    let (_tmp, app, token, engine) = create_app_with_engine();
    create_database(&app, &token, DB).await;
    create_collection(&app, &token, DB, COLL, Some("blob")).await;

    let payload = patterned(1000);
    let parts: Vec<&[u8]> = vec![
        &payload[0..7],
        &payload[7..300],
        &payload[300..301],
        &payload[301..750],
        &payload[750..1000],
    ];
    store_old_style_blob(
        &engine,
        "legacy",
        &parts,
        json!({
            "_key": "legacy",
            "name": "old.mp3",
            "type": "audio/mpeg",
            "size": 1000,
            "chunks": 5,
        }),
    );

    assert_partial(&app, &token, "legacy", "bytes=0-9", &payload, 0, 9).await;
    assert_partial(&app, &token, "legacy", "bytes=5-760", &payload, 5, 760).await;
    assert_partial(&app, &token, "legacy", "bytes=400-499", &payload, 400, 499).await;
    assert_partial(&app, &token, "legacy", "bytes=300-300", &payload, 300, 300).await;
    assert_partial(&app, &token, "legacy", "bytes=-5", &payload, 995, 999).await;
    assert_partial(&app, &token, "legacy", "bytes=10-", &payload, 10, 999).await;
}

#[tokio::test]
async fn range_with_a_layout_that_lies_fails_instead_of_sending_wrong_bytes() {
    let (_tmp, app, token, engine) = create_app_with_engine();
    create_database(&app, &token, DB).await;
    create_collection(&app, &token, DB, COLL, Some("blob")).await;

    let payload = patterned(10);
    // Stored as 3 + 5 + 2, but the document claims uniform 4-byte chunks —
    // consistent with `size` and `chunks`, so it is believed.
    store_old_style_blob(
        &engine,
        "liar",
        &[&payload[0..3], &payload[3..8], &payload[8..10]],
        json!({"_key": "liar", "type": "audio/mpeg", "size": 10, "chunks": 3, "chunk_size": 4}),
    );

    let resp = app
        .clone()
        .oneshot(ranged_request(&token, "liar", "GET", Some("bytes=4-6")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert!(
        axum::body::to_bytes(resp.into_body(), 1024).await.is_err(),
        "a chunk contradicting its layout must abort the body"
    );
}

#[tokio::test]
async fn range_past_the_end_is_416() {
    let (_tmp, app, token) = create_app();
    let payload = patterned(100);
    let meta = upload_multipart(&app, &token, "ep.mp3", &payload).await;
    let key = meta["_key"].as_str().unwrap();

    for range in ["bytes=100-", "bytes=100-200", "bytes=-0"] {
        let resp = app
            .clone()
            .oneshot(ranged_request(&token, key, "GET", Some(range)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE, "{range}");
        assert_eq!(
            header_str(&resp, header::CONTENT_RANGE).as_deref(),
            Some("bytes */100"),
            "{range}"
        );
        assert!(body_bytes(resp).await.is_empty());
    }
}

#[tokio::test]
async fn malformed_or_multi_range_serves_full_200() {
    let (_tmp, app, token) = create_app();
    let payload = patterned(100);
    let meta = upload_multipart(&app, &token, "ep.mp3", &payload).await;
    let key = meta["_key"].as_str().unwrap();

    for range in [
        "bytes=5-2",
        "bytes=0-1,4-5",
        "items=0-3",
        "garbage",
        "bytes=x-y",
    ] {
        let resp = app
            .clone()
            .oneshot(ranged_request(&token, key, "GET", Some(range)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{range}");
        assert!(resp.headers().get(header::CONTENT_RANGE).is_none());
        assert_eq!(
            header_str(&resp, header::CONTENT_LENGTH),
            Some("100".into())
        );
        assert_eq!(body_bytes(resp).await, payload, "{range}");
    }
}

#[tokio::test]
async fn range_under_if_range_serves_full_200() {
    // No validator is ever sent, so no If-Range can match: RFC 9110 says
    // send the whole representation.
    let (_tmp, app, token) = create_app();
    let payload = patterned(100);
    let meta = upload_multipart(&app, &token, "ep.mp3", &payload).await;
    let key = meta["_key"].as_str().unwrap();

    let mut req = ranged_request(&token, key, "GET", Some("bytes=0-9"));
    req.headers_mut()
        .insert(header::IF_RANGE, "\"some-etag\"".parse().unwrap());
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_bytes(resp).await, payload);
}

#[tokio::test]
async fn head_reports_size_type_and_ranges_without_body() {
    let (_tmp, app, token) = create_app();
    let payload = patterned(100);
    let meta = upload_multipart(&app, &token, "ep.mp3", &payload).await;
    let key = meta["_key"].as_str().unwrap();

    // HEAD ignores Range (defined for GET only) and describes the whole blob.
    for range in [None, Some("bytes=0-9")] {
        let resp = app
            .clone()
            .oneshot(ranged_request(&token, key, "HEAD", range))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            header_str(&resp, header::CONTENT_LENGTH),
            Some("100".into())
        );
        assert_eq!(
            header_str(&resp, header::CONTENT_TYPE).as_deref(),
            Some("audio/mpeg")
        );
        assert_eq!(
            header_str(&resp, header::ACCEPT_RANGES).as_deref(),
            Some("bytes")
        );
        assert!(body_bytes(resp).await.is_empty());
    }
}
