use solidb::aql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn test_execute_contains_function() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test boolean mode (found)
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "CONTAINS".to_string(),
                    args: vec![
                        Expression::Literal(json!("foobar")),
                        Expression::Literal(json!("foo")),
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
    assert_eq!(result[0], json!(true));

    // Test boolean mode (not found)
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "CONTAINS".to_string(),
                    args: vec![
                        Expression::Literal(json!("foobar")),
                        Expression::Literal(json!("baz")),
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
    assert_eq!(result[0], json!(false));
    
     // Test index mode (found)
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "CONTAINS".to_string(),
                    args: vec![
                        Expression::Literal(json!("foobar")),
                        Expression::Literal(json!("bar")),
                        Expression::Literal(json!(true)),
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
    // "bar" starts at index 3
    assert_eq!(result[0], json!(3));

    // Test index mode (not found)
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "CONTAINS".to_string(),
                    args: vec![
                        Expression::Literal(json!("foobar")),
                        Expression::Literal(json!("baz")),
                        Expression::Literal(json!(true)),
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
    assert_eq!(result[0], json!(-1));
}
