//! Miscellaneous utility functions for SDBQL.
//!
//! TYPEOF, COALESCE, type casts, object helpers (KEEP, UNSET, MERGE, …).
//! UUIDs live in `phonetic/id.rs`, which is dispatched first.

use crate::error::{DbError, DbResult};
use crate::sdbql::executor::helpers::{to_bool, values_equal};
use serde_json::{Map, Value};

/// Same ceiling RANGE has always had.
const MAX_RANGE: usize = 1_000_000;

/// Evaluate misc functions
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
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
            // values_equal: NULLIF(1, 1.0) is null, like `1 == 1.0`.
            if values_equal(&args[0], &args[1]) {
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
        "RANGE" => range(args).map(Some),
        "TO_NUMBER" | "TO_NUM" => {
            check_args(name, args, 1)?;
            Ok(Some(to_number(&args[0])))
        }
        "TO_STRING" | "TO_STR" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::String(to_string_value(&args[0]))))
        }
        "TO_BOOL" | "TO_BOOLEAN" => {
            check_args(name, args, 1)?;
            // One truthiness rule for TO_BOOL, FILTER, `!` and `? :`.
            Ok(Some(Value::Bool(to_bool(&args[0]))))
        }
        "TO_ARRAY" | "TO_LIST" => {
            check_args(name, args, 1)?;
            Ok(Some(match &args[0] {
                Value::Array(arr) => Value::Array(arr.clone()),
                Value::Null => Value::Array(vec![]),
                // AQL: an object becomes the array of its values.
                Value::Object(o) => Value::Array(o.values().cloned().collect()),
                other => Value::Array(vec![other.clone()]),
            }))
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
            if args.is_empty() || args.len() > 3 {
                return Err(DbError::ExecutionError(format!(
                    "{name} requires 1-3 arguments: document, [removeInternal], [sort]"
                )));
            }
            let remove_internal = opt_flag(name, "removeInternal", args.get(1))?;
            let sort = opt_flag(name, "sort", args.get(2))?;
            let keep = |k: &str| !(remove_internal && k.starts_with('_'));
            let mut keys: Vec<Value> = match &args[0] {
                Value::Null => return Ok(Some(Value::Null)),
                Value::Object(obj) => obj
                    .keys()
                    .filter(|k| keep(k.as_str()))
                    .map(|k| Value::String(k.clone()))
                    .collect(),
                Value::Array(arr) => {
                    let mut keys = Vec::new();
                    for item in arr {
                        if let Value::Object(obj) = item {
                            keys.extend(
                                obj.keys()
                                    .filter(|k| keep(k.as_str()))
                                    .map(|k| Value::String(k.clone())),
                            );
                        }
                    }
                    keys
                }
                _ => {
                    return Err(DbError::ExecutionError(format!(
                        "{name}: argument must be an object or array of objects"
                    )));
                }
            };
            if sort {
                keys.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
            }
            Ok(Some(Value::Array(keys)))
        }
        "VALUES" => {
            if args.is_empty() || args.len() > 2 {
                return Err(DbError::ExecutionError(
                    "VALUES requires 1-2 arguments: document, [removeInternal]".to_string(),
                ));
            }
            let remove_internal = opt_flag(name, "removeInternal", args.get(1))?;
            let pick = |obj: &Map<String, Value>, out: &mut Vec<Value>| {
                out.extend(
                    obj.iter()
                        .filter(|(k, _)| !(remove_internal && k.starts_with('_')))
                        .map(|(_, v)| v.clone()),
                );
            };
            let mut values = Vec::new();
            match &args[0] {
                Value::Null => return Ok(Some(Value::Null)),
                Value::Object(obj) => pick(obj, &mut values),
                Value::Array(arr) => {
                    for item in arr {
                        if let Value::Object(obj) = item {
                            pick(obj, &mut values);
                        }
                    }
                }
                _ => {
                    return Err(DbError::ExecutionError(
                        "VALUES: argument must be an object or array of objects".to_string(),
                    ));
                }
            }
            Ok(Some(Value::Array(values)))
        }
        "KEEP" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "KEEP requires at least 2 arguments: object, key1, key2, ... or object, [keys]"
                        .to_string(),
                ));
            }
            let obj = match &args[0] {
                Value::Null => return Ok(Some(Value::Null)),
                Value::Object(obj) => obj,
                _ => {
                    return Err(DbError::ExecutionError(
                        "KEEP: first argument must be an object".to_string(),
                    ));
                }
            };
            let mut result = Map::new();
            for key in key_args("KEEP", &args[1..])? {
                if let Some(v) = obj.get(key) {
                    result.insert(key.to_string(), v.clone());
                }
            }
            Ok(Some(Value::Object(result)))
        }
        "UNSET" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "UNSET requires at least 2 arguments: object, key1, key2, ... or object, [keys]"
                        .to_string(),
                ));
            }
            let obj = match &args[0] {
                Value::Null => return Ok(Some(Value::Null)),
                Value::Object(obj) => obj,
                _ => {
                    return Err(DbError::ExecutionError(
                        "UNSET: first argument must be an object".to_string(),
                    ));
                }
            };
            let drop: std::collections::HashSet<&str> =
                key_args("UNSET", &args[1..])?.into_iter().collect();
            let result: Map<String, Value> = obj
                .iter()
                .filter(|(k, _)| !drop.contains(k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Ok(Some(Value::Object(result)))
        }
        "REDACT" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "REDACT requires 2 arguments: object, keys[]".to_string(),
                ));
            }
            let keys: Vec<String> = match &args[1] {
                Value::Array(a) => a
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect(),
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
            Ok(Some(match identifier_of(&args[0]) {
                Some(s) => parse_ident(s),
                None => Value::Null,
            }))
        }
        "PARSE_COLLECTION" => {
            check_args(name, args, 1)?;
            Ok(Some(match identifier_of(&args[0]) {
                Some(s) => parse_ident(s)
                    .get("collection")
                    .cloned()
                    .unwrap_or(Value::Null),
                None => Value::Null,
            }))
        }
        "PARSE_KEY" => {
            check_args(name, args, 1)?;
            Ok(Some(match identifier_of(&args[0]) {
                Some(s) => parse_ident(s).get("key").cloned().unwrap_or(Value::Null),
                None => Value::Null,
            }))
        }
        "UNSET_RECURSIVE" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "UNSET_RECURSIVE requires object, keys... or object, [keys]".to_string(),
                ));
            }
            let keys: Vec<String> = key_args(name, &args[1..])?
                .into_iter()
                .map(str::to_string)
                .collect();
            Ok(Some(redact_value(&args[0], &keys)))
        }
        "KEEP_RECURSIVE" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "KEEP_RECURSIVE requires object, keys... or object, [keys]".to_string(),
                ));
            }
            let keys: std::collections::HashSet<&str> =
                key_args(name, &args[1..])?.into_iter().collect();
            Ok(Some(match &args[0] {
                Value::Object(o) => Value::Object(keep_recursive_obj(o, &keys)),
                Value::Array(_) => {
                    keep_recursive_search(&args[0], &keys).unwrap_or_else(|| Value::Array(vec![]))
                }
                other => other.clone(),
            }))
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
            Ok(Some(lookup_path(&args[0], path).cloned().unwrap_or_else(
                || args.get(2).cloned().unwrap_or(Value::Null),
            )))
        }
        "SET_PATH" => {
            check_args(name, args, 3)?;
            let parts = path_parts(name, &args[1])?;
            let mut root = args[0].clone();
            set_path(&mut root, &parts, args[2].clone())?;
            Ok(Some(root))
        }
        "UNSET_PATH" => {
            check_args(name, args, 2)?;
            let parts = path_parts(name, &args[1])?;
            let mut root = args[0].clone();
            unset_path(&mut root, &parts);
            Ok(Some(root))
        }
        "MERGE" => {
            let docs = merge_inputs(name, args)?;
            let mut result = Map::new();
            for doc in docs {
                match doc {
                    Value::Object(obj) => {
                        for (key, value) in obj {
                            result.insert(key.clone(), value.clone());
                        }
                    }
                    Value::Null => {}
                    other => {
                        return Err(DbError::ExecutionError(format!(
                            "MERGE: all arguments must be objects, got: {}",
                            type_name(other)
                        )));
                    }
                }
            }
            Ok(Some(Value::Object(result)))
        }
        "DEEP_MERGE" | "MERGE_RECURSIVE" => {
            let docs = merge_inputs(name, args)?;
            let mut result = Value::Object(Map::new());
            for doc in docs {
                match doc {
                    Value::Null => {}
                    Value::Object(_) => deep_merge_into(&mut result, doc),
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "{name}: all arguments must be objects"
                        )));
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
            let mut obj = Map::new();
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
            let key = args[1].as_str().ok_or_else(|| {
                DbError::ExecutionError("HAS: second argument must be a string (key)".to_string())
            })?;
            let has_key = match &args[0] {
                Value::Object(obj) => obj.contains_key(key),
                Value::Array(arr) => arr
                    .iter()
                    .any(|item| item.as_object().is_some_and(|o| o.contains_key(key))),
                _ => false,
            };
            Ok(Some(Value::Bool(has_key)))
        }
        "MATCHES" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "MATCHES requires 2-3 arguments: document, examples, [returnIndex]".to_string(),
                ));
            }
            let return_index = opt_flag(name, "returnIndex", args.get(2))?;
            let examples: &[Value] = match &args[1] {
                Value::Array(a) => a.as_slice(),
                v @ Value::Object(_) => std::slice::from_ref(v),
                _ => {
                    return Err(DbError::ExecutionError(
                        "MATCHES: examples must be an object or an array of objects".to_string(),
                    ))
                }
            };
            let hit = match &args[0] {
                Value::Object(doc) => examples.iter().position(|ex| match ex {
                    Value::Object(ex) => ex
                        .iter()
                        .all(|(k, v)| doc.get(k).is_some_and(|d| deep_equal(d, v))),
                    _ => false,
                }),
                _ => None,
            };
            Ok(Some(if return_index {
                Value::from(hit.map(|i| i as i64).unwrap_or(-1))
            } else {
                Value::Bool(hit.is_some())
            }))
        }
        "TRANSLATE" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "TRANSLATE requires 2-3 arguments: value, lookup, [default]".to_string(),
                ));
            }
            let fallback = || args.get(2).cloned().unwrap_or_else(|| args[0].clone());
            let lookup = match &args[1] {
                Value::Object(o) => o,
                Value::Null => return Ok(Some(fallback())),
                _ => {
                    return Err(DbError::ExecutionError(
                        "TRANSLATE: lookup must be an object".to_string(),
                    ))
                }
            };
            let key = to_string_value(&args[0]);
            Ok(Some(lookup.get(&key).cloned().unwrap_or_else(fallback)))
        }
        "VALUE" => {
            check_args(name, args, 2)?;
            let path = args[1].as_array().ok_or_else(|| {
                DbError::ExecutionError(
                    "VALUE: path must be an array of attribute names and indexes".to_string(),
                )
            })?;
            let mut cur = &args[0];
            for step in path {
                let next = match (cur, step) {
                    (Value::Object(o), Value::String(k)) => o.get(k),
                    (Value::Array(a), Value::Number(n)) => n.as_i64().and_then(|i| {
                        let idx = if i < 0 { a.len() as i64 + i } else { i };
                        usize::try_from(idx).ok().and_then(|i| a.get(i))
                    }),
                    (_, Value::String(_) | Value::Number(_)) => None,
                    _ => {
                        return Err(DbError::ExecutionError(
                            "VALUE: path elements must be strings or numbers".to_string(),
                        ))
                    }
                };
                match next {
                    Some(v) => cur = v,
                    None => return Ok(Some(Value::Null)),
                }
            }
            Ok(Some(cur.clone()))
        }
        "HASH" => {
            check_args(name, args, 1)?;
            let mut buf = Vec::new();
            hash_encode(&args[0], &mut buf);
            // Low 52 bits, like AQL: the result survives a round trip through
            // a double (JavaScript clients) unchanged.
            let h = seahash::hash(&buf) & ((1u64 << 52) - 1);
            Ok(Some(Value::from(h)))
        }
        _ => Ok(None),
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Optional boolean flag: absent/null is false.
fn opt_flag(fname: &str, what: &str, v: Option<&Value>) -> DbResult<bool> {
    match v {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(DbError::ExecutionError(format!(
            "{fname}: {what} must be a boolean"
        ))),
    }
}

