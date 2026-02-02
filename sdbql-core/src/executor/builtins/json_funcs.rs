//! JSON builtin functions.

use serde_json::Value;

use crate::error::{SdbqlError, SdbqlResult};

/// Call a JSON function. Returns None if function not found.
pub fn call(name: &str, args: &[Value]) -> SdbqlResult<Option<Value>> {
    let result = match name {
        "JSON_PARSE" | "PARSE_JSON" => {
            check_args(name, args, 1)?;
            let s = args[0].as_str().ok_or_else(|| {
                SdbqlError::ExecutionError("JSON_PARSE: argument must be a string".to_string())
            })?;
            let parsed: Value = serde_json::from_str(s).map_err(|e| {
                SdbqlError::ExecutionError(format!("JSON_PARSE: invalid JSON: {}", e))
            })?;
            Some(parsed)
        }

        "JSON_STRINGIFY" | "TO_JSON" => {
            check_args(name, args, 1)?;
            let s = serde_json::to_string(&args[0])
                .map_err(|e| SdbqlError::ExecutionError(format!("JSON_STRINGIFY: {}", e)))?;
            Some(Value::String(s))
        }

        "JSON_STRINGIFY_PRETTY" => {
            check_args(name, args, 1)?;
            let s = serde_json::to_string_pretty(&args[0])
                .map_err(|e| SdbqlError::ExecutionError(format!("JSON_STRINGIFY_PRETTY: {}", e)))?;
            Some(Value::String(s))
        }

        "KEYS" => {
            check_args(name, args, 1)?;
            match &args[0] {
                Value::Object(obj) => {
                    let keys: Vec<Value> = obj.keys().map(|k| Value::String(k.clone())).collect();
                    Some(Value::Array(keys))
                }
                _ => Some(Value::Array(vec![])),
            }
        }

        "VALUES" => {
            check_args(name, args, 1)?;
            match &args[0] {
                Value::Object(obj) => {
                    let values: Vec<Value> = obj.values().cloned().collect();
                    Some(Value::Array(values))
                }
                _ => Some(Value::Array(vec![])),
            }
        }

        "ENTRIES" => {
            check_args(name, args, 1)?;
            match &args[0] {
                Value::Object(obj) => {
                    let entries: Vec<Value> = obj
                        .iter()
                        .map(|(k, v)| Value::Array(vec![Value::String(k.clone()), v.clone()]))
                        .collect();
                    Some(Value::Array(entries))
                }
                _ => Some(Value::Array(vec![])),
            }
        }

        "FROM_ENTRIES" => {
            check_args(name, args, 1)?;
            match &args[0] {
                Value::Array(arr) => {
                    let mut obj = serde_json::Map::new();
                    for item in arr {
                        if let Value::Array(pair) = item {
                            if pair.len() >= 2 {
                                if let Value::String(key) = &pair[0] {
                                    obj.insert(key.clone(), pair[1].clone());
                                }
                            }
                        }
                    }
                    Some(Value::Object(obj))
                }
                _ => Some(Value::Object(serde_json::Map::new())),
            }
        }

        "MERGE" | "MERGE_OBJECTS" => {
            let mut result = serde_json::Map::new();
            for arg in args {
                if let Value::Object(obj) = arg {
                    for (k, v) in obj {
                        result.insert(k.clone(), v.clone());
                    }
                }
            }
            Some(Value::Object(result))
        }

        "MERGE_DEEP" | "MERGE_RECURSIVE" => {
            if args.is_empty() {
                return Ok(Some(Value::Object(serde_json::Map::new())));
            }
            let mut result = args[0].clone();
            for arg in args.iter().skip(1) {
                result = deep_merge(&result, arg);
            }
            Some(result)
        }

        "HAS" | "HAS_KEY" => {
            if args.len() != 2 {
                return Err(SdbqlError::ExecutionError(
                    "HAS requires 2 arguments: object, key".to_string(),
                ));
            }
            let key = args[1].as_str().unwrap_or("");
            let has = match &args[0] {
                Value::Object(obj) => obj.contains_key(key),
                _ => false,
            };
            Some(Value::Bool(has))
        }

        "UNSET" | "WITHOUT" => {
            if args.len() < 2 {
                return Err(SdbqlError::ExecutionError(
                    "UNSET requires at least 2 arguments: object, key(s)".to_string(),
                ));
            }
            match &args[0] {
                Value::Object(obj) => {
                    let mut result = obj.clone();
                    for arg in args.iter().skip(1) {
                        if let Value::String(key) = arg {
                            result.remove(key);
                        } else if let Value::Array(keys) = arg {
                            for k in keys {
                                if let Value::String(key) = k {
                                    result.remove(key);
                                }
                            }
                        }
                    }
                    Some(Value::Object(result))
                }
                _ => Some(args[0].clone()),
            }
        }

        "KEEP" => {
            if args.len() < 2 {
                return Err(SdbqlError::ExecutionError(
                    "KEEP requires at least 2 arguments: object, key(s)".to_string(),
                ));
            }
            match &args[0] {
                Value::Object(obj) => {
                    let mut keys_to_keep = std::collections::HashSet::new();
                    for arg in args.iter().skip(1) {
                        if let Value::String(key) = arg {
                            keys_to_keep.insert(key.clone());
                        } else if let Value::Array(keys) = arg {
                            for k in keys {
                                if let Value::String(key) = k {
                                    keys_to_keep.insert(key.clone());
                                }
                            }
                        }
                    }
                    let result: serde_json::Map<String, Value> = obj
                        .iter()
                        .filter(|(k, _)| keys_to_keep.contains(*k))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    Some(Value::Object(result))
                }
                _ => Some(args[0].clone()),
            }
        }

        "ZIP_OBJECTS" => {
            if args.len() != 2 {
                return Err(SdbqlError::ExecutionError(
                    "ZIP_OBJECTS requires 2 arguments: keys array, values array".to_string(),
                ));
            }
            let keys = args[0].as_array();
            let values = args[1].as_array();

            match (keys, values) {
                (Some(k), Some(v)) => {
                    let mut obj = serde_json::Map::new();
                    for (key, val) in k.iter().zip(v.iter()) {
                        if let Value::String(key_str) = key {
                            obj.insert(key_str.clone(), val.clone());
                        }
                    }
                    Some(Value::Object(obj))
                }
                _ => Some(Value::Object(serde_json::Map::new())),
            }
        }

        _ => None,
    };

    Ok(result)
}

