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

use serde_json::Value;

use super::utils::{number_from_f64, safe_regex};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{BinaryOperator, UnaryOperator};

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

#[inline]
pub fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        _ => left == right,
    }
}

#[inline]
pub fn evaluate_binary_op(left: &Value, op: &BinaryOperator, right: &Value) -> DbResult<Value> {
    match op {
        BinaryOperator::Equal => Ok(Value::Bool(values_equal(left, right))),
        BinaryOperator::NotEqual => Ok(Value::Bool(!values_equal(left, right))),

        BinaryOperator::LessThan => Ok(Value::Bool(
            compare_values(left, right) == Ordering::Less,
        )),
        BinaryOperator::LessThanOrEqual => Ok(Value::Bool(
            compare_values(left, right) != Ordering::Greater,
        )),
        BinaryOperator::GreaterThan => Ok(Value::Bool(
            compare_values(left, right) == Ordering::Greater,
        )),
        BinaryOperator::GreaterThanOrEqual => Ok(Value::Bool(
            compare_values(left, right) != Ordering::Less,
        )),
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
            // Escape regex characters
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

            // Use safe_regex for size limits (pattern is already escaped so ReDoS risk is low)
            match safe_regex(&regex_pattern) {
                Ok(re) => {
                    let is_match = re.is_match(s);
                    if matches!(op, BinaryOperator::NotLike) {
                        Ok(Value::Bool(!is_match))
                    } else {
                        Ok(Value::Bool(is_match))
                    }
                }
                Err(_) => Ok(Value::Bool(false)), // Invalid regex (shouldn't happen with escaped pattern)
            }
        }

        BinaryOperator::RegEx | BinaryOperator::NotRegEx => {
            let s = left.as_str().unwrap_or("");
            let pattern = right.as_str().unwrap_or("");

            // Use safe_regex to prevent DoS from malicious patterns
            match safe_regex(pattern) {
                Ok(re) => {
                    let is_match = re.is_match(s);
                    if matches!(op, BinaryOperator::NotRegEx) {
                        Ok(Value::Bool(!is_match))
                    } else {
                        Ok(Value::Bool(is_match))
                    }
                }
                Err(_) => Ok(Value::Bool(false)), // Invalid or oversized regex results in false
            }
        }

        BinaryOperator::FuzzyEqual => {
            let left_str = left.as_str().unwrap_or("");
            let right_str = right.as_str().unwrap_or("");
            let distance = crate::storage::levenshtein_distance(left_str, right_str);
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
                Err(DbError::ExecutionError(
                    "Cannot add these types".to_string(),
                ))
            }
        }

        BinaryOperator::Subtract => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(number_from_f64(a - b)))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot subtract non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::Multiply => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(number_from_f64(a * b)))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot multiply non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::Divide => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                if b == 0.0 {
                    Err(DbError::ExecutionError("Division by zero".to_string()))
                } else {
                    Ok(Value::Number(number_from_f64(a / b)))
                }
            } else {
                Err(DbError::ExecutionError(
                    "Cannot divide non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::Modulus => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                if b == 0.0 {
                    Err(DbError::ExecutionError("Division by zero".to_string()))
                } else {
                    Ok(Value::Number(number_from_f64(a % b)))
                }
            } else {
                Err(DbError::ExecutionError(
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
                Err(DbError::ExecutionError(
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
                Err(DbError::ExecutionError(
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
                Err(DbError::ExecutionError(
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
                Err(DbError::ExecutionError(
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
                Err(DbError::ExecutionError(
                    "Cannot right shift non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::Exponent => {
            if let (Some(base), Some(exp)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(number_from_f64(base.powf(exp))))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot exponentiate non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::NullCoalesce => {
            // Short-circuit evaluation is handled in evaluate_expr_with_context
            // This branch is here for exhaustiveness but shouldn't be reached
            if left.is_null() {
                Ok(right.clone())
            } else {
                Ok(left.clone())
            }
        }

        BinaryOperator::LogicalOr => {
            // Short-circuit evaluation is handled in evaluate_expr_with_context
            // This branch is here for exhaustiveness but shouldn't be reached
            if to_bool(left) {
                Ok(left.clone())
            } else {
                Ok(right.clone())
            }
        }
    }
}

#[inline]
pub fn evaluate_unary_op(op: &UnaryOperator, operand: &Value) -> DbResult<Value> {
    match op {
        UnaryOperator::Not => Ok(Value::Bool(!to_bool(operand))),
        UnaryOperator::Negate => {
            if let Some(n) = operand.as_f64() {
                Ok(Value::Number(number_from_f64(-n)))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot negate non-number".to_string(),
                ))
            }
        }
        UnaryOperator::BitwiseNot => {
            if let Some(n) = operand.as_f64() {
                Ok(Value::Number(serde_json::Number::from(!(n as i64))))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot bitwise NOT non-number".to_string(),
                ))
            }
        }
    }
}

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
