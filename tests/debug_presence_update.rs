use solidb::{parse, BindVars, Collection, QueryExecutor, StorageEngine};
use solidb::storage::Database;
use std::sync::Arc;
use tempfile::TempDir;
use serde_json::Value;

#[test]
fn test_presence_update_query() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(StorageEngine::new(temp_dir.path()).unwrap());
    
    // Create database and collection
    storage.create_database("test_db".to_string()).unwrap();
    let db = storage.get_database("test_db").unwrap();
    db.create_collection("users".to_string(), None).unwrap();
    let users = db.get_collection("users").unwrap();

    // Insert a test user
    let user_json = serde_json::json!({
        "username": "testuser",
        "connection_count": 0,
        "status": "offline"
    });
    let inserted = users.insert(user_json).unwrap();
    let user_id = inserted.id.clone();
    let user_key = inserted.key.clone();

    println!("Created user with ID: {} and key: {}", user_id, user_key);

    // Test the specific query used in the Lua script
    let query_string = r#"
        FOR u IN users
        FILTER u._id == @id OR u._key == @id
        UPDATE u WITH {
            connection_count: IF(u.connection_count != null, u.connection_count, 0) + 1,
            status: "online",
            last_seen: DATE_NOW()
        } IN users
        RETURN NEW
    "#;

    // Use explicit BindVars type
    let mut bind_vars: BindVars = std::collections::HashMap::new();
    bind_vars.insert("id".to_string(), serde_json::Value::String(user_id.clone()));

    let executor = QueryExecutor::with_database_and_bind_vars(&storage, "test_db".to_string(), bind_vars);
    let ast = parse(query_string).expect("Failed to parse query");
    let results = executor.execute(&ast).expect("Failed to execute query");

    assert_eq!(results.len(), 1, "Should identify and update one user");
    
    let updated_user = &results[0];
    let count = updated_user.get("connection_count").unwrap().as_f64().unwrap() as i64;
    let status = updated_user.get("status").unwrap().as_str().unwrap();

    assert_eq!(count, 1, "Connection count should increment to 1");
    // status should be online
    assert_eq!(status, "online", "Status should be online");

    // Verify persistence
    let stored_user = users.get(&user_key).unwrap();
    assert_eq!(stored_user.data.get("connection_count").unwrap().as_f64().unwrap() as i64, 1);
}
