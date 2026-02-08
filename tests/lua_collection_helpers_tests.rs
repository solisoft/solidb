//! Enhanced Lua Collection Helpers Tests
//!
//! Tests for:
//! - Simplified find operations
//! - Bulk operations
//! - Upsert functionality
//! - Count with filters
//! - Enhanced collection methods

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
async fn test_collection_find_with_filter() {
    let (engine, script_engine, _tmp) = create_test_env();

    // Create test collection and data
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("users".to_string(), None).unwrap();
    let users = db.get_collection("users").unwrap();

    users
        .insert(json!({"_key": "1", "name": "Alice", "age": 30, "status": "active"}))
        .unwrap();
    users
        .insert(json!({"_key": "2", "name": "Bob", "age": 25, "status": "inactive"}))
        .unwrap();
    users
        .insert(json!({"_key": "3", "name": "Charlie", "age": 35, "status": "active"}))
        .unwrap();

    let code = r#"
        local users = db:collection("users")
        local active_users = users:find({ status = "active" })

        local results = {}
        for i, user in ipairs(active_users) do
            results[i] = {
                name = user.name,
                age = user.age,
                status = user.status
            }
        end

        return {
            count = #results,
            active_users = results
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("count").unwrap().as_i64().unwrap(), 2);
    let active_users = body.get("active_users").unwrap().as_array().unwrap();
    assert_eq!(active_users.len(), 2);
}

#[tokio::test]
async fn test_find_one_single_document() {
    let (engine, script_engine, _tmp) = create_test_env();

    // Create test collection and data
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("products".to_string(), None).unwrap();
    let products = db.get_collection("products").unwrap();

    products
        .insert(json!({"_key": "1", "name": "Laptop", "price": 999.99, "in_stock": true}))
        .unwrap();
    products
        .insert(json!({"_key": "2", "name": "Mouse", "price": 29.99, "in_stock": false}))
        .unwrap();

    let code = r#"
        local products = db:collection("products")
        local laptop = products:find_one({ name = "Laptop" })
        local out_of_stock = products:find_one({ in_stock = false })
        local not_found = products:find_one({ name = "Tablet" })

        return {
            laptop_found = laptop ~= nil,
            laptop_name = laptop and laptop.name,
            out_of_stock_found = out_of_stock ~= nil,
            out_of_stock_name = out_of_stock and out_of_stock.name,
            not_found = not_found == nil
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("laptop_found").unwrap().as_bool().unwrap(), true);
    assert_eq!(body.get("laptop_name").unwrap().as_str().unwrap(), "Laptop");
    assert_eq!(
        body.get("out_of_stock_found").unwrap().as_bool().unwrap(),
        true
    );
    assert_eq!(
        body.get("out_of_stock_name").unwrap().as_str().unwrap(),
        "Mouse"
    );
    assert_eq!(body.get("not_found").unwrap().as_bool().unwrap(), true);
}

#[tokio::test]
async fn test_upsert_operation() {
    let (engine, script_engine, _tmp) = create_test_env();

    // Create test collection
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("counters".to_string(), None).unwrap();

    let code = r#"
        local counters = db:collection("counters")

        -- First upsert - should insert
        local result1 = counters:upsert("page_views", { count = 1, last_updated = solidb.now() })

        -- Second upsert - should update
        local result2 = counters:upsert("page_views", { count = 2, last_updated = solidb.now() })

        return {
            first_operation = result1._key,
            first_count = result1.count,
            second_operation = result2._key,
            second_count = result2.count,
            is_same_key = result1._key == result2._key
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("first_count").unwrap().as_i64().unwrap(), 1);
    assert_eq!(body.get("second_count").unwrap().as_i64().unwrap(), 2);
    assert_eq!(body.get("is_same_key").unwrap().as_bool().unwrap(), true);
}

