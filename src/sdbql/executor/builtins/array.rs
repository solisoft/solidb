//! Array functions for SDBQL.
//!
//! FIRST, LAST, LENGTH, REVERSE, SORTED, UNIQUE, FLATTEN, etc.

use std::collections::HashMap;

use super::super::{compare_values, hash_value, to_bool, values_equal, ValueSet};
use crate::error::{DbError, DbResult};
use serde_json::Value;

/// Ceiling on the padding `REPLACE_NTH` may add past the end of an array.
/// Same bound as `RANGE`: a position is user input, so it must not size an
/// allocation on its own.
const MAX_PAD_LEN: usize = 1_000_000;

/// Read an integer argument.
///
/// Arithmetic in SDBQL produces floats (`n - 1` is `2.0`, not `2`), so
/// reading integer arguments with `as_i64` silently treated `TAKE(a, n - 1)`
/// as `TAKE(a, 0)`. This accepts any integral, finite number; a `u64` above
/// `i64::MAX` saturates. Anything else — fractions, strings, null — is `None`.
pub(crate) fn as_int(v: &Value) -> Option<i64> {
    let Value::Number(n) = v else { return None };
    if let Some(i) = n.as_i64() {
        return Some(i);
    }
    if let Some(u) = n.as_u64() {
        return Some(i64::try_from(u).unwrap_or(i64::MAX));
    }
    let f = n.as_f64()?;
    if f.is_finite() && f.fract() == 0.0 && f >= i64::MIN as f64 && f < i64::MAX as f64 {
        Some(f as i64)
    } else {
        None
    }
}

/// AQL `LENGTH` / `COUNT`: elements, attributes or characters; `true` is 1,
/// `false` and `null` are 0, and a number is the length of its string form.
fn length_of(v: &Value) -> usize {
    match v {
        Value::Array(arr) => arr.len(),
        Value::Object(obj) => obj.len(),
        Value::String(s) => s.chars().count(),
        Value::Null => 0,
        Value::Bool(b) => usize::from(*b),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                n.to_string().len()
            } else {
                n.as_f64().map(|f| f.to_string().len()).unwrap_or(0)
            }
        }
    }
}

fn array_arg<'v>(name: &str, v: &'v Value, which: &str) -> DbResult<&'v Vec<Value>> {
    v.as_array().ok_or_else(|| {
        DbError::ExecutionError(format!("{}: {} argument must be an array", name, which))
    })
}

/// Resolve a possibly negative position against `len`. `None` when it falls
/// before the start.
fn resolve_position(pos: i64, len: usize) -> Option<usize> {
    if pos < 0 {
        let p = len as i64 + pos;
        (p >= 0).then_some(p as usize)
    } else {
        Some(usize::try_from(pos).unwrap_or(usize::MAX))
    }
}