/// Attribute-name arguments: any mix of strings and arrays of strings, as
/// in AQL's `KEEP(doc, "a", "b")` and `KEEP(doc, ["a", "b"])`. Anything else
/// is an error, so a redaction can never silently drop nothing.
fn key_args<'a>(fname: &str, args: &'a [Value]) -> DbResult<Vec<&'a str>> {
    let bad = || {
        DbError::ExecutionError(format!(
            "{fname}: attribute names must be strings or arrays of strings"
        ))
    };
    let mut keys = Vec::new();
    for a in args {
        match a {
            Value::String(s) => keys.push(s.as_str()),
            Value::Array(items) => {
                for it in items {
                    keys.push(it.as_str().ok_or_else(bad)?);
                }
            }
            _ => return Err(bad()),
        }
    }
    Ok(keys)
}

/// `MERGE(a, b, …)` or the single-array form `MERGE([a, b, …])`.
fn merge_inputs<'a>(fname: &str, args: &'a [Value]) -> DbResult<&'a [Value]> {
    if args.is_empty() {
        return Err(DbError::ExecutionError(format!(
            "{fname} requires at least 1 argument"
        )));
    }
    if args.len() == 1 {
        if let Value::Array(items) = &args[0] {
            return Ok(items);
        }
    }
    Ok(args)
}