#[tokio::test]
async fn test_bulk_insert_operation() {
    let (engine, script_engine, _tmp) = create_test_env();

    // Create test collection
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("logs".to_string(), None).unwrap();

    let code = r#"
        local logs = db:collection("logs")

        local new_logs = {
            { level = "info", message = "User login", timestamp = solidb.now() },
            { level = "warn", message = "High memory usage", timestamp = solidb.now() },
            { level = "error", message = "Database connection failed", timestamp = solidb.now() },
            { level = "info", message = "User logout", timestamp = solidb.now() }
        }

        local inserted = logs:bulk_insert(new_logs)

        local results = {}
        for i, log in ipairs(inserted) do
            results[i] = {
                level = log.level,
                message = log.message,
                has_key = log._key ~= nil
            }
        end

        return {
            inserted_count = #inserted,
            logs = results
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("inserted_count").unwrap().as_i64().unwrap(), 4);
    let logs = body.get("logs").unwrap().as_array().unwrap();
    assert_eq!(logs.len(), 4);

    for i in 0..4 {
        let log = &logs[i];
        assert!(log.get("has_key").unwrap().as_bool().unwrap());
        assert!(log.get("level").unwrap().is_string());
        assert!(log.get("message").unwrap().is_string());
    }
}

#[tokio::test]
async fn test_count_with_filters() {
    let (engine, script_engine, _tmp) = create_test_env();

    // Create test collection and data
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("orders".to_string(), None).unwrap();
    let orders = db.get_collection("orders").unwrap();

    orders
        .insert(json!({"_key": "1", "status": "completed", "total": 100.0}))
        .unwrap();
    orders
        .insert(json!({"_key": "2", "status": "pending", "total": 50.0}))
        .unwrap();
    orders
        .insert(json!({"_key": "3", "status": "completed", "total": 200.0}))
        .unwrap();
    orders
        .insert(json!({"_key": "4", "status": "cancelled", "total": 75.0}))
        .unwrap();
    orders
        .insert(json!({"_key": "5", "status": "completed", "total": 150.0}))
        .unwrap();

    let code = r#"
        local orders = db:collection("orders")

        local total_count = orders:count()
        local completed_count = orders:count({ status = "completed" })
        -- Count high value orders manually (filter doesn't support >= yet)
        local all_orders = orders:find({})
        local high_value_count = 0
        for _, order in ipairs(all_orders) do
            if order.total and order.total >= 100 then
                high_value_count = high_value_count + 1
            end
        end
        local pending_count = orders:count({ status = "pending" })
        local cancelled_count = orders:count({ status = "cancelled" })

        return {
            total = total_count,
            completed = completed_count,
            high_value = high_value_count,
            pending = pending_count,
            cancelled = cancelled_count
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("total").unwrap().as_i64().unwrap(), 5);
    assert_eq!(body.get("completed").unwrap().as_i64().unwrap(), 3);
    assert_eq!(body.get("high_value").unwrap().as_i64().unwrap(), 3);
    assert_eq!(body.get("pending").unwrap().as_i64().unwrap(), 1);
    assert_eq!(body.get("cancelled").unwrap().as_i64().unwrap(), 1);
}

#[tokio::test]
async fn test_complex_filter_combinations() {
    let (engine, script_engine, _tmp) = create_test_env();

    // Create test collection and data
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("events".to_string(), None).unwrap();
    let events = db.get_collection("events").unwrap();

    events.insert(json!({"_key": "1", "type": "click", "user_id": "u1", "timestamp": 1640995200, "value": 10.0})).unwrap();
    events.insert(json!({"_key": "2", "type": "view", "user_id": "u2", "timestamp": 1640995260, "value": 5.0})).unwrap();
    events.insert(json!({"_key": "3", "type": "click", "user_id": "u1", "timestamp": 1640995320, "value": 15.0})).unwrap();
    events.insert(json!({"_key": "4", "type": "purchase", "user_id": "u1", "timestamp": 1640995380, "value": 100.0})).unwrap();

    let code = r#"
        local events = db:collection("events")

        -- Find events for user u1 and filter for value >= 10 manually
        local user1_events = events:find({ user_id = "u1" })
        local user1_high_value = {}
        for _, event in ipairs(user1_events) do
            if event.value and event.value >= 10 then
                table.insert(user1_high_value, event)
            end
        end

        -- Find click events
        local click_events = events:find({ type = "click" })

        -- Count recent click events manually
        local recent_click_count = 0
        for _, event in ipairs(click_events) do
            if event.timestamp and event.timestamp >= 1640995200 and event.timestamp <= 1640995400 then
                recent_click_count = recent_click_count + 1
            end
        end

        return {
            user1_high_value_count = #user1_high_value,
            click_count = #click_events,
            recent_click_count = recent_click_count
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    // Events for u1 with value >= 10: event 1 (10.0), event 3 (15.0), event 4 (100.0) = 3
    assert_eq!(
        body.get("user1_high_value_count")
            .unwrap()
            .as_i64()
            .unwrap(),
        3
    );
    assert_eq!(body.get("click_count").unwrap().as_i64().unwrap(), 2);
    assert_eq!(body.get("recent_click_count").unwrap().as_i64().unwrap(), 2);
}

#[tokio::test]
async fn test_bulk_insert_with_validation() {
    let (engine, script_engine, _tmp) = create_test_env();

    // Create test collection and schema
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("users".to_string(), None).unwrap();

    let code = r#"
        local users = db:collection("users")

        -- Define a simple schema for validation
        local user_schema = {
            type = "object",
            properties = {
                email = { type = "string" },
                age = { type = "number" }
            },
            required = {"email"}
        }

        local valid_users = {
            { email = "alice@example.com", age = 30 },
            { email = "bob@example.com", age = 25 }
        }

        local invalid_users = {
            { age = 30 },  -- missing required email
            { name = "test" }  -- also missing email
        }

        -- Validate before inserting
        local valid_insertions = {}
        for i, user in ipairs(valid_users) do
            if solidb.validate(user, user_schema) then
                local result = users:insert(user)
                table.insert(valid_insertions, result._key)
            end
        end

        local invalid_count = 0
        for i, user in ipairs(invalid_users) do
            if not solidb.validate(user, user_schema) then
                invalid_count = invalid_count + 1
            end
        end

        return {
            valid_insertions = #valid_insertions,
            invalid_detected = invalid_count
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("valid_insertions").unwrap().as_i64().unwrap(), 2);
    assert_eq!(body.get("invalid_detected").unwrap().as_i64().unwrap(), 2);
}

#[tokio::test]
async fn test_collection_helper_chaining() {
    let (engine, script_engine, _tmp) = create_test_env();

    // Create test collection
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("metrics".to_string(), None).unwrap();

    let code = r#"
        local metrics = db:collection("metrics")

        -- Insert initial data
        local initial_data = {
            { metric = "cpu", value = 45.2, host = "server1" },
            { metric = "memory", value = 78.5, host = "server1" },
            { metric = "cpu", value = 38.1, host = "server2" },
            { metric = "memory", value = 65.3, host = "server2" }
        }

        local inserted = metrics:bulk_insert(initial_data)

        -- Query for CPU metrics
        local cpu_metrics = metrics:find({ metric = "cpu" })

        -- Count metrics by host
        local server1_count = metrics:count({ host = "server1" })
        local server2_count = metrics:count({ host = "server2" })

        -- Find high CPU usage manually
        local high_cpu = {}
        for _, m in ipairs(cpu_metrics) do
            if m.value and m.value >= 40 then
                table.insert(high_cpu, m)
            end
        end

        return {
            total_inserted = #inserted,
            cpu_metrics_count = #cpu_metrics,
            server1_count = server1_count,
            server2_count = server2_count,
            high_cpu_count = #high_cpu
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("total_inserted").unwrap().as_i64().unwrap(), 4);
    assert_eq!(body.get("cpu_metrics_count").unwrap().as_i64().unwrap(), 2);
    assert_eq!(body.get("server1_count").unwrap().as_i64().unwrap(), 2);
    assert_eq!(body.get("server2_count").unwrap().as_i64().unwrap(), 2);
    assert_eq!(body.get("high_cpu_count").unwrap().as_i64().unwrap(), 1);
}
