//! A value read for `COLLECT`: a JSON value, or a string borrowed straight
//! from a stored document.
//!
//! Group keys and aggregate arguments used to be `serde_json::Value`s, so a
//! string read from a document had to become a `String` — an allocation per
//! row — only to be hashed, compared with the group's key and dropped. A
//! [`Datum`] lets the typed scan (`execution::typed_scan`) hand the fold a
//! `&str` pointing into the stored MessagePack instead. Every other scalar is
//! a `Value` already (numbers, booleans and null own no heap memory), and
//! arrays and objects are decoded as before.
//!
//! The rules are the ones `Value`s follow, so a group or an aggregate cannot
//! tell where its input came from: equality is [`values_equal`] (`1 == 1.0`,
//! `-0.0 == 0`), order is [`compare_values`], and [`key_hash`] hashes a
//! `Datum::Str` exactly as it hashes the `Value::String` it stands for.

use std::borrow::Cow;
use std::cmp::Ordering;

use serde_json::Value;

use super::{compare_values, values_equal};

#[derive(Debug, Clone)]
pub(crate) enum Datum<'a> {
    Null,
    Bool(bool),
    /// A number: no heap memory, so no drop work either.
    Num(serde_json::Number),
    /// A string, borrowed (from a stored document, or from a `Value`).
    Str(&'a str),
    /// An array or object, borrowed or owned.
    Val(Cow<'a, Value>),
}

/// `null`, for filling arrays.
pub(crate) const NULL: Datum<'static> = Datum::Null;

impl<'a> Datum<'a> {
    /// A `Value` as a datum, borrowing its string if it is one.
    #[inline]
    pub(crate) fn of(value: Cow<'a, Value>) -> Self {
        match value {
            Cow::Borrowed(Value::String(s)) => Datum::Str(s),
            Cow::Borrowed(Value::Null) | Cow::Owned(Value::Null) => Datum::Null,
            Cow::Borrowed(Value::Bool(b)) => Datum::Bool(*b),
            Cow::Owned(Value::Bool(b)) => Datum::Bool(b),
            Cow::Borrowed(Value::Number(n)) => Datum::Num(n.clone()),
            Cow::Owned(Value::Number(n)) => Datum::Num(n),
            other => Datum::Val(other),
        }
    }

    /// The same datum, borrowing what this one owns.
    #[inline]
    pub(crate) fn reborrow(&self) -> Datum<'_> {
        match self {
            Datum::Null => Datum::Null,
            Datum::Bool(b) => Datum::Bool(*b),
            Datum::Num(n) => Datum::Num(n.clone()),
            Datum::Str(s) => Datum::Str(s),
            Datum::Val(v) => Datum::Val(Cow::Borrowed(v.as_ref())),
        }
    }

    #[inline]
    pub(crate) fn is_null(&self) -> bool {
        match self {
            Datum::Null => true,
            Datum::Val(v) => v.is_null(),
            _ => false,
        }
    }

    /// The number, as `Value::as_f64` reads it; `None` for anything else.
    #[inline]
    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            Datum::Num(n) => n.as_f64(),
            Datum::Val(v) => v.as_f64(),
            _ => None,
        }
    }

    /// The value as a `Value` when that needs no allocation.
    #[inline]
    fn scalar(&self) -> Option<Value> {
        match self {
            Datum::Null => Some(Value::Null),
            Datum::Bool(b) => Some(Value::Bool(*b)),
            Datum::Num(n) => Some(Value::Number(n.clone())),
            _ => None,
        }
    }

    /// An integer-valued view for the bit aggregates; `None` for
    /// non-numbers.
    #[inline]
    pub(crate) fn as_value(&self) -> Option<Cow<'_, Value>> {
        match self {
            Datum::Str(_) => None,
            Datum::Val(v) => Some(Cow::Borrowed(v.as_ref())),
            other => other.scalar().map(Cow::Owned),
        }
    }

    /// The value, copied out where it is borrowed.
    pub(crate) fn into_value(self) -> Value {
        match self {
            Datum::Null => Value::Null,
            Datum::Bool(b) => Value::Bool(b),
            Datum::Num(n) => Value::Number(n),
            Datum::Str(s) => Value::String(s.to_owned()),
            Datum::Val(v) => v.into_owned(),
        }
    }

    pub(crate) fn into_cow(self) -> Cow<'a, Value> {
        match self {
            Datum::Val(v) => v,
            other => Cow::Owned(other.into_value()),
        }
    }

    /// `values_equal(self, other)`.
    #[inline]
    pub(crate) fn eq_value(&self, other: &Value) -> bool {
        match self {
            Datum::Null => other.is_null(),
            Datum::Bool(b) => other.as_bool() == Some(*b),
            Datum::Num(n) => match other {
                Value::Number(m) => {
                    values_equal(&Value::Number(n.clone()), &Value::Number(m.clone()))
                }
                _ => false,
            },
            Datum::Str(s) => other.as_str() == Some(*s),
            Datum::Val(v) => values_equal(v, other),
        }
    }

    /// `compare_values(self, other)`.
    #[inline]
    pub(crate) fn cmp_value(&self, other: &Value) -> Ordering {
        match self {
            Datum::Str(s) => match other {
                Value::String(t) => (*s).cmp(t.as_str()),
                _ => STRING_RANK.cmp(&type_rank(other)),
            },
            Datum::Val(v) => compare_values(v, other),
            scalar => compare_values(&scalar.scalar().expect("a scalar"), other),
        }
    }

    /// AQL truthiness, as `to_bool` reads the value.
    #[inline]
    pub(crate) fn truthy(&self) -> bool {
        match self {
            Datum::Null => false,
            Datum::Bool(b) => *b,
            Datum::Num(n) => n.as_f64().unwrap_or(0.0) != 0.0,
            Datum::Str(s) => !s.is_empty(),
            Datum::Val(v) => super::to_bool(v),
        }
    }
}

