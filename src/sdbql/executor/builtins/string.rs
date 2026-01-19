//! String functions for SDBQL.
//!
//! UPPER, LOWER, TRIM, SPLIT, CONCAT, CONTAINS, SUBSTRING, etc.

use crate::error::{DbError, DbResult};
use serde_json::Value;

/// Evaluate string functions
#[allow(clippy::get_first)]
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "UPPER" | "TO_UPPER" | "TOUPPER" => {
            check_args(name, args, 1)?;
            let s = args[0].as_str().unwrap_or("");
            Ok(Some(Value::String(s.to_uppercase())))
        }
        "LOWER" | "TO_LOWER" | "TOLOWER" => {
            check_args(name, args, 1)?;
            let s = args[0].as_str().unwrap_or("");
            Ok(Some(Value::String(s.to_lowercase())))
        }
        "TRIM" => {
            check_args(name, args, 1)?;
            let s = args[0].as_str().unwrap_or("");
            Ok(Some(Value::String(s.trim().to_string())))
        }
        "LTRIM" => {
            check_args(name, args, 1)?;
            let s = args[0].as_str().unwrap_or("");
            Ok(Some(Value::String(s.trim_start().to_string())))
        }
        "RTRIM" => {
            check_args(name, args, 1)?;
            let s = args[0].as_str().unwrap_or("");
            Ok(Some(Value::String(s.trim_end().to_string())))
        }
        "CONCAT" | "CONCAT_WS" => {
            if args.is_empty() {
                return Ok(Some(Value::String(String::new())));
            }
            let result: String = args
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    Value::Null => String::new(),
                    _ => serde_json::to_string(v).unwrap_or_default(),
                })
                .collect();
            Ok(Some(Value::String(result)))
        }
        "CONTAINS" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "CONTAINS requires 2 arguments: string, search".to_string(),
                ));
            }
            let haystack = args[0].as_str().unwrap_or("");
            let needle = args[1].as_str().unwrap_or("");
            Ok(Some(Value::Bool(haystack.contains(needle))))
        }
        "STARTS_WITH" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "STARTS_WITH requires 2 arguments".to_string(),
                ));
            }
            let s = args[0].as_str().unwrap_or("");
            let prefix = args[1].as_str().unwrap_or("");
            Ok(Some(Value::Bool(s.starts_with(prefix))))
        }
        "ENDS_WITH" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "ENDS_WITH requires 2 arguments".to_string(),
                ));
            }
            let s = args[0].as_str().unwrap_or("");
            let suffix = args[1].as_str().unwrap_or("");
            Ok(Some(Value::Bool(s.ends_with(suffix))))
        }
        "SPLIT" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "SPLIT requires 2 arguments: string, separator".to_string(),
                ));
            }
            let s = args[0].as_str().unwrap_or("");
            let sep = args[1].as_str().unwrap_or(",");
            let parts: Vec<Value> = s.split(sep).map(|p| Value::String(p.to_string())).collect();
            Ok(Some(Value::Array(parts)))
        }
        "SUBSTRING" | "SUBSTR" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "SUBSTRING requires 2-3 arguments: string, start, [length]".to_string(),
                ));
            }
            let s = args[0].as_str().unwrap_or("");
            let start = args[1].as_u64().unwrap_or(0) as usize;
            let chars: Vec<char> = s.chars().collect();

            if start >= chars.len() {
                return Ok(Some(Value::String(String::new())));
            }

            let end = if args.len() > 2 {
                let len = args[2].as_u64().unwrap_or(chars.len() as u64) as usize;
                std::cmp::min(start + len, chars.len())
            } else {
                chars.len()
            };

            let result: String = chars[start..end].iter().collect();
            Ok(Some(Value::String(result)))
        }
        "REPLACE" => {
            if args.len() < 3 {
                return Err(DbError::ExecutionError(
                    "REPLACE requires 3 arguments: string, search, replace".to_string(),
                ));
            }
            let s = args[0].as_str().unwrap_or("");
            let search = args[1].as_str().unwrap_or("");
            let replace = args[2].as_str().unwrap_or("");
            Ok(Some(Value::String(s.replace(search, replace))))
        }
        "LEFT" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "LEFT requires 2 arguments: string, length".to_string(),
                ));
            }
            let s = args[0].as_str().unwrap_or("");
            let len = args[1].as_u64().unwrap_or(0) as usize;
            let result: String = s.chars().take(len).collect();
            Ok(Some(Value::String(result)))
        }
        "RIGHT" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "RIGHT requires 2 arguments: string, length".to_string(),
                ));
            }
            let s = args[0].as_str().unwrap_or("");
            let len = args[1].as_u64().unwrap_or(0) as usize;
            let chars: Vec<char> = s.chars().collect();
            let start = chars.len().saturating_sub(len);
            let result: String = chars[start..].iter().collect();
            Ok(Some(Value::String(result)))
        }
        "CHAR_LENGTH" | "CHARACTER_LENGTH" => {
            check_args(name, args, 1)?;
            let s = args[0].as_str().unwrap_or("");
            Ok(Some(Value::Number(serde_json::Number::from(
                s.chars().count(),
            ))))
        }
        "REVERSE" if args.get(0).map(|v| v.is_string()).unwrap_or(false) => {
            check_args(name, args, 1)?;
            let s = args[0].as_str().unwrap_or("");
            let reversed: String = s.chars().rev().collect();
            Ok(Some(Value::String(reversed)))
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