/// Keep the first occurrence of each value, in order.
fn dedup_values(items: Vec<Value>) -> Vec<Value> {
    let mut seen = ValueSet::with_capacity(items.len());
    items.into_iter().filter(|v| seen.insert(v)).collect()
}

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
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
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
        "SORTED_UNIQUE" => {
            check_args(name, args, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = array_arg(name, &args[0], "first")?;
            let mut sorted = dedup_values(arr.clone());
            sorted.sort_by(compare_values);
            Ok(Some(Value::Array(sorted)))
        }
        "UNIQUE" => {
            check_args(name, args, 1)?;
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("UNIQUE: argument must be an array".to_string())
            })?;
            Ok(Some(Value::Array(dedup_values(arr.clone()))))
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
            let depth = args.get(1).and_then(as_int).unwrap_or(1).max(0);
            let depth = usize::try_from(depth).unwrap_or(usize::MAX);
            let flattened = flatten_array(arr, depth);
            Ok(Some(Value::Array(flattened)))
        }
        "PUSH" => {
            // AQL: PUSH(anyArray, value, unique?)
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "PUSH requires 2-3 arguments: array, value, [unique]".to_string(),
                ));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("PUSH: first argument must be an array".to_string())
            })?;
            let unique = args.get(2).map(to_bool).unwrap_or(false);
            let mut result = arr.clone();
            if !(unique && arr.iter().any(|x| values_equal(x, &args[1]))) {
                result.push(args[1].clone());
            }
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
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "SLICE requires 2-3 arguments: array, start, [length]".to_string(),
                ));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("SLICE: first argument must be an array".to_string())
            })?;
            let len = arr.len() as i64;
            let start = as_int(&args[1]).unwrap_or(0);
            // Past the end is an empty slice, not a panic.
            let start = if start < 0 {
                (len + start).max(0)
            } else {
                start.min(len)
            };
            let end = match args.get(2) {
                None | Some(Value::Null) => len,
                Some(v) => match as_int(v) {
                    // AQL: a negative length excludes that many elements
                    // from the end.
                    Some(l) if l < 0 => len.saturating_add(l),
                    Some(l) => start.saturating_add(l).min(len),
                    None => len,
                },
            };
            let end = end.clamp(start, len);
            let result: Vec<Value> = arr[start as usize..end as usize].to_vec();
            Ok(Some(Value::Array(result)))
        }
        "POSITION" => {
            // AQL: POSITION(anyArray, search, returnIndex?) — a boolean by
            // default, the index (or -1) when returnIndex is true.
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "POSITION requires 2-3 arguments: array, value, [returnIndex]".to_string(),
                ));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("POSITION: first argument must be an array".to_string())
            })?;
            let idx = arr.iter().position(|item| values_equal(item, &args[1]));
            if args.get(2).map(to_bool).unwrap_or(false) {
                let i = idx.map(|i| i as i64).unwrap_or(-1);
                Ok(Some(Value::Number(i.into())))
            } else {
                Ok(Some(Value::Bool(idx.is_some())))
            }
        }
        "INDEX_OF" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "INDEX_OF requires 2 arguments: array, value".to_string(),
                ));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("INDEX_OF: first argument must be an array".to_string())
            })?;
            let i = arr
                .iter()
                .position(|item| values_equal(item, &args[1]))
                .map(|i| i as i64)
                .unwrap_or(-1);
            Ok(Some(Value::Number(i.into())))
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
            let raw = as_int(&args[1])
                .or_else(|| args[1].as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
                .ok_or_else(|| {
                    DbError::ExecutionError("NTH: index must be a number".to_string())
                })?;
            Ok(Some(
                resolve_position(raw, arr.len())
                    .and_then(|i| arr.get(i))
                    .cloned()
                    .unwrap_or(Value::Null),
            ))
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
            let n = as_int(&args[1]).unwrap_or(0);
            if n <= 0 {
                return Ok(Some(Value::Array(vec![])));
            }
            let n = usize::try_from(n).unwrap_or(usize::MAX);
            Ok(Some(Value::Array(arr.iter().take(n).cloned().collect())))
        }
        "DROP" | "SKIP" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(format!(
                    "{} requires 2 arguments: array, n",
                    name
                )));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError(format!("{}: first argument must be an array", name))
            })?;
            let n = as_int(&args[1]).unwrap_or(0).max(0);
            let n = usize::try_from(n).unwrap_or(usize::MAX);
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
            let size = as_int(&args[1]).unwrap_or(0);
            if size <= 0 {
                return Err(DbError::ExecutionError(
                    "CHUNK: size must be a positive integer".to_string(),
                ));
            }
            let size = usize::try_from(size).unwrap_or(usize::MAX);
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
        // AQL: COUNT is an alias of LENGTH.
        "LENGTH" | "COUNT" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Number(length_of(&args[0]).into())))
        }
        "OUTERSECTION" | "SYMDIFF" => {
            // AQL: the values that occur exactly once across all arrays.
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "OUTERSECTION requires at least 2 array arguments".to_string(),
                ));
            }
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
            let mut entries: Vec<(&Value, usize)> = Vec::new();
            for arg in args {
                let arr = array_arg(name, arg, "every")?;
                for v in arr {
                    let bucket = buckets.entry(hash_value(v)).or_default();
                    let found = bucket
                        .iter()
                        .copied()
                        .find(|&i| values_equal(entries[i].0, v));
                    match found {
                        Some(i) => entries[i].1 += 1,
                        None => {
                            bucket.push(entries.len());
                            entries.push((v, 1));
                        }
                    }
                }
            }
            Ok(Some(Value::Array(
                entries
                    .into_iter()
                    .filter(|&(_, n)| n == 1)
                    .map(|(v, _)| v.clone())
                    .collect(),
            )))
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
            // AQL: APPEND(anyArray, values, unique). A boolean third argument
            // is the unique flag, not a value to append; beyond three
            // arguments every extra one is appended (SoliDB's variadic form).
            let (extra_args, unique) = match args {
                [_, values, Value::Bool(u)] => (std::slice::from_ref(values), *u),
                _ => (&args[1..], false),
            };
            let extra: usize = extra_args
                .iter()
                .map(|a| match a {
                    Value::Array(items) => items.len(),
                    _ => 1,
                })
                .sum();
            let mut arr = Vec::with_capacity(first.len() + extra);
            arr.extend_from_slice(first);
            for arg in extra_args {
                if let Value::Array(items) = arg {
                    arr.extend_from_slice(items);
                } else {
                    arr.push(arg.clone());
                }
            }
            if unique {
                arr = dedup_values(arr);
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
            // AQL: UNSHIFT(anyArray, value, unique). A boolean third argument
            // is the unique flag; otherwise every argument after the array is
            // prepended (SoliDB's variadic form).
            if let [_, value, Value::Bool(unique)] = args {
                if *unique && base.iter().any(|x| values_equal(x, value)) {
                    return Ok(Some(Value::Array(base.clone())));
                }
                let mut items = Vec::with_capacity(base.len() + 1);
                items.push(value.clone());
                items.extend_from_slice(base);
                return Ok(Some(Value::Array(items)));
            }
            let mut items = Vec::with_capacity(base.len() + args.len() - 1);
            items.extend_from_slice(&args[1..]);
            items.extend_from_slice(base);
            Ok(Some(Value::Array(items)))
        }
        // SoliDB's UNION has always removed duplicates (documented);
        // UNION_DISTINCT is the AQL name for exactly that.
        "UNION" | "UNION_DISTINCT" => {
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
                        return Err(DbError::ExecutionError(format!(
                            "{}: all arguments must be arrays",
                            name
                        )));
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
            let others = value_sets(name, &args[1..])?;
            // AQL: the result holds each common value once.
            let mut emitted = ValueSet::with_capacity(first.len());
            let result: Vec<Value> = first
                .iter()
                .filter(|item| others.iter().all(|s| s.contains(item)) && emitted.insert(item))
                .cloned()
                .collect();
            Ok(Some(Value::Array(result)))
        }
        "MINUS" | "DIFFERENCE" => {
            // AQL: values of the first array that are in none of the others,
            // each once.
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "MINUS requires at least 2 arguments".to_string(),
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
            let others = value_sets(name, &args[1..])?;
            let mut emitted = ValueSet::with_capacity(arr1.len());
            let result: Vec<Value> = arr1
                .iter()
                .filter(|item| !others.iter().any(|s| s.contains(item)) && emitted.insert(item))
                .cloned()
                .collect();
            Ok(Some(Value::Array(result)))
        }
        "REMOVE_VALUE" => {
            // AQL: REMOVE_VALUE(anyArray, value, limit?)
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "REMOVE_VALUE requires 2-3 arguments: array, value, [limit]".to_string(),
                ));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = array_arg(name, &args[0], "first")?;
            let mut remaining = match args.get(2) {
                None | Some(Value::Null) => usize::MAX,
                Some(v) => {
                    let n = as_int(v).ok_or_else(|| {
                        DbError::ExecutionError(
                            "REMOVE_VALUE: limit must be an integer".to_string(),
                        )
                    })?;
                    usize::try_from(n.max(0)).unwrap_or(usize::MAX)
                }
            };
            let mut out = Vec::with_capacity(arr.len());
            for v in arr {
                if remaining > 0 && values_equal(v, &args[1]) {
                    remaining -= 1;
                } else {
                    out.push(v.clone());
                }
            }
            Ok(Some(Value::Array(out)))
        }
        "REMOVE_VALUES" => {
            check_args(name, args, 2)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = array_arg(name, &args[0], "first")?;
            let removed = value_sets(name, &args[1..2])?;
            Ok(Some(Value::Array(
                arr.iter()
                    .filter(|v| !removed[0].contains(v))
                    .cloned()
                    .collect(),
            )))
        }
        "REMOVE_NTH" => {
            check_args(name, args, 2)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = array_arg(name, &args[0], "first")?;
            let pos = as_int(&args[1]).ok_or_else(|| {
                DbError::ExecutionError("REMOVE_NTH: position must be an integer".to_string())
            })?;
            let mut out = arr.clone();
            if let Some(i) = resolve_position(pos, arr.len()).filter(|&i| i < arr.len()) {
                out.remove(i);
            }
            Ok(Some(Value::Array(out)))
        }
        "REPLACE_NTH" => {
            // AQL: REPLACE_NTH(anyArray, position, replaceValue, defaultPaddingValue?)
            if args.len() < 3 || args.len() > 4 {
                return Err(DbError::ExecutionError(
                    "REPLACE_NTH requires 3-4 arguments: array, position, value, [padding]"
                        .to_string(),
                ));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = array_arg(name, &args[0], "first")?;
            let pos = as_int(&args[1]).ok_or_else(|| {
                DbError::ExecutionError("REPLACE_NTH: position must be an integer".to_string())
            })?;
            // A negative position past the start clamps to the first element.
            let idx = resolve_position(pos, arr.len()).unwrap_or(0);
            let mut out = arr.clone();
            if idx < out.len() {
                out[idx] = args[2].clone();
            } else {
                if idx - out.len() > MAX_PAD_LEN {
                    return Err(DbError::ExecutionError(format!(
                        "REPLACE_NTH: position is more than {} past the end of the array",
                        MAX_PAD_LEN
                    )));
                }
                let pad = args.get(3).cloned().unwrap_or(Value::Null);
                out.resize(idx, pad);
                out.push(args[2].clone());
            }
            Ok(Some(Value::Array(out)))
        }
        "JACCARD" => {
            // AQL: |A ∩ B| / |A ∪ B| over distinct values; two empty sets are
            // identical (1).
            check_args(name, args, 2)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let a = array_arg(name, &args[0], "first")?;
            let b = array_arg(name, &args[1], "second")?;
            let mut set_a = ValueSet::with_capacity(a.len());
            let distinct_a = a.iter().filter(|v| set_a.insert(v)).count();
            let mut set_b = ValueSet::with_capacity(b.len());
            let mut inter = 0usize;
            let mut distinct_b = 0usize;
            for v in b {
                if set_b.insert(v) {
                    distinct_b += 1;
                    if set_a.contains(v) {
                        inter += 1;
                    }
                }
            }
            let union = distinct_a + distinct_b - inter;
            let j = if union == 0 {
                1.0
            } else {
                inter as f64 / union as f64
            };
            Ok(Some(
                serde_json::Number::from_f64(j)
                    .map(Value::Number)
                    .unwrap_or(Value::Null),
            ))
        }
        "INTERLEAVE" => {
            // AQL: take one element from each array in turn until all are
            // exhausted.
            if args.is_empty() {
                return Err(DbError::ExecutionError(
                    "INTERLEAVE requires at least 1 array argument".to_string(),
                ));
            }
            let arrays = args
                .iter()
                .map(|a| array_arg(name, a, "every"))
                .collect::<DbResult<Vec<_>>>()?;
            let total: usize = arrays.iter().map(|a| a.len()).sum();
            let longest = arrays.iter().map(|a| a.len()).max().unwrap_or(0);
            let mut out = Vec::with_capacity(total);
            for i in 0..longest {
                for arr in &arrays {
                    if let Some(v) = arr.get(i) {
                        out.push(v.clone());
                    }
                }
            }
            Ok(Some(Value::Array(out)))
        }
        _ => Ok(None),
    }
}

