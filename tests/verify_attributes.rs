use solidb::aql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn test_execute_attributes_function() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test with plain object
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "ATTRIBUTES".to_string(),
                    args: vec![Expression::Literal(json!({
                        "a": 1,
                        "b": 2,
                        "c": 3,
                        "_key": "123",
                        "_id": "coll/123"
                    }))],
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
    assert_eq!(result.len(), 1);
    let attributes = result[0].as_array().unwrap();
    // Default behavior: don't sort, don't remove internal (but order is not guaranteed in JSON map)
    // So we just check containment
    assert!(attributes.contains(&json!("a")));
    assert!(attributes.contains(&json!("b")));
    assert!(attributes.contains(&json!("c")));
    assert!(attributes.contains(&json!("_key")));
    assert!(attributes.contains(&json!("_id")));
    assert_eq!(attributes.len(), 5);

    // Test with removeInternal = true
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "ATTRIBUTES".to_string(),
                    args: vec![
                        Expression::Literal(json!({
                            "a": 1,
                            "b": 2,
                            "_key": "123"
                        })),
                        Expression::Literal(json!(true)), // removeInternal
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
    let attributes = result[0].as_array().unwrap();
    assert!(attributes.contains(&json!("a")));
    assert!(attributes.contains(&json!("b")));
    assert!(!attributes.contains(&json!("_key")));
    assert_eq!(attributes.len(), 2);

    // Test with sort = true
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "ATTRIBUTES".to_string(),
                    args: vec![
                        Expression::Literal(json!({
                            "c": 3,
                            "a": 1,
                            "b": 2
                        })),
                        Expression::Literal(json!(false)), // removeInternal
                        Expression::Literal(json!(true)),  // sort
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
    let attributes = result[0].as_array().unwrap();
    assert_eq!(attributes, &vec![json!("a"), json!("b"), json!("c")]);
}
