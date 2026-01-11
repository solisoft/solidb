//! Lua Integration Tests
//!
//! Tests for:
//! - Lua Script Engine initialization
//! - Script execution
//! - DB access from Lua (insert/get/query)
//! - Global functions (solidb.log, solidb.now)

use serde_json::json;
use solidb::scripting::{Script, ScriptContext, ScriptEngine, ScriptStats, ScriptUser};
use solidb::storage::StorageEngine;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

fn create_test_env() -> (Arc<StorageEngine>, ScriptEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = Arc::new(
        StorageEngine::new(tmp_dir.path().to_str().unwrap())
            .expect("Failed to create storage engine"),
    );

    // Create DB
    engine.create_database("testdb".to_string()).unwrap();

    let stats = Arc::new(ScriptStats::default());
    let script_engine = ScriptEngine::new(engine.clone(), stats);

    (engine, script_engine, tmp_dir)
}

fn create_context() -> ScriptContext {
    ScriptContext {
        method: "POST".to_string(),
        path: "/test".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: Some(json!({})),
        is_websocket: false,
        user: ScriptUser::anonymous(),
    }
}

fn create_script(code: &str) -> Script {
    Script {
        key: "test_script".to_string(),
        name: "Test Script".to_string(),
        methods: vec!["POST".to_string()],
        path: "/test".to_string(),
        database: "testdb".to_string(),
        collection: None,
        code: code.to_string(),
        description: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
    }
}

#[tokio::test]
async fn test_lua_basic_return() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        return { message = "Hello Lua" }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();

    let body = result.body.as_object().unwrap();
    assert_eq!(body.get("message").unwrap().as_str().unwrap(), "Hello Lua");
}

#[tokio::test]
async fn test_lua_db_insert_get() {
    let (engine, script_engine, _tmp) = create_test_env();

    // Create collection first
    engine
        .get_database("testdb")
        .unwrap()
        .create_collection("users".to_string(), None)
        .unwrap();

    let code = r#"
        local users = db:collection("users")
        local doc = users:insert({ name = "Alice", age = 30 })
        
        local fetched = users:get(doc._key)
        
        return { 
            inserted_key = doc._key,
            fetched_name = fetched.name
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert!(body.contains_key("inserted_key"));
    assert_eq!(body.get("fetched_name").unwrap().as_str().unwrap(), "Alice");
}

#[tokio::test]
async fn test_lua_query() {
    let (engine, script_engine, _tmp) = create_test_env();

    let db = engine.get_database("testdb").unwrap();
    db.create_collection("items".to_string(), None).unwrap();
    let items = db.get_collection("items").unwrap();

    items.insert(json!({"_key": "1", "val": 10})).unwrap();
    items.insert(json!({"_key": "2", "val": 20})).unwrap();

    let code = r#"
        local results = db:query("FOR i IN items FILTER i.val > @limit RETURN i.val", { limit = 15 })
        return { matches = results }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    let matches = body.get("matches").unwrap().as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].as_i64().unwrap(), 20);
}

#[tokio::test]
async fn test_lua_globals() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local ts = solidb.now()
        local type_ts = type(ts)
        return { ts_type = type_ts }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("ts_type").unwrap().as_str().unwrap(), "number");
}

#[tokio::test]
async fn test_lua_security() {
    let (_engine, script_engine, _tmp) = create_test_env();

    // Check that dangerous globals are not available
    let code = r#"
        return { 
            has_os = (os ~= nil),
            has_io = (io ~= nil),
            has_debug = (debug ~= nil),
            has_package = (package ~= nil),
            has_dofile = (dofile ~= nil)
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("has_os").unwrap().as_bool().unwrap(), false);
    assert_eq!(body.get("has_io").unwrap().as_bool().unwrap(), false);
    assert_eq!(body.get("has_debug").unwrap().as_bool().unwrap(), false);
    assert_eq!(body.get("has_package").unwrap().as_bool().unwrap(), false);
    assert_eq!(body.get("has_dofile").unwrap().as_bool().unwrap(), false);
}

#[tokio::test]
async fn test_lua_regex_replace() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local text = "The quick brown fox"
        local replaced = string.regex_replace(text, "brown", "red")
        local is_match = string.regex(text, "^The .* fox$")
        
        return { 
            replaced = replaced,
            match_result = is_match
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(
        body.get("replaced").unwrap().as_str().unwrap(),
        "The quick red fox"
    );
    assert_eq!(body.get("match_result").unwrap().as_bool().unwrap(), true);
}
