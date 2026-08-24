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
    let query = parse("FOR doc IN users FILTER doc.age > 18 FILTER doc.active RETURN doc").unwrap();
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

#[test]
fn test_any_syntax() {
    let query = parse("FOR doc IN collection FILTER ANY member IN doc.members RETURN doc");
    assert!(
        query.is_ok(),
        "Failed to parse ANY syntax: {:?}",
        query.err()
    );
}

#[test]
fn test_any_satisfies_syntax() {
    let query = parse(
        "FOR doc IN collection FILTER ANY member IN doc.members SATISFIES member.age > 10 RETURN doc",
    );
    assert!(
        query.is_ok(),
        "Failed to parse ANY ... SATISFIES syntax: {:?}",
        query.err()
    );
}

#[test]
fn test_cte_simple() {
    let query = parse("WITH temp AS (FOR doc IN coll RETURN doc) FOR t IN temp RETURN t");
    assert!(query.is_ok(), "Failed to parse CTE: {:?}", query.err());
    let query = query.unwrap();
    assert!(query.with_clause.is_some());
    let with = query.with_clause.unwrap();
    assert_eq!(with.ctes.len(), 1);
    assert_eq!(with.ctes[0].name, "temp");
}

#[test]
fn test_cte_multiple() {
    let query = parse(
        "WITH a AS (FOR x IN coll RETURN x), b AS (FOR y IN coll2 RETURN y) FOR t IN a RETURN t",
    );
    assert!(
        query.is_ok(),
        "Failed to parse multiple CTEs: {:?}",
        query.err()
    );
    let query = query.unwrap();
    assert!(query.with_clause.is_some());
    let with = query.with_clause.unwrap();
    assert_eq!(with.ctes.len(), 2);
    assert_eq!(with.ctes[0].name, "a");
    assert_eq!(with.ctes[1].name, "b");
}

#[test]
fn test_cte_with_columns() {
    let query =
        parse("WITH temp(col1, col2) AS (FOR doc IN coll RETURN doc) FOR t IN temp RETURN t");
    assert!(
        query.is_ok(),
        "Failed to parse CTE with columns: {:?}",
        query.err()
    );
    let query = query.unwrap();
    assert!(query.with_clause.is_some());
    let with = query.with_clause.unwrap();
    assert_eq!(with.ctes[0].columns, vec!["col1", "col2"]);
}

#[test]
fn test_no_cte() {
    let query = parse("FOR doc IN coll RETURN doc").unwrap();
    assert!(query.with_clause.is_none());
}

#[test]
fn test_recursive_cte() {
    let query = parse(
        "WITH RECURSIVE tree AS (FOR d IN nodes FILTER d._key == @root RETURN d._key \
         UNION ALL FOR n IN nodes FILTER n.parent IN tree RETURN n._key) \
         FOR x IN tree RETURN x",
    );
    assert!(
        query.is_ok(),
        "Failed to parse recursive CTE: {:?}",
        query.err()
    );
    let query = query.unwrap();
    let with = query.with_clause.unwrap();
    assert_eq!(with.ctes.len(), 1);
    assert!(with.ctes[0].recursive);
    // Body must be anchor UNION ALL step
    assert_eq!(with.ctes[0].query.set_operations.len(), 1);
}

#[test]
fn test_return_distinct() {
    let query = parse("FOR doc IN coll RETURN DISTINCT doc.city").unwrap();
    let rc = query.return_clause.unwrap();
    assert!(rc.distinct);

    // Plain RETURN must not set the flag
    let query = parse("FOR doc IN coll RETURN doc.city").unwrap();
    assert!(!query.return_clause.unwrap().distinct);
}

#[test]
fn test_set_operations() {
    for (sql, expected) in [
        (
            "FOR a IN c1 RETURN a.x UNION FOR b IN c2 RETURN b.y",
            "Union",
        ),
        (
            "FOR a IN c1 RETURN a.x UNION ALL FOR b IN c2 RETURN b.y",
            "UnionAll",
        ),
        (
            "FOR a IN c1 RETURN a.x INTERSECT FOR b IN c2 RETURN b.y",
            "Intersect",
        ),
        (
            "FOR a IN c1 RETURN a.x EXCEPT FOR b IN c2 RETURN b.y",
            "Except",
        ),
    ] {
        let query = parse(sql).unwrap_or_else(|e| panic!("Failed to parse {sql}: {e:?}"));
        assert_eq!(query.set_operations.len(), 1, "{sql}");
        let op_name = format!("{:?}", query.set_operations[0].op);
        assert_eq!(op_name, expected, "{sql}");
    }
}

