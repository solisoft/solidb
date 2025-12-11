//! Tests for Lua scripting transaction functionality

use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::Duration;

const BASE_URL: &str = "http://localhost:16746";

struct TestServer {
    process: Child,
}

impl TestServer {
    fn start() -> Self {
        // Clean up any existing data
        let _ = std::fs::remove_dir_all("./test_lua_tx_data");

        let process = Command::new("cargo")
            .args(["run", "--", "--port", "16746", "--data-dir", "./test_lua_tx_data"])
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
        let _ = std::fs::remove_dir_all("./test_lua_tx_data");
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
fn test_transaction_commit() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // Create a test database and collection
    client
        .post(&format!("{}/_api/database", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({"name": "txdb"}))
        .send()
        .expect("Failed to create database");

    client
        .post(&format!("{}/_api/database/txdb/collection", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({"name": "accounts"}))
        .send()
        .expect("Failed to create collection");

    // Create a script that uses db:transaction
    let create_resp = client
        .post(&format!("{}/_api/database/txdb/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Transaction Test",
            "path": "transfer",
            "methods": ["POST"],
            "code": r#"
                -- Use transaction to insert two accounts atomically
                db:transaction(function(tx)
                    local accounts = tx:collection("accounts")
                    accounts:insert({ _key = "alice", balance = 100 })
                    accounts:insert({ _key = "bob", balance = 50 })
                end)
                
                -- Verify the data was committed
                local accounts = db:collection("accounts")
                local alice = accounts:get("alice")
                local bob = accounts:get("bob")
                
                return {
                    alice_balance = alice.balance,
                    bob_balance = bob.balance,
                    total = alice.balance + bob.balance
                }
            "#
        }))
        .send()
        .expect("Failed to create script");

    assert_eq!(create_resp.status(), 200, "Failed to create script: {:?}", create_resp.text());

    // Execute the script
    let exec_resp = client
        .post(&format!("{}/api/custom/txdb/transfer", BASE_URL))
        .json(&json!({}))
        .send()
        .expect("Failed to execute script");

    let status = exec_resp.status();
    let result: Value = exec_resp.json().expect("Failed to parse response");
    
    if status != 200 {
        println!("Transaction script failed: {:?}", result);
        panic!("Script failed with status {}", status);
    }

    // Verify the transaction committed correctly
    assert_eq!(result["alice_balance"], 100);
    assert_eq!(result["bob_balance"], 50);
    assert_eq!(result["total"], 150);
}

#[test]
fn test_transaction_rollback_on_error() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // Create a test database and collection
    client
        .post(&format!("{}/_api/database", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({"name": "rollbackdb"}))
        .send()
        .expect("Failed to create database");

    client
        .post(&format!("{}/_api/database/rollbackdb/collection", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({"name": "items"}))
        .send()
        .expect("Failed to create collection");

    // Create a script that uses db:transaction but errors out
    client
        .post(&format!("{}/_api/database/rollbackdb/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Rollback Test",
            "path": "rollback-test",
            "methods": ["POST"],
            "code": r#"
                -- This transaction should rollback because of the error
                local ok, err = pcall(function()
                    db:transaction(function(tx)
                        local items = tx:collection("items")
                        items:insert({ _key = "item1", name = "First Item" })
                        -- Force an error to trigger rollback
                        error("Simulated error to trigger rollback")
                    end)
                end)
                
                -- Check if the item was NOT inserted (rollback should have occurred)
                local items = db:collection("items")
                local item1 = items:get("item1")
                
                return {
                    error_occurred = not ok,
                    item_exists = (item1 ~= nil),
                    count = items:count()
                }
            "#
        }))
        .send()
        .expect("Failed to create script");

    // Execute the script
    let exec_resp = client
        .post(&format!("{}/api/custom/rollbackdb/rollback-test", BASE_URL))
        .json(&json!({}))
        .send()
        .expect("Failed to execute script");

    let status = exec_resp.status();
    let result: Value = exec_resp.json().expect("Failed to parse response");
    
    if status != 200 {
        println!("Rollback script failed unexpectedly: {:?}", result);
        panic!("Script failed with status {}", status);
    }

    // Verify: error occurred, item should NOT exist (rolled back), count should be 0
    assert_eq!(result["error_occurred"], true, "Error should have occurred");
    assert_eq!(result["item_exists"], false, "Item should not exist after rollback");
    assert_eq!(result["count"], 0, "Count should be 0 after rollback");
}

#[test]
fn test_transaction_update_and_delete() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // Create database and collection
    client
        .post(&format!("{}/_api/database", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({"name": "updatedb"}))
        .send()
        .expect("Failed to create database");

    client
        .post(&format!("{}/_api/database/updatedb/collection", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({"name": "users"}))
        .send()
        .expect("Failed to create collection");

    // Seed some initial data
    client
        .post(&format!("{}/_api/database/updatedb/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Seeder",
            "path": "seed",
            "methods": ["POST"],
            "code": r#"
                local users = db:collection("users")
                users:insert({ _key = "user1", name = "Alice", status = "active" })
                users:insert({ _key = "user2", name = "Bob", status = "active" })
                users:insert({ _key = "user3", name = "Charlie", status = "active" })
                return { seeded = 3 }
            "#
        }))
        .send()
        .expect("Failed to create seeder");

    client
        .post(&format!("{}/api/custom/updatedb/seed", BASE_URL))
        .json(&json!({}))
        .send()
        .expect("Failed to seed");

    // Create script that updates and deletes in a transaction
    client
        .post(&format!("{}/_api/database/updatedb/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Update Delete Test",
            "path": "modify",
            "methods": ["POST"],
            "code": r#"
                db:transaction(function(tx)
                    local users = tx:collection("users")
                    -- Update user1
                    users:update("user1", { name = "Alice Updated", status = "inactive" })
                    -- Delete user3
                    users:delete("user3")
                end)
                
                local users = db:collection("users")
                return {
                    user1 = users:get("user1"),
                    user2 = users:get("user2"),
                    user3_exists = (users:get("user3") ~= nil),
                    count = users:count()
                }
            "#
        }))
        .send()
        .expect("Failed to create modify script");

    // Execute
    let exec_resp = client
        .post(&format!("{}/api/custom/updatedb/modify", BASE_URL))
        .json(&json!({}))
        .send()
        .expect("Failed to execute modify");

    let status = exec_resp.status();
    let result: Value = exec_resp.json().expect("Failed to parse response");

    if status != 200 {
        println!("Modify script failed: {:?}", result);
        panic!("Script failed with status {}", status);
    }

    // Verify update
    assert_eq!(result["user1"]["name"], "Alice Updated");
    assert_eq!(result["user1"]["status"], "inactive");
    // Verify user2 unchanged
    assert_eq!(result["user2"]["name"], "Bob");
    // Verify user3 deleted
    assert_eq!(result["user3_exists"], false);
    // Verify count is 2
    assert_eq!(result["count"], 2);
}
