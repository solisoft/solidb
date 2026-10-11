//! Query result cache vs. DDL.
//!
//! Regression tests for a cached `/cursor` answer outliving the data it was
//! computed from: dropping a database (or a collection) and recreating it
//! under the same name, or truncating a collection, must not let the next
//! identical query be answered from the cache with the old rows.
//!
//! The HTTP tests reproduce the reported sequence end to end; the storage
//! tests drive `StorageEngine` / `Database` / `Collection` directly, which is
//! the path replication apply, the driver protocol and Lua take, none of
//! which goes through the HTTP handlers.
//!
//! The result cache is process-global, its key does not identify the engine,
//! and a write to a collection invalidates that collection name in every
//! database: every test uses its own database *and* collection name so that
//! parallel tests cannot evict each other's entries.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::sdbql::QueryPrincipal;
use solidb::server::routes::create_router;
use solidb::storage::{query_cache, StorageEngine};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

fn query_text(coll: &str) -> String {
    format!("FOR p IN {coll} FILTER p.object == @o RETURN p._key")
}

fn create_test_app() -> (axum::Router, TempDir, String) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine
        .initialize()
        .expect("Failed to initialize storage engine");

    let router = create_router(
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
    let token = common::seed_user_token(&engine, "test_admin", &["admin"]);
    (router, tmp_dir, token)
}

