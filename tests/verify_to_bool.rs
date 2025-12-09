use solidb::sdbql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn test_execute_to_bool_function() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test NULL -> false
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_BOOL".to_string(),
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
    assert_eq!(result[0], json!(false));

    // Test Boolean -> identity
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_BOOL".to_string(),
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
    assert_eq!(result[0], json!(true));

    // Test Number 0 -> false
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_BOOL".to_string(),
                    args: vec![Expression::Literal(json!(0))],
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
    assert_eq!(result[0], json!(false));

    // Test Number 1 -> true
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_BOOL".to_string(),
                    args: vec![Expression::Literal(json!(1))],
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
    assert_eq!(result[0], json!(true));

    // Test String empty -> false
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_BOOL".to_string(),
                    args: vec![Expression::Literal(json!(""))],
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
    assert_eq!(result[0], json!(false));

    // Test String non-empty -> true
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_BOOL".to_string(),
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
    assert_eq!(result[0], json!(true));

    // Test Array -> true
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_BOOL".to_string(),
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
    assert_eq!(result[0], json!(true));

    // Test Object -> true
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "TO_BOOL".to_string(),
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
    assert_eq!(result[0], json!(true));
}
