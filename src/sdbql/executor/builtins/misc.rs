//! Miscellaneous utility functions for SDBQL.
//!
//! UUID, SLEEP, TYPEOF, COALESCE, etc.

use crate::error::{DbError, DbResult};
use serde_json::Value;
use uuid::Uuid;

/// Evaluate misc functions
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "UUID" | "UUID_V4" => Ok(Some(Value::String(Uuid::new_v4().to_string()))),
        "UUID_V7" => Ok(Some(Value::String(Uuid::now_v7().to_string()))),
        "TYPEOF" | "TYPE_OF" | "TYPENAME" => {
            check_args(name, args, 1)?;
            let type_name = match &args[0] {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            Ok(Some(Value::String(type_name.to_string())))
        }
        "COALESCE" | "NOT_NULL" => {
            for arg in args {
                if !arg.is_null() {
                    return Ok(Some(arg.clone()));
                }
            }
            Ok(Some(Value::Null))
        }
        "NULLIF" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "NULLIF requires 2 arguments".to_string(),
                ));
            }
            if args[0] == args[1] {
                Ok(Some(Value::Null))
            } else {
                Ok(Some(args[0].clone()))
            }
        }
        "ASSERT" => {
            if args.is_empty() {
                return Err(DbError::ExecutionError(
                    "ASSERT requires at least 1 argument".to_string(),
                ));
            }
            let condition = match &args[0] {
                Value::Bool(b) => *b,
                Value::Null => false,
                _ => true,
            };
            if !condition {
                let msg = args
                    .get(1)
                    .and_then(|v| v.as_str())
                    .unwrap_or("Assertion failed");
                return Err(DbError::ExecutionError(format!("ASSERT: {}", msg)));
            }
            Ok(Some(Value::Bool(true)))
        }
        "SLEEP" => {
            check_args(name, args, 1)?;
            let ms = args[0].as_u64().ok_or_else(|| {
                DbError::ExecutionError("SLEEP: argument must be a number".to_string())
            })?;
            std::thread::sleep(std::time::Duration::from_millis(ms));
            Ok(Some(Value::Null))
        }
        "RANGE" => {
            if args.is_empty() || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "RANGE requires 1-3 arguments: end or start, end, [step]".to_string(),
                ));
            }
            let (start, end, step) = if args.len() == 1 {
                (0i64, args[0].as_i64().unwrap_or(0), 1i64)
            } else if args.len() == 2 {
                (
                    args[0].as_i64().unwrap_or(0),
                    args[1].as_i64().unwrap_or(0),
                    1i64,
                )
            } else {
                (
                    args[0].as_i64().unwrap_or(0),
                    args[1].as_i64().unwrap_or(0),
                    args[2].as_i64().unwrap_or(1).max(1),
                )
            };

            let mut result = Vec::new();
            let mut i = start;
            while i < end {
                result.push(Value::Number(serde_json::Number::from(i)));
                i += step;
            }
            Ok(Some(Value::Array(result)))
        }
        "TO_NUMBER" | "TO_NUM" => {
            check_args(name, args, 1)?;
            let num = match &args[0] {
                Value::Number(n) => n.clone(),
                Value::String(s) => s
                    .parse::<f64>()
                    .map(|f| serde_json::Number::from_f64(f).unwrap_or(serde_json::Number::from(0)))
                    .unwrap_or(serde_json::Number::from(0)),
                Value::Bool(true) => serde_json::Number::from(1),
                Value::Bool(false) => serde_json::Number::from(0),
                _ => serde_json::Number::from(0),
            };
            Ok(Some(Value::Number(num)))
        }
        "TO_STRING" | "TO_STR" => {
            check_args(name, args, 1)?;
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => "null".to_string(),
                v => serde_json::to_string(v).unwrap_or_default(),
            };
            Ok(Some(Value::String(s)))
        }
        "TO_BOOL" | "TO_BOOLEAN" => {
            check_args(name, args, 1)?;
            let b = match &args[0] {
                Value::Bool(b) => *b,
                Value::Null => false,
                Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
                Value::String(s) => !s.is_empty() && s != "false" && s != "0",
                Value::Array(a) => !a.is_empty(),
                Value::Object(o) => !o.is_empty(),
            };
            Ok(Some(Value::Bool(b)))
        }
        "TO_ARRAY" | "TO_LIST" => {
            check_args(name, args, 1)?;
            match &args[0] {
                Value::Array(arr) => Ok(Some(Value::Array(arr.clone()))),
                Value::Null => Ok(Some(Value::Array(vec![]))),
                other => Ok(Some(Value::Array(vec![other.clone()]))),
            }
        }
        "IF" => {
            if args.len() != 3 {
                return Err(DbError::ExecutionError(
                    "IF requires 3 arguments: condition, true_value, false_value".to_string(),
                ));
            }
            let condition = match &args[0] {
                Value::Bool(b) => *b,
                Value::Null => false,
                _ => true,
            };
            Ok(Some(if condition {
                args[1].clone()
            } else {
                args[2].clone()
            }))
        }
        "ATTRIBUTES" | "KEYS" => {
            check_args(name, args, 1)?;
            let keys = match &args[0] {
                Value::Object(obj) => obj.keys().map(|k| Value::String(k.clone())).collect(),
                Value::Array(arr) => {
                    let mut keys = Vec::new();
                    for item in arr {
                        if let Value::Object(obj) = item {
                            keys.extend(obj.keys().map(|k| Value::String(k.clone())));
                        }
                    }
                    keys
                }
                _ => {
                    return Err(DbError::ExecutionError(
                        "ATTRIBUTES: argument must be an object or array of objects".to_string(),
                    ));
                }
            };
            Ok(Some(Value::Array(keys)))
        }
        "VALUES" => {
            check_args(name, args, 1)?;
            let values = match &args[0] {
                Value::Object(obj) => obj.values().cloned().collect(),
                Value::Array(arr) => {
                    let mut values = Vec::new();
                    for item in arr {
                        if let Value::Object(obj) = item {
                            values.extend(obj.values().cloned());
                        }
                    }
                    values
                }
                _ => {
                    return Err(DbError::ExecutionError(
                        "VALUES: argument must be an object or array of objects".to_string(),
                    ));
                }
            };
            Ok(Some(Value::Array(values)))
        }
        "KEEP" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "KEEP requires at least 2 arguments: object, key1, key2, ...".to_string(),
                ));
            }
            let obj = match &args[0] {
                Value::Object(obj) => obj.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "KEEP: first argument must be an object".to_string(),
                    ));
                }
            };
            let keys: Vec<&str> = args[1..].iter().filter_map(|v| v.as_str()).collect();
            let result: serde_json::Map<String, Value> = obj
                .into_iter()
                .filter(|(k, _)| keys.contains(&k.as_str()))
                .collect();
            Ok(Some(Value::Object(result)))
        }
        "UNSET" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "UNSET requires at least 2 arguments: object, key1, key2, ...".to_string(),
                ));
            }
            let obj = match &args[0] {
                Value::Object(obj) => obj.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "UNSET: first argument must be an object".to_string(),
                    ));
                }
            };
            let keys: Vec<&str> = args[1..].iter().filter_map(|v| v.as_str()).collect();
            let result: serde_json::Map<String, Value> = obj
                .into_iter()
                .filter(|(k, _)| !keys.contains(&k.as_str()))
                .collect();
            Ok(Some(Value::Object(result)))
        }
        "HAS" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "HAS requires 2 arguments: object, key".to_string(),
                ));
            }
            let key = match &args[1] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "HAS: second argument must be a string (key)".to_string(),
                    ));
                }
            };
            let has_key = match &args[0] {
                Value::Object(obj) => obj.contains_key(&key),
                Value::Array(arr) => arr.iter().any(|item| {
                    if let Value::Object(obj) = item {
                        obj.contains_key(&key)
                    } else {
                        false
                    }
                }),
                _ => false,
            };
            Ok(Some(Value::Bool(has_key)))
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
