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
use std::sync::{LazyLock, Mutex};

use regex::Regex;
use serde_json::Value;

use crate::ast::{BinaryOperator, UnaryOperator};
use crate::error::{SdbqlError, SdbqlResult};

static REGEX_CACHE: LazyLock<Mutex<HashMap<String, Regex>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const REGEX_CACHE_SIZE: usize = 1000;

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
/// Numbers compare by value at every depth, so `1 == 1.0` and
/// `[1] == [1.0]`; integers compare exactly, not through `f64`.
#[inline]
pub fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => compare_numbers(a, b) == Ordering::Equal,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| values_equal(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|w| values_equal(v, w)))
        }
        _ => left == right,
    }
}

fn int128(n: &serde_json::Number) -> Option<i128> {
    n.as_i64()
        .map(i128::from)
        .or_else(|| n.as_u64().map(i128::from))
}

/// Numeric ordering: exact for two integers, `f64` otherwise.
#[inline]
pub fn compare_numbers(a: &serde_json::Number, b: &serde_json::Number) -> Ordering {
    if let (Some(x), Some(y)) = (int128(a), int128(b)) {
        return x.cmp(&y);
    }
    let x = a.as_f64().unwrap_or(0.0);
    let y = b.as_f64().unwrap_or(0.0);
    x.partial_cmp(&y).unwrap_or(Ordering::Equal)
}

/// Stable 64-bit fingerprint of a JSON value, consistent with
/// [`values_equal`]: `1` and `1.0` hash alike, and object keys are hashed in
/// sorted order.
pub fn hash_value(v: &Value) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
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
            let f = n.as_f64().unwrap_or(0.0);
            // -0.0 == 0.0, so they must share a hash.
            let f = if f == 0.0 { 0.0 } else { f };
            f.to_bits().hash(h);
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
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            for k in keys {
                k.hash(h);
                if let Some(val) = o.get(k) {
                    write_value_hash(val, h);
                }
            }
        }
    }
}

/// Hash set of JSON values using [`values_equal`] semantics.
#[derive(Default, Clone)]
pub struct ValueSet {
    buckets: HashMap<u64, Vec<Value>>,
}

impl ValueSet {
    /// Create a set sized for `n` values.
    pub fn with_capacity(n: usize) -> Self {
        Self {
            buckets: HashMap::with_capacity(n),
        }
    }

    /// Insert `v`; returns true if it was not already present.
    pub fn insert(&mut self, v: &Value) -> bool {
        let bucket = self.buckets.entry(hash_value(v)).or_default();
        if bucket.iter().any(|x| values_equal(x, v)) {
            false
        } else {
            bucket.push(v.clone());
            true
        }
    }

    /// True if an equal value is in the set.
    pub fn contains(&self, v: &Value) -> bool {
        self.buckets
            .get(&hash_value(v))
            .is_some_and(|b| b.iter().any(|x| values_equal(x, v)))
    }
}

/// Create a serde_json::Number from an f64 value.
#[inline]
pub fn number_from_f64(n: f64) -> serde_json::Number {
    serde_json::Number::from_f64(n).unwrap_or_else(|| serde_json::Number::from(0))
}

/// Longest regex pattern accepted, in bytes (same as the server).
pub const MAX_REGEX_PATTERN_LEN: usize = 1024;
/// Compiled-program size limit, in bytes (same as the server).
pub const MAX_REGEX_SIZE: usize = 1 << 20;

/// Safely compile a regex with size limits to prevent ReDoS attacks.
/// Uses a global cache to avoid recompiling frequently used patterns.
pub fn safe_regex(pattern: &str) -> Result<Regex, regex::Error> {
    if pattern.len() > MAX_REGEX_PATTERN_LEN {
        return Err(regex::Error::Syntax(format!(
            "Pattern too long ({} bytes, max {})",
            pattern.len(),
            MAX_REGEX_PATTERN_LEN
        )));
    }

    let compile = || {
        regex::RegexBuilder::new(pattern)
            .size_limit(MAX_REGEX_SIZE)
            .build()
    };

    if let Ok(mut cache) = REGEX_CACHE.lock() {
        if let Some(cached) = cache.get(pattern) {
            return Ok(cached.clone());
        }
        let re = compile()?;
        if cache.len() < REGEX_CACHE_SIZE {
            cache.insert(pattern.to_string(), re.clone());
        }
        Ok(re)
    } else {
        compile()
    }
}

