//! SDBQL Parser Unit Tests
//!
//! Tests for the SDBQL query language parser, covering:
//! - Basic query parsing (FOR, RETURN)
//! - Filtering (FILTER)
//! - Sorting (SORT)
//! - Limiting (LIMIT)
//! - Let bindings (LET)
//! - CRUD operations (INSERT, UPDATE, REMOVE)
//! - Graph traversals
//! - Expression parsing

use solidb::parse;

// ============================================================================
// Basic Query Parsing
// ============================================================================

#[test]
fn test_simple_for_return() {
    let query = "FOR doc IN users RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse simple FOR...RETURN: {:?}", result.err());
}

#[test]
fn test_for_with_filter() {
    let query = "FOR doc IN users FILTER doc.age > 18 RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FOR with FILTER: {:?}", result.err());
}

#[test]
fn test_for_with_sort() {
    let query = "FOR doc IN users SORT doc.name RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FOR with SORT: {:?}", result.err());
}

#[test]
fn test_for_with_sort_desc() {
    let query = "FOR doc IN users SORT doc.name DESC RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FOR with SORT DESC: {:?}", result.err());
}

#[test]
fn test_for_with_limit() {
    let query = "FOR doc IN users LIMIT 10 RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FOR with LIMIT: {:?}", result.err());
}

#[test]
fn test_for_with_limit_offset() {
    let query = "FOR doc IN users LIMIT 5, 10 RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FOR with LIMIT offset: {:?}", result.err());
}

// ============================================================================
// Let Bindings
// ============================================================================

#[test]
fn test_let_binding() {
    let query = "LET x = 42 RETURN x";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse LET binding: {:?}", result.err());
}

#[test]
fn test_let_with_expression() {
    let query = "LET x = 10 + 20 RETURN x";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse LET with expression: {:?}", result.err());
}

#[test]
fn test_multiple_let_bindings() {
    let query = "LET a = 1 LET b = 2 RETURN a + b";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse multiple LET bindings: {:?}", result.err());
}

// ============================================================================
// Filter Expressions
// ============================================================================

#[test]
fn test_filter_equals() {
    let query = "FOR doc IN users FILTER doc.name == 'Alice' RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FILTER equals: {:?}", result.err());
}

#[test]
fn test_filter_not_equals() {
    let query = "FOR doc IN users FILTER doc.name != 'Bob' RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FILTER not equals: {:?}", result.err());
}

#[test]
fn test_filter_comparison_operators() {
    let queries = [
        "FOR doc IN users FILTER doc.age > 18 RETURN doc",
        "FOR doc IN users FILTER doc.age >= 18 RETURN doc",
        "FOR doc IN users FILTER doc.age < 65 RETURN doc",
        "FOR doc IN users FILTER doc.age <= 65 RETURN doc",
    ];
    
    for query in queries {
        let result = parse(query);
        assert!(result.is_ok(), "Failed to parse: {} - {:?}", query, result.err());
    }
}

#[test]
fn test_filter_and() {
    let query = "FOR doc IN users FILTER doc.age > 18 AND doc.active == true RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FILTER AND: {:?}", result.err());
}

#[test]
fn test_filter_or() {
    let query = "FOR doc IN users FILTER doc.role == 'admin' OR doc.role == 'moderator' RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FILTER OR: {:?}", result.err());
}

#[test]
fn test_filter_not() {
    let query = "FOR doc IN users FILTER NOT doc.deleted RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FILTER NOT: {:?}", result.err());
}

#[test]
fn test_filter_like() {
    let query = "FOR doc IN users FILTER doc.name LIKE 'A%' RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FILTER LIKE: {:?}", result.err());
}

#[test]
fn test_filter_not_like() {
    let query = "FOR doc IN users FILTER doc.name NOT LIKE 'B%' RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FILTER NOT LIKE: {:?}", result.err());
}

#[test]
fn test_filter_regex() {
    let query = "FOR doc IN users FILTER doc.email =~ '^[a-z]+@' RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FILTER regex: {:?}", result.err());
}

#[test]
fn test_filter_not_regex() {
    let query = "FOR doc IN users FILTER doc.email !~ 'spam' RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FILTER NOT regex: {:?}", result.err());
}

#[test]
fn test_filter_in() {
    let query = "FOR doc IN users FILTER doc.role IN ['admin', 'moderator'] RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FILTER IN: {:?}", result.err());
}

#[test]
fn test_filter_complex_not() {
    // NOT IN is parsed differently - use NOT (x IN array) form
    let query = "FOR doc IN users FILTER NOT (doc.role IN ['guest', 'banned']) RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse FILTER NOT (IN): {:?}", result.err());
}

// ============================================================================
// CRUD Operations
// ============================================================================

#[test]
fn test_insert() {
    let query = "INSERT { name: 'Alice', age: 30 } INTO users";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse INSERT: {:?}", result.err());
}

#[test]
fn test_insert_with_return() {
    let query = "INSERT { name: 'Alice' } INTO users RETURN NEW";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse INSERT with RETURN NEW: {:?}", result.err());
}

#[test]
fn test_update() {
    let query = "FOR doc IN users FILTER doc._key == '123' UPDATE doc WITH { active: true } IN users";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse UPDATE: {:?}", result.err());
}

#[test]
fn test_update_with_return() {
    let query = "FOR doc IN users FILTER doc._key == '123' UPDATE doc WITH { score: 100 } IN users RETURN NEW";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse UPDATE with RETURN NEW: {:?}", result.err());
}

