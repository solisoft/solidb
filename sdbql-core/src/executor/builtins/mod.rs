//! Builtin functions for SDBQL queries.
//!
//! Storage-independent functions that can be used in local queries.

mod array;
mod common;
mod datetime;
mod geo;
mod json_funcs;
mod math;
mod string;
mod timeseries;
mod type_check;

pub(crate) use common::MAX_RANGE;

use serde_json::Value;

use crate::error::{SdbqlError, SdbqlResult};

/// Container for builtin function implementations.
pub struct BuiltinFunctions;

impl BuiltinFunctions {
    /// Call a builtin function by name.
    pub fn call(name: &str, args: &[Value]) -> SdbqlResult<Value> {
        let upper_name = name.to_uppercase();

        // String functions
        if let Some(result) = string::call(&upper_name, args)? {
            return Ok(result);
        }

        // Array functions
        if let Some(result) = array::call(&upper_name, args)? {
            return Ok(result);
        }

        // Math functions
        if let Some(result) = math::call(&upper_name, args)? {
            return Ok(result);
        }

        // DateTime functions
        if let Some(result) = datetime::call(&upper_name, args)? {
            return Ok(result);
        }

        // Type check functions
        if let Some(result) = type_check::call(&upper_name, args)? {
            return Ok(result);
        }

        // JSON functions
        if let Some(result) = json_funcs::call(&upper_name, args)? {
            return Ok(result);
        }

        // Geo functions
        if let Some(result) = geo::call(&upper_name, args)? {
            return Ok(result);
        }

        // Time-series functions
        if let Some(result) = timeseries::call(&upper_name, args)? {
            return Ok(result);
        }

        Err(SdbqlError::ExecutionError(format!(
            "Unknown function: {}",
            name
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_string_functions() {
        assert_eq!(
            BuiltinFunctions::call("UPPER", &[json!("hello")]).unwrap(),
            json!("HELLO")
        );
        assert_eq!(
            BuiltinFunctions::call("LOWER", &[json!("HELLO")]).unwrap(),
            json!("hello")
        );
        assert_eq!(
            BuiltinFunctions::call("LENGTH", &[json!("hello")]).unwrap(),
            json!(5)
        );
    }

    #[test]
    fn test_array_functions() {
        assert_eq!(
            BuiltinFunctions::call("LENGTH", &[json!([1, 2, 3])]).unwrap(),
            json!(3)
        );
        assert_eq!(
            BuiltinFunctions::call("FIRST", &[json!([1, 2, 3])]).unwrap(),
            json!(1)
        );
        assert_eq!(
            BuiltinFunctions::call("LAST", &[json!([1, 2, 3])]).unwrap(),
            json!(3)
        );
    }

    #[test]
    fn test_math_functions() {
        assert_eq!(
            BuiltinFunctions::call("ABS", &[json!(-5)]).unwrap(),
            json!(5.0)
        );
        assert_eq!(
            BuiltinFunctions::call("FLOOR", &[json!(3.7)]).unwrap(),
            json!(3.0)
        );
        assert_eq!(
            BuiltinFunctions::call("CEIL", &[json!(3.2)]).unwrap(),
            json!(4.0)
        );
    }

    #[test]
    fn test_type_functions() {
        assert_eq!(
            BuiltinFunctions::call("IS_STRING", &[json!("hello")]).unwrap(),
            json!(true)
        );
        assert_eq!(
            BuiltinFunctions::call("IS_NUMBER", &[json!(42)]).unwrap(),
            json!(true)
        );
        assert_eq!(
            BuiltinFunctions::call("IS_ARRAY", &[json!([1, 2])]).unwrap(),
            json!(true)
        );
    }

    #[test]
    fn every_new_function_is_dispatched() {
        for (name, args) in [
            (
                "DATE_TRUNC",
                vec![json!("2024-05-17T13:45:12Z"), json!("day")],
            ),
            ("DATE_QUARTER", vec![json!("2024-05-17")]),
            ("TIME_BUCKET", vec![json!(1000), json!("1s")]),
            ("GEO_DISTANCE", vec![json!([0, 0]), json!([1, 1])]),
            ("DELTA", vec![json!([1, 2])]),
            ("MATCH_SEQ", vec![json!([]), json!("k"), json!([])]),
            ("MEDIAN", vec![json!([1, 2, 3])]),
            ("SORTED_UNIQUE", vec![json!([2, 1, 2])]),
            ("REMOVE_VALUE", vec![json!([1, 2]), json!(1)]),
            ("GET", vec![json!({"a": 1}), json!("a")]),
            ("COALESCE", vec![Value::Null, json!(1)]),
            ("MERGE_RECURSIVE", vec![json!({"a": 1})]),
        ] {
            assert!(BuiltinFunctions::call(name, &args).is_ok(), "{name}");
        }
        assert!(BuiltinFunctions::call("NO_SUCH_FN", &[]).is_err());
    }

    #[test]
    fn contains_dispatches_on_the_first_argument() {
        assert_eq!(
            BuiltinFunctions::call("CONTAINS", &[json!([1, 2]), json!(2.0)]).unwrap(),
            json!(true)
        );
        assert_eq!(
            BuiltinFunctions::call("CONTAINS", &[json!("abc"), json!("b")]).unwrap(),
            json!(true)
        );
    }

    #[test]
    fn test_new_helpers() {
        let toks =
            BuiltinFunctions::call("TOKENS", &[json!("The Quick Fox"), json!("text_en")]).unwrap();
        assert!(toks.as_array().unwrap().iter().any(|t| t == "quick"));
        assert_eq!(
            BuiltinFunctions::call(
                "PHRASE",
                &[json!("the quick brown"), json!("quick"), json!("brown")]
            )
            .unwrap(),
            json!(true)
        );
        assert_eq!(
            BuiltinFunctions::call("ZIP_OBJECT", &[json!(["a", "b"]), json!([1, 2])]).unwrap(),
            json!({"a": 1, "b": 2})
        );
        assert_eq!(
            BuiltinFunctions::call("PARSE_IDENTIFIER", &[json!("users/ada")]).unwrap(),
            json!({"collection": "users", "key": "ada"})
        );
        assert_eq!(
            BuiltinFunctions::call("BOOST", &[json!(true), json!(3)]).unwrap(),
            json!(3.0)
        );
    }
}
