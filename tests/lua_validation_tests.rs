//! Enhanced Lua Validation Tests
//!
//! Tests for:
//! - JSON schema validation
//! - Input sanitization
//! - Enhanced type checking
//! - Schema management

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
async fn test_validate_basic_schema() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local schema = {
            type = "object",
            properties = {
                name = { type = "string", minLength = 1 },
                age = { type = "number", minimum = 0 }
            },
            required = {"name"}
        }

        local valid_data = { name = "Alice", age = 30 }
        local invalid_data = { age = 30 }  -- missing required name

        return {
            valid = solidb.validate(valid_data, schema),
            invalid = solidb.validate(invalid_data, schema)
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("valid").unwrap().as_bool().unwrap(), true);
    assert_eq!(body.get("invalid").unwrap().as_bool().unwrap(), false);
}

#[tokio::test]
async fn test_validate_detailed_errors() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local schema = {
            type = "object",
            properties = {
                email = { type = "string", format = "email" },
                age = { type = "number", minimum = 18, maximum = 120 }
            },
            required = {"email", "age"}
        }

        local invalid_data = {
            email = "not-an-email",
            age = 15
        }

        local result = solidb.validate_detailed(invalid_data, schema)

        return {
            valid = result.valid,
            error_count = #result.errors,
            first_error = result.errors[1].message
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("valid").unwrap().as_bool().unwrap(), false);
    assert!(body.get("error_count").unwrap().as_i64().unwrap() > 0);
}

#[tokio::test]
async fn test_sanitize_input() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local dirty_data = {
            name = "  Alice Smith  ",
            email = "ALICE@EXAMPLE.COM",
            message = "  Hello   World  "
        }

        local operations = {
            trim = true,
            lowercase = {"email"}
        }

        local clean_data = solidb.sanitize(dirty_data, operations)

        return {
            name = clean_data.name,
            email = clean_data.email,
            message = clean_data.message
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("name").unwrap().as_str().unwrap(), "Alice Smith");
    assert_eq!(
        body.get("email").unwrap().as_str().unwrap(),
        "alice@example.com"
    );
    assert_eq!(
        body.get("message").unwrap().as_str().unwrap(),
        "Hello World"
    );
}

#[tokio::test]
async fn test_enhanced_typeof() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        return {
            string_type = solidb.typeof("hello"),
            number_type = solidb.typeof(42),
            boolean_type = solidb.typeof(true),
            table_type = solidb.typeof({}),
            nil_type = solidb.typeof(nil),
            function_type = solidb.typeof(function() end)
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("string_type").unwrap().as_str().unwrap(), "string");
    assert_eq!(body.get("number_type").unwrap().as_str().unwrap(), "number");
    assert_eq!(
        body.get("boolean_type").unwrap().as_str().unwrap(),
        "boolean"
    );
    assert_eq!(body.get("table_type").unwrap().as_str().unwrap(), "table");
    assert_eq!(body.get("nil_type").unwrap().as_str().unwrap(), "nil");
    assert_eq!(
        body.get("function_type").unwrap().as_str().unwrap(),
        "function"
    );
}

#[tokio::test]
async fn test_schema_storage_and_retrieval() {
    let (engine, script_engine, _tmp) = create_test_env();

    // Create _schemas collection
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("_schemas".to_string(), None).unwrap();

    let code = r#"
        -- Store a schema
        local user_schema = {
            type = "object",
            properties = {
                username = { type = "string", minLength = 3, maxLength = 20 },
                email = { type = "string", format = "email" },
                created_at = { type = "string", format = "date-time" }
            },
            required = {"username", "email"}
        }

        local schemas = db:collection("_schemas")
        local stored = schemas:insert({
            name = "user_schema",
            version = "1.0",
            schema = user_schema
        })

        -- Retrieve and validate
        local retrieved = schemas:get(stored._key)
        local test_data = {
            username = "alice",
            email = "alice@example.com",
            created_at = "2024-01-01T00:00:00Z"
        }

        return {
            stored_key = stored._key,
            schema_valid = solidb.validate(test_data, retrieved.schema),
            schema_name = retrieved.name
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert!(body.contains_key("stored_key"));
    assert_eq!(body.get("schema_valid").unwrap().as_bool().unwrap(), true);
    assert_eq!(
        body.get("schema_name").unwrap().as_str().unwrap(),
        "user_schema"
    );
}

#[tokio::test]
async fn test_validation_with_nested_objects() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local schema = {
            type = "object",
            properties = {
                user = {
                    type = "object",
                    properties = {
                        profile = {
                            type = "object",
                            properties = {
                                name = { type = "string", minLength = 1 },
                                preferences = {
                                    type = "object",
                                    properties = {
                                        theme = { type = "string", enum = {"light", "dark"} },
                                        notifications = { type = "boolean" }
                                    },
                                    required = {"theme"}
                                }
                            },
                            required = {"name", "preferences"}
                        }
                    },
                    required = {"profile"}
                }
            },
            required = {"user"}
        }

        local valid_data = {
            user = {
                profile = {
                    name = "Alice",
                    preferences = {
                        theme = "dark",
                        notifications = true
                    }
                }
            }
        }

        local invalid_data = {
            user = {
                profile = {
                    name = "",
                    preferences = {
                        theme = "invalid"
                    }
                }
            }
        }

        return {
            valid = solidb.validate(valid_data, schema),
            invalid = solidb.validate(invalid_data, schema)
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("valid").unwrap().as_bool().unwrap(), true);
    assert_eq!(body.get("invalid").unwrap().as_bool().unwrap(), false);
}

#[tokio::test]
async fn test_array_validation() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local schema = {
            type = "object",
            properties = {
                tags = {
                    type = "array",
                    items = { type = "string", minLength = 1 },
                    minItems = 1,
                    maxItems = 5
                },
                scores = {
                    type = "array",
                    items = { type = "number", minimum = 0, maximum = 100 }
                }
            },
            required = {"tags"}
        }

        local valid_data = {
            tags = {"urgent", "important"},
            scores = {85, 92, 78}
        }

        local invalid_data = {
            tags = {},  -- empty array, violates minItems
            scores = {85, 105}  -- 105 exceeds maximum
        }

        return {
            valid = solidb.validate(valid_data, schema),
            invalid = solidb.validate(invalid_data, schema)
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("valid").unwrap().as_bool().unwrap(), true);
    assert_eq!(body.get("invalid").unwrap().as_bool().unwrap(), false);
}
