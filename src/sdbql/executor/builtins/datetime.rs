//! Date and time functions for SDBQL.
//!
//! NOW, DATE_*, TIME_BUCKET, etc.

use serde_json::Value;
use chrono::{Datelike, Timelike, Utc};
use crate::error::{DbError, DbResult};

/// Evaluate datetime functions
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "NOW" | "DATE_NOW" => {
            let now = Utc::now();
            Ok(Some(Value::Number(serde_json::Number::from(now.timestamp_millis()))))
        }
        "NOW_ISO" | "DATE_NOW_ISO" => {
            Ok(Some(Value::String(Utc::now().to_rfc3339())))
        }
        "DATE_YEAR" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.year()))))
        }
        "DATE_MONTH" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.month()))))
        }
        "DATE_DAY" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.day()))))
        }
        "DATE_HOUR" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.hour()))))
        }
        "DATE_MINUTE" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.minute()))))
        }
        "DATE_SECOND" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.second()))))
        }
        "DATE_DAYOFWEEK" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            // 0 = Sunday, 6 = Saturday
            let dow = dt.weekday().num_days_from_sunday();
            Ok(Some(Value::Number(serde_json::Number::from(dow))))
        }
        "DATE_DAYOFYEAR" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.ordinal()))))
        }
        "DATE_WEEK" | "DATE_ISOWEEK" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.iso_week().week()))))
        }
        "DATE_ISO8601" | "DATE_FORMAT" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::String(dt.to_rfc3339())))
        }
        "DATE_TIMESTAMP" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.timestamp_millis()))))
        }
        "DATE_ADD" => {
            if args.len() < 3 {
                return Err(DbError::ExecutionError(
                    "DATE_ADD requires 3 arguments: date, amount, unit".to_string()
                ));
            }
            let dt = parse_datetime(&args[0])?;
            let amount = args[1].as_i64().ok_or_else(|| {
                DbError::ExecutionError("DATE_ADD: amount must be an integer".to_string())
            })?;
            let unit = args[2].as_str().unwrap_or("d").to_lowercase();
            
            let new_dt = match unit.as_str() {
                "y" | "year" | "years" => dt + chrono::Duration::days(amount * 365),
                "m" | "month" | "months" => dt + chrono::Duration::days(amount * 30),
                "w" | "week" | "weeks" => dt + chrono::Duration::weeks(amount),
                "d" | "day" | "days" => dt + chrono::Duration::days(amount),
                "h" | "hour" | "hours" => dt + chrono::Duration::hours(amount),
                "i" | "minute" | "minutes" => dt + chrono::Duration::minutes(amount),
                "s" | "second" | "seconds" => dt + chrono::Duration::seconds(amount),
                _ => return Err(DbError::ExecutionError(format!(
                    "DATE_ADD: unknown unit '{}', use y/m/w/d/h/i/s", unit
                ))),
            };
            Ok(Some(Value::String(new_dt.to_rfc3339())))
        }
        "DATE_SUBTRACT" | "DATE_SUB" => {
            if args.len() < 3 {
                return Err(DbError::ExecutionError(
                    "DATE_SUBTRACT requires 3 arguments: date, amount, unit".to_string()
                ));
            }
            let dt = parse_datetime(&args[0])?;
            let amount = args[1].as_i64().ok_or_else(|| {
                DbError::ExecutionError("DATE_SUBTRACT: amount must be an integer".to_string())
            })?;
            let unit = args[2].as_str().unwrap_or("d").to_lowercase();
            
            let new_dt = match unit.as_str() {
                "y" | "year" | "years" => dt - chrono::Duration::days(amount * 365),
                "m" | "month" | "months" => dt - chrono::Duration::days(amount * 30),
                "w" | "week" | "weeks" => dt - chrono::Duration::weeks(amount),
                "d" | "day" | "days" => dt - chrono::Duration::days(amount),
                "h" | "hour" | "hours" => dt - chrono::Duration::hours(amount),
                "i" | "minute" | "minutes" => dt - chrono::Duration::minutes(amount),
                "s" | "second" | "seconds" => dt - chrono::Duration::seconds(amount),
                _ => return Err(DbError::ExecutionError(format!(
                    "DATE_SUBTRACT: unknown unit '{}', use y/m/w/d/h/i/s", unit
                ))),
            };
            Ok(Some(Value::String(new_dt.to_rfc3339())))
        }
        "DATE_DIFF" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "DATE_DIFF requires 2-3 arguments: date1, date2, [unit]".to_string()
                ));
            }
            let dt1 = parse_datetime(&args[0])?;
            let dt2 = parse_datetime(&args[1])?;
            let unit = args.get(2).and_then(|v| v.as_str()).unwrap_or("d").to_lowercase();
            
            let diff = dt1.signed_duration_since(dt2);
            let result = match unit.as_str() {
                "y" | "year" | "years" => diff.num_days() / 365,
                "m" | "month" | "months" => diff.num_days() / 30,
                "w" | "week" | "weeks" => diff.num_weeks(),
                "d" | "day" | "days" => diff.num_days(),
                "h" | "hour" | "hours" => diff.num_hours(),
                "i" | "minute" | "minutes" => diff.num_minutes(),
                "s" | "second" | "seconds" => diff.num_seconds(),
                "ms" | "millisecond" | "milliseconds" => diff.num_milliseconds(),
                _ => return Err(DbError::ExecutionError(format!(
                    "DATE_DIFF: unknown unit '{}', use y/m/w/d/h/i/s/ms", unit
                ))),
            };
            Ok(Some(Value::Number(serde_json::Number::from(result))))
        }
        _ => Ok(None),
    }
}

fn parse_datetime(v: &Value) -> DbResult<chrono::DateTime<Utc>> {
    match v {
        Value::String(s) => {
            // Try RFC3339 first
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .or_else(|_| {
                    // Try common formats
                    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                        .map(|dt| dt.and_utc())
                })
                .or_else(|_| {
                    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                        .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc())
                })
                .map_err(|_| DbError::ExecutionError(
                    format!("Cannot parse date string: {}", s)
                ))
        }
        Value::Number(n) => {
            // Assume milliseconds timestamp
            let ms = n.as_i64().ok_or_else(|| {
                DbError::ExecutionError("Invalid timestamp number".to_string())
            })?;
            chrono::DateTime::from_timestamp_millis(ms)
                .ok_or_else(|| DbError::ExecutionError("Invalid timestamp".to_string()))
        }
        _ => Err(DbError::ExecutionError(
            "Date must be a string or number".to_string()
        )),
    }
}

fn check_args(name: &str, args: &[Value], expected: usize) -> DbResult<()> {
    if args.len() != expected {
        return Err(DbError::ExecutionError(format!(
            "{} requires {} argument(s)",
            name, expected
        )));
    }
    Ok(())
}
