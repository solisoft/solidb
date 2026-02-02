//! Core evaluation helper functions for SDBQL executor.
//!
//! This module contains helper functions for expression evaluation:
//! - get_field_value: Extract nested field values from JSON
//! - values_equal: Compare two JSON values for equality
//! - evaluate_binary_op: Evaluate binary operators
//! - evaluate_unary_op: Evaluate unary operators
//! - to_bool: Convert JSON value to boolean
//! - compare_values: Compare two JSON values for ordering

use std::cmp::Ordering;

use regex::Regex;
use serde_json::Value;

use crate::ast::{BinaryOperator, UnaryOperator};
use crate::error::{SdbqlError, SdbqlResult};

/// Extract a nested field value from a JSON document.
///
/// # Arguments
/// * `value` - The JSON value to extract from
/// * `field_path` - Dot-separated field path (e.g., "address.city")
///
/// # Returns
/// The field value, or Value::Null if not found
#[inline]
pub fn get_field_value(value: &Value, field_path: &str) -> Value {
    let mut current = value;

    for part in field_path.split('.') {
        match current.get(part) {
            Some(val) => current = val,
            None => return Value::Null,
        }
    }

    current.clone()
}

/// Compare two JSON values for equality.
///
/// Numbers are compared by their f64 representation for proper numeric comparison.
#[inline]
pub fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        _ => left == right,
    }
}

/// Create a serde_json::Number from an f64 value.
#[inline]
pub fn number_from_f64(n: f64) -> serde_json::Number {
    serde_json::Number::from_f64(n).unwrap_or_else(|| serde_json::Number::from(0))
}

/// Safely compile a regex with size limits to prevent ReDoS attacks.
///
/// # Arguments
/// * `pattern` - The regex pattern to compile
///
/// # Returns
/// Compiled regex or error
pub fn safe_regex(pattern: &str) -> Result<Regex, regex::Error> {
    // Limit pattern length to prevent very complex patterns
    if pattern.len() > 1000 {
        return Err(regex::Error::Syntax(
            "Pattern too long (max 1000 chars)".to_string(),
        ));
    }
    Regex::new(pattern)
}

