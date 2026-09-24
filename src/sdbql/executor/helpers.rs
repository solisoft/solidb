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

use super::utils::number_from_f64;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{BinaryOperator, UnaryOperator};

#[inline]
pub fn get_field_value(value: &Value, field_path: &str) -> Value {
    get_field_ref(value, field_path)
        .cloned()
        .unwrap_or(Value::Null)
}

/// Reference-returning sibling of [`get_field_value`]: walks the dotted path
/// and borrows the leaf instead of cloning it. `None` when any segment is
/// missing — the caller decides whether that reads as `Null`.
///
/// A key that itself contains a dot (`doc["a.b"]`, which the parser turns
/// into a field access on `"a.b"`) is looked up literally first; only when no
/// such key exists is the path split into segments.
#[inline]
pub fn get_field_ref<'v>(value: &'v Value, field_path: &str) -> Option<&'v Value> {
    if !field_path.contains('.') {
        return value.get(field_path);
    }
    if let Some(v) = value.as_object().and_then(|o| o.get(field_path)) {
        return Some(v);
    }
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

/// Integer view of a JSON number, when it is one (i64 or u64).
#[inline]
fn number_as_i128(n: &serde_json::Number) -> Option<i128> {
    n.as_i64()
        .map(i128::from)
        .or_else(|| n.as_u64().map(i128::from))
}

/// Numeric ordering: exact when both sides are integers (so values above
/// 2^53 do not collapse onto each other), IEEE otherwise.
#[inline]
fn compare_numbers(a: &serde_json::Number, b: &serde_json::Number) -> Ordering {
    if let (Some(x), Some(y)) = (number_as_i128(a), number_as_i128(b)) {
        return x.cmp(&y);
    }
    let x = a.as_f64().unwrap_or(0.0);
    let y = b.as_f64().unwrap_or(0.0);
    x.partial_cmp(&y).unwrap_or(Ordering::Equal)
}

#[inline]
fn numbers_equal(a: &serde_json::Number, b: &serde_json::Number) -> bool {
    if let (Some(x), Some(y)) = (number_as_i128(a), number_as_i128(b)) {
        return x == y;
    }
    a.as_f64() == b.as_f64()
}

/// `==` semantics. Numbers compare by value (`1 == 1.0`), also inside arrays
/// and objects, so equality agrees with [`compare_values`] returning `Equal`.
#[inline]
pub fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => numbers_equal(a, b),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| values_equal(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|w| values_equal(v, w)))
        }
        _ => left == right,
    }
}

// ---------------------------------------------------------------------------
// LIKE
// ---------------------------------------------------------------------------

/// How a LIKE pattern can be matched. Most patterns in practice are a literal
/// with `%` at one or both ends, which needs no regex at all.
#[derive(Debug, PartialEq, Eq)]
enum LikePlan<'p> {
    /// No wildcard: plain equality.
    Exact(&'p str),
    /// `abc%`
    Prefix(&'p str),
    /// `%abc`
    Suffix(&'p str),
    /// `%abc%`
    Contains(&'p str),
    /// Only `%`s: matches every string.
    Any,
    /// `_`, an escape, or an inner `%`: go through the regex translation.
    Regex,
}

fn like_plan(pattern: &str) -> LikePlan<'_> {
    if pattern.contains(['_', '\\']) {
        return LikePlan::Regex;
    }
    if !pattern.contains('%') {
        return LikePlan::Exact(pattern);
    }
    let leading = pattern.starts_with('%');
    let trailing = pattern.ends_with('%');
    let middle = pattern.trim_matches('%');
    if middle.is_empty() {
        return LikePlan::Any;
    }
    if middle.contains('%') {
        return LikePlan::Regex;
    }
    match (leading, trailing) {
        (true, true) => LikePlan::Contains(middle),
        (true, false) => LikePlan::Suffix(middle),
        (false, true) => LikePlan::Prefix(middle),
        (false, false) => LikePlan::Regex, // unreachable: a '%' exists and is not inner
    }
}

/// Case-sensitive SQL LIKE (`%` any run, `_` one character, `\%` / `\_` /
/// `\\` literals). Literal-with-`%`-ends patterns skip the regex engine; the
/// rest go through the shared LIKE translation and process-wide regex cache.
pub(crate) fn like_match(text: &str, pattern: &str) -> DbResult<bool> {
    Ok(match like_plan(pattern) {
        LikePlan::Exact(p) => text == p,
        LikePlan::Prefix(p) => text.starts_with(p),
        LikePlan::Suffix(p) => text.ends_with(p),
        LikePlan::Contains(p) => text.contains(p),
        LikePlan::Any => true,
        LikePlan::Regex => {
            use crate::sdbql::executor::builtins::string::{cached_regex_with, RegexKind};
            cached_regex_with(pattern, false, RegexKind::Like)?.is_match(text)
        }
    })
}

/// `=~`: a non-string subject never matches; an invalid pattern is an error
/// (as in `REGEX_TEST`), not a silent `false`.
fn regex_match(subject: &Value, pattern: &Value) -> DbResult<bool> {
    let (Some(s), Some(p)) = (subject.as_str(), pattern.as_str()) else {
        return Ok(false);
    };
    use crate::sdbql::executor::builtins::string::cached_regex_arc;
    Ok(cached_regex_arc(p)?.is_match(s))
}

// ---------------------------------------------------------------------------
// Arithmetic (AQL coercion)
// ---------------------------------------------------------------------------

/// An arithmetic operand after AQL's number conversion.
#[derive(Clone, Copy, Debug)]
enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    #[inline]
    fn as_f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Float(f) => f,
        }
    }
}

