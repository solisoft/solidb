//! CASE/WHEN Expression Tests for SDBQL
//!
//! Tests for SQL-style CASE expressions:
//! - Simple CASE: CASE expr WHEN val1 THEN res1 ... END
//! - Searched CASE: CASE WHEN cond1 THEN res1 ... END

use serde_json::json;
use solidb::parse;
use solidb::sdbql::QueryExecutor;
use solidb::storage::StorageEngine;
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine =
        StorageEngine::new(tmp_dir.path().to_str().unwrap()).expect("Failed to create storage");
    (engine, tmp_dir)
}

fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect("Failed to parse query");
    let executor = QueryExecutor::new(engine);
    executor.execute(&query).expect("Failed to execute query")
}

fn setup_test_data(engine: &StorageEngine) {
    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let products = engine.get_collection("products").unwrap();

    products
        .insert(json!({"_key": "p1", "name": "Laptop", "category": "electronics", "price": 999, "stock": 50}))
        .unwrap();
    products
        .insert(
            json!({"_key": "p2", "name": "Book", "category": "books", "price": 25, "stock": 200}),
        )
        .unwrap();
    products
        .insert(json!({"_key": "p3", "name": "Headphones", "category": "electronics", "price": 150, "stock": 0}))
        .unwrap();
    products
        .insert(json!({"_key": "p4", "name": "Shirt", "category": "clothing", "price": 45, "stock": 75}))
        .unwrap();
    products
        .insert(json!({"_key": "p5", "name": "Coffee Maker", "category": "appliances", "price": 80, "stock": 30}))
        .unwrap();
}

// ============================================================================
// Simple CASE Tests
// ============================================================================

#[test]
fn test_simple_case_basic() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN products
        RETURN {
            name: doc.name,
            category_label: CASE doc.category
                WHEN "electronics" THEN "Tech"
                WHEN "books" THEN "Reading"
                WHEN "clothing" THEN "Fashion"
                ELSE "Other"
            END
        }
    "#,
    );

    assert_eq!(results.len(), 5);

    let laptop = results
        .iter()
        .find(|r| r["name"] == json!("Laptop"))
        .unwrap();
    assert_eq!(laptop["category_label"], json!("Tech"));

    let book = results.iter().find(|r| r["name"] == json!("Book")).unwrap();
    assert_eq!(book["category_label"], json!("Reading"));

    let shirt = results
        .iter()
        .find(|r| r["name"] == json!("Shirt"))
        .unwrap();
    assert_eq!(shirt["category_label"], json!("Fashion"));

    let coffee = results
        .iter()
        .find(|r| r["name"] == json!("Coffee Maker"))
        .unwrap();
    assert_eq!(coffee["category_label"], json!("Other"));
}

#[test]
fn test_simple_case_with_numbers() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("grades".to_string(), None)
        .unwrap();
    let grades = engine.get_collection("grades").unwrap();

    grades.insert(json!({"_key": "1", "score": 95})).unwrap();
    grades.insert(json!({"_key": "2", "score": 85})).unwrap();
    grades.insert(json!({"_key": "3", "score": 75})).unwrap();

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN grades
        RETURN {
            score: doc.score,
            grade: CASE doc.score
                WHEN 95 THEN "A+"
                WHEN 85 THEN "B+"
                WHEN 75 THEN "C+"
                ELSE "?"
            END
        }
    "#,
    );

    assert_eq!(results.len(), 3);
    let a_plus = results.iter().find(|r| r["score"] == json!(95)).unwrap();
    assert_eq!(a_plus["grade"], json!("A+"));
}

#[test]
fn test_simple_case_no_match_no_else() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN products
        FILTER doc.category == "appliances"
        RETURN {
            name: doc.name,
            label: CASE doc.category
                WHEN "electronics" THEN "Tech"
                WHEN "books" THEN "Reading"
            END
        }
    "#,
    );

    assert_eq!(results.len(), 1);
    // No ELSE, no match -> null
    assert!(results[0]["label"].is_null());
}

// ============================================================================
// Searched CASE Tests
// ============================================================================

