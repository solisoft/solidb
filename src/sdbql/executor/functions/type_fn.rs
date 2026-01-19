//! Type checking and conversion functions for SDBQL

use serde_json::Value;

use crate::error::{DbError, DbResult};

/// Evaluate type checking and conversion functions
pub fn evaluate_type_fn(name: &str, args: &[Value]) -> DbResult<Value> {
    match name {
        // Type checking functions
        "IS_ARRAY" | "IS_LIST" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "IS_ARRAY requires 1 argument".to_string(),
                ));
            }
            Ok(Value::Bool(matches!(args[0], Value::Array(_))))
        }

        "IS_BOOL" | "IS_BOOLEAN" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "IS_BOOLEAN requires 1 argument".to_string(),
                ));
            }
            Ok(Value::Bool(matches!(args[0], Value::Bool(_))))
        }

        "IS_NUMBER" | "IS_NUMERIC" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "IS_NUMBER requires 1 argument".to_string(),
                ));
            }
            Ok(Value::Bool(matches!(args[0], Value::Number(_))))
        }

        "IS_INTEGER" | "IS_INT" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "IS_INTEGER requires 1 argument".to_string(),
                ));
            }
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
            Ok(Value::Bool(is_int))
        }

        "IS_STRING" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "IS_STRING requires 1 argument".to_string(),
                ));
            }
            Ok(Value::Bool(matches!(args[0], Value::String(_))))
        }

        "IS_OBJECT" | "IS_DOCUMENT" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "IS_OBJECT requires 1 argument".to_string(),
                ));
            }
            Ok(Value::Bool(matches!(args[0], Value::Object(_))))
        }

        "IS_NULL" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "IS_NULL requires 1 argument".to_string(),
                ));
            }
            Ok(Value::Bool(matches!(args[0], Value::Null)))
        }

        "IS_DATETIME" | "IS_DATESTRING" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "IS_DATETIME requires 1 argument".to_string(),
                ));
            }
            let is_datetime = match &args[0] {
                Value::String(s) => chrono::DateTime::parse_from_rfc3339(s).is_ok(),
                Value::Number(n) => {
                    if let Some(ts) = n.as_i64() {
                        ts >= 0 && ts < 32503680000000
                    } else {
                        false
                    }
                }
                _ => false,
            };
            Ok(Value::Bool(is_datetime))
        }

        "TYPENAME" | "TYPE_OF" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "TYPENAME requires 1 argument".to_string(),
                ));
            }
            let type_name = match &args[0] {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            Ok(Value::String(type_name.to_string()))
        }

        _ => Err(DbError::ExecutionError(format!(
            "Unknown type function: {}",
            name
        ))),
    }
}
