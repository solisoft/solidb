//! Array builtin functions.
//!
//! Set-like functions (UNIQUE, UNION, INTERSECTION, MINUS, POSITION, …)
//! compare values with [`values_equal`] and hash them with [`ValueSet`], so
//! `1` and `1.0` are the same value and nothing goes through `to_string`.

use serde_json::Value;

use super::common::{as_int, check_arity, err, int_arg, length_of, numbers_of, MAX_RANGE};
use crate::error::SdbqlResult;
use crate::executor::helpers::{compare_values, number_from_f64, values_equal, ValueSet};

fn array_arg<'a>(name: &str, args: &'a [Value], i: usize) -> SdbqlResult<&'a Vec<Value>> {
    args.get(i)
        .and_then(Value::as_array)
        .ok_or_else(|| err(format!("{}: argument {} must be an array", name, i + 1)))
}

/// A trailing boolean at position `i` is AQL's `unique` flag (PUSH,
/// APPEND, UNSHIFT); `None` when there is no such flag.
fn unique_flag(args: &[Value], i: usize) -> Option<bool> {
    if args.len() == i + 1 {
        args[i].as_bool()
    } else {
        None
    }
}

/// Resolve a possibly negative index against `len`; `None` when out of range.
fn resolve_index(idx: i64, len: usize) -> Option<usize> {
    let i = if idx < 0 {
        (len as i64).checked_add(idx)?
    } else {
        idx
    };
    usize::try_from(i).ok().filter(|&i| i < len)
}

fn dedup(items: impl IntoIterator<Item = Value>) -> Vec<Value> {
    let mut seen = ValueSet::default();
    items.into_iter().filter(|v| seen.insert(v)).collect()
}

/// AQL SLICE: `start` may be negative (from the end); `length` is a count,
/// and a negative `length` is an end offset from the end (exclusive).
fn slice(arr: &[Value], start: i64, length: Option<i64>) -> Vec<Value> {
    let n = arr.len() as i64;
    let s = if start < 0 {
        n.saturating_add(start).max(0)
    } else {
        start.min(n)
    };
    let e = match length {
        None => n,
        Some(l) if l < 0 => n.saturating_add(l),
        Some(l) => s.saturating_add(l).min(n),
    };
    if e <= s {
        return Vec::new();
    }
    arr[s as usize..e as usize].to_vec()
}

fn range(args: &[Value]) -> SdbqlResult<Value> {
    check_arity("RANGE", args, 1, 3)?;
    let is_float = args
        .iter()
        .any(|v| v.as_f64().is_some_and(|f| f.fract() != 0.0));
    if is_float {
        return range_float(args);
    }
    let get = |i: usize| -> SdbqlResult<i64> {
        args.get(i)
            .and_then(as_int)
            .ok_or_else(|| err("RANGE: arguments must be numbers"))
    };
    let (start, end, step) = match args.len() {
        1 => (0, get(0)?, 1),
        2 => (get(0)?, get(1)?, 1),
        _ => (get(0)?, get(1)?, get(2)?),
    };
    if step == 0 {
        return Err(err("RANGE: step cannot be 0"));
    }
    // i128: `end - start` overflows i64 across the full range.
    let (s, e, st) = (i128::from(start), i128::from(end), i128::from(step));
    let count = if (st > 0 && e < s) || (st < 0 && e > s) {
        0
    } else {
        (e - s) / st + 1
    };
    if count > MAX_RANGE as i128 {
        return Err(err(format!(
            "RANGE: result would have {} elements (max {})",
            count, MAX_RANGE
        )));
    }
    let out = (0..count)
        .map(|k| Value::from((s + k * st) as i64))
        .collect();
    Ok(Value::Array(out))
}

fn range_float(args: &[Value]) -> SdbqlResult<Value> {
    let get = |i: usize| -> SdbqlResult<f64> {
        args.get(i)
            .and_then(Value::as_f64)
            .ok_or_else(|| err("RANGE: arguments must be numbers"))
    };
    let (start, end, step) = match args.len() {
        1 => (0.0, get(0)?, 1.0),
        2 => (get(0)?, get(1)?, 1.0),
        _ => (get(0)?, get(1)?, get(2)?),
    };
    if step == 0.0 {
        return Err(err("RANGE: step cannot be 0"));
    }
    let span = (end - start) / step;
    if span < 0.0 {
        return Ok(Value::Array(vec![]));
    }
    // A small epsilon so RANGE(0, 1, 0.1) includes 1.
    let count = (span + 1e-9).floor() + 1.0;
    if !count.is_finite() || count > MAX_RANGE as f64 {
        return Err(err(format!(
            "RANGE: result would exceed {} elements",
            MAX_RANGE
        )));
    }
    let out = (0..count as usize)
        .map(|k| Value::Number(number_from_f64(start + k as f64 * step)))
        .collect();
    Ok(Value::Array(out))
}

