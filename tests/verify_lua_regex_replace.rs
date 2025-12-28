use solidb::scripting::{ScriptEngine, Script, ScriptContext, ScriptStats};
use solidb::storage::StorageEngine;
use std::sync::Arc;
use std::collections::HashMap;

#[tokio::test]
async fn verify_lua_regex_replace() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(StorageEngine::new(tmp_dir.path()).unwrap());
    let engine = ScriptEngine::new(storage, Arc::new(ScriptStats::default()));

    let context = ScriptContext {
        method: "GET".to_string(),
        path: "test".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };

    // Test 1: Simple replacement
    let script_simple = Script {
        key: "test_simple".to_string(),
        name: "Test Simple".to_string(),
        methods: vec!["GET".to_string()],
        path: "simple".to_string(),
        database: "_system".to_string(),
        collection: None,
        code: r#"
            local result = string.regex_replace("hello world", "world", "Lua")
            return { body = result }
        "#.to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let result_simple = engine.execute(&script_simple, "_system", &context).await.unwrap();
    assert_eq!(result_simple.body["body"], serde_json::json!("hello Lua"));

    // Test 2: Pattern replacement
    let script_pattern = Script {
        key: "test_pattern".to_string(),
        name: "Test Pattern".to_string(),
        methods: vec!["GET".to_string()],
        path: "pattern".to_string(),
        database: "_system".to_string(),
        collection: None,
        code: r#"
            local result = string.regex_replace("foo123bar456", "\\d+", "X")
            return { body = result }
        "#.to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let result_pattern = engine.execute(&script_pattern, "_system", &context).await.unwrap();
    assert_eq!(result_pattern.body["body"], serde_json::json!("fooXbarX"));

    // Test 3: Method syntax
    let script_method = Script {
        key: "test_method".to_string(),
        name: "Test Method".to_string(),
        methods: vec!["GET".to_string()],
        path: "method".to_string(),
        database: "_system".to_string(),
        collection: None,
        code: r#"
            local s = "hello world world"
            return { body = s:regex_replace("world", "Rust") }
        "#.to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let result_method = engine.execute(&script_method, "_system", &context).await.unwrap();
    assert_eq!(result_method.body["body"], serde_json::json!("hello Rust Rust"));
}
