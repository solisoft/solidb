//! SDBQL Advanced Query Tests
//!
//! Tests for advanced SDBQL query features including:
//! - Complex aggregations
//! - Subqueries
//! - Graph traversals with depth
//! - Multi-collection queries
//! - Edge cases

mod common;
use common::{create_test_engine, execute_query};
use serde_json::json;

// ============================================================================
// Complex Filter Tests
// ============================================================================

#[test]
fn test_filter_with_multiple_conditions() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let products = engine.get_collection("products").unwrap();

    products
        .insert(json!({"_key": "p1", "name": "Widget", "price": 10, "category": "tools"}))
        .unwrap();
    products
        .insert(json!({"_key": "p2", "name": "Gadget", "price": 50, "category": "electronics"}))
        .unwrap();
    products
        .insert(json!({"_key": "p3", "name": "Gizmo", "price": 30, "category": "tools"}))
        .unwrap();
    products
        .insert(json!({"_key": "p4", "name": "Device", "price": 100, "category": "electronics"}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR p IN products FILTER p.category == 'tools' AND p.price > 5 RETURN p.name",
    );

    assert_eq!(results.len(), 2);
}

#[test]
fn test_filter_with_or_condition() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();

    items
        .insert(json!({"_key": "i1", "status": "active"}))
        .unwrap();
    items
        .insert(json!({"_key": "i2", "status": "pending"}))
        .unwrap();
    items
        .insert(json!({"_key": "i3", "status": "inactive"}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR i IN items FILTER i.status == 'active' OR i.status == 'pending' RETURN i._key",
    );

    assert_eq!(results.len(), 2);
}

#[test]
fn test_filter_with_not() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    users
        .insert(json!({"_key": "u1", "name": "Alice", "admin": true}))
        .unwrap();
    users
        .insert(json!({"_key": "u2", "name": "Bob", "admin": false}))
        .unwrap();
    users
        .insert(json!({"_key": "u3", "name": "Charlie", "admin": false}))
        .unwrap();

    let results = execute_query(&engine, "FOR u IN users FILTER NOT u.admin RETURN u.name");

    assert_eq!(results.len(), 2);
}

#[test]
fn test_filter_with_in_array() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("orders".to_string(), None)
        .unwrap();
    let orders = engine.get_collection("orders").unwrap();

    orders
        .insert(json!({"_key": "o1", "status": "pending"}))
        .unwrap();
    orders
        .insert(json!({"_key": "o2", "status": "shipped"}))
        .unwrap();
    orders
        .insert(json!({"_key": "o3", "status": "delivered"}))
        .unwrap();
    orders
        .insert(json!({"_key": "o4", "status": "cancelled"}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR o IN orders FILTER o.status IN ['pending', 'shipped'] RETURN o._key",
    );

    assert_eq!(results.len(), 2);
}

// ============================================================================
// Sorting Tests
// ============================================================================

#[test]
fn test_sort_ascending() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("nums".to_string(), None).unwrap();
    let nums = engine.get_collection("nums").unwrap();

    nums.insert(json!({"_key": "n3", "value": 3})).unwrap();
    nums.insert(json!({"_key": "n1", "value": 1})).unwrap();
    nums.insert(json!({"_key": "n2", "value": 2})).unwrap();

    let results = execute_query(&engine, "FOR n IN nums SORT n.value RETURN n.value");

    assert_eq!(results, vec![json!(1), json!(2), json!(3)]);
}

#[test]
fn test_sort_descending() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("nums".to_string(), None).unwrap();
    let nums = engine.get_collection("nums").unwrap();

    nums.insert(json!({"_key": "n1", "value": 1})).unwrap();
    nums.insert(json!({"_key": "n2", "value": 2})).unwrap();
    nums.insert(json!({"_key": "n3", "value": 3})).unwrap();

    let results = execute_query(&engine, "FOR n IN nums SORT n.value DESC RETURN n.value");

    assert_eq!(results, vec![json!(3), json!(2), json!(1)]);
}

#[test]
fn test_sort_multiple_fields() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("people".to_string(), None)
        .unwrap();
    let people = engine.get_collection("people").unwrap();

    people
        .insert(json!({"_key": "p1", "last_name": "Smith", "first_name": "Bob"}))
        .unwrap();
    people
        .insert(json!({"_key": "p2", "last_name": "Jones", "first_name": "Alice"}))
        .unwrap();
    people
        .insert(json!({"_key": "p3", "last_name": "Smith", "first_name": "Alice"}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR p IN people SORT p.last_name, p.first_name RETURN p.first_name",
    );

    assert_eq!(results.len(), 3);
    // Jones comes before Smith, within Smith, Alice comes before Bob
    assert_eq!(results[0], json!("Alice")); // Jones, Alice
    assert_eq!(results[1], json!("Alice")); // Smith, Alice
    assert_eq!(results[2], json!("Bob")); // Smith, Bob
}