fn to_string_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => number_to_string(n),
        Value::Bool(b) => b.to_string(),
        // AQL and the docs: TO_STRING(null) is "".
        Value::Null => String::new(),
        v => serde_json::to_string(v).unwrap_or_default(),
    }
}

/// Integral floats print without a fraction (`1 + 1` is "2", not "2.0").
fn number_to_string(n: &serde_json::Number) -> String {
    if n.is_f64() {
        if let Some(f) = n.as_f64() {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                return format!("{}", f as i64);
            }
        }
    }
    n.to_string()
}

/// AQL TO_NUMBER: strings are trimmed and parsed integer-first; anything
/// unparseable is 0.
fn to_number(v: &Value) -> Value {
    match v {
        Value::Number(n) => Value::Number(n.clone()),
        Value::Bool(b) => Value::from(i64::from(*b)),
        Value::Null => Value::from(0),
        Value::String(s) => {
            let t = s.trim();
            if let Ok(i) = t.parse::<i64>() {
                return Value::from(i);
            }
            match t.parse::<f64>() {
                Ok(f) if f.is_finite() && !t.is_empty() => serde_json::Number::from_f64(f)
                    .map(Value::Number)
                    .unwrap_or_else(|| Value::from(0)),
                _ => Value::from(0),
            }
        }
        Value::Array(a) if a.len() == 1 => to_number(&a[0]),
        Value::Array(_) | Value::Object(_) => Value::from(0),
    }
}

