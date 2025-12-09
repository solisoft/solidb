//! Tests for Lua scripting functionality

use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::Duration;

const BASE_URL: &str = "http://localhost:16745";

struct TestServer {
    process: Child,
}

impl TestServer {
    fn start() -> Self {
        // Clean up any existing data
        let _ = std::fs::remove_dir_all("./test_lua_data");

        let process = Command::new("cargo")
            .args(["run", "--", "--port", "16745", "--data-dir", "./test_lua_data"])
            .env("SOLIDB_ADMIN_PASSWORD", "admin")
            .spawn()
            .expect("Failed to start server");

        // Wait for server to start
        sleep(Duration::from_secs(15));

        TestServer { process }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = std::fs::remove_dir_all("./test_lua_data");
    }
}

fn get_auth_token(client: &Client) -> String {
    let resp = client
        .post(&format!("{}/auth/login", BASE_URL))
        .json(&json!({"username": "admin", "password": "admin"}))
        .send()
        .expect("Login failed");
    
    let body: Value = resp.json().expect("Failed to parse login response");
    body["token"].as_str().expect("No token in response").to_string()
}

#[test]
fn test_create_and_execute_script() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // Create a simple script that returns a greeting
    let create_resp = client
        .post(&format!("{}/_api/database/_system/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Hello World",
            "path": "hello",
            "methods": ["GET"],
            "collection": "mycol",
            "code": r#"
                return {
                    message = "Hello from Lua!"
                }
            "#,
            "description": "A simple greeting endpoint"
        }))
        .send()
        .expect("Failed to create script");

    assert_eq!(create_resp.status(), 200);
    let created: Value = create_resp.json().expect("Failed to parse create response");
    assert_eq!(created["name"], "Hello World");
    assert_eq!(created["path"], "hello");

    // Execute the script
    let exec_resp = client
        .get(&format!("{}/api/custom/_system/hello", BASE_URL))
        .send()
        .expect("Failed to execute script");

    assert_eq!(exec_resp.status(), 200);
    let result: Value = exec_resp.json().expect("Failed to parse execution response");
    assert_eq!(result["message"], "Hello from Lua!");
}

#[test]
fn test_script_with_database_access() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // Create a test database and collection
    client
        .post(&format!("{}/_api/database", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({"name": "testdb"}))
        .send()
        .expect("Failed to create database");

    client
        .post(&format!("{}/_api/database/testdb/collection", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({"name": "items"}))
        .send()
        .expect("Failed to create collection");

    // Create a script that inserts and retrieves data
    client
        .post(&format!("{}/_api/database/testdb/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "CRUD Items",
            "path": "items",
            "methods": ["GET", "POST"],
            "collection": "items",
            "code": r#"
                local items = db:collection("items")
                
                if request.method == "POST" then
                    -- Insert new item from request body
                    local doc = items:insert(request.body)
                    return { status = "created", item = doc }
                else
                    -- Return all items
                    local all = items:all()
                    return { items = all, count = items:count() }
                end
            "#
        }))
        .send()
        .expect("Failed to create script");

    // POST to create an item
    let post_resp = client
        .post(&format!("{}/api/custom/testdb/items", BASE_URL))
        .json(&json!({"name": "Test Item", "price": 19.99}))
        .send()
        .expect("Failed to POST to script");

    let status = post_resp.status();
    let post_result: Value = post_resp.json().expect("Failed to parse POST response");

    if status != 200 {
        println!("Script error: {:?}", post_result);
        panic!("Script failed with status {}", status);
    }
    
    assert_eq!(status, 200);
    assert_eq!(post_result["status"], "created");
    assert!(post_result["item"]["_key"].is_string());

    // GET all items
    let get_resp = client
        .get(&format!("{}/api/custom/testdb/items", BASE_URL))
        .send()
        .expect("Failed to GET from script");

    assert_eq!(get_resp.status(), 200);
    let get_result: Value = get_resp.json().expect("Failed to parse GET response");
    assert_eq!(get_result["count"], 1);
    assert_eq!(get_result["items"][0]["name"], "Test Item");
}

#[test]
fn test_list_scripts() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // Create a couple of scripts
    for i in 1..=3 {
        client
            .post(&format!("{}/_api/database/_system/scripts", BASE_URL))
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "name": format!("Script {}", i),
                "path": format!("test{}", i),
                "collection": "sys",
                "methods": ["GET"],
                "code": format!("return {{ n = {} }}", i)
            }))
            .send()
            .expect("Failed to create script");
    }

    // List all scripts
    let list_resp = client
        .get(&format!("{}/_api/database/_system/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .expect("Failed to list scripts");

    assert_eq!(list_resp.status(), 200);
    let list: Value = list_resp.json().expect("Failed to parse list response");
    assert!(list["scripts"].as_array().unwrap().len() >= 3);
}

#[test]
fn test_delete_script() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // Create a script
    let create_resp = client
        .post(&format!("{}/_api/database/_system/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "To Delete",
            "path": "deleteme",
            "collection": "temp",
            "methods": ["GET"],
            "code": "return {}"
        }))
        .send()
        .expect("Failed to create script");

    let created: Value = create_resp.json().unwrap();
    let script_id = created["id"].as_str().unwrap();

    // Delete the script
    let delete_resp = client
        .delete(&format!("{}/_api/database/_system/scripts/{}", BASE_URL, script_id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .expect("Failed to delete script");

    assert_eq!(delete_resp.status(), 200);

    // Verify it's gone - calling the endpoint should now 404
    let exec_resp = client
        .get(&format!("{}/api/custom/_system/deleteme", BASE_URL))
        .send()
        .expect("Failed to check deleted script");

    assert_eq!(exec_resp.status(), 404);
}

#[test]
fn test_script_error_handling() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // Create a script with a Lua error
    client
        .post(&format!("{}/_api/database/_system/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Bad Script",
            "path": "bad",
            "collection": "err",
            "methods": ["GET"],
            "code": "error(\"Runtime Error\")"
        }))
        .send()
        .expect("Failed to create script");

    // Execute the bad script
    let exec_resp = client
        .get(&format!("{}/api/custom/_system/bad", BASE_URL))
        .send()
        .expect("Failed to execute script");

    // Should return 500 with an error
    assert_eq!(exec_resp.status(), 500);
    let error: Value = exec_resp.json().expect("Failed to parse error response");
    assert!(error["error"].as_str().unwrap().contains("Lua error"));
}