#[test]
fn test_searched_case_basic() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN products
        RETURN {
            name: doc.name,
            price_tier: CASE
                WHEN doc.price >= 500 THEN "premium"
                WHEN doc.price >= 100 THEN "mid-range"
                WHEN doc.price >= 50 THEN "budget"
                ELSE "bargain"
            END
        }
    "#,
    );

    assert_eq!(results.len(), 5);

    let laptop = results
        .iter()
        .find(|r| r["name"] == json!("Laptop"))
        .unwrap();
    assert_eq!(laptop["price_tier"], json!("premium"));

    let headphones = results
        .iter()
        .find(|r| r["name"] == json!("Headphones"))
        .unwrap();
    assert_eq!(headphones["price_tier"], json!("mid-range"));

    let coffee = results
        .iter()
        .find(|r| r["name"] == json!("Coffee Maker"))
        .unwrap();
    assert_eq!(coffee["price_tier"], json!("budget")); // $80 is >= 50

    let shirt = results
        .iter()
        .find(|r| r["name"] == json!("Shirt"))
        .unwrap();
    assert_eq!(shirt["price_tier"], json!("bargain")); // $45 is < 50

    let book = results.iter().find(|r| r["name"] == json!("Book")).unwrap();
    assert_eq!(book["price_tier"], json!("bargain")); // $25 is < 50
}

#[test]
fn test_searched_case_stock_status() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN products
        RETURN {
            name: doc.name,
            availability: CASE
                WHEN doc.stock == 0 THEN "Out of Stock"
                WHEN doc.stock < 50 THEN "Low Stock"
                WHEN doc.stock < 100 THEN "In Stock"
                ELSE "Well Stocked"
            END
        }
    "#,
    );

    assert_eq!(results.len(), 5);

    let headphones = results
        .iter()
        .find(|r| r["name"] == json!("Headphones"))
        .unwrap();
    assert_eq!(headphones["availability"], json!("Out of Stock"));

    let coffee = results
        .iter()
        .find(|r| r["name"] == json!("Coffee Maker"))
        .unwrap();
    assert_eq!(coffee["availability"], json!("Low Stock"));

    let laptop = results
        .iter()
        .find(|r| r["name"] == json!("Laptop"))
        .unwrap();
    assert_eq!(laptop["availability"], json!("In Stock"));

    let book = results.iter().find(|r| r["name"] == json!("Book")).unwrap();
    assert_eq!(book["availability"], json!("Well Stocked"));
}

#[test]
fn test_searched_case_multiple_conditions() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN products
        RETURN {
            name: doc.name,
            deal: CASE
                WHEN doc.category == "electronics" AND doc.price < 200 THEN "Tech Deal"
                WHEN doc.stock > 100 THEN "Bulk Available"
                ELSE "Regular"
            END
        }
    "#,
    );

    assert_eq!(results.len(), 5);

    let headphones = results
        .iter()
        .find(|r| r["name"] == json!("Headphones"))
        .unwrap();
    assert_eq!(headphones["deal"], json!("Tech Deal"));

    let book = results.iter().find(|r| r["name"] == json!("Book")).unwrap();
    assert_eq!(book["deal"], json!("Bulk Available"));

    let laptop = results
        .iter()
        .find(|r| r["name"] == json!("Laptop"))
        .unwrap();
    assert_eq!(laptop["deal"], json!("Regular"));
}

// ============================================================================
// CASE in Different Contexts
// ============================================================================

#[test]
fn test_case_in_filter() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN products
        LET tier = CASE
            WHEN doc.price >= 100 THEN "high"
            ELSE "low"
        END
        FILTER tier == "high"
        RETURN doc.name
    "#,
    );

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Laptop")));
    assert!(results.contains(&json!("Headphones")));
}

#[test]
fn test_case_in_sort() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();

    items
        .insert(json!({"_key": "1", "priority": "high"}))
        .unwrap();
    items
        .insert(json!({"_key": "2", "priority": "low"}))
        .unwrap();
    items
        .insert(json!({"_key": "3", "priority": "medium"}))
        .unwrap();

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN items
        LET sort_order = CASE doc.priority
            WHEN "high" THEN 1
            WHEN "medium" THEN 2
            WHEN "low" THEN 3
            ELSE 4
        END
        SORT sort_order
        RETURN doc.priority
    "#,
    );

    assert_eq!(results, vec![json!("high"), json!("medium"), json!("low")]);
}

#[test]
fn test_case_nested() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN products
        RETURN {
            name: doc.name,
            status: CASE doc.category
                WHEN "electronics" THEN CASE
                    WHEN doc.stock == 0 THEN "Electronics - Unavailable"
                    ELSE "Electronics - Available"
                END
                ELSE "Other Product"
            END
        }
    "#,
    );

    assert_eq!(results.len(), 5);

    let laptop = results
        .iter()
        .find(|r| r["name"] == json!("Laptop"))
        .unwrap();
    assert_eq!(laptop["status"], json!("Electronics - Available"));

    let headphones = results
        .iter()
        .find(|r| r["name"] == json!("Headphones"))
        .unwrap();
    assert_eq!(headphones["status"], json!("Electronics - Unavailable"));

    let book = results.iter().find(|r| r["name"] == json!("Book")).unwrap();
    assert_eq!(book["status"], json!("Other Product"));
}

