//! Array functions for SDBQL

use serde_json::Value;

use crate::error::{DbError, DbResult};
use crate::sdbql::executor::utils::number_from_f64;

#[inline]
fn make_number(f: f64) -> Value {
    Value::Number(number_from_f64(f))
}

/// Helper function to flatten arrays
fn flatten_arr(arr: &[Value], depth: usize) -> Vec<Value> {
    let mut result = Vec::new();
    for item in arr {
        if depth > 0 {
            if let Value::Array(inner) = item {
                result.extend(flatten_arr(inner, depth - 1));
            } else {
                result.push(item.clone());
            }
        } else {
            result.push(item.clone());
        }
    }
    result
}

/// Helper to compare JSON values for sorting
fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Null, _) => std::cmp::Ordering::Less,
        (_, Value::Null) => std::cmp::Ordering::Greater,
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Number(a), Value::Number(b)) => {
            let a_num = a.as_f64();
            let b_num = b.as_f64();
            a_num
                .partial_cmp(&b_num)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::Array(a), Value::Array(b)) => a.len().cmp(&b.len()),
        (Value::Object(a), Value::Object(b)) => a.len().cmp(&b.len()),
        _ => std::cmp::Ordering::Equal,
    }
}

/// Evaluate array functions
pub fn evaluate_array_fn(name: &str, args: &[Value]) -> DbResult<Value> {
    match name {
        "LENGTH" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "LENGTH requires 1 argument".to_string(),
                ));
            }
            match &args[0] {
                Value::Array(arr) => Ok(make_number(arr.len() as f64)),
                Value::String(s) => Ok(make_number(s.chars().count() as f64)),
                Value::Object(obj) => Ok(make_number(obj.len() as f64)),
                Value::Null => Ok(make_number(0.0)),
                _ => Ok(make_number(0.0)),
            }
        }

        "APPEND" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "APPEND requires at least 2 arguments".to_string(),
                ));
            }
            let mut arr = match &args[0] {
                Value::Array(a) => a.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "APPEND: first argument must be an array".to_string(),
                    ));
                }
            };
            for item in &args[1..] {
                arr.push(item.clone());
            }
            Ok(Value::Array(arr))
        }

        "PUSH" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "PUSH requires at least 2 arguments".to_string(),
                ));
            }
            let mut arr = match &args[0] {
                Value::Array(a) => a.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "PUSH: first argument must be an array".to_string(),
                    ));
                }
            };
            for item in &args[1..] {
                arr.push(item.clone());
            }
            Ok(Value::Array(arr))
        }

        "POP" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "POP requires 1 argument".to_string(),
                ));
            }
            let mut arr = match &args[0] {
                Value::Array(a) => a.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "POP: argument must be an array".to_string(),
                    ));
                }
            };
            arr.pop();
            Ok(Value::Array(arr))
        }

        "SHIFT" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "SHIFT requires 1 argument".to_string(),
                ));
            }
            let mut arr = match &args[0] {
                Value::Array(a) => a.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "SHIFT: argument must be an array".to_string(),
                    ));
                }
            };
            arr.remove(0);
            Ok(Value::Array(arr))
        }

        "UNSHIFT" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "UNSHIFT requires at least 2 arguments".to_string(),
                ));
            }
            let mut arr = match &args[0] {
                Value::Array(a) => a.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "UNSHIFT: first argument must be an array".to_string(),
                    ))
                }
            };
            let mut items = args[1..].to_vec();
            items.append(&mut arr);
            Ok(Value::Array(items))
        }

        "SLICE" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "SLICE requires 2-3 arguments".to_string(),
                ));
            }
            let arr = match &args[0] {
                Value::Array(a) => a.clone(),
                Value::String(s) => {
                    let chars: Vec<Value> =
                        s.chars().map(|c| Value::String(c.to_string())).collect();
                    chars
                }
                _ => {
                    return Err(DbError::ExecutionError(
                        "SLICE: first argument must be an array or string".to_string(),
                    ))
                }
            };
            let start = match &args[1] {
                Value::Number(n) => {
                    let idx = n.as_i64().unwrap_or(0);
                    if idx < 0 {
                        arr.len() as i64 + idx
                    } else {
                        idx
                    }
                }
                _ => {
                    return Err(DbError::ExecutionError(
                        "SLICE: second argument must be a number".to_string(),
                    ))
                }
            };
            let length = args.get(2).and_then(|v| v.as_u64()).map(|l| l as usize);

            let start = if start < 0 { 0 } else { start as usize };
            let slice = if let Some(len) = length {
                arr[start..std::cmp::min(start + len, arr.len())].to_vec()
            } else {
                arr[start..].to_vec()
            };
            Ok(Value::Array(slice))
        }

        "FLATTEN" => {
            if args.is_empty() || args.len() > 2 {
                return Err(DbError::ExecutionError(
                    "FLATTEN requires 1-2 arguments".to_string(),
                ));
            }
            let arr = match &args[0] {
                Value::Array(a) => a.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "FLATTEN: first argument must be an array".to_string(),
                    ))
                }
            };
            let depth = if args.len() > 1 {
                args[1].as_u64().unwrap_or(1) as usize
            } else {
                1
            };
            Ok(Value::Array(flatten_arr(&arr, depth)))
        }

        "UNIQUE" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "UNIQUE requires 1 argument".to_string(),
                ));
            }
            let arr = match &args[0] {
                Value::Array(a) => a.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "UNIQUE: argument must be an array".to_string(),
                    ))
                }
            };
            let mut seen = std::collections::HashSet::new();
            let unique: Vec<Value> = arr
                .iter()
                .filter(|v| seen.insert(v.to_string()))
                .cloned()
                .collect();
            Ok(Value::Array(unique))
        }

        "SORTED" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "SORTED requires 1 argument".to_string(),
                ));
            }
            let mut arr = match &args[0] {
                Value::Array(a) => a.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "SORTED: argument must be an array".to_string(),
                    ))
                }
            };
            arr.sort_by(compare_values);
            Ok(Value::Array(arr))
        }

        "UNION" => {
            let mut result = Vec::new();
            for arg in args {
                match arg {
                    Value::Array(arr) => {
                        for item in arr {
                            if !result.contains(item) {
                                result.push(item.clone());
                            }
                        }
                    }
                    _ => {
                        return Err(DbError::ExecutionError(
                            "UNION: all arguments must be arrays".to_string(),
                        ))
                    }
                }
            }
            Ok(Value::Array(result))
        }

        "INTERSECTION" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "INTERSECTION requires at least 2 arguments".to_string(),
                ));
            }
            let mut result = match &args[0] {
                Value::Array(a) => a.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "INTERSECTION: first argument must be an array".to_string(),
                    ))
                }
            };
            for arg in &args[1..] {
                let arr = match arg {
                    Value::Array(a) => a,
                    _ => {
                        return Err(DbError::ExecutionError(
                            "INTERSECTION: all arguments must be arrays".to_string(),
                        ))
                    }
                };
                result.retain(|item| arr.contains(item));
            }
            Ok(Value::Array(result))
        }

        "DIFFERENCE" | "MINUS" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "DIFFERENCE requires 2 arguments".to_string(),
                ));
            }
            let arr1 = match &args[0] {
                Value::Array(a) => a.clone(),
                _ => {
                    return Err(DbError::ExecutionError(
                        "DIFFERENCE: first argument must be an array".to_string(),
                    ))
                }
            };
            let arr2 = match &args[1] {
                Value::Array(a) => a,
                _ => {
                    return Err(DbError::ExecutionError(
                        "DIFFERENCE: second argument must be an array".to_string(),
                    ))
                }
            };
            let result: Vec<Value> = arr1
                .into_iter()
                .filter(|item| !arr2.contains(item))
                .collect();
            Ok(Value::Array(result))
        }

        "NTH" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "NTH requires 2 arguments".to_string(),
                ));
            }
            let arr = match &args[0] {
                Value::Array(a) => a,
                _ => {
                    return Err(DbError::ExecutionError(
                        "NTH: first argument must be an array".to_string(),
                    ))
                }
            };
            let index = match &args[1] {
                Value::Number(n) => n.as_i64().unwrap_or(0),
                _ => {
                    return Err(DbError::ExecutionError(
                        "NTH: second argument must be a number".to_string(),
                    ))
                }
            };
            let index = if index < 0 {
                arr.len() as i64 + index
            } else {
                index
            };
            if index >= 0 && index < arr.len() as i64 {
                Ok(arr[index as usize].clone())
            } else {
                Ok(Value::Null)
            }
        }

        _ => Err(DbError::ExecutionError(format!(
            "Unknown array function: {}",
            name
        ))),
    }
}
