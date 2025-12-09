use solidb::sdbql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn test_execute_unset_function() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test with varargs
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "UNSET".to_string(),
                    args: vec![
                        Expression::Literal(json!({
                            "a": 1,
                            "b": 2,
                            "c": 3
                        })),
                        Expression::Literal(json!("a")),
                        Expression::Literal(json!("c")),
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
    let doc = result[0].as_object().unwrap();
    assert!(!doc.contains_key("a"));
    assert!(doc.contains_key("b"));
    assert!(!doc.contains_key("c"));
    assert_eq!(doc.len(), 1);

    // Test with array argument
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "UNSET".to_string(),
                    args: vec![
                        Expression::Literal(json!({
                            "a": 1,
                            "b": 2,
                            "c": 3
                        })),
                        Expression::Literal(json!(["a", "b"])),
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
    let doc = result[0].as_object().unwrap();
    assert!(!doc.contains_key("a"));
    assert!(!doc.contains_key("b"));
    assert!(doc.contains_key("c"));
    assert_eq!(doc.len(), 1);

    // Test with non-existent keys (should not fail)
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "UNSET".to_string(),
                    args: vec![
                        Expression::Literal(json!({
                            "a": 1
                        })),
                        Expression::Literal(json!("z")),
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
    let doc = result[0].as_object().unwrap();
    assert!(doc.contains_key("a"));
    assert_eq!(doc.len(), 1);
}
