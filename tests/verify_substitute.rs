use solidb::aql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn test_execute_substitute_function() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test simple replacement
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "SUBSTITUTE".to_string(),
                    args: vec![
                        Expression::Literal(json!("foobar")),
                        Expression::Literal(json!("foo")),
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
    assert_eq!(result[0], json!("bazbar"));

    // Test simple replacement with limit
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "SUBSTITUTE".to_string(),
                    args: vec![
                        Expression::Literal(json!("banana")),
                        Expression::Literal(json!("a")),
                        Expression::Literal(json!("o")),
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
    assert_eq!(result[0], json!("bonona"));

    // Test mapping replacement
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "SUBSTITUTE".to_string(),
                    args: vec![
                        Expression::Literal(json!("the quick brown fox")),
                        Expression::Literal(json!({
                            "quick": "slow",
                            "brown": "red",
                            "fox": "dog"
                        })),
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
    // Order is undefined but all should be replaced
    let s = result[0].as_str().unwrap();
    assert!(s.contains("slow"));
    assert!(s.contains("red"));
    assert!(s.contains("dog"));
    assert!(!s.contains("quick"));
}