const STRING_RANK: u8 = 3;

/// `compare_values`' order of types: null, bool, number, string, array,
/// object.
fn type_rank(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => STRING_RANK,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

/// A fast, non-cryptographic hash for group keys, which live only as long as
/// one query. Values that are `values_equal` hash alike: numbers by their
/// `f64` (`-0.0` as `0.0`), and a borrowed string like the owned one.
#[derive(Default)]
pub(crate) struct KeyHasher(u64);

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl KeyHasher {
    #[inline]
    fn word(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(SEED);
    }

    #[inline]
    fn bytes(&mut self, b: &[u8]) {
        let (words, rest) = b.as_chunks::<8>();
        for w in words {
            self.word(u64::from_le_bytes(*w));
        }
        if !rest.is_empty() {
            let tail = rest
                .iter()
                .rev()
                .fold(0u64, |acc, &b| (acc << 8) | b as u64);
            self.word(tail);
        }
        self.word(b.len() as u64);
    }

    #[inline]
    fn string(&mut self, s: &str) {
        self.word(3);
        self.bytes(s.as_bytes());
    }

    #[inline]
    fn number(&mut self, n: &serde_json::Number) {
        let f = n.as_f64().unwrap_or(0.0);
        let f = if f == 0.0 { 0.0 } else { f };
        self.word(2);
        self.word(f.to_bits());
    }

    fn value(&mut self, v: &Value) {
        match v {
            Value::Null => self.word(0),
            Value::Bool(b) => self.word(1 + ((*b as u64) << 8)),
            Value::Number(n) => self.number(n),
            Value::String(s) => self.string(s),
            Value::Array(a) => {
                self.word(4);
                self.word(a.len() as u64);
                for x in a {
                    self.value(x);
                }
            }
            Value::Object(o) => {
                self.word(5);
                self.word(o.len() as u64);
                for (k, x) in o {
                    self.bytes(k.as_bytes());
                    self.value(x);
                }
            }
        }
    }

    #[inline]
    pub(crate) fn datum(&mut self, d: &Datum<'_>) {
        match d {
            Datum::Null => self.word(0),
            Datum::Bool(b) => self.word(1 + ((*b as u64) << 8)),
            Datum::Num(n) => self.number(n),
            Datum::Str(s) => self.string(s),
            Datum::Val(v) => self.value(v),
        }
    }

    /// The hash, with its bits mixed so that any of them can index a table.
    #[inline]
    pub(crate) fn finish(&self) -> u64 {
        let mut x = self.0;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 33;
        x
    }
}

/// The hash of a group key given as datums.
#[inline]
pub(crate) fn key_hash(key: &[Datum<'_>]) -> u64 {
    let mut h = KeyHasher::default();
    for d in key {
        h.datum(d);
    }
    h.finish()
}

/// The hash of a group key given as values: [`key_hash`] of the same key.
pub(crate) fn value_key_hash(key: &[Value]) -> u64 {
    let mut h = KeyHasher::default();
    for v in key {
        h.value(v);
    }
    h.finish()
}

/// A `HashMap` hasher for keys that are already hashes.
#[derive(Default, Clone, Copy)]
pub(crate) struct PreHashed(u64);

impl std::hash::Hasher for PreHashed {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        // Only u64 keys are hashed with it; anything else still works.
        for &b in bytes {
            self.0 = (self.0 << 8) | b as u64;
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

pub(crate) type PreHashedBuild = std::hash::BuildHasherDefault<PreHashed>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_borrowed_string_is_the_string() {
        let owned = json!("paid");
        let d = Datum::Str("paid");
        assert!(d.eq_value(&owned));
        assert!(!d.eq_value(&json!("pai")));
        assert!(!Datum::Str("1").eq_value(&json!(1)));
        assert_eq!(
            key_hash(&[Datum::Str("paid")]),
            value_key_hash(std::slice::from_ref(&owned))
        );
        assert_eq!(
            key_hash(&[Datum::of(Cow::Borrowed(&owned))]),
            key_hash(&[Datum::Str("paid")])
        );
        assert_eq!(d.cmp_value(&json!("pa")), Ordering::Greater);
        assert_eq!(d.cmp_value(&json!(5)), Ordering::Greater);
        assert_eq!(d.cmp_value(&json!([1])), Ordering::Less);
        assert_eq!(d.cmp_value(&json!(null)), Ordering::Greater);
        assert!(d.truthy());
        assert!(!Datum::Str("").truthy());
    }

    #[test]
    fn scalars_are_their_values() {
        for v in [
            json!(null),
            json!(true),
            json!(false),
            json!(1),
            json!(1.0),
            json!(-0.0),
            json!(u64::MAX),
            json!(-5),
            json!(2.5),
        ] {
            let d = Datum::of(Cow::Borrowed(&v));
            assert!(!matches!(d, Datum::Val(_)), "{v}");
            assert_eq!(d.clone().into_value(), v);
            assert_eq!(
                key_hash(std::slice::from_ref(&d)),
                value_key_hash(std::slice::from_ref(&v))
            );
            for w in [
                json!(null),
                json!(false),
                json!(1),
                json!(1.0),
                json!(0),
                json!("a"),
                json!([1]),
                json!({"a": 1}),
                json!(u64::MAX),
                json!(2.5),
            ] {
                assert_eq!(d.eq_value(&w), values_equal(&v, &w), "{v} == {w}");
                assert_eq!(d.cmp_value(&w), compare_values(&v, &w), "{v} <=> {w}");
            }
            assert_eq!(d.truthy(), super::super::to_bool(&v));
            assert_eq!(d.as_f64(), v.as_f64());
            assert_eq!(d.is_null(), v.is_null());
        }
    }

    #[test]
    fn equal_values_hash_alike() {
        let pairs = [
            (json!(1), json!(1.0)),
            (json!(-0.0), json!(0)),
            (json!([1, 2]), json!([1.0, 2])),
            (json!({"x": 1}), json!({"x": 1.0})),
        ];
        for (a, b) in pairs {
            assert!(values_equal(&a, &b));
            assert_eq!(
                value_key_hash(std::slice::from_ref(&a)),
                value_key_hash(std::slice::from_ref(&b)),
                "{a} {b}"
            );
        }
        // Composite keys are not confused by where one part ends.
        assert_ne!(
            value_key_hash(&[json!("ab"), json!("c")]),
            value_key_hash(&[json!("a"), json!("bc")])
        );
        assert_ne!(
            value_key_hash(&[json!(null)]),
            value_key_hash(&[json!(false)])
        );
    }
}
