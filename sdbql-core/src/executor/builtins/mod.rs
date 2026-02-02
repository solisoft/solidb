//! Builtin functions for SDBQL queries.
//!
//! Storage-independent functions that can be used in local queries.

mod array;
mod datetime;
mod json_funcs;
mod math;
mod string;
mod type_check;

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
}
