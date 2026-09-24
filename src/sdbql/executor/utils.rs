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

/// Numbers below this magnitude are read as **seconds** since the epoch by the
/// date functions (`DATE_*`, `HUMAN_TIME`); at or above it, as milliseconds.
/// 1e10 s is the year 2286, and 1e10 ms is 1970-04-26, so the two ranges only
/// collide for millisecond timestamps in the first four months of 1970.
/// Time-series functions (`TIME_BUCKET`, `RATE`, `RESAMPLE`, …) do not apply
/// it: a number there is always milliseconds.
pub const SECONDS_EPOCH_THRESHOLD: f64 = 10_000_000_000.0;

/// A date string, before any timezone has been applied to it.
#[derive(Debug, Clone, Copy)]
pub enum ParsedDate {
    /// Carried an explicit offset (`Z`, `+02:00`, `+0200`).
    Offset(chrono::DateTime<chrono::FixedOffset>),
    /// No offset: a wall-clock time. The date functions read it as UTC;
    /// `DATE_LOCALTOUTC` reads it in the zone it is given.
    Naive(chrono::NaiveDateTime),
}

impl ParsedDate {
    pub fn to_utc(self) -> chrono::DateTime<Utc> {
        match self {
            ParsedDate::Offset(dt) => dt.with_timezone(&Utc),
            ParsedDate::Naive(n) => n.and_utc(),
        }
    }

    /// The wall-clock fields as written, whatever the offset.
    pub fn naive_local(self) -> chrono::NaiveDateTime {
        match self {
            ParsedDate::Offset(dt) => dt.naive_local(),
            ParsedDate::Naive(n) => n,
        }
    }
}

/// The single ISO-8601 string parser behind every date function.
///
/// Accepts, with `T` or a space between date and time: `YYYY-MM-DD`,
/// `YYYY-MM-DDTHH:MM`, `YYYY-MM-DDTHH:MM:SS`, any fraction of a second, and
/// an optional offset `Z`, `±HH:MM`, `±HHMM` or `±HH`. Surrounding
/// whitespace is ignored.
pub fn parse_date_str(s: &str) -> Option<ParsedDate> {
    use chrono::{DateTime, NaiveDate, NaiveDateTime};

    let s = s.trim();
    if s.len() < 10 || !s.is_char_boundary(10) {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(ParsedDate::Offset(dt));
    }
    if s.len() == 10 {
        return NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(ParsedDate::Naive);
    }
    let (date, rest) = s.split_at(10);
    let rest = rest
        .strip_prefix('T')
        .or_else(|| rest.strip_prefix('t'))
        .or_else(|| rest.strip_prefix(' '))?;
    // Split the time from a trailing offset. The time part only holds digits,
    // ':' and '.', so the first other character starts the offset.
    let off_at = rest
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_digit() || *c == ':' || *c == '.'))
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    let (time, offset) = rest.split_at(off_at);
    let naive_str = format!("{date}T{time}");
    let naive = NaiveDateTime::parse_from_str(&naive_str, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(&naive_str, "%Y-%m-%dT%H:%M"))
        .ok()?;
    let offset = offset.trim();
    if offset.is_empty() {
        return Some(ParsedDate::Naive(naive));
    }
    if offset.eq_ignore_ascii_case("z") {
        return Some(ParsedDate::Offset(naive.and_utc().fixed_offset()));
    }
    let secs = parse_offset_secs(offset)?;
    let fixed = chrono::FixedOffset::east_opt(secs)?;
    naive
        .and_local_timezone(fixed)
        .single()
        .map(ParsedDate::Offset)
}

/// `+02:00`, `-0530`, `+02` → seconds east of UTC.
fn parse_offset_secs(s: &str) -> Option<i32> {
    let mut chars = s.chars();
    let sign = match chars.next()? {
        '+' => 1,
        '-' | '\u{2212}' => -1,
        _ => return None,
    };
    let digits: String = chars.filter(|c| *c != ':').collect();
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m) = match digits.len() {
        2 => (digits.parse::<i32>().ok()?, 0),
        4 => (
            digits[..2].parse::<i32>().ok()?,
            digits[2..].parse::<i32>().ok()?,
        ),
        _ => return None,
    };
    if h > 23 || m > 59 {
        return None;
    }
    Some(sign * (h * 3600 + m * 60))
}

/// A numeric date → milliseconds since the epoch, applying
/// [`SECONDS_EPOCH_THRESHOLD`]. Fractional seconds keep their milliseconds.
pub fn number_to_epoch_ms(n: &serde_json::Number) -> DbResult<i64> {
    if let Some(i) = n.as_i64() {
        return Ok(if (i as f64).abs() < SECONDS_EPOCH_THRESHOLD {
            i.saturating_mul(1000)
        } else {
            i
        });
    }
    let f = n
        .as_f64()
        .filter(|f| f.is_finite())
        .ok_or_else(|| DbError::ExecutionError("Invalid timestamp".to_string()))?;
    let ms = if f.abs() < SECONDS_EPOCH_THRESHOLD {
        f * 1000.0
    } else {
        f
    };
    if ms.abs() >= 9.2e18 {
        return Err(DbError::ExecutionError(format!("Invalid timestamp: {}", f)));
    }
    Ok(ms.floor() as i64)
}

/// Parse a date value into `DateTime<Utc>`.
///
/// Numbers are milliseconds since epoch, unless `|n| < 10_000_000_000`
/// (then seconds — so `DATE_YEAR(1609459200)` is 2021); see
/// [`SECONDS_EPOCH_THRESHOLD`]. Strings go through [`parse_date_str`]; one
/// without an offset is read as UTC.
pub fn parse_datetime(value: &Value) -> DbResult<chrono::DateTime<Utc>> {
    match value {
        Value::Number(n) => {
            let timestamp_ms = number_to_epoch_ms(n)?;
            chrono::DateTime::from_timestamp_millis(timestamp_ms).ok_or_else(|| {
                DbError::ExecutionError(format!("Invalid timestamp: {}", timestamp_ms))
            })
        }
        Value::String(s) => parse_date_str(s)
            .map(ParsedDate::to_utc)
            .ok_or_else(|| DbError::ExecutionError(format!("Invalid date string '{}'", s))),
        _ => Err(DbError::ExecutionError(
            "Date must be a timestamp or date string".to_string(),
        )),
    }
}

/// Like [`parse_datetime`] but keeps a string's wall-clock fields apart from
/// its offset; a number becomes the UTC wall clock of that instant.
pub fn parse_date_value(value: &Value) -> DbResult<ParsedDate> {
    match value {
        Value::String(s) => parse_date_str(s)
            .ok_or_else(|| DbError::ExecutionError(format!("Invalid date string '{}'", s))),
        other => Ok(ParsedDate::Naive(parse_datetime(other)?.naive_utc())),
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
