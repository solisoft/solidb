use solidb::sdbql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn test_execute_to_number_function() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test NULL -> 0
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_NUMBER".to_string(),
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
    assert_eq!(result[0], json!(0));

    // Test Boolean true -> 1
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_NUMBER".to_string(),
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
    assert_eq!(result[0], json!(1));

    // Test Boolean false -> 0
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_NUMBER".to_string(),
                    args: vec![Expression::Literal(json!(false))],
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
    assert_eq!(result[0], json!(0));

    // Test Number identity
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_NUMBER".to_string(),
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
    assert_eq!(result[0], json!(123));

    // Test String parse valid
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_NUMBER".to_string(),
                    args: vec![Expression::Literal(json!("456.78"))],
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
    assert_eq!(result[0], json!(456.78));

    // Test String parse invalid
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_NUMBER".to_string(),
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
    assert_eq!(result[0], json!(0));

    // Test Array empty -> 0
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_NUMBER".to_string(),
                    args: vec![Expression::Literal(json!([]))],
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
    assert_eq!(result[0], json!(0));

    // Test Array single element -> recurse
     let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_NUMBER".to_string(),
                    args: vec![Expression::Literal(json!(["123"]))],
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
    assert_eq!(result[0], json!(123));

    // Test Array multiple elements -> 0
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_NUMBER".to_string(),
                    args: vec![Expression::Literal(json!([1, 2]))],
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
    assert_eq!(result[0], json!(0));

    // Test Object -> 0
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_NUMBER".to_string(),
                    args: vec![Expression::Literal(json!({}))],
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
    assert_eq!(result[0], json!(0));
}