#[test]
fn test_set_operation_chain_is_flat_and_left_to_right() {
    // `a EXCEPT b EXCEPT c` must be one flat chain — nesting it to the right
    // would mean `a EXCEPT (b EXCEPT c)`.
    let query =
        parse("FOR a IN c1 RETURN a.x EXCEPT FOR b IN c2 RETURN b.x EXCEPT FOR c IN c3 RETURN c.x")
            .unwrap();
    assert_eq!(query.set_operations.len(), 2);
    assert!(query.set_operations[0].query.set_operations.is_empty());
}

#[test]
fn test_intersect_binds_tighter_than_union() {
    // `a UNION b INTERSECT c` groups as `a UNION (b INTERSECT c)`
    let query = parse(
        "FOR a IN c1 RETURN a.x UNION FOR b IN c2 RETURN b.x INTERSECT FOR c IN c3 RETURN c.x",
    )
    .unwrap();
    assert_eq!(query.set_operations.len(), 1);
    assert!(matches!(query.set_operations[0].op, SetOperator::Union));
    let nested = &query.set_operations[0].query.set_operations;
    assert_eq!(nested.len(), 1);
    assert!(matches!(nested[0].op, SetOperator::Intersect));
}

#[test]
fn test_parenthesized_left_operand() {
    // Explicit grouping on the left: `(a UNION b) INTERSECT c` intersects the
    // union, so the chain stays flat instead of nesting under `b`.
    let query = parse(
        "(FOR a IN c1 RETURN a.x UNION FOR b IN c2 RETURN b.x)          INTERSECT FOR c IN c3 RETURN c.x",
    )
    .unwrap();
    assert_eq!(query.set_operations.len(), 2);
    assert!(matches!(query.set_operations[0].op, SetOperator::Union));
    assert!(matches!(query.set_operations[1].op, SetOperator::Intersect));
    assert!(query.set_operations[0].query.set_operations.is_empty());
}

#[test]
fn test_offset_without_limit_has_no_count() {
    // A standalone OFFSET must not invent a count: the count is pushed into
    // storage scans as an allocation size.
    let query = parse("FOR d IN coll OFFSET 5 RETURN d").unwrap();
    let limit = query.limit_clause.expect("OFFSET produces a limit clause");
    assert!(limit.count.is_none());

    let query = parse("FOR d IN coll LIMIT 10 OFFSET 5 RETURN d").unwrap();
    let limit = query.limit_clause.expect("limit clause");
    assert_eq!(
        limit.count,
        Some(Expression::Literal(serde_json::json!(10)))
    );
    assert_eq!(limit.offset, Expression::Literal(serde_json::json!(5)));
}

#[test]
fn test_has_mutations_sees_nested_blocks() {
    // The HTTP handler decides caching, transaction handling and write
    // permission from this: a mutation hidden in an operand or a CTE body must
    // not read as a read-only query.
    let query =
        parse("FOR a IN c1 RETURN a.x UNION FOR d IN c2 REMOVE d IN c2 RETURN d._key").unwrap();
    assert!(query.has_mutations());

    let query =
        parse("WITH gone AS (FOR d IN c2 REMOVE d IN c2 RETURN d._key) FOR x IN gone RETURN x")
            .unwrap();
    assert!(query.has_mutations());

    let query = parse("FOR a IN c1 RETURN a.x UNION FOR b IN c2 RETURN b.x").unwrap();
    assert!(!query.has_mutations());
}

#[test]
fn test_set_operations_parenthesized_operand() {
    let query =
        parse("FOR a IN c1 RETURN a.x UNION (FOR b IN c2 FILTER b.z > 1 RETURN b.y)").unwrap();
    assert_eq!(query.set_operations.len(), 1);
    assert!(matches!(query.set_operations[0].op, SetOperator::Union));
}

#[test]
fn test_collect_keep() {
    let query = parse(
        "FOR u IN users COLLECT city = u.city INTO groups KEEP name, age SORT city RETURN city",
    )
    .unwrap();
    let collect = query
        .body_clauses
        .iter()
        .find_map(|c| match c {
            BodyClause::Collect(cc) => Some(cc.clone()),
            _ => None,
        })
        .expect("COLLECT clause");
    assert_eq!(collect.keep_vars, vec!["name", "age"]);
}

#[test]
fn test_parse_collect_with_aggregate_count() {
    let query =
        parse("FOR u IN users COLLECT city = u.city AGGREGATE count = COUNT() RETURN count");
    assert!(query.is_ok(), "Failed to parse: {:?}", query.err());
}

#[test]
fn test_parse_collect_aggregate_no_group_var() {
    let query = parse("FOR u IN users COLLECT AGGREGATE count = COUNT() RETURN count");
    assert!(query.is_ok(), "Failed to parse: {:?}", query.err());
}