/// Evaluate a binary operation on two values.
#[inline]
pub fn evaluate_binary_op(left: &Value, op: &BinaryOperator, right: &Value) -> SdbqlResult<Value> {
    match op {
        BinaryOperator::Equal => Ok(Value::Bool(values_equal(left, right))),
        BinaryOperator::NotEqual => Ok(Value::Bool(!values_equal(left, right))),

        BinaryOperator::LessThan => Ok(Value::Bool(compare_values(left, right) == Ordering::Less)),
        BinaryOperator::LessThanOrEqual => Ok(Value::Bool(
            compare_values(left, right) != Ordering::Greater,
        )),
        BinaryOperator::GreaterThan => Ok(Value::Bool(
            compare_values(left, right) == Ordering::Greater,
        )),
        BinaryOperator::GreaterThanOrEqual => {
            Ok(Value::Bool(compare_values(left, right) != Ordering::Less))
        }
        BinaryOperator::In => match right {
            Value::Array(arr) => {
                let mut found = false;
                for val in arr {
                    if values_equal(left, val) {
                        found = true;
                        break;
                    }
                }
                Ok(Value::Bool(found))
            }
            Value::Object(obj) => {
                if let Some(s) = left.as_str() {
                    Ok(Value::Bool(obj.contains_key(s)))
                } else {
                    Ok(Value::Bool(false))
                }
            }
            _ => Ok(Value::Bool(false)),
        },

        BinaryOperator::NotIn => match right {
            Value::Array(arr) => {
                let mut found = false;
                for val in arr {
                    if values_equal(left, val) {
                        found = true;
                        break;
                    }
                }
                Ok(Value::Bool(!found))
            }
            Value::Object(obj) => {
                if let Some(s) = left.as_str() {
                    Ok(Value::Bool(!obj.contains_key(s)))
                } else {
                    Ok(Value::Bool(true))
                }
            }
            _ => Ok(Value::Bool(true)),
        },

        BinaryOperator::Like | BinaryOperator::NotLike => {
            let s = left.as_str().unwrap_or("");
            let pattern = right.as_str().unwrap_or("");

            // Convert SQL LIKE pattern to Regex
            let mut regex_pattern = String::new();
            regex_pattern.push('^');
            for c in pattern.chars() {
                match c {
                    '%' => regex_pattern.push_str(".*"),
                    '_' => regex_pattern.push('.'),
                    '^' | '$' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
                    | '\\' => {
                        regex_pattern.push('\\');
                        regex_pattern.push(c);
                    }
                    _ => regex_pattern.push(c),
                }
            }
            regex_pattern.push('$');

            match safe_regex(&regex_pattern) {
                Ok(re) => {
                    let is_match = re.is_match(s);
                    if matches!(op, BinaryOperator::NotLike) {
                        Ok(Value::Bool(!is_match))
                    } else {
                        Ok(Value::Bool(is_match))
                    }
                }
                Err(_) => Ok(Value::Bool(false)),
            }
        }

        BinaryOperator::RegEx | BinaryOperator::NotRegEx => {
            let s = left.as_str().unwrap_or("");
            let pattern = right.as_str().unwrap_or("");

            match safe_regex(pattern) {
                Ok(re) => {
                    let is_match = re.is_match(s);
                    if matches!(op, BinaryOperator::NotRegEx) {
                        Ok(Value::Bool(!is_match))
                    } else {
                        Ok(Value::Bool(is_match))
                    }
                }
                Err(_) => Ok(Value::Bool(false)),
            }
        }

        BinaryOperator::FuzzyEqual => {
            let left_str = left.as_str().unwrap_or("");
            let right_str = right.as_str().unwrap_or("");
            let distance = levenshtein_distance(left_str, right_str);
            Ok(Value::Bool(distance <= 2)) // Default max distance of 2
        }

        BinaryOperator::And => Ok(Value::Bool(to_bool(left) && to_bool(right))),
        BinaryOperator::Or => Ok(Value::Bool(to_bool(left) || to_bool(right))),

        BinaryOperator::Add => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(number_from_f64(a + b)))
            } else if let (Some(a), Some(b)) = (left.as_str(), right.as_str()) {
                Ok(Value::String(format!("{}{}", a, b)))
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot add these types".to_string(),
                ))
            }
        }

        BinaryOperator::Subtract => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(number_from_f64(a - b)))
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot subtract non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::Multiply => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(number_from_f64(a * b)))
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot multiply non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::Divide => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                if b == 0.0 {
                    Err(SdbqlError::ExecutionError("Division by zero".to_string()))
                } else {
                    Ok(Value::Number(number_from_f64(a / b)))
                }
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot divide non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::Modulus => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                if b == 0.0 {
                    Err(SdbqlError::ExecutionError("Division by zero".to_string()))
                } else {
                    Ok(Value::Number(number_from_f64(a % b)))
                }
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot modulus non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::BitwiseAnd => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from(
                    (a as i64) & (b as i64),
                )))
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot bitwise AND non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::BitwiseOr => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from(
                    (a as i64) | (b as i64),
                )))
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot bitwise OR non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::BitwiseXor => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from(
                    (a as i64) ^ (b as i64),
                )))
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot bitwise XOR non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::LeftShift => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from(
                    (a as i64) << (b as i64),
                )))
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot left shift non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::RightShift => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from(
                    (a as i64) >> (b as i64),
                )))
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot right shift non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::Exponent => {
            if let (Some(base), Some(exp)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(number_from_f64(base.powf(exp))))
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot exponentiate non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::NullCoalesce => {
            if left.is_null() {
                Ok(right.clone())
            } else {
                Ok(left.clone())
            }
        }

        BinaryOperator::LogicalOr => {
            if to_bool(left) {
                Ok(left.clone())
            } else {
                Ok(right.clone())
            }
        }
    }
}

/// Evaluate a unary operation on a value.
#[inline]
pub fn evaluate_unary_op(op: &UnaryOperator, operand: &Value) -> SdbqlResult<Value> {
    match op {
        UnaryOperator::Not => Ok(Value::Bool(!to_bool(operand))),
        UnaryOperator::Negate => {
            if let Some(n) = operand.as_f64() {
                Ok(Value::Number(number_from_f64(-n)))
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot negate non-number".to_string(),
                ))
            }
        }
        UnaryOperator::BitwiseNot => {
            if let Some(n) = operand.as_f64() {
                Ok(Value::Number(serde_json::Number::from(!(n as i64))))
            } else {
                Err(SdbqlError::ExecutionError(
                    "Cannot bitwise NOT non-number".to_string(),
                ))
            }
        }
    }
}

