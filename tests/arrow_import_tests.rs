//! Parquet and Arrow IPC imported into a collection (card #1004): what the
//! export wrote comes back as the documents it was, over HTTP and in the
//! storage layer.
#![cfg(feature = "export-arrow")]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::arrow_export::{export_collection, ExportFormat};
use solidb::storage::arrow_import::{read_file, ImportFormat};
use solidb::storage::{StorageEngine, VectorIndexConfig};
use std::io::Write;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

/// A collection with a field of every kind the export writes.
fn source(engine: &StorageEngine) {
    engine.create_collection("src".to_string(), None).unwrap();
    let src = engine.get_collection("src").unwrap();
    for i in 0..300 {
        src.insert(json!({
            "_key": format!("k{i:03}"),
            "n": i,
            "x": i as f64 + 0.5,
            "name": format!("item {i}"),
            "ok": i % 2 == 0,
            "tags": ["a", i],
            "meta": {"depth": i % 3, "label": "m"},
            "emb": [0.5, 0.25, i as f64],
        }))
        .unwrap();
    }
    src.create_vector_index(VectorIndexConfig::new(
        "emb_idx".to_string(),
        "emb".to_string(),
        3,
    ))
    .unwrap();
}

/// What a document holds, without what the server gives each one.
fn content(mut doc: Value) -> Value {
    for field in ["_id", "_rev", "_created_at", "_updated_at"] {
        doc.as_object_mut().unwrap().remove(field);
    }
    doc
}

fn sorted(engine: &StorageEngine, name: &str) -> Vec<Value> {
    let mut docs: Vec<Value> = engine
        .get_collection(name)
        .unwrap()
        .scan_values(None)
        .into_iter()
        .map(content)
        .collect();
    docs.sort_by(|a, b| a["_key"].as_str().cmp(&b["_key"].as_str()));
    docs
}

#[test]
fn an_export_imports_back_as_the_same_documents() {
    let (engine, tmp) = common::create_test_engine();
    source(&engine);
    let src = engine.get_collection("src").unwrap();
    for (format, name) in [
        (ExportFormat::Parquet, "dst_parquet"),
        (ExportFormat::Arrow, "dst_arrow"),
    ] {
        let path = tmp.path().join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        export_collection(&src, format, &mut file).unwrap();
        file.flush().unwrap();
        let head = std::fs::read(&path).unwrap();
        let sniffed = ImportFormat::sniff(&head).unwrap();

        engine.create_collection(name.to_string(), None).unwrap();
        let dst = engine.get_collection(name).unwrap();
        let rows = read_file(&path, sniffed, |docs| {
            dst.insert_batch(docs)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(rows, 300);
        assert_eq!(sorted(&engine, name), sorted(&engine, "src"), "{name}");
    }
}

async fn import(app: &axum::Router, token: &str, query: &str, file: &[u8]) -> (StatusCode, Value) {
    const BOUNDARY: &str = "arrow-import-boundary";
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"rows\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    let request = Request::builder()
        .method("POST")
        .uri(format!(
            "/_api/database/_system/collection/dst/import{query}"
        ))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn parquet_and_arrow_are_imported_over_http() {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    let app = create_router(
        engine.clone(),
        None,
        None,
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    let admin = common::seed_user_token(&engine, "arrow_import_admin", &["admin"]);
    let db = engine.get_database("_system").unwrap();
    db.create_collection("src".to_string(), None).unwrap();
    let src = db.get_collection("src").unwrap();
    for i in 0..50 {
        src.insert(json!({"_key": format!("k{i}"), "n": i, "name": "x"}))
            .unwrap();
    }
    let mut parquet = Vec::new();
    export_collection(&src, ExportFormat::Parquet, &mut parquet).unwrap();
    let mut arrow = Vec::new();
    export_collection(&src, ExportFormat::Arrow, &mut arrow).unwrap();

    // Parquet, told by its first bytes.
    let (status, answer) = import(&app, &admin, "", &parquet).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["count"], json!(50), "{answer}");
    // Arrow, named; upsert over the same keys.
    let (status, answer) = import(&app, &admin, "?format=arrow&mode=upsert", &arrow).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["count"], json!(50), "{answer}");
    let dst = db.get_collection("dst").unwrap();
    assert_eq!(dst.count(), 50);
    assert_eq!(dst.get("k7").unwrap().to_value()["n"], json!(7));

    let (status, _) = import(&app, &admin, "?format=csv", &parquet).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
