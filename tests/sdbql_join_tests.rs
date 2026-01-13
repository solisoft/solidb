// Tests for SDBQL JOIN operations
use serde_json::json;
use solidb::error::DbResult;
use solidb::sdbql::{parse, QueryExecutor};
use solidb::storage::StorageEngine;

// Helper to create a test storage engine with sample data
fn setup_test_data() -> DbResult<(StorageEngine, String)> {
    let storage = StorageEngine::new("/tmp/test_join_db")?;
    
    // Use unique database name with timestamp to avoid conflicts
    let db_name = format!("test_db_{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis());
    
    storage.create_database(db_name.clone())?;
    
    // Create collections
    storage.create_collection(format!("{}:users", db_name), None)?;
    storage.create_collection(format!("{}:orders", db_name), None)?;
    storage.create_collection(format!("{}:profiles", db_name), None)?;
    
    // Insert users
    let users_coll = storage.get_collection(&format!("{}:users", db_name))?;
    users_coll.insert(json!({"_key": "u1", "name": "Alice", "age": 30}))?;
    users_coll.insert(json!({"_key": "u2", "name": "Bob", "age": 25}))?;
    users_coll.insert(json!({"_key": "u3", "name": "Charlie", "age": 35}))?;
    
    // Insert orders
    let orders_coll = storage.get_collection(&format!("{}:orders", db_name))?;
    orders_coll.insert(json!({"_key": "o1", "user_key": "u1", "total": 100}))?;
    orders_coll.insert(json!({"_key": "o2", "user_key": "u1", "total": 200}))?;
    orders_coll.insert(json!({"_key": "o3", "user_key": "u2", "total": 150}))?;
    // Note: u3 (Charlie) has no orders
    
    // Insert profiles (sparse - not all users have profiles)
    let profiles_coll = storage.get_collection(&format!("{}:profiles", db_name))?;
    profiles_coll.insert(json!({"_key": "p1", "user_key": "u1", "bio": "Software engineer"}))?;
    profiles_coll.insert(json!({"_key": "p2", "user_key": "u2", "bio": "Designer"}))?;
    // Note: u3 (Charlie) has no profile
    
    Ok((storage, db_name))
}

#[test]
fn test_parse_basic_join() -> DbResult<()> {
    let query = "FOR user IN users JOIN orders ON user._key == orders.user_key RETURN {user, orders}";
    let parsed = parse(query)?;
    
    assert_eq!(parsed.join_clauses.len(), 1);
    assert_eq!(parsed.join_clauses[0].variable, "orders");
    assert_eq!(parsed.join_clauses[0].collection, "orders");
    
    Ok(())
}

#[test]
fn test_parse_left_join() -> DbResult<()> {
    let query = "FOR user IN users LEFT JOIN profiles ON user._key == profiles.user_key RETURN {user, profiles}";
    let parsed = parse(query)?;
    
    assert_eq!(parsed.join_clauses.len(), 1);
    assert_eq!(parsed.join_clauses[0].variable, "profiles");
    assert!(matches!(parsed.join_clauses[0].join_type, solidb::sdbql::ast::JoinType::Left));
    
    Ok(())
}

#[test]
fn test_parse_multiple_joins() -> DbResult<()> {
    let query = r#"
        FOR user IN users
          JOIN orders ON user._key == orders.user_key
          LEFT JOIN profiles ON user._key == profiles.user_key
          RETURN {user, orders, profiles}
    "#;
    let parsed = parse(query)?;
    
    assert_eq!(parsed.join_clauses.len(), 2);
    assert_eq!(parsed.join_clauses[0].variable, "orders");
    assert_eq!(parsed.join_clauses[1].variable, "profiles");
    
    Ok(())
}

#[test]
fn test_execute_basic_inner_join() -> DbResult<()> {
    let (storage, db_name) = setup_test_data()?;
    let executor = QueryExecutor::with_database(&storage, db_name);
    
    let query_str = r#"
        FOR user IN users
          JOIN orders ON user._key == orders.user_key
          RETURN {user_name: user.name, orders: orders}
    "#;
    
    let query = parse(query_str)?;
    let results = executor.execute(&query)?;
    
    // Should have 2 results (Alice and Bob have orders, Charlie doesn't)
    assert_eq!(results.len(), 2);
    
    // Check Alice's orders (should have 2 orders in array)
    let alice = results.iter().find(|r| r["user_name"] == "Alice").unwrap();
    assert_eq!(alice["orders"].as_array().unwrap().len(), 2);
    
    // Check Bob's orders (should have 1 order in array)
    let bob = results.iter().find(|r| r["user_name"] == "Bob").unwrap();
    assert_eq!(bob["orders"].as_array().unwrap().len(), 1);
    
    Ok(())
}

#[test]
fn test_execute_left_join() -> DbResult<()> {
    let (storage, db_name) = setup_test_data()?;
    let executor = QueryExecutor::with_database(&storage, db_name);
    
    let query_str = r#"
        FOR user IN users
          LEFT JOIN profiles ON user._key == profiles.user_key
          RETURN {user_name: user.name, profiles: profiles}
    "#;
    
    let query = parse(query_str)?;
    let results = executor.execute(&query)?;
    
    // Should have 3 results (all users included with LEFT JOIN)
    assert_eq!(results.len(), 3);
    
    // Check Alice has profile
    let alice = results.iter().find(|r| r["user_name"] == "Alice").unwrap();
    assert_eq!(alice["profiles"].as_array().unwrap().len(), 1);
    
    // Check Charlie has empty array (no profile)
    let charlie = results.iter().find(|r| r["user_name"] == "Charlie").unwrap();
    assert_eq!(charlie["profiles"].as_array().unwrap().len(), 0);
    
    Ok(())
}

#[test]
fn test_multiple_joins() -> DbResult<()> {
    let (storage, db_name) = setup_test_data()?;
    let executor = QueryExecutor::with_database(&storage, db_name);
    
    let query_str = r#"
        FOR user IN users
          JOIN orders ON user._key == orders.user_key
          LEFT JOIN profiles ON user._key == profiles.user_key
          RETURN {
            user_name: user.name,
            order_count: LENGTH(orders),
            has_profile: LENGTH(profiles) > 0
          }
    "#;
    
    let query = parse(query_str)?;
    let results = executor.execute(&query)?;
    
    // Should have 2 results (only users with orders due to INNER JOIN)
    assert_eq!(results.len(), 2);
    
    // Check Alice
    let alice = results.iter().find(|r| r["user_name"] == "Alice").unwrap();
    assert_eq!(alice["order_count"], 2);
    assert_eq!(alice["has_profile"], true);
    
    // Check Bob
    let bob = results.iter().find(|r| r["user_name"] == "Bob").unwrap();
    assert_eq!(bob["order_count"], 1);
    assert_eq!(bob["has_profile"], true);
    
    Ok(())
}

#[test]
fn test_join_with_filter() -> DbResult<()> {
    let (storage, db_name) = setup_test_data()?;
    let executor = QueryExecutor::with_database(&storage, db_name);
    
    let query_str = r#"
        FOR user IN users
          JOIN orders ON user._key == orders.user_key
          FILTER LENGTH(orders) > 1
          RETURN {user_name: user.name, order_count: LENGTH(orders)}
    "#;
    
    let query = parse(query_str)?;
    let results = executor.execute(&query)?;
    
    // Only Alice has more than 1 order
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["user_name"], "Alice");
    assert_eq!(results[0]["order_count"], 2);
    
    Ok(())
}

