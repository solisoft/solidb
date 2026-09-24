//! JSON and document builtin functions.

use serde_json::{Map, Value};

use super::common::{check_arity, err};
use crate::error::{SdbqlError, SdbqlResult};

/// Key arguments of KEEP / UNSET and their recursive forms: each argument
/// is a string or an array of strings (the AQL array form must not be
/// silently dropped — that is how `UNSET(doc, ["password"])` leaked).
fn key_list(name: &str, args: &[Value]) -> SdbqlResult<Vec<String>> {
    let mut keys = Vec::new();
    for arg in args {
        match arg {
            Value::String(s) => keys.push(s.clone()),
            Value::Array(items) => {
                for item in items {
                    match item {
                        Value::String(s) => keys.push(s.clone()),
                        _ => return Err(err(format!("{}: attribute names must be strings", name))),
                    }
                }
            }
            _ => {
                return Err(err(format!(
                    "{}: attribute names must be strings or arrays of strings",
                    name
                )))
            }
        }
    }
    Ok(keys)
}

/// Object argument, with null passed through as `Ok(None)`.
fn object_or_null<'a>(name: &str, v: &'a Value) -> SdbqlResult<Option<&'a Map<String, Value>>> {
    match v {
        Value::Null => Ok(None),
        Value::Object(o) => Ok(Some(o)),
        _ => Err(err(format!("{}: argument must be an object", name))),
    }
}

/// The objects an ATTRIBUTES / KEYS / VALUES call works on: one object, or
/// every object of an array (as on the server).
fn documents<'a>(name: &str, v: &'a Value) -> SdbqlResult<Option<Vec<&'a Map<String, Value>>>> {
    match v {
        Value::Null => Ok(None),
        Value::Object(o) => Ok(Some(vec![o])),
        Value::Array(items) => Ok(Some(items.iter().filter_map(Value::as_object).collect())),
        _ => Err(err(format!(
            "{}: argument must be an object or an array of objects",
            name
        ))),
    }
}

