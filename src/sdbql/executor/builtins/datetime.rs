//! Date and time functions for SDBQL (AQL-shaped).
//!
//! Single implementation — `phonetic/date.rs` no longer shadows these.

use crate::error::{DbError, DbResult};
use crate::sdbql::executor::utils::{number_from_f64, parse_datetime};
use chrono::{Datelike, Duration, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Copy)]
enum Unit {
    Year,
    Month,
    Week,
    Day,
    Hour,
    Minute,
    Second,
    Millisecond,
}

fn parse_unit(s: &str) -> Option<Unit> {
    if s.eq_ignore_ascii_case("y")
        || s.eq_ignore_ascii_case("year")
        || s.eq_ignore_ascii_case("years")
    {
        Some(Unit::Year)
    } else if s.eq_ignore_ascii_case("m")
        || s.eq_ignore_ascii_case("month")
        || s.eq_ignore_ascii_case("months")
    {
        Some(Unit::Month)
    } else if s.eq_ignore_ascii_case("w")
        || s.eq_ignore_ascii_case("week")
        || s.eq_ignore_ascii_case("weeks")
    {
        Some(Unit::Week)
    } else if s.eq_ignore_ascii_case("d")
        || s.eq_ignore_ascii_case("day")
        || s.eq_ignore_ascii_case("days")
    {
        Some(Unit::Day)
    } else if s.eq_ignore_ascii_case("h")
        || s.eq_ignore_ascii_case("hour")
        || s.eq_ignore_ascii_case("hours")
    {
        Some(Unit::Hour)
    } else if s.eq_ignore_ascii_case("i")
        || s.eq_ignore_ascii_case("minute")
        || s.eq_ignore_ascii_case("minutes")
    {
        Some(Unit::Minute)
    } else if s.eq_ignore_ascii_case("s")
        || s.eq_ignore_ascii_case("second")
        || s.eq_ignore_ascii_case("seconds")
    {
        Some(Unit::Second)
    } else if s.eq_ignore_ascii_case("f")
        || s.eq_ignore_ascii_case("ms")
        || s.eq_ignore_ascii_case("millisecond")
        || s.eq_ignore_ascii_case("milliseconds")
    {
        Some(Unit::Millisecond)
    } else {
        None
    }
}

fn parse_tz(s: &str) -> DbResult<Tz> {
    s.parse()
        .map_err(|_| DbError::ExecutionError(format!("unknown timezone '{}'", s)))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let this = NaiveDate::from_ymd_opt(year, month, 1);
    let next = NaiveDate::from_ymd_opt(ny, nm, 1);
    match (this, next) {
        (Some(a), Some(b)) => (b - a).num_days() as u32,
        _ => 30,
    }
}

fn add_calendar(
    dt: chrono::DateTime<Utc>,
    amount: i64,
    unit: Unit,
    tz: Tz,
) -> DbResult<chrono::DateTime<Utc>> {
    let local = dt.with_timezone(&tz);
    let result = match unit {
        Unit::Year | Unit::Month => {
            let months = match unit {
                Unit::Year => amount.saturating_mul(12),
                _ => amount,
            };
            let total = i64::from(local.year()) * 12 + i64::from(local.month()) - 1 + months;
            let new_year = (total.div_euclid(12)) as i32;
            let new_month = (total.rem_euclid(12) + 1) as u32;
            let new_day = local.day().min(days_in_month(new_year, new_month));
            let date = NaiveDate::from_ymd_opt(new_year, new_month, new_day).ok_or_else(|| {
                DbError::ExecutionError("DATE_ADD: invalid calendar date".to_string())
            })?;
            let time = local.time();
            tz.from_local_datetime(&date.and_time(time))
                .single()
                .ok_or_else(|| {
                    DbError::ExecutionError("DATE_ADD: invalid local datetime".to_string())
                })?
        }
        Unit::Week => local + Duration::weeks(amount),
        Unit::Day => local + Duration::days(amount),
        Unit::Hour => local + Duration::hours(amount),
        Unit::Minute => local + Duration::minutes(amount),
        Unit::Second => local + Duration::seconds(amount),
        Unit::Millisecond => local + Duration::milliseconds(amount),
    };
    Ok(result.with_timezone(&Utc))
}

