use solidb::sdbql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn test_execute_values_function() {
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
                    name: "VALUES".to_string(),
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
    let values = result[0].as_array().unwrap();
    // Since serde_json uses BTreeMap, keys are sorted: _id, _key, a, b, c
    // Values should correspond to that order
    assert!(values.contains(&json!(1)));
    assert!(values.contains(&json!(2)));
    assert!(values.contains(&json!(3)));
    assert!(values.contains(&json!("123")));
    assert!(values.contains(&json!("coll/123")));
    assert_eq!(values.len(), 5);

    // Test with removeInternal = true
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "VALUES".to_string(),
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
    let values = result[0].as_array().unwrap();
    assert!(values.contains(&json!(1)));
    assert!(values.contains(&json!(2)));
    assert!(!values.contains(&json!("123")));
    assert_eq!(values.len(), 2);
}
