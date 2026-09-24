//! Type checking functions for SDBQL.
//!
//! IS_ARRAY, IS_BOOL, IS_NUMBER, IS_STRING, IS_NULL, IS_OBJECT, etc.

use crate::error::{DbError, DbResult};
use serde_json::Value;

/// Evaluate type checking functions
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "IS_ARRAY" | "IS_LIST" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Array(_)))))
        }
        "IS_BOOL" | "IS_BOOLEAN" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Bool(_)))))
        }
        "IS_NUMBER" | "IS_NUMERIC" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Number(_)))))
        }
        "IS_INTEGER" | "IS_INT" => {
            check_args(name, args, 1)?;
            let is_int = match &args[0] {
                Value::Number(n) => {
                    if n.as_i64().is_some() {
                        true
                    } else if let Some(f) = n.as_f64() {
                        f.fract() == 0.0 && f.is_finite()
                    } else {
                        false
                    }
                }
                _ => false,
            };
            Ok(Some(Value::Bool(is_int)))
        }
        "IS_STRING" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::String(_)))))
        }
        "IS_NULL" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Null))))
        }
        "IS_OBJECT" | "IS_DOCUMENT" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Object(_)))))
        }
        "IS_EMPTY" => {
            check_args(name, args, 1)?;
            let is_empty = match &args[0] {
                Value::Null => true,
                Value::String(s) => s.is_empty(),
                Value::Array(arr) => arr.is_empty(),
                Value::Object(obj) => obj.is_empty(),
                _ => false,
            };
            Ok(Some(Value::Bool(is_empty)))
        }
        // Strings the date functions can parse (RFC 3339, `YYYY-MM-DD`,
        // `YYYY-MM-DD HH:MM:SS`). Numbers are not dates here even though the
        // date functions accept epoch timestamps: every number would pass.
        "IS_DATE" | "IS_DATETIME" | "IS_DATESTRING" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(
                args[0].is_string()
                    && crate::sdbql::executor::utils::parse_datetime(&args[0]).is_ok(),
            )))
        }
        "IS_IPV4" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(
                args[0]
                    .as_str()
                    .and_then(super::string::parse_ipv4)
                    .is_some(),
            )))
        }
        "IS_KEY" => {
            check_args(name, args, 1)?;
            let ok = args[0].as_str().is_some_and(|s| {
                !s.is_empty()
                    && s.len() <= 254
                    && !s.contains('/')
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
            });
            Ok(Some(Value::Bool(ok)))
        }
        "IS_SAME_COLLECTION" => {
            check_args(name, args, 2)?;
            let extract_collection = |val: &Value| -> Option<String> {
                match val {
                    Value::String(s) => s.split('/').next().map(|c| c.to_string()),
                    Value::Object(obj) => obj
                        .get("_id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.split('/').next().map(|c| c.to_string())),
                    _ => None,
                }
            };
            let col1 = extract_collection(&args[0]);
            let col2 = extract_collection(&args[1]);
            match (col1, col2) {
                (Some(c1), Some(c2)) => Ok(Some(Value::Bool(c1 == c2))),
                _ => Ok(Some(Value::Bool(false))),
            }
        }
        _ => Ok(None),
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

    fn call(name: &str, v: Value) -> Value {
        evaluate(name, &[v]).unwrap().unwrap()
    }

    #[test]
    fn date_predicates_accept_only_date_strings() {
        for f in ["IS_DATE", "IS_DATETIME", "IS_DATESTRING"] {
            assert_eq!(call(f, json!("2024-01-15T10:30:00Z")), json!(true), "{}", f);
            assert_eq!(call(f, json!("2024-01-15")), json!(true), "{}", f);
            assert_eq!(call(f, json!("nope")), json!(false), "{}", f);
            assert_eq!(call(f, json!(1_700_000_000)), json!(false), "{}", f);
            assert_eq!(call(f, Value::Null), json!(false), "{}", f);
        }
    }

    #[test]
    fn is_ipv4() {
        assert_eq!(call("IS_IPV4", json!("127.0.0.1")), json!(true));
        assert_eq!(call("IS_IPV4", json!("255.255.255.255")), json!(true));
        assert_eq!(call("IS_IPV4", json!("1.2.3.04")), json!(false));
        assert_eq!(call("IS_IPV4", json!("1.2.3")), json!(false));
        assert_eq!(call("IS_IPV4", json!("1.2.3.256")), json!(false));
        assert_eq!(call("IS_IPV4", json!(" 1.2.3.4")), json!(false));
        assert_eq!(call("IS_IPV4", json!(16909060)), json!(false));
    }
}
