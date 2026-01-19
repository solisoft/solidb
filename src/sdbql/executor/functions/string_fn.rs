//! String functions for SDBQL

use serde_json::Value;

use crate::error::{DbError, DbResult};
use crate::sdbql::executor::utils::number_from_f64;

#[inline]
fn make_number(f: f64) -> Value {
    Value::Number(number_from_f64(f))
}

/// Evaluate string functions
pub fn evaluate_string_fn(name: &str, args: &[Value]) -> DbResult<Value> {
    match name {
        "REGEX_REPLACE" => {
            if args.len() != 3 && args.len() != 4 {
                return Err(DbError::ExecutionError(
                    "REGEX_REPLACE requires 3-4 arguments".to_string(),
                ));
            }
            let text = match &args[0] {
                Value::String(s) => s.clone(),
                Value::Null => return Ok(Value::Null),
                _ => return Err(DbError::ExecutionError(
                    "REGEX_REPLACE: first argument must be a string".to_string(),
                )),
            };
            let pattern = match &args[1] {
                Value::String(s) => s.clone(),
                _ => return Err(DbError::ExecutionError(
                    "REGEX_REPLACE: second argument must be a string (pattern)".to_string(),
                )),
            };
            let replacement = match &args[2] {
                Value::String(s) => s.clone(),
                _ => return Err(DbError::ExecutionError(
                    "REGEX_REPLACE: third argument must be a string (replacement)".to_string(),
                )),
            };

            let re = regex::Regex::new(&pattern).map_err(|e| {
                DbError::ExecutionError(format!("REGEX_REPLACE: invalid pattern: {}", e))
            })?;

            let result = re.replace_all(&text, replacement).to_string();
            Ok(Value::String(result))
        }

        "CONTAINS" => {
            if args.len() != 2 && args.len() != 3 {
                return Err(DbError::ExecutionError(
                    "CONTAINS requires 2-3 arguments".to_string(),
                ));
            }
            let text = match &args[0] {
                Value::String(s) => s.clone(),
                Value::Null => return Ok(Value::Bool(false)),
                _ => return Err(DbError::ExecutionError(
                    "CONTAINS: first argument must be a string".to_string(),
                )),
            };
            let search = match &args[1] {
                Value::String(s) => s.clone(),
                _ => return Err(DbError::ExecutionError(
                    "CONTAINS: second argument must be a string".to_string(),
                )),
            };
            let case_insensitive = args.get(2).and_then(|v| v.as_bool()).unwrap_or(false);

            let result = if case_insensitive {
                text.to_lowercase().contains(&search.to_lowercase())
            } else {
                text.contains(&search)
            };
            Ok(Value::Bool(result))
        }

        "LENGTH" | "CHAR_LENGTH" | "CHARACTER_LENGTH" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "LENGTH requires 1 argument".to_string(),
                ));
            }
            match &args[0] {
                Value::String(s) => Ok(make_number(s.chars().count() as f64)),
                Value::Array(arr) => Ok(make_number(arr.len() as f64)),
                Value::Object(obj) => Ok(make_number(obj.len() as f64)),
                Value::Null => Ok(make_number(0.0)),
                _ => Ok(make_number(0.0)),
            }
        }

        "UPPER" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError("UPPER requires 1 argument".to_string()));
            }
            match &args[0] {
                Value::String(s) => Ok(Value::String(s.to_uppercase())),
                Value::Null => Ok(Value::Null),
                _ => Err(DbError::ExecutionError(
                    "UPPER: argument must be a string".to_string(),
                )),
            }
        }

        "LOWER" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError("LOWER requires 1 argument".to_string()));
            }
            match &args[0] {
                Value::String(s) => Ok(Value::String(s.to_lowercase())),
                Value::Null => Ok(Value::Null),
                _ => Err(DbError::ExecutionError(
                    "LOWER: argument must be a string".to_string(),
                )),
            }
        }

        "TRIM" | "LTRIM" | "RTRIM" => {
            if args.len() > 2 {
                return Err(DbError::ExecutionError(
                    "TRIM/LTRIM/RTRIM requires 1-2 arguments".to_string(),
                ));
            }
            let text = match &args[0] {
                Value::String(s) => s.clone(),
                Value::Null => return Ok(Value::Null),
                _ => return Err(DbError::ExecutionError(
                    "TRIM: first argument must be a string".to_string(),
                )),
            };
            let chars = match args.get(1) {
                Some(Value::String(s)) => s.chars().collect::<Vec<_>>(),
                Some(_) => return Err(DbError::ExecutionError(
                    "TRIM: second argument must be a string".to_string(),
                )),
                None => vec![' '],
            };

            let result = match name {
                "TRIM" => text.trim_matches(|c| chars.contains(&c)).to_string(),
                "LTRIM" => text.trim_start_matches(|c| chars.contains(&c)).to_string(),
                "RTRIM" => text.trim_end_matches(|c| chars.contains(&c)).to_string(),
                _ => text,
            };
            Ok(Value::String(result))
        }

        "SUBSTRING" | "SUBSTR" => {
            if args.len() != 2 && args.len() != 3 {
                return Err(DbError::ExecutionError(
                    "SUBSTRING requires 2-3 arguments".to_string(),
                ));
            }
            let text = match &args[0] {
                Value::String(s) => s.clone(),
                Value::Null => return Ok(Value::Null),
                _ => return Err(DbError::ExecutionError(
                    "SUBSTRING: first argument must be a string".to_string(),
                )),
            };
            let start = match &args[1] {
                Value::Number(n) => {
                    let idx = n.as_i64().unwrap_or(0);
                    if idx < 0 {
                        text.len() as i64 + idx
                    } else {
                        idx
                    }
                }
                _ => return Err(DbError::ExecutionError(
                    "SUBSTRING: second argument must be a number".to_string(),
                )),
            };
            let length = args.get(2).and_then(|v| v.as_u64()).map(|l| l as usize);

            let start = start.max(0) as usize;
            let result = if let Some(len) = length {
                if start < text.len() {
                    text[start..std::cmp::min(start + len, text.len())].to_string()
                } else {
                    String::new()
                }
            } else if start < text.len() {
                text[start..].to_string()
            } else {
                String::new()
            };
            Ok(Value::String(result))
        }

        "REPLACE" => {
            if args.len() != 3 {
                return Err(DbError::ExecutionError(
                    "REPLACE requires 3 arguments".to_string(),
                ));
            }
            let text = match &args[0] {
                Value::String(s) => s.clone(),
                Value::Null => return Ok(Value::Null),
                _ => return Err(DbError::ExecutionError(
                    "REPLACE: first argument must be a string".to_string(),
                )),
            };
            let search = match &args[1] {
                Value::String(s) => s.clone(),
                _ => return Err(DbError::ExecutionError(
                    "REPLACE: second argument must be a string".to_string(),
                )),
            };
            let replacement = match &args[2] {
                Value::String(s) => s.clone(),
                _ => return Err(DbError::ExecutionError(
                    "REPLACE: third argument must be a string".to_string(),
                )),
            };
            Ok(Value::String(text.replace(&search, &replacement)))
        }

        "REVERSE" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError("REVERSE requires 1 argument".to_string()));
            }
            match &args[0] {
                Value::String(s) => {
                    let rev: String = s.chars().rev().collect();
                    Ok(Value::String(rev))
                }
                Value::Null => Ok(Value::Null),
                _ => Err(DbError::ExecutionError(
                    "REVERSE: argument must be a string".to_string(),
                )),
            }
        }

        "SPLIT" => {
            if args.len() != 2 && args.len() != 3 {
                return Err(DbError::ExecutionError(
                    "SPLIT requires 2-3 arguments".to_string(),
                ));
            }
            let text = match &args[0] {
                Value::String(s) => s.clone(),
                Value::Null => return Ok(Value::Array(vec![])),
                _ => return Err(DbError::ExecutionError(
                    "SPLIT: first argument must be a string".to_string(),
                )),
            };
            let separator = match &args[1] {
                Value::String(s) => s.clone(),
                _ => return Err(DbError::ExecutionError(
                    "SPLIT: second argument must be a string".to_string(),
                )),
            };
            let max_parts = args.get(2).and_then(|v| v.as_u64()).map(|v| v as usize);

            let parts: Vec<Value> = if let Some(max) = max_parts {
                text.splitn(max, &separator).map(|s| Value::String(s.to_string())).collect()
            } else {
                text.split(&separator).map(|s| Value::String(s.to_string())).collect()
            };
            Ok(Value::Array(parts))
        }

        _ => Err(DbError::ExecutionError(format!(
            "Unknown string function: {}",
            name
        ))),
    }
}