#[test]
fn test_remove() {
    let query = "FOR doc IN users FILTER doc.deleted == true REMOVE doc IN users";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse REMOVE: {:?}", result.err());
}

// ============================================================================
// Object and Array Expressions
// ============================================================================

#[test]
fn test_object_literal() {
    let query = "RETURN { name: 'test', value: 42 }";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse object literal: {:?}", result.err());
}

#[test]
fn test_nested_object() {
    let query = "RETURN { user: { name: 'Alice', address: { city: 'Paris' } } }";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse nested object: {:?}", result.err());
}

#[test]
fn test_array_literal() {
    let query = "RETURN [1, 2, 3, 4, 5]";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse array literal: {:?}", result.err());
}

#[test]
fn test_array_of_objects() {
    let query = "RETURN [{ a: 1 }, { a: 2 }, { a: 3 }]";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse array of objects: {:?}", result.err());
}

// ============================================================================
// Function Calls
// ============================================================================

#[test]
fn test_function_length() {
    let query = "FOR doc IN users RETURN LENGTH(doc.tags)";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse LENGTH function: {:?}", result.err());
}

#[test]
fn test_function_upper() {
    let query = "FOR doc IN users RETURN UPPER(doc.name)";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse UPPER function: {:?}", result.err());
}

#[test]
fn test_function_lower() {
    let query = "FOR doc IN users RETURN LOWER(doc.name)";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse LOWER function: {:?}", result.err());
}

#[test]
fn test_nested_function_calls() {
    let query = "FOR doc IN users RETURN UPPER(TRIM(doc.name))";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse nested function calls: {:?}", result.err());
}

// ============================================================================
// Ternary Expressions
// ============================================================================

#[test]
fn test_ternary_expression() {
    let query = "FOR doc IN users RETURN doc.age >= 18 ? 'adult' : 'minor'";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse ternary expression: {:?}", result.err());
}

// ============================================================================
// Collection Operations (COLLECT)
// ============================================================================

#[test]
fn test_collect_basic() {
    // COLLECT requires explicit field names in return object
    let query = "FOR doc IN orders COLLECT status = doc.status RETURN { status: status }";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse COLLECT: {:?}", result.err());
}

#[test]
fn test_collect_with_into() {
    let query = "FOR doc IN orders COLLECT status = doc.status INTO items RETURN { status: status, items: items }";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse COLLECT INTO: {:?}", result.err());
}

#[test]
fn test_collect_with_count() {
    // COLLECT WITH COUNT INTO syntax
    let query = "FOR doc IN orders COLLECT WITH COUNT INTO total RETURN total";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse COLLECT WITH COUNT: {:?}", result.err());
}

// ============================================================================
// Graph Traversals
// ============================================================================

#[test]
fn test_graph_outbound() {
    let query = "FOR v IN 1..3 OUTBOUND 'vertices/start' edges RETURN v";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse OUTBOUND traversal: {:?}", result.err());
}

#[test]
fn test_graph_inbound() {
    let query = "FOR v IN 1..3 INBOUND 'vertices/start' edges RETURN v";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse INBOUND traversal: {:?}", result.err());
}

#[test]
fn test_graph_any() {
    let query = "FOR v IN 1..3 ANY 'vertices/start' edges RETURN v";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse ANY traversal: {:?}", result.err());
}

#[test]
fn test_graph_with_edge_variable() {
    let query = "FOR v, e IN 1..3 OUTBOUND 'vertices/start' edges RETURN { vertex: v, edge: e }";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse traversal with edge variable: {:?}", result.err());
}

// ============================================================================
// Bind Variables
// ============================================================================

#[test]
fn test_bind_variable_value() {
    // Simple value bind variable with @
    let query = "FOR doc IN users FILTER doc.age > @minAge RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse value bind variable: {:?}", result.err());
}

#[test]
fn test_bind_variable_in_limit() {
    let query = "FOR doc IN users LIMIT @offset, @count RETURN doc";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse bind variables in LIMIT: {:?}", result.err());
}

// ============================================================================
// Complex Queries
// ============================================================================

#[test]
fn test_complex_query() {
    let query = r#"
        FOR user IN users
        FILTER user.active == true AND user.age >= 18
        LET posts = (FOR post IN posts FILTER post.userId == user._key RETURN post)
        SORT user.name ASC
        LIMIT 10
        RETURN { user, postCount: LENGTH(posts) }
    "#;
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse complex query: {:?}", result.err());
}

#[test]
fn test_subquery() {
    let query = "FOR user IN users RETURN { name: user.name, posts: (FOR p IN posts FILTER p.userId == user._key RETURN p) }";
    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse subquery: {:?}", result.err());
}

// ============================================================================
// Error Cases
// ============================================================================

#[test]
fn test_error_missing_return() {
    let query = "FOR doc IN users";
    let result = parse(query);
    assert!(result.is_err(), "Should fail without RETURN clause");
}

#[test]
fn test_error_invalid_syntax() {
    let query = "FOR IN users RETURN";
    let result = parse(query);
    assert!(result.is_err(), "Should fail with invalid syntax");
}

#[test]
fn test_error_unclosed_string() {
    let query = "FOR doc IN users FILTER doc.name == 'Alice RETURN doc";
    let result = parse(query);
    assert!(result.is_err(), "Should fail with unclosed string");
}
