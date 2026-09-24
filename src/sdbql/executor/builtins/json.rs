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
            // Documented: null for null / non-string input and for invalid
            // JSON, so one malformed row does not abort the whole query.
            let parsed = args[0]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(Value::Null);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_parse_returns_null_on_error_or_null() {
        let parse = |v: Value| evaluate("JSON_PARSE", &[v]).unwrap().unwrap();
        assert_eq!(parse(json!("{\"a\":1}")), json!({"a": 1}));
        assert_eq!(parse(json!("{not json")), Value::Null);
        assert_eq!(parse(Value::Null), Value::Null);
        assert_eq!(parse(json!(42)), Value::Null);
        assert_eq!(parse(json!("null")), Value::Null);
    }
}
