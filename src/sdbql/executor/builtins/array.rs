//! Array functions for SDBQL.
//!
//! FIRST, LAST, LENGTH, REVERSE, SORTED, UNIQUE, FLATTEN, etc.

use super::super::{compare_values, values_equal, ValueSet};
use crate::error::{DbError, DbResult};
use serde_json::Value;

/// Evaluate array functions
#[allow(clippy::get_first)]
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "FIRST" => {
            check_args(name, args, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
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
            sorted.sort_unstable_by(compare_values);
            Ok(Some(Value::Array(sorted)))
        }
        "SORTED_DESC" => {
            check_args(name, args, 1)?;
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("SORTED_DESC: argument must be an array".to_string())
            })?;
            let mut sorted = arr.clone();
            sorted.sort_unstable_by(|a, b| compare_values(b, a));
            Ok(Some(Value::Array(sorted)))
        }
        "UNIQUE" => {
            check_args(name, args, 1)?;
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("UNIQUE: argument must be an array".to_string())
            })?;
            let mut seen = ValueSet::with_capacity(arr.len());
            let mut unique = Vec::with_capacity(arr.len());
            for v in arr {
                if seen.insert(v) {
                    unique.push(v.clone());
                }
            }
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
                // Past the end is an empty slice, not a panic.
                (start as usize).min(arr.len())
            };
            let end = if args.len() > 2 {
                let len = args[2].as_u64().unwrap_or(arr.len() as u64) as usize;
                start.saturating_add(len).min(arr.len())
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
            let raw = args[1]
                .as_i64()
                .or_else(|| args[1].as_f64().map(|f| f as i64))
                .ok_or_else(|| {
                    DbError::ExecutionError("NTH: index must be a number".to_string())
                })?;
            let idx = if raw < 0 {
                let n = arr.len() as i64 + raw;
                if n < 0 {
                    return Ok(Some(Value::Null));
                }
                n as usize
            } else {
                raw as usize
            };
            Ok(Some(arr.get(idx).cloned().unwrap_or(Value::Null)))
        }
        "CONTAINS" | "CONTAINS_ARRAY" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "CONTAINS requires 2 arguments: array, value".to_string(),
                ));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("CONTAINS: first argument must be an array".to_string())
            })?;
            Ok(Some(Value::Bool(
                arr.iter().any(|item| values_equal(item, &args[1])),
            )))
        }
        "TAKE" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "TAKE requires 2 arguments: array, n".to_string(),
                ));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("TAKE: first argument must be an array".to_string())
            })?;
            let n = args[1].as_i64().unwrap_or(0);
            if n <= 0 {
                return Ok(Some(Value::Array(vec![])));
            }
            Ok(Some(Value::Array(
                arr.iter().take(n as usize).cloned().collect(),
            )))
        }
        "DROP" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "DROP requires 2 arguments: array, n".to_string(),
                ));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("DROP: first argument must be an array".to_string())
            })?;
            let n = args[1].as_i64().unwrap_or(0).max(0) as usize;
            Ok(Some(Value::Array(arr.iter().skip(n).cloned().collect())))
        }
        "CHUNK" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "CHUNK requires 2 arguments: array, size".to_string(),
                ));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("CHUNK: first argument must be an array".to_string())
            })?;
            let size = args[1].as_i64().unwrap_or(0);
            if size <= 0 {
                return Err(DbError::ExecutionError(
                    "CHUNK: size must be a positive integer".to_string(),
                ));
            }
            let size = size as usize;
            let chunks: Vec<Value> = arr.chunks(size).map(|c| Value::Array(c.to_vec())).collect();
            Ok(Some(Value::Array(chunks)))
        }
        "ZIP" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "ZIP requires at least 2 array arguments".to_string(),
                ));
            }
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let arrays: Result<Vec<&Vec<Value>>, DbError> = args
                .iter()
                .map(|a| {
                    a.as_array().ok_or_else(|| {
                        DbError::ExecutionError("ZIP: all arguments must be arrays".to_string())
                    })
                })
                .collect();
            let arrays = arrays?;
            let len = arrays.iter().map(|a| a.len()).min().unwrap_or(0);
            // AQL: ZIP(keys, values) → object when there are exactly two
            // arrays and every key is a string.
            if arrays.len() == 2 && arrays[0].iter().all(|k| k.is_string()) {
                let mut obj = serde_json::Map::new();
                for (key, value) in arrays[0].iter().zip(arrays[1].iter()) {
                    if let Some(s) = key.as_str() {
                        obj.insert(s.to_string(), value.clone());
                    }
                }
                return Ok(Some(Value::Object(obj)));
            }
            let zipped: Vec<Value> = (0..len)
                .map(|i| Value::Array(arrays.iter().map(|a| a[i].clone()).collect()))
                .collect();
            Ok(Some(Value::Array(zipped)))
        }
        "ZIP_OBJECT" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "ZIP_OBJECT requires keys[], values[]".to_string(),
                ));
            }
            let keys = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("ZIP_OBJECT: keys must be an array".to_string())
            })?;
            let vals = args[1].as_array().ok_or_else(|| {
                DbError::ExecutionError("ZIP_OBJECT: values must be an array".to_string())
            })?;
            let mut obj = serde_json::Map::new();
            for (k, v) in keys.iter().zip(vals.iter()) {
                if let Some(s) = k.as_str() {
                    obj.insert(s.to_string(), v.clone());
                }
            }
            Ok(Some(Value::Object(obj)))
        }
        "COUNT" => {
            check_args(name, args, 1)?;
            match &args[0] {
                Value::Array(arr) => Ok(Some(Value::Number(serde_json::Number::from(arr.len())))),
                Value::Object(obj) => Ok(Some(Value::Number(serde_json::Number::from(obj.len())))),
                Value::String(s) => Ok(Some(Value::Number(serde_json::Number::from(
                    s.chars().count(),
                )))),
                Value::Null => Ok(Some(Value::Number(serde_json::Number::from(0)))),
                _ => Ok(Some(Value::Number(serde_json::Number::from(1)))),
            }
        }
        "OUTERSECTION" | "SYMDIFF" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "OUTERSECTION requires 2 array arguments".to_string(),
                ));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let a = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("OUTERSECTION: first argument must be an array".to_string())
            })?;
            let b = args[1].as_array().ok_or_else(|| {
                DbError::ExecutionError(
                    "OUTERSECTION: second argument must be an array".to_string(),
                )
            })?;
            let mut in_a = ValueSet::with_capacity(a.len());
            let mut in_b = ValueSet::with_capacity(b.len());
            for v in a {
                in_a.insert(v);
            }
            for v in b {
                in_b.insert(v);
            }
            let mut out = Vec::with_capacity(a.len() + b.len());
            for v in a {
                if !in_b.contains(v) {
                    out.push(v.clone());
                }
            }
            for v in b {
                if !in_a.contains(v) {
                    out.push(v.clone());
                }
            }
            Ok(Some(Value::Array(out)))
        }
        "LENGTH" => {
            check_args(name, args, 1)?;
            match &args[0] {
                Value::Array(arr) => Ok(Some(Value::Number(serde_json::Number::from(arr.len())))),
                Value::String(s) => Ok(Some(Value::Number(serde_json::Number::from(
                    s.chars().count(),
                )))),
                Value::Object(obj) => Ok(Some(Value::Number(serde_json::Number::from(obj.len())))),
                Value::Null => Ok(Some(Value::Number(serde_json::Number::from(0)))),
                _ => Ok(Some(Value::Number(serde_json::Number::from(0)))),
            }
        }
        "APPEND" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "APPEND requires at least 2 arguments".to_string(),
                ));
            }
            let first = match &args[0] {
                Value::Array(a) => a,
                _ => {
                    return Err(DbError::ExecutionError(
                        "APPEND: first argument must be an array".to_string(),
                    ));
                }
            };
            let extra: usize = args[1..]
                .iter()
                .map(|a| match a {
                    Value::Array(items) => items.len(),
                    _ => 1,
                })
                .sum();
            let mut arr = Vec::with_capacity(first.len() + extra);
            arr.extend_from_slice(first);
            for arg in &args[1..] {
                if let Value::Array(items) = arg {
                    arr.extend_from_slice(items);
                } else {
                    arr.push(arg.clone());
                }
            }
            Ok(Some(Value::Array(arr)))
        }
        "SHIFT" => {
            check_args(name, args, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = match &args[0] {
                Value::Array(a) => a,
                _ => {
                    return Err(DbError::ExecutionError(
                        "SHIFT: argument must be an array".to_string(),
                    ));
                }
            };
            if arr.is_empty() {
                return Ok(Some(Value::Array(vec![])));
            }
            Ok(Some(Value::Array(arr[1..].to_vec())))
        }
        "UNSHIFT" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "UNSHIFT requires at least 2 arguments".to_string(),
                ));
            }
            let base = match &args[0] {
                Value::Array(a) => a,
                _ => {
                    return Err(DbError::ExecutionError(
                        "UNSHIFT: first argument must be an array".to_string(),
                    ));
                }
            };
            let mut items = Vec::with_capacity(base.len() + args.len() - 1);
            items.extend_from_slice(&args[1..]);
            items.extend_from_slice(base);
            Ok(Some(Value::Array(items)))
        }
        "UNION" => {
            let cap: usize = args
                .iter()
                .map(|a| a.as_array().map(|x| x.len()).unwrap_or(0))
                .sum();
            let mut seen = ValueSet::with_capacity(cap);
            let mut result = Vec::with_capacity(cap);
            for arg in args {
                match arg {
                    Value::Array(arr) => {
                        for item in arr {
                            if seen.insert(item) {
                                result.push(item.clone());
                            }
                        }
                    }
                    _ => {
                        return Err(DbError::ExecutionError(
                            "UNION: all arguments must be arrays".to_string(),
                        ));
                    }
                }
            }
            Ok(Some(Value::Array(result)))
        }
        "INTERSECTION" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "INTERSECTION requires at least 2 arguments".to_string(),
                ));
            }
            let first = match &args[0] {
                Value::Array(a) => a,
                _ => {
                    return Err(DbError::ExecutionError(
                        "INTERSECTION: first argument must be an array".to_string(),
                    ));
                }
            };
            let mut others: Vec<ValueSet> = Vec::with_capacity(args.len() - 1);
            for arg in &args[1..] {
                let arr = match arg {
                    Value::Array(a) => a,
                    _ => {
                        return Err(DbError::ExecutionError(
                            "INTERSECTION: all arguments must be arrays".to_string(),
                        ));
                    }
                };
                let mut set = ValueSet::with_capacity(arr.len());
                for item in arr {
                    set.insert(item);
                }
                others.push(set);
            }
            let result: Vec<Value> = first
                .iter()
                .filter(|item| others.iter().all(|s| s.contains(item)))
                .cloned()
                .collect();
            Ok(Some(Value::Array(result)))
        }
        "MINUS" | "DIFFERENCE" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "MINUS requires 2 arguments".to_string(),
                ));
            }
            let arr1 = match &args[0] {
                Value::Array(a) => a,
                _ => {
                    return Err(DbError::ExecutionError(
                        "MINUS: first argument must be an array".to_string(),
                    ));
                }
            };
            let arr2 = match &args[1] {
                Value::Array(a) => a,
                _ => {
                    return Err(DbError::ExecutionError(
                        "MINUS: second argument must be an array".to_string(),
                    ));
                }
            };
            let mut minus = ValueSet::with_capacity(arr2.len());
            for item in arr2 {
                minus.insert(item);
            }
            let result: Vec<Value> = arr1
                .iter()
                .filter(|item| !minus.contains(item))
                .cloned()
                .collect();
            Ok(Some(Value::Array(result)))
        }
        _ => Ok(None),
    }
}

fn flatten_array(arr: &[Value], depth: usize) -> Vec<Value> {
    if depth == 0 {
        return arr.to_vec();
    }
    let mut result = Vec::with_capacity(arr.len());
    flatten_into(arr, depth, &mut result);
    result
}

fn flatten_into(arr: &[Value], depth: usize, out: &mut Vec<Value>) {
    if depth == 0 {
        out.extend_from_slice(arr);
        return;
    }
    for item in arr {
        if let Value::Array(inner) = item {
            flatten_into(inner, depth - 1, out);
        } else {
            out.push(item.clone());
        }
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