/// One `ValueSet` per argument, each of which must be an array.
fn value_sets(name: &str, args: &[Value]) -> DbResult<Vec<ValueSet>> {
    let mut sets = Vec::with_capacity(args.len());
    for arg in args {
        let arr = match arg {
            Value::Array(a) => a,
            _ => {
                return Err(DbError::ExecutionError(format!(
                    "{}: all arguments must be arrays",
                    name
                )));
            }
        };
        let mut set = ValueSet::with_capacity(arr.len());
        for item in arr {
            set.insert(item);
        }
        sets.push(set);
    }
    Ok(sets)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, args: &[Value]) -> Value {
        evaluate(name, args)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .unwrap_or_else(|| panic!("{name} not handled"))
    }

    #[test]
    fn as_int_accepts_integral_floats() {
        assert_eq!(as_int(&json!(3)), Some(3));
        assert_eq!(as_int(&json!(3.0)), Some(3));
        assert_eq!(as_int(&json!(-2.0)), Some(-2));
        assert_eq!(as_int(&json!(2.5)), None);
        assert_eq!(as_int(&json!("3")), None);
        assert_eq!(as_int(&Value::Null), None);
        assert_eq!(as_int(&json!(u64::MAX)), Some(i64::MAX));
        assert_eq!(as_int(&json!(1e300)), None);
    }

    #[test]
    fn integer_arguments_computed_as_floats() {
        let a = json!([1, 2, 3, 4, 5]);
        assert_eq!(call("TAKE", &[a.clone(), json!(2.0)]), json!([1, 2]));
        assert_eq!(call("DROP", &[a.clone(), json!(3.0)]), json!([4, 5]));
        assert_eq!(call("SKIP", &[a.clone(), json!(3.0)]), json!([4, 5]));
        assert_eq!(call("SLICE", &[a.clone(), json!(2.0)]), json!([3, 4, 5]));
        assert_eq!(
            call("CHUNK", &[a.clone(), json!(2.0)]),
            json!([[1, 2], [3, 4], [5]])
        );
        assert_eq!(call("NTH", &[a, json!(-1.0)]), json!(5));
    }

    #[test]
    fn slice_negative_length_is_an_end_offset() {
        let a = json!([1, 2, 3, 4, 5]);
        assert_eq!(
            call("SLICE", &[a.clone(), json!(0), json!(-2)]),
            json!([1, 2, 3])
        );
        assert_eq!(
            call("SLICE", &[a.clone(), json!(1), json!(-1)]),
            json!([2, 3, 4])
        );
        assert_eq!(call("SLICE", &[a.clone(), json!(4), json!(-3)]), json!([]));
        assert_eq!(call("SLICE", &[a, json!(-2), json!(1)]), json!([4]));
    }

    #[test]
    fn length_and_count_follow_aql() {
        for f in ["LENGTH", "COUNT"] {
            assert_eq!(call(f, &[json!([1, 2])]), json!(2));
            assert_eq!(call(f, &[json!({"a": 1})]), json!(1));
            assert_eq!(call(f, &[json!("héllo")]), json!(5));
            assert_eq!(call(f, &[Value::Null]), json!(0));
            assert_eq!(call(f, &[json!(true)]), json!(1));
            assert_eq!(call(f, &[json!(false)]), json!(0));
            assert_eq!(call(f, &[json!(1234)]), json!(4));
            assert_eq!(call(f, &[json!(-1.5)]), json!(4));
        }
    }

    #[test]
    fn last_of_null_is_null() {
        assert_eq!(call("LAST", &[Value::Null]), Value::Null);
    }

    #[test]
    fn unique_flags_on_push_append_unshift() {
        assert_eq!(
            call("PUSH", &[json!([1, 2]), json!(2), json!(true)]),
            json!([1, 2])
        );
        assert_eq!(call("PUSH", &[json!([1, 2]), json!(2)]), json!([1, 2, 2]));
        assert_eq!(
            call("APPEND", &[json!([1, 2]), json!([2, 3, 3]), json!(true)]),
            json!([1, 2, 3])
        );
        assert_eq!(
            call("APPEND", &[json!([1]), json!([2]), json!(false)]),
            json!([1, 2])
        );
        // Variadic form still appends every argument.
        assert_eq!(
            call("APPEND", &[json!([1]), json!(2), json!(3), json!(4)]),
            json!([1, 2, 3, 4])
        );
        assert_eq!(
            call("UNSHIFT", &[json!([1, 2]), json!(1), json!(true)]),
            json!([1, 2])
        );
        assert_eq!(
            call("UNSHIFT", &[json!([1, 2]), json!(0), json!(false)]),
            json!([0, 1, 2])
        );
        assert_eq!(
            call("UNSHIFT", &[json!([3]), json!(1), json!(2)]),
            json!([1, 2, 3])
        );
    }

    #[test]
    fn set_operations_are_nary_and_distinct() {
        assert_eq!(
            call("INTERSECTION", &[json!([1, 1, 2, 3]), json!([1, 2, 2])]),
            json!([1, 2])
        );
        assert_eq!(
            call("MINUS", &[json!([1, 2, 2, 3, 4]), json!([2]), json!([4])]),
            json!([1, 3])
        );
        assert_eq!(
            call(
                "OUTERSECTION",
                &[json!([1, 2]), json!([2, 3]), json!([3, 4])]
            ),
            json!([1, 4])
        );
        assert_eq!(
            call("OUTERSECTION", &[json!([1, 2]), json!([2, 3])]),
            json!([1, 3])
        );
        assert_eq!(
            call("UNION_DISTINCT", &[json!([1, 2]), json!([2, 3])]),
            json!([1, 2, 3])
        );
    }

    #[test]
    fn position_follows_aql() {
        let a = json!([10, 20, 30]);
        assert_eq!(call("POSITION", &[a.clone(), json!(20)]), json!(true));
        assert_eq!(call("POSITION", &[a.clone(), json!(40)]), json!(false));
        assert_eq!(
            call("POSITION", &[a.clone(), json!(20), json!(true)]),
            json!(1)
        );
        assert_eq!(
            call("POSITION", &[a.clone(), json!(40), json!(true)]),
            json!(-1)
        );
        assert_eq!(call("INDEX_OF", &[a, json!(30)]), json!(2));
    }

    #[test]
    fn sorted_unique_and_removals() {
        assert_eq!(
            call("SORTED_UNIQUE", &[json!([3, 1, 2, 2, 1])]),
            json!([1, 2, 3])
        );
        assert_eq!(
            call("REMOVE_VALUE", &[json!([1, 2, 1, 3, 1]), json!(1)]),
            json!([2, 3])
        );
        assert_eq!(
            call(
                "REMOVE_VALUE",
                &[json!([1, 2, 1, 3, 1]), json!(1), json!(2)]
            ),
            json!([2, 3, 1])
        );
        assert_eq!(
            call("REMOVE_VALUES", &[json!([1, 2, 3, 4]), json!([2, 4])]),
            json!([1, 3])
        );
        assert_eq!(
            call("REMOVE_NTH", &[json!([1, 2, 3]), json!(1)]),
            json!([1, 3])
        );
        assert_eq!(
            call("REMOVE_NTH", &[json!([1, 2, 3]), json!(-1)]),
            json!([1, 2])
        );
        assert_eq!(
            call("REMOVE_NTH", &[json!([1, 2, 3]), json!(9)]),
            json!([1, 2, 3])
        );
    }

    #[test]
    fn replace_nth_pads() {
        let a = json!(["a", "b", "c"]);
        assert_eq!(
            call("REPLACE_NTH", &[a.clone(), json!(1), json!("z")]),
            json!(["a", "z", "c"])
        );
        assert_eq!(
            call("REPLACE_NTH", &[a.clone(), json!(-1), json!("z")]),
            json!(["a", "b", "z"])
        );
        assert_eq!(
            call("REPLACE_NTH", &[a.clone(), json!(3), json!("z")]),
            json!(["a", "b", "c", "z"])
        );
        assert_eq!(
            call(
                "REPLACE_NTH",
                &[a.clone(), json!(5), json!("z"), json!("y")]
            ),
            json!(["a", "b", "c", "y", "y", "z"])
        );
        assert!(evaluate("REPLACE_NTH", &[a, json!(i64::MAX), json!(1)]).is_err());
    }

    #[test]
    fn jaccard_and_interleave() {
        assert_eq!(
            call("JACCARD", &[json!([1, 2, 3]), json!([2, 3, 4])]),
            json!(0.5)
        );
        assert_eq!(call("JACCARD", &[json!([]), json!([])]), json!(1.0));
        assert_eq!(
            call("INTERLEAVE", &[json!([1, 1, 1]), json!([2, 2]), json!([3])]),
            json!([1, 2, 3, 1, 2, 1])
        );
    }
}
