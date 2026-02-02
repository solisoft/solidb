//! String builtin functions.

use serde_json::Value;

use crate::error::SdbqlResult;

/// Call a string function. Returns None if function not found.
pub fn call(name: &str, args: &[Value]) -> SdbqlResult<Option<Value>> {
    let result = match name {
        "UPPER" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            Some(Value::String(s.to_uppercase()))
        }

        "LOWER" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            Some(Value::String(s.to_lowercase()))
        }

        "TRIM" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            Some(Value::String(s.trim().to_string()))
        }

        "LTRIM" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            Some(Value::String(s.trim_start().to_string()))
        }

        "RTRIM" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            Some(Value::String(s.trim_end().to_string()))
        }

        "LENGTH" => match args.first() {
            Some(Value::String(s)) => {
                Some(Value::Number(serde_json::Number::from(s.chars().count())))
            }
            Some(Value::Array(arr)) => Some(Value::Number(serde_json::Number::from(arr.len()))),
            Some(Value::Object(obj)) => Some(Value::Number(serde_json::Number::from(obj.len()))),
            _ => Some(Value::Number(serde_json::Number::from(0))),
        },

        "CHAR_LENGTH" | "CHAR_COUNT" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            Some(Value::Number(serde_json::Number::from(s.chars().count())))
        }

        "BYTE_LENGTH" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            Some(Value::Number(serde_json::Number::from(s.len())))
        }

        "CONCAT" => {
            let mut result = String::new();
            for arg in args {
                match arg {
                    Value::String(s) => result.push_str(s),
                    Value::Null => {}
                    _ => result.push_str(&arg.to_string()),
                }
            }
            Some(Value::String(result))
        }

        "CONCAT_SEPARATOR" => {
            if args.is_empty() {
                return Ok(Some(Value::String(String::new())));
            }
            let separator = args[0].as_str().unwrap_or("");
            let parts: Vec<String> = args[1..]
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Null => None,
                    _ => Some(v.to_string()),
                })
                .collect();
            Some(Value::String(parts.join(separator)))
        }

        "CONTAINS" => {
            let haystack = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let needle = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            Some(Value::Bool(haystack.contains(needle)))
        }

        "STARTS_WITH" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let prefix = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            Some(Value::Bool(s.starts_with(prefix)))
        }

        "ENDS_WITH" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let suffix = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            Some(Value::Bool(s.ends_with(suffix)))
        }

        "SUBSTRING" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let start = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as usize;
            let len = args.get(2).and_then(|v| v.as_i64());

            let chars: Vec<char> = s.chars().collect();
            if start >= chars.len() {
                return Ok(Some(Value::String(String::new())));
            }

            let result: String = if let Some(len) = len {
                chars[start..].iter().take(len as usize).collect()
            } else {
                chars[start..].iter().collect()
            };
            Some(Value::String(result))
        }

        "LEFT" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let n = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as usize;
            Some(Value::String(s.chars().take(n).collect()))
        }

        "RIGHT" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let n = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as usize;
            let chars: Vec<char> = s.chars().collect();
            if n >= chars.len() {
                return Ok(Some(Value::String(s.to_string())));
            }
            Some(Value::String(chars[chars.len() - n..].iter().collect()))
        }

        "SPLIT" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let separator = args.get(1).and_then(|v| v.as_str()).unwrap_or(",");
            let parts: Vec<Value> = s
                .split(separator)
                .map(|p| Value::String(p.to_string()))
                .collect();
            Some(Value::Array(parts))
        }

        "JOIN" => match args.first() {
            Some(Value::Array(arr)) => {
                let separator = args.get(1).and_then(|v| v.as_str()).unwrap_or(",");
                let parts: Vec<String> = arr
                    .iter()
                    .filter_map(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        Value::Null => None,
                        _ => Some(v.to_string()),
                    })
                    .collect();
                Some(Value::String(parts.join(separator)))
            }
            _ => Some(Value::Null),
        },

        "REVERSE" => match args.first() {
            Some(Value::String(s)) => Some(Value::String(s.chars().rev().collect())),
            Some(Value::Array(arr)) => {
                let mut reversed = arr.clone();
                reversed.reverse();
                Some(Value::Array(reversed))
            }
            _ => Some(Value::Null),
        },

        "REPLACE" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let search = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            let replace = args.get(2).and_then(|v| v.as_str()).unwrap_or("");
            Some(Value::String(s.replace(search, replace)))
        }

        "REGEX_REPLACE" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let pattern = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            let replace = args.get(2).and_then(|v| v.as_str()).unwrap_or("");

            match regex::Regex::new(pattern) {
                Ok(re) => Some(Value::String(re.replace_all(s, replace).into_owned())),
                Err(_) => Some(Value::String(s.to_string())),
            }
        }

        "REGEX_TEST" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let pattern = args.get(1).and_then(|v| v.as_str()).unwrap_or("");

            match regex::Regex::new(pattern) {
                Ok(re) => Some(Value::Bool(re.is_match(s))),
                Err(_) => Some(Value::Bool(false)),
            }
        }

        "REGEX_MATCHES" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let pattern = args.get(1).and_then(|v| v.as_str()).unwrap_or("");

            match regex::Regex::new(pattern) {
                Ok(re) => {
                    let matches: Vec<Value> = re
                        .find_iter(s)
                        .map(|m| Value::String(m.as_str().to_string()))
                        .collect();
                    Some(Value::Array(matches))
                }
                Err(_) => Some(Value::Array(vec![])),
            }
        }

        "TO_STRING" => match args.first() {
            Some(Value::String(s)) => Some(Value::String(s.clone())),
            Some(Value::Null) => Some(Value::String("null".to_string())),
            Some(v) => Some(Value::String(v.to_string())),
            None => Some(Value::Null),
        },

        "REPEAT" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let n = args.get(1).and_then(|v| v.as_i64()).unwrap_or(1) as usize;
            Some(Value::String(s.repeat(n)))
        }

        "PAD_LEFT" | "LPAD" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let len = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as usize;
            let pad = args.get(2).and_then(|v| v.as_str()).unwrap_or(" ");
            let current_len = s.chars().count();
            if current_len >= len {
                return Ok(Some(Value::String(s.to_string())));
            }
            let padding = pad.repeat((len - current_len).div_ceil(pad.len()));
            Some(Value::String(format!(
                "{}{}",
                &padding[..len - current_len],
                s
            )))
        }

        "PAD_RIGHT" | "RPAD" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let len = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as usize;
            let pad = args.get(2).and_then(|v| v.as_str()).unwrap_or(" ");
            let current_len = s.chars().count();
            if current_len >= len {
                return Ok(Some(Value::String(s.to_string())));
            }
            let padding = pad.repeat((len - current_len).div_ceil(pad.len()));
            Some(Value::String(format!(
                "{}{}",
                s,
                &padding[..len - current_len]
            )))
        }

        "FIND_FIRST" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let needle = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            match s.find(needle) {
                Some(pos) => Some(Value::Number(serde_json::Number::from(pos))),
                None => Some(Value::Number(serde_json::Number::from(-1i64))),
            }
        }

        "FIND_LAST" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let needle = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            match s.rfind(needle) {
                Some(pos) => Some(Value::Number(serde_json::Number::from(pos))),
                None => Some(Value::Number(serde_json::Number::from(-1i64))),
            }
        }

        _ => None,
    };

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_upper_lower() {
        assert_eq!(
            call("UPPER", &[json!("hello")]).unwrap(),
            Some(json!("HELLO"))
        );
        assert_eq!(
            call("LOWER", &[json!("HELLO")]).unwrap(),
            Some(json!("hello"))
        );
    }

    #[test]
    fn test_trim() {
        assert_eq!(
            call("TRIM", &[json!("  hello  ")]).unwrap(),
            Some(json!("hello"))
        );
        assert_eq!(
            call("LTRIM", &[json!("  hello")]).unwrap(),
            Some(json!("hello"))
        );
        assert_eq!(
            call("RTRIM", &[json!("hello  ")]).unwrap(),
            Some(json!("hello"))
        );
    }

    #[test]
    fn test_concat() {
        assert_eq!(
            call("CONCAT", &[json!("hello"), json!(" "), json!("world")]).unwrap(),
            Some(json!("hello world"))
        );
    }

    #[test]
    fn test_split_join() {
        assert_eq!(
            call("SPLIT", &[json!("a,b,c"), json!(",")]).unwrap(),
            Some(json!(["a", "b", "c"]))
        );
        assert_eq!(
            call("JOIN", &[json!(["a", "b", "c"]), json!("-")]).unwrap(),
            Some(json!("a-b-c"))
        );
    }

    #[test]
    fn test_substring() {
        assert_eq!(
            call("SUBSTRING", &[json!("hello world"), json!(0), json!(5)]).unwrap(),
            Some(json!("hello"))
        );
        assert_eq!(
            call("SUBSTRING", &[json!("hello"), json!(2)]).unwrap(),
            Some(json!("llo"))
        );
    }

    #[test]
    fn test_contains() {
        assert_eq!(
            call("CONTAINS", &[json!("hello world"), json!("world")]).unwrap(),
            Some(json!(true))
        );
        assert_eq!(
            call("CONTAINS", &[json!("hello"), json!("xyz")]).unwrap(),
            Some(json!(false))
        );
    }

    #[test]
    fn test_replace() {
        assert_eq!(
            call(
                "REPLACE",
                &[json!("hello world"), json!("world"), json!("there")]
            )
            .unwrap(),
            Some(json!("hello there"))
        );
    }
}
