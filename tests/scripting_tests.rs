//! Comprehensive Scripting Module Tests
//! 
//! Tests for areas not covered by lua_integration_tests and lua_crypto_tests:
//! - Request context (method, path, query_params, params, headers, body)
//! - Time namespace functions
//! - Collection CRUD operations (update, delete, count)
//! - Crypto utilities (uuid, random_bytes)
//! - solidb.log and solidb.stats
//! - db:enqueue for job queuing
//! - Error handling and script failures

use solidb::storage::StorageEngine;
use solidb::scripting::{ScriptEngine, Script, ScriptContext, ScriptStats};
use serde_json::json;
use tempfile::TempDir;
use std::sync::Arc;
use std::collections::HashMap;

fn create_test_env() -> (Arc<StorageEngine>, ScriptEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = Arc::new(StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine"));
    engine.create_database("testdb".to_string()).unwrap();
    let stats = Arc::new(ScriptStats::default());
    let script_engine = ScriptEngine::new(engine.clone(), stats);
    (engine, script_engine, tmp_dir)
}

fn create_script(code: &str) -> Script {
    Script {
        key: "test_script".to_string(),
        name: "Test Script".to_string(),
        methods: vec!["GET".to_string(), "POST".to_string()],
        path: "/test".to_string(),
        database: "testdb".to_string(),
        collection: None,
        code: code.to_string(),
        description: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
    }
}

// ============================================================================
// Request Context Tests
// ============================================================================

#[tokio::test]
async fn test_request_method_and_path() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "DELETE".to_string(),
        path: "/api/users/123".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        return { 
            method = request.method,
            path = request.path
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["method"], "DELETE");
    assert_eq!(body["path"], "/api/users/123");
}

#[tokio::test]
async fn test_request_query_params() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let mut query_params = HashMap::new();
    query_params.insert("page".to_string(), "5".to_string());
    query_params.insert("limit".to_string(), "20".to_string());
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/users".to_string(),
        query_params,
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        return { 
            page = request.query.page,
            limit = request.query_params.limit 
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["page"], "5");
    assert_eq!(body["limit"], "20");
}

#[tokio::test]
async fn test_request_url_params() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let mut params = HashMap::new();
    params.insert("id".to_string(), "user_456".to_string());
    params.insert("action".to_string(), "edit".to_string());
    
    let ctx = ScriptContext {
        method: "PUT".to_string(),
        path: "/users/:id/:action".to_string(),
        query_params: HashMap::new(),
        params,
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        return { 
            id = request.params.id,
            action = request.params.action
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["id"], "user_456");
    assert_eq!(body["action"], "edit");
}

#[tokio::test]
async fn test_request_headers() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let mut headers = HashMap::new();
    headers.insert("Authorization".to_string(), "Bearer token123".to_string());
    headers.insert("X-Custom-Header".to_string(), "custom_value".to_string());
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/protected".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers,
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        return { 
            auth = request.headers["Authorization"],
            custom = request.headers["X-Custom-Header"]
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["auth"], "Bearer token123");
    assert_eq!(body["custom"], "custom_value");
}

#[tokio::test]
async fn test_request_body() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "POST".to_string(),
        path: "/users".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: Some(json!({
            "name": "Alice",
            "email": "alice@example.com",
            "age": 30
        })),
        is_websocket: false,
    };
    
    let code = r#"
        return { 
            name = request.body.name,
            email = request.body.email,
            age = request.body.age
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["name"], "Alice");
    assert_eq!(body["email"], "alice@example.com");
    assert_eq!(body["age"], 30);
}

#[tokio::test]
async fn test_request_is_websocket() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/ws".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: true,
    };
    
    let code = r#"return { is_ws = request.is_websocket }"#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["is_ws"], true);
}

// ============================================================================
// Time Namespace Tests
// ============================================================================

