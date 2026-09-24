//! DateTime builtin functions (AQL-shaped, aligned with the server).
//!
//! Calendar units (months, years) are calendar arithmetic, not 30/365-day
//! approximations; every addition is checked, so an out-of-range amount is
//! an error instead of a chrono panic. An optional timezone argument takes
//! an IANA name ("Europe/Paris"); the default is UTC.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde_json::Value;

use super::common::{err, parse_interval_ms};
use crate::error::{SdbqlError, SdbqlResult};
use crate::executor::helpers::number_from_f64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

impl Unit {
    /// Fixed length in milliseconds; `None` for calendar units.
    fn fixed_ms(self) -> Option<i64> {
        match self {
            Unit::Week => Some(7 * 86_400_000),
            Unit::Day => Some(86_400_000),
            Unit::Hour => Some(3_600_000),
            Unit::Minute => Some(60_000),
            Unit::Second => Some(1_000),
            Unit::Millisecond => Some(1),
            Unit::Year | Unit::Month => None,
        }
    }
}

fn parse_unit(s: &str) -> Option<Unit> {
    let unit = match s.to_ascii_lowercase().as_str() {
        "y" | "year" | "years" => Unit::Year,
        "m" | "month" | "months" => Unit::Month,
        "w" | "week" | "weeks" => Unit::Week,
        "d" | "day" | "days" => Unit::Day,
        "h" | "hour" | "hours" => Unit::Hour,
        "i" | "minute" | "minutes" => Unit::Minute,
        "s" | "second" | "seconds" => Unit::Second,
        "f" | "ms" | "millisecond" | "milliseconds" => Unit::Millisecond,
        _ => return None,
    };
    Some(unit)
}

fn unit_arg(name: &str, v: &Value) -> SdbqlResult<Unit> {
    v.as_str()
        .and_then(parse_unit)
        .ok_or_else(|| err(format!("{}: unknown unit {}, use y/m/w/d/h/i/s/f", name, v)))
}

fn parse_tz(s: &str) -> SdbqlResult<Tz> {
    s.parse()
        .map_err(|_| err(format!("unknown timezone '{}'", s)))
}

