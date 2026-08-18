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
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

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

/// Reference-returning sibling of [`get_field_value`]: walks the dotted path
/// and borrows the leaf instead of cloning it. `None` when any segment is
/// missing — the caller decides whether that reads as `Null`.
#[inline]
pub fn get_field_ref<'v>(value: &'v Value, field_path: &str) -> Option<&'v Value> {
    let mut current = value;
    for part in field_path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

/// Compare two rows of precomputed sort keys, honoring per-field direction.
#[inline]
pub fn compare_key_rows(a: &[Value], b: &[Value], ascending: &[bool]) -> Ordering {
    for ((a_val, b_val), asc) in a.iter().zip(b.iter()).zip(ascending.iter()) {
        let cmp = compare_values(a_val, b_val);
        if cmp != Ordering::Equal {
            return if *asc { cmp } else { cmp.reverse() };
        }
    }
    Ordering::Equal
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

        BinaryOperator::Spaceship => Ok(spaceship_value(left, right)),
        BinaryOperator::SemanticMatch => {
            let ls = value_as_text(left);
            let rs = value_as_text(right);
            Ok(Value::Bool(trigram_similarity(&ls, &rs) >= 0.45))
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

/// Stable 64-bit fingerprint of a JSON value (objects hashed in key order).
#[inline]
pub fn hash_value(v: &Value) -> u64 {
    let mut h = seahash::SeaHasher::new();
    write_value_hash(v, &mut h);
    h.finish()
}

fn write_value_hash(v: &Value, h: &mut impl Hasher) {
    match v {
        Value::Null => 0u8.hash(h),
        Value::Bool(b) => {
            1u8.hash(h);
            b.hash(h);
        }
        Value::Number(n) => {
            2u8.hash(h);
            n.as_f64().unwrap_or(0.0).to_bits().hash(h);
        }
        Value::String(s) => {
            3u8.hash(h);
            s.hash(h);
        }
        Value::Array(a) => {
            4u8.hash(h);
            a.len().hash(h);
            for x in a {
                write_value_hash(x, h);
            }
        }
        Value::Object(o) => {
            5u8.hash(h);
            o.len().hash(h);
            for (k, val) in o {
                k.hash(h);
                write_value_hash(val, h);
            }
        }
    }
}

/// Hash-set of JSON values. Expected O(1) insert/lookup; collisions fall
/// back to `values_equal`.
pub struct ValueSet {
    buckets: HashMap<u64, Vec<Value>>,
}

impl ValueSet {
    #[inline]
    pub fn with_capacity(n: usize) -> Self {
        Self {
            buckets: HashMap::with_capacity(n),
        }
    }

    /// Returns true if `v` was not already present.
    pub fn insert(&mut self, v: &Value) -> bool {
        let h = hash_value(v);
        let bucket = self.buckets.entry(h).or_default();
        if bucket.iter().any(|x| values_equal(x, v)) {
            false
        } else {
            bucket.push(v.clone());
            true
        }
    }

    #[inline]
    pub fn contains(&self, v: &Value) -> bool {
        self.buckets
            .get(&hash_value(v))
            .is_some_and(|b| b.iter().any(|x| values_equal(x, v)))
    }
}

fn value_as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn as_f64_vec(v: &Value) -> Option<Vec<f64>> {
    match v {
        Value::Array(a) => {
            let mut out = Vec::with_capacity(a.len());
            for x in a {
                out.push(x.as_f64()?);
            }
            Some(out)
        }
        Value::Object(o) => o.get("vector").and_then(as_f64_vec),
        _ => None,
    }
}

fn cosine_distance(a: &[f64], b: &[f64]) -> f64 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 1.0;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        1.0
    } else {
        (1.0 - dot / denom).clamp(0.0, 2.0)
    }
}

pub fn trigram_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    fn grams(s: &str) -> std::collections::HashSet<String> {
        let padded = format!("  {s} ");
        let chars: Vec<char> = padded.chars().collect();
        chars.windows(3).map(|w| w.iter().collect()).collect()
    }
    let ga = grams(a);
    let gb = grams(b);
    let inter = ga.intersection(&gb).count() as f64;
    let union = ga.union(&gb).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn spaceship_value(left: &Value, right: &Value) -> Value {
    if let (Some(a), Some(b)) = (as_f64_vec(left), as_f64_vec(right)) {
        return Value::Number(number_from_f64(cosine_distance(&a, &b)));
    }
    let n = match compare_values(left, right) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    };
    Value::Number(n.into())
}
