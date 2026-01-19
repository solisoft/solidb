//! Array functions for SDBQL.
//!
//! FIRST, LAST, LENGTH, REVERSE, SORTED, UNIQUE, FLATTEN, etc.

use super::super::values_equal;
use crate::error::{DbError, DbResult};
use serde_json::Value;

/// Evaluate array functions
#[allow(clippy::get_first)]
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "FIRST" => {
            check_args(name, args, 1)?;
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("FIRST: argument must be an array".to_string())
            })?;
            Ok(Some(arr.first().cloned().unwrap_or(Value::Null)))
        }
        "LAST" => {
            check_args(name, args, 1)?;
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("LAST: argument must be an array".to_string())
            })?;
            Ok(Some(arr.last().cloned().unwrap_or(Value::Null)))
        }
        "REVERSE" if args.get(0).map(|v| v.is_array()).unwrap_or(false) => {
            check_args(name, args, 1)?;
            let arr = args[0].as_array().unwrap();
            let mut reversed = arr.clone();
            reversed.reverse();
            Ok(Some(Value::Array(reversed)))
        }
        "SORTED" | "SORT" => {
            check_args(name, args, 1)?;
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("SORTED: argument must be an array".to_string())
            })?;
            let mut sorted = arr.clone();
            sorted.sort_by(|a, b| match (a, b) {
                (Value::Number(na), Value::Number(nb)) => na
                    .as_f64()
                    .unwrap_or(0.0)
                    .partial_cmp(&nb.as_f64().unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal),
                (Value::String(sa), Value::String(sb)) => sa.cmp(sb),
                _ => std::cmp::Ordering::Equal,
            });
            Ok(Some(Value::Array(sorted)))
        }
        "SORTED_DESC" => {
            check_args(name, args, 1)?;
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("SORTED_DESC: argument must be an array".to_string())
            })?;
            let mut sorted = arr.clone();
            sorted.sort_by(|a, b| match (a, b) {
                (Value::Number(na), Value::Number(nb)) => nb
                    .as_f64()
                    .unwrap_or(0.0)
                    .partial_cmp(&na.as_f64().unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal),
                (Value::String(sa), Value::String(sb)) => sb.cmp(sa),
                _ => std::cmp::Ordering::Equal,
            });
            Ok(Some(Value::Array(sorted)))
        }
        "UNIQUE" => {
            check_args(name, args, 1)?;
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("UNIQUE: argument must be an array".to_string())
            })?;
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let unique: Vec<Value> = arr
                .iter()
                .filter(|v| {
                    let key = serde_json::to_string(v).unwrap_or_default();
                    seen.insert(key)
                })
                .cloned()
                .collect();
            Ok(Some(Value::Array(unique)))
        }
        "FLATTEN" => {
            if args.is_empty() {
                return Err(DbError::ExecutionError(
                    "FLATTEN requires at least 1 argument".to_string(),
                ));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("FLATTEN: first argument must be an array".to_string())
            })?;
            let depth = args.get(1).and_then(|v| v.as_u64()).unwrap_or(1) as usize;
            let flattened = flatten_array(arr, depth);
            Ok(Some(Value::Array(flattened)))
        }
        "PUSH" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "PUSH requires 2 arguments: array, value".to_string(),
                ));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("PUSH: first argument must be an array".to_string())
            })?;
            let mut result = arr.clone();
            result.push(args[1].clone());
            Ok(Some(Value::Array(result)))
        }
        "POP" => {
            check_args(name, args, 1)?;
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("POP: argument must be an array".to_string())
            })?;
            let mut result = arr.clone();
            result.pop();
            Ok(Some(Value::Array(result)))
        }
        "SLICE" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "SLICE requires 2-3 arguments: array, start, [length]".to_string(),
                ));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("SLICE: first argument must be an array".to_string())
            })?;
            let start = args[1].as_i64().unwrap_or(0);
            let start = if start < 0 {
                (arr.len() as i64 + start).max(0) as usize
            } else {
                start as usize
            };
            let end = if args.len() > 2 {
                let len = args[2].as_u64().unwrap_or(arr.len() as u64) as usize;
                std::cmp::min(start + len, arr.len())
            } else {
                arr.len()
            };
            let result: Vec<Value> = arr[start..end].to_vec();
            Ok(Some(Value::Array(result)))
        }
        "POSITION" | "INDEX_OF" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "POSITION requires 2 arguments: array, value".to_string(),
                ));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("POSITION: first argument must be an array".to_string())
            })?;
            let search = &args[1];
            for (i, item) in arr.iter().enumerate() {
                if values_equal(item, search) {
                    return Ok(Some(Value::Number(serde_json::Number::from(i))));
                }
            }
            Ok(Some(Value::Number(serde_json::Number::from(-1i64))))
        }
        "NTH" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "NTH requires 2 arguments: array, index".to_string(),
                ));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("NTH: first argument must be an array".to_string())
            })?;
            let idx = args[1]
                .as_u64()
                .ok_or_else(|| DbError::ExecutionError("NTH: index must be a number".to_string()))?
                as usize;
            Ok(Some(arr.get(idx).cloned().unwrap_or(Value::Null)))
        }
        "COUNT" => {
            check_args(name, args, 1)?;
            match &args[0] {
                Value::Array(arr) => Ok(Some(Value::Number(serde_json::Number::from(arr.len())))),
                Value::Null => Ok(Some(Value::Number(serde_json::Number::from(0)))),
                _ => Ok(Some(Value::Number(serde_json::Number::from(1)))),
            }
        }
        _ => Ok(None),
    }
}

fn flatten_array(arr: &[Value], depth: usize) -> Vec<Value> {
    if depth == 0 {
        return arr.to_vec();
    }
    let mut result = Vec::new();
    for item in arr {
        if let Value::Array(inner) = item {
            result.extend(flatten_array(inner, depth - 1));
        } else {
            result.push(item.clone());
        }
    }
    result
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
