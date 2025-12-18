use solidb::sdbql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn test_execute_intersection_function() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test with two arrays
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "INTERSECTION".to_string(),
                    args: vec![
                        Expression::Literal(json!([1, 2, 3, 4, 5])),
                        Expression::Literal(json!([2, 3, 4, 6])),
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
    let arr = result[0].as_array().unwrap();
    // Intersection of [1,2,3,4,5] and [2,3,4,6] is [2,3,4]
    assert_eq!(arr.len(), 3);
    assert!(arr.contains(&json!(2)));
    assert!(arr.contains(&json!(3)));
    assert!(arr.contains(&json!(4)));

    // Test with three arrays
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "INTERSECTION".to_string(),
                    args: vec![
                        Expression::Literal(json!([1, 2, 3, 4, 5])),
                        Expression::Literal(json!([2, 3, 4, 6])),
                        Expression::Literal(json!([3, 4, 7])),
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
    let arr = result[0].as_array().unwrap();
    // Intersection of [1,2,3,4,5], [2,3,4,6], [3,4,7] is [3,4]
    assert_eq!(arr.len(), 2);
    assert!(arr.contains(&json!(3)));
    assert!(arr.contains(&json!(4)));

    // Test duplicates removal (result should be unique)
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "INTERSECTION".to_string(),
                    args: vec![
                        Expression::Literal(json!([1, 2, 2, 3])),
                        Expression::Literal(json!([2, 2, 3, 4])),
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
    let arr = result[0].as_array().unwrap();
    // Intersection should be [2, 3] (unique)
    assert_eq!(arr.len(), 2);
    assert!(arr.contains(&json!(2)));
    assert!(arr.contains(&json!(3)));

    // Test strictness: requires all arrays
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "INTERSECTION".to_string(),
                    args: vec![
                        Expression::Literal(json!([1, 2, 3])),
                        Expression::Literal(json!("not an array")),
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

    let result = executor.execute(&query);
    assert!(result.is_err());
}
