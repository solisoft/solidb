//! Type checking functions for SDBQL.
//!
//! IS_ARRAY, IS_BOOL, IS_NUMBER, IS_STRING, IS_NULL, IS_OBJECT, etc.

use crate::error::{DbError, DbResult};
use serde_json::Value;

/// Evaluate type checking functions
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "IS_ARRAY" | "IS_LIST" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Array(_)))))
        }
        "IS_BOOL" | "IS_BOOLEAN" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Bool(_)))))
        }
        "IS_NUMBER" | "IS_NUMERIC" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Number(_)))))
        }
        "IS_INTEGER" | "IS_INT" => {
            check_args(name, args, 1)?;
            let is_int = match &args[0] {
                Value::Number(n) => {
                    if n.as_i64().is_some() {
                        true
                    } else if let Some(f) = n.as_f64() {
                        f.fract() == 0.0 && f.is_finite()
                    } else {
                        false
                    }
                }
                _ => false,
            };
            Ok(Some(Value::Bool(is_int)))
        }
        "IS_STRING" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::String(_)))))
        }
        "IS_NULL" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Null))))
        }
        "IS_OBJECT" | "IS_DOCUMENT" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Object(_)))))
        }
        "IS_EMPTY" => {
            check_args(name, args, 1)?;
            let is_empty = match &args[0] {
                Value::Null => true,
                Value::String(s) => s.is_empty(),
                Value::Array(arr) => arr.is_empty(),
                Value::Object(obj) => obj.is_empty(),
                _ => false,
            };
            Ok(Some(Value::Bool(is_empty)))
        }
        "IS_DATE" => {
            check_args(name, args, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Bool(false)));
            }
            Ok(Some(Value::Bool(
                crate::sdbql::executor::utils::parse_datetime(&args[0]).is_ok(),
            )))
        }
        "IS_KEY" => {
            check_args(name, args, 1)?;
            let ok = args[0].as_str().is_some_and(|s| {
                !s.is_empty()
                    && s.len() <= 254
                    && !s.contains('/')
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
            });
            Ok(Some(Value::Bool(ok)))
        }
        "IS_SAME_COLLECTION" => {
            check_args(name, args, 2)?;
            let extract_collection = |val: &Value| -> Option<String> {
                match val {
                    Value::String(s) => s.split('/').next().map(|c| c.to_string()),
                    Value::Object(obj) => obj
                        .get("_id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.split('/').next().map(|c| c.to_string())),
                    _ => None,
                }
            };
            let col1 = extract_collection(&args[0]);
            let col2 = extract_collection(&args[1]);
            match (col1, col2) {
                (Some(c1), Some(c2)) => Ok(Some(Value::Bool(c1 == c2))),
                _ => Ok(Some(Value::Bool(false))),
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
