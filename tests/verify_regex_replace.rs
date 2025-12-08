use solidb::aql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn test_execute_regex_replace_function() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test basic replacement
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "REGEX_REPLACE".to_string(),
                    args: vec![
                        Expression::Literal(json!("the quick brown fox")),
                        Expression::Literal(json!("the (.*) fox")),
                        Expression::Literal(json!("a $1 dog")),
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
    assert_eq!(result[0].as_str().unwrap(), "a quick brown dog");

    // Test case insensitive
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "REGEX_REPLACE".to_string(),
                    args: vec![
                        Expression::Literal(json!("foobar")),
                        Expression::Literal(json!("FOO")),
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
    assert_eq!(result[0].as_str().unwrap(), "bazbar");

    // Test global replacement
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "REGEX_REPLACE".to_string(),
                    args: vec![
                        Expression::Literal(json!("banana")),
                        Expression::Literal(json!("a")),
                        Expression::Literal(json!("o")),
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
    assert_eq!(result[0].as_str().unwrap(), "bonono");
}
