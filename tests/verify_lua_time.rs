//! Verify Lua Time Functions
//!
//! Verifies `solidb.now()` and time handling in Lua.

use solidb::storage::StorageEngine;
use solidb::scripting::{ScriptEngine, ScriptStats, ScriptUser};
use tempfile::TempDir;
use std::sync::Arc;

fn create_test_env() -> (Arc<StorageEngine>, Arc<ScriptEngine>, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = Arc::new(StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine"));
    
    engine.create_database("testdb".to_string()).unwrap();
    
    let stats = Arc::new(ScriptStats::default());
    let script_engine = Arc::new(ScriptEngine::new(engine.clone(), stats));
    
    (engine, script_engine, tmp_dir)
}

#[tokio::test]
async fn test_lua_now() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let script = solidb::scripting::Script {
        key: "test_time".to_string(),
        name: "test_time".to_string(),
        methods: vec!["POST".to_string()],
        path: "time".to_string(),
        database: "testdb".to_string(),
        collection: None,
        code: r#"
            local t1 = solidb.now()
            -- Busy wait a bit? No, now() is seconds resolution.
            -- Just return it.
            return { timestamp = t1 }
        "#.to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };
    
    let context = solidb::scripting::ScriptContext {
        method: "POST".to_string(),
        path: "time".to_string(),
        query_params: std::collections::HashMap::new(),
        headers: std::collections::HashMap::new(),
        body: None,
        params: std::collections::HashMap::new(),
        is_websocket: false,
        user: ScriptUser::anonymous(),
    };
    
    let result = script_engine.execute(&script, "testdb", &context).await.unwrap();
    let json = result.body;
    
    let ts = json["timestamp"].as_u64().expect("Expected u64 timestamp");
    let now_sys = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    // Check if within 5 seconds drift (mostly ensuring it's recent)
    assert!(ts <= now_sys && ts >= now_sys - 5);
}
