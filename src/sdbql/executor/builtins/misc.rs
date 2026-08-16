//! Miscellaneous utility functions for SDBQL.
//!
//! UUID, TYPEOF, COALESCE, etc.

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
                Value::Number(_) => "int",
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
        "RANGE" => {
            if args.is_empty() || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "RANGE requires 1-3 arguments: end or start, end, [step]".to_string(),
                ));
            }
            let get_i64 = |v: &Value| -> i64 {
                v.as_i64()
                    .unwrap_or_else(|| v.as_f64().unwrap_or(0.0) as i64)
            };
            let (start, end, step) = if args.len() == 1 {
                (0i64, get_i64(&args[0]), 1i64)
            } else if args.len() == 2 {
                (get_i64(&args[0]), get_i64(&args[1]), 1i64)
            } else {
                let step_val = get_i64(&args[2]);
                if step_val == 0 {
                    return Err(DbError::ExecutionError(
                        "RANGE: step cannot be 0".to_string(),
                    ));
                }
                (get_i64(&args[0]), get_i64(&args[1]), step_val)
            };

            let count = if step > 0 {
                if end < start {
                    0
                } else {
                    ((end - start) / step + 1) as usize
                }
            } else if end > start {
                0
            } else {
                ((start - end) / step.abs() + 1) as usize
            };
            const MAX_RANGE: usize = 1_000_000;
            if count > MAX_RANGE {
                return Err(DbError::ExecutionError(format!(
                    "RANGE: result would have {} elements (max {})",
                    count, MAX_RANGE
                )));
            }
            let mut result = Vec::with_capacity(count);
            let mut i = start;
            if step > 0 {
                while i <= end {
                    result.push(Value::Number(serde_json::Number::from(i)));
                    i += step;
                }
            } else {
                while i >= end {
                    result.push(Value::Number(serde_json::Number::from(i)));
                    i += step;
                }
            }
            Ok(Some(Value::Array(result)))
        }
        "TO_NUMBER" | "TO_NUM" => {
            check_args(name, args, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
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
                Value::Object(obj) => obj,
                _ => {
                    return Err(DbError::ExecutionError(
                        "KEEP: first argument must be an object".to_string(),
                    ));
                }
            };
            let mut result = serde_json::Map::new();
            for key in args[1..].iter().filter_map(Value::as_str) {
                if let Some(v) = obj.get(key) {
                    result.insert(key.to_string(), v.clone());
                }
            }
            Ok(Some(Value::Object(result)))
        }
        "UNSET" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "UNSET requires at least 2 arguments: object, key1, key2, ...".to_string(),
                ));
            }
            let obj = match &args[0] {
                Value::Object(obj) => obj,
                _ => {
                    return Err(DbError::ExecutionError(
                        "UNSET: first argument must be an object".to_string(),
                    ));
                }
            };
            let drop: std::collections::HashSet<&str> =
                args[1..].iter().filter_map(Value::as_str).collect();
            let mut result = serde_json::Map::new();
            for (k, v) in obj {
                if !drop.contains(k.as_str()) {
                    result.insert(k.clone(), v.clone());
                }
            }
            Ok(Some(Value::Object(result)))
        }
        "REDACT" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "REDACT requires 2 arguments: object, keys[]".to_string(),
                ));
            }
            let keys: Vec<String> = match &args[1] {
                Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                Value::String(s) => vec![s.clone()],
                _ => {
                    return Err(DbError::ExecutionError(
                        "REDACT: keys must be an array or string".to_string(),
                    ))
                }
            };
            Ok(Some(redact_value(&args[0], &keys)))
        }
        "PARSE_IDENTIFIER" => {
            check_args(name, args, 1)?;
            let s = args[0].as_str().unwrap_or("");
            Ok(Some(parse_ident(s)))
        }
        "PARSE_COLLECTION" => {
            check_args(name, args, 1)?;
            let s = args[0].as_str().unwrap_or("");
            Ok(Some(parse_ident(s).get("collection").cloned().unwrap_or(Value::Null)))
        }
        "PARSE_KEY" => {
            check_args(name, args, 1)?;
            let s = args[0].as_str().unwrap_or("");
            Ok(Some(parse_ident(s).get("key").cloned().unwrap_or(Value::Null)))
        }
        "UNSET_RECURSIVE" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "UNSET_RECURSIVE requires object, keys...".to_string(),
                ));
            }
            let keys: Vec<String> = args[1..]
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            Ok(Some(redact_value(&args[0], &keys)))
        }
        "KEEP_RECURSIVE" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "KEEP_RECURSIVE requires object, keys...".to_string(),
                ));
            }
            let keys: Vec<String> = args[1..]
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            Ok(Some(keep_recursive(&args[0], &keys)))
        }
        "GET" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "GET requires 2-3 arguments: object, path, [default]".to_string(),
                ));
            }
            if args[0].is_null() {
                return Ok(Some(args.get(2).cloned().unwrap_or(Value::Null)));
            }
            let path = args[1]
                .as_str()
                .ok_or_else(|| DbError::ExecutionError("GET: path must be a string".to_string()))?;
            if !path.contains('.') {
                let found = match &args[0] {
                    Value::Object(obj) => obj.get(path),
                    Value::Array(arr) => path.parse::<usize>().ok().and_then(|i| arr.get(i)),
                    _ => None,
                };
                return Ok(Some(
                    found
                        .cloned()
                        .unwrap_or_else(|| args.get(2).cloned().unwrap_or(Value::Null)),
                ));
            }
            let mut cur = &args[0];
            for part in path.split('.').filter(|p| !p.is_empty()) {
                cur = match cur {
                    Value::Object(obj) => match obj.get(part) {
                        Some(v) => v,
                        None => return Ok(Some(args.get(2).cloned().unwrap_or(Value::Null))),
                    },
                    Value::Array(arr) => {
                        match part.parse::<usize>().ok().and_then(|i| arr.get(i)) {
                            Some(v) => v,
                            None => return Ok(Some(args.get(2).cloned().unwrap_or(Value::Null))),
                        }
                    }
                    _ => return Ok(Some(args.get(2).cloned().unwrap_or(Value::Null))),
                };
            }
            Ok(Some(cur.clone()))
        }
        "DEEP_MERGE" => {
            if args.is_empty() {
                return Err(DbError::ExecutionError(
                    "DEEP_MERGE requires at least 1 argument".to_string(),
                ));
            }
            let mut result = Value::Object(serde_json::Map::new());
            for arg in args {
                match arg {
                    Value::Null => {}
                    Value::Object(_) => deep_merge_into(&mut result, arg),
                    _ => {
                        return Err(DbError::ExecutionError(
                            "DEEP_MERGE: all arguments must be objects".to_string(),
                        ));
                    }
                }
            }
            Ok(Some(result))
        }
        "ENTRIES" => {
            check_args(name, args, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let obj = args[0].as_object().ok_or_else(|| {
                DbError::ExecutionError("ENTRIES: argument must be an object".to_string())
            })?;
            let pairs: Vec<Value> = obj
                .iter()
                .map(|(k, v)| Value::Array(vec![Value::String(k.clone()), v.clone()]))
                .collect();
            Ok(Some(Value::Array(pairs)))
        }
        "FROM_ENTRIES" => {
            check_args(name, args, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError(
                    "FROM_ENTRIES: argument must be an array of pairs".to_string(),
                )
            })?;
            let mut obj = serde_json::Map::new();
            for item in arr {
                let pair = item.as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "FROM_ENTRIES: each item must be [key, value]".to_string(),
                    )
                })?;
                let key = pair.first().and_then(Value::as_str).ok_or_else(|| {
                    DbError::ExecutionError("FROM_ENTRIES: key must be a string".to_string())
                })?;
                let val = pair.get(1).cloned().unwrap_or(Value::Null);
                obj.insert(key.to_string(), val);
            }
            Ok(Some(Value::Object(obj)))
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