fn deep_merge(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(base_obj), Value::Object(overlay_obj)) => {
            let mut result = base_obj.clone();
            for (key, overlay_val) in overlay_obj {
                let merged_val = if let Some(base_val) = base_obj.get(key) {
                    deep_merge(base_val, overlay_val)
                } else {
                    overlay_val.clone()
                };
                result.insert(key.clone(), merged_val);
            }
            Value::Object(result)
        }
        _ => overlay.clone(),
    }
}

fn check_args(name: &str, args: &[Value], expected: usize) -> SdbqlResult<()> {
    if args.len() != expected {
        return Err(SdbqlError::ExecutionError(format!(
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
    fn test_json_parse_stringify() {
        assert_eq!(
            call("JSON_PARSE", &[json!(r#"{"a":1}"#)]).unwrap(),
            Some(json!({"a": 1}))
        );
        assert_eq!(
            call("JSON_STRINGIFY", &[json!({"a": 1})]).unwrap(),
            Some(json!(r#"{"a":1}"#))
        );
    }

    #[test]
    fn test_keys_values() {
        let obj = json!({"a": 1, "b": 2});
        let keys = call("KEYS", &[obj.clone()]).unwrap().unwrap();
        assert!(keys.as_array().unwrap().contains(&json!("a")));
        assert!(keys.as_array().unwrap().contains(&json!("b")));

        let values = call("VALUES", &[obj]).unwrap().unwrap();
        assert!(values.as_array().unwrap().contains(&json!(1)));
        assert!(values.as_array().unwrap().contains(&json!(2)));
    }

    #[test]
    fn test_entries_from_entries() {
        let obj = json!({"a": 1, "b": 2});
        let entries = call("ENTRIES", &[obj]).unwrap().unwrap();
        assert!(entries.is_array());

        let reconstructed = call("FROM_ENTRIES", &[entries]).unwrap().unwrap();
        assert_eq!(reconstructed.get("a"), Some(&json!(1)));
        assert_eq!(reconstructed.get("b"), Some(&json!(2)));
    }

    #[test]
    fn test_merge() {
        let obj1 = json!({"a": 1, "b": 2});
        let obj2 = json!({"b": 3, "c": 4});
        let merged = call("MERGE", &[obj1, obj2]).unwrap().unwrap();
        assert_eq!(merged, json!({"a": 1, "b": 3, "c": 4}));
    }

    #[test]
    fn test_merge_deep() {
        let obj1 = json!({"a": {"x": 1}, "b": 2});
        let obj2 = json!({"a": {"y": 2}, "c": 3});
        let merged = call("MERGE_DEEP", &[obj1, obj2]).unwrap().unwrap();
        assert_eq!(merged, json!({"a": {"x": 1, "y": 2}, "b": 2, "c": 3}));
    }

    #[test]
    fn test_has() {
        let obj = json!({"a": 1});
        assert_eq!(
            call("HAS", &[obj.clone(), json!("a")]).unwrap(),
            Some(json!(true))
        );
        assert_eq!(call("HAS", &[obj, json!("b")]).unwrap(), Some(json!(false)));
    }

    #[test]
    fn test_unset_keep() {
        let obj = json!({"a": 1, "b": 2, "c": 3});
        assert_eq!(
            call("UNSET", &[obj.clone(), json!("b")]).unwrap(),
            Some(json!({"a": 1, "c": 3}))
        );
        assert_eq!(
            call("KEEP", &[obj, json!("a"), json!("c")]).unwrap(),
            Some(json!({"a": 1, "c": 3}))
        );
    }

    #[test]
    fn test_zip_objects() {
        let keys = json!(["a", "b", "c"]);
        let values = json!([1, 2, 3]);
        assert_eq!(
            call("ZIP_OBJECTS", &[keys, values]).unwrap(),
            Some(json!({"a": 1, "b": 2, "c": 3}))
        );
    }
}
