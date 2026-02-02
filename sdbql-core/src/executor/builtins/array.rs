//! Array builtin functions.

use serde_json::Value;
use std::collections::HashSet;

use crate::error::SdbqlResult;
use crate::executor::helpers::compare_values;

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
            let n = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0);
            match arr {
                Some(arr) => {
                    let idx = if n < 0 {
                        (arr.len() as i64 + n) as usize
                    } else {
                        n as usize
                    };
                    Some(arr.get(idx).cloned().unwrap_or(Value::Null))
                }
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

        "UNIQUE" => match args.first() {
            Some(Value::Array(arr)) => {
                let mut seen = HashSet::new();
                let mut unique = Vec::new();
                for item in arr {
                    let key = serde_json::to_string(item).unwrap_or_default();
                    if seen.insert(key) {
                        unique.push(item.clone());
                    }
                }
                Some(Value::Array(unique))
            }
            _ => Some(Value::Null),
        },

        "FLATTEN" => match args.first() {
            Some(Value::Array(arr)) => {
                let depth = args.get(1).and_then(|v| v.as_i64()).unwrap_or(1);
                let result = flatten_array(arr, depth as usize);
                Some(Value::Array(result))
            }
            _ => Some(Value::Null),
        },

        "PUSH" => match args.first() {
            Some(Value::Array(arr)) => {
                let mut result = arr.clone();
                for item in args.iter().skip(1) {
                    result.push(item.clone());
                }
                Some(Value::Array(result))
            }
            _ => Some(Value::Null),
        },

        "UNSHIFT" => match args.first() {
            Some(Value::Array(arr)) => {
                let mut result: Vec<Value> = args.iter().skip(1).cloned().collect();
                result.extend(arr.clone());
                Some(Value::Array(result))
            }
            _ => Some(Value::Null),
        },

        "POP" => match args.first() {
            Some(Value::Array(arr)) => {
                if arr.is_empty() {
                    Some(Value::Array(vec![]))
                } else {
                    Some(Value::Array(arr[..arr.len() - 1].to_vec()))
                }
            }
            _ => Some(Value::Null),
        },

        "SHIFT" => match args.first() {
            Some(Value::Array(arr)) => {
                if arr.is_empty() {
                    Some(Value::Array(vec![]))
                } else {
                    Some(Value::Array(arr[1..].to_vec()))
                }
            }
            _ => Some(Value::Null),
        },

        "SLICE" => match args.first() {
            Some(Value::Array(arr)) => {
                let start = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0);
                let end = args.get(2).and_then(|v| v.as_i64());

                let len = arr.len() as i64;
                let start = if start < 0 { len + start } else { start } as usize;
                let end = match end {
                    Some(e) if e < 0 => (len + e) as usize,
                    Some(e) => e as usize,
                    None => arr.len(),
                };

                if start >= arr.len() {
                    return Ok(Some(Value::Array(vec![])));
                }
                let end = end.min(arr.len());
                Some(Value::Array(arr[start..end].to_vec()))
            }
            _ => Some(Value::Null),
        },

        "APPEND" | "UNION" => {
            let mut result = Vec::new();
            for arg in args {
                match arg {
                    Value::Array(arr) => result.extend(arr.clone()),
                    _ => result.push(arg.clone()),
                }
            }
            Some(Value::Array(result))
        }

        "INTERSECTION" => {
            if args.is_empty() {
                return Ok(Some(Value::Array(vec![])));
            }

            let first = match args.first() {
                Some(Value::Array(arr)) => arr,
                _ => return Ok(Some(Value::Null)),
            };

            let mut result: Vec<Value> = first.clone();

            for arg in args.iter().skip(1) {
                if let Value::Array(arr) = arg {
                    let set: HashSet<String> = arr
                        .iter()
                        .map(|v| serde_json::to_string(v).unwrap_or_default())
                        .collect();
                    result.retain(|v| set.contains(&serde_json::to_string(v).unwrap_or_default()));
                }
            }

            Some(Value::Array(result))
        }

        "MINUS" | "DIFFERENCE" => {
            let first = match args.first() {
                Some(Value::Array(arr)) => arr.clone(),
                _ => return Ok(Some(Value::Null)),
            };

            let mut result = first;

            for arg in args.iter().skip(1) {
                if let Value::Array(arr) = arg {
                    let set: HashSet<String> = arr
                        .iter()
                        .map(|v| serde_json::to_string(v).unwrap_or_default())
                        .collect();
                    result.retain(|v| !set.contains(&serde_json::to_string(v).unwrap_or_default()));
                }
            }

            Some(Value::Array(result))
        }

        "POSITION" | "INDEX_OF" => {
            let arr = args.first().and_then(|v| v.as_array());
            let needle = args.get(1);

            match (arr, needle) {
                (Some(arr), Some(needle)) => {
                    let needle_str = serde_json::to_string(needle).unwrap_or_default();
                    for (i, item) in arr.iter().enumerate() {
                        if serde_json::to_string(item).unwrap_or_default() == needle_str {
                            return Ok(Some(Value::Number(serde_json::Number::from(i))));
                        }
                    }
                    Some(Value::Number(serde_json::Number::from(-1i64)))
                }
                _ => Some(Value::Null),
            }
        }

        "CONTAINS_ARRAY" => {
            let arr = args.first().and_then(|v| v.as_array());
            let needle = args.get(1);

            match (arr, needle) {
                (Some(arr), Some(needle)) => {
                    let needle_str = serde_json::to_string(needle).unwrap_or_default();
                    for item in arr {
                        if serde_json::to_string(item).unwrap_or_default() == needle_str {
                            return Ok(Some(Value::Bool(true)));
                        }
                    }
                    Some(Value::Bool(false))
                }
                _ => Some(Value::Bool(false)),
            }
        }

        "COUNT" => match args.first() {
            Some(Value::Array(arr)) => Some(Value::Number(serde_json::Number::from(arr.len()))),
            _ => Some(Value::Number(serde_json::Number::from(0))),
        },

        "SUM" => match args.first() {
            Some(Value::Array(arr)) => {
                let sum: f64 = arr.iter().filter_map(|v| v.as_f64()).sum();
                Some(Value::Number(
                    serde_json::Number::from_f64(sum)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ))
            }
            _ => Some(Value::Number(serde_json::Number::from(0))),
        },

        "AVG" | "AVERAGE" => match args.first() {
            Some(Value::Array(arr)) => {
                let values: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if values.is_empty() {
                    return Ok(Some(Value::Null));
                }
                let avg = values.iter().sum::<f64>() / values.len() as f64;
                Some(Value::Number(
                    serde_json::Number::from_f64(avg)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ))
            }
            _ => Some(Value::Null),
        },

        "MIN" => match args.first() {
            Some(Value::Array(arr)) => {
                let min = arr
                    .iter()
                    .filter(|v| !v.is_null())
                    .min_by(|a, b| compare_values(a, b));
                Some(min.cloned().unwrap_or(Value::Null))
            }
            _ => Some(Value::Null),
        },

        "MAX" => match args.first() {
            Some(Value::Array(arr)) => {
                let max = arr
                    .iter()
                    .filter(|v| !v.is_null())
                    .max_by(|a, b| compare_values(a, b));
                Some(max.cloned().unwrap_or(Value::Null))
            }
            _ => Some(Value::Null),
        },

        "RANGE" => {
            let start = args.first().and_then(|v| v.as_i64()).unwrap_or(0);
            let end = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0);
            let step = args.get(2).and_then(|v| v.as_i64()).unwrap_or(1);

            if step == 0 {
                return Ok(Some(Value::Array(vec![])));
            }

            let mut result = Vec::new();
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
            Some(Value::Array(result))
        }

        "ZIP" => {
            let arrays: Vec<&Vec<Value>> = args.iter().filter_map(|v| v.as_array()).collect();
            if arrays.is_empty() {
                return Ok(Some(Value::Array(vec![])));
            }

            let min_len = arrays.iter().map(|a| a.len()).min().unwrap_or(0);
            let mut result = Vec::new();

            for i in 0..min_len {
                let tuple: Vec<Value> = arrays.iter().map(|a| a[i].clone()).collect();
                result.push(Value::Array(tuple));
            }

            Some(Value::Array(result))
        }

        _ => None,
    };

    Ok(result)
}

fn flatten_array(arr: &[Value], depth: usize) -> Vec<Value> {
    if depth == 0 {
        return arr.to_vec();
    }

    let mut result = Vec::new();
    for item in arr {
        match item {
            Value::Array(inner) => {
                result.extend(flatten_array(inner, depth - 1));
            }
            _ => result.push(item.clone()),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    }

    #[test]
    fn test_unique() {
        assert_eq!(
            call("UNIQUE", &[json!([1, 2, 2, 3, 1])]).unwrap(),
            Some(json!([1, 2, 3]))
        );
    }

    #[test]
    fn test_flatten() {
        assert_eq!(
            call("FLATTEN", &[json!([[1, 2], [3, 4]])]).unwrap(),
            Some(json!([1, 2, 3, 4]))
        );
    }

    #[test]
    fn test_slice() {
        assert_eq!(
            call("SLICE", &[json!([1, 2, 3, 4, 5]), json!(1), json!(4)]).unwrap(),
            Some(json!([2, 3, 4]))
        );
        assert_eq!(
            call("SLICE", &[json!([1, 2, 3, 4, 5]), json!(-2)]).unwrap(),
            Some(json!([4, 5]))
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
    }
}
