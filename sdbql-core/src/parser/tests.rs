//! Tests for the SDBQL parser.

use super::*;
use crate::ast::*;
use serde_json::json;

#[test]
fn test_simple_query() {
    let query = parse("FOR doc IN users RETURN doc").unwrap();
    assert_eq!(query.for_clauses.len(), 1);
    assert_eq!(query.for_clauses[0].variable, "doc");
    assert_eq!(query.for_clauses[0].collection, "users");
    assert!(query.return_clause.is_some());
}

#[test]
fn test_filter_clause() {
    let query = parse("FOR doc IN users FILTER doc.age > 18 RETURN doc").unwrap();
    assert_eq!(query.filter_clauses.len(), 1);
}

#[test]
fn test_sort_clause() {
    let query = parse("FOR doc IN users SORT doc.age DESC RETURN doc").unwrap();
    assert!(query.sort_clause.is_some());
    let sort = query.sort_clause.unwrap();
    assert_eq!(sort.fields.len(), 1);
    assert!(!sort.fields[0].1); // DESC = false
}

#[test]
fn test_limit_clause() {
    let query = parse("FOR doc IN users LIMIT 10 RETURN doc").unwrap();
    assert!(query.limit_clause.is_some());
}

#[test]
fn test_limit_with_offset() {
    let query = parse("FOR doc IN users LIMIT 5, 10 RETURN doc").unwrap();
    let limit = query.limit_clause.unwrap();
    assert_eq!(limit.offset, Expression::Literal(json!(5)));
    assert_eq!(limit.count, Expression::Literal(json!(10)));
}

#[test]
fn test_let_clause() {
    let query = parse("LET x = 10 FOR doc IN users RETURN doc").unwrap();
    assert_eq!(query.let_clauses.len(), 1);
    assert_eq!(query.let_clauses[0].variable, "x");
}

#[test]
fn test_multiple_let_bindings() {
    let query = parse("LET a = 1, b = 2, c = 3 RETURN a + b + c").unwrap();
    assert_eq!(query.let_clauses.len(), 3);
    assert_eq!(query.let_clauses[0].variable, "a");
    assert_eq!(query.let_clauses[1].variable, "b");
    assert_eq!(query.let_clauses[2].variable, "c");
}

#[test]
fn test_expression_binary_op() {
    let query = parse("RETURN 1 + 2").unwrap();
    let ret = query.return_clause.unwrap();
    if let Expression::BinaryOp { op, .. } = ret.expression {
        assert_eq!(op, BinaryOperator::Add);
    } else {
        panic!("Expected BinaryOp");
    }
}

#[test]
fn test_expression_comparison() {
    let query = parse("FOR doc IN users FILTER doc.age > 18 RETURN doc").unwrap();
    let filter = &query.filter_clauses[0];
    if let Expression::BinaryOp { op, .. } = &filter.expression {
        assert_eq!(*op, BinaryOperator::GreaterThan);
    } else {
        panic!("Expected BinaryOp");
    }
}

#[test]
fn test_function_call() {
    let query = parse("RETURN LENGTH([1, 2, 3])").unwrap();
    let ret = query.return_clause.unwrap();
    if let Expression::FunctionCall { name, args } = ret.expression {
        assert_eq!(name, "LENGTH");
        assert_eq!(args.len(), 1);
    } else {
        panic!("Expected FunctionCall");
    }
}

#[test]
fn test_object_expression() {
    let query = parse("RETURN {name: 'test', value: 42}").unwrap();
    let ret = query.return_clause.unwrap();
    if let Expression::Object(fields) = ret.expression {
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].0, "name");
        assert_eq!(fields[1].0, "value");
    } else {
        panic!("Expected Object");
    }
}

#[test]
fn test_array_expression() {
    let query = parse("RETURN [1, 2, 3]").unwrap();
    let ret = query.return_clause.unwrap();
    if let Expression::Array(elements) = ret.expression {
        assert_eq!(elements.len(), 3);
    } else {
        panic!("Expected Array");
    }
}

#[test]
fn test_range_expression() {
    let query = parse("FOR i IN 1..5 RETURN i").unwrap();
    let for_clause = &query.for_clauses[0];
    assert!(for_clause.source_expression.is_some());
    if let Some(Expression::Range(_, _)) = &for_clause.source_expression {
        // OK
    } else {
        panic!("Expected Range expression");
    }
}

#[test]
fn test_ternary_expression() {
    let query = parse("RETURN true ? 1 : 0").unwrap();
    let ret = query.return_clause.unwrap();
    if let Expression::Ternary { .. } = ret.expression {
        // OK
    } else {
        panic!("Expected Ternary");
    }
}

#[test]
fn test_field_access() {
    let query = parse("FOR doc IN users RETURN doc.name").unwrap();
    let ret = query.return_clause.unwrap();
    if let Expression::FieldAccess(_, field) = ret.expression {
        assert_eq!(field, "name");
    } else {
        panic!("Expected FieldAccess");
    }
}

#[test]
fn test_bind_variable() {
    let query = parse("FOR doc IN users FILTER doc._key == @id RETURN doc").unwrap();
    let filter = &query.filter_clauses[0];
    if let Expression::BinaryOp { right, .. } = &filter.expression {
        if let Expression::BindVariable(name) = right.as_ref() {
            assert_eq!(name, "id");
        } else {
            panic!("Expected BindVariable");
        }
    } else {
        panic!("Expected BinaryOp");
    }
}

