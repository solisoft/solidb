//! Lua Regex Tests
//!
//! Verifies the exposed `string.regex` and `string.regex_replace` functions in Lua.

use solidb::scripting::{ScriptEngine, ScriptStats, ScriptUser};
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;

fn create_test_env() -> (Arc<StorageEngine>, Arc<ScriptEngine>, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = Arc::new(
        StorageEngine::new(tmp_dir.path().to_str().unwrap())
            .expect("Failed to create storage engine"),
    );

    engine.create_database("testdb".to_string()).unwrap();

    let stats = Arc::new(ScriptStats::default());
    let script_engine = Arc::new(ScriptEngine::new(engine.clone(), stats));

    (engine, script_engine, tmp_dir)
}

#[tokio::test]
async fn test_lua_regex_match() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let script = solidb::scripting::Script {
        key: "test_regex".to_string(),
        name: "test_regex".to_string(),
        methods: vec!["POST".to_string()],
        path: "test".to_string(),
        database: "testdb".to_string(),
        service: "default".to_string(),
        collection: None,
        code: r#"
            -- Validate email pattern
            local email = "test@example.com"
            local is_match = string.regex(email, "^[\\w\\.-]+@[\\w\\.-]+\\.[a-zA-Z]+$")

            -- Validate non-match
            local is_not_match = string.regex("invalid-email", "@")

            return { match = is_match, not_match = is_not_match }
        "#
        .to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let context = solidb::scripting::ScriptContext {
        method: "POST".to_string(),
        path: "test".to_string(),
        query_params: std::collections::HashMap::new(),
        headers: std::collections::HashMap::new(),
        body: None,
        params: std::collections::HashMap::new(),
        is_websocket: false,
        user: ScriptUser::anonymous(),
    };

    let result = script_engine
        .execute(&script, "testdb", &context)
        .await
        .unwrap();
    let json = result.body;

    assert_eq!(json["match"], true);
    // Wait, regex "invalid-email" matches "@"? No.
    // string.regex is regex::new(pattern).is_match(s).
    // Pattern "@" on "invalid-email" -> false (no @).
    // Pattern ".*" on "invalid-email" -> true.
    assert_eq!(json["not_match"], false);
}

#[tokio::test]
async fn test_lua_regex_replace() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let script = solidb::scripting::Script {
        key: "test_replace".to_string(),
        name: "test_replace".to_string(),
        methods: vec!["POST".to_string()],
        path: "test".to_string(),
        database: "testdb".to_string(),
        service: "default".to_string(),
        collection: None,
        code: r#"
            local text = "The year is 2023"
            -- Replace year with 2024 using regex \d+
            local new_text = string.regex_replace(text, "\\d+", "2024")

            -- Mask sensitive data
            local secret = "My secret is 12345"
            local masked = string.regex_replace(secret, "\\d+", "*****")

            return { text = new_text, masked = masked }
        "#
        .to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let context = solidb::scripting::ScriptContext {
        method: "POST".to_string(),
        path: "test".to_string(),
        query_params: std::collections::HashMap::new(),
        headers: std::collections::HashMap::new(),
        body: None,
        params: std::collections::HashMap::new(),
        is_websocket: false,
        user: ScriptUser::anonymous(),
    };

    let result = script_engine
        .execute(&script, "testdb", &context)
        .await
        .unwrap();
    let json = result.body;

    assert_eq!(json["text"], "The year is 2024");
    assert_eq!(json["masked"], "My secret is *****");
}

#[tokio::test]
async fn test_lua_regex_capture_groups() {
    let (_engine, script_engine, _tmp) = create_test_env();

    // Rust regex replacement uses ${name} or $1 syntax?
    // Regex crate: $name or ${name} or $0.

    let script = solidb::scripting::Script {
        key: "test_groups".to_string(),
        name: "test_groups".to_string(),
        methods: vec!["POST".to_string()],
        path: "test".to_string(),
        database: "testdb".to_string(),
        service: "default".to_string(),
        collection: None,
        code: r#"
            local text = "John Doe"
            -- Swap names: (\w+) (\w+) -> $2 $1
            local swapped = string.regex_replace(text, "(\\w+) (\\w+)", "$2 $1")

            return { swapped = swapped }
        "#
        .to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let context = solidb::scripting::ScriptContext {
        method: "POST".to_string(),
        path: "test".to_string(),
        query_params: std::collections::HashMap::new(),
        headers: std::collections::HashMap::new(),
        body: None,
        params: std::collections::HashMap::new(),
        is_websocket: false,
        user: ScriptUser::anonymous(),
    };

    let result = script_engine
        .execute(&script, "testdb", &context)
        .await
        .unwrap();
    let json = result.body;

    assert_eq!(json["swapped"], "Doe John");
}