async fn call(
    app: &axum::Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", token))
        .body(match body {
            Some(b) => Body::from(b.to_string()),
            None => Body::empty(),
        })
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

async fn create_database(app: &axum::Router, token: &str, db: &str) {
    let (status, body) = call(
        app,
        token,
        "POST",
        "/_api/database",
        Some(json!({ "name": db })),
    )
    .await;
    assert!(status.is_success(), "create database {db}: {status} {body}");
}

async fn create_layouts(app: &axum::Router, token: &str, db: &str, coll: &str) {
    let (status, body) = call(
        app,
        token,
        "POST",
        &format!("/_api/database/{db}/collection"),
        Some(json!({ "name": coll })),
    )
    .await;
    assert!(status.is_success(), "create collection: {status} {body}");
}

async fn insert_ghost(app: &axum::Router, token: &str, db: &str, coll: &str) {
    let (status, body) = call(
        app,
        token,
        "POST",
        &format!("/_api/database/{db}/document/{coll}"),
        Some(json!({ "_key": "ghost1", "object": "accounts" })),
    )
    .await;
    assert!(status.is_success(), "insert: {status} {body}");
}

async fn run_query(app: &axum::Router, token: &str, db: &str, coll: &str) -> Value {
    let (status, body) = call(
        app,
        token,
        "POST",
        &format!("/_api/database/{db}/cursor"),
        Some(json!({ "query": query_text(coll), "bindVars": { "o": "accounts" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "cursor: {body}");
    body
}

/// Create `db.coll` holding `ghost1` and run the query until its answer
/// comes from the cache. A concurrent write anywhere in the process bumps the
/// cache generation and stops a result from being stored, so one repetition
/// is not always enough when tests run in parallel.
async fn seed_and_cache(app: &axum::Router, token: &str, db: &str, coll: &str) {
    create_database(app, token, db).await;
    create_layouts(app, token, db, coll).await;
    insert_ghost(app, token, db, coll).await;

    for _ in 0..50 {
        let body = run_query(app, token, db, coll).await;
        assert_eq!(body["result"], json!(["ghost1"]));
        if body["cached"] == json!(true) {
            return;
        }
    }
    panic!("the query never came back from the cache");
}

fn assert_fresh_and_empty(body: &Value, what: &str) {
    assert_eq!(
        body["result"],
        json!([]),
        "{what}: the query answered with rows that no longer exist: {body}"
    );
    assert_ne!(body["cached"], json!(true), "{what}: served from cache");
}

#[tokio::test]
async fn database_drop_and_recreate_does_not_serve_cached_rows() {
    let (app, _tmp, token) = create_test_app();
    let (db, coll) = ("qc_drop_db", "qc_drop_db_layouts");
    seed_and_cache(&app, &token, db, coll).await;

    let (status, _) = call(
        &app,
        &token,
        "DELETE",
        &format!("/_api/database/{db}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    create_database(&app, &token, db).await;
    create_layouts(&app, &token, db, coll).await;

    let body = run_query(&app, &token, db, coll).await;
    assert_fresh_and_empty(&body, "database recreated");
}

#[tokio::test]
async fn collection_drop_and_recreate_does_not_serve_cached_rows() {
    let (app, _tmp, token) = create_test_app();
    let (db, coll) = ("qc_drop_coll", "qc_drop_coll_layouts");
    seed_and_cache(&app, &token, db, coll).await;

    let (status, body) = call(
        &app,
        &token,
        "DELETE",
        &format!("/_api/database/{db}/collection/{coll}"),
        None,
    )
    .await;
    assert!(status.is_success(), "drop collection: {status} {body}");

    create_layouts(&app, &token, db, coll).await;

    let body = run_query(&app, &token, db, coll).await;
    assert_fresh_and_empty(&body, "collection recreated");
}

#[tokio::test]
async fn truncate_does_not_serve_cached_rows() {
    let (app, _tmp, token) = create_test_app();
    let (db, coll) = ("qc_truncate", "qc_truncate_layouts");
    seed_and_cache(&app, &token, db, coll).await;

    let (status, body) = call(
        &app,
        &token,
        "PUT",
        &format!("/_api/database/{db}/collection/{coll}/truncate"),
        None,
    )
    .await;
    assert!(status.is_success(), "truncate: {status} {body}");

    let body = run_query(&app, &token, db, coll).await;
    assert_fresh_and_empty(&body, "collection truncated");
}

// ============================================================================
// Storage layer: paths that never reach the HTTP handlers
// ============================================================================

/// Create `db.coll` holding `ghost1`.
fn seed(engine: &StorageEngine, db: &str, coll: &str) {
    engine.create_database(db.to_string()).unwrap();
    let database = engine.get_database(db).unwrap();
    database.create_collection(coll.to_string(), None).unwrap();
    database
        .get_collection(coll)
        .unwrap()
        .insert(json!({ "_key": "ghost1", "object": "accounts" }))
        .unwrap();
}

/// Cache an answer under the key `/cursor` would use for `db.coll`, and
/// return that key.
fn cache_answer(engine: &StorageEngine, db: &str, coll: &str) -> String {
    let text = query_text(coll);
    let query = solidb::parse(&text).unwrap();
    let bind_vars: HashMap<String, Value> = HashMap::from([("o".to_string(), json!("accounts"))]);
    let key = query_cache::cache_key_for(
        engine,
        db,
        &text,
        &query,
        &bind_vars,
        &QueryPrincipal::default(),
    )
    .expect("query should be cacheable");
    query_cache::get_query_cache().put(key.clone(), vec![json!("ghost1")]);
    assert!(query_cache::get_query_cache().get(&key).is_some());
    key
}

fn assert_evicted(key: &str, what: &str) {
    assert!(
        query_cache::get_query_cache().get(key).is_none(),
        "{what}: the cached answer survived"
    );
}

fn test_engine() -> (StorageEngine, TempDir) {
    let (engine, tmp) = common::create_test_engine();
    engine.initialize().unwrap();
    (engine, tmp)
}

#[test]
fn storage_delete_database_drops_its_cached_answers() {
    let (engine, _tmp) = test_engine();
    let (db, coll) = ("qc_storage_db", "qc_storage_db_layouts");
    seed(&engine, db, coll);
    let key = cache_answer(&engine, db, coll);

    engine.delete_database(db).unwrap();
    assert_evicted(&key, "database dropped");

    // Recreated, the same query resolves to the same key again: still absent.
    engine.create_database(db.to_string()).unwrap();
    engine
        .get_database(db)
        .unwrap()
        .create_collection(coll.to_string(), None)
        .unwrap();
    assert_evicted(&key, "database recreated");
}

#[test]
fn storage_delete_database_keeps_other_databases_answers() {
    let (engine, _tmp) = test_engine();
    let coll = "qc_storage_shared_layouts";
    // A database whose name extends the dropped one: a bare prefix match
    // would reach it.
    let (doomed_db, kept_db) = ("qc_storage_doomed", "qc_storage_doomed2");
    seed(&engine, doomed_db, coll);
    seed(&engine, kept_db, coll);
    let doomed = cache_answer(&engine, doomed_db, coll);
    let kept = cache_answer(&engine, kept_db, coll);

    engine.delete_database(doomed_db).unwrap();
    assert_evicted(&doomed, "database dropped");
    assert!(
        query_cache::get_query_cache().get(&kept).is_some(),
        "dropping {doomed_db} evicted {kept_db}'s answers"
    );
}

#[test]
fn storage_delete_collection_drops_its_cached_answers() {
    let (engine, _tmp) = test_engine();
    let (db, coll) = ("qc_storage_coll", "qc_storage_coll_layouts");
    seed(&engine, db, coll);
    let key = cache_answer(&engine, db, coll);

    engine
        .get_database(db)
        .unwrap()
        .delete_collection(coll)
        .unwrap();
    assert_evicted(&key, "collection dropped");
}

#[test]
fn storage_create_collection_drops_stale_answers() {
    let (engine, _tmp) = test_engine();
    let (db, coll) = ("qc_storage_create", "qc_storage_create_layouts");
    seed(&engine, db, coll);
    let key = cache_answer(&engine, db, coll);
    let database = engine.get_database(db).unwrap();
    database.delete_collection(coll).unwrap();
    // An entry that outlived its collection by some other path.
    query_cache::get_query_cache().put(key.clone(), vec![json!("ghost1")]);

    database.create_collection(coll.to_string(), None).unwrap();
    assert_evicted(&key, "collection created");
}

#[test]
fn storage_truncate_drops_its_cached_answers() {
    let (engine, _tmp) = test_engine();
    let (db, coll) = ("qc_storage_trunc", "qc_storage_trunc_layouts");
    seed(&engine, db, coll);
    let key = cache_answer(&engine, db, coll);

    engine
        .get_database(db)
        .unwrap()
        .get_collection(coll)
        .unwrap()
        .truncate()
        .unwrap();
    assert_evicted(&key, "collection truncated");
}

#[test]
fn storage_drop_index_drops_its_cached_answers() {
    let (engine, _tmp) = test_engine();
    let (db, coll) = ("qc_storage_idx", "qc_storage_idx_layouts");
    seed(&engine, db, coll);
    let collection = engine
        .get_database(db)
        .unwrap()
        .get_collection(coll)
        .unwrap();
    collection
        .create_index(
            "by_object".to_string(),
            vec!["object".to_string()],
            solidb::storage::IndexType::Persistent,
            false,
        )
        .unwrap();
    let key = cache_answer(&engine, db, coll);

    collection.drop_index("by_object").unwrap();
    assert_evicted(&key, "index dropped");
}

#[test]
fn storage_create_fulltext_index_drops_cached_answers() {
    // FULLTEXT on a collection without a fulltext index answers `[]`, and
    // that answer is cacheable: it must not outlive the index's creation.
    let (engine, _tmp) = test_engine();
    let (db, coll) = ("qc_storage_ft", "qc_storage_ft_layouts");
    seed(&engine, db, coll);
    let collection = engine
        .get_database(db)
        .unwrap()
        .get_collection(coll)
        .unwrap();
    let key = cache_answer(&engine, db, coll);

    collection
        .create_fulltext_index("ft_object".to_string(), vec!["object".to_string()], None)
        .unwrap();
    assert_evicted(&key, "fulltext index created");
}

#[test]
fn storage_create_index_drops_cached_answers() {
    // The index metadata is visible before the backfill finishes, so a query
    // can be answered from a half-built index; its result must not outlive
    // the creation.
    let (engine, _tmp) = test_engine();
    let (db, coll) = ("qc_storage_mkidx", "qc_storage_mkidx_layouts");
    seed(&engine, db, coll);
    let collection = engine
        .get_database(db)
        .unwrap()
        .get_collection(coll)
        .unwrap();
    let key = cache_answer(&engine, db, coll);
    let generation = query_cache::current_generation();

    collection
        .create_index(
            "by_object".to_string(),
            vec!["object".to_string()],
            solidb::storage::IndexType::Persistent,
            false,
        )
        .unwrap();
    assert_evicted(&key, "index created");

    // A query that took its snapshot before the backfill ended cannot store.
    query_cache::get_query_cache().put_if_current(key.clone(), vec![json!("partial")], generation);
    assert_evicted(&key, "in-flight result stored after index creation");
}

#[test]
fn storage_create_geo_index_drops_cached_answers() {
    let (engine, _tmp) = test_engine();
    let (db, coll) = ("qc_storage_geo", "qc_storage_geo_layouts");
    seed(&engine, db, coll);
    let collection = engine
        .get_database(db)
        .unwrap()
        .get_collection(coll)
        .unwrap();
    let key = cache_answer(&engine, db, coll);

    collection
        .create_geo_index("geo_loc".to_string(), "loc".to_string())
        .unwrap();
    assert_evicted(&key, "geo index created");
}

#[test]
fn storage_delete_database_rejects_in_flight_results() {
    // A query that started before the drop read the old rows: whatever it
    // tries to store afterwards must be refused.
    let (engine, _tmp) = test_engine();
    let (db, coll) = ("qc_storage_inflight", "qc_storage_inflight_layouts");
    seed(&engine, db, coll);
    let key = cache_answer(&engine, db, coll);
    let generation = query_cache::current_generation();

    engine.delete_database(db).unwrap();
    query_cache::get_query_cache().put_if_current(key.clone(), vec![json!("ghost1")], generation);
    assert_evicted(&key, "in-flight result stored after database drop");
}