#[tokio::test]
async fn test_time_now_and_now_ms() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/time".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local now_sec = time.now()
        local now_ms = time.now_ms()
        return { 
            now_type = type(now_sec),
            now_ms_type = type(now_ms),
            ms_greater_than_sec = (now_ms > now_sec)
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["now_type"], "number");
    assert_eq!(body["now_ms_type"], "number");
    assert_eq!(body["ms_greater_than_sec"], true);
}

#[tokio::test]
async fn test_time_iso() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/time".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local iso = time.iso()
        return { 
            iso = iso,
            has_t = string.find(iso, "T") ~= nil,
            has_z = string.find(iso, "+") ~= nil or string.find(iso, "Z") ~= nil
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert!(body["iso"].as_str().unwrap().len() > 10);
    assert_eq!(body["has_t"], true);
}

#[tokio::test]
async fn test_time_add_subtract() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/time".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local base = 1000.0
        local plus_1s = time.add(base, 1, "s")
        local plus_1m = time.add(base, 1, "m")
        local plus_1h = time.add(base, 1, "h")
        local plus_1d = time.add(base, 1, "d")
        local minus_1s = time.subtract(base, 1, "s")
        
        return { 
            plus_1s = plus_1s,
            plus_1m = plus_1m,
            plus_1h = plus_1h,
            plus_1d = plus_1d,
            minus_1s = minus_1s
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["plus_1s"].as_f64().unwrap(), 1001.0);
    assert_eq!(body["plus_1m"].as_f64().unwrap(), 1060.0);
    assert_eq!(body["plus_1h"].as_f64().unwrap(), 4600.0);
    assert_eq!(body["plus_1d"].as_f64().unwrap(), 87400.0);
    assert_eq!(body["minus_1s"].as_f64().unwrap(), 999.0);
}

#[tokio::test]
async fn test_time_format() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/time".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    // Use a known timestamp: 2024-01-15 12:30:45 UTC
    let code = r#"
        local ts = 1705321845.0
        local formatted = time.format(ts, "%Y-%m-%d %H:%M:%S")
        return { formatted = formatted }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["formatted"], "2024-01-15 12:30:45");
}

#[tokio::test]
async fn test_time_parse() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/time".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local ts = time.parse("2024-01-15T12:30:45+00:00")
        return { ts = ts }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    let ts = result.body["ts"].as_f64().unwrap();
    assert!((ts - 1705321845.0).abs() < 1.0);
}

// ============================================================================
// Collection CRUD Tests
// ============================================================================

#[tokio::test]
async fn test_collection_update() {
    let (engine, script_engine, _tmp) = create_test_env();
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("users".to_string(), None).unwrap();
    
    let ctx = ScriptContext {
        method: "PUT".to_string(),
        path: "/users".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local users = db:collection("users")
        local doc = users:insert({ _key = "alice", name = "Alice", age = 25 })
        
        local updated = users:update("alice", { name = "Alice Updated", age = 26 })
        local fetched = users:get("alice")
        
        return { 
            updated_name = fetched.name,
            updated_age = fetched.age
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["updated_name"], "Alice Updated");
    assert_eq!(body["updated_age"], 26);
}

#[tokio::test]
async fn test_collection_delete() {
    let (engine, script_engine, _tmp) = create_test_env();
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("items".to_string(), None).unwrap();
    
    let ctx = ScriptContext {
        method: "DELETE".to_string(),
        path: "/items".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local items = db:collection("items")
        items:insert({ _key = "item1", name = "Item 1" })
        items:insert({ _key = "item2", name = "Item 2" })
        
        local count_before = items:count()
        local deleted = items:delete("item1")
        local count_after = items:count()
        local fetched = items:get("item1")
        
        return { 
            count_before = count_before,
            count_after = count_after,
            deleted_result = deleted,
            item1_is_nil = (fetched == nil)
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["count_before"], 2);
    assert_eq!(body["count_after"], 1);
    assert_eq!(body["deleted_result"], true);
    assert_eq!(body["item1_is_nil"], true);
}

// ============================================================================
// Crypto Utilities Tests
// ============================================================================

#[tokio::test]
async fn test_crypto_uuid() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/uuid".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local uuid1 = crypto.uuid()
        local uuid2 = crypto.uuid()
        local uuidv7 = crypto.uuid_v7()
        
        return { 
            uuid1 = uuid1,
            uuid2 = uuid2,
            uuidv7 = uuidv7,
            different = (uuid1 ~= uuid2),
            has_dashes = string.find(uuid1, "-") ~= nil
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["different"], true);
    assert_eq!(body["has_dashes"], true);
    assert_eq!(body["uuid1"].as_str().unwrap().len(), 36);
    assert_eq!(body["uuidv7"].as_str().unwrap().len(), 36);
}

#[tokio::test]
async fn test_crypto_random_bytes() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/random".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local bytes16 = crypto.random_bytes(16)
        local bytes32 = crypto.random_bytes(32)
        
        return { 
            len16 = #bytes16,
            len32 = #bytes32
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["len16"], 16);
    assert_eq!(body["len32"], 32);
}

