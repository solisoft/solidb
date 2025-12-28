use solidb::storage::engine::StorageEngine;
use solidb::server::handlers::AppState;
use solidb::sharding::coordinator::CollectionShardConfig;
use solidb::scripting::ScriptStats;
use std::sync::Arc;
use tempfile::TempDir;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use tower::ServiceExt; // for oneshot

#[tokio::test]
async fn verify_sharding_endpoint_returns_shards() {
    let temp_dir = TempDir::new().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    storage.initialize().unwrap();

    let db_name = "test_db";
    storage.create_database(db_name.to_string()).unwrap();
    let db = storage.get_database(db_name).unwrap();

    // Create sharded collection
    let coll_name = "sharded_coll";
    // First create the collection (default type)
    db.create_collection(coll_name.to_string(), None).unwrap();

    // Get collection handle and set sharding config
    let collection = db.get_collection(coll_name).unwrap();

    let config = CollectionShardConfig {
        num_shards: 4,
        replication_factor: 1,
        shard_key: "_key".to_string(),
    };

    collection.set_shard_config(&config).unwrap();

    // Setup AppState (minimal for handler)
    // IMPORTANT: storage needs to be the same instance
    let state = AppState {
        storage: Arc::new(storage),
        cursor_store: solidb::server::cursor_store::CursorStore::new(std::time::Duration::from_secs(60)),
        cluster_manager: None,
        replication_log: None,
        shard_coordinator: None,
        startup_time: std::time::Instant::now(),
        request_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        system_monitor: Arc::new(std::sync::Mutex::new(sysinfo::System::new())),
        queue_worker: None,
        script_stats: Arc::new(ScriptStats::default()),
    };

    // Create a mini router with just the needed route
    let app = Router::new()
        .route("/_api/database/{db}/collection/{addr}/sharding", get(solidb::server::handlers::get_sharding_details))
        .with_state(state);

    let uri = format!("/_api/database/{}/collection/{}/sharding", db_name, coll_name);
    let request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    println!("Response: {}", serde_json::to_string_pretty(&body_json).unwrap());

    // Check sharded flag
    assert_eq!(body_json.get("sharded").and_then(|v| v.as_bool()), Some(true));

    // Check shards list
    let shards = body_json.get("shards").expect("Response should have 'shards' field");
    assert!(shards.is_array(), "'shards' should be an array");

    let shards_array = shards.as_array().unwrap();
    assert_eq!(shards_array.len(), 4, "Should return 4 shards");

    let first = &shards_array[0];
    assert!(first.get("shard_id").is_some());
    // In fallback mode (no coordinator), primary might be 'unknown' or not present depending on logic?
    // Handler logic:
    // ... loop ...
        // if let Some(table) = shard_coordinator.get_shard_table() ...
        // else ... fallback ... primary_node = "unknown" (or from modulo logic if it had nodes list)
        // Wait, modulo logic uses `all_nodes`. If cluster_manager/coordinator is None/empty, `all_nodes` might be empty.
        // If `all_nodes` is empty, logic uses `total_nodes = 0`.
        // If `total_nodes > 0`, it assigns.
        // If `total_nodes == 0`, it does `nodes_for_shard.push("unknown")`?
        // Let's check logic:
        // if total_nodes > 0 { ... }
        // else { /* do nothing */ }
        // shards_info.push({ ..., nodes: nodes_for_shard })

    // So nodes list will be empty if total_nodes is 0.
    // But shard object will exist.
}