// ============================================================================
// LET Binding Tests
// ============================================================================

#[test]
fn test_let_binding_simple() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();

    data.insert(json!({"_key": "d1", "value": 10})).unwrap();

    let results = execute_query(
        &engine,
        "FOR d IN data LET doubled = d.value * 2 RETURN doubled",
    );

    assert_eq!(results[0].as_f64().unwrap(), 20.0);
}

#[test]
fn test_let_binding_with_function() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("words".to_string(), None).unwrap();
    let words = engine.get_collection("words").unwrap();

    words
        .insert(json!({"_key": "w1", "text": "Hello World"}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR w IN words LET upper = UPPER(w.text) RETURN upper",
    );

    assert_eq!(results, vec![json!("HELLO WORLD")]);
}

#[test]
fn test_multiple_let_bindings() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();

    items
        .insert(json!({"_key": "i1", "price": 100, "quantity": 5}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR i IN items LET subtotal = i.price * i.quantity LET tax = subtotal * 0.1 RETURN subtotal + tax",
    );

    assert_eq!(results, vec![json!(550.0)]);
}

// ============================================================================
// Object Return Tests
// ============================================================================

#[test]
fn test_return_object_literal() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    users
        .insert(json!({"_key": "u1", "name": "Alice", "age": 30}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR u IN users RETURN { name: u.name, age: u.age }",
    );

    assert_eq!(results[0]["name"], json!("Alice"));
    assert_eq!(results[0]["age"], json!(30));
}

#[test]
fn test_return_computed_field() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let products = engine.get_collection("products").unwrap();

    products
        .insert(json!({"_key": "p1", "name": "Widget", "price": 10, "tax_rate": 0.1}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR p IN products RETURN { name: p.name, total: p.price * (1 + p.tax_rate) }",
    );

    assert_eq!(results[0]["name"], json!("Widget"));
    // 10 * 1.1 = 11.0
    let total = results[0]["total"].as_f64().unwrap();
    assert!((total - 11.0).abs() < 0.001);
}

// ============================================================================
// Array Function Tests
// ============================================================================

#[test]
fn test_array_length() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();

    items
        .insert(json!({"_key": "i1", "tags": ["a", "b", "c"]}))
        .unwrap();

    let results = execute_query(&engine, "FOR i IN items RETURN LENGTH(i.tags)");

    assert_eq!(results, vec![json!(3)]);
}

#[test]
fn test_array_push() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();

    items.insert(json!({"_key": "i1", "nums": [1, 2]})).unwrap();

    let results = execute_query(&engine, "FOR i IN items RETURN PUSH(i.nums, 3)");

    assert_eq!(results[0].as_array().unwrap().len(), 3);
}

#[test]
fn test_array_first_last() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();

    items
        .insert(json!({"_key": "i1", "nums": [10, 20, 30]}))
        .unwrap();

    let first = execute_query(&engine, "FOR i IN items RETURN FIRST(i.nums)");
    let last = execute_query(&engine, "FOR i IN items RETURN LAST(i.nums)");

    assert_eq!(first, vec![json!(10)]);
    assert_eq!(last, vec![json!(30)]);
}

#[test]
fn test_array_slice() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();

    items
        .insert(json!({"_key": "i1", "nums": [1, 2, 3, 4, 5]}))
        .unwrap();

    let results = execute_query(&engine, "FOR i IN items RETURN SLICE(i.nums, 1, 3)");

    assert_eq!(results[0], json!([2, 3, 4]));
}

// ============================================================================
// String Function Tests
// ============================================================================

#[test]
fn test_string_concat() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    users
        .insert(json!({"_key": "u1", "first": "John", "last": "Doe"}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR u IN users RETURN CONCAT(u.first, ' ', u.last)",
    );

    assert_eq!(results, vec![json!("John Doe")]);
}

#[test]
fn test_string_substring() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();

    data.insert(json!({"_key": "d1", "text": "Hello World"}))
        .unwrap();

    let results = execute_query(&engine, "FOR d IN data RETURN SUBSTRING(d.text, 0, 5)");

    assert_eq!(results, vec![json!("Hello")]);
}