#[test]
fn test_script_with_query_params() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // Create a script that uses query parameters
    client
        .post(&format!("{}/_api/database/_system/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Query Params Test",
            "path": "echo",
            "collection": "params",
            "methods": ["GET"],
            "code": r#"
                return {
                    method = request.method,
                    path = request.path,
                    name = request.query.name or "anonymous",
                    count = tonumber(request.query.count) or 0
                }
            "#
        }))
        .send()
        .expect("Failed to create script");

    // Call with query params
    let resp = client
        .get(&format!("{}/api/custom/_system/echo?name=test&count=42", BASE_URL))
        .send()
        .expect("Failed to execute script");

    assert_eq!(resp.status(), 200);
    let result: Value = resp.json().expect("Failed to parse response");
    assert_eq!(result["name"], "test");
    assert_eq!(result["count"], 42);
}

#[test]
fn test_script_with_sdbql() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // Create a test database and collection with data
    client
        .post(&format!("{}/_api/database", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({"name": "querydb"}))
        .send()
        .expect("Failed to create database");

    client
        .post(&format!("{}/_api/database/querydb/collection", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({"name": "users"}))
        .send()
        .expect("Failed to create collection");

    // Insert some users
    let users = vec![
        json!({"name": "Alice", "age": 30}),
        json!({"name": "Bob", "age": 25}),
        json!({"name": "Charlie", "age": 35}),
    ];

    for user in users {
        client
            .post(&format!("{}/api/custom/querydb/users", BASE_URL)) // Using direct custom endpoint won't work yet as we haven't made a script for it. Use regular API? No, we don't have regular document API exposed in these tests easily? 
            // Wait, we can use a script to insert data or just assume we have document API.
            // Let's use a setup script to insert data.
            // Actually, let's just use the sdbql script to insert data too! sdbql supports INSERT.
            .json(&json!({})) // Dummy
            .send()
            .ok();
    }
    
    // Better: Helper script to seed data
    client
        .post(&format!("{}/_api/database/querydb/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Seeder",
            "path": "seed",
            "methods": ["POST"],
            "code": r#"
                local users = db:collection("users")
                users:insert({name = "Alice", age = 30})
                users:insert({name = "Bob", age = 25})
                users:insert({name = "Charlie", age = 35})
                return { count = users:count() }
            "#
        }))
        .send()
        .expect("Failed to create seeder");
        
    client.post(&format!("{}/api/custom/querydb/seed", BASE_URL)).send().expect("Failed to seed");

    // Create script using db:query and db:request
    client
        .post(&format!("{}/_api/database/querydb/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "SDBQL Test",
            "path": "query",
            "methods": ["GET"],
            "code": r#"
                -- Test db:query
                local old_users = db:query("FOR u IN users FILTER u.age > @age RETURN u", {age = 30})
                
                return {
                    old = old_users
                }
            "#
        }))
        .send()
        .expect("Failed to create query script");

    // Execute
    let resp = client
        .get(&format!("{}/api/custom/querydb/query", BASE_URL))
        .send()
        .expect("Failed to execute query script");

    let status = resp.status();
    if status != 200 {
        let err: Value = resp.json().unwrap_or(json!({"error": "Unknown"}));
        println!("SDBQL Script Failed: {:?}", err);
        panic!("Script failed with status {}", status);
    }
    
    let result: Value = resp.json().expect("Failed to parse response");
    
    // Verify old users (Charlie 35)
    let old = result["old"].as_array().expect("Result 'old' should be array");
    assert!(old.len() >= 1);
}

#[test]
fn test_script_with_url_params() {
    let _server = TestServer::start(); 
    let client = Client::new();
    let token = get_auth_token(&client);

    // 1. Create script with params
    let script_code = r#"
        return {
            id = request.params.id,
            action = request.params.action,
            message = "Processed customer " .. (request.params.id or "unknown")
        }
    "#;

    let resp = client
        .post(&format!("{}/_api/database/_system/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Customer Action",
            "path": "customers/:id/:action",
            "methods": ["GET"],
            "code": script_code,
            "database": "_system"
        }))
        .send()
        .expect("Failed to create script");

    assert_eq!(resp.status(), 200, "Failed to create script: {:?}", resp.text());

    // 2. Execute script
    let resp = client
        .get(&format!("{}/api/custom/_system/customers/555/archive", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .expect("Failed to execute script");

    assert_eq!(resp.status(), 200, "Script execution failed: {:?}", resp.text());
    let body: serde_json::Value = resp.json().expect("Failed to parse response");
    
    assert_eq!(body["id"], "555");
    assert_eq!(body["action"], "archive");
    assert_eq!(body["message"], "Processed customer 555");
}