#[test]
fn test_join_with_complex_condition() -> DbResult<()> {
    let (storage, db_name) = setup_test_data()?;
    let executor = QueryExecutor::with_database(&storage, db_name);
    
    let query_str = r#"
        FOR user IN users
          JOIN orders ON user._key == orders.user_key AND orders.total >= 150
          RETURN {user_name: user.name, high_value_orders: orders}
    "#;
    
    let query = parse(query_str)?;
    let results = executor.execute(&query)?;
    
    // Alice has 1 order >= 150 (o2: 200), Bob has 1 order >= 150 (o3: 150)
    assert_eq!(results.len(), 2);
    
    // Check that Alice only has the high-value order
    let alice = results.iter().find(|r| r["user_name"] == "Alice").unwrap();
    let alice_orders = alice["high_value_orders"].as_array().unwrap();
    assert_eq!(alice_orders.len(), 1);
    assert_eq!(alice_orders[0]["total"], 200);
    
    Ok(())
}

#[test]
fn test_join_with_aggregation() -> DbResult<()> {
    let (storage, db_name) = setup_test_data()?;
    let executor = QueryExecutor::with_database(&storage, db_name);
    
    let query_str = r#"
        FOR user IN users
          JOIN orders ON user._key == orders.user_key
          RETURN {
            user_name: user.name,
            total_spent: SUM(orders[*].total)
          }
    "#;
    
    let query = parse(query_str)?;
    let results = executor.execute(&query)?;
    
    // Alice: 100 + 200 = 300, Bob: 150
    let alice = results.iter().find(|r| r["user_name"] == "Alice").unwrap();
    assert_eq!(alice["total_spent"].as_f64().unwrap(), 300.0);
    
    let bob = results.iter().find(|r| r["user_name"] == "Bob").unwrap();
    assert_eq!(bob["total_spent"].as_f64().unwrap(), 150.0);
    
    Ok(())
}

