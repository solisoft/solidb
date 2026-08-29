use chrono::Utc;
use serde_json::Value;

use super::super::ast::*;
use crate::error::{DbError, DbResult};

/// Maximum allowed regex pattern length to prevent DoS attacks
const MAX_REGEX_PATTERN_LEN: usize = 1024;

/// Maximum regex compiled size (1MB) to prevent memory exhaustion
const MAX_REGEX_SIZE: usize = 1 << 20;

/// Create a regex with safety limits to prevent ReDoS attacks.
/// While the Rust regex crate is inherently ReDoS-resistant (uses Thompson NFA),
/// we still limit pattern size and compiled size to prevent memory exhaustion.
pub fn safe_regex(pattern: &str) -> Result<regex::Regex, DbError> {
    if pattern.len() > MAX_REGEX_PATTERN_LEN {
        return Err(DbError::ExecutionError(format!(
            "Regex pattern too long: {} bytes (max {})",
            pattern.len(),
            MAX_REGEX_PATTERN_LEN
        )));
    }

    regex::RegexBuilder::new(pattern)
        .size_limit(MAX_REGEX_SIZE)
        .build()
        .map_err(|e| DbError::ExecutionError(format!("Invalid regex pattern: {}", e)))
}

/// Convert f64 to serde_json::Number, returning 0 for NaN/Infinity instead of panicking
pub fn number_from_f64(f: f64) -> serde_json::Number {
    serde_json::Number::from_f64(f).unwrap_or_else(|| serde_json::Number::from(0))
}

/// Parse a date value into `DateTime<Utc>`.
///
/// Numbers are milliseconds since epoch, unless `|n| < 10_000_000_000`
/// (then seconds — so `DATE_YEAR(1609459200)` is 2021). Strings accept
/// RFC3339, `YYYY-MM-DD`, and `YYYY-MM-DD HH:MM:SS`.
pub fn parse_datetime(value: &Value) -> DbResult<chrono::DateTime<Utc>> {
    use chrono::{DateTime, NaiveDate, NaiveDateTime};

    match value {
        Value::Number(n) => {
            let raw = n
                .as_i64()
                .or_else(|| n.as_f64().map(|f| f as i64))
                .ok_or_else(|| DbError::ExecutionError("Invalid timestamp".to_string()))?;
            let timestamp_ms = if raw.abs() < 10_000_000_000 {
                raw.saturating_mul(1000)
            } else {
                raw
            };
            DateTime::from_timestamp_millis(timestamp_ms).ok_or_else(|| {
                DbError::ExecutionError(format!("Invalid timestamp: {}", timestamp_ms))
            })
        }
        Value::String(s) => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Ok(dt.with_timezone(&Utc));
            }
            if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                return Ok(d
                    .and_hms_opt(0, 0, 0)
                    .ok_or_else(|| DbError::ExecutionError("Invalid date".to_string()))?
                    .and_utc());
            }
            if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
                return Ok(dt.and_utc());
            }
            if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
                return Ok(dt.and_utc());
            }
            if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
                return Ok(dt.and_utc());
            }
            Err(DbError::ExecutionError(format!(
                "Invalid date string '{}'",
                s
            )))
        }
        _ => Err(DbError::ExecutionError(
            "Date must be a timestamp or date string".to_string(),
        )),
    }
}

/// Format an Expression as a human-readable string
pub fn format_expression(expr: &Expression) -> String {
    match expr {
        Expression::Variable(name) => name.clone(),
        Expression::BindVariable(name) => format!("@{}", name),
        Expression::FieldAccess(base, field) => {
            format!("{}.{}", format_expression(base), field)
        }
        Expression::OptionalFieldAccess(base, field) => {
            format!("{}?.{}", format_expression(base), field)
        }
        Expression::DynamicFieldAccess(base, field_expr) => {
            format!(
                "{}[{}]",
                format_expression(base),
                format_expression(field_expr)
            )
        }
        Expression::ArrayAccess(base, index) => {
            format!("{}[{}]", format_expression(base), format_expression(index))
        }
        Expression::ArraySpreadAccess(base, field_path) => {
            let base_str = format_expression(base);
            match field_path {
                Some(path) => format!("{}[*].{}", base_str, path),
                None => format!("{}[*]", base_str),
            }
        }
        Expression::Literal(value) => format!("{}", value),
        Expression::FunctionCall { name, args } => {
            let args_str = args
                .iter()
                .map(format_expression)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({})", name, args_str)
        }
        Expression::Pipeline { left, right } => {
            format!(
                "{} |> {}",
                format_expression(left),
                format_expression(right)
            )
        }
        Expression::Lambda { params, body } => {
            if params.len() == 1 {
                format!("{} -> {}", params[0], format_expression(body))
            } else {
                format!("({}) -> {}", params.join(", "), format_expression(body))
            }
        }
        _ => format!("{:?}", expr), // Fallback to debug for complex expressions
    }
}