#[tokio::test]
async fn test_crypto_hmac() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/hmac".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local key = "secret_key"
        local data = "hello world"
        local hmac256 = crypto.hmac_sha256(key, data)
        local hmac512 = crypto.hmac_sha512(key, data)
        
        return { 
            hmac256_len = #hmac256,
            hmac512_len = #hmac512,
            hmac256 = hmac256
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["hmac256_len"], 64); // SHA256 = 32 bytes = 64 hex chars
    assert_eq!(body["hmac512_len"], 128); // SHA512 = 64 bytes = 128 hex chars
}

#[tokio::test]
async fn test_crypto_base32() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/base32".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local original = "Hello"
        local encoded = crypto.base32_encode(original)
        local decoded = crypto.base32_decode(encoded)
        
        return { 
            encoded = encoded,
            decoded = decoded,
            match = (original == decoded)
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["encoded"], "JBSWY3DP"); // Standard Base32 for "Hello"
    assert_eq!(body["decoded"], "Hello");
    assert_eq!(body["match"], true);
}

// ============================================================================
// solidb Namespace Tests
// ============================================================================

#[tokio::test]
async fn test_solidb_stats() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/stats".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local stats = solidb.stats()
        return { 
            has_active = (stats.active_scripts ~= nil),
            has_ws = (stats.active_ws ~= nil),
            has_total = (stats.total_scripts_executed ~= nil)
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["has_active"], true);
    assert_eq!(body["has_ws"], true);
    assert_eq!(body["has_total"], true);
}

#[tokio::test]
async fn test_solidb_log() {
    let (engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "POST".to_string(),
        path: "/log".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        solidb.log("Test log message")
        solidb.log({ key = "value", num = 123 })
        return { logged = true }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["logged"], true);
    
    // Verify logs were written to _logs collection
    let db = engine.get_database("testdb").unwrap();
    let logs = db.get_collection("_logs").unwrap();
    assert!(logs.count() >= 2);
}

// ============================================================================
// db:enqueue Tests
// ============================================================================

