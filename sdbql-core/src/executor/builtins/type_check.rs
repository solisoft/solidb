//! Type checking builtin functions.

use serde_json::Value;

use crate::error::{SdbqlError, SdbqlResult};

/// Call a type checking function. Returns None if function not found.
pub fn call(name: &str, args: &[Value]) -> SdbqlResult<Option<Value>> {
    let result = match name {
        "IS_ARRAY" | "IS_LIST" => {
            check_args(name, args, 1)?;
            Some(Value::Bool(matches!(args[0], Value::Array(_))))
        }

        "IS_BOOL" | "IS_BOOLEAN" => {
            check_args(name, args, 1)?;
            Some(Value::Bool(matches!(args[0], Value::Bool(_))))
        }

        "IS_NUMBER" | "IS_NUMERIC" => {
            check_args(name, args, 1)?;
            Some(Value::Bool(matches!(args[0], Value::Number(_))))
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
            Some(Value::Bool(is_int))
        }

        "IS_STRING" => {
            check_args(name, args, 1)?;
            Some(Value::Bool(matches!(args[0], Value::String(_))))
        }

        "IS_NULL" => {
            check_args(name, args, 1)?;
            Some(Value::Bool(matches!(args[0], Value::Null)))
        }

        "IS_OBJECT" | "IS_DOCUMENT" => {
            check_args(name, args, 1)?;
            Some(Value::Bool(matches!(args[0], Value::Object(_))))
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
            Some(Value::Bool(is_empty))
        }

        "TYPENAME" | "TYPE_OF" => {
            check_args(name, args, 1)?;
            let type_name = match &args[0] {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            Some(Value::String(type_name.to_string()))
        }

        "TO_BOOL" | "TO_BOOLEAN" => {
            check_args(name, args, 1)?;
            let b = match &args[0] {
                Value::Bool(b) => *b,
                Value::Null => false,
                Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
                Value::String(s) => !s.is_empty(),
                Value::Array(arr) => !arr.is_empty(),
                Value::Object(obj) => !obj.is_empty(),
            };
            Some(Value::Bool(b))
        }

        "TO_NUMBER" => {
            check_args(name, args, 1)?;
            let n = match &args[0] {
                Value::Number(n) => Some(n.clone()),
                Value::Bool(b) => Some(serde_json::Number::from(if *b { 1 } else { 0 })),
                Value::String(s) => s.parse::<f64>().ok().and_then(serde_json::Number::from_f64),
                Value::Null => Some(serde_json::Number::from(0)),
                _ => None,
            };
            match n {
                Some(num) => Some(Value::Number(num)),
                None => Some(Value::Null),
            }
        }

        "TO_ARRAY" => {
            check_args(name, args, 1)?;
            match &args[0] {
                Value::Array(arr) => Some(Value::Array(arr.clone())),
                Value::Null => Some(Value::Array(vec![])),
                v => Some(Value::Array(vec![v.clone()])),
            }
        }

        "NOT_NULL" | "FIRST_NOT_NULL" => {
            let mut found = Value::Null;
            for arg in args {
                if !arg.is_null() {
                    found = arg.clone();
                    break;
                }
            }
            Some(found)
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
    fn test_type_checks() {
        assert_eq!(
            call("IS_ARRAY", &[json!([1, 2, 3])]).unwrap(),
            Some(json!(true))
        );
        assert_eq!(
            call("IS_ARRAY", &[json!("hello")]).unwrap(),
            Some(json!(false))
        );

        assert_eq!(
            call("IS_STRING", &[json!("hi")]).unwrap(),
            Some(json!(true))
        );
        assert_eq!(call("IS_NUMBER", &[json!(42)]).unwrap(), Some(json!(true)));
        assert_eq!(call("IS_BOOL", &[json!(true)]).unwrap(), Some(json!(true)));
        assert_eq!(call("IS_NULL", &[Value::Null]).unwrap(), Some(json!(true)));
        assert_eq!(
            call("IS_OBJECT", &[json!({"a": 1})]).unwrap(),
            Some(json!(true))
        );
    }

    #[test]
    fn test_is_integer() {
        assert_eq!(call("IS_INT", &[json!(42)]).unwrap(), Some(json!(true)));
        assert_eq!(call("IS_INT", &[json!(42.0)]).unwrap(), Some(json!(true)));
        assert_eq!(call("IS_INT", &[json!(42.5)]).unwrap(), Some(json!(false)));
    }

    #[test]
    fn test_is_empty() {
        assert_eq!(call("IS_EMPTY", &[json!("")]).unwrap(), Some(json!(true)));
        assert_eq!(call("IS_EMPTY", &[json!([])]).unwrap(), Some(json!(true)));
        assert_eq!(call("IS_EMPTY", &[json!({})]).unwrap(), Some(json!(true)));
        assert_eq!(call("IS_EMPTY", &[Value::Null]).unwrap(), Some(json!(true)));
        assert_eq!(
            call("IS_EMPTY", &[json!("hello")]).unwrap(),
            Some(json!(false))
        );
    }

    #[test]
    fn test_typename() {
        assert_eq!(
            call("TYPENAME", &[json!("hello")]).unwrap(),
            Some(json!("string"))
        );
        assert_eq!(
            call("TYPENAME", &[json!(42)]).unwrap(),
            Some(json!("number"))
        );
        assert_eq!(
            call("TYPENAME", &[json!([1, 2])]).unwrap(),
            Some(json!("array"))
        );
    }

    #[test]
    fn test_conversions() {
        assert_eq!(call("TO_BOOL", &[json!(1)]).unwrap(), Some(json!(true)));
        assert_eq!(call("TO_BOOL", &[json!(0)]).unwrap(), Some(json!(false)));
        assert_eq!(
            call("TO_NUMBER", &[json!("42")]).unwrap(),
            Some(json!(42.0))
        );
        assert_eq!(call("TO_ARRAY", &[json!(5)]).unwrap(), Some(json!([5])));
    }

    #[test]
    fn test_not_null() {
        assert_eq!(
            call("NOT_NULL", &[Value::Null, json!(1), json!(2)]).unwrap(),
            Some(json!(1))
        );
        assert_eq!(
            call("NOT_NULL", &[Value::Null, Value::Null]).unwrap(),
            Some(Value::Null)
        );
    }
}