#[test]
fn test_collect_clause() {
    let query = parse(
        "FOR doc IN orders COLLECT region = doc.region WITH COUNT INTO count RETURN {region, count}",
    )
    .unwrap();
    let collect = query.body_clauses.iter().find_map(|c| {
        if let BodyClause::Collect(c) = c {
            Some(c)
        } else {
            None
        }
    });
    assert!(collect.is_some());
    let collect = collect.unwrap();
    assert_eq!(collect.group_vars.len(), 1);
    assert_eq!(collect.count_var, Some("count".to_string()));
}

#[test]
fn test_join_clause() {
    let query =
        parse("FOR u IN users JOIN orders ON u._key == orders.user_id RETURN {u, orders}").unwrap();
    assert_eq!(query.join_clauses.len(), 1);
    assert_eq!(query.join_clauses[0].join_type, JoinType::Inner);
    assert_eq!(query.join_clauses[0].collection, "orders");
}

#[test]
fn test_left_join() {
    let query =
        parse("FOR u IN users LEFT JOIN orders ON u._key == orders.user_id RETURN {u, orders}")
            .unwrap();
    assert_eq!(query.join_clauses[0].join_type, JoinType::Left);
}

#[test]
fn test_insert_clause() {
    let query = parse("INSERT {name: 'test'} INTO users").unwrap();
    let insert = query.body_clauses.iter().find_map(|c| {
        if let BodyClause::Insert(i) = c {
            Some(i)
        } else {
            None
        }
    });
    assert!(insert.is_some());
    assert_eq!(insert.unwrap().collection, "users");
}

#[test]
fn test_update_clause() {
    let query = parse("FOR doc IN users UPDATE doc WITH {age: 30} IN users").unwrap();
    let update = query.body_clauses.iter().find_map(|c| {
        if let BodyClause::Update(u) = c {
            Some(u)
        } else {
            None
        }
    });
    assert!(update.is_some());
}

#[test]
fn test_remove_clause() {
    let query = parse("FOR doc IN users REMOVE doc IN users").unwrap();
    let remove = query.body_clauses.iter().find_map(|c| {
        if let BodyClause::Remove(r) = c {
            Some(r)
        } else {
            None
        }
    });
    assert!(remove.is_some());
}

#[test]
fn test_case_expression() {
    let query = parse("RETURN CASE WHEN x > 0 THEN 'positive' ELSE 'non-positive' END").unwrap();
    let ret = query.return_clause.unwrap();
    if let Expression::Case { when_clauses, .. } = ret.expression {
        assert_eq!(when_clauses.len(), 1);
    } else {
        panic!("Expected Case");
    }
}

#[test]
fn test_lambda_expression() {
    let query = parse("RETURN FILTER([1,2,3], x -> x > 1)").unwrap();
    let ret = query.return_clause.unwrap();
    if let Expression::FunctionCall { args, .. } = ret.expression {
        if let Expression::Lambda { params, .. } = &args[1] {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0], "x");
        } else {
            panic!("Expected Lambda");
        }
    } else {
        panic!("Expected FunctionCall");
    }
}

#[test]
fn test_null_coalesce() {
    let query = parse("RETURN doc.name ?? 'unknown'").unwrap();
    let ret = query.return_clause.unwrap();
    if let Expression::BinaryOp { op, .. } = ret.expression {
        assert_eq!(op, BinaryOperator::NullCoalesce);
    } else {
        panic!("Expected BinaryOp with NullCoalesce");
    }
}

#[test]
fn test_pipeline_expression() {
    let query = parse("RETURN [1,2,3] |> FILTER(x -> x > 1) |> LENGTH()").unwrap();
    let ret = query.return_clause.unwrap();
    if let Expression::Pipeline { .. } = ret.expression {
        // OK
    } else {
        panic!("Expected Pipeline");
    }
}

#[test]
fn test_subquery() {
    let query = parse("LET subs = (FOR doc IN users RETURN doc.name) RETURN subs").unwrap();
    let let_clause = &query.let_clauses[0];
    if let Expression::Subquery(_) = &let_clause.expression {
        // OK
    } else {
        panic!("Expected Subquery");
    }
}

#[test]
fn test_error_missing_return() {
    let result = parse("FOR doc IN users");
    assert!(result.is_err());
}

#[test]
fn test_error_unexpected_token() {
    let result = parse("FOR doc IN users INVALID RETURN doc");
    assert!(result.is_err());
}

#[test]
fn test_return_only_query() {
    let query = parse("RETURN 1 + 2").unwrap();
    assert!(query.for_clauses.is_empty());
    assert!(query.return_clause.is_some());
}

#[test]
fn test_optional_chaining() {
    let query = parse("RETURN doc?.address?.city").unwrap();
    let ret = query.return_clause.unwrap();
    if let Expression::OptionalFieldAccess(base, field) = ret.expression {
        assert_eq!(field, "city");
        if let Expression::OptionalFieldAccess(_, inner_field) = *base {
            assert_eq!(inner_field, "address");
        } else {
            panic!("Expected nested OptionalFieldAccess");
        }
    } else {
        panic!("Expected OptionalFieldAccess");
    }
}

#[test]
fn test_array_spread_access() {
    let query = parse("RETURN docs[*].name").unwrap();
    let ret = query.return_clause.unwrap();
    if let Expression::ArraySpreadAccess(_, field_path) = ret.expression {
        assert_eq!(field_path, Some("name".to_string()));
    } else {
        panic!("Expected ArraySpreadAccess");
    }
}