#[tokio::test]
async fn test_db_enqueue_job() {
    let (engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "POST".to_string(),
        path: "/enqueue".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local job_id = db:enqueue("emails", "send_email", { to = "test@example.com" })
        return { 
            job_id = job_id,
            has_id = (job_id ~= nil and #job_id > 0)
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["has_id"], true);
    assert_eq!(body["job_id"].as_str().unwrap().len(), 36); // UUID length
    
    // Verify job was created in _jobs collection
    let db = engine.get_database("testdb").unwrap();
    let jobs = db.get_collection("_jobs").unwrap();
    assert_eq!(jobs.count(), 1);
}

#[tokio::test]
async fn test_db_enqueue_with_options() {
    let (engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "POST".to_string(),
        path: "/enqueue".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local job_id = db:enqueue("priority_queue", "urgent_task", { data = "test" }, {
            priority = 100,
            max_retries = 5
        })
        return { job_id = job_id }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert!(result.body["job_id"].as_str().unwrap().len() > 0);
    
    // Verify job properties
    let db = engine.get_database("testdb").unwrap();
    let jobs = db.get_collection("_jobs").unwrap();
    let job_doc = jobs.scan(None).pop().unwrap();
    assert_eq!(job_doc.data["priority"], 100);
    assert_eq!(job_doc.data["max_retries"], 5);
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[tokio::test]
async fn test_lua_syntax_error() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/error".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        return { this is invalid syntax
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await;
    
    assert!(result.is_err());
}

#[tokio::test]
async fn test_lua_runtime_error() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/error".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        error("Intentional error for testing")
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await;
    
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Intentional error"));
}

#[tokio::test]
async fn test_collection_not_found_returns_nil() {
    let (engine, script_engine, _tmp) = create_test_env();
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("test".to_string(), None).unwrap();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/get".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local col = db:collection("test")
        local doc = col:get("nonexistent_key")
        return { is_nil = (doc == nil) }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["is_nil"], true);
}

// ============================================================================
// Script Statistics Tests
// ============================================================================

#[tokio::test]
async fn test_script_stats_tracking() {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = Arc::new(StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine"));
    engine.create_database("testdb".to_string()).unwrap();
    
    let stats = Arc::new(ScriptStats::default());
    let script_engine = ScriptEngine::new(engine.clone(), stats.clone());
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/stats".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"return { ok = true }"#;
    let script = create_script(code);
    
    // Execute multiple scripts
    for _ in 0..3 {
        script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    }
    
    // Check stats
    assert_eq!(stats.total_scripts_executed.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(stats.active_scripts.load(std::sync::atomic::Ordering::SeqCst), 0); // All done
}

// ============================================================================
// Context Alias Tests
// ============================================================================

#[tokio::test]
async fn test_context_alias() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "POST".to_string(),
        path: "/alias".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: Some(json!({"data": "test"})),
        is_websocket: false,
    };
    
    // 'context' should be an alias for 'request'
    let code = r#"
        return { 
            method = context.method,
            body_data = context.body.data
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["method"], "POST");
    assert_eq!(body["body_data"], "test");
}

// ============================================================================
// JWT Edge Cases Tests
// ============================================================================

#[tokio::test]
async fn test_jwt_invalid_token_format() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/jwt".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local ok, err = pcall(function()
            crypto.jwt_decode("invalid.token", "secret")
        end)
        return { ok = ok, has_error = (err ~= nil) }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["ok"], false);
}

#[tokio::test]
async fn test_jwt_wrong_secret() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/jwt".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local token = crypto.jwt_encode({ user = "test" }, "secret1")
        local ok, err = pcall(function()
            crypto.jwt_decode(token, "wrong_secret")
        end)
        return { ok = ok }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["ok"], false);
}

// ============================================================================
// JSON Conversion Edge Cases
// ============================================================================

#[tokio::test]
async fn test_json_nested_objects() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "POST".to_string(),
        path: "/json".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: Some(json!({
            "level1": {
                "level2": {
                    "level3": {
                        "value": "deep"
                    }
                }
            }
        })),
        is_websocket: false,
    };
    
    let code = r#"
        return { deep_value = request.body.level1.level2.level3.value }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["deep_value"], "deep");
}

#[tokio::test]
async fn test_json_arrays() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "POST".to_string(),
        path: "/json".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: Some(json!({
            "items": [1, 2, 3, 4, 5],
            "names": ["a", "b", "c"]
        })),
        is_websocket: false,
    };
    
    let code = r#"
        local sum = 0
        for _, v in ipairs(request.body.items) do
            sum = sum + v
        end
        return { 
            sum = sum,
            first_name = request.body.names[1],
            items_count = #request.body.items
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["sum"], 15);
    assert_eq!(result.body["first_name"], "a");
    assert_eq!(result.body["items_count"], 5);
}