#[test]
fn test_string_split() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();

    data.insert(json!({"_key": "d1", "csv": "a,b,c"})).unwrap();

    let results = execute_query(&engine, "FOR d IN data RETURN SPLIT(d.csv, ',')");

    assert_eq!(results[0], json!(["a", "b", "c"]));
}

#[test]
fn test_string_trim() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();

    data.insert(json!({"_key": "d1", "text": "  hello  "}))
        .unwrap();

    let results = execute_query(&engine, "FOR d IN data RETURN TRIM(d.text)");

    assert_eq!(results, vec![json!("hello")]);
}

// ============================================================================
// Numeric Function Tests
// ============================================================================

#[test]
fn test_math_functions() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("nums".to_string(), None).unwrap();
    let nums = engine.get_collection("nums").unwrap();

    nums.insert(json!({"_key": "n1", "value": 16})).unwrap();

    let sqrt = execute_query(&engine, "FOR n IN nums RETURN SQRT(n.value)");
    let abs = execute_query(&engine, "FOR n IN nums RETURN ABS(-5)");
    let floor = execute_query(&engine, "FOR n IN nums RETURN FLOOR(3.7)");
    let ceil = execute_query(&engine, "FOR n IN nums RETURN CEIL(3.2)");
    let round = execute_query(&engine, "FOR n IN nums RETURN ROUND(3.5)");

    assert_eq!(sqrt[0].as_f64().unwrap(), 4.0);
    assert_eq!(abs[0].as_f64().unwrap(), 5.0);
    assert_eq!(floor[0].as_f64().unwrap(), 3.0);
    assert_eq!(ceil[0].as_f64().unwrap(), 4.0);
    assert_eq!(round[0].as_f64().unwrap(), 4.0);
}

#[test]
fn test_min_max() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("nums".to_string(), None).unwrap();
    let nums = engine.get_collection("nums").unwrap();

    nums.insert(json!({"_key": "n1", "values": [5, 2, 8, 1, 9]}))
        .unwrap();

    let min = execute_query(&engine, "FOR n IN nums RETURN MIN(n.values)");
    let max = execute_query(&engine, "FOR n IN nums RETURN MAX(n.values)");

    assert_eq!(min[0].as_f64().unwrap(), 1.0);
    assert_eq!(max[0].as_f64().unwrap(), 9.0);
}

// ============================================================================
// Type Checking Function Tests
// ============================================================================

#[test]
fn test_type_checks() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("mixed".to_string(), None).unwrap();
    let mixed = engine.get_collection("mixed").unwrap();

    mixed.insert(json!({"_key": "m1", "str": "hello", "num": 42, "bool": true, "nothing": null, "arr": [1,2], "obj": {"a": 1}})).unwrap();

    let is_string = execute_query(&engine, "FOR m IN mixed RETURN IS_STRING(m.str)");
    let is_number = execute_query(&engine, "FOR m IN mixed RETURN IS_NUMBER(m.num)");
    let is_bool = execute_query(&engine, "FOR m IN mixed RETURN IS_BOOL(m.bool)");
    let is_null = execute_query(&engine, "FOR m IN mixed RETURN IS_NULL(m.nothing)");
    let is_array = execute_query(&engine, "FOR m IN mixed RETURN IS_ARRAY(m.arr)");
    let is_object = execute_query(&engine, "FOR m IN mixed RETURN IS_OBJECT(m.obj)");

    assert_eq!(is_string, vec![json!(true)]);
    assert_eq!(is_number, vec![json!(true)]);
    assert_eq!(is_bool, vec![json!(true)]);
    assert_eq!(is_null, vec![json!(true)]);
    assert_eq!(is_array, vec![json!(true)]);
    assert_eq!(is_object, vec![json!(true)]);
}

// ============================================================================
// Conditional Function Tests
// ============================================================================

#[test]
fn test_ternary_expression() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    users.insert(json!({"_key": "u1", "age": 20})).unwrap();
    users.insert(json!({"_key": "u2", "age": 15})).unwrap();

    let results = execute_query(
        &engine,
        "FOR u IN users SORT u._key RETURN u.age >= 18 ? 'adult' : 'minor'",
    );

    assert_eq!(results, vec![json!("adult"), json!("minor")]);
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_empty_collection() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("empty".to_string(), None).unwrap();

    let results = execute_query(&engine, "FOR x IN empty RETURN x");
    assert!(results.is_empty());
}

