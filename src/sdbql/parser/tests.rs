//! Unit tests for the SDBQL parser.

use super::*;

#[test]
fn test_parse_simple_for_return() {
    let query = parse("FOR doc IN users RETURN doc").unwrap();
    assert_eq!(query.for_clauses.len(), 1);
    assert!(query.return_clause.is_some());
}

#[test]
fn test_parse_for_filter_return() {
    let query = parse("FOR doc IN users FILTER doc.age > 18 RETURN doc").unwrap();
    assert_eq!(query.filter_clauses.len(), 1);
    assert!(query.return_clause.is_some());
}

#[test]
fn test_parse_for_sort_limit_return() {
    let query = parse("FOR doc IN users SORT doc.name ASC LIMIT 10 RETURN doc").unwrap();
    assert!(query.sort_clause.is_some());
    assert!(query.limit_clause.is_some());
}

#[test]
fn test_parse_insert() {
    let query = parse("INSERT { name: \"Alice\" } INTO users").unwrap();
    assert!(query
        .body_clauses
        .iter()
        .any(|c| matches!(c, BodyClause::Insert(_))));
}

#[test]
fn test_parse_update() {
    let query = parse("FOR doc IN users UPDATE doc WITH { active: true } IN users").unwrap();
    assert!(query
        .body_clauses
        .iter()
        .any(|c| matches!(c, BodyClause::Update(_))));
}

#[test]
fn test_parse_remove() {
    let query = parse("FOR doc IN users REMOVE doc IN users").unwrap();
    assert!(query
        .body_clauses
        .iter()
        .any(|c| matches!(c, BodyClause::Remove(_))));
}

#[test]
fn test_parse_collect() {
    let query = parse("FOR doc IN users COLLECT city = doc.city RETURN city").unwrap();
    assert!(query
        .body_clauses
        .iter()
        .any(|c| matches!(c, BodyClause::Collect(_))));
}

#[test]
fn test_parse_let_clause() {
    let query = parse("LET x = 5 RETURN x").unwrap();
    assert_eq!(query.let_clauses.len(), 1);
}

#[test]
fn test_parse_let_multiple_bindings() {
    // Test comma-separated LET bindings
    let query = parse("LET a = 1, b = 2, c = 3 RETURN a + b + c").unwrap();
    assert_eq!(query.let_clauses.len(), 3);
    assert_eq!(query.let_clauses[0].variable, "a");
    assert_eq!(query.let_clauses[1].variable, "b");
    assert_eq!(query.let_clauses[2].variable, "c");
}

#[test]
fn test_parse_let_multiple_in_body() {
    // Test comma-separated LET bindings after FOR
    let query = parse("FOR doc IN users LET x = doc.a, y = doc.b RETURN {x, y}").unwrap();
    let let_count = query
        .body_clauses
        .iter()
        .filter(|c| matches!(c, BodyClause::Let(_)))
        .count();
    assert_eq!(let_count, 2);
}

#[test]
fn test_parse_return_arithmetic() {
    let query = parse("RETURN 1 + 2 * 3").unwrap();
    assert!(query.return_clause.is_some());
    let ret = query.return_clause.unwrap();
    assert!(matches!(ret.expression, Expression::BinaryOp { .. }));
}

#[test]
fn test_parse_error_incomplete() {
    let result = parse("FOR doc IN");
    assert!(result.is_err());
}

#[test]
fn test_parse_error_invalid_token() {
    let result = parse("FOR 123 IN users");
    assert!(result.is_err());
}

#[test]
fn test_parse_sort_desc() {
    let query = parse("FOR doc IN users SORT doc.age DESC RETURN doc").unwrap();
    let sort = query.sort_clause.unwrap();
    assert_eq!(sort.fields.len(), 1);
    assert!(!sort.fields[0].1);
}

#[test]
fn test_parse_multiple_filters() {
    let query =
        parse("FOR doc IN users FILTER doc.age > 18 FILTER doc.active RETURN doc").unwrap();
    assert_eq!(query.filter_clauses.len(), 2);
}

#[test]
fn test_parse_nested_for() {
    let query = parse("FOR a IN users FOR b IN orders RETURN { user: a, order: b }").unwrap();
    assert_eq!(query.for_clauses.len(), 2);
}

#[test]
fn test_parse_not_in() {
    let query = parse("FOR x IN collection FILTER x.id NOT IN [1, 2, 3] RETURN x").unwrap();
    if let BodyClause::Filter(filter) = &query.body_clauses[1] {
        if let Expression::BinaryOp { op, .. } = &filter.expression {
            assert_eq!(*op, BinaryOperator::NotIn);
        } else {
            panic!("Expected BinaryOp::NotIn");
        }
    } else {
        panic!("Expected FilterClause");
    }
}

#[test]
fn test_parse_create_stream() {
    let input = r#"
            CREATE STREAM high_value_txns AS
            FOR txn IN transactions
            WINDOW TUMBLING (SIZE "1m")
            FILTER txn.amount > 1000
            RETURN txn
        "#;
    let mut parser = Parser::new(input).unwrap();
    let query = parser.parse().unwrap();

    assert!(query.create_stream_clause.is_some());
    assert_eq!(query.create_stream_clause.unwrap().name, "high_value_txns");
    assert!(query.window_clause.is_some());
    assert_eq!(query.window_clause.unwrap().duration, "1m");
    assert_eq!(query.for_clauses.len(), 1);
    assert_eq!(query.for_clauses[0].collection, "transactions");
}