/// [`safe_regex`] with the error mapped to an [`SdbqlError`].
pub fn compile_regex(pattern: &str) -> SdbqlResult<Regex> {
    safe_regex(pattern)
        .map_err(|e| SdbqlError::ExecutionError(format!("Invalid regex pattern: {}", e)))
}

/// Translate a SQL LIKE pattern (`%`, `_`) into an anchored regex.
pub fn like_to_regex(pattern: &str) -> String {
    let mut regex_pattern = String::with_capacity(pattern.len() + 2);
    regex_pattern.push('^');
    for c in pattern.chars() {
        match c {
            '%' => regex_pattern.push_str(".*"),
            '_' => regex_pattern.push('.'),
            '^' | '$' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                regex_pattern.push('\\');
                regex_pattern.push(c);
            }
            _ => regex_pattern.push(c),
        }
    }
    regex_pattern.push('$');
    regex_pattern
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

            let re = compile_regex(&like_to_regex(pattern))?;
            let is_match = re.is_match(s);
            if matches!(op, BinaryOperator::NotLike) {
                Ok(Value::Bool(!is_match))
            } else {
                Ok(Value::Bool(is_match))
            }
        }

        BinaryOperator::RegEx | BinaryOperator::NotRegEx => {
            let s = left.as_str().unwrap_or("");
            let pattern = right.as_str().unwrap_or("");

            // An invalid pattern is an error, as in REGEX_TEST, not a
            // silent non-match.
            let re = compile_regex(pattern)?;
            let is_match = re.is_match(s);
            if matches!(op, BinaryOperator::NotRegEx) {
                Ok(Value::Bool(!is_match))
            } else {
                Ok(Value::Bool(is_match))
            }
        }

        BinaryOperator::FuzzyEqual => {
            let left_str = left.as_str().unwrap_or("");
            let right_str = right.as_str().unwrap_or("");
            let distance = levenshtein_distance(left_str, right_str);
            Ok(Value::Bool(distance <= 2)) // Default max distance of 2
        }
        BinaryOperator::Spaceship => Ok(spaceship_value(left, right)),
        BinaryOperator::SemanticMatch => {
            let ls = match left {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let rs = match right {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            Ok(Value::Bool(trigram_sim(&ls, &rs) >= 0.45))
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

fn type_rank(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

/// Compare two JSON values for ordering (AQL type order, a total order).
///
/// Null < Bool < Number < String < Array < Object.
/// Arrays are compared lexicographically element-by-element. Objects are
/// compared attribute by attribute over the sorted union of their keys, a
/// missing attribute counting as null.
#[inline]
pub fn compare_values(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Number(a), Value::Number(b)) => compare_numbers(a, b),
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::Array(a), Value::Array(b)) => compare_arrays(a, b),
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let x = a.get(k).unwrap_or(&Value::Null);
                let y = b.get(k).unwrap_or(&Value::Null);
                let c = compare_values(x, y);
                if c != Ordering::Equal {
                    return c;
                }
            }
            Ordering::Equal
        }
        _ => type_rank(a).cmp(&type_rank(b)),
    }
}

fn compare_arrays(a: &[Value], b: &[Value]) -> Ordering {
    let min_len = a.len().min(b.len());
    for i in 0..min_len {
        let cmp = compare_values(&a[i], &b[i]);
        if cmp != Ordering::Equal {
            return cmp;
        }
    }
    a.len().cmp(&b.len())
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

fn trigram_sim(a: &str, b: &str) -> f64 {
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
        if a.is_empty() || b.is_empty() || a.len() != b.len() {
            return Value::Number(number_from_f64(1.0));
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
        let d = if denom == 0.0 {
            1.0
        } else {
            (1.0 - dot / denom).clamp(0.0, 2.0)
        };
        return Value::Number(number_from_f64(d));
    }
    let n = match compare_values(left, right) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    };
    Value::Number(n.into())
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
        assert_eq!(
            evaluate_binary_op(&json!(1), &BinaryOperator::Spaceship, &json!(2)).unwrap(),
            json!(-1)
        );
        assert_eq!(
            evaluate_binary_op(
                &json!([1.0, 0.0]),
                &BinaryOperator::Spaceship,
                &json!([1.0, 0.0])
            )
            .unwrap()
            .as_f64()
            .unwrap(),
            0.0
        );
        assert_eq!(
            evaluate_binary_op(&json!("aa"), &BinaryOperator::SemanticMatch, &json!("aa")).unwrap(),
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