#[tokio::test]
async fn test_json_null_handling() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "POST".to_string(),
        path: "/json".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: Some(json!({
            "value": null,
            "present": "yes"
        })),
        is_websocket: false,
    };
    
    let code = r#"
        return { 
            value_is_nil = (request.body.value == nil),
            present = request.body.present
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["value_is_nil"], true);
    assert_eq!(result.body["present"], "yes");
}

#[tokio::test]
async fn test_json_boolean_handling() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "POST".to_string(),
        path: "/json".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: Some(json!({
            "active": true,
            "deleted": false
        })),
        is_websocket: false,
    };
    
    let code = r#"
        return { 
            active = request.body.active,
            deleted = request.body.deleted,
            active_type = type(request.body.active)
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["active"], true);
    assert_eq!(result.body["deleted"], false);
    assert_eq!(result.body["active_type"], "boolean");
}

#[tokio::test]
async fn test_lua_return_array() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/array".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        return { items = { 10, 20, 30 } }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    let items = result.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], 10);
    assert_eq!(items[2], 30);
}

#[tokio::test]
async fn test_lua_return_mixed_table() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/mixed".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        return { 
            string_val = "hello",
            number_val = 42,
            float_val = 3.14,
            bool_val = true,
            nil_val = nil
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    let body = result.body.as_object().unwrap();
    
    assert_eq!(body["string_val"], "hello");
    assert_eq!(body["number_val"], 42);
    assert!((body["float_val"].as_f64().unwrap() - 3.14).abs() < 0.001);
    assert_eq!(body["bool_val"], true);
    assert!(body.get("nil_val").is_none() || body["nil_val"].is_null());
}

// ============================================================================
// Response Helper Tests
// ============================================================================

#[tokio::test]
async fn test_response_json_helper() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/response".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        return response.json({ status = "ok", data = { id = 1 } })
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["status"], "ok");
    assert_eq!(result.body["data"]["id"], 1);
}

// ============================================================================
// Database Query Tests
// ============================================================================

#[tokio::test]
async fn test_db_query_with_complex_binds() {
    let (engine, script_engine, _tmp) = create_test_env();
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("products".to_string(), None).unwrap();
    
    let products = db.get_collection("products").unwrap();
    products.insert(json!({"name": "Product A", "price": 100, "category": "electronics"})).unwrap();
    products.insert(json!({"name": "Product B", "price": 200, "category": "electronics"})).unwrap();
    products.insert(json!({"name": "Product C", "price": 50, "category": "books"})).unwrap();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/query".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local results = db:query(
            "FOR p IN products FILTER p.price > @minPrice AND p.category == @cat RETURN p.name",
            { minPrice = 75, cat = "electronics" }
        )
        return { count = #results, names = results }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["count"], 2);
}

#[tokio::test]
async fn test_db_query_no_bind_vars() {
    let (engine, script_engine, _tmp) = create_test_env();
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("simple".to_string(), None).unwrap();
    
    let simple = db.get_collection("simple").unwrap();
    simple.insert(json!({"val": 1})).unwrap();
    simple.insert(json!({"val": 2})).unwrap();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/query".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local results = db:query("FOR s IN simple RETURN s.val")
        local sum = 0
        for _, v in ipairs(results) do sum = sum + v end
        return { sum = sum }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["sum"], 3);
}

// ============================================================================
// Collection Count Tests
// ============================================================================

#[tokio::test]
async fn test_collection_count() {
    let (engine, script_engine, _tmp) = create_test_env();
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("countable".to_string(), None).unwrap();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/count".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local col = db:collection("countable")
        local count_before = col:count()
        col:insert({ name = "item1" })
        col:insert({ name = "item2" })
        col:insert({ name = "item3" })
        local count_after = col:count()
        return { before = count_before, after = count_after }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["before"], 0);
    assert_eq!(result.body["after"], 3);
}

// ============================================================================
// Time Edge Cases
// ============================================================================