/// Call an array function. Returns None if function not found.
pub fn call(name: &str, args: &[Value]) -> SdbqlResult<Option<Value>> {
    let result = match name {
        "FIRST" => match args.first() {
            Some(Value::Array(arr)) => Some(arr.first().cloned().unwrap_or(Value::Null)),
            _ => Some(Value::Null),
        },

        "LAST" => match args.first() {
            Some(Value::Array(arr)) => Some(arr.last().cloned().unwrap_or(Value::Null)),
            _ => Some(Value::Null),
        },

        "NTH" => {
            let arr = args.first().and_then(|v| v.as_array());
            let n = int_arg(args, 1, 0);
            match arr {
                // `len + n` used to wrap to a huge usize (or overflow).
                Some(arr) => Some(
                    resolve_index(n, arr.len())
                        .and_then(|i| arr.get(i).cloned())
                        .unwrap_or(Value::Null),
                ),
                _ => Some(Value::Null),
            }
        }

        "SORTED" | "SORT" => match args.first() {
            Some(Value::Array(arr)) => {
                let mut sorted = arr.clone();
                sorted.sort_by(compare_values);
                Some(Value::Array(sorted))
            }
            _ => Some(Value::Null),
        },

        "SORTED_DESC" => match args.first() {
            Some(Value::Array(arr)) => {
                let mut sorted = arr.clone();
                sorted.sort_by(|a, b| compare_values(b, a));
                Some(Value::Array(sorted))
            }
            _ => Some(Value::Null),
        },

        "SORTED_UNIQUE" => match args.first() {
            Some(Value::Array(arr)) => {
                let mut out = dedup(arr.iter().cloned());
                out.sort_by(compare_values);
                Some(Value::Array(out))
            }
            _ => Some(Value::Null),
        },

        "UNIQUE" => match args.first() {
            Some(Value::Array(arr)) => Some(Value::Array(dedup(arr.iter().cloned()))),
            _ => Some(Value::Null),
        },

        "FLATTEN" => match args.first() {
            Some(Value::Array(arr)) => {
                let depth = int_arg(args, 1, 1).max(0) as usize;
                Some(Value::Array(flatten_array(arr, depth)))
            }
            _ => Some(Value::Null),
        },

        "PUSH" => {
            // AQL PUSH(array, value, unique). Without the flag, extra values
            // are all appended (older SoliDB form).
            check_arity(name, args, 2, usize::MAX)?;
            match &args[0] {
                Value::Null => Some(Value::Null),
                Value::Array(arr) => {
                    let mut result = arr.clone();
                    match unique_flag(args, 2) {
                        Some(unique) => {
                            if !unique || !arr.iter().any(|x| values_equal(x, &args[1])) {
                                result.push(args[1].clone());
                            }
                        }
                        None => result.extend(args[1..].iter().cloned()),
                    }
                    Some(Value::Array(result))
                }
                _ => return Err(err("PUSH: first argument must be an array")),
            }
        }

        "APPEND" => {
            // AQL APPEND(array, values, unique); also variadic.
            check_arity(name, args, 1, usize::MAX)?;
            let mut result = match &args[0] {
                Value::Null => Vec::new(),
                Value::Array(arr) => arr.clone(),
                other => vec![other.clone()],
            };
            let flag = unique_flag(args, 2);
            let unique = flag == Some(true);
            let rest = if flag.is_some() {
                &args[1..2]
            } else {
                &args[1..]
            };
            for arg in rest {
                match arg {
                    Value::Array(arr) => result.extend(arr.iter().cloned()),
                    Value::Null if unique => {}
                    other => result.push(other.clone()),
                }
            }
            if unique {
                result = dedup(result);
            }
            Some(Value::Array(result))
        }

        "UNSHIFT" => {
            // AQL UNSHIFT(array, value, unique).
            check_arity(name, args, 2, usize::MAX)?;
            match &args[0] {
                Value::Null => Some(Value::Null),
                Value::Array(arr) => {
                    if let Some(unique) = unique_flag(args, 2) {
                        if unique && arr.iter().any(|x| values_equal(x, &args[1])) {
                            return Ok(Some(Value::Array(arr.clone())));
                        }
                        let mut result = Vec::with_capacity(arr.len() + 1);
                        result.push(args[1].clone());
                        result.extend(arr.iter().cloned());
                        Some(Value::Array(result))
                    } else {
                        let mut result: Vec<Value> = args[1..].to_vec();
                        result.extend(arr.iter().cloned());
                        Some(Value::Array(result))
                    }
                }
                _ => return Err(err("UNSHIFT: first argument must be an array")),
            }
        }

        "POP" => match args.first() {
            Some(Value::Array(arr)) => {
                Some(Value::Array(arr[..arr.len().saturating_sub(1)].to_vec()))
            }
            _ => Some(Value::Null),
        },

        "SHIFT" => match args.first() {
            Some(Value::Array(arr)) => Some(Value::Array(arr.iter().skip(1).cloned().collect())),
            _ => Some(Value::Null),
        },

        "SLICE" => {
            check_arity(name, args, 2, 3)?;
            match &args[0] {
                Value::Array(arr) => {
                    let length = args
                        .get(2)
                        .filter(|v| !v.is_null())
                        .map(|v| as_int(v).unwrap_or(0));
                    Some(Value::Array(slice(arr, int_arg(args, 1, 0), length)))
                }
                _ => Some(Value::Null),
            }
        }

        "UNION" | "UNION_DISTINCT" => {
            // SoliDB's documented UNION removes duplicates (AQL's UNION does
            // not; AQL's UNION_DISTINCT does). Both names dedup here, as on
            // the server.
            let mut seen = ValueSet::default();
            let mut result = Vec::new();
            for (i, arg) in args.iter().enumerate() {
                let arr = match arg {
                    Value::Array(a) => a,
                    Value::Null => continue,
                    _ => {
                        return Err(err(format!(
                            "{}: argument {} must be an array",
                            name,
                            i + 1
                        )))
                    }
                };
                for item in arr {
                    if seen.insert(item) {
                        result.push(item.clone());
                    }
                }
            }
            Some(Value::Array(result))
        }

        "INTERSECTION" => {
            check_arity(name, args, 1, usize::MAX)?;
            let first = array_arg(name, args, 0)?;
            let mut others = Vec::with_capacity(args.len() - 1);
            for i in 1..args.len() {
                let mut set = ValueSet::default();
                for v in array_arg(name, args, i)? {
                    set.insert(v);
                }
                others.push(set);
            }
            // AQL returns each common value once.
            let mut seen = ValueSet::default();
            let result: Vec<Value> = first
                .iter()
                .filter(|v| others.iter().all(|s| s.contains(v)) && seen.insert(v))
                .cloned()
                .collect();
            Some(Value::Array(result))
        }

        "MINUS" | "DIFFERENCE" => {
            check_arity(name, args, 1, usize::MAX)?;
            let first = array_arg(name, args, 0)?;
            let mut exclude = ValueSet::default();
            for i in 1..args.len() {
                for v in array_arg(name, args, i)? {
                    exclude.insert(v);
                }
            }
            let mut seen = ValueSet::default();
            let result: Vec<Value> = first
                .iter()
                .filter(|v| !exclude.contains(v) && seen.insert(v))
                .cloned()
                .collect();
            Some(Value::Array(result))
        }

        "OUTERSECTION" | "SYMDIFF" => {
            // AQL: the values that occur in exactly one of the arrays.
            check_arity(name, args, 1, usize::MAX)?;
            let mut sets = Vec::with_capacity(args.len());
            for i in 0..args.len() {
                let mut set = ValueSet::default();
                for v in array_arg(name, args, i)? {
                    set.insert(v);
                }
                sets.push(set);
            }
            let mut seen = ValueSet::default();
            let mut out = Vec::new();
            for i in 0..args.len() {
                for v in array_arg(name, args, i)? {
                    if sets.iter().filter(|s| s.contains(v)).count() == 1 && seen.insert(v) {
                        out.push(v.clone());
                    }
                }
            }
            Some(Value::Array(out))
        }

        "JACCARD" => {
            check_arity(name, args, 2, 2)?;
            let a = dedup(array_arg(name, args, 0)?.iter().cloned());
            let b = array_arg(name, args, 1)?;
            let mut set_b = ValueSet::default();
            let mut union = a.len();
            for v in b {
                if set_b.insert(v) && !a.iter().any(|x| values_equal(x, v)) {
                    union += 1;
                }
            }
            let inter = a.iter().filter(|v| set_b.contains(v)).count();
            let j = if union == 0 {
                1.0
            } else {
                inter as f64 / union as f64
            };
            Some(Value::Number(number_from_f64(j)))
        }

        "INTERLEAVE" => {
            let mut arrays = Vec::with_capacity(args.len());
            for i in 0..args.len() {
                arrays.push(array_arg(name, args, i)?);
            }
            let longest = arrays.iter().map(|a| a.len()).max().unwrap_or(0);
            let mut out = Vec::with_capacity(arrays.iter().map(|a| a.len()).sum());
            for k in 0..longest {
                for a in &arrays {
                    if let Some(v) = a.get(k) {
                        out.push(v.clone());
                    }
                }
            }
            Some(Value::Array(out))
        }

        "POSITION" | "INDEX_OF" => {
            // SoliDB documents an index result (-1 when absent) and an
            // optional start position.
            check_arity(name, args, 2, 3)?;
            match &args[0] {
                Value::Array(arr) => {
                    let start = match args.get(2) {
                        Some(v) if v.is_number() => {
                            let s = as_int(v).unwrap_or(0);
                            if s < 0 {
                                (arr.len() as i64).saturating_add(s).max(0) as usize
                            } else {
                                s as usize
                            }
                        }
                        _ => 0,
                    };
                    let pos = arr
                        .iter()
                        .enumerate()
                        .skip(start)
                        .find(|(_, item)| values_equal(item, &args[1]))
                        .map(|(i, _)| i as i64)
                        .unwrap_or(-1);
                    Some(Value::from(pos))
                }
                _ => Some(Value::Null),
            }
        }

        "CONTAINS_ARRAY" | "CONTAINS" => {
            check_arity(name, args, 2, 2)?;
            match &args[0] {
                Value::Null => Some(Value::Null),
                Value::Array(arr) => Some(Value::Bool(
                    arr.iter().any(|item| values_equal(item, &args[1])),
                )),
                _ => Some(Value::Bool(false)),
            }
        }

        "REMOVE_VALUE" => {
            check_arity(name, args, 2, 3)?;
            match &args[0] {
                Value::Null => Some(Value::Null),
                Value::Array(arr) => {
                    let mut limit = match args.get(2).and_then(as_int) {
                        Some(n) if n >= 0 => n as usize,
                        _ => usize::MAX,
                    };
                    let mut out = Vec::with_capacity(arr.len());
                    for v in arr {
                        if limit > 0 && values_equal(v, &args[1]) {
                            limit -= 1;
                        } else {
                            out.push(v.clone());
                        }
                    }
                    Some(Value::Array(out))
                }
                _ => return Err(err("REMOVE_VALUE: first argument must be an array")),
            }
        }

        "REMOVE_VALUES" => {
            check_arity(name, args, 2, 2)?;
            match &args[0] {
                Value::Null => Some(Value::Null),
                Value::Array(arr) => {
                    let mut drop = ValueSet::default();
                    match &args[1] {
                        Value::Array(vals) => vals.iter().for_each(|v| {
                            drop.insert(v);
                        }),
                        Value::Null => {}
                        other => {
                            drop.insert(other);
                        }
                    }
                    Some(Value::Array(
                        arr.iter().filter(|v| !drop.contains(v)).cloned().collect(),
                    ))
                }
                _ => return Err(err("REMOVE_VALUES: first argument must be an array")),
            }
        }

        "REMOVE_NTH" => {
            check_arity(name, args, 2, 2)?;
            let arr = array_arg(name, args, 0)?;
            let mut out = arr.clone();
            if let Some(i) = resolve_index(int_arg(args, 1, 0), arr.len()) {
                out.remove(i);
            }
            Some(Value::Array(out))
        }

        "REPLACE_NTH" => {
            // AQL REPLACE_NTH(array, position, value, padding): a position
            // past the end pads with `padding` (or appends when omitted).
            check_arity(name, args, 3, 4)?;
            let arr = array_arg(name, args, 0)?;
            let pos = int_arg(args, 1, 0);
            let mut out = arr.clone();
            if let Some(i) = resolve_index(pos, arr.len()) {
                out[i] = args[2].clone();
            } else if pos >= 0 {
                if let Some(pad) = args.get(3) {
                    let target = pos as usize;
                    if target > MAX_RANGE {
                        return Err(err(format!(
                            "REPLACE_NTH: position {} is too large (max {})",
                            target, MAX_RANGE
                        )));
                    }
                    out.resize(target, pad.clone());
                }
                out.push(args[2].clone());
            } else {
                // A negative position before the start replaces the first.
                if out.is_empty() {
                    out.push(args[2].clone());
                } else {
                    out[0] = args[2].clone();
                }
            }
            Some(Value::Array(out))
        }

        "TAKE" => {
            check_arity(name, args, 2, 2)?;
            match &args[0] {
                Value::Null => Some(Value::Null),
                Value::Array(arr) => {
                    let n = int_arg(args, 1, 0).max(0) as usize;
                    Some(Value::Array(arr.iter().take(n).cloned().collect()))
                }
                _ => return Err(err("TAKE: first argument must be an array")),
            }
        }

        "DROP" | "SKIP" => {
            check_arity(name, args, 2, 2)?;
            match &args[0] {
                Value::Null => Some(Value::Null),
                Value::Array(arr) => {
                    let n = int_arg(args, 1, 0).max(0) as usize;
                    Some(Value::Array(arr.iter().skip(n).cloned().collect()))
                }
                _ => return Err(err(format!("{}: first argument must be an array", name))),
            }
        }

        "CHUNK" => {
            check_arity(name, args, 2, 2)?;
            match &args[0] {
                Value::Null => Some(Value::Null),
                Value::Array(arr) => {
                    let size = int_arg(args, 1, 0);
                    if size <= 0 {
                        return Err(err("CHUNK: size must be a positive integer"));
                    }
                    Some(Value::Array(
                        arr.chunks(size as usize)
                            .map(|c| Value::Array(c.to_vec()))
                            .collect(),
                    ))
                }
                _ => return Err(err("CHUNK: first argument must be an array")),
            }
        }

        "COUNT" => {
            // AQL: COUNT is an alias of LENGTH. (AGGREGATE COUNT(expr), which
            // counts non-null values, is handled by COLLECT.)
            check_arity(name, args, 1, 1)?;
            Some(Value::from(length_of(&args[0])))
        }

        "COUNT_DISTINCT" | "COUNT_UNIQUE" | "UNIQUE_COUNT" => match args.first() {
            Some(Value::Array(arr)) => {
                let mut seen = ValueSet::default();
                Some(Value::from(
                    arr.iter()
                        .filter(|v| !v.is_null() && seen.insert(v))
                        .count(),
                ))
            }
            _ => Some(Value::Null),
        },

        "SUM" => match args.first() {
            Some(Value::Array(arr)) => {
                let sum: f64 = numbers_of(arr).iter().sum();
                Some(Value::Number(number_from_f64(sum)))
            }
            _ => Some(Value::Number(serde_json::Number::from(0))),
        },

        "PRODUCT" => match args.first() {
            Some(Value::Array(arr)) => {
                let p: f64 = numbers_of(arr).iter().product();
                Some(Value::Number(number_from_f64(p)))
            }
            _ => Some(Value::Null),
        },

        "AVG" | "AVERAGE" => match args.first() {
            Some(Value::Array(arr)) => {
                let values = numbers_of(arr);
                if values.is_empty() {
                    return Ok(Some(Value::Null));
                }
                let avg = values.iter().sum::<f64>() / values.len() as f64;
                Some(Value::Number(number_from_f64(avg)))
            }
            _ => Some(Value::Null),
        },

        "MIN" | "MINIMUM" | "MAX" | "MAXIMUM" => {
            // MIN(array), or MIN(a, b, ...) over the arguments.
            let items: &[Value] = match args {
                [Value::Array(arr)] => arr.as_slice(),
                [_] | [] => return Ok(Some(Value::Null)),
                many => many,
            };
            let values = items.iter().filter(|v| !v.is_null());
            let found = if name.starts_with("MIN") {
                values.min_by(|a, b| compare_values(a, b))
            } else {
                values.max_by(|a, b| compare_values(a, b))
            };
            Some(found.cloned().unwrap_or(Value::Null))
        }

        "RANGE" => Some(range(args)?),

        "ZIP" => {
            let arrays: Vec<&Vec<Value>> = args.iter().filter_map(|v| v.as_array()).collect();
            if arrays.is_empty() {
                return Ok(Some(Value::Array(vec![])));
            }

            let min_len = arrays.iter().map(|a| a.len()).min().unwrap_or(0);
            let mut result = Vec::with_capacity(min_len);

            for i in 0..min_len {
                let tuple: Vec<Value> = arrays.iter().map(|a| a[i].clone()).collect();
                result.push(Value::Array(tuple));
            }

            Some(Value::Array(result))
        }

        "ZIP_OBJECT" => {
            let keys = args.first().and_then(Value::as_array);
            let vals = args.get(1).and_then(Value::as_array);
            match (keys, vals) {
                (Some(k), Some(v)) => {
                    let mut obj = serde_json::Map::new();
                    for (kk, vv) in k.iter().zip(v.iter()) {
                        if let Some(s) = kk.as_str() {
                            obj.insert(s.to_string(), vv.clone());
                        }
                    }
                    Some(Value::Object(obj))
                }
                _ => Some(Value::Null),
            }
        }

        _ => None,
    };

    Ok(result)
}