/// Optional timezone argument at `i`; missing or null means UTC.
fn tz_arg(args: &[Value], i: usize) -> SdbqlResult<Tz> {
    match args.get(i) {
        None | Some(Value::Null) => Ok(chrono_tz::UTC),
        Some(Value::String(s)) => parse_tz(s),
        Some(_) => Err(err("timezone must be a string")),
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 {
        // Saturating: `year` can come from caller-controlled arithmetic.
        (year.saturating_add(1), 1)
    } else {
        (year, month + 1)
    };
    match (
        NaiveDate::from_ymd_opt(year, month, 1),
        NaiveDate::from_ymd_opt(ny, nm, 1),
    ) {
        (Some(a), Some(b)) => (b - a).num_days() as u32,
        _ => 30,
    }
}

fn out_of_range(name: &str) -> SdbqlError {
    err(format!(
        "{}: result is out of the representable date range",
        name
    ))
}

/// Resolve a local wall-clock time. An ambiguous time (DST fall-back) takes
/// the earlier instant; a time inside a DST gap moves forward past the gap.
fn resolve_local(name: &str, tz: Tz, naive: NaiveDateTime) -> SdbqlResult<DateTime<Tz>> {
    if let Some(dt) = tz.from_local_datetime(&naive).earliest() {
        return Ok(dt);
    }
    for minutes in [30, 60, 90, 120, 180] {
        let shifted = naive
            .checked_add_signed(Duration::minutes(minutes))
            .ok_or_else(|| out_of_range(name))?;
        if let Some(dt) = tz.from_local_datetime(&shifted).earliest() {
            return Ok(dt);
        }
    }
    Err(err(format!("{}: invalid local datetime {}", name, naive)))
}

/// Add whole months to a naive datetime, clamping the day to the target
/// month's length (Jan 31 + 1 month = Feb 28/29).
fn add_months_naive(n: NaiveDateTime, months: i64) -> Option<NaiveDateTime> {
    let total = (i64::from(n.year()) * 12 + i64::from(n.month()) - 1).checked_add(months)?;
    let year = i32::try_from(total.div_euclid(12)).ok()?;
    let month = (total.rem_euclid(12) + 1) as u32;
    let day = n.day().min(days_in_month(year, month));
    Some(NaiveDate::from_ymd_opt(year, month, day)?.and_time(n.time()))
}

fn add_calendar(
    name: &str,
    dt: DateTime<Utc>,
    amount: i64,
    unit: Unit,
    tz: Tz,
) -> SdbqlResult<DateTime<Utc>> {
    let local = dt.with_timezone(&tz).naive_local();
    let result = match unit {
        Unit::Year | Unit::Month => {
            let months = if unit == Unit::Year {
                amount.checked_mul(12).ok_or_else(|| out_of_range(name))?
            } else {
                amount
            };
            let naive = add_months_naive(local, months).ok_or_else(|| out_of_range(name))?;
            resolve_local(name, tz, naive)?.with_timezone(&Utc)
        }
        // Days and weeks are calendar days in the timezone (a day across a
        // DST change is 23 or 25 hours).
        Unit::Week | Unit::Day => {
            let days = if unit == Unit::Week {
                amount.checked_mul(7).ok_or_else(|| out_of_range(name))?
            } else {
                amount
            };
            let delta = Duration::try_days(days).ok_or_else(|| out_of_range(name))?;
            let naive = local
                .checked_add_signed(delta)
                .ok_or_else(|| out_of_range(name))?;
            resolve_local(name, tz, naive)?.with_timezone(&Utc)
        }
        _ => {
            let delta = match unit {
                Unit::Hour => Duration::try_hours(amount),
                Unit::Minute => Duration::try_minutes(amount),
                Unit::Second => Duration::try_seconds(amount),
                _ => Duration::try_milliseconds(amount),
            }
            .ok_or_else(|| out_of_range(name))?;
            dt.checked_add_signed(delta)
                .ok_or_else(|| out_of_range(name))?
        }
    };
    Ok(result)
}

/// Parse a date value. Numbers are milliseconds since the epoch, unless
/// `|n| < 10_000_000_000`, which are seconds (same rule as the server, so
/// `DATE_YEAR(1609459200)` is 2021). Strings accept RFC 3339,
/// `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM:SS[.fff]` and `YYYY-MM-DD HH:MM:SS`.
pub(crate) fn parse_datetime(v: &Value) -> SdbqlResult<DateTime<Utc>> {
    match v {
        Value::Number(n) => {
            let raw = n
                .as_i64()
                .or_else(|| n.as_f64().map(|f| f as i64))
                .ok_or_else(|| err("Invalid timestamp number"))?;
            // `unsigned_abs`: `i64::MIN.abs()` overflows.
            let ms = if raw.unsigned_abs() < 10_000_000_000 {
                raw.saturating_mul(1000)
            } else {
                raw
            };
            DateTime::from_timestamp_millis(ms)
                .ok_or_else(|| err(format!("Invalid timestamp: {}", ms)))
        }
        Value::String(s) => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Ok(dt.with_timezone(&Utc));
            }
            if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                return Ok(d.and_time(chrono::NaiveTime::MIN).and_utc());
            }
            for fmt in [
                "%Y-%m-%dT%H:%M:%S%.f",
                "%Y-%m-%dT%H:%M:%S",
                "%Y-%m-%d %H:%M:%S%.f",
                "%Y-%m-%d %H:%M:%S",
            ] {
                if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
                    return Ok(dt.and_utc());
                }
            }
            Err(err(format!("Cannot parse date string: {}", s)))
        }
        _ => Err(err("Date must be a string or number")),
    }
}