fn range(args: &[Value]) -> DbResult<Value> {
    if args.is_empty() || args.len() > 3 {
        return Err(DbError::ExecutionError(
            "RANGE requires 1-3 arguments: end or start, end, [step]".to_string(),
        ));
    }
    let num = |v: &Value| -> DbResult<f64> {
        v.as_f64()
            .filter(|f| f.is_finite())
            .ok_or_else(|| DbError::ExecutionError("RANGE: arguments must be numbers".to_string()))
    };
    let (start_v, end_v) = if args.len() == 1 {
        (Value::from(0), &args[0])
    } else {
        (args[0].clone(), &args[1])
    };
    let start = num(&start_v)?;
    let end = num(end_v)?;
    // AQL: without a step, count down when start > end.
    let step = match args.get(2) {
        Some(v) => num(v)?,
        None if start > end => -1.0,
        None => 1.0,
    };
    if step == 0.0 {
        return Err(DbError::ExecutionError(
            "RANGE: step cannot be 0".to_string(),
        ));
    }
    let as_int = |v: &Value| -> Option<i64> {
        v.as_i64().or_else(|| {
            v.as_f64()
                .filter(|f| f.fract() == 0.0 && f.abs() < 9.0e15)
                .map(|f| f as i64)
        })
    };
    let int_args = (
        as_int(&start_v),
        as_int(end_v),
        match args.get(2) {
            Some(v) => as_int(v),
            None => Some(step as i64),
        },
    );

    if let (Some(start), Some(end), Some(step)) = int_args {
        // i128: `end - start` overflows i64 across the full range, and in
        // release that wrapped to a count of 0, walked past the cap, and
        // allocated until the process was killed.
        let (start_w, end_w, step_w) = (start as i128, end as i128, step as i128);
        let count = if step > 0 {
            if end < start {
                0
            } else {
                (end_w - start_w) / step_w + 1
            }
        } else if end > start {
            0
        } else {
            (start_w - end_w) / step_w.abs() + 1
        };
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        if count > MAX_RANGE {
            return Err(DbError::ExecutionError(format!(
                "RANGE: result would have {} elements (max {})",
                count, MAX_RANGE
            )));
        }
        let mut result = Vec::with_capacity(count);
        let mut i = start;
        while result.len() < count {
            result.push(Value::from(i));
            i = i.saturating_add(step);
        }
        return Ok(Value::Array(result));
    }

    // Float range: RANGE(1, 2, 0.5) is [1, 1.5, 2]. Each value is computed
    // from the start, so the step's rounding error does not accumulate.
    let span = (end - start) / step;
    let count = if span < 0.0 {
        0.0
    } else {
        (span + 1e-9).floor() + 1.0
    };
    if count > MAX_RANGE as f64 {
        return Err(DbError::ExecutionError(format!(
            "RANGE: result would have {} elements (max {})",
            count, MAX_RANGE
        )));
    }
    let count = count as usize;
    Ok(Value::Array(
        (0..count)
            .map(|i| Value::from(start + step * i as f64))
            .collect(),
    ))
}

/// Deep equality where numbers compare by value (1 == 1.0) at every depth.
fn deep_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| deep_equal(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| deep_equal(v, w)))
        }
        _ => a == b,
    }
}

