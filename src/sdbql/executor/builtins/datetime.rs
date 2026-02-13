//! Date and time functions for SDBQL.
//!
//! NOW, DATE_*, TIME_BUCKET, UUIDV4, UUIDV7, etc.

use crate::error::{DbError, DbResult};
use chrono::{Datelike, Timelike, Utc};
use serde_json::Value;
use uuid::Uuid;

/// Evaluate datetime functions
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "NOW" | "DATE_NOW" => {
            let now = Utc::now();
            Ok(Some(Value::Number(serde_json::Number::from(
                now.timestamp_millis(),
            ))))
        }
        "NOW_ISO" | "DATE_NOW_ISO" => Ok(Some(Value::String(Utc::now().to_rfc3339()))),
        "UUIDV4" => Ok(Some(Value::String(Uuid::new_v4().to_string()))),
        "UUIDV7" => {
            let ts = uuid::Timestamp::now(uuid::NoContext);
            Ok(Some(Value::String(Uuid::new_v7(ts).to_string())))
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
            Ok(Some(Value::Number(serde_json::Number::from(
                dt.iso_week().week(),
            ))))
        }
        "DATE_ISO8601" | "DATE_FORMAT" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::String(dt.to_rfc3339())))
        }
        "DATE_TIMESTAMP" => {
            check_args(name, args, 1)?;
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(
                dt.timestamp_millis(),
            ))))
        }
        "DATE_ADD" => {
            if args.len() < 3 {
                return Err(DbError::ExecutionError(
                    "DATE_ADD requires 3 arguments: date, amount, unit".to_string(),
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
                _ => {
                    return Err(DbError::ExecutionError(format!(
                        "DATE_ADD: unknown unit '{}', use y/m/w/d/h/i/s",
                        unit
                    )))
                }
            };
            Ok(Some(Value::String(new_dt.to_rfc3339())))
        }
        "DATE_SUBTRACT" | "DATE_SUB" => {
            if args.len() < 3 {
                return Err(DbError::ExecutionError(
                    "DATE_SUBTRACT requires 3 arguments: date, amount, unit".to_string(),
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
                _ => {
                    return Err(DbError::ExecutionError(format!(
                        "DATE_SUBTRACT: unknown unit '{}', use y/m/w/d/h/i/s",
                        unit
                    )))
                }
            };
            Ok(Some(Value::String(new_dt.to_rfc3339())))
        }
        "DATE_DIFF" => {
            if args.len() < 2 {
                return Err(DbError::ExecutionError(
                    "DATE_DIFF requires 2-3 arguments: date1, date2, [unit]".to_string(),
                ));
            }
            let dt1 = parse_datetime(&args[0])?;
            let dt2 = parse_datetime(&args[1])?;
            let unit = args
                .get(2)
                .and_then(|v| v.as_str())
                .unwrap_or("d")
                .to_lowercase();

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
                _ => {
                    return Err(DbError::ExecutionError(format!(
                        "DATE_DIFF: unknown unit '{}', use y/m/w/d/h/i/s/ms",
                        unit
                    )))
                }
            };
            Ok(Some(Value::Number(serde_json::Number::from(result))))
        }
        "TIME_BUCKET" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    " 2 arguments:TIME_BUCKET requires timestamp, interval (e.g. '5m')".to_string(),
                ));
            }
            let interval_str = args[1].as_str().ok_or_else(|| {
                DbError::ExecutionError("TIME_BUCKET: interval must be a string".to_string())
            })?;

            let len = interval_str.len();
            if len < 2 {
                return Err(DbError::ExecutionError(
                    "TIME_BUCKET: invalid interval format".to_string(),
                ));
            }

            let unit = &interval_str[len - 1..];
            let val_str = &interval_str[..len - 1];
            let val: u64 = val_str.parse().map_err(|_| {
                DbError::ExecutionError("TIME_BUCKET: invalid interval number".to_string())
            })?;

            let interval_ms = match unit {
                "s" => val * 1000,
                "m" => val * 1000 * 60,
                "h" => val * 1000 * 60 * 60,
                "d" => val * 1000 * 60 * 60 * 24,
                _ => {
                    return Err(DbError::ExecutionError(
                        "TIME_BUCKET: valid units are s, m, h, d".to_string(),
                    ))
                }
            };

            if interval_ms == 0 {
                return Err(DbError::ExecutionError(
                    "TIME_BUCKET: interval cannot be 0".to_string(),
                ));
            }

            match &args[0] {
                Value::Number(n) => {
                    let ts = n.as_i64().ok_or_else(|| {
                        DbError::ExecutionError(
                            "TIME_BUCKET: timestamp must be a valid number".to_string(),
                        )
                    })?;
                    let bucket = ts.div_euclid(interval_ms as i64) * (interval_ms as i64);
                    Ok(Some(Value::Number(bucket.into())))
                }
                Value::String(s) => {
                    let dt = chrono::DateTime::parse_from_rfc3339(s).map_err(|_| {
                        DbError::ExecutionError("TIME_BUCKET: invalid timestamp string".to_string())
                    })?;
                    let ts = dt.timestamp_millis();
                    let bucket_ts = ts.div_euclid(interval_ms as i64) * (interval_ms as i64);

                    let seconds = bucket_ts.div_euclid(1000);
                    let nanos = (bucket_ts.rem_euclid(1000) * 1_000_000) as u32;

                    if let Some(dt) = chrono::DateTime::from_timestamp(seconds, nanos) {
                        Ok(Some(Value::String(dt.to_rfc3339())))
                    } else {
                        Err(DbError::ExecutionError(
                            "TIME_BUCKET: failed to construct date".to_string(),
                        ))
                    }
                }
                _ => Err(DbError::ExecutionError(
                    "TIME_BUCKET: timestamp must be number or string".to_string(),
                )),
            }
        }
        "HUMAN_TIME" => {
            if args.is_empty() {
                return Err(DbError::ExecutionError(
                    "HUMAN_TIME requires at least 1 argument".to_string(),
                ));
            }

            let date_value = &args[0];
            let now = args
                .get(1)
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| Utc::now().timestamp_millis());

            let date_ts = match date_value {
                Value::Number(n) => n.as_i64().ok_or_else(|| {
                    DbError::ExecutionError("HUMAN_TIME: invalid timestamp".to_string())
                })?,
                Value::String(_s) => {
                    let dt = parse_datetime(date_value)?;
                    dt.timestamp_millis()
                }
                _ => {
                    return Err(DbError::ExecutionError(
                        "HUMAN_TIME: first argument must be a timestamp or date string".to_string(),
                    ));
                }
            };

            let diff_secs = (now - date_ts) / 1000;

            let result = if diff_secs.abs() < 60 {
                "just now".to_string()
            } else if diff_secs < 0 {
                "in the future".to_string()
            } else if diff_secs < 3600 {
                let mins = diff_secs / 60;
                format!("{} minute{} ago", mins, if mins == 1 { "" } else { "s" })
            } else if diff_secs < 86400 {
                let hours = diff_secs / 3600;
                format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
            } else if diff_secs < 2592000 {
                let days = diff_secs / 86400;
                format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
            } else if diff_secs < 31536000 {
                let months = diff_secs / 2592000;
                format!("{} month{} ago", months, if months == 1 { "" } else { "s" })
            } else {
                let years = diff_secs / 31536000;
                format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
            };

            Ok(Some(Value::String(result)))
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
                .map_err(|_| DbError::ExecutionError(format!("Cannot parse date string: {}", s)))
        }
        Value::Number(n) => {
            // Assume milliseconds timestamp
            let ms = n
                .as_i64()
                .ok_or_else(|| DbError::ExecutionError("Invalid timestamp number".to_string()))?;
            chrono::DateTime::from_timestamp_millis(ms)
                .ok_or_else(|| DbError::ExecutionError("Invalid timestamp".to_string()))
        }
        _ => Err(DbError::ExecutionError(
            "Date must be a string or number".to_string(),
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