fn rfc3339_ms(dt: chrono::DateTime<Utc>) -> Value {
    Value::String(dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn null_arg(args: &[Value]) -> bool {
    args.iter().any(Value::is_null)
}

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "NOW" | "DATE_NOW" => Ok(Some(Value::Number(serde_json::Number::from(
            Utc::now().timestamp_millis(),
        )))),
        "NOW_ISO" | "DATE_NOW_ISO" => Ok(Some(Value::String(Utc::now().to_rfc3339()))),
        "UUIDV4" => Ok(Some(Value::String(Uuid::new_v4().to_string()))),
        "UUIDV7" => {
            let ts = uuid::Timestamp::now(uuid::NoContext);
            Ok(Some(Value::String(Uuid::new_v7(ts).to_string())))
        }
        "DATE_YEAR" => extract(args, |dt| i64::from(dt.year())),
        "DATE_MONTH" => extract(args, |dt| i64::from(dt.month())),
        "DATE_DAY" => extract(args, |dt| i64::from(dt.day())),
        "DATE_HOUR" => extract(args, |dt| i64::from(dt.hour())),
        "DATE_MINUTE" => extract(args, |dt| i64::from(dt.minute())),
        "DATE_SECOND" => extract(args, |dt| i64::from(dt.second())),
        "DATE_MILLISECOND" => extract(args, |dt| i64::from(dt.timestamp_subsec_millis())),
        "DATE_DAYOFWEEK" => extract(args, |dt| i64::from(dt.weekday().num_days_from_sunday())),
        "DATE_DAYOFYEAR" => {
            if args.is_empty() || args.len() > 2 {
                return Err(arity("DATE_DAYOFYEAR", "1-2"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let dt = parse_datetime(&args[0])?;
            let n = if let Some(tz) = args.get(1).and_then(Value::as_str) {
                dt.with_timezone(&parse_tz(tz)?).ordinal()
            } else {
                dt.ordinal()
            };
            Ok(Some(Value::Number(serde_json::Number::from(n))))
        }
        "DATE_WEEK" | "DATE_ISOWEEK" => extract(args, |dt| i64::from(dt.iso_week().week())),
        "DATE_ISOWEEKYEAR" => extract(args, |dt| i64::from(dt.iso_week().year())),
        "DATE_QUARTER" => extract(args, |dt| i64::from((dt.month() - 1) / 3 + 1)),
        "DATE_LEAPYEAR" => {
            if args.len() != 1 {
                return Err(arity(name, "1"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let y = parse_datetime(&args[0])?.year();
            Ok(Some(Value::Bool(
                NaiveDate::from_ymd_opt(y, 2, 29).is_some(),
            )))
        }
        "DATE_COMPARE" => {
            if args.len() != 2 {
                return Err(arity(name, "2"));
            }
            if null_arg(args) {
                return Ok(Some(Value::Null));
            }
            let a = parse_datetime(&args[0])?;
            let b = parse_datetime(&args[1])?;
            let cmp = a.cmp(&b);
            Ok(Some(Value::Number(serde_json::Number::from(
                cmp as i8 as i64,
            ))))
        }
        "DATE_ISO8601" => {
            if args.len() != 1 {
                return Err(arity(name, "1"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            Ok(Some(rfc3339_ms(parse_datetime(&args[0])?)))
        }
        "DATE_TIMESTAMP" => {
            if args.len() != 1 {
                return Err(arity(name, "1"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            Ok(Some(Value::Number(serde_json::Number::from(
                parse_datetime(&args[0])?.timestamp_millis(),
            ))))
        }
        "DATE_FORMAT" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(arity(name, "2-3: date, format, [timezone]"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let dt = parse_datetime(&args[0])?;
            let fmt = args[1].as_str().ok_or_else(|| {
                DbError::ExecutionError("DATE_FORMAT: format must be a string".to_string())
            })?;
            let tz = if let Some(s) = args.get(2).and_then(Value::as_str) {
                parse_tz(s)?
            } else {
                chrono_tz::UTC
            };
            Ok(Some(Value::String(
                dt.with_timezone(&tz).format(fmt).to_string(),
            )))
        }
        "DATE_TRUNC" | "DATE_ROUND" => date_trunc(args),
        "DATE_DAYS_IN_MONTH" => {
            if args.is_empty() || args.len() > 2 {
                return Err(arity(name, "1-2"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let dt = parse_datetime(&args[0])?;
            let (y, m) = if let Some(s) = args.get(1).and_then(Value::as_str) {
                let local = dt.with_timezone(&parse_tz(s)?);
                (local.year(), local.month())
            } else {
                (dt.year(), dt.month())
            };
            Ok(Some(Value::Number(serde_json::Number::from(
                days_in_month(y, m),
            ))))
        }
        "DATE_ADD" => date_add_args(args, 1),
        "DATE_SUBTRACT" | "DATE_SUB" => date_add_args(args, -1),
        "DATE_DIFF" => date_diff(args),
        "TIME_BUCKET" => time_bucket(args),
        "HUMAN_TIME" => human_time(args),
        _ => Ok(None),
    }
}

fn extract(
    args: &[Value],
    f: impl FnOnce(chrono::DateTime<Utc>) -> i64,
) -> DbResult<Option<Value>> {
    if args.len() != 1 {
        return Err(arity("DATE_*", "1"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    Ok(Some(Value::Number(serde_json::Number::from(f(
        parse_datetime(&args[0])?,
    )))))
}

fn date_add_args(args: &[Value], sign: i64) -> DbResult<Option<Value>> {
    if args.len() < 3 || args.len() > 4 {
        return Err(arity("DATE_ADD", "3-4: date, amount, unit, [timezone]"));
    }
    if args[0].is_null() || args[1].is_null() {
        return Ok(Some(Value::Null));
    }
    let dt = parse_datetime(&args[0])?;
    let amount = args[1]
        .as_i64()
        .or_else(|| args[1].as_f64().map(|f| f as i64))
        .ok_or_else(|| DbError::ExecutionError("DATE_ADD: amount must be a number".to_string()))?;
    let unit = args[2]
        .as_str()
        .and_then(parse_unit)
        .ok_or_else(|| DbError::ExecutionError("DATE_ADD: unknown unit".to_string()))?;
    let tz = if let Some(s) = args.get(3).and_then(Value::as_str) {
        parse_tz(s)?
    } else {
        chrono_tz::UTC
    };
    Ok(Some(rfc3339_ms(add_calendar(dt, amount * sign, unit, tz)?)))
}

fn date_trunc(args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() < 2 || args.len() > 3 {
        return Err(arity("DATE_TRUNC", "2-3: date, unit, [timezone]"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let dt = parse_datetime(&args[0])?;
    let unit = args[1]
        .as_str()
        .and_then(parse_unit)
        .ok_or_else(|| DbError::ExecutionError("DATE_TRUNC: unknown unit".to_string()))?;
    let tz = if let Some(s) = args.get(2).and_then(Value::as_str) {
        parse_tz(s)?
    } else {
        chrono_tz::UTC
    };
    let local = dt.with_timezone(&tz);
    let naive = match unit {
        Unit::Year => NaiveDate::from_ymd_opt(local.year(), 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
        Unit::Month => NaiveDate::from_ymd_opt(local.year(), local.month(), 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
        Unit::Week => {
            let wd = local.weekday().num_days_from_monday();
            (local.date_naive() - Duration::days(i64::from(wd)))
                .and_hms_opt(0, 0, 0)
                .unwrap()
        }
        Unit::Day => local.date_naive().and_hms_opt(0, 0, 0).unwrap(),
        Unit::Hour => local.date_naive().and_hms_opt(local.hour(), 0, 0).unwrap(),
        Unit::Minute => local
            .date_naive()
            .and_hms_opt(local.hour(), local.minute(), 0)
            .unwrap(),
        Unit::Second => local
            .date_naive()
            .and_hms_opt(local.hour(), local.minute(), local.second())
            .unwrap(),
        Unit::Millisecond => {
            return Ok(Some(rfc3339_ms(dt)));
        }
    };
    let truncated = tz
        .from_local_datetime(&naive)
        .single()
        .ok_or_else(|| DbError::ExecutionError("DATE_TRUNC: invalid local datetime".to_string()))?;
    Ok(Some(rfc3339_ms(truncated.with_timezone(&Utc))))
}

fn date_diff(args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() < 2 || args.len() > 4 {
        return Err(arity("DATE_DIFF", "2-4: date1, date2, [unit], [asFloat]"));
    }
    if args[0].is_null() || args[1].is_null() {
        return Ok(Some(Value::Null));
    }
    let a = parse_datetime(&args[0])?;
    let b = parse_datetime(&args[1])?;
    let unit = args
        .get(2)
        .and_then(Value::as_str)
        .and_then(parse_unit)
        .unwrap_or(Unit::Day);
    let as_float = args.get(3).and_then(Value::as_bool).unwrap_or(false);
    let ms = b.timestamp_millis() - a.timestamp_millis();
    let val = match unit {
        Unit::Year => {
            let years = b.year() - a.year();
            if as_float {
                f64::from(years) + (f64::from(b.month()) - f64::from(a.month())) / 12.0
            } else {
                f64::from(years)
            }
        }
        Unit::Month => {
            let months = (b.year() * 12 + b.month() as i32) - (a.year() * 12 + a.month() as i32);
            if as_float {
                f64::from(months) + (f64::from(b.day()) - f64::from(a.day())) / 30.0
            } else {
                f64::from(months)
            }
        }
        Unit::Week => ms as f64 / (7.0 * 86_400_000.0),
        Unit::Day => ms as f64 / 86_400_000.0,
        Unit::Hour => ms as f64 / 3_600_000.0,
        Unit::Minute => ms as f64 / 60_000.0,
        Unit::Second => ms as f64 / 1_000.0,
        Unit::Millisecond => ms as f64,
    };
    let out = if as_float || matches!(unit, Unit::Millisecond) {
        val
    } else {
        val.trunc()
    };
    Ok(Some(Value::Number(number_from_f64(out))))
}

fn time_bucket(args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() != 2 {
        return Err(arity("TIME_BUCKET", "2: timestamp, interval (e.g. '5m')"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let interval_str = args[1].as_str().ok_or_else(|| {
        DbError::ExecutionError("TIME_BUCKET: interval must be a string".to_string())
    })?;
    if interval_str.len() < 2 {
        return Err(DbError::ExecutionError(
            "TIME_BUCKET: invalid interval format".to_string(),
        ));
    }
    let (num, unit) = interval_str.split_at(interval_str.len() - 1);
    let val: u64 = num
        .parse()
        .map_err(|_| DbError::ExecutionError("TIME_BUCKET: invalid interval number".to_string()))?;
    let interval_ms = match unit {
        "s" => val.saturating_mul(1000),
        "m" => val.saturating_mul(60_000),
        "h" => val.saturating_mul(3_600_000),
        "d" => val.saturating_mul(86_400_000),
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
    let ts = match &args[0] {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .ok_or_else(|| {
                DbError::ExecutionError("TIME_BUCKET: timestamp must be a valid number".to_string())
            })?,
        Value::String(_) => parse_datetime(&args[0])?.timestamp_millis(),
        _ => {
            return Err(DbError::ExecutionError(
                "TIME_BUCKET: timestamp must be number or string".to_string(),
            ))
        }
    };
    let bucket = ts.div_euclid(interval_ms as i64) * interval_ms as i64;
    if args[0].is_string() {
        Ok(Some(rfc3339_ms(
            chrono::DateTime::from_timestamp_millis(bucket).ok_or_else(|| {
                DbError::ExecutionError("TIME_BUCKET: failed to construct date".to_string())
            })?,
        )))
    } else {
        Ok(Some(Value::Number(serde_json::Number::from(bucket))))
    }
}

fn human_time(args: &[Value]) -> DbResult<Option<Value>> {
    if args.is_empty() {
        return Err(arity("HUMAN_TIME", "1-2"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let now = args
        .get(1)
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        .unwrap_or_else(|| Utc::now().timestamp_millis());
    let date_ts = parse_datetime(&args[0])?.timestamp_millis();
    let diff_secs = (now - date_ts) / 1000;
    let future = diff_secs < 0;
    let abs = diff_secs.abs();
    let phrase = if abs < 60 {
        "just now".to_string()
    } else {
        let (n, unit) = if abs < 3600 {
            (abs / 60, "minute")
        } else if abs < 86_400 {
            (abs / 3600, "hour")
        } else if abs < 2_592_000 {
            (abs / 86_400, "day")
        } else if abs < 31_536_000 {
            (abs / 2_592_000, "month")
        } else {
            (abs / 31_536_000, "year")
        };
        let plural = if n == 1 { "" } else { "s" };
        if future {
            format!("in {} {}{}", n, unit, plural)
        } else {
            format!("{} {}{} ago", n, unit, plural)
        }
    };
    Ok(Some(Value::String(phrase)))
}

fn arity(name: &str, expected: &str) -> DbError {
    DbError::ExecutionError(format!("{} requires {} argument(s)", name, expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, args: &[Value]) -> Value {
        evaluate(name, args).unwrap().unwrap()
    }

    #[test]
    fn date_only_and_seconds_epoch() {
        assert_eq!(call("DATE_YEAR", &[json!("2024-06-15")]), json!(2024));
        assert_eq!(call("DATE_YEAR", &[json!(1_609_459_200)]), json!(2021));
    }

    #[test]
    fn add_month_clamps_day() {
        let r = call(
            "DATE_ADD",
            &[json!("2024-01-31T00:00:00Z"), json!(1), json!("month")],
        );
        assert!(r.as_str().unwrap().starts_with("2024-02-29"));
    }

    #[test]
    fn compare_and_leap() {
        assert_eq!(
            call(
                "DATE_COMPARE",
                &[json!("2020-01-01T00:00:00Z"), json!("2020-01-02T00:00:00Z")]
            ),
            json!(-1)
        );
        assert_eq!(
            call("DATE_LEAPYEAR", &[json!("2024-01-01T00:00:00Z")]),
            json!(true)
        );
        assert_eq!(
            call("DATE_LEAPYEAR", &[json!("2023-01-01T00:00:00Z")]),
            json!(false)
        );
    }

    #[test]
    fn null_propagates() {
        assert_eq!(call("DATE_YEAR", &[Value::Null]), Value::Null);
        assert_eq!(
            call("DATE_ADD", &[Value::Null, json!(1), json!("day")]),
            Value::Null
        );
    }
}
