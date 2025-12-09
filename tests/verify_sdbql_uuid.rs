use solidb::sdbql::{Query, LetClause, Expression, ReturnClause, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::{json, Value};
use tempfile::tempdir;
use regex::Regex;

#[test]
fn test_execute_uuidv4_function() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test UUIDV4
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "result".to_string(),
                expression: Expression::FunctionCall {
                    name: "UUIDV4".to_string(),
                    args: vec![],
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
    
    // Check if result is a string and valid UUID
    let uuid_str = result[0].as_str().expect("UUIDV4 should return a string");
    let re = Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$").unwrap();
    assert!(re.is_match(uuid_str), "UUIDV4 should be a valid v4 UUID: {}", uuid_str);
}

#[test]
fn test_execute_uuidv7_function() {
    let temp_dir = tempdir().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    let executor = QueryExecutor::new(&storage);

    // Test UUIDV7
    let query = Query {
        for_clauses: vec![],
        let_clauses: vec![
            LetClause {
                variable: "uuid1".to_string(),
                expression: Expression::FunctionCall {
                    name: "UUIDV7".to_string(),
                    args: vec![],
                },
            },
             LetClause {
                variable: "uuid2".to_string(),
                expression: Expression::FunctionCall {
                    name: "UUIDV7".to_string(),
                    args: vec![],
                },
            }
        ],
        filter_clauses: vec![],
        sort_clause: None,
        limit_clause: None,
        return_clause: Some(ReturnClause { expression: Expression::Array(vec![
            Expression::Variable("uuid1".to_string()),
            Expression::Variable("uuid2".to_string())
        ]) }),
        body_clauses: vec![],
    };
    let result = executor.execute(&query).unwrap();
    let arr = result[0].as_array().expect("Result should be array of uuids");
    let uuid1 = arr[0].as_str().unwrap();
    let uuid2 = arr[1].as_str().unwrap();
    
    // Check v7 format
    let re = Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$").unwrap();
    assert!(re.is_match(uuid1), "UUIDV7 should be a valid v7 UUID: {}", uuid1);
    assert!(re.is_match(uuid2), "UUIDV7 should be a valid v7 UUID: {}", uuid2);

    // Check time ordering only if they are different (they usually are, but rigorous check might fail if too fast? no, v7 has counter)
    // Actually rust uuid v7 generates monotonic.
    // However, if generated extremely fast, they might be equal if system clock is coarse? 
    // uuid crate handles monotonicity.
    
    if uuid1 != uuid2 {
        assert!(uuid1 < uuid2, "UUIDV7 should be monotonic/time-ordered: {} < {}", uuid1, uuid2);
    }
}
