use solidb::aql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn test_execute_split_function() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test simple split
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "SPLIT".to_string(),
                    args: vec![
                        Expression::Literal(json!("foo-bar-baz")),
                        Expression::Literal(json!("-")),
                    ],
                },
            }
        ],
        filter_clauses: vec![],
        sort_clause: None,
        limit_clause: None,
        return_clause: Some(ReturnClause {
            expression: Expression::Variable("result".to_string()),
        }),
        body_clauses: vec![],
    };
    let result = executor.execute(&query).unwrap();
    assert_eq!(result[0], json!(["foo", "bar", "baz"]));

    // Test split limit 2 (left)
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "SPLIT".to_string(),
                    args: vec![
                        Expression::Literal(json!("foo-bar-baz")),
                        Expression::Literal(json!("-")),
                        Expression::Literal(json!(2)),
                    ],
                },
            }
        ],
        filter_clauses: vec![],
        sort_clause: None,
        limit_clause: None,
        return_clause: Some(ReturnClause {
            expression: Expression::Variable("result".to_string()),
        }),
        body_clauses: vec![],
    };
    let result = executor.execute(&query).unwrap();
    assert_eq!(result[0], json!(["foo", "bar-baz"]));

    // Test split limit -2 (right)
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "SPLIT".to_string(),
                    args: vec![
                        Expression::Literal(json!("foo-bar-baz")),
                        Expression::Literal(json!("-")),
                        Expression::Literal(json!(-2)),
                    ],
                },
            }
        ],
        filter_clauses: vec![],
        sort_clause: None,
        limit_clause: None,
        return_clause: Some(ReturnClause {
            expression: Expression::Variable("result".to_string()),
        }),
        body_clauses: vec![],
    };
    let result = executor.execute(&query).unwrap();
    assert_eq!(result[0], json!(["foo-bar", "baz"]));

    // Test empty separator (char split)
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "SPLIT".to_string(),
                    args: vec![
                        Expression::Literal(json!("foo")),
                        Expression::Literal(json!("")),
                    ],
                },
            }
        ],
        filter_clauses: vec![],
        sort_clause: None,
        limit_clause: None,
        return_clause: Some(ReturnClause {
            expression: Expression::Variable("result".to_string()),
        }),
        body_clauses: vec![],
    };
    let result = executor.execute(&query).unwrap();
    assert_eq!(result[0], json!(["f", "o", "o"]));
}
