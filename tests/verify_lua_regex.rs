use solidb::scripting::{ScriptEngine, Script, ScriptContext, ScriptStats};
use solidb::storage::StorageEngine;
use std::sync::Arc;
use std::collections::HashMap;

#[tokio::test]
async fn verify_lua_regex() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(StorageEngine::new(tmp_dir.path()).unwrap());
    let engine = ScriptEngine::new(storage, Arc::new(ScriptStats::default()));

    // Test 1: Valid Regex Match
    let script_match = Script {
        key: "test_match".to_string(),
        name: "Test Match".to_string(),
        methods: vec!["GET".to_string()],
        path: "match".to_string(),
        database: "_system".to_string(),
        collection: None,
        code: r#"
            local is_match = string.regex("hello world", "^hello")
            return { status = 200, body = is_match }
        "#.to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let context = ScriptContext {
        method: "GET".to_string(),
        path: "match".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };

    let result_match = engine.execute(&script_match, "_system", &context).await.unwrap();
    assert_eq!(result_match.body["body"], serde_json::Value::Bool(true));

    // Test 2: Valid Regex No Match
    let script_no_match = Script {
        key: "test_no_match".to_string(),
        name: "Test No Match".to_string(),
        methods: vec!["GET".to_string()],
        path: "no_match".to_string(),
        database: "_system".to_string(),
        collection: None,
        code: r#"
            local is_match = string.regex("hello world", "^bye")
            return { status = 200, body = is_match }
        "#.to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let result_no_match = engine.execute(&script_no_match, "_system", &context).await.unwrap();
    assert_eq!(result_no_match.body["body"], serde_json::Value::Bool(false));

    // Test 3: Object Syntax
    let script_obj = Script {
        key: "test_obj".to_string(),
        name: "Test Obj".to_string(),
        methods: vec!["GET".to_string()],
        path: "obj".to_string(),
        database: "_system".to_string(),
        collection: None,
        code: r#"
            local s = "hello world"
            return { status = 200, body = s:regex("world$") }
        "#.to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let result_obj = engine.execute(&script_obj, "_system", &context).await.unwrap();
    assert_eq!(result_obj.body["body"], serde_json::Value::Bool(true));

    // Test 4: Invalid Regex
    let script_invalid = Script {
        key: "test_invalid".to_string(),
        name: "Test Invalid".to_string(),
        methods: vec!["GET".to_string()],
        path: "invalid".to_string(),
        database: "_system".to_string(),
        collection: None,
        code: r#"
            string.regex("foo", "[")
        "#.to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let result_invalid = engine.execute(&script_invalid, "_system", &context).await;
    assert!(result_invalid.is_err());
    let msg = result_invalid.err().unwrap().to_string();
    assert!(msg.contains("Lua error"), "Unexpected error: {}", msg);
}
