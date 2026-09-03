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

    assert!(body.get("success").unwrap().as_bool().unwrap());
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

    assert!(body.get("cached").unwrap().as_bool().unwrap());
    assert!(body.contains_key("user_data"));
}

#[tokio::test]
async fn test_cors_headers() {
    let (_engine, script_engine, _tmp) = create_test_env();

    // response.cors(body, options?) returns an explicit response whose
    // headers the HTTP layer sends.
    let code = r#"
        return response.cors({ message = "CORS headers set" }, {
            origin = "https://example.com",
            methods = "GET, POST, PUT, DELETE",
            headers = "Content-Type, Authorization",
            credentials = true,
            max_age = 86400
        })
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    assert_eq!(result.status, 200);
    assert_eq!(result.body["message"], "CORS headers set");
    assert_eq!(
        result.headers["Access-Control-Allow-Origin"],
        "https://example.com"
    );
    assert_eq!(
        result.headers["Access-Control-Allow-Methods"],
        "GET, POST, PUT, DELETE"
    );
    assert_eq!(result.headers["Access-Control-Allow-Credentials"], "true");
    assert_eq!(result.headers["Access-Control-Max-Age"], "86400");
}

#[tokio::test]
async fn test_response_helpers() {
    let (_engine, script_engine, _tmp) = create_test_env();

    // response.html(content, status?) is sent as text/html, verbatim.
    let script = create_script(
        r#"return response.html("<html><body><h1>Hello World</h1></body></html>", 201)"#,
    );
    let result = script_engine
        .execute(&script, "testdb", &create_context())
        .await
        .unwrap();
    assert_eq!(result.status, 201);
    assert!(result.headers["content-type"].starts_with("text/html"));
    assert_eq!(
        result.raw_body.as_deref(),
        Some(b"<html><body><h1>Hello World</h1></body></html>".as_slice())
    );

    // response.json(body, status?, headers?) keeps the body as JSON.
    let script = create_script(
        r#"return response.json({ message = "Hello from API" }, 202, { ["X-Api"] = "v1" })"#,
    );
    let result = script_engine
        .execute(&script, "testdb", &create_context())
        .await
        .unwrap();
    assert_eq!(result.status, 202);
    assert_eq!(result.body["message"], "Hello from API");
    assert_eq!(result.headers["X-Api"], "v1");
    assert!(result.raw_body.is_none());
}

#[tokio::test]
async fn test_file_download_response() {
    let (_engine, script_engine, _tmp) = create_test_env();

    // response.file(key) serves a file stored with solidb.upload; it is
    // resolved when the response is built, so an unknown key is an error
    // rather than a 200 with nothing in it.
    let script = create_script(r#"return response.file("no-such-file")"#);
    let err = script_engine
        .execute(&script, "testdb", &create_context())
        .await
        .expect_err("unknown file key must fail");
    assert!(
        matches!(err, solidb::error::DbError::DocumentNotFound(_)),
        "{err:?}"
    );
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

    for test_result in results.iter().take(3) {
        assert!(test_result.get("success").unwrap().as_bool().unwrap());
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

    for test_result in results.iter().take(4) {
        assert!(test_result.get("cached").unwrap().as_bool().unwrap());
    }
}