#[test] 
fn test_join_empty_collection() -> DbResult<()> {
    let storage = StorageEngine::new("/tmp/test_join_empty_db")?;
    
    let db_name = format!("empty_test_{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis());
    
    storage.create_database(db_name.clone())?;
    storage.create_collection(format!("{}:users", db_name), None)?;
    storage.create_collection(format!("{}:orders", db_name), None)?;;
    
    // Insert only users, no orders
    let users_coll = storage.get_collection(&format!("{}:users", db_name))?;
    users_coll.insert(json!({"_key": "u1", "name": "Alice"}))?;
    
    let executor = QueryExecutor::with_database(&storage, db_name);
    
    let query_str = "FOR user IN users JOIN orders ON user._key == orders.user_key RETURN user";
    let query = parse(query_str)?;
    let results = executor.execute(&query)?;
    
    // No results because INNER JOIN with empty orders collection
    assert_eq!(results.len(), 0);
    
    Ok(())
}

// ========== RIGHT JOIN Tests ==========

#[test]
fn test_parse_right_join() -> DbResult<()> {
    let query = "FOR user IN users RIGHT JOIN orders ON user._key == orders.user_key RETURN {user, orders}";
    let parsed = parse(query)?;
    
    assert_eq!(parsed.join_clauses.len(), 1);
    assert_eq!(parsed.join_clauses[0].variable, "orders");
    assert!(matches!(parsed.join_clauses[0].join_type, solidb::sdbql::ast::JoinType::Right));
    
    Ok(())
}

#[test]
fn test_parse_full_outer_join() -> DbResult<()> {
    let query = "FOR user IN users FULL OUTER JOIN orders ON user._key == orders.user_key RETURN {user, orders}";
    let parsed = parse(query)?;
    
    assert_eq!(parsed.join_clauses.len(), 1);
    assert_eq!(parsed.join_clauses[0].variable, "orders");
    assert!(matches!(parsed.join_clauses[0].join_type, solidb::sdbql::ast::JoinType::FullOuter));
    
    Ok(())
}

#[test]
fn test_parse_full_join_without_outer() -> DbResult<()> {
    let query = "FOR user IN users FULL JOIN orders ON user._key == orders.user_key RETURN {user, orders}";
    let parsed = parse(query)?;
    
    assert_eq!(parsed.join_clauses.len(), 1);
    assert!(matches!(parsed.join_clauses[0].join_type, solidb::sdbql::ast::JoinType::FullOuter));
    
    Ok(())
}

#[test]
fn test_execute_right_join() -> DbResult<()> {
    let (storage, db_name) = setup_test_data()?;
    let executor = QueryExecutor::with_database(&storage, db_name);
    
    let query_str = r#"
        FOR user IN users
          RIGHT JOIN orders ON user._key == orders.user_key
          RETURN {order_key: orders._key, user_name: user.name}
    "#;
    
    let query = parse(query_str)?;
    let results = executor.execute(&query)?;
    
    // Should have 3 results (all orders: o1, o2, o3)
    assert_eq!(results.len(), 3);
    
    // All results should have order_key populated
    for result in &results {
        assert!(result.get("order_key").is_some());
    }
    
    Ok(())
}

#[test]
fn test_execute_full_outer_join() -> DbResult<()> {
    let (storage, db_name) = setup_test_data()?;
    let executor = QueryExecutor::with_database(&storage, db_name);
    
    let query_str = r#"
        FOR user IN users
          FULL OUTER JOIN orders ON user._key == orders.user_key
          RETURN {user_name: user.name, orders: orders}
    "#;
    
    let query = parse(query_str)?;
    let results = executor.execute(&query)?;
    
    // Should have at least 3 results (3 users)
    // Users with orders: Alice (2 orders), Bob (1 order) = 2 rows
    // User without orders: Charlie = 1 row
    // Total from LEFT part = 3 rows
    // No unmatched orders (all 3 orders have matching users)
    // FULL OUTER = 3 rows
    assert_eq!(results.len(), 3);
    
    // Charlie should have empty orders array
    let charlie = results.iter().find(|r| {
        r.get("user_name").and_then(|v| v.as_str()) == Some("Charlie")
    });
    assert!(charlie.is_some());
    let charlie_orders = charlie.unwrap().get("orders").unwrap().as_array().unwrap();
    assert_eq!(charlie_orders.len(), 0);
    
    Ok(())
}

#[test]
fn test_right_join_with_no_left_matches() -> DbResult<()> {
    let storage = StorageEngine::new("/tmp/test_right_join_db")?;
    
    let db_name = format!("right_test_{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis());
    
    storage.create_database(db_name.clone())?;
    storage.create_collection(format!("{}:users", db_name), None)?;
    storage.create_collection(format!("{}:orders", db_name), None)?;
    
    // Insert only orders, no users
    let orders_coll = storage.get_collection(&format!("{}:orders", db_name))?;
    orders_coll.insert(json!({"_key": "o1", "user_key": "nonexistent", "total": 100}))?;
    orders_coll.insert(json!({"_key": "o2", "user_key": "nonexistent", "total": 200}))?;
    
    let executor = QueryExecutor::with_database(&storage, db_name);
    
    let query_str = "FOR user IN users RIGHT JOIN orders ON user._key == orders.user_key RETURN {order_key: orders._key}";
    let query = parse(query_str)?;
    let results = executor.execute(&query)?;
    
    // Should have 2 results (all orders, even without matching users)
    assert_eq!(results.len(), 2);
    
    Ok(())
}

#[test]
fn test_full_outer_join_comprehensive() -> DbResult<()> {
    let storage = StorageEngine::new("/tmp/test_full_outer_db")?;
    
    let db_name = format!("full_test_{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis());
    
    storage.create_database(db_name.clone())?;
    storage.create_collection(format!("{}:users", db_name), None)?;
    storage.create_collection(format!("{}:orders", db_name), None)?;
    
    // Insert users
    let users_coll = storage.get_collection(&format!("{}:users", db_name))?;
    users_coll.insert(json!({"_key": "u1", "name": "Alice"}))?;
    users_coll.insert(json!({"_key": "u2", "name": "Bob"}))?;
    users_coll.insert(json!({"_key": "u3", "name": "Charlie"}))?; // No orders
    
    // Insert orders  
    let orders_coll = storage.get_collection(&format!("{}:orders", db_name))?;
    orders_coll.insert(json!({"_key": "o1", "user_key": "u1", "total": 100}))?;
    orders_coll.insert(json!({"_key": "o2", "user_key": "u1", "total": 200}))?;
    orders_coll.insert(json!({"_key": "o3", "user_key": "orphan", "total": 150}))?; // No matching user
    
    let executor = QueryExecutor::with_database(&storage, db_name);
    
    let query_str = r#"
        FOR user IN users
          FULL OUTER JOIN orders ON user._key == orders.user_key
          RETURN {
            user_name: IS_NULL(user.name) ? null : user.name, 
            has_orders: LENGTH(orders) > 0
          }
    "#;
    
    let query = parse(query_str)?;
    let results = executor.execute(&query)?;
    
    // Should have 4 results:
    // - Alice (has orders)
    // - Bob (no orders)  
    // - Charlie (no orders)
    // - Orphan order o3 (no user)
    assert_eq!(results.len(), 4);
    
    // Count users with orders
    let with_orders = results.iter().filter(|r| {
        r.get("has_orders").and_then(|v| v.as_bool()) == Some(true)
    }).count();
    assert_eq!(with_orders, 1); // Only Alice
    
    // Count rows without user_name (orphan orders)
    let orphan_orders = results.iter().filter(|r| {
        r.get("user_name").is_none() || r.get("user_name").unwrap().is_null()
    }).count();
    assert_eq!(orphan_orders, 1); // Orphan order o3
    
    Ok(())
}