#[tokio::test]
async fn test_time_invalid_unit() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/time".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local ok, err = pcall(function()
            time.add(1000, 1, "invalid_unit")
        end)
        return { ok = ok }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["ok"], false);
}

#[tokio::test]
async fn test_time_milliseconds_unit() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/time".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local base = 1000.0
        local plus_500ms = time.add(base, 500, "ms")
        return { result = plus_500ms }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["result"].as_f64().unwrap(), 1000.5);
}

// ============================================================================
// Crypto Edge Cases
// ============================================================================

#[tokio::test]
async fn test_crypto_empty_string_hash() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/hash".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        return { 
            md5 = crypto.md5(""),
            sha256 = crypto.sha256("")
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    // Known empty string hash values
    assert_eq!(result.body["md5"], "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(result.body["sha256"], "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
}

// ============================================================================
// Security Tests
// ============================================================================

#[tokio::test]
async fn test_security_load_removed() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/security".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        return { 
            has_load = (load ~= nil),
            has_loadfile = (loadfile ~= nil),
            has_require = (require ~= nil)
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["has_load"], false);
    assert_eq!(result.body["has_loadfile"], false);
    assert_eq!(result.body["has_require"], false);
}

// ============================================================================
// Empty Body Tests
// ============================================================================

#[tokio::test]
async fn test_request_no_body() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/nobody".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        return { has_body = (request.body ~= nil) }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    // body should be nil when not provided
    assert_eq!(result.body["has_body"], false);
}

// ============================================================================
// Multiple Operations Tests
// ============================================================================

#[tokio::test]
async fn test_multiple_collection_operations() {
    let (engine, script_engine, _tmp) = create_test_env();
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("multi".to_string(), None).unwrap();
    
    let ctx = ScriptContext {
        method: "POST".to_string(),
        path: "/multi".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local col = db:collection("multi")
        
        -- Insert multiple
        for i = 1, 5 do
            col:insert({ _key = "item" .. i, value = i * 10 })
        end
        
        -- Update one
        col:update("item3", { value = 999 })
        
        -- Delete one
        col:delete("item1")
        
        -- Get results
        local item3 = col:get("item3")
        local item1 = col:get("item1")
        local count = col:count()
        
        return {
            item3_value = item3.value,
            item1_exists = (item1 ~= nil),
            final_count = count
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["item3_value"], 999);
    assert_eq!(result.body["item1_exists"], false);
    assert_eq!(result.body["final_count"], 4);
}

// ============================================================================
// Large Data Tests
// ============================================================================

#[tokio::test]
async fn test_large_string_handling() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/large".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local str = string.rep("a", 10000)
        local hash = crypto.sha256(str)
        return { 
            len = #str,
            hash_len = #hash
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["len"], 10000);
    assert_eq!(result.body["hash_len"], 64); // SHA256 = 64 hex chars
}

// ============================================================================
// solidb.now() Tests
// ============================================================================

#[tokio::test]
async fn test_solidb_now() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/now".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        local ts = solidb.now()
        return { 
            ts_type = type(ts),
            is_reasonable = (ts > 1700000000) -- After 2023
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["ts_type"], "number");
    assert_eq!(result.body["is_reasonable"], true);
}

// ============================================================================
// Numeric Edge Cases
// ============================================================================

#[tokio::test]
async fn test_numeric_precision() {
    let (_engine, script_engine, _tmp) = create_test_env();
    
    let ctx = ScriptContext {
        method: "GET".to_string(),
        path: "/numeric".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };
    
    let code = r#"
        return { 
            int = 123456789,
            float = 123.456789,
            neg = -42,
            zero = 0
        }
    "#;
    
    let script = create_script(code);
    let result = script_engine.execute(&script, "testdb", &ctx).await.unwrap();
    
    assert_eq!(result.body["int"], 123456789);
    assert!((result.body["float"].as_f64().unwrap() - 123.456789).abs() < 0.0001);
    assert_eq!(result.body["neg"], -42);
    assert_eq!(result.body["zero"], 0);
}