/// Convert a JSON value to boolean.
///
/// - Bool: returns the value
/// - Null: returns false
/// - Number: returns false if 0, true otherwise
/// - String: returns false if empty, true otherwise
/// - Array: returns false if empty, true otherwise
/// - Object: returns false if empty, true otherwise
#[inline]
pub fn to_bool(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Compare two JSON values for ordering.
///
/// Null < Bool < Number < String < Array < Object
#[inline]
pub fn compare_values(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Number(a), Value::Number(b)) => {
            let a_f64 = a.as_f64().unwrap_or(0.0);
            let b_f64 = b.as_f64().unwrap_or(0.0);
            a_f64.partial_cmp(&b_f64).unwrap_or(Ordering::Equal)
        }
        (Value::String(a), Value::String(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}

/// Calculate Levenshtein distance between two strings.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_len = a.chars().count();
    let b_len = b.chars().count();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row: Vec<usize> = vec![0; b_len + 1];

    for (i, a_char) in a.chars().enumerate() {
        curr_row[0] = i + 1;
        for (j, b_char) in b.chars().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };
            curr_row[j + 1] = (prev_row[j + 1] + 1)
                .min(curr_row[j] + 1)
                .min(prev_row[j] + cost);
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_field_value() {
        let doc = json!({"name": "Alice", "address": {"city": "NYC"}});
        assert_eq!(get_field_value(&doc, "name"), json!("Alice"));
        assert_eq!(get_field_value(&doc, "address.city"), json!("NYC"));
        assert_eq!(get_field_value(&doc, "missing"), Value::Null);
    }

    #[test]
    fn test_values_equal() {
        assert!(values_equal(&json!(1), &json!(1)));
        assert!(values_equal(&json!(1.0), &json!(1)));
        assert!(values_equal(&json!("hello"), &json!("hello")));
        assert!(!values_equal(&json!(1), &json!(2)));
    }

    #[test]
    fn test_to_bool() {
        assert!(to_bool(&json!(true)));
        assert!(!to_bool(&json!(false)));
        assert!(!to_bool(&Value::Null));
        assert!(to_bool(&json!(1)));
        assert!(!to_bool(&json!(0)));
        assert!(to_bool(&json!("hello")));
        assert!(!to_bool(&json!("")));
        assert!(to_bool(&json!([1, 2])));
        assert!(!to_bool(&json!([])));
    }

    #[test]
    fn test_compare_values() {
        assert_eq!(compare_values(&json!(1), &json!(2)), Ordering::Less);
        assert_eq!(compare_values(&json!(2), &json!(1)), Ordering::Greater);
        assert_eq!(compare_values(&json!(1), &json!(1)), Ordering::Equal);
        assert_eq!(compare_values(&json!("a"), &json!("b")), Ordering::Less);
        assert_eq!(compare_values(&Value::Null, &json!(1)), Ordering::Less);
    }

    #[test]
    fn test_binary_ops() {
        // Arithmetic
        assert_eq!(
            evaluate_binary_op(&json!(2), &BinaryOperator::Add, &json!(3)).unwrap(),
            json!(5.0)
        );
        assert_eq!(
            evaluate_binary_op(&json!(5), &BinaryOperator::Subtract, &json!(3)).unwrap(),
            json!(2.0)
        );
        assert_eq!(
            evaluate_binary_op(&json!(4), &BinaryOperator::Multiply, &json!(3)).unwrap(),
            json!(12.0)
        );
        assert_eq!(
            evaluate_binary_op(&json!(6), &BinaryOperator::Divide, &json!(2)).unwrap(),
            json!(3.0)
        );

        // Comparison
        assert_eq!(
            evaluate_binary_op(&json!(1), &BinaryOperator::Equal, &json!(1)).unwrap(),
            json!(true)
        );
        assert_eq!(
            evaluate_binary_op(&json!(1), &BinaryOperator::LessThan, &json!(2)).unwrap(),
            json!(true)
        );

        // String concatenation
        assert_eq!(
            evaluate_binary_op(&json!("hello"), &BinaryOperator::Add, &json!(" world")).unwrap(),
            json!("hello world")
        );
    }

    #[test]
    fn test_unary_ops() {
        assert_eq!(
            evaluate_unary_op(&UnaryOperator::Not, &json!(true)).unwrap(),
            json!(false)
        );
        assert_eq!(
            evaluate_unary_op(&UnaryOperator::Negate, &json!(5)).unwrap(),
            json!(-5.0)
        );
    }

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("abc", "abd"), 1);
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn test_like_operator() {
        // % matches any sequence
        assert_eq!(
            evaluate_binary_op(
                &json!("hello world"),
                &BinaryOperator::Like,
                &json!("hello%")
            )
            .unwrap(),
            json!(true)
        );
        assert_eq!(
            evaluate_binary_op(
                &json!("hello world"),
                &BinaryOperator::Like,
                &json!("%world")
            )
            .unwrap(),
            json!(true)
        );
        assert_eq!(
            evaluate_binary_op(
                &json!("hello world"),
                &BinaryOperator::Like,
                &json!("%lo wo%")
            )
            .unwrap(),
            json!(true)
        );

        // _ matches single character
        assert_eq!(
            evaluate_binary_op(&json!("abc"), &BinaryOperator::Like, &json!("a_c")).unwrap(),
            json!(true)
        );
        assert_eq!(
            evaluate_binary_op(&json!("ac"), &BinaryOperator::Like, &json!("a_c")).unwrap(),
            json!(false)
        );
    }

    #[test]
    fn test_in_operator() {
        assert_eq!(
            evaluate_binary_op(&json!(2), &BinaryOperator::In, &json!([1, 2, 3])).unwrap(),
            json!(true)
        );
        assert_eq!(
            evaluate_binary_op(&json!(4), &BinaryOperator::In, &json!([1, 2, 3])).unwrap(),
            json!(false)
        );
        assert_eq!(
            evaluate_binary_op(&json!("a"), &BinaryOperator::In, &json!({"a": 1, "b": 2})).unwrap(),
            json!(true)
        );
    }
}
