use solidb::sdbql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn test_execute_to_array_function() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test NULL -> []
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_ARRAY".to_string(),
                    args: vec![Expression::Literal(Value::Null)],
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
    assert_eq!(result[0], json!([]));

    // Test Boolean -> [value]
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_ARRAY".to_string(),
                    args: vec![Expression::Literal(json!(true))],
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
    assert_eq!(result[0], json!([true]));

    // Test Number -> [value]
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_ARRAY".to_string(),
                    args: vec![Expression::Literal(json!(123))],
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
    assert_eq!(result[0], json!([123]));

    // Test String -> [value]
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_ARRAY".to_string(),
                    args: vec![Expression::Literal(json!("foo"))],
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
    assert_eq!(result[0], json!(["foo"]));

    // Test Array -> identity
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_ARRAY".to_string(),
                    args: vec![Expression::Literal(json!([1, 2, 3]))],
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
    assert_eq!(result[0], json!([1, 2, 3]));

    // Test Object -> [values]
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_ARRAY".to_string(),
                    args: vec![Expression::Literal(json!({"a": 1}))],
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
    // Check that 1 is in the array
    assert!(result[0].as_array().unwrap().contains(&json!(1)));
}
