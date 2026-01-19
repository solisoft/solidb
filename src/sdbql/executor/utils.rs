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

/// Parse a date value (timestamp or ISO string) into DateTime<Utc>
pub fn parse_datetime(value: &Value) -> DbResult<chrono::DateTime<Utc>> {
    use chrono::{DateTime, TimeZone};

    match value {
        Value::Number(n) => {
            let timestamp_ms = if let Some(i) = n.as_i64() {
                i
            } else if let Some(f) = n.as_f64() {
                f as i64
            } else {
                return Err(DbError::ExecutionError("Invalid timestamp".to_string()));
            };
            let secs = timestamp_ms / 1000;
            let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
            match Utc.timestamp_opt(secs, nanos) {
                chrono::LocalResult::Single(dt) => Ok(dt),
                _ => Err(DbError::ExecutionError(format!(
                    "Invalid timestamp: {}",
                    timestamp_ms
                ))),
            }
        }
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| DbError::ExecutionError(format!("Invalid ISO 8601 date '{}': {}", s, e))),
        _ => Err(DbError::ExecutionError(
            "Date must be a timestamp or ISO 8601 string".to_string(),
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
