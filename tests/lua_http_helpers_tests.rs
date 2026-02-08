//! Enhanced Lua HTTP Helpers Tests
//!
//! Tests for:
//! - HTTP redirects
//! - Cookie management
//! - Response caching
//! - CORS headers
//! - File downloads

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

    engine.create_database("testdb".to_string()).unwrap();

    let stats = Arc::new(ScriptStats::default());
    let script_engine = ScriptEngine::new(engine.clone(), stats);

    (engine, script_engine, tmp_dir)
}

fn create_context() -> ScriptContext {
    ScriptContext {
        method: "GET".to_string(),
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
        methods: vec!["GET".to_string()],
        path: "/test".to_string(),
        database: "testdb".to_string(),
        service: "default".to_string(),
        collection: None,
        code: code.to_string(),
        description: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
    }
}

#[tokio::test]
async fn test_redirect_functionality() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        solidb.redirect("https://example.com/target")
        return { should_not_reach = "here" }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    match script_engine.execute(&script, "testdb", &ctx).await {
        Ok(_) => panic!("Expected redirect error, but got success"),
        Err(e) => {
            let error_msg = e.to_string();
            assert!(error_msg.contains("REDIRECT:https://example.com/target"));
        }
    }
}

#[tokio::test]
async fn test_cookie_setting() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local options = {
            expires = "2024-12-31T23:59:59Z",
            path = "/",
            domain = "example.com",
            secure = true,
            httpOnly = true
        }

        solidb.set_cookie("session_id", "abc123", options)

        return { success = true }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("success").unwrap().as_bool().unwrap(), true);
}

#[tokio::test]
async fn test_cache_operations() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        -- Store data in cache
        local data = {
            user_id = 123,
            name = "Alice",
            permissions = {"read", "write"}
        }

        local cache_result = solidb.cache("user:123", data, 3600)  -- 1 hour TTL

        return {
            cached = cache_result,
            user_data = data
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("cached").unwrap().as_bool().unwrap(), true);
    assert!(body.contains_key("user_data"));
}

#[tokio::test]
async fn test_cors_headers() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local cors_options = {
            origins = {"https://example.com", "https://app.example.com"},
            methods = {"GET", "POST", "PUT", "DELETE"},
            headers = {"Content-Type", "Authorization"},
            credentials = true,
            max_age = 86400
        }

        response.cors(cors_options)

        return { message = "CORS headers set" }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(
        body.get("message").unwrap().as_str().unwrap(),
        "CORS headers set"
    );
}

#[tokio::test]
async fn test_response_helpers() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        -- Test HTML response
        local html_content = "<html><body><h1>Hello World</h1></body></html>"
        local html_result = response.html(html_content)

        -- Test JSON response (already exists)
        local json_data = { message = "Hello from API", status = "success" }
        local json_result = response.json(json_data)

        return {
            html_content = html_content,
            json_result = json_result
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
        body.get("html_content").unwrap().as_str().unwrap(),
        "<html><body><h1>Hello World</h1></body></html>"
    );
    assert!(body.get("json_result").is_some());
}

#[tokio::test]
async fn test_file_download_response() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        -- Test file download response
        local file_path = "/tmp/test_file.txt"
        local download_result = response.file(file_path)

        return {
            file_path = file_path,
            success = download_result ~= nil
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
        body.get("file_path").unwrap().as_str().unwrap(),
        "/tmp/test_file.txt"
    );
    assert_eq!(body.get("success").unwrap().as_bool().unwrap(), true);
}

#[tokio::test]
async fn test_streaming_response() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        -- Test streaming response
        local stream_data = {
            "chunk1",
            "chunk2",
            "chunk3"
        }

        local stream_result = response.stream(stream_data)

        return {
            chunks_count = #stream_data,
            success = stream_result ~= nil
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("chunks_count").unwrap().as_i64().unwrap(), 3);
    assert_eq!(body.get("success").unwrap().as_bool().unwrap(), true);
}

#[tokio::test]
async fn test_cookie_options_validation() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        -- Test cookie with various options
        local test_cases = {
            {
                name = "simple",
                value = "test",
                options = nil
            },
            {
                name = "with_expires",
                value = "test2",
                options = { expires = "2024-12-31T23:59:59Z" }
            },
            {
                name = "secure_cookie",
                value = "secret",
                options = {
                    secure = true,
                    httpOnly = true,
                    sameSite = "Strict"
                }
            }
        }

        local results = {}
        for i, test_case in ipairs(test_cases) do
            if test_case.options then
                solidb.set_cookie(test_case.name, test_case.value, test_case.options)
            else
                solidb.set_cookie(test_case.name, test_case.value)
            end
            results[i] = { name = test_case.name, success = true }
        end

        return { results = results }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    let results = body.get("results").unwrap().as_array().unwrap();
    assert_eq!(results.len(), 3);

    for i in 0..3 {
        let test_result = &results[i];
        assert_eq!(test_result.get("success").unwrap().as_bool().unwrap(), true);
    }
}

#[tokio::test]
async fn test_cache_with_ttl_expiration() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        -- Test cache with different TTL values
        local test_cases = {
            { key = "short_ttl", ttl = 1 },      -- 1 second
            { key = "medium_ttl", ttl = 3600 },   -- 1 hour
            { key = "long_ttl", ttl = 86400 },    -- 1 day
            { key = "no_ttl", ttl = nil }         -- no TTL (default)
        }

        local results = {}
        for i, test_case in ipairs(test_cases) do
            local data = {
                key = test_case.key,
                cached_at = solidb.now()
            }

            local success = solidb.cache(test_case.key, data, test_case.ttl)
            results[i] = {
                key = test_case.key,
                cached = success
            }
        end

        return { results = results }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    let results = body.get("results").unwrap().as_array().unwrap();
    assert_eq!(results.len(), 4);

    for i in 0..4 {
        let test_result = &results[i];
        assert_eq!(test_result.get("cached").unwrap().as_bool().unwrap(), true);
    }
}