/// AQL `TO_NUMBER` as applied by the arithmetic operators: `null`, `false`,
/// objects, non-numeric strings and arrays of other than one element are 0;
/// `true` is 1; a numeric string is its value; `[x]` is `x` converted.
fn arith_operand(v: &Value) -> Num {
    match v {
        Value::Null => Num::Int(0),
        Value::Bool(b) => Num::Int(i64::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => Num::Int(i),
            None => Num::Float(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => {
            let t = s.trim();
            if let Ok(i) = t.parse::<i64>() {
                Num::Int(i)
            } else {
                match t.parse::<f64>() {
                    // Rust accepts "inf" / "NaN"; AQL does not.
                    Ok(f) if f.is_finite() => Num::Float(f),
                    _ => Num::Int(0),
                }
            }
        }
        Value::Array(a) if a.len() == 1 => arith_operand(&a[0]),
        Value::Array(_) | Value::Object(_) => Num::Int(0),
    }
}

#[inline]
fn int_value(i: i64) -> Value {
    Value::Number(serde_json::Number::from(i))
}

/// A float result; NaN and ±Infinity have no JSON form and read as `null`.
#[inline]
fn float_value(f: f64) -> Value {
    serde_json::Number::from_f64(f)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

/// `+ - * / % **` with AQL semantics: operands are converted to numbers,
/// integer arithmetic stays integral unless it would overflow, and division
/// or modulus by zero and non-finite results are `null`.
fn arithmetic(op: &BinaryOperator, left: &Value, right: &Value) -> Value {
    let a = arith_operand(left);
    let b = arith_operand(right);
    if let (Num::Int(x), Num::Int(y)) = (a, b) {
        let exact = match op {
            BinaryOperator::Add => x.checked_add(y),
            BinaryOperator::Subtract => x.checked_sub(y),
            BinaryOperator::Multiply => x.checked_mul(y),
            BinaryOperator::Divide => {
                if y == 0 {
                    return Value::Null;
                }
                match x.checked_rem(y) {
                    Some(0) => x.checked_div(y),
                    _ => None,
                }
            }
            BinaryOperator::Modulus => {
                if y == 0 {
                    return Value::Null;
                }
                // i64::MIN % -1 overflows; mathematically it is 0.
                Some(x.checked_rem(y).unwrap_or(0))
            }
            BinaryOperator::Exponent => u32::try_from(y).ok().and_then(|e| x.checked_pow(e)),
            _ => None,
        };
        if let Some(i) = exact {
            return int_value(i);
        }
    }
    let x = a.as_f64();
    let y = b.as_f64();
    let r = match op {
        BinaryOperator::Add => x + y,
        BinaryOperator::Subtract => x - y,
        BinaryOperator::Multiply => x * y,
        BinaryOperator::Divide => {
            if y == 0.0 {
                return Value::Null;
            }
            x / y
        }
        BinaryOperator::Modulus => {
            if y == 0.0 {
                return Value::Null;
            }
            x % y
        }
        BinaryOperator::Exponent => x.powf(y),
        _ => return Value::Null,
    };
    float_value(r)
}

/// Integer operand for a bitwise operator: an integral number that fits in
/// an i64. `Err` for non-numbers (as before), `Ok(None)` for numbers that
/// are not representable (fractional, out of range), which read as `null`.
fn bit_operand(v: &Value, what: &str) -> DbResult<Option<i64>> {
    match v {
        Value::Number(n) => Ok(match n.as_i64() {
            Some(i) => Some(i),
            None => n
                .as_f64()
                .filter(|f| f.is_finite() && f.fract() == 0.0)
                .filter(|f| (i64::MIN as f64..i64::MAX as f64).contains(f))
                .map(|f| f as i64),
        }),
        _ => Err(DbError::ExecutionError(format!(
            "Cannot {} non-numbers",
            what
        ))),
    }
}

fn bitwise(op: &BinaryOperator, left: &Value, right: &Value) -> DbResult<Value> {
    let what = match op {
        BinaryOperator::BitwiseAnd => "bitwise AND",
        BinaryOperator::BitwiseOr => "bitwise OR",
        BinaryOperator::BitwiseXor => "bitwise XOR",
        BinaryOperator::LeftShift => "left shift",
        _ => "right shift",
    };
    let (Some(a), Some(b)) = (bit_operand(left, what)?, bit_operand(right, what)?) else {
        return Ok(Value::Null);
    };
    let r = match op {
        BinaryOperator::BitwiseAnd => Some(a & b),
        BinaryOperator::BitwiseOr => Some(a | b),
        BinaryOperator::BitwiseXor => Some(a ^ b),
        // A shift by a negative amount or by 64+ bits has no defined result
        // (it panicked in debug builds); it reads as null.
        BinaryOperator::LeftShift => u32::try_from(b).ok().and_then(|s| a.checked_shl(s)),
        _ => u32::try_from(b).ok().and_then(|s| a.checked_shr(s)),
    };
    Ok(r.map(int_value).unwrap_or(Value::Null))
}

#[inline]
fn membership(left: &Value, right: &Value) -> bool {
    match right {
        Value::Array(arr) => arr.iter().any(|v| values_equal(left, v)),
        Value::Object(obj) => left.as_str().is_some_and(|s| obj.contains_key(s)),
        _ => false,
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
        BinaryOperator::In => Ok(Value::Bool(membership(left, right))),
        BinaryOperator::NotIn => Ok(Value::Bool(!membership(left, right))),

        // A non-string subject or pattern never matches (`null LIKE "%"` is
        // false); NOT LIKE is the negation of LIKE.
        BinaryOperator::Like | BinaryOperator::NotLike => {
            let is_match = match (left.as_str(), right.as_str()) {
                (Some(s), Some(p)) => like_match(s, p)?,
                _ => false,
            };
            Ok(Value::Bool(
                is_match != matches!(op, BinaryOperator::NotLike),
            ))
        }

        BinaryOperator::RegEx | BinaryOperator::NotRegEx => {
            let is_match = regex_match(left, right)?;
            Ok(Value::Bool(
                is_match != matches!(op, BinaryOperator::NotRegEx),
            ))
        }

        BinaryOperator::FuzzyEqual => {
            // `doc.missing ~= "jo"` used to compare "" with "jo" and match.
            let (Some(l), Some(r)) = (left.as_str(), right.as_str()) else {
                return Ok(Value::Bool(false));
            };
            let distance = crate::storage::levenshtein_distance(l, r);
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

        // SoliDB extension kept from before: two strings concatenate. Every
        // other combination follows AQL and adds numerically.
        BinaryOperator::Add => {
            if let (Value::String(a), Value::String(b)) = (left, right) {
                let mut s = String::with_capacity(a.len() + b.len());
                s.push_str(a);
                s.push_str(b);
                return Ok(Value::String(s));
            }
            Ok(arithmetic(op, left, right))
        }

        BinaryOperator::Subtract
        | BinaryOperator::Multiply
        | BinaryOperator::Divide
        | BinaryOperator::Modulus
        | BinaryOperator::Exponent => Ok(arithmetic(op, left, right)),

        BinaryOperator::BitwiseAnd
        | BinaryOperator::BitwiseOr
        | BinaryOperator::BitwiseXor
        | BinaryOperator::LeftShift
        | BinaryOperator::RightShift => bitwise(op, left, right),

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
        // AQL: the operand is converted to a number first (`-null` is 0).
        UnaryOperator::Negate => Ok(match arith_operand(operand) {
            Num::Int(i) => i
                .checked_neg()
                .map(int_value)
                .unwrap_or_else(|| float_value(-(i as f64))),
            Num::Float(f) => float_value(-f),
        }),
        UnaryOperator::BitwiseNot => Ok(bit_operand(operand, "bitwise NOT")?
            .map(|n| int_value(!n))
            .unwrap_or(Value::Null)),
    }
}

/// AQL truthiness, shared by FILTER, `!`, `AND`/`OR`, the ternary, `IF` and
/// row policies: `null`, `false`, `0` and `""` are falsy; everything else —
/// including `[]` and `{}` — is truthy.
#[inline]
pub fn to_bool(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

#[inline]
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

/// Total order over JSON values, following AQL: `null < bool < number <
/// string < array < object`. Arrays compare element by element (a proper
/// prefix is smaller); objects compare their sorted key lists first, then the
/// values key by key. Returns `Equal` exactly when [`values_equal`] is true.
#[inline]
pub fn compare_values(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => compare_numbers(x, y),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Array(x), Value::Array(y)) => {
            for (p, q) in x.iter().zip(y.iter()) {
                let c = compare_values(p, q);
                if c != Ordering::Equal {
                    return c;
                }
            }
            x.len().cmp(&y.len())
        }
        (Value::Object(x), Value::Object(y)) => {
            // serde_json's Map (no `preserve_order`) iterates in key order.
            let keys = x.keys().cmp(y.keys());
            if keys != Ordering::Equal {
                return keys;
            }
            for (p, q) in x.values().zip(y.values()) {
                let c = compare_values(p, q);
                if c != Ordering::Equal {
                    return c;
                }
            }
            Ordering::Equal
        }
        _ => type_rank(a).cmp(&type_rank(b)),
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
            // -0.0 == 0.0 under values_equal, so they must hash alike.
            let f = n.as_f64().unwrap_or(0.0);
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

    /// A set holding each distinct element of `values`.
    pub fn from_values(values: &[Value]) -> Self {
        let mut set = Self::with_capacity(values.len());
        for v in values {
            set.insert(v);
        }
        set
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn op(l: Value, o: BinaryOperator, r: Value) -> Value {
        evaluate_binary_op(&l, &o, &r).unwrap()
    }

    #[test]
    fn compare_follows_aql_type_order() {
        let ordered = [
            json!(null),
            json!(false),
            json!(true),
            json!(-1),
            json!(0.5),
            json!(18),
            json!(""),
            json!("17"),
            json!("abc"),
            json!([]),
            json!([1]),
            json!([1, 2]),
            json!([2]),
            json!({}),
            json!({"a": 1}),
            json!({"a": 2}),
            json!({"b": 0}),
        ];
        for (i, a) in ordered.iter().enumerate() {
            for (j, b) in ordered.iter().enumerate() {
                assert_eq!(compare_values(a, b), i.cmp(&j), "{a} vs {b}");
            }
        }
    }

    #[test]
    fn mixed_type_relational_operators() {
        // AQL type order: null < bool < number < string < array < object.
        // So `x >= 18` is false for null and bools and true for any string.
        for v in [json!(true), json!(null)] {
            assert_eq!(
                op(v.clone(), BinaryOperator::GreaterThanOrEqual, json!(18)),
                json!(false),
                "{v}"
            );
        }
        for v in [json!("17"), json!("abc")] {
            assert_eq!(
                op(v.clone(), BinaryOperator::GreaterThanOrEqual, json!(18)),
                json!(true),
                "{v}"
            );
        }
        for v in [json!([1]), json!({"a": 1})] {
            assert_eq!(
                op(v.clone(), BinaryOperator::LessThanOrEqual, json!(18)),
                json!(false),
                "{v}"
            );
        }
    }

    #[test]
    fn large_integers_compare_exactly() {
        let a = json!(9_007_199_254_740_993i64);
        let b = json!(9_007_199_254_740_992i64);
        assert!(!values_equal(&a, &b));
        assert_eq!(compare_values(&a, &b), Ordering::Greater);
        assert_eq!(
            compare_values(&json!(u64::MAX), &json!(-1)),
            Ordering::Greater
        );
        assert!(values_equal(&json!(1), &json!(1.0)));
        assert!(values_equal(
            &json!([1, {"a": 2}]),
            &json!([1.0, {"a": 2.0}])
        ));
        assert_eq!(hash_value(&json!(0.0)), hash_value(&json!(-0.0)));
    }

    #[test]
    fn arithmetic_follows_aql() {
        assert_eq!(op(json!(null), BinaryOperator::Add, json!(1)), json!(1));
        assert_eq!(op(json!(true), BinaryOperator::Add, json!(1)), json!(2));
        assert_eq!(op(json!("2"), BinaryOperator::Multiply, json!(3)), json!(6));
        assert_eq!(op(json!("x"), BinaryOperator::Add, json!(1)), json!(1));
        assert_eq!(op(json!([5]), BinaryOperator::Subtract, json!(1)), json!(4));
        assert_eq!(op(json!("a"), BinaryOperator::Add, json!("b")), json!("ab"));
        assert_eq!(op(json!(1), BinaryOperator::Add, json!(2)), json!(3));
        assert_eq!(op(json!(1), BinaryOperator::Add, json!(0.5)), json!(1.5));
        assert_eq!(op(json!(6), BinaryOperator::Divide, json!(3)), json!(2));
        assert_eq!(op(json!(7), BinaryOperator::Divide, json!(2)), json!(3.5));
        assert_eq!(op(json!(1), BinaryOperator::Divide, json!(0)), json!(null));
        assert_eq!(op(json!(1), BinaryOperator::Modulus, json!(0)), json!(null));
        assert_eq!(
            op(json!(1.5), BinaryOperator::Divide, json!(0.0)),
            json!(null)
        );
        assert_eq!(op(json!(-7), BinaryOperator::Modulus, json!(3)), json!(-1));
        assert_eq!(
            op(json!(i64::MIN), BinaryOperator::Modulus, json!(-1)),
            json!(0)
        );
        // Overflow falls back to floating point instead of wrapping.
        assert_eq!(
            op(json!(i64::MAX), BinaryOperator::Add, json!(1)),
            json!(i64::MAX as f64 + 1.0)
        );
        assert_eq!(
            op(json!(2), BinaryOperator::Exponent, json!(10)),
            json!(1024)
        );
        assert_eq!(
            op(json!(10), BinaryOperator::Exponent, json!(400)),
            json!(null)
        );
        assert_eq!(
            evaluate_unary_op(&UnaryOperator::Negate, &json!(null)).unwrap(),
            json!(0)
        );
    }

    #[test]
    fn shifts_are_checked() {
        assert_eq!(op(json!(1), BinaryOperator::LeftShift, json!(2)), json!(4));
        assert_eq!(
            op(json!(1), BinaryOperator::LeftShift, json!(64)),
            json!(null)
        );
        assert_eq!(
            op(json!(1), BinaryOperator::RightShift, json!(-1)),
            json!(null)
        );
        assert!(evaluate_binary_op(&json!("a"), &BinaryOperator::BitwiseAnd, &json!(1)).is_err());
    }

    #[test]
    fn truthiness_matches_aql() {
        for v in [json!(null), json!(false), json!(0), json!(0.0), json!("")] {
            assert!(!to_bool(&v), "{v}");
        }
        for v in [json!([]), json!({}), json!("false"), json!(-1), json!("0")] {
            assert!(to_bool(&v), "{v}");
        }
    }

    #[test]
    fn like_plans() {
        assert_eq!(like_plan("abc"), LikePlan::Exact("abc"));
        assert_eq!(like_plan("abc%"), LikePlan::Prefix("abc"));
        assert_eq!(like_plan("%abc"), LikePlan::Suffix("abc"));
        assert_eq!(like_plan("%%abc%"), LikePlan::Contains("abc"));
        assert_eq!(like_plan("%"), LikePlan::Any);
        assert_eq!(like_plan("a%c"), LikePlan::Regex);
        assert_eq!(like_plan("a_c"), LikePlan::Regex);
        assert_eq!(like_plan("100\\%"), LikePlan::Regex);
    }

    #[test]
    fn like_operator_semantics() {
        assert_eq!(
            op(json!("Alice"), BinaryOperator::Like, json!("A%")),
            json!(true)
        );
        assert_eq!(
            op(json!("line1\nline2"), BinaryOperator::Like, json!("line1%")),
            json!(true)
        );
        assert_eq!(
            op(json!("abc"), BinaryOperator::Like, json!("%b%")),
            json!(true)
        );
        assert_eq!(
            op(json!("abc"), BinaryOperator::Like, json!("a_c")),
            json!(true)
        );
        assert_eq!(
            op(json!("abc"), BinaryOperator::NotLike, json!("x%")),
            json!(true)
        );
        assert_eq!(
            op(json!(null), BinaryOperator::Like, json!("%")),
            json!(false)
        );
        assert_eq!(
            op(json!(null), BinaryOperator::NotLike, json!("%")),
            json!(true)
        );
        assert_eq!(
            op(json!(null), BinaryOperator::FuzzyEqual, json!("jo")),
            json!(false)
        );
        assert_eq!(
            op(json!(null), BinaryOperator::RegEx, json!(".*")),
            json!(false)
        );
        assert!(evaluate_binary_op(&json!("a"), &BinaryOperator::RegEx, &json!("(")).is_err());
    }

    #[test]
    fn dotted_keys_are_looked_up_literally_first() {
        let doc = json!({"a.b": 1, "a": {"b": 2}, "x": {"y": 3}});
        assert_eq!(get_field_value(&doc, "a.b"), json!(1));
        assert_eq!(get_field_value(&doc, "x.y"), json!(3));
    }
}
