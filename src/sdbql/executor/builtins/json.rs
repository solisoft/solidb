//! JSON functions for SDBQL.
//!
//! JSON_PARSE, JSON_STRINGIFY, etc.

use crate::error::{DbError, DbResult};
use serde_json::Value;

/// Evaluate JSON functions
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "JSON_PARSE" | "PARSE_JSON" => {
            check_args(name, args, 1)?;
            let s = args[0].as_str().ok_or_else(|| {
                DbError::ExecutionError("JSON_PARSE: argument must be a string".to_string())
            })?;
            let parsed: Value = serde_json::from_str(s)
                .map_err(|e| DbError::ExecutionError(format!("JSON_PARSE: invalid JSON: {}", e)))?;
            Ok(Some(parsed))
        }
        "JSON_STRINGIFY" | "TO_JSON" => {
            check_args(name, args, 1)?;
            let s = serde_json::to_string(&args[0])
                .map_err(|e| DbError::ExecutionError(format!("JSON_STRINGIFY: {}", e)))?;
            Ok(Some(Value::String(s)))
        }
        "JSON_STRINGIFY_PRETTY" => {
            check_args(name, args, 1)?;
            let s = serde_json::to_string_pretty(&args[0])
                .map_err(|e| DbError::ExecutionError(format!("JSON_STRINGIFY_PRETTY: {}", e)))?;
            Ok(Some(Value::String(s)))
        }
        "JSON_POINTER" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "JSON_POINTER requires 2-3 arguments: value, pointer, [default]".to_string(),
                ));
            }
            if args[0].is_null() {
                return Ok(Some(args.get(2).cloned().unwrap_or(Value::Null)));
            }
            let pointer = args[1].as_str().ok_or_else(|| {
                DbError::ExecutionError("JSON_POINTER: pointer must be a string".to_string())
            })?;
            match args[0].pointer(pointer) {
                Some(v) => Ok(Some(v.clone())),
                None => Ok(Some(args.get(2).cloned().unwrap_or(Value::Null))),
            }
        }
        _ => Ok(None),
    }
}

fn check_args(name: &str, args: &[Value], expected: usize) -> DbResult<()> {
    if args.len() != expected {
        return Err(DbError::ExecutionError(format!(
            "{} requires {} argument(s)",
            name, expected
        )));
    }
    Ok(())
}