// ============================================================================
// CASE with Other Features
// ============================================================================

#[test]
fn test_case_with_functions() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN products
        RETURN {
            name: doc.name,
            name_length_category: CASE
                WHEN LENGTH(doc.name) > 10 THEN "long"
                WHEN LENGTH(doc.name) > 5 THEN "medium"
                ELSE "short"
            END
        }
    "#,
    );

    assert_eq!(results.len(), 5);

    let coffee = results
        .iter()
        .find(|r| r["name"] == json!("Coffee Maker"))
        .unwrap();
    assert_eq!(coffee["name_length_category"], json!("long"));

    let laptop = results
        .iter()
        .find(|r| r["name"] == json!("Laptop"))
        .unwrap();
    assert_eq!(laptop["name_length_category"], json!("medium"));

    let book = results.iter().find(|r| r["name"] == json!("Book")).unwrap();
    assert_eq!(book["name_length_category"], json!("short"));
}

#[test]
fn test_case_with_null_coalescing() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    users
        .insert(json!({"_key": "1", "name": "Alice", "role": "admin"}))
        .unwrap();
    users
        .insert(json!({"_key": "2", "name": "Bob", "role": null}))
        .unwrap();
    users
        .insert(json!({"_key": "3", "name": "Charlie"}))
        .unwrap();

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        RETURN {
            name: doc.name,
            access: CASE doc.role ?? "guest"
                WHEN "admin" THEN "full"
                WHEN "user" THEN "limited"
                ELSE "read-only"
            END
        }
    "#,
    );

    assert_eq!(results.len(), 3);

    let alice = results
        .iter()
        .find(|r| r["name"] == json!("Alice"))
        .unwrap();
    assert_eq!(alice["access"], json!("full"));

    let bob = results.iter().find(|r| r["name"] == json!("Bob")).unwrap();
    assert_eq!(bob["access"], json!("read-only"));

    let charlie = results
        .iter()
        .find(|r| r["name"] == json!("Charlie"))
        .unwrap();
    assert_eq!(charlie["access"], json!("read-only"));
}

#[test]
fn test_case_with_optional_chaining() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("orders".to_string(), None)
        .unwrap();
    let orders = engine.get_collection("orders").unwrap();

    orders
        .insert(json!({"_key": "1", "customer": {"tier": "gold"}}))
        .unwrap();
    orders
        .insert(json!({"_key": "2", "customer": {"tier": "silver"}}))
        .unwrap();
    orders
        .insert(json!({"_key": "3", "customer": null}))
        .unwrap();

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN orders
        RETURN {
            key: doc._key,
            discount: CASE doc.customer?.tier
                WHEN "gold" THEN 20
                WHEN "silver" THEN 10
                ELSE 0
            END
        }
    "#,
    );

    assert_eq!(results.len(), 3);

    let gold = results.iter().find(|r| r["key"] == json!("1")).unwrap();
    assert_eq!(gold["discount"], json!(20));

    let silver = results.iter().find(|r| r["key"] == json!("2")).unwrap();
    assert_eq!(silver["discount"], json!(10));

    let none = results.iter().find(|r| r["key"] == json!("3")).unwrap();
    assert_eq!(none["discount"], json!(0));
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_case_single_when() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN products
        FILTER doc._key == "p1"
        RETURN CASE
            WHEN doc.price > 500 THEN "expensive"
            ELSE "affordable"
        END
    "#,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("expensive"));
}

#[test]
fn test_case_with_boolean_result() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN products
        RETURN {
            name: doc.name,
            is_premium: CASE
                WHEN doc.price >= 500 THEN true
                ELSE false
            END
        }
    "#,
    );

    assert_eq!(results.len(), 5);

    let laptop = results
        .iter()
        .find(|r| r["name"] == json!("Laptop"))
        .unwrap();
    assert_eq!(laptop["is_premium"], json!(true));

    let book = results.iter().find(|r| r["name"] == json!("Book")).unwrap();
    assert_eq!(book["is_premium"], json!(false));
}

#[test]
fn test_case_returns_object() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN products
        FILTER doc._key == "p1"
        RETURN CASE doc.category
            WHEN "electronics" THEN { type: "tech", icon: "laptop" }
            ELSE { type: "other", icon: "box" }
        END
    "#,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["type"], json!("tech"));
    assert_eq!(results[0]["icon"], json!("laptop"));
}