/// Canonical byte encoding for HASH: independent of key order, and an
/// integral float hashes like the integer (HASH(1) == HASH(1.0)).
fn hash_encode(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Null => out.push(b'n'),
        Value::Bool(b) => out.push(if *b { b't' } else { b'f' }),
        Value::Number(n) => {
            let int = n.as_i64().or_else(|| {
                n.as_f64()
                    .filter(|f| f.fract() == 0.0 && f.abs() < 9.0e18)
                    .map(|f| f as i64)
            });
            match (int, n.as_u64()) {
                (Some(i), _) => {
                    out.push(b'i');
                    out.extend_from_slice(&i.to_le_bytes());
                }
                (None, Some(u)) => {
                    out.push(b'u');
                    out.extend_from_slice(&u.to_le_bytes());
                }
                (None, None) => {
                    out.push(b'd');
                    out.extend_from_slice(&n.as_f64().unwrap_or(0.0).to_bits().to_le_bytes());
                }
            }
        }
        Value::String(s) => {
            out.push(b's');
            out.extend_from_slice(&(s.len() as u64).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        Value::Array(a) => {
            out.push(b'a');
            out.extend_from_slice(&(a.len() as u64).to_le_bytes());
            for x in a {
                hash_encode(x, out);
            }
        }
        Value::Object(o) => {
            out.push(b'o');
            out.extend_from_slice(&(o.len() as u64).to_le_bytes());
            let mut entries: Vec<(&String, &Value)> = o.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            for (k, x) in entries {
                out.extend_from_slice(&(k.len() as u64).to_le_bytes());
                out.extend_from_slice(k.as_bytes());
                hash_encode(x, out);
            }
        }
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

/// A document id from a string, or from a document's `_id`.
fn identifier_of(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) => Some(s),
        Value::Object(o) => o.get("_id").and_then(Value::as_str),
        _ => None,
    }
}

fn parse_ident(id: &str) -> Value {
    match id.split_once('/') {
        Some((c, k)) => serde_json::json!({ "collection": c, "key": k }),
        None => serde_json::json!({ "collection": Value::Null, "key": id }),
    }
}

/// KEEP_RECURSIVE on an object: listed keys are kept (and their values
/// filtered the same way); unlisted keys survive only as containers of
/// listed keys somewhere below.
fn keep_recursive_obj(
    o: &Map<String, Value>,
    keys: &std::collections::HashSet<&str>,
) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, val) in o {
        if keys.contains(k.as_str()) {
            out.insert(k.clone(), keep_recursive_kept(val, keys));
        } else if let Some(child) = keep_recursive_search(val, keys) {
            out.insert(k.clone(), child);
        }
    }
    out
}

/// The value of a listed key: kept whole, with nested objects filtered.
fn keep_recursive_kept(v: &Value, keys: &std::collections::HashSet<&str>) -> Value {
    match v {
        Value::Object(o) => Value::Object(keep_recursive_obj(o, keys)),
        Value::Array(a) => Value::Array(a.iter().map(|x| keep_recursive_kept(x, keys)).collect()),
        other => other.clone(),
    }
}

/// The value of an unlisted key: `None` unless something listed is inside.
/// Scalars in such an array are dropped — they are not under a listed key.
fn keep_recursive_search(v: &Value, keys: &std::collections::HashSet<&str>) -> Option<Value> {
    match v {
        Value::Object(o) => {
            let m = keep_recursive_obj(o, keys);
            (!m.is_empty()).then_some(Value::Object(m))
        }
        Value::Array(a) => {
            let items: Vec<Value> = a
                .iter()
                .filter_map(|x| keep_recursive_search(x, keys))
                .collect();
            (!items.is_empty()).then_some(Value::Array(items))
        }
        _ => None,
    }
}