#[test]
fn test_null_field_access() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("partial".to_string(), None)
        .unwrap();
    let partial = engine.get_collection("partial").unwrap();

    partial
        .insert(json!({"_key": "p1", "name": "Alice"}))
        .unwrap();

    let results = execute_query(&engine, "FOR p IN partial RETURN p.nonexistent");
    assert_eq!(results, vec![json!(null)]);
}

#[test]
fn test_nested_field_access() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("nested".to_string(), None)
        .unwrap();
    let nested = engine.get_collection("nested").unwrap();

    nested
        .insert(json!({"_key": "n1", "user": {"profile": {"name": "Alice"}}}))
        .unwrap();

    let results = execute_query(&engine, "FOR n IN nested RETURN n.user.profile.name");
    assert_eq!(results, vec![json!("Alice")]);
}

#[test]
fn test_array_index_access() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("arrays".to_string(), None)
        .unwrap();
    let arrays = engine.get_collection("arrays").unwrap();

    arrays
        .insert(json!({"_key": "a1", "items": ["first", "second", "third"]}))
        .unwrap();

    let results = execute_query(&engine, "FOR a IN arrays RETURN a.items[1]");
    assert_eq!(results, vec![json!("second")]);
}

// ============================================================================
// Array Spread Access Tests [*]
// ============================================================================

#[test]
fn test_array_spread_basic() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("events".to_string(), None)
        .unwrap();
    let events = engine.get_collection("events").unwrap();

    events
        .insert(json!({
            "_key": "e1",
            "attendees": [
                {"user_key": "u1", "name": "Alice"},
                {"user_key": "u2", "name": "Bob"}
            ]
        }))
        .unwrap();

    let results = execute_query(&engine, "FOR e IN events RETURN e.attendees[*].user_key");
    assert_eq!(results, vec![json!(["u1", "u2"])]);
}

#[test]
fn test_array_spread_nested_field() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();

    data.insert(json!({
        "_key": "d1",
        "items": [
            {"user": {"name": "Alice"}},
            {"user": {"name": "Bob"}}
        ]
    }))
    .unwrap();

    let results = execute_query(&engine, "FOR d IN data RETURN d.items[*].user.name");
    assert_eq!(results, vec![json!(["Alice", "Bob"])]);
}

#[test]
fn test_array_spread_empty_array() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("empty".to_string(), None).unwrap();
    let empty = engine.get_collection("empty").unwrap();

    empty.insert(json!({"_key": "e1", "items": []})).unwrap();

    let results = execute_query(&engine, "FOR e IN empty RETURN e.items[*].name");
    assert_eq!(results, vec![json!([])]);
}

#[test]
fn test_array_spread_non_array() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("single".to_string(), None)
        .unwrap();
    let single = engine.get_collection("single").unwrap();

    single
        .insert(json!({"_key": "s1", "value": "not an array"}))
        .unwrap();

    let results = execute_query(&engine, "FOR s IN single RETURN s.value[*].field");
    assert_eq!(results, vec![json!([])]);
}

#[test]
fn test_array_spread_nested_spread() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("nested".to_string(), None)
        .unwrap();
    let nested = engine.get_collection("nested").unwrap();

    nested
        .insert(json!({
            "_key": "n1",
            "items": [
                {"tags": ["a", "b"]},
                {"tags": ["c"]}
            ]
        }))
        .unwrap();

    let results = execute_query(&engine, "FOR n IN nested RETURN n.items[*].tags[*]");
    assert_eq!(results, vec![json!(["a", "b", "c"])]);
}

#[test]
fn test_array_spread_missing_field() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("partial".to_string(), None)
        .unwrap();
    let partial = engine.get_collection("partial").unwrap();

    partial
        .insert(json!({
            "_key": "p1",
            "items": [
                {"name": "Alice"},
                {"other": "Bob"}
            ]
        }))
        .unwrap();

    let results = execute_query(&engine, "FOR p IN partial RETURN p.items[*].name");
    assert_eq!(results, vec![json!(["Alice", null])]);
}

#[test]
fn test_array_spread_bare() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("bare".to_string(), None).unwrap();
    let bare = engine.get_collection("bare").unwrap();

    bare.insert(json!({"_key": "b1", "values": [1, 2, 3]}))
        .unwrap();

    let results = execute_query(&engine, "FOR b IN bare RETURN b.values[*]");
    assert_eq!(results, vec![json!([1, 2, 3])]);
}
