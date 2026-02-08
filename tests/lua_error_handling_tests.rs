//! Enhanced Lua Error Handling Tests
//!
//! Tests for:
//! - Standardized error responses
//! - Assertion utilities
//! - Try-catch patterns
//! - Error formatting and reporting

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
        service: "default".to_string(),
        collection: None,
        code: code.to_string(),
        description: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
    }
}

#[tokio::test]
async fn test_basic_error_functionality() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        solidb.error("Validation failed", 400)
        return { should_not_reach = "here" }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    match script_engine.execute(&script, "testdb", &ctx).await {
        Ok(_) => panic!("Expected error, but got success"),
        Err(e) => {
            let error_msg = e.to_string();
            assert!(error_msg.contains("ERROR:400:Validation failed"));
        }
    }
}

#[tokio::test]
async fn test_error_with_default_code() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        solidb.error("Internal server error")
        return { should_not_reach = "here" }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    match script_engine.execute(&script, "testdb", &ctx).await {
        Ok(_) => panic!("Expected error, but got success"),
        Err(e) => {
            let error_msg = e.to_string();
            assert!(error_msg.contains("ERROR:500:Internal server error"));
        }
    }
}

#[tokio::test]
async fn test_assertion_success() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local valid_data = { name = "Alice", age = 30 }
        solidb.assert(valid_data.name ~= nil, "Name is required")
        solidb.assert(valid_data.age > 0, "Age must be positive")

        return { success = true, message = "All assertions passed" }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("success").unwrap().as_bool().unwrap(), true);
    assert_eq!(
        body.get("message").unwrap().as_str().unwrap(),
        "All assertions passed"
    );
}

#[tokio::test]
async fn test_assertion_failure() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local invalid_data = { name = nil, age = -5 }
        solidb.assert(invalid_data.name ~= nil, "Name cannot be nil")
        solidb.assert(invalid_data.age > 0, "Age must be positive")

        return { should_not_reach = "here" }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    match script_engine.execute(&script, "testdb", &ctx).await {
        Ok(_) => panic!("Expected assertion error, but got success"),
        Err(e) => {
            let error_msg = e.to_string();
            assert!(error_msg.contains("ASSERT:Name cannot be nil"));
        }
    }
}

#[tokio::test]
async fn test_try_catch_success_pattern() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local function risky_operation()
            return { success = true, data = "operation completed" }
        end

        local result = solidb.try(risky_operation, function(error)
            return { success = false, error = error }
        end)

        return result
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("success").unwrap().as_bool().unwrap(), true);
    assert_eq!(
        body.get("data").unwrap().as_str().unwrap(),
        "operation completed"
    );
}

#[tokio::test]
async fn test_try_catch_failure_pattern() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local function risky_operation()
            solidb.error("Something went wrong!", 500)
        end

        local result = solidb.try(risky_operation, function(error)
            return {
                success = false,
                error_message = error,
                handled = true
            }
        end)

        return result
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("success").unwrap().as_bool().unwrap(), false);
    assert_eq!(body.get("handled").unwrap().as_bool().unwrap(), true);
    assert!(body.get("error_message").unwrap().is_string());
}

#[tokio::test]
async fn test_try_without_catch() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local function risky_operation()
            solidb.error("Unhandled error", 400)
        end

        -- No catch function provided
        local result = solidb.try(risky_operation)

        return result
    "#;

    let script = create_script(code);
    let ctx = create_context();

    match script_engine.execute(&script, "testdb", &ctx).await {
        Ok(_) => panic!("Expected error, but got success"),
        Err(e) => {
            let error_msg = e.to_string();
            assert!(error_msg.contains("Unhandled error"));
        }
    }
}

#[tokio::test]
async fn test_nested_error_handling() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local function validate_user(user)
            solidb.assert(user.name, "Name is required")
            solidb.assert(user.email, "Email is required")
            solidb.assert(user.age >= 18, "User must be 18 or older")
            return { valid = true }
        end

        local function process_user(user)
            return solidb.try(function()
                return solidb.try(function()
                    return validate_user(user)
                end, function(error)
                    solidb.error("Validation failed: " .. error, 400)
                end)
            end, function(error)
                solidb.error("Processing failed: " .. error, 500)
            end)
        end

        local valid_user = { name = "Alice", email = "alice@example.com", age = 25 }
        local invalid_user = { name = "", email = "invalid", age = 16 }

        local result1 = process_user(valid_user)
        local result2 = process_user(invalid_user)

        return {
            valid_result = result1,
            invalid_result = result2
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    match script_engine.execute(&script, "testdb", &ctx).await {
        Ok(_) => panic!("Expected error, but got success"),
        Err(e) => {
            let error_msg = e.to_string();
            assert!(
                error_msg.contains("Validation failed") || error_msg.contains("Processing failed")
            );
        }
    }
}

#[tokio::test]
async fn test_error_with_context() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local function validate_order(order)
            if not order.customer_id then
                solidb.error("Customer ID is required for order validation", 400)
            end

            if not order.items or #order.items == 0 then
                solidb.error("Order must contain at least one item", 400)
            end

            if order.total <= 0 then
                solidb.error("Order total must be positive", 400)
            end

            return { valid = true }
        end

        local order = {
            customer_id = nil,
            items = {},
            total = -50
        }

        -- This will fail on the first validation
        validate_order(order)

        return { success = true }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    match script_engine.execute(&script, "testdb", &ctx).await {
        Ok(_) => panic!("Expected error, but got success"),
        Err(e) => {
            let error_msg = e.to_string();
            assert!(error_msg.contains("Customer ID is required"));
        }
    }
}

#[tokio::test]
async fn test_async_error_handling() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local async_operation = solidb.try(function()
            time.sleep(100)  -- Wait 100ms
            solidb.error("Async operation failed", 503)
        end, function(error)
            return {
                error_caught = true,
                message = error,
                async_handled = true
            }
        end)

        return async_operation
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("error_caught").unwrap().as_bool().unwrap(), true);
    assert_eq!(body.get("async_handled").unwrap().as_bool().unwrap(), true);
    assert!(body.get("message").unwrap().is_string());
}

#[tokio::test]
async fn test_error_codes_and_messages() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local test_cases = {
            { code = 400, message = "Bad Request" },
            { code = 401, message = "Unauthorized" },
            { code = 403, message = "Forbidden" },
            { code = 404, message = "Not Found" },
            { code = 500, message = "Internal Server Error" },
            { code = 503, message = "Service Unavailable" }
        }

        local errors = {}
        for i, test_case in ipairs(test_cases) do
            solidb.try(function()
                solidb.error(test_case.message, test_case.code)
            end, function(error)
                errors[i] = {
                    code = test_case.code,
                    message = test_case.message,
                    error = error
                }
            end)
        end

        return {
            errors_tested = #errors,
            first_error = errors[1].error,
            last_error = errors[#errors].error
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("errors_tested").unwrap().as_i64().unwrap(), 6);
    assert!(body.get("first_error").unwrap().is_string());
    assert!(body.get("last_error").unwrap().is_string());
}