fn flatten_array(arr: &[Value], depth: usize) -> Vec<Value> {
    let mut result = Vec::with_capacity(arr.len());
    flatten_into(arr, depth, &mut result);
    result
}

fn flatten_into(arr: &[Value], depth: usize, out: &mut Vec<Value>) {
    for item in arr {
        match item {
            Value::Array(inner) if depth > 0 => flatten_into(inner, depth - 1, out),
            _ => out.push(item.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ok(name: &str, args: &[Value]) -> Value {
        call(name, args).unwrap().unwrap()
    }

    #[test]
    fn test_first_last() {
        assert_eq!(call("FIRST", &[json!([1, 2, 3])]).unwrap(), Some(json!(1)));
        assert_eq!(call("LAST", &[json!([1, 2, 3])]).unwrap(), Some(json!(3)));
        assert_eq!(call("FIRST", &[json!([])]).unwrap(), Some(Value::Null));
    }

    #[test]
    fn test_sorted() {
        assert_eq!(
            call("SORTED", &[json!([3, 1, 2])]).unwrap(),
            Some(json!([1, 2, 3]))
        );
        // AQL type order: null < bool < number < string < array < object.
        assert_eq!(
            ok("SORTED", &[json!([{"a": 1}, [1], "a", 1, true, null])]),
            json!([null, true, 1, "a", [1], {"a": 1}])
        );
    }

    #[test]
    fn test_unique() {
        assert_eq!(
            call("UNIQUE", &[json!([1, 2, 2, 3, 1])]).unwrap(),
            Some(json!([1, 2, 3]))
        );
        // 1 and 1.0 are the same value.
        assert_eq!(
            ok("UNIQUE", &[json!([1, 1.0, [1], [1.0]])]),
            json!([1, [1]])
        );
        assert_eq!(
            ok("SORTED_UNIQUE", &[json!([3, 1, 2, 2])]),
            json!([1, 2, 3])
        );
    }

    #[test]
    fn test_flatten() {
        assert_eq!(
            call("FLATTEN", &[json!([[1, 2], [3, 4]])]).unwrap(),
            Some(json!([1, 2, 3, 4]))
        );
        assert_eq!(
            ok("FLATTEN", &[json!([[1, [2]]]), json!(-3)]),
            json!([[1, [2]]])
        );
    }

    #[test]
    fn test_slice() {
        // The third argument is a LENGTH (AQL), not an end index: this
        // test used to assert [2, 3, 4] for SLICE(a, 1, 4).
        assert_eq!(
            call("SLICE", &[json!([1, 2, 3, 4, 5]), json!(1), json!(3)]).unwrap(),
            Some(json!([2, 3, 4]))
        );
        assert_eq!(
            call("SLICE", &[json!([1, 2, 3, 4, 5]), json!(1), json!(4)]).unwrap(),
            Some(json!([2, 3, 4, 5]))
        );
        assert_eq!(
            call("SLICE", &[json!([1, 2, 3, 4, 5]), json!(-2)]).unwrap(),
            Some(json!([4, 5]))
        );
    }

    #[test]
    fn slice_never_panics() {
        let a = json!([1, 2, 3, 4, 5]);
        // Was a panic: start 3 > end 1.
        assert_eq!(ok("SLICE", &[a.clone(), json!(3), json!(1)]), json!([4]));
        // Negative length is an end offset from the end.
        assert_eq!(
            ok("SLICE", &[a.clone(), json!(0), json!(-2)]),
            json!([1, 2, 3])
        );
        assert_eq!(ok("SLICE", &[a.clone(), json!(3), json!(-4)]), json!([]));
        assert_eq!(ok("SLICE", &[a.clone(), json!(99)]), json!([]));
        assert_eq!(
            ok("SLICE", &[a.clone(), json!(-99), json!(2)]),
            json!([1, 2])
        );
        assert_eq!(
            ok("SLICE", &[a.clone(), json!(i64::MIN), json!(i64::MAX)]),
            json!([1, 2, 3, 4, 5])
        );
        assert_eq!(
            ok("SLICE", &[a, json!(i64::MAX), json!(i64::MIN)]),
            json!([])
        );
    }

    #[test]
    fn test_push_pop() {
        assert_eq!(
            call("PUSH", &[json!([1, 2]), json!(3)]).unwrap(),
            Some(json!([1, 2, 3]))
        );
        assert_eq!(
            call("POP", &[json!([1, 2, 3])]).unwrap(),
            Some(json!([1, 2]))
        );
    }

    #[test]
    fn unique_flag_is_not_a_value() {
        assert_eq!(
            ok("PUSH", &[json!([1, 2]), json!(2), json!(true)]),
            json!([1, 2])
        );
        assert_eq!(
            ok("PUSH", &[json!([1, 2]), json!(3), json!(true)]),
            json!([1, 2, 3])
        );
        assert_eq!(
            ok("PUSH", &[json!([1]), json!(1), json!(false)]),
            json!([1, 1])
        );
        assert_eq!(
            ok("UNSHIFT", &[json!([1, 2]), json!(1), json!(true)]),
            json!([1, 2])
        );
        assert_eq!(
            ok("APPEND", &[json!([1, 2]), json!([2, 3]), json!(true)]),
            json!([1, 2, 3])
        );
        assert_eq!(ok("APPEND", &[json!([1]), json!([2, 3])]), json!([1, 2, 3]));
    }

    #[test]
    fn set_functions_use_value_equality() {
        assert_eq!(
            ok("UNION", &[json!([1, 2]), json!([2.0, 3])]),
            json!([1, 2, 3])
        );
        assert!(call("UNION", &[json!([1]), json!("x")]).is_err());
        assert_eq!(
            ok("INTERSECTION", &[json!([1, 1, 2, 3]), json!([1.0, 3, 3])]),
            json!([1, 3])
        );
        assert_eq!(
            ok("MINUS", &[json!([1, 2, 3]), json!([2.0])]),
            json!([1, 3])
        );
        assert_eq!(
            ok(
                "OUTERSECTION",
                &[json!([1, 2, 3]), json!([2, 3, 4]), json!([3, 4, 5])]
            ),
            json!([1, 5])
        );
        assert_eq!(ok("POSITION", &[json!([10, 20]), json!(20.0)]), json!(1));
        assert_eq!(ok("POSITION", &[json!([10, 20]), json!(30)]), json!(-1));
        assert_eq!(
            ok("POSITION", &[json!([1, 2, 1]), json!(1), json!(1)]),
            json!(2)
        );
        assert_eq!(
            ok("CONTAINS_ARRAY", &[json!([[1]]), json!([1.0])]),
            json!(true)
        );
        assert_eq!(
            ok("JACCARD", &[json!([1, 2]), json!([2, 3])]),
            json!(1.0 / 3.0)
        );
        assert_eq!(
            ok("INTERLEAVE", &[json!([1, 3]), json!([2, 4, 5])]),
            json!([1, 2, 3, 4, 5])
        );
    }

    #[test]
    fn remove_and_replace() {
        assert_eq!(
            ok("REMOVE_VALUE", &[json!([1, 2, 1, 3]), json!(1)]),
            json!([2, 3])
        );
        assert_eq!(
            ok("REMOVE_VALUE", &[json!([1, 2, 1, 3]), json!(1), json!(1)]),
            json!([2, 1, 3])
        );
        assert_eq!(
            ok("REMOVE_VALUES", &[json!([1, 2, 3]), json!([1, 3])]),
            json!([2])
        );
        assert_eq!(
            ok("REMOVE_NTH", &[json!([1, 2, 3]), json!(-1)]),
            json!([1, 2])
        );
        assert_eq!(
            ok("REMOVE_NTH", &[json!([1, 2, 3]), json!(i64::MIN)]),
            json!([1, 2, 3])
        );
        assert_eq!(
            ok(
                "REPLACE_NTH",
                &[json!([1, 2]), json!(4), json!(9), json!(0)]
            ),
            json!([1, 2, 0, 0, 9])
        );
        assert!(call(
            "REPLACE_NTH",
            &[json!([]), json!(i64::MAX), json!(1), json!(0)]
        )
        .is_err());
    }

    #[test]
    fn take_drop_chunk() {
        assert_eq!(ok("TAKE", &[json!([1, 2, 3]), json!(2)]), json!([1, 2]));
        assert_eq!(ok("TAKE", &[json!([1, 2, 3]), json!(-1)]), json!([]));
        assert_eq!(ok("DROP", &[json!([1, 2, 3]), json!(2)]), json!([3]));
        assert_eq!(ok("SKIP", &[json!([1, 2, 3]), json!(1)]), json!([2, 3]));
        assert_eq!(
            ok("CHUNK", &[json!([1, 2, 3, 4, 5]), json!(2)]),
            json!([[1, 2], [3, 4], [5]])
        );
        assert!(call("CHUNK", &[json!([1]), json!(0)]).is_err());
    }

    #[test]
    fn test_sum_avg() {
        assert_eq!(
            call("SUM", &[json!([1, 2, 3, 4])]).unwrap(),
            Some(json!(10.0))
        );
        assert_eq!(call("AVG", &[json!([2, 4, 6])]).unwrap(), Some(json!(4.0)));
    }

    #[test]
    fn test_min_max() {
        assert_eq!(
            call("MIN", &[json!([3, 1, 4, 1, 5])]).unwrap(),
            Some(json!(1))
        );
        assert_eq!(
            call("MAX", &[json!([3, 1, 4, 1, 5])]).unwrap(),
            Some(json!(5))
        );
        assert_eq!(ok("MIN", &[json!([null, 2, 1])]), json!(1));
        assert_eq!(ok("MAX", &[json!(3), json!(7), Value::Null]), json!(7));
    }

    #[test]
    fn count_is_length() {
        assert_eq!(ok("COUNT", &[json!([1, null, 2])]), json!(3));
        assert_eq!(ok("COUNT", &[json!("abc")]), json!(3));
        assert_eq!(ok("COUNT", &[Value::Null]), json!(0));
    }

    #[test]
    fn test_range() {
        assert_eq!(
            call("RANGE", &[json!(1), json!(5)]).unwrap(),
            Some(json!([1, 2, 3, 4, 5]))
        );
        assert_eq!(
            call("RANGE", &[json!(0), json!(10), json!(2)]).unwrap(),
            Some(json!([0, 2, 4, 6, 8, 10]))
        );
        assert_eq!(
            ok("RANGE", &[json!(5), json!(1), json!(-2)]),
            json!([5, 3, 1])
        );
        assert_eq!(ok("RANGE", &[json!(5), json!(1)]), json!([]));
        assert_eq!(
            ok("RANGE", &[json!(1), json!(2), json!(0.5)]),
            json!([1.0, 1.5, 2.0])
        );
    }

    #[test]
    fn range_is_capped_and_overflow_safe() {
        assert!(call("RANGE", &[json!(0), json!(10_000_000)]).is_err());
        assert!(call("RANGE", &[json!(i64::MIN), json!(i64::MAX)]).is_err());
        assert!(call("RANGE", &[json!(0), json!(5), json!(0)]).is_err());
        // Near i64::MAX the step used to overflow `i += step`.
        assert_eq!(
            ok(
                "RANGE",
                &[json!(i64::MAX - 1), json!(i64::MAX), json!(i64::MAX)]
            ),
            json!([i64::MAX - 1])
        );
        assert_eq!(
            ok(
                "RANGE",
                &[json!(i64::MIN + 1), json!(i64::MIN), json!(i64::MIN)]
            ),
            json!([i64::MIN + 1])
        );
        assert!(call("RANGE", &[json!(0), json!(1e300), json!(0.5)]).is_err());
    }

    #[test]
    fn nth_never_panics() {
        assert_eq!(ok("NTH", &[json!([1, 2, 3]), json!(-1)]), json!(3));
        assert_eq!(ok("NTH", &[json!([1, 2, 3]), json!(-4)]), Value::Null);
        assert_eq!(ok("NTH", &[json!([1, 2, 3]), json!(i64::MIN)]), Value::Null);
    }
}