fn rfc3339_ms(dt: DateTime<Utc>) -> Value {
    Value::String(dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn arity(name: &str, expected: &str) -> SdbqlError {
    err(format!("{} requires {} argument(s)", name, expected))
}

/// DATE_YEAR(date, [timezone]) and friends.
fn extract(
    name: &str,
    args: &[Value],
    f: impl FnOnce(DateTime<Tz>) -> i64,
) -> SdbqlResult<Option<Value>> {
    if args.is_empty() || args.len() > 2 {
        return Err(arity(name, "1-2: date, [timezone]"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let tz = tz_arg(args, 1)?;
    let dt = parse_datetime(&args[0])?.with_timezone(&tz);
    Ok(Some(Value::from(f(dt))))
}

fn amount_arg(name: &str, v: &Value) -> SdbqlResult<i64> {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .ok_or_else(|| err(format!("{}: amount must be a number", name)))
}

/// DATE_ADD / DATE_SUBTRACT(date, amount, unit, [timezone]).
fn date_add(name: &str, args: &[Value], sign: i64) -> SdbqlResult<Option<Value>> {
    if args.len() < 3 || args.len() > 4 {
        return Err(arity(name, "3-4: date, amount, unit, [timezone]"));
    }
    if args[0].is_null() || args[1].is_null() {
        return Ok(Some(Value::Null));
    }
    let dt = parse_datetime(&args[0])?;
    let amount = amount_arg(name, &args[1])?
        .checked_mul(sign)
        .ok_or_else(|| out_of_range(name))?;
    let unit = unit_arg(name, &args[2])?;
    let tz = tz_arg(args, 3)?;
    Ok(Some(rfc3339_ms(add_calendar(name, dt, amount, unit, tz)?)))
}

/// Whole (or fractional) calendar months from `a` to `b`, both naive local
/// times. Negative when `b` is before `a`.
fn months_between(a: NaiveDateTime, b: NaiveDateTime, as_float: bool) -> Option<f64> {
    if b < a {
        return months_between(b, a, as_float).map(|m| -m);
    }
    let mut whole = (i64::from(b.year()) - i64::from(a.year())) * 12
        + (i64::from(b.month()) - i64::from(a.month()));
    let mut anchor = add_months_naive(a, whole)?;
    if anchor > b {
        whole -= 1;
        anchor = add_months_naive(a, whole)?;
    }
    if !as_float {
        return Some(whole as f64);
    }
    let next = add_months_naive(a, whole + 1)?;
    let span = (next - anchor).num_milliseconds();
    let frac = if span > 0 {
        (b - anchor).num_milliseconds() as f64 / span as f64
    } else {
        0.0
    };
    Some(whole as f64 + frac)
}

/// DATE_DIFF(date1, date2, [unit], [asFloat], [tz1], [tz2]) = date2 - date1.
fn date_diff(args: &[Value]) -> SdbqlResult<Option<Value>> {
    const NAME: &str = "DATE_DIFF";
    if args.len() < 2 || args.len() > 6 {
        return Err(arity(
            NAME,
            "2-6: date1, date2, [unit], [asFloat], [timezone1], [timezone2]",
        ));
    }
    if args[0].is_null() || args[1].is_null() {
        return Ok(Some(Value::Null));
    }
    let a = parse_datetime(&args[0])?;
    let b = parse_datetime(&args[1])?;
    let unit = match args.get(2) {
        None | Some(Value::Null) => Unit::Day,
        Some(v) => unit_arg(NAME, v)?,
    };
    let as_float = args.get(3).and_then(Value::as_bool).unwrap_or(false);
    let tz1 = tz_arg(args, 4)?;
    let tz2 = if args.get(5).is_some_and(|v| !v.is_null()) {
        tz_arg(args, 5)?
    } else {
        tz1
    };

    let value = match unit.fixed_ms() {
        Some(unit_ms) => {
            let ms = b.timestamp_millis() - a.timestamp_millis();
            if as_float {
                Value::Number(number_from_f64(ms as f64 / unit_ms as f64))
            } else {
                // Integer division truncates toward zero, like AQL.
                Value::from(ms / unit_ms)
            }
        }
        None => {
            let na = a.with_timezone(&tz1).naive_local();
            let nb = b.with_timezone(&tz2).naive_local();
            let months = months_between(na, nb, as_float).ok_or_else(|| out_of_range(NAME))?;
            let v = if unit == Unit::Year {
                months / 12.0
            } else {
                months
            };
            if as_float {
                Value::Number(number_from_f64(v))
            } else {
                Value::from(v.trunc() as i64)
            }
        }
    };
    Ok(Some(value))
}

fn midnight(d: NaiveDate) -> NaiveDateTime {
    d.and_time(chrono::NaiveTime::MIN)
}

/// DATE_TRUNC(date, unit, [timezone]): start of the enclosing unit.
fn date_trunc(args: &[Value]) -> SdbqlResult<Option<Value>> {
    const NAME: &str = "DATE_TRUNC";
    if args.len() < 2 || args.len() > 3 {
        return Err(arity(NAME, "2-3: date, unit, [timezone]"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let dt = parse_datetime(&args[0])?;
    let unit = unit_arg(NAME, &args[1])?;
    let tz = tz_arg(args, 2)?;
    let local = dt.with_timezone(&tz).naive_local();
    let date = local.date();
    let naive = match unit {
        Unit::Year => {
            midnight(NaiveDate::from_ymd_opt(date.year(), 1, 1).ok_or_else(|| out_of_range(NAME))?)
        }
        Unit::Month => midnight(
            NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
                .ok_or_else(|| out_of_range(NAME))?,
        ),
        Unit::Week => {
            let back = i64::from(date.weekday().num_days_from_monday());
            midnight(
                date.checked_sub_signed(Duration::days(back))
                    .ok_or_else(|| out_of_range(NAME))?,
            )
        }
        Unit::Day => midnight(date),
        Unit::Hour => local
            .with_minute(0)
            .and_then(|t| t.with_second(0))
            .and_then(|t| t.with_nanosecond(0))
            .ok_or_else(|| out_of_range(NAME))?,
        Unit::Minute => local
            .with_second(0)
            .and_then(|t| t.with_nanosecond(0))
            .ok_or_else(|| out_of_range(NAME))?,
        Unit::Second => local.with_nanosecond(0).ok_or_else(|| out_of_range(NAME))?,
        Unit::Millisecond => return Ok(Some(rfc3339_ms(dt))),
    };
    Ok(Some(rfc3339_ms(
        resolve_local(NAME, tz, naive)?.with_timezone(&Utc),
    )))
}

/// AQL DATE_ROUND(date, amount, unit): floor into buckets of `amount`
/// units (d/h/i/s/f, and w). DATE_ROUND(date, unit) truncates, as on the
/// server.
fn date_round(args: &[Value]) -> SdbqlResult<Option<Value>> {
    const NAME: &str = "DATE_ROUND";
    if args.len() >= 2 && args[1].is_string() {
        return date_trunc(args);
    }
    if args.len() != 3 {
        return Err(arity(NAME, "3: date, amount, unit"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let dt = parse_datetime(&args[0])?;
    let amount = amount_arg(NAME, &args[1])?;
    let unit = unit_arg(NAME, &args[2])?;
    let unit_ms = unit
        .fixed_ms()
        .ok_or_else(|| err("DATE_ROUND: unit must be w, d, h, i, s or f"))?;
    if amount <= 0 {
        return Err(err("DATE_ROUND: amount must be positive"));
    }
    let bucket = amount
        .checked_mul(unit_ms)
        .ok_or_else(|| out_of_range(NAME))?;
    let ts = dt.timestamp_millis();
    let floored = ts.div_euclid(bucket) * bucket;
    Ok(Some(rfc3339_ms(
        DateTime::from_timestamp_millis(floored).ok_or_else(|| out_of_range(NAME))?,
    )))
}

fn time_bucket(args: &[Value]) -> SdbqlResult<Option<Value>> {
    const NAME: &str = "TIME_BUCKET";
    if args.len() != 2 {
        return Err(arity(NAME, "2: timestamp, interval (e.g. '5m')"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let interval = args[1]
        .as_str()
        .ok_or_else(|| err("TIME_BUCKET: interval must be a string"))?;
    let interval_ms = parse_interval_ms(interval)?;
    let ts = match &args[0] {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .ok_or_else(|| err("TIME_BUCKET: timestamp must be a valid number"))?,
        Value::String(_) => parse_datetime(&args[0])?.timestamp_millis(),
        _ => return Err(err("TIME_BUCKET: timestamp must be number or string")),
    };
    let bucket = ts
        .div_euclid(interval_ms)
        .checked_mul(interval_ms)
        .ok_or_else(|| err("TIME_BUCKET: timestamp out of range"))?;
    if args[0].is_string() {
        Ok(Some(rfc3339_ms(
            DateTime::from_timestamp_millis(bucket).ok_or_else(|| out_of_range(NAME))?,
        )))
    } else {
        Ok(Some(Value::from(bucket)))
    }
}

fn human_time(args: &[Value]) -> SdbqlResult<Option<Value>> {
    if args.is_empty() || args.len() > 2 {
        return Err(arity("HUMAN_TIME", "1-2: date, [now]"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    // `now` may be a timestamp or a date string, like the date itself.
    let now = match args.get(1) {
        None | Some(Value::Null) => Utc::now().timestamp_millis(),
        Some(v) => parse_datetime(v)?.timestamp_millis(),
    };
    let date_ts = parse_datetime(&args[0])?.timestamp_millis();
    let diff_secs = now
        .checked_sub(date_ts)
        .ok_or_else(|| err("HUMAN_TIME: timestamp out of range"))?
        / 1000;
    let future = diff_secs < 0;
    let abs = diff_secs.unsigned_abs();
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

/// Call a datetime function. Returns None if function not found.
pub fn call(name: &str, args: &[Value]) -> SdbqlResult<Option<Value>> {
    match name {
        "NOW" | "DATE_NOW" => Ok(Some(Value::from(Utc::now().timestamp_millis()))),
        "NOW_ISO" | "DATE_NOW_ISO" => Ok(Some(Value::String(Utc::now().to_rfc3339()))),

        "DATE_YEAR" => extract(name, args, |dt| i64::from(dt.year())),
        "DATE_MONTH" => extract(name, args, |dt| i64::from(dt.month())),
        "DATE_DAY" => extract(name, args, |dt| i64::from(dt.day())),
        "DATE_HOUR" => extract(name, args, |dt| i64::from(dt.hour())),
        "DATE_MINUTE" => extract(name, args, |dt| i64::from(dt.minute())),
        "DATE_SECOND" => extract(name, args, |dt| i64::from(dt.second())),
        "DATE_MILLISECOND" => extract(name, args, |dt| i64::from(dt.timestamp_subsec_millis())),
        // 0 = Sunday, 6 = Saturday
        "DATE_DAYOFWEEK" => extract(name, args, |dt| {
            i64::from(dt.weekday().num_days_from_sunday())
        }),
        "DATE_DAYOFYEAR" => extract(name, args, |dt| i64::from(dt.ordinal())),
        "DATE_WEEK" | "DATE_ISOWEEK" => extract(name, args, |dt| i64::from(dt.iso_week().week())),
        "DATE_ISOWEEKYEAR" => extract(name, args, |dt| i64::from(dt.iso_week().year())),
        "DATE_QUARTER" => extract(name, args, |dt| i64::from((dt.month() - 1) / 3 + 1)),
        "DATE_DAYS_IN_MONTH" => extract(name, args, |dt| {
            i64::from(days_in_month(dt.year(), dt.month()))
        }),

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
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let a = parse_datetime(&args[0])?;
            let b = parse_datetime(&args[1])?;
            Ok(Some(Value::from(a.cmp(&b) as i8 as i64)))
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
            Ok(Some(Value::from(
                parse_datetime(&args[0])?.timestamp_millis(),
            )))
        }

        "DATE_FORMAT" => {
            // DATE_FORMAT(date) is the ISO string (older core form);
            // DATE_FORMAT(date, format, [timezone]) uses strftime, as the
            // server does.
            if args.is_empty() || args.len() > 3 {
                return Err(arity(name, "1-3: date, [format], [timezone]"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let dt = parse_datetime(&args[0])?;
            let Some(fmt_arg) = args.get(1) else {
                return Ok(Some(rfc3339_ms(dt)));
            };
            let fmt = fmt_arg
                .as_str()
                .ok_or_else(|| err("DATE_FORMAT: format must be a string"))?;
            let tz = tz_arg(args, 2)?;
            // `to_string()` panics on an invalid specifier; `write!` reports it.
            use std::fmt::Write as _;
            let mut out = String::new();
            write!(out, "{}", dt.with_timezone(&tz).format(fmt))
                .map_err(|_| err(format!("DATE_FORMAT: invalid format string '{}'", fmt)))?;
            Ok(Some(Value::String(out)))
        }

        "DATE_TRUNC" => date_trunc(args),
        "DATE_ROUND" => date_round(args),
        "DATE_ADD" => date_add(name, args, 1),
        "DATE_SUBTRACT" | "DATE_SUB" => date_add(name, args, -1),
        "DATE_DIFF" => date_diff(args),
        "TIME_BUCKET" => time_bucket(args),
        "HUMAN_TIME" => human_time(args),

        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ok(name: &str, args: &[Value]) -> Value {
        call(name, args).unwrap().unwrap()
    }

    #[test]
    fn test_now() {
        let result = call("NOW", &[]).unwrap().unwrap();
        assert!(result.is_number());

        let result = call("NOW_ISO", &[]).unwrap().unwrap();
        assert!(result.is_string());
    }

    #[test]
    fn test_date_parts() {
        let date = json!("2024-03-15T10:30:45Z");

        assert_eq!(
            call("DATE_YEAR", &[date.clone()]).unwrap(),
            Some(json!(2024))
        );
        assert_eq!(call("DATE_MONTH", &[date.clone()]).unwrap(), Some(json!(3)));
        assert_eq!(call("DATE_DAY", &[date.clone()]).unwrap(), Some(json!(15)));
        assert_eq!(call("DATE_HOUR", &[date.clone()]).unwrap(), Some(json!(10)));
        assert_eq!(
            call("DATE_MINUTE", &[date.clone()]).unwrap(),
            Some(json!(30))
        );
        assert_eq!(call("DATE_SECOND", &[date]).unwrap(), Some(json!(45)));
    }

    #[test]
    fn test_date_add() {
        let date = json!("2024-03-15T10:30:45Z");
        let result = call("DATE_ADD", &[date, json!(7), json!("d")])
            .unwrap()
            .unwrap();
        let result_str = result.as_str().unwrap();
        assert!(result_str.contains("2024-03-22"));
    }

    #[test]
    fn test_date_diff() {
        // DATE_DIFF is date2 - date1 (AQL and the server). This test used to
        // assert 5 for (03-15, 03-10), the opposite sign.
        let date1 = json!("2024-03-15T00:00:00Z");
        let date2 = json!("2024-03-10T00:00:00Z");
        let result = call("DATE_DIFF", &[date1.clone(), date2.clone(), json!("d")])
            .unwrap()
            .unwrap();
        assert_eq!(result.as_i64().unwrap(), -5);
        let result = call("DATE_DIFF", &[date2, date1, json!("d")])
            .unwrap()
            .unwrap();
        assert_eq!(result.as_i64().unwrap(), 5);
    }

    #[test]
    fn date_diff_uses_calendar_months_and_years() {
        // Was 365-day years: one day across New Year is 0 years, not 1.
        assert_eq!(
            ok(
                "DATE_DIFF",
                &[json!("2023-12-31"), json!("2024-01-01"), json!("y")]
            ),
            json!(0)
        );
        assert_eq!(
            ok(
                "DATE_DIFF",
                &[json!("2000-06-15"), json!("2024-06-14"), json!("years")]
            ),
            json!(23)
        );
        assert_eq!(
            ok(
                "DATE_DIFF",
                &[json!("2000-06-15"), json!("2024-06-15"), json!("years")]
            ),
            json!(24)
        );
        // Jan 31 -> Mar 1 is one whole month (Feb 29 anchor), not 30-day math.
        assert_eq!(
            ok(
                "DATE_DIFF",
                &[json!("2024-01-31"), json!("2024-03-01"), json!("m")]
            ),
            json!(1)
        );
        assert_eq!(
            ok(
                "DATE_DIFF",
                &[json!("2024-03-01"), json!("2024-01-31"), json!("m")]
            ),
            json!(-1)
        );
        let half = ok(
            "DATE_DIFF",
            &[
                json!("2024-01-01"),
                json!("2024-01-16T12:00:00Z"),
                json!("m"),
                json!(true),
            ],
        );
        assert!((half.as_f64().unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(
            ok(
                "DATE_DIFF",
                &[
                    json!("2024-01-01T00:00:00Z"),
                    json!("2024-01-01T00:00:01.5Z"),
                    json!("ms")
                ]
            ),
            json!(1500)
        );
        assert!(call(
            "DATE_DIFF",
            &[json!("2024-01-01"), json!("2024-01-02"), json!("fortnight")]
        )
        .is_err());
    }

    #[test]
    fn test_timestamp_input() {
        // March 15, 2024 in milliseconds
        let ts = json!(1710489600000_i64);
        let result = call("DATE_YEAR", &[ts]).unwrap().unwrap();
        assert_eq!(result.as_i64().unwrap(), 2024);
        // Small numbers are seconds, as on the server.
        assert_eq!(ok("DATE_YEAR", &[json!(1_609_459_200)]), json!(2021));
    }

    #[test]
    fn add_month_clamps_day_and_years_are_calendar() {
        let r = ok(
            "DATE_ADD",
            &[json!("2024-01-31T00:00:00Z"), json!(1), json!("month")],
        );
        assert!(r.as_str().unwrap().starts_with("2024-02-29"));
        // Was 365 days: 2024 is a leap year.
        let r = ok(
            "DATE_ADD",
            &[json!("2024-01-01T00:00:00Z"), json!(1), json!("y")],
        );
        assert!(r.as_str().unwrap().starts_with("2025-01-01"));
        let r = ok(
            "DATE_SUBTRACT",
            &[json!("2024-03-31T00:00:00Z"), json!(1), json!("m")],
        );
        assert!(r.as_str().unwrap().starts_with("2024-02-29"));
        let r = ok(
            "DATE_ADD",
            &[json!("2024-01-01T00:00:00Z"), json!(1500), json!("ms")],
        );
        assert_eq!(r, json!("2024-01-01T00:00:01.500Z"));
    }

    #[test]
    fn date_add_overflow_is_an_error_not_a_panic() {
        for unit in [
            "day", "week", "hour", "minute", "second", "ms", "month", "year",
        ] {
            let r = call(
                "DATE_ADD",
                &[json!("2024-01-01T00:00:00Z"), json!(1e17), json!(unit)],
            );
            assert!(r.is_err(), "unit {unit} should overflow");
        }
        for amount in [i64::MAX, i64::MIN] {
            for unit in ["d", "w", "y", "m", "h", "i", "s", "f"] {
                assert!(call(
                    "DATE_ADD",
                    &[json!("2024-01-01T00:00:00Z"), json!(amount), json!(unit)]
                )
                .is_err());
                assert!(call(
                    "DATE_SUBTRACT",
                    &[json!("2024-01-01T00:00:00Z"), json!(amount), json!(unit)]
                )
                .is_err());
            }
        }
        assert!(call(
            "DATE_ADD",
            &[json!("2024-01-01"), json!(1), json!("fortnight")]
        )
        .is_err());
    }

    #[test]
    fn timezone_arguments() {
        // Paris is UTC+1 in January.
        assert_eq!(
            ok(
                "DATE_HOUR",
                &[json!("2024-01-01T23:30:00Z"), json!("Europe/Paris")]
            ),
            json!(0)
        );
        assert_eq!(
            ok(
                "DATE_DAY",
                &[json!("2024-01-01T23:30:00Z"), json!("Europe/Paris")]
            ),
            json!(2)
        );
        // A calendar day across the spring-forward change is 23 hours.
        let r = ok(
            "DATE_ADD",
            &[
                json!("2024-03-30T12:00:00Z"),
                json!(1),
                json!("d"),
                json!("Europe/Paris"),
            ],
        );
        assert_eq!(r, json!("2024-03-31T11:00:00.000Z"));
        assert!(call("DATE_YEAR", &[json!("2024-01-01"), json!("Mars/Olympus")]).is_err());
    }

    #[test]
    fn new_date_functions() {
        assert_eq!(ok("DATE_QUARTER", &[json!("2024-12-01")]), json!(4));
        assert_eq!(
            ok("DATE_DAYS_IN_MONTH", &[json!("2024-02-15T00:00:00Z")]),
            json!(29)
        );
        assert_eq!(ok("DATE_LEAPYEAR", &[json!("2023-06-01")]), json!(false));
        assert_eq!(ok("DATE_ISOWEEKYEAR", &[json!("2021-01-01")]), json!(2020));
        assert_eq!(
            ok("DATE_MILLISECOND", &[json!("2024-01-01T00:00:00.250Z")]),
            json!(250)
        );
        assert_eq!(
            ok(
                "DATE_TRUNC",
                &[json!("2024-05-17T13:45:12Z"), json!("month")]
            ),
            json!("2024-05-01T00:00:00.000Z")
        );
        assert_eq!(
            ok(
                "DATE_TRUNC",
                &[json!("2024-05-17T13:45:12Z"), json!("week")]
            ),
            json!("2024-05-13T00:00:00.000Z")
        );
        assert_eq!(
            ok(
                "DATE_ROUND",
                &[json!("2024-05-17T13:47:12Z"), json!(15), json!("i")]
            ),
            json!("2024-05-17T13:45:00.000Z")
        );
        assert_eq!(
            ok(
                "DATE_FORMAT",
                &[json!("2024-05-17T13:45:12Z"), json!("%Y/%m/%d")]
            ),
            json!("2024/05/17")
        );
        assert!(call("DATE_FORMAT", &[json!("2024-01-01T00:00:00Z"), json!("%Q")]).is_err());
        assert_eq!(
            ok("DATE_ISO8601", &[json!(1_733_234_387_000_i64)]),
            json!("2024-12-03T13:59:47.000Z")
        );
        assert_eq!(
            ok("DATE_COMPARE", &[json!("2020-01-01"), json!("2020-01-02")]),
            json!(-1)
        );
    }

    #[test]
    fn time_bucket_and_human_time() {
        assert_eq!(
            ok("TIME_BUCKET", &[json!(1_700_000_123_456_i64), json!("1m")]),
            json!(1_700_000_100_000_i64)
        );
        assert_eq!(
            ok("TIME_BUCKET", &[json!("2024-01-01T10:07:00Z"), json!("5m")]),
            json!("2024-01-01T10:05:00.000Z")
        );
        // Non-ASCII unit used to panic in `split_at`.
        assert!(call("TIME_BUCKET", &[json!(1), json!("5µ")]).is_err());
        assert!(call("TIME_BUCKET", &[json!(1), json!("é")]).is_err());
        let _ = call("TIME_BUCKET", &[json!(i64::MIN), json!("7d")]);
        assert_eq!(
            ok(
                "HUMAN_TIME",
                &[json!("2024-01-01T00:00:00Z"), json!("2024-01-01T00:05:00Z")]
            ),
            json!("5 minutes ago")
        );
        assert_eq!(
            ok(
                "HUMAN_TIME",
                &[json!("2024-01-03T00:00:00Z"), json!("2024-01-01T00:00:00Z")]
            ),
            json!("in 2 days")
        );
        assert!(call(
            "HUMAN_TIME",
            &[json!("2024-01-01T00:00:00Z"), json!(i64::MIN)]
        )
        .is_err());
    }

    #[test]
    fn null_propagates() {
        assert_eq!(ok("DATE_YEAR", &[Value::Null]), Value::Null);
        assert_eq!(
            ok("DATE_ADD", &[Value::Null, json!(1), json!("day")]),
            Value::Null
        );
        assert_eq!(
            ok("DATE_DIFF", &[Value::Null, json!("2024-01-01")]),
            Value::Null
        );
    }
}