fn deep_merge_into(dst: &mut Value, src: &Value) {
    match (dst, src) {
        (Value::Object(d), Value::Object(s)) => {
            for (k, v) in s {
                match d.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => {
                        deep_merge_into(existing, v);
                    }
                    _ => {
                        d.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (d, s) => *d = s.clone(),
    }
}

fn parse_ident(id: &str) -> Value {
    match id.split_once('/') {
        Some((c, k)) => serde_json::json!({ "collection": c, "key": k }),
        None => serde_json::json!({ "collection": Value::Null, "key": id }),
    }
}

fn keep_recursive(v: &Value, keys: &[String]) -> Value {
    match v {
        Value::Object(o) => {
            let mut out = serde_json::Map::new();
            for (k, val) in o {
                if keys.iter().any(|kk| kk == k) {
                    out.insert(k.clone(), keep_recursive(val, keys));
                } else if val.is_object() || val.is_array() {
                    let child = keep_recursive(val, keys);
                    let keep = match &child {
                        Value::Object(m) => !m.is_empty(),
                        Value::Array(a) => !a.is_empty(),
                        _ => false,
                    };
                    if keep {
                        out.insert(k.clone(), child);
                    }
                }
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|x| keep_recursive(x, keys))
                .filter(|x| !x.is_null() && x != &json_empty())
                .collect(),
        ),
        other => other.clone(),
    }
}

fn json_empty() -> Value {
    Value::Object(serde_json::Map::new())
}

fn redact_value(v: &Value, keys: &[String]) -> Value {
    match v {
        Value::Object(o) => {
            let mut out = serde_json::Map::new();
            for (k, val) in o {
                if keys.iter().any(|dk| dk == k) {
                    continue;
                }
                out.insert(k.clone(), redact_value(val, keys));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| redact_value(x, keys)).collect()),
        other => other.clone(),
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