fn deep_merge_into(dst: &mut Value, src: &Value) {
    match (dst, src) {
        (Value::Object(d), Value::Object(s)) => {
            for (k, v) in s {
                match d.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => {
                        deep_merge_into(existing, v)
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

fn redact_value(v: &Value, keys: &[String]) -> Value {
    match v {
        Value::Object(o) => Value::Object(
            o.iter()
                .filter(|(k, _)| !keys.iter().any(|dk| dk == *k))
                .map(|(k, val)| (k.clone(), redact_value(val, keys)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|x| redact_value(x, keys)).collect()),
        other => other.clone(),
    }
}

fn keep_recursive(v: &Value, keys: &[String]) -> Value {
    match v {
        Value::Object(o) => {
            let mut out = Map::new();
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
                .filter(|x| !x.is_null() && x.as_object().is_none_or(|m| !m.is_empty()))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// GET(doc, "a.b.0.c", default): object keys and array indexes.
fn get_path(root: &Value, path: &str, default: &Value) -> Value {
    let mut cur = root;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        let next = match cur {
            Value::Object(obj) => obj.get(part),
            Value::Array(arr) => part.parse::<usize>().ok().and_then(|i| arr.get(i)),
            _ => None,
        };
        match next {
            Some(v) => cur = v,
            None => return default.clone(),
        }
    }
    cur.clone()
}

/// Call a JSON function. Returns None if function not found.
pub fn call(name: &str, args: &[Value]) -> SdbqlResult<Option<Value>> {
    let result = match name {
        "PARSE_IDENTIFIER" => {
            let s = args.first().and_then(Value::as_str).unwrap_or("");
            Some(match s.split_once('/') {
                Some((c, k)) => serde_json::json!({ "collection": c, "key": k }),
                None => serde_json::json!({ "collection": Value::Null, "key": s }),
            })
        }
        "PARSE_COLLECTION" => {
            let s = args.first().and_then(Value::as_str).unwrap_or("");
            Some(
                s.split_once('/')
                    .map(|(c, _)| Value::String(c.to_string()))
                    .unwrap_or(Value::Null),
            )
        }
        "PARSE_KEY" => {
            let s = args.first().and_then(Value::as_str).unwrap_or("");
            Some(
                s.split_once('/')
                    .map(|(_, k)| Value::String(k.to_string()))
                    .unwrap_or(Value::String(s.to_string())),
            )
        }
        "JSON_PARSE" | "PARSE_JSON" => {
            // The docs: null on invalid input rather than an error.
            check_args(name, args, 1)?;
            Some(
                args[0]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(Value::Null),
            )
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

        "KEYS" | "ATTRIBUTES" => {
            // ATTRIBUTES(doc, removeInternal, sort)
            check_arity(name, args, 1, 3)?;
            let Some(docs) = documents(name, &args[0])? else {
                return Ok(Some(Value::Null));
            };
            let remove_internal = args.get(1).and_then(Value::as_bool).unwrap_or(false);
            let sort = args.get(2).and_then(Value::as_bool).unwrap_or(false);
            let mut keys: Vec<String> = docs
                .iter()
                .flat_map(|o| o.keys())
                .filter(|k| !(remove_internal && k.starts_with('_')))
                .cloned()
                .collect();
            if sort {
                keys.sort();
            }
            Some(Value::Array(keys.into_iter().map(Value::String).collect()))
        }

        "VALUES" => {
            // VALUES(doc, removeInternal)
            check_arity(name, args, 1, 2)?;
            let Some(docs) = documents(name, &args[0])? else {
                return Ok(Some(Value::Null));
            };
            let remove_internal = args.get(1).and_then(Value::as_bool).unwrap_or(false);
            Some(Value::Array(
                docs.iter()
                    .flat_map(|o| o.iter())
                    .filter(|(k, _)| !(remove_internal && k.starts_with('_')))
                    .map(|(_, v)| v.clone())
                    .collect(),
            ))
        }

        "ENTRIES" => {
            check_args(name, args, 1)?;
            match object_or_null(name, &args[0])? {
                None => Some(Value::Null),
                Some(obj) => Some(Value::Array(
                    obj.iter()
                        .map(|(k, v)| Value::Array(vec![Value::String(k.clone()), v.clone()]))
                        .collect(),
                )),
            }
        }

        "FROM_ENTRIES" => {
            check_args(name, args, 1)?;
            match &args[0] {
                Value::Null => Some(Value::Null),
                Value::Array(arr) => {
                    let mut obj = Map::new();
                    for item in arr {
                        let pair = item
                            .as_array()
                            .ok_or_else(|| err("FROM_ENTRIES: each item must be [key, value]"))?;
                        let key = pair
                            .first()
                            .and_then(Value::as_str)
                            .ok_or_else(|| err("FROM_ENTRIES: key must be a string"))?;
                        obj.insert(key.to_string(), pair.get(1).cloned().unwrap_or(Value::Null));
                    }
                    Some(Value::Object(obj))
                }
                _ => return Err(err("FROM_ENTRIES: argument must be an array of pairs")),
            }
        }

        "MERGE" | "MERGE_OBJECTS" => {
            // MERGE(doc1, doc2, ...) or MERGE([doc1, doc2, ...]). Nulls are
            // skipped; any other non-object is an error, not silently dropped.
            let items: &[Value] = match args {
                [Value::Array(list)] => list.as_slice(),
                _ => args,
            };
            let mut result = Map::new();
            for arg in items {
                match arg {
                    Value::Null => {}
                    Value::Object(obj) => {
                        for (k, v) in obj {
                            result.insert(k.clone(), v.clone());
                        }
                    }
                    _ => return Err(err(format!("{}: all arguments must be objects", name))),
                }
            }
            Some(Value::Object(result))
        }

        "DEEP_MERGE" | "MERGE_DEEP" | "MERGE_RECURSIVE" => {
            let mut result = Value::Object(Map::new());
            for arg in args {
                match arg {
                    Value::Null => {}
                    Value::Object(_) => deep_merge_into(&mut result, arg),
                    _ => return Err(err(format!("{}: all arguments must be objects", name))),
                }
            }
            Some(result)
        }

        "HAS" | "HAS_KEY" => {
            if args.len() != 2 {
                return Err(SdbqlError::ExecutionError(
                    "HAS requires 2 arguments: object, key".to_string(),
                ));
            }
            let key = args[1]
                .as_str()
                .ok_or_else(|| err("HAS: second argument must be a string"))?;
            let has = match &args[0] {
                Value::Object(obj) => obj.contains_key(key),
                // As on the server: true if any object of the array has it.
                Value::Array(arr) => arr
                    .iter()
                    .any(|item| item.as_object().is_some_and(|o| o.contains_key(key))),
                _ => false,
            };
            Some(Value::Bool(has))
        }

        "UNSET" | "WITHOUT" => {
            check_arity(name, args, 2, usize::MAX)?;
            let keys = key_list(name, &args[1..])?;
            match object_or_null(name, &args[0])? {
                None => Some(Value::Null),
                Some(obj) => Some(Value::Object(
                    obj.iter()
                        .filter(|(k, _)| !keys.contains(k))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                )),
            }
        }

        "KEEP" => {
            check_arity(name, args, 2, usize::MAX)?;
            let keys = key_list(name, &args[1..])?;
            match object_or_null(name, &args[0])? {
                None => Some(Value::Null),
                Some(obj) => Some(Value::Object(
                    obj.iter()
                        .filter(|(k, _)| keys.contains(k))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                )),
            }
        }

        "UNSET_RECURSIVE" | "KEEP_RECURSIVE" => {
            check_arity(name, args, 2, usize::MAX)?;
            let keys = key_list(name, &args[1..])?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            if name == "UNSET_RECURSIVE" {
                Some(redact_value(&args[0], &keys))
            } else {
                Some(keep_recursive(&args[0], &keys))
            }
        }

        "REDACT" => {
            check_arity(name, args, 2, 2)?;
            let keys = key_list(name, &args[1..])?;
            Some(redact_value(&args[0], &keys))
        }

        "GET" => {
            check_arity(name, args, 2, 3)?;
            let default = args.get(2).cloned().unwrap_or(Value::Null);
            if args[0].is_null() {
                return Ok(Some(default));
            }
            let path = args[1]
                .as_str()
                .ok_or_else(|| err("GET: path must be a string"))?;
            Some(get_path(&args[0], path, &default))
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
                    let mut obj = Map::new();
                    for (key, val) in k.iter().zip(v.iter()) {
                        if let Value::String(key_str) = key {
                            obj.insert(key_str.clone(), val.clone());
                        }
                    }
                    Some(Value::Object(obj))
                }
                _ => Some(Value::Object(Map::new())),
            }
        }

        _ => None,
    };

    Ok(result)
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
        assert_eq!(
            call("JSON_PARSE", &[json!("{nope")]).unwrap(),
            Some(Value::Null)
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
    fn keys_on_non_object_is_an_error() {
        assert!(call("KEYS", &[json!(5)]).is_err());
        assert!(call("VALUES", &[json!("x")]).is_err());
        assert_eq!(call("KEYS", &[Value::Null]).unwrap(), Some(Value::Null));
        assert_eq!(
            call(
                "ATTRIBUTES",
                &[json!({"_key": 1, "b": 2, "a": 3}), json!(true), json!(true)]
            )
            .unwrap(),
            Some(json!(["a", "b"]))
        );
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
    fn merge_rejects_non_objects() {
        assert!(call("MERGE", &[json!({"a": 1}), json!(5)]).is_err());
        assert!(call("DEEP_MERGE", &[json!({"a": 1}), json!("x")]).is_err());
        assert_eq!(
            call("MERGE", &[json!([{"a": 1}, {"b": 2}])]).unwrap(),
            Some(json!({"a": 1, "b": 2}))
        );
        assert_eq!(
            call("MERGE", &[Value::Null, json!({"a": 1})]).unwrap(),
            Some(json!({"a": 1}))
        );
    }

    #[test]
    fn test_merge_deep() {
        let obj1 = json!({"a": {"x": 1}, "b": 2});
        let obj2 = json!({"a": {"y": 2}, "c": 3});
        let merged = call("MERGE_DEEP", &[obj1.clone(), obj2.clone()])
            .unwrap()
            .unwrap();
        assert_eq!(merged, json!({"a": {"x": 1, "y": 2}, "b": 2, "c": 3}));
        assert_eq!(call("DEEP_MERGE", &[obj1, obj2]).unwrap(), Some(merged));
    }

    #[test]
    fn test_has() {
        let obj = json!({"a": 1});
        assert_eq!(
            call("HAS", &[obj.clone(), json!("a")]).unwrap(),
            Some(json!(true))
        );
        assert_eq!(call("HAS", &[obj, json!("b")]).unwrap(), Some(json!(false)));
        assert_eq!(
            call("HAS", &[json!([{"x": 1}, {"a": 1}]), json!("a")]).unwrap(),
            Some(json!(true))
        );
        assert_eq!(
            call("HAS", &[json!([1, 2]), json!("a")]).unwrap(),
            Some(json!(false))
        );
    }

    #[test]
    fn test_unset_keep() {
        let obj = json!({"a": 1, "b": 2, "c": 3});
        assert_eq!(
            call("UNSET", &[obj.clone(), json!("b")]).unwrap(),
            Some(json!({"a": 1, "c": 3}))
        );
        assert_eq!(
            call("KEEP", &[obj.clone(), json!("a"), json!("c")]).unwrap(),
            Some(json!({"a": 1, "c": 3}))
        );
        assert_eq!(
            call("UNSET", &[obj.clone(), json!(["a", "b"])]).unwrap(),
            Some(json!({"c": 3}))
        );
        assert!(call("UNSET", &[obj, json!(5)]).is_err());
    }

    #[test]
    fn get_redact_recursive() {
        let doc = json!({"user": {"name": "Ada", "tags": ["x", "y"]}, "ssn": 1});
        assert_eq!(
            call("GET", &[doc.clone(), json!("user.name")]).unwrap(),
            Some(json!("Ada"))
        );
        assert_eq!(
            call("GET", &[doc.clone(), json!("user.tags.1")]).unwrap(),
            Some(json!("y"))
        );
        assert_eq!(
            call("GET", &[doc, json!("user.missing"), json!("N/A")]).unwrap(),
            Some(json!("N/A"))
        );
        assert_eq!(
            call(
                "REDACT",
                &[json!({"a": {"ssn": 1, "b": 2}, "ssn": 3}), json!(["ssn"])]
            )
            .unwrap(),
            Some(json!({"a": {"b": 2}}))
        );
        assert_eq!(
            call(
                "KEEP_RECURSIVE",
                &[
                    json!({"user": {"name": "Ada", "age": 3}, "ssn": 1}),
                    json!("name")
                ]
            )
            .unwrap(),
            Some(json!({"user": {"name": "Ada"}}))
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
