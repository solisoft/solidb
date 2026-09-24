//! Helpers shared by the builtin modules: argument coercion, AQL string
//! conversion, error constructors and the output caps.

use serde_json::{Number, Value};

use crate::error::{SdbqlError, SdbqlResult};

/// Largest string REPEAT, PAD_*, REPLACE and REGEX_REPLACE may build.
/// Same value as the server's `MAX_REPEAT_BYTES`.
pub const MAX_REPEAT_BYTES: usize = 1_048_576;

/// Largest array RANGE (and the `a..b` operator) may build. Same as the
/// server.
pub const MAX_RANGE: usize = 1_000_000;

pub fn err(msg: impl Into<String>) -> SdbqlError {
    SdbqlError::ExecutionError(msg.into())
}

pub fn arity(name: &str, expected: &str) -> SdbqlError {
    err(format!("{} requires {} argument(s)", name, expected))
}

/// Error unless `min <= args.len() <= max`.
pub fn check_arity(name: &str, args: &[Value], min: usize, max: usize) -> SdbqlResult<()> {
    if args.len() < min || args.len() > max {
        let expected = if min == max {
            min.to_string()
        } else if max == usize::MAX {
            format!("at least {}", min)
        } else {
            format!("{}-{}", min, max)
        };
        return Err(arity(name, &expected));
    }
    Ok(())
}

/// Read an integer argument. Floats are truncated (`2.0` from `4 / 2` is
/// 2), out-of-range values saturate. Non-numbers give `None`.
pub fn as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_u64().map(|u| i64::try_from(u).unwrap_or(i64::MAX)))
            // `as` saturates and maps NaN to 0; it never panics.
            .or_else(|| n.as_f64().map(|f| f as i64)),
        _ => None,
    }
}

/// [`as_int`] with a default for missing or non-numeric values.
pub fn int_arg(args: &[Value], i: usize, default: i64) -> i64 {
    args.get(i).and_then(as_int).unwrap_or(default)
}

/// Format a number the way AQL prints it: integral floats without `.0`.
pub fn format_number(n: &Number) -> String {
    if n.is_f64() {
        if let Some(f) = n.as_f64() {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                return format!("{}", f as i64);
            }
        }
    }
    n.to_string()
}

/// AQL `TO_STRING`: null is `""`, numbers print without a trailing `.0`,
/// arrays and objects are JSON.
pub fn stringify(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_number(n),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// AQL `TO_NUMBER`: null/false/unparseable are 0, true is 1, strings are
/// trimmed and parsed (integer first), a one-element array converts its
/// element.
pub fn to_number(v: &Value) -> Value {
    match v {
        Value::Null => Value::from(0),
        Value::Bool(b) => Value::from(i64::from(*b)),
        Value::Number(n) => Value::Number(n.clone()),
        Value::String(s) => {
            let t = s.trim();
            if let Ok(i) = t.parse::<i64>() {
                Value::from(i)
            } else {
                match t.parse::<f64>() {
                    // `parse` accepts "inf" and "NaN"; AQL does not.
                    Ok(f) if f.is_finite() => Number::from_f64(f)
                        .map(Value::Number)
                        .unwrap_or_else(|| Value::from(0)),
                    _ => Value::from(0),
                }
            }
        }
        Value::Array(a) if a.len() == 1 => to_number(&a[0]),
        Value::Array(_) | Value::Object(_) => Value::from(0),
    }
}

/// AQL `LENGTH`: characters of a string (or of a number's text), elements
/// of an array, attributes of an object; true is 1, null and false are 0.
pub fn length_of(v: &Value) -> usize {
    match v {
        Value::Null => 0,
        Value::Bool(b) => usize::from(*b),
        Value::Number(n) => format_number(n).chars().count(),
        Value::String(s) => s.chars().count(),
        Value::Array(a) => a.len(),
        Value::Object(o) => o.len(),
    }
}

/// The numeric values of an array, nulls and non-numbers skipped.
pub fn numbers_of(arr: &[Value]) -> Vec<f64> {
    arr.iter().filter_map(Value::as_f64).collect()
}

/// Parse an interval such as `"5m"`, `"250ms"`, `"1h"`, `"7d"` or `"2w"`
/// into milliseconds. Rejects zero, negative and non-ASCII input (the unit
/// is split on characters, never on bytes).
pub fn parse_interval_ms(interval: &str) -> SdbqlResult<i64> {
    let s = interval.trim();
    let split = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| i)
        .ok_or_else(|| err(format!("interval '{}': missing unit", interval)))?;
    let (num, unit) = s.split_at(split);
    let val: i64 = num
        .parse()
        .map_err(|_| err(format!("interval '{}': invalid number", interval)))?;
    let factor: i64 = match unit {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 7 * 86_400_000,
        _ => {
            return Err(err(format!(
                "interval '{}': valid units are ms, s, m, h, d, w",
                interval
            )))
        }
    };
    let ms = val
        .checked_mul(factor)
        .ok_or_else(|| err(format!("interval '{}' is too large", interval)))?;
    if ms <= 0 {
        return Err(err(format!("interval '{}' must be positive", interval)));
    }
    Ok(ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stringify_follows_aql() {
        assert_eq!(stringify(&Value::Null), "");
        assert_eq!(stringify(&json!(2.0)), "2");
        assert_eq!(stringify(&json!(2.5)), "2.5");
        assert_eq!(stringify(&json!([1, "a"])), r#"[1,"a"]"#);
    }

    #[test]
    fn to_number_follows_aql() {
        assert_eq!(to_number(&Value::Null), json!(0));
        assert_eq!(to_number(&json!("abc")), json!(0));
        assert_eq!(to_number(&json!(" 12 ")), json!(12));
        assert_eq!(to_number(&json!("1.5")), json!(1.5));
        assert_eq!(to_number(&json!("inf")), json!(0));
        assert_eq!(to_number(&json!(["7"])), json!(7));
        assert_eq!(to_number(&json!([1, 2])), json!(0));
        assert_eq!(to_number(&json!(true)), json!(1));
    }

    #[test]
    fn interval_parsing_never_panics() {
        assert_eq!(parse_interval_ms("5m").unwrap(), 300_000);
        assert_eq!(parse_interval_ms("250ms").unwrap(), 250);
        assert!(parse_interval_ms("é").is_err());
        assert!(parse_interval_ms("5µ").is_err());
        assert!(parse_interval_ms("0s").is_err());
        assert!(parse_interval_ms("-5s").is_err());
        assert!(parse_interval_ms("99999999999999999d").is_err());
        assert!(parse_interval_ms("").is_err());
    }

    #[test]
    fn as_int_accepts_integral_floats() {
        assert_eq!(as_int(&json!(2.0)), Some(2));
        assert_eq!(as_int(&json!(1e300)), Some(i64::MAX));
        assert_eq!(as_int(&json!("2")), None);
    }
}
