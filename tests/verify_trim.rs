use solidb::sdbql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn test_execute_trim_functions() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test TRIM default (both sides whitespace)
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TRIM".to_string(),
                    args: vec![Expression::Literal(json!("  foo  "))],
                },
            }
        ],
        filter_clauses: vec![],
        sort_clause: None,
        limit_clause: None,
        return_clause: Some(ReturnClause { expression: Expression::Variable("result".to_string()) }),
        body_clauses: vec![],
    };
    let result = executor.execute(&query).unwrap();
    assert_eq!(result[0], json!("foo"));

    // Test TRIM left (1)
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TRIM".to_string(),
                    args: vec![Expression::Literal(json!("  foo  ")), Expression::Literal(json!(1))],
                },
            }
        ],
        filter_clauses: vec![],
        sort_clause: None,
        limit_clause: None,
        return_clause: Some(ReturnClause { expression: Expression::Variable("result".to_string()) }),
        body_clauses: vec![],
    };
    let result = executor.execute(&query).unwrap();
    assert_eq!(result[0], json!("foo  "));

    // Test TRIM chars
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TRIM".to_string(),
                    args: vec![Expression::Literal(json!("--foo--")), Expression::Literal(json!("-"))],
                },
            }
        ],
        filter_clauses: vec![],
        sort_clause: None,
        limit_clause: None,
        return_clause: Some(ReturnClause { expression: Expression::Variable("result".to_string()) }),
        body_clauses: vec![],
    };
    let result = executor.execute(&query).unwrap();
    assert_eq!(result[0], json!("foo"));

    // Test LTRIM default
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "LTRIM".to_string(),
                    args: vec![Expression::Literal(json!("  foo  "))],
                },
            }
        ],
        filter_clauses: vec![],
        sort_clause: None,
        limit_clause: None,
        return_clause: Some(ReturnClause { expression: Expression::Variable("result".to_string()) }),
        body_clauses: vec![],
    };
    let result = executor.execute(&query).unwrap();
    assert_eq!(result[0], json!("foo  "));

    // Test LTRIM chars
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "LTRIM".to_string(),
                    args: vec![Expression::Literal(json!("foobar")), Expression::Literal(json!("fao"))],
                },
            }
        ],
        filter_clauses: vec![],
        sort_clause: None,
        limit_clause: None,
        return_clause: Some(ReturnClause { expression: Expression::Variable("result".to_string()) }),
        body_clauses: vec![],
    };
    let result = executor.execute(&query).unwrap();
    // 'f', 'o', 'o' are in "fao", 'b' is not. so trims until b.
    assert_eq!(result[0], json!("bar"));

    // Test RTRIM chars
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "RTRIM".to_string(),
                    args: vec![Expression::Literal(json!("foobar")), Expression::Literal(json!("rab"))],
                },
            }
        ],
        filter_clauses: vec![],
        sort_clause: None,
        limit_clause: None,
        return_clause: Some(ReturnClause { expression: Expression::Variable("result".to_string()) }),
        body_clauses: vec![],
    };
    let result = executor.execute(&query).unwrap();
    assert_eq!(result[0], json!("foo"));
}