fn redact_value(v: &Value, keys: &[String]) -> Value {
    match v {
        Value::Object(o) => {
            let mut out = Map::new();
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

/// `GET`'s path walk: `"a.b.0"` descends through objects by key and through
/// arrays by index. Empty segments are skipped; a path with no dot is one key,
/// so `GET(doc, "")` reads the attribute named `""`.
pub(crate) fn lookup_path<'v>(root: &'v Value, path: &str) -> Option<&'v Value> {
    if !path.contains('.') {
        return match root {
            Value::Object(obj) => obj.get(path),
            Value::Array(arr) => path.parse::<usize>().ok().and_then(|i| arr.get(i)),
            _ => None,
        };
    }
    let mut cur = root;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        cur = match cur {
            Value::Object(obj) => obj.get(part)?,
            Value::Array(arr) => arr.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// A `SET_PATH` / `UNSET_PATH` path: a dotted string like `GET`'s, or an
/// array of segments for keys that themselves contain a dot.
fn path_parts(name: &str, path: &Value) -> DbResult<Vec<String>> {
    let parts: Vec<String> = match path {
        Value::String(s) => s
            .split('.')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect(),
        Value::Array(items) => items
            .iter()
            .map(|p| match p {
                Value::String(s) => Ok(s.clone()),
                Value::Number(n) if n.is_u64() => Ok(n.to_string()),
                _ => Err(DbError::ExecutionError(format!(
                    "{name}: path segments must be strings or non-negative integers"
                ))),
            })
            .collect::<DbResult<_>>()?,
        _ => {
            return Err(DbError::ExecutionError(format!(
                "{name}: path must be a string or an array"
            )))
        }
    };
    if parts.is_empty() {
        return Err(DbError::ExecutionError(format!("{name}: path is empty")));
    }
    Ok(parts)
}

/// Set `parts` in `root`, creating missing (or null) intermediate levels as
/// objects. An array level takes an index, and an index equal to its length
/// appends. Walking through a string, number or boolean is an error rather
/// than a silent overwrite.
fn set_path(root: &mut Value, parts: &[String], value: Value) -> DbResult<()> {
    let mut cur = root;
    for (depth, part) in parts.iter().enumerate() {
        if cur.is_null() {
            *cur = Value::Object(Map::new());
        }
        let last = depth + 1 == parts.len();
        cur = match cur {
            Value::Object(obj) => {
                if last {
                    obj.insert(part.clone(), value);
                    return Ok(());
                }
                obj.entry(part.clone()).or_insert(Value::Null)
            }
            Value::Array(arr) => {
                let len = arr.len();
                let i = part
                    .parse::<usize>()
                    .ok()
                    .filter(|&i| i <= len)
                    .ok_or_else(|| {
                        DbError::ExecutionError(format!(
                            "SET_PATH: '{}' is not an index into an array of length {}",
                            parts[..=depth].join("."),
                            len
                        ))
                    })?;
                if i == len {
                    arr.push(Value::Null);
                }
                if last {
                    arr[i] = value;
                    return Ok(());
                }
                &mut arr[i]
            }
            other => {
                let at = if depth == 0 {
                    "the value".to_string()
                } else {
                    format!("'{}'", parts[..depth].join("."))
                };
                return Err(DbError::ExecutionError(format!(
                    "SET_PATH: {at} is a {}, not an object or array",
                    json_type_name(other)
                )));
            }
        };
    }
    Ok(())
}

/// Remove the value at `parts`, if there is one. A path that does not exist
/// leaves `root` unchanged.
fn unset_path(root: &mut Value, parts: &[String]) {
    let Some((leaf, parents)) = parts.split_last() else {
        return;
    };
    let mut cur = root;
    for part in parents {
        cur = match cur {
            Value::Object(obj) => match obj.get_mut(part) {
                Some(v) => v,
                None => return,
            },
            Value::Array(arr) => match part.parse::<usize>().ok().and_then(|i| arr.get_mut(i)) {
                Some(v) => v,
                None => return,
            },
            _ => return,
        };
    }
    match cur {
        Value::Object(obj) => {
            obj.remove(leaf);
        }
        Value::Array(arr) => {
            if let Some(i) = leaf.parse::<usize>().ok().filter(|&i| i < arr.len()) {
                arr.remove(i);
            }
        }
        _ => {}
    }
}

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, args: &[Value]) -> Value {
        evaluate(name, args).unwrap().unwrap()
    }

    #[test]
    fn set_path_creates_and_replaces() {
        assert_eq!(
            call(
                "SET_PATH",
                &[json!({"a": {"b": 1}}), json!("a.c.d"), json!(2)]
            ),
            json!({"a": {"b": 1, "c": {"d": 2}}})
        );
        assert_eq!(
            call("SET_PATH", &[json!(null), json!("a"), json!(1)]),
            json!({"a": 1})
        );
        assert_eq!(
            call("SET_PATH", &[json!({"a": null}), json!("a.b"), json!(1)]),
            json!({"a": {"b": 1}})
        );
        assert_eq!(
            call("SET_PATH", &[json!({"l": [1, 2]}), json!("l.1"), json!(9)]),
            json!({"l": [1, 9]})
        );
        assert_eq!(
            call("SET_PATH", &[json!({"l": [1, 2]}), json!("l.2"), json!(3)]),
            json!({"l": [1, 2, 3]}),
            "an index equal to the length appends"
        );
        assert_eq!(
            call("SET_PATH", &[json!({}), json!(["a.b", "c"]), json!(1)]),
            json!({"a.b": {"c": 1}}),
            "array segments may contain dots"
        );
    }

    #[test]
    fn set_path_refuses_to_overwrite_scalars() {
        assert!(evaluate("SET_PATH", &[json!({"a": "x"}), json!("a.b"), json!(1)]).is_err());
        assert!(evaluate("SET_PATH", &[json!(5), json!("a"), json!(1)]).is_err());
        assert!(evaluate("SET_PATH", &[json!({"l": []}), json!("l.3"), json!(1)]).is_err());
        assert!(evaluate("SET_PATH", &[json!({"l": []}), json!("l.x"), json!(1)]).is_err());
        assert!(evaluate("SET_PATH", &[json!({}), json!(""), json!(1)]).is_err());
        assert!(evaluate("SET_PATH", &[json!({}), json!([true]), json!(1)]).is_err());
    }

    #[test]
    fn unset_path_removes_only_what_exists() {
        assert_eq!(
            call(
                "UNSET_PATH",
                &[json!({"a": {"b": 1, "c": 2}}), json!("a.b")]
            ),
            json!({"a": {"c": 2}})
        );
        assert_eq!(
            call("UNSET_PATH", &[json!({"l": [1, 2, 3]}), json!("l.1")]),
            json!({"l": [1, 3]})
        );
        let doc = json!({"a": {"b": 1}});
        for missing in ["x.y", "a.b.c", "a.z"] {
            assert_eq!(call("UNSET_PATH", &[doc.clone(), json!(missing)]), doc);
        }
    }

    #[test]
    fn get_path_walk_is_unchanged() {
        let doc = json!({"a": {"l": [10, {"b": 2}]}, "": 7, "n": null});
        assert_eq!(call("GET", &[doc.clone(), json!("a.l.1.b")]), json!(2));
        assert_eq!(call("GET", &[doc.clone(), json!("")]), json!(7));
        assert_eq!(
            call("GET", &[doc.clone(), json!("n"), json!("d")]),
            Value::Null
        );
        assert_eq!(call("GET", &[doc, json!("a.x"), json!("d")]), json!("d"));
    }

    #[test]
    fn keep_and_unset_accept_arrays() {
        let doc = json!({"name": "a", "password": "x", "email": "e"});
        assert_eq!(
            call("UNSET", &[doc.clone(), json!(["password"])]),
            json!({"name": "a", "email": "e"})
        );
        assert_eq!(
            call("UNSET", &[doc.clone(), json!("password"), json!(["email"])]),
            json!({"name": "a"})
        );
        assert_eq!(
            call("KEEP", &[doc.clone(), json!(["name", "email"])]),
            json!({"name": "a", "email": "e"})
        );
        assert!(evaluate("UNSET", &[doc.clone(), json!(1)]).is_err());
        assert!(evaluate("KEEP", &[doc.clone(), json!([1])]).is_err());
        let nested = json!({"a": 1, "n": {"password": 2, "b": 3}});
        assert_eq!(
            call("UNSET_RECURSIVE", &[nested, json!(["password"])]),
            json!({"a": 1, "n": {"b": 3}})
        );
    }

    #[test]
    fn keep_recursive_arrays() {
        let doc = json!({"a": 1, "nest": {"a": 2, "b": 3}, "tags": [1, 2], "list": [{"a": 5, "c": 6}, 7], "a2": {"a": [1, {"z": 1}]}});
        let r = call("KEEP_RECURSIVE", &[doc, json!(["a"])]);
        assert_eq!(r["a"], json!(1));
        assert_eq!(r["nest"], json!({"a": 2}));
        // Unlisted array of scalars is dropped.
        assert!(r.get("tags").is_none());
        assert_eq!(r["list"], json!([{"a": 5}]));
        // A listed key's array is kept whole (objects inside filtered).
        assert_eq!(r["a2"], json!({"a": [1, {}]}));
    }

    #[test]
    fn conversions_follow_docs() {
        assert_eq!(call("TYPENAME", &[json!(2.5)]), json!("number"));
        assert_eq!(call("TYPENAME", &[json!(3)]), json!("number"));
        assert_eq!(call("TO_STRING", &[Value::Null]), json!(""));
        assert_eq!(call("TO_STRING", &[json!(2.0)]), json!("2"));
        assert_eq!(call("TO_STRING", &[json!(2.5)]), json!("2.5"));
        assert_eq!(call("TO_ARRAY", &[json!({"a": 1})]), json!([1]));
        assert_eq!(call("TO_NUMBER", &[json!(" 12 ")]), json!(12));
        assert_eq!(call("TO_NUMBER", &[json!("123")]), json!(123));
        assert_eq!(call("TO_NUMBER", &[json!("1.5")]), json!(1.5));
        assert_eq!(call("TO_NUMBER", &[json!("foo")]), json!(0));
        assert_eq!(call("TO_NUMBER", &[json!("")]), json!(0));
        assert_eq!(call("TO_NUMBER", &[json!(["7"])]), json!(7));
        assert_eq!(call("TO_BOOL", &[json!("false")]), json!(true));
        assert_eq!(call("TO_BOOL", &[json!("")]), json!(false));
        assert_eq!(
            call("TO_BOOL", &[json!([])]),
            Value::Bool(to_bool(&json!([])))
        );
    }

    #[test]
    fn attributes_and_values_options() {
        let doc = json!({"_key": "k", "b": 2, "a": 1});
        assert_eq!(
            call("ATTRIBUTES", &[doc.clone(), json!(true), json!(true)]),
            json!(["a", "b"])
        );
        assert_eq!(
            call("VALUES", &[doc.clone(), json!(true)])
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(call("ATTRIBUTES", &[doc]).as_array().unwrap().len(), 3);
    }

    #[test]
    fn merge_forms() {
        assert_eq!(
            call("MERGE", &[json!([{"a": 1}, {"b": 2}, {"a": 3}])]),
            json!({"a": 3, "b": 2})
        );
        assert_eq!(
            call("MERGE", &[json!({"a": 1}), Value::Null, json!({"b": 2})]),
            json!({"a": 1, "b": 2})
        );
        assert_eq!(
            call(
                "MERGE_RECURSIVE",
                &[json!({"a": {"b": 1}}), json!({"a": {"c": 2}})]
            ),
            json!({"a": {"b": 1, "c": 2}})
        );
        assert_eq!(
            call(
                "MERGE_RECURSIVE",
                &[json!([{"a": {"b": 1}}, {"a": {"c": 2}}])]
            ),
            json!({"a": {"b": 1, "c": 2}})
        );
        assert!(evaluate("MERGE", &[json!({"a": 1}), json!(1)]).is_err());
    }

    #[test]
    fn range_forms() {
        assert_eq!(
            call("RANGE", &[json!(1), json!(2), json!(0.5)]),
            json!([1.0, 1.5, 2.0])
        );
        assert_eq!(call("RANGE", &[json!(5), json!(1)]), json!([5, 4, 3, 2, 1]));
        assert_eq!(call("RANGE", &[json!(5), json!(1), json!(1)]), json!([]));
        assert_eq!(
            call("RANGE", &[json!(0), json!(1), json!(0.25)])
                .as_array()
                .unwrap()
                .len(),
            5
        );
        assert!(evaluate("RANGE", &[json!(0), json!(1), json!(0)]).is_err());
        assert!(evaluate("RANGE", &[json!(0), json!(1e12), json!(0.5)]).is_err());
    }

    #[test]
    fn matches_translate_value_hash() {
        let doc = json!({"a": 1, "b": {"c": 2}});
        assert_eq!(
            call("MATCHES", &[doc.clone(), json!({"a": 1.0})]),
            json!(true)
        );
        assert_eq!(
            call(
                "MATCHES",
                &[doc.clone(), json!([{"a": 2}, {"b": {"c": 2}}]), json!(true)]
            ),
            json!(1)
        );
        assert_eq!(
            call("MATCHES", &[doc.clone(), json!([{"a": 2}]), json!(true)]),
            json!(-1)
        );
        assert_eq!(
            call("TRANSLATE", &[json!("FR"), json!({"FR": "France"})]),
            json!("France")
        );
        assert_eq!(
            call("TRANSLATE", &[json!(42), json!({"42": "x"})]),
            json!("x")
        );
        assert_eq!(
            call("TRANSLATE", &[json!("DE"), json!({"FR": "France"})]),
            json!("DE")
        );
        assert_eq!(
            call(
                "TRANSLATE",
                &[json!("DE"), json!({"FR": "France"}), json!("?")]
            ),
            json!("?")
        );
        assert_eq!(call("VALUE", &[doc.clone(), json!(["b", "c"])]), json!(2));
        assert_eq!(
            call("VALUE", &[json!({"l": [1, 2, 3]}), json!(["l", -1])]),
            json!(3)
        );
        assert_eq!(
            call("VALUE", &[doc.clone(), json!(["x", "y"])]),
            Value::Null
        );
        let h1 = call("HASH", &[json!({"a": 1, "b": [1, 2]})]);
        let h2 = call("HASH", &[json!({"b": [1.0, 2], "a": 1})]);
        assert_eq!(h1, h2);
        assert_ne!(h1, call("HASH", &[json!({"a": 2})]));
        assert!(h1.as_u64().unwrap() < (1u64 << 52));
    }

    #[test]
    fn parse_identifier_of_document_and_nullif() {
        assert_eq!(
            call(
                "PARSE_IDENTIFIER",
                &[json!({"_id": "users/ada", "_key": "ada"})]
            ),
            json!({"collection": "users", "key": "ada"})
        );
        assert_eq!(
            call("PARSE_KEY", &[json!({"_id": "users/ada"})]),
            json!("ada")
        );
        assert_eq!(call("PARSE_IDENTIFIER", &[Value::Null]), Value::Null);
        assert_eq!(call("NULLIF", &[json!(1), json!(1.0)]), Value::Null);
    }
}
