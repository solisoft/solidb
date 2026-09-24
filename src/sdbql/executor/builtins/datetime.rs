//! Date and time functions for SDBQL (AQL-shaped).
//!
//! Single implementation — `phonetic/date.rs` forwards here.
//!
//! Conventions shared by every function in this file:
//! - A date is an ISO-8601 string (see `utils::parse_date_str`) or a number of
//!   milliseconds since the epoch; a number with `|n| < 1e10` is read as
//!   seconds (`utils::SECONDS_EPOCH_THRESHOLD`). `TIME_BUCKET` is the
//!   exception: a number there is always milliseconds, like the other
//!   time-series functions.
//! - A timezone argument is an IANA name; anything else that is not null is
//!   an error.
//! - Local times that fall in a DST overlap resolve to the earlier instant
//!   (or to the offset the input already had); local times in a DST gap move
//!   forward by the length of the gap (02:30 in a spring-forward becomes 03:30).

use crate::error::{DbError, DbResult};
use crate::sdbql::executor::builtins::timeseries::{parse_interval, Interval};
use crate::sdbql::executor::utils::{number_from_f64, parse_date_value, parse_datetime};
use chrono::{
    DateTime, Datelike, Duration, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, Offset,
    TimeZone, Timelike, Utc,
};
use chrono_tz::Tz;
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
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
    /// Fixed length in ms, for units that have one.
    fn fixed_ms(self) -> Option<i64> {
        match self {
            Unit::Week => Some(604_800_000),
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
    let l = s.trim().to_ascii_lowercase();
    Some(match l.as_str() {
        "y" | "year" | "years" => Unit::Year,
        "m" | "month" | "months" => Unit::Month,
        "w" | "week" | "weeks" => Unit::Week,
        "d" | "day" | "days" => Unit::Day,
        "h" | "hour" | "hours" => Unit::Hour,
        "i" | "minute" | "minutes" => Unit::Minute,
        "s" | "second" | "seconds" => Unit::Second,
        "f" | "ms" | "millisecond" | "milliseconds" => Unit::Millisecond,
        _ => return None,
    })
}

/// Unit argument: must be a string naming a known unit.
fn unit_arg(fname: &str, v: &Value) -> DbResult<Unit> {
    let s = v
        .as_str()
        .ok_or_else(|| DbError::ExecutionError(format!("{fname}: unit must be a string")))?;
    parse_unit(s).ok_or_else(|| {
        DbError::ExecutionError(format!(
            "{fname}: unknown unit '{s}' (y, m, w, d, h, i, s, f)"
        ))
    })
}

fn parse_tz(s: &str) -> DbResult<Tz> {
    s.parse()
        .map_err(|_| DbError::ExecutionError(format!("unknown timezone '{}'", s)))
}

/// Optional timezone at `args[idx]`: absent or null is UTC, a string must be
/// an IANA name, anything else is an error.
fn opt_tz(fname: &str, args: &[Value], idx: usize) -> DbResult<Tz> {
    match args.get(idx) {
        None | Some(Value::Null) => Ok(chrono_tz::UTC),
        Some(Value::String(s)) => parse_tz(s),
        Some(_) => Err(DbError::ExecutionError(format!(
            "{fname}: timezone must be a string"
        ))),
    }
}

fn opt_bool(fname: &str, what: &str, v: Option<&Value>) -> DbResult<bool> {
    match v {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(DbError::ExecutionError(format!(
            "{fname}: {what} must be a boolean"
        ))),
    }
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

/// Turn a wall-clock time into an instant in `tz`.
///
/// Overlap (clocks go back): the candidate whose offset is `prefer`, else the
/// earlier one. Gap (clocks go forward): shift forward by the gap's length.
fn resolve_local(
    tz: Tz,
    naive: NaiveDateTime,
    prefer: Option<FixedOffset>,
) -> DbResult<DateTime<Tz>> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(d) => Ok(d),
        LocalResult::Ambiguous(a, b) => {
            if let Some(off) = prefer {
                if a.offset().fix() == off {
                    return Ok(a);
                }
                if b.offset().fix() == off {
                    return Ok(b);
                }
            }
            Ok(a.min(b))
        }
        LocalResult::None => {
            if let Some(gap) = chrono_tz::GapInfo::new(&naive, &tz) {
                if let Some((_, before)) = gap.begin {
                    let secs = i64::from(before.fix().local_minus_utc());
                    if let Some(utc) = naive.checked_sub_signed(Duration::seconds(secs)) {
                        return Ok(tz.from_utc_datetime(&utc));
                    }
                }
                if let Some(end) = gap.end {
                    return Ok(end);
                }
            }
            Err(DbError::ExecutionError(format!(
                "local time {} does not exist in {}",
                naive,
                tz.name()
            )))
        }
    }
}

fn add_months_naive(naive: NaiveDateTime, months: i64) -> DbResult<NaiveDateTime> {
    // Audit A9: checked all the way — `months` is caller-controlled.
    let total = (i64::from(naive.year()) * 12 + i64::from(naive.month()) - 1)
        .checked_add(months)
        .ok_or_else(out_of_range)?;
    let new_year = i32::try_from(total.div_euclid(12)).map_err(|_| out_of_range())?;
    let new_month = (total.rem_euclid(12) + 1) as u32;
    let new_day = naive.day().min(days_in_month(new_year, new_month));
    let date = NaiveDate::from_ymd_opt(new_year, new_month, new_day).ok_or_else(out_of_range)?;
    Ok(date.and_time(naive.time()))
}

fn add_calendar(dt: DateTime<Utc>, amount: i64, unit: Unit, tz: Tz) -> DbResult<DateTime<Utc>> {
    let local = dt.with_timezone(&tz);
    let prefer = Some(local.offset().fix());
    let result = match unit {
        Unit::Year | Unit::Month => {
            let months = if unit == Unit::Year {
                amount.checked_mul(12).ok_or_else(out_of_range)?
            } else {
                amount
            };
            let naive = add_months_naive(local.naive_local(), months)?;
            resolve_local(tz, naive, prefer)?
        }
        // Days and weeks are calendar days in `tz`: the wall-clock time is
        // kept across a DST change, so one "day" may be 23 or 25 hours.
        Unit::Week | Unit::Day => {
            let days = if unit == Unit::Week {
                amount.checked_mul(7).ok_or_else(out_of_range)?
            } else {
                amount
            };
            let delta = Duration::try_days(days).ok_or_else(out_of_range)?;
            let naive = local
                .naive_local()
                .checked_add_signed(delta)
                .ok_or_else(out_of_range)?;
            resolve_local(tz, naive, prefer)?
        }
        // Audit A9: `Duration::hours(1e12)` and `DateTime + Duration` both
        // panic on overflow; the `try_`/`checked_` forms report it instead.
        _ => {
            let delta = match unit {
                Unit::Hour => Duration::try_hours(amount),
                Unit::Minute => Duration::try_minutes(amount),
                Unit::Second => Duration::try_seconds(amount),
                _ => Duration::try_milliseconds(amount),
            }
            .ok_or_else(out_of_range)?;
            return dt.checked_add_signed(delta).ok_or_else(out_of_range);
        }
    };
    Ok(result.with_timezone(&Utc))
}

fn out_of_range() -> DbError {
    DbError::ExecutionError("DATE_ADD: result is out of the representable date range".to_string())
}

/// An ISO-8601 duration: calendar months, calendar days, then exact ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct IsoDuration {
    months: i64,
    days: i64,
    ms: i64,
}

/// `P1Y2M3W4DT5H6M7.5S`, optionally signed (`-P1D`). Fractions are allowed
/// on seconds only.
fn parse_iso_duration(s: &str) -> Option<IsoDuration> {
    let s = s.trim();
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let rest = rest.strip_prefix(['P', 'p'])?;
    let mut d = IsoDuration::default();
    let mut in_time = false;
    let mut any = false;
    let mut time_any = false;
    let mut num = String::new();
    // Units must appear in order; track the last one seen per section.
    let mut last_rank = 0u8;
    for c in rest.chars() {
        let c = c.to_ascii_uppercase();
        if c == 'T' {
            if in_time || !num.is_empty() {
                return None;
            }
            in_time = true;
            last_rank = 0;
            continue;
        }
        if c.is_ascii_digit() || c == '.' || c == ',' {
            num.push(if c == ',' { '.' } else { c });
            continue;
        }
        if num.is_empty() {
            return None;
        }
        let rank = match (in_time, c) {
            (false, 'Y') => 1,
            (false, 'M') => 2,
            (false, 'W') => 3,
            (false, 'D') => 4,
            (true, 'H') => 1,
            (true, 'M') => 2,
            (true, 'S') => 3,
            _ => return None,
        };
        if rank <= last_rank {
            return None;
        }
        last_rank = rank;
        let is_seconds = in_time && c == 'S';
        if num.contains('.') && !is_seconds {
            return None;
        }
        if is_seconds {
            let secs: f64 = num.parse().ok()?;
            let ms = (secs * 1000.0).round();
            if !ms.is_finite() || ms.abs() > 9.0e15 {
                return None;
            }
            d.ms = d.ms.checked_add(ms as i64)?;
        } else {
            let n: i64 = num.parse().ok()?;
            match (in_time, c) {
                (false, 'Y') => d.months = d.months.checked_add(n.checked_mul(12)?)?,
                (false, 'M') => d.months = d.months.checked_add(n)?,
                (false, 'W') => d.days = d.days.checked_add(n.checked_mul(7)?)?,
                (false, 'D') => d.days = d.days.checked_add(n)?,
                (true, 'H') => d.ms = d.ms.checked_add(n.checked_mul(3_600_000)?)?,
                (true, 'M') => d.ms = d.ms.checked_add(n.checked_mul(60_000)?)?,
                _ => return None,
            }
        }
        num.clear();
        any = true;
        time_any |= in_time;
    }
    if !num.is_empty() || !any || (in_time && !time_any) {
        return None;
    }
    if neg {
        d.months = d.months.checked_neg()?;
        d.days = d.days.checked_neg()?;
        d.ms = d.ms.checked_neg()?;
    }
    Some(d)
}

fn apply_duration(dt: DateTime<Utc>, d: IsoDuration, tz: Tz) -> DbResult<DateTime<Utc>> {
    let mut out = dt;
    if d.months != 0 {
        out = add_calendar(out, d.months, Unit::Month, tz)?;
    }
    if d.days != 0 {
        out = add_calendar(out, d.days, Unit::Day, tz)?;
    }
    if d.ms != 0 {
        out = add_calendar(out, d.ms, Unit::Millisecond, tz)?;
    }
    Ok(out)
}

fn rfc3339_ms(dt: DateTime<Utc>) -> Value {
    Value::String(dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn local_iso(naive: NaiveDateTime) -> String {
    naive.format("%Y-%m-%dT%H:%M:%S%.3f").to_string()
}

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "NOW" | "DATE_NOW" => Ok(Some(Value::Number(serde_json::Number::from(
            Utc::now().timestamp_millis(),
        )))),
        "NOW_ISO" | "DATE_NOW_ISO" => Ok(Some(rfc3339_ms(Utc::now()))),
        // One UUID implementation: phonetic/id.rs.
        "UUIDV4" => crate::sdbql::executor::phonetic::id::evaluate("UUID_V4", args),
        "UUIDV7" => crate::sdbql::executor::phonetic::id::evaluate("UUID_V7", args),
        "DATE_YEAR" => extract(name, args, |dt| i64::from(dt.year())),
        "DATE_MONTH" => extract(name, args, |dt| i64::from(dt.month())),
        "DATE_DAY" => extract(name, args, |dt| i64::from(dt.day())),
        "DATE_HOUR" => extract(name, args, |dt| i64::from(dt.hour())),
        "DATE_MINUTE" => extract(name, args, |dt| i64::from(dt.minute())),
        "DATE_SECOND" => extract(name, args, |dt| i64::from(dt.second())),
        "DATE_MILLISECOND" => extract(name, args, |dt| i64::from(dt.timestamp_subsec_millis())),
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
            if args.is_empty() || args.len() > 2 {
                return Err(arity(name, "1-2: date, [timezone]"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let tz = opt_tz(name, args, 1)?;
            let y = parse_datetime(&args[0])?.with_timezone(&tz).year();
            Ok(Some(Value::Bool(
                NaiveDate::from_ymd_opt(y, 2, 29).is_some(),
            )))
        }
        "DATE_COMPARE" => date_compare(args),
        "DATE_ISO8601" => {
            if args.len() >= 3 {
                return Ok(Some(rfc3339_ms(from_components(name, args)?)));
            }
            if args.len() != 1 {
                return Err(arity(
                    name,
                    "1 (date) or 3-7 (year, month, day, [hour], [minute], [second], [ms])",
                ));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            Ok(Some(rfc3339_ms(parse_datetime(&args[0])?)))
        }
        "DATE_TIMESTAMP" => {
            let dt = if args.len() >= 3 {
                from_components(name, args)?
            } else if args.len() == 1 {
                if args[0].is_null() {
                    return Ok(Some(Value::Null));
                }
                parse_datetime(&args[0])?
            } else {
                return Err(arity(
                    name,
                    "1 (date) or 3-7 (year, month, day, [hour], [minute], [second], [ms])",
                ));
            };
            Ok(Some(Value::Number(serde_json::Number::from(
                dt.timestamp_millis(),
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
            let tz = opt_tz(name, args, 2)?;
            // `to_string()` panics when the format string holds an invalid
            // specifier (the Display impl returns an error); write instead.
            use std::fmt::Write as _;
            let mut out = String::new();
            write!(out, "{}", dt.with_timezone(&tz).format(fmt)).map_err(|_| {
                DbError::ExecutionError(format!("DATE_FORMAT: invalid format string '{}'", fmt))
            })?;
            Ok(Some(Value::String(out)))
        }
        "DATE_TRUNC" => date_trunc(name, args),
        "DATE_ROUND" => date_round(args),
        "DATE_ADD" => date_add_args("DATE_ADD", args, 1),
        "DATE_SUBTRACT" | "DATE_SUB" => date_add_args("DATE_SUBTRACT", args, -1),
        "DATE_DIFF" => date_diff(args),
        "DATE_UTCTOLOCAL" => utc_to_local(args),
        "DATE_LOCALTOUTC" => local_to_utc(args),
        "DATE_TIMEZONE" => {
            if !args.is_empty() {
                return Err(arity(name, "0"));
            }
            Ok(Some(Value::String(server_timezone().to_string())))
        }
        "DATE_TIMEZONES" => {
            if !args.is_empty() {
                return Err(arity(name, "0"));
            }
            Ok(Some(Value::Array(
                chrono_tz::TZ_VARIANTS
                    .iter()
                    .map(|tz| Value::String(tz.name().to_string()))
                    .collect(),
            )))
        }
        "TIME_BUCKET" => time_bucket(args),
        "HUMAN_TIME" => human_time(args),
        _ => Ok(None),
    }
}

/// `DATE_YEAR(date, [timezone])` and friends.
fn extract(
    name: &str,
    args: &[Value],
    f: impl FnOnce(DateTime<Tz>) -> i64,
) -> DbResult<Option<Value>> {
    if args.is_empty() || args.len() > 2 {
        return Err(arity(name, "1-2: date, [timezone]"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let tz = opt_tz(name, args, 1)?;
    let dt = parse_datetime(&args[0])?.with_timezone(&tz);
    Ok(Some(Value::Number(serde_json::Number::from(f(dt)))))
}

fn int_component(fname: &str, what: &str, v: Option<&Value>, default: i64) -> DbResult<i64> {
    match v {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_i64()
            .or_else(|| {
                v.as_f64()
                    .filter(|f| f.fract() == 0.0 && f.is_finite())
                    .map(|f| f as i64)
            })
            .ok_or_else(|| DbError::ExecutionError(format!("{fname}: {what} must be an integer"))),
    }
}

/// `(year, month, day, [hour], [minute], [second], [millisecond])`, UTC.
fn from_components(fname: &str, args: &[Value]) -> DbResult<DateTime<Utc>> {
    if args.len() > 7 {
        return Err(arity(
            fname,
            "3-7: year, month, day, [hour], [minute], [second], [millisecond]",
        ));
    }
    let names = [
        "year",
        "month",
        "day",
        "hour",
        "minute",
        "second",
        "millisecond",
    ];
    let mut c = [0i64; 7];
    for (i, what) in names.iter().enumerate() {
        c[i] = int_component(fname, what, args.get(i), 0)?;
    }
    let bad = || DbError::ExecutionError(format!("{fname}: invalid date components"));
    let year = i32::try_from(c[0]).map_err(|_| bad())?;
    let date = NaiveDate::from_ymd_opt(
        year,
        u32::try_from(c[1]).map_err(|_| bad())?,
        u32::try_from(c[2]).map_err(|_| bad())?,
    )
    .ok_or_else(bad)?;
    let time = date
        .and_hms_milli_opt(
            u32::try_from(c[3]).map_err(|_| bad())?,
            u32::try_from(c[4]).map_err(|_| bad())?,
            u32::try_from(c[5]).map_err(|_| bad())?,
            u32::try_from(c[6]).map_err(|_| bad())?,
        )
        .ok_or_else(bad)?;
    Ok(time.and_utc())
}

fn date_add_args(fname: &str, args: &[Value], sign: i64) -> DbResult<Option<Value>> {
    if args.len() < 2 || args.len() > 4 {
        return Err(arity(
            fname,
            "3-4: date, amount, unit, [timezone] — or 2-3: date, isoDuration, [timezone]",
        ));
    }
    if args[0].is_null() || args[1].is_null() {
        return Ok(Some(Value::Null));
    }
    let dt = parse_datetime(&args[0])?;

    // DATE_ADD(date, "P1DT2H", [timezone])
    if let Value::String(s) = &args[1] {
        if args.len() > 3 {
            return Err(arity(fname, "2-3: date, isoDuration, [timezone]"));
        }
        let d = parse_iso_duration(s).ok_or_else(|| {
            DbError::ExecutionError(format!(
                "{fname}: '{s}' is not an ISO 8601 duration (like 'P1DT2H'); \
                 pass a number and a unit instead"
            ))
        })?;
        let d = if sign < 0 {
            IsoDuration {
                months: d.months.checked_neg().ok_or_else(out_of_range)?,
                days: d.days.checked_neg().ok_or_else(out_of_range)?,
                ms: d.ms.checked_neg().ok_or_else(out_of_range)?,
            }
        } else {
            d
        };
        let tz = opt_tz(fname, args, 2)?;
        return Ok(Some(rfc3339_ms(apply_duration(dt, d, tz)?)));
    }

    if args.len() < 3 {
        return Err(arity(fname, "3-4: date, amount, unit, [timezone]"));
    }
    let amount = args[1]
        .as_i64()
        .or_else(|| args[1].as_f64().map(|f| f as i64))
        .ok_or_else(|| DbError::ExecutionError(format!("{fname}: amount must be a number")))?;
    let unit = unit_arg(fname, &args[2])?;
    let tz = opt_tz(fname, args, 3)?;
    let amount = amount.checked_mul(sign).ok_or_else(out_of_range)?;
    Ok(Some(rfc3339_ms(add_calendar(dt, amount, unit, tz)?)))
}

fn truncate_to(dt: DateTime<Utc>, unit: Unit, tz: Tz) -> DbResult<DateTime<Utc>> {
    let local = dt.with_timezone(&tz);
    let midnight = |d: NaiveDate| d.and_hms_opt(0, 0, 0).expect("midnight is always valid");
    let naive = match unit {
        Unit::Year => midnight(
            NaiveDate::from_ymd_opt(local.year(), 1, 1)
                .ok_or_else(|| DbError::ExecutionError("DATE_TRUNC: invalid year".to_string()))?,
        ),
        Unit::Month => midnight(
            NaiveDate::from_ymd_opt(local.year(), local.month(), 1)
                .ok_or_else(|| DbError::ExecutionError("DATE_TRUNC: invalid month".to_string()))?,
        ),
        Unit::Week => {
            let wd = local.weekday().num_days_from_monday();
            midnight(
                local
                    .date_naive()
                    .checked_sub_signed(Duration::days(i64::from(wd)))
                    .ok_or_else(|| {
                        DbError::ExecutionError("DATE_TRUNC: week start out of range".to_string())
                    })?,
            )
        }
        Unit::Day => midnight(local.date_naive()),
        Unit::Hour => local
            .date_naive()
            .and_hms_opt(local.hour(), 0, 0)
            .expect("valid hour"),
        Unit::Minute => local
            .date_naive()
            .and_hms_opt(local.hour(), local.minute(), 0)
            .expect("valid minute"),
        Unit::Second => local
            .date_naive()
            .and_hms_opt(local.hour(), local.minute(), local.second())
            .expect("valid second"),
        Unit::Millisecond => {
            let ms = dt.timestamp_millis();
            return DateTime::from_timestamp_millis(ms).ok_or_else(|| {
                DbError::ExecutionError("DATE_TRUNC: date out of range".to_string())
            });
        }
    };
    Ok(resolve_local(tz, naive, Some(local.offset().fix()))?.with_timezone(&Utc))
}

fn date_trunc(fname: &str, args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() < 2 || args.len() > 3 {
        return Err(arity(fname, "2-3: date, unit, [timezone]"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let dt = parse_datetime(&args[0])?;
    let unit = unit_arg(fname, &args[1])?;
    let tz = opt_tz(fname, args, 2)?;
    Ok(Some(rfc3339_ms(truncate_to(dt, unit, tz)?)))
}

/// `DATE_ROUND(date, amount, unit)` — AQL: round *down* to a multiple of
/// `amount` units (d, h, i, s, f) since the epoch. The two-argument form
/// `DATE_ROUND(date, unit, [timezone])` is kept as an alias of DATE_TRUNC.
fn date_round(args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() < 2 || args.len() > 3 {
        return Err(arity(
            "DATE_ROUND",
            "3: date, amount, unit (or 2-3: date, unit, [timezone])",
        ));
    }
    if !args[1].is_number() {
        return date_trunc("DATE_ROUND", args);
    }
    if args.len() != 3 {
        return Err(arity("DATE_ROUND", "3: date, amount, unit"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let dt = parse_datetime(&args[0])?;
    let amount = args[1].as_i64().filter(|n| *n > 0).ok_or_else(|| {
        DbError::ExecutionError("DATE_ROUND: amount must be a positive integer".to_string())
    })?;
    let unit = unit_arg("DATE_ROUND", &args[2])?;
    let unit_ms = match unit {
        Unit::Day | Unit::Hour | Unit::Minute | Unit::Second | Unit::Millisecond => {
            unit.fixed_ms().expect("fixed unit")
        }
        _ => {
            return Err(DbError::ExecutionError(
                "DATE_ROUND: unit must be one of d, h, i, s, f".to_string(),
            ))
        }
    };
    let step = amount
        .checked_mul(unit_ms)
        .ok_or_else(|| DbError::ExecutionError("DATE_ROUND: amount is too large".to_string()))?;
    let ms = dt.timestamp_millis();
    let rounded = ms.div_euclid(step) * step;
    let out = DateTime::from_timestamp_millis(rounded)
        .ok_or_else(|| DbError::ExecutionError("DATE_ROUND: date out of range".to_string()))?;
    Ok(Some(rfc3339_ms(out)))
}

/// `DATE_COMPARE(date1, date2, unitRangeStart, [unitRangeEnd])` (AQL): true
/// when the two dates agree on every component from `unitRangeStart` down to
/// `unitRangeEnd` (default: `unitRangeStart`).
fn date_compare(args: &[Value]) -> DbResult<Option<Value>> {
    // The original two-argument form (-1 / 0 / 1) predates the AQL one and
    // is kept so existing queries keep working.
    if args.len() == 2 {
        if args[0].is_null() || args[1].is_null() {
            return Ok(Some(Value::Null));
        }
        let cmp = parse_datetime(&args[0])?.cmp(&parse_datetime(&args[1])?);
        return Ok(Some(Value::Number(serde_json::Number::from(
            cmp as i8 as i64,
        ))));
    }
    if args.len() < 3 || args.len() > 4 {
        return Err(arity(
            "DATE_COMPARE",
            "2 (date1, date2) or 3-4 (date1, date2, unitRangeStart, [unitRangeEnd])",
        ));
    }
    if args[0].is_null() || args[1].is_null() {
        return Ok(Some(Value::Null));
    }
    let a = parse_datetime(&args[0])?;
    let b = parse_datetime(&args[1])?;
    let start = unit_arg("DATE_COMPARE", &args[2])?;
    let end = match args.get(3) {
        None | Some(Value::Null) => start,
        Some(v) => unit_arg("DATE_COMPARE", v)?,
    };
    if start == Unit::Week || end == Unit::Week {
        return Err(DbError::ExecutionError(
            "DATE_COMPARE: units are y, m, d, h, i, s, f".to_string(),
        ));
    }
    if end < start {
        return Err(DbError::ExecutionError(
            "DATE_COMPARE: unitRangeEnd must not be larger than unitRangeStart".to_string(),
        ));
    }
    let field = |dt: &DateTime<Utc>, u: Unit| -> i64 {
        match u {
            Unit::Year => i64::from(dt.year()),
            Unit::Month => i64::from(dt.month()),
            Unit::Week => 0,
            Unit::Day => i64::from(dt.day()),
            Unit::Hour => i64::from(dt.hour()),
            Unit::Minute => i64::from(dt.minute()),
            Unit::Second => i64::from(dt.second()),
            Unit::Millisecond => i64::from(dt.timestamp_subsec_millis()),
        }
    };
    let units = [
        Unit::Year,
        Unit::Month,
        Unit::Day,
        Unit::Hour,
        Unit::Minute,
        Unit::Second,
        Unit::Millisecond,
    ];
    let equal = units
        .iter()
        .filter(|u| **u >= start && **u <= end)
        .all(|u| field(&a, *u) == field(&b, *u));
    Ok(Some(Value::Bool(equal)))
}

/// Whole calendar months from `a` to `b` (truncated toward zero), plus the
/// fraction of the next month when `as_float`.
fn months_between(a: NaiveDateTime, b: NaiveDateTime, as_float: bool) -> DbResult<f64> {
    let mut whole = (i64::from(b.year()) * 12 + i64::from(b.month()))
        - (i64::from(a.year()) * 12 + i64::from(a.month()));
    let anchor = add_months_naive(a, whole)?;
    if whole > 0 && anchor > b {
        whole -= 1;
    } else if whole < 0 && anchor < b {
        whole += 1;
    }
    if !as_float {
        return Ok(whole as f64);
    }
    let anchor = add_months_naive(a, whole)?;
    let dir = if b >= anchor { 1 } else { -1 };
    let next = add_months_naive(a, whole + dir)?;
    let span = (next - anchor).num_milliseconds().abs();
    if span == 0 {
        return Ok(whole as f64);
    }
    let frac = (b - anchor).num_milliseconds() as f64 / span as f64;
    Ok(whole as f64 + frac)
}

/// `DATE_DIFF(date1, date2, unit, [asFloat], [tz1], [tz2])`.
///
/// Years and months count whole calendar months (2023-12-31 → 2024-01-01 is
/// 0 years). Days and weeks compare local calendar time in the zones given;
/// hours and smaller measure elapsed time. `tz2` defaults to `tz1`.
fn date_diff(args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() < 2 || args.len() > 6 {
        return Err(arity(
            "DATE_DIFF",
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
        Some(v) => unit_arg("DATE_DIFF", v)?,
    };
    let as_float = opt_bool("DATE_DIFF", "asFloat", args.get(3))?;
    let tz1 = opt_tz("DATE_DIFF", args, 4)?;
    let tz2 = match args.get(5) {
        None | Some(Value::Null) => tz1,
        Some(_) => opt_tz("DATE_DIFF", args, 5)?,
    };
    let la = a.with_timezone(&tz1).naive_local();
    let lb = b.with_timezone(&tz2).naive_local();
    let val = match unit {
        Unit::Year => months_between(la, lb, as_float)? / 12.0,
        Unit::Month => months_between(la, lb, as_float)?,
        Unit::Week | Unit::Day => {
            (lb - la).num_milliseconds() as f64 / unit.fixed_ms().expect("fixed unit") as f64
        }
        _ => {
            (b.timestamp_millis() - a.timestamp_millis()) as f64
                / unit.fixed_ms().expect("fixed unit") as f64
        }
    };
    if as_float {
        return Ok(Some(Value::Number(number_from_f64(val))));
    }
    Ok(Some(Value::Number(serde_json::Number::from(
        val.trunc() as i64
    ))))
}

fn zone_info(dt: &DateTime<Tz>) -> Value {
    use chrono_tz::{OffsetComponents, OffsetName};
    let off = dt.offset();
    serde_json::json!({
        "name": off.abbreviation().unwrap_or(""),
        "begin": Value::Null,
        "end": Value::Null,
        "dst": off.dst_offset().num_seconds() != 0,
        "offset": off.fix().local_minus_utc(),
    })
}

/// `DATE_UTCTOLOCAL(date, timezone, [zoneinfo])` → local wall-clock time,
/// ISO-8601 without an offset.
fn utc_to_local(args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() < 2 || args.len() > 3 {
        return Err(arity("DATE_UTCTOLOCAL", "2-3: date, timezone, [zoneinfo]"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let tz = match &args[1] {
        Value::String(s) => parse_tz(s)?,
        _ => {
            return Err(DbError::ExecutionError(
                "DATE_UTCTOLOCAL: timezone must be a string".to_string(),
            ))
        }
    };
    let local = parse_datetime(&args[0])?.with_timezone(&tz);
    let text = Value::String(local_iso(local.naive_local()));
    if !opt_bool("DATE_UTCTOLOCAL", "zoneinfo", args.get(2))? {
        return Ok(Some(text));
    }
    Ok(Some(serde_json::json!({
        "local": text,
        "tzdb": chrono_tz::IANA_TZDB_VERSION,
        "zoneInfo": zone_info(&local),
    })))
}

/// `DATE_LOCALTOUTC(date, timezone, [zoneinfo])`: read the wall-clock fields
/// of `date` as local time in `timezone` (any offset it carries is ignored)
/// and return the UTC instant.
fn local_to_utc(args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() < 2 || args.len() > 3 {
        return Err(arity("DATE_LOCALTOUTC", "2-3: date, timezone, [zoneinfo]"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let tz = match &args[1] {
        Value::String(s) => parse_tz(s)?,
        _ => {
            return Err(DbError::ExecutionError(
                "DATE_LOCALTOUTC: timezone must be a string".to_string(),
            ))
        }
    };
    let naive = parse_date_value(&args[0])?.naive_local();
    let local = resolve_local(tz, naive, None)?;
    let utc = rfc3339_ms(local.with_timezone(&Utc));
    if !opt_bool("DATE_LOCALTOUTC", "zoneinfo", args.get(2))? {
        return Ok(Some(utc));
    }
    Ok(Some(serde_json::json!({
        "utc": utc,
        "tzdb": chrono_tz::IANA_TZDB_VERSION,
        "zoneInfo": zone_info(&local),
    })))
}

/// The server's IANA timezone: `TZ`, then `/etc/timezone`, then the
/// `/etc/localtime` symlink target, else `UTC`. Read once.
fn server_timezone() -> &'static str {
    static TZ_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TZ_NAME.get_or_init(|| {
        let valid = |s: &str| {
            let s = s.trim().trim_start_matches(':');
            s.parse::<Tz>().ok().map(|tz| tz.name().to_string())
        };
        if let Some(tz) = std::env::var("TZ").ok().as_deref().and_then(valid) {
            return tz;
        }
        if let Some(tz) = std::fs::read_to_string("/etc/timezone")
            .ok()
            .as_deref()
            .and_then(valid)
        {
            return tz;
        }
        if let Ok(target) = std::fs::read_link("/etc/localtime") {
            let t = target.to_string_lossy();
            if let Some(idx) = t.find("zoneinfo/") {
                if let Some(tz) = valid(&t[idx + "zoneinfo/".len()..]) {
                    return tz;
                }
            }
        }
        "UTC".to_string()
    })
}

/// Default origin for week buckets: Monday 1970-01-05, so weeks start on
/// Monday (the epoch itself is a Thursday).
const WEEK_ORIGIN_MS: i64 = 4 * 86_400_000;

/// `TIME_BUCKET(time, interval, [options])`.
///
/// `interval`: `ms`, `s`, `m`, `h`, `d`, `w`, or calendar `mo` / `y`.
/// `options`: `{origin: date, timezone: "Europe/Paris"}` — buckets are
/// aligned to `origin` (default: the epoch; Monday 1970-01-05 for weeks;
/// January for years) and, with a timezone, to local wall-clock time.
/// A numeric `time` is milliseconds (no seconds heuristic) and yields a
/// number; a string yields an ISO-8601 string.
fn time_bucket(args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() < 2 || args.len() > 3 {
        return Err(arity(
            "TIME_BUCKET",
            "2-3: timestamp, interval (e.g. '5m'), [options {origin, timezone}]",
        ));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let interval_str = args[1].as_str().ok_or_else(|| {
        DbError::ExecutionError("TIME_BUCKET: interval must be a string".to_string())
    })?;
    let interval = parse_interval(interval_str)?;
    let unit_lc = interval_str.trim_end().to_ascii_lowercase();
    let week_unit =
        unit_lc.ends_with('w') || unit_lc.ends_with("week") || unit_lc.ends_with("weeks");

    let (tz, origin) = match args.get(2) {
        None | Some(Value::Null) => (None, None),
        Some(Value::Object(o)) => {
            let tz = match o.get("timezone").or_else(|| o.get("tz")) {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(parse_tz(s)?),
                Some(_) => {
                    return Err(DbError::ExecutionError(
                        "TIME_BUCKET: timezone must be a string".to_string(),
                    ))
                }
            };
            let origin = match o.get("origin") {
                None | Some(Value::Null) => None,
                Some(v) => Some(time_bucket_ms(v)?),
            };
            (tz, origin)
        }
        Some(_) => {
            return Err(DbError::ExecutionError(
                "TIME_BUCKET: options must be an object {origin, timezone}".to_string(),
            ))
        }
    };

    let ts = time_bucket_ms(&args[0])?;
    let out_of_range =
        || DbError::ExecutionError("TIME_BUCKET: timestamp out of range".to_string());
    let tz = tz.filter(|t| *t != chrono_tz::UTC);

    let bucket = match interval {
        Interval::Fixed(step) => {
            let default_origin = if week_unit { WEEK_ORIGIN_MS } else { 0 };
            let floor = |t: i64, o: i64| -> Option<i64> {
                let rel = t.checked_sub(o)?;
                rel.div_euclid(step).checked_mul(step)?.checked_add(o)
            };
            match tz {
                None => floor(ts, origin.unwrap_or(default_origin)).ok_or_else(out_of_range)?,
                Some(tz) => {
                    // Bucket on the local wall clock, then map back to an instant.
                    let local_ms = |t: i64| -> Option<i64> {
                        let dt = DateTime::from_timestamp_millis(t)?.with_timezone(&tz);
                        t.checked_add(i64::from(dt.offset().fix().local_minus_utc()) * 1000)
                    };
                    let o = match origin {
                        Some(o) => local_ms(o).ok_or_else(out_of_range)?,
                        None => default_origin,
                    };
                    let b = floor(local_ms(ts).ok_or_else(out_of_range)?, o)
                        .ok_or_else(out_of_range)?;
                    let naive = DateTime::from_timestamp_millis(b)
                        .ok_or_else(out_of_range)?
                        .naive_utc();
                    resolve_local(tz, naive, None)?.timestamp_millis()
                }
            }
        }
        Interval::Months(n) => {
            let tz = tz.unwrap_or(chrono_tz::UTC);
            let month_index = |t: i64| -> Option<i64> {
                let l = DateTime::from_timestamp_millis(t)?.with_timezone(&tz);
                Some(i64::from(l.year()) * 12 + i64::from(l.month()) - 1)
            };
            let origin_idx = match origin {
                Some(o) => month_index(o).ok_or_else(out_of_range)?,
                None => 1970 * 12,
            };
            let idx = month_index(ts).ok_or_else(out_of_range)?;
            let b = (idx - origin_idx).div_euclid(n) * n + origin_idx;
            let year = i32::try_from(b.div_euclid(12)).map_err(|_| out_of_range())?;
            let month = (b.rem_euclid(12) + 1) as u32;
            let naive = NaiveDate::from_ymd_opt(year, month, 1)
                .ok_or_else(out_of_range)?
                .and_hms_opt(0, 0, 0)
                .expect("midnight is always valid");
            resolve_local(tz, naive, None)?.timestamp_millis()
        }
    };

    if args[0].is_string() {
        Ok(Some(rfc3339_ms(
            DateTime::from_timestamp_millis(bucket).ok_or_else(|| {
                DbError::ExecutionError("TIME_BUCKET: failed to construct date".to_string())
            })?,
        )))
    } else {
        Ok(Some(Value::Number(serde_json::Number::from(bucket))))
    }
}

/// TIME_BUCKET's reading of a time: a number is milliseconds as-is.
fn time_bucket_ms(v: &Value) -> DbResult<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| {
                n.as_f64()
                    .filter(|f| f.is_finite() && f.abs() < 9.2e18)
                    .map(|f| f.floor() as i64)
            })
            .ok_or_else(|| {
                DbError::ExecutionError("TIME_BUCKET: timestamp must be a valid number".to_string())
            }),
        Value::String(_) => Ok(parse_datetime(v)?.timestamp_millis()),
        _ => Err(DbError::ExecutionError(
            "TIME_BUCKET: timestamp must be number or string".to_string(),
        )),
    }
}

/// `HUMAN_TIME(date, [now])`. Both arguments are dates in any accepted form
/// (the same seconds/milliseconds rule applies to both).
fn human_time(args: &[Value]) -> DbResult<Option<Value>> {
    if args.is_empty() || args.len() > 2 {
        return Err(arity("HUMAN_TIME", "1-2: date, [now]"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let now = match args.get(1) {
        None | Some(Value::Null) => Utc::now().timestamp_millis(),
        Some(v) => parse_datetime(v)?.timestamp_millis(),
    };
    let date_ts = parse_datetime(&args[0])?.timestamp_millis();
    let diff_secs = now
        .checked_sub(date_ts)
        .ok_or_else(|| DbError::ExecutionError("HUMAN_TIME: timestamp out of range".to_string()))?
        / 1000;
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
    fn more_iso_formats() {
        for s in [
            "2024-06-15T10:20:30Z",
            "2024-06-15T10:20:30.123Z",
            "2024-06-15 10:20:30",
            "2024-06-15T10:20",
            "2024-06-15T10:20Z",
            "2024-06-15T12:20:30+02:00",
            "2024-06-15T12:20:30+0200",
            "2024-06-15T12:20:30+02",
            " 2024-06-15T10:20:30Z ",
        ] {
            assert_eq!(call("DATE_HOUR", &[json!(s)]), json!(10), "{s}");
        }
        assert!(evaluate("DATE_HOUR", &[json!("2024-06-15X10")]).is_err());
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
    fn compare_is_aql_shaped() {
        assert_eq!(
            call(
                "DATE_COMPARE",
                &[
                    json!("2020-01-01T10:00:00Z"),
                    json!("2020-01-01T23:00:00Z"),
                    json!("y"),
                    json!("d")
                ]
            ),
            json!(true)
        );
        assert_eq!(
            call(
                "DATE_COMPARE",
                &[
                    json!("2020-01-01T10:00:00Z"),
                    json!("2020-01-01T23:00:00Z"),
                    json!("h")
                ]
            ),
            json!(false)
        );
        assert_eq!(
            call(
                "DATE_COMPARE",
                &[
                    json!("2019-05-01T10:00:00Z"),
                    json!("2020-05-01T23:00:00Z"),
                    json!("m"),
                    json!("d")
                ]
            ),
            json!(true)
        );
        // The legacy two-argument form orders the dates.
        assert_eq!(
            evaluate("DATE_COMPARE", &[json!("2020-01-01"), json!("2020-01-02")]).unwrap(),
            Some(json!(-1))
        );
        assert!(evaluate(
            "DATE_COMPARE",
            &[
                json!("2020-01-01"),
                json!("2020-01-02"),
                json!("d"),
                json!("y")
            ]
        )
        .is_err());
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
    fn diff_counts_calendar_months() {
        let d = |a: &str, b: &str, u: &str| call("DATE_DIFF", &[json!(a), json!(b), json!(u)]);
        assert_eq!(d("2023-12-31", "2024-01-01", "years"), json!(0));
        assert_eq!(d("2000-06-15", "2024-06-14", "years"), json!(23));
        assert_eq!(d("2000-06-15", "2024-06-15", "years"), json!(24));
        assert_eq!(d("2024-01-31", "2024-02-29", "months"), json!(1));
        assert_eq!(d("2024-03-15", "2024-01-20", "months"), json!(-1));
        assert_eq!(
            d("2020-01-01T00:00:00Z", "2020-01-11T00:00:00Z", "days"),
            json!(10)
        );
        let f = call(
            "DATE_DIFF",
            &[
                json!("2024-01-01"),
                json!("2024-01-16T12:00:00Z"),
                json!("months"),
                json!(true),
            ],
        );
        assert!((f.as_f64().unwrap() - 0.5).abs() < 1e-9, "{f}");
        let half_day = call(
            "DATE_DIFF",
            &[
                json!("2025-06-15T00:00:00Z"),
                json!("2025-06-15T12:00:00Z"),
                json!("days"),
                json!(true),
            ],
        );
        assert_eq!(half_day, json!(0.5));
        assert!(evaluate(
            "DATE_DIFF",
            &[json!("2024-01-01"), json!("2024-01-02"), json!("fortnight")]
        )
        .is_err());
    }

    #[test]
    fn diff_with_timezones() {
        // Both are 2024-03-01 local in their zones: same calendar day.
        let r = call(
            "DATE_DIFF",
            &[
                json!("2024-03-01T23:00:00Z"),
                json!("2024-03-01T05:00:00Z"),
                json!("days"),
                json!(false),
                json!("Asia/Tokyo"),
                json!("America/New_York"),
            ],
        );
        // Tokyo: 2024-03-02 08:00; New York: 2024-03-01 00:00 → -1 day (truncated)
        assert_eq!(r, json!(-1));
        // A calendar day across a DST change is still 1 day.
        let dst = call(
            "DATE_DIFF",
            &[
                json!("2024-03-30T12:00:00+01:00"),
                json!("2024-03-31T12:00:00+02:00"),
                json!("days"),
                json!(true),
                json!("Europe/Paris"),
            ],
        );
        assert_eq!(dst, json!(1.0));
    }

    #[test]
    fn dst_add_days_keeps_wall_clock_and_gaps_skip_forward() {
        // Europe/Paris springs forward on 2024-03-31 at 02:00 → 03:00.
        let r = call(
            "DATE_ADD",
            &[
                json!("2024-03-30T11:00:00Z"),
                json!(1),
                json!("day"),
                json!("Europe/Paris"),
            ],
        );
        assert_eq!(r, json!("2024-03-31T10:00:00.000Z"));
        // 02:30 local does not exist on that day: moves to 03:30 (01:30Z).
        let r = call(
            "DATE_ADD",
            &[
                json!("2024-03-30T01:30:00Z"),
                json!(1),
                json!("day"),
                json!("Europe/Paris"),
            ],
        );
        assert_eq!(r, json!("2024-03-31T01:30:00.000Z"));
        // Overlap (falls back 03:00 → 02:00 on 2024-10-27): earliest instant.
        let r = call(
            "DATE_LOCALTOUTC",
            &[json!("2024-10-27T02:30:00"), json!("Europe/Paris")],
        );
        assert_eq!(r, json!("2024-10-27T00:30:00.000Z"));
    }

    #[test]
    fn utc_local_round_trip() {
        assert_eq!(
            call(
                "DATE_UTCTOLOCAL",
                &[json!("2020-03-15T00:00:00.000Z"), json!("Europe/Berlin")]
            ),
            json!("2020-03-15T01:00:00.000")
        );
        assert_eq!(
            call(
                "DATE_LOCALTOUTC",
                &[json!("2020-03-15T01:00:00.000"), json!("Europe/Berlin")]
            ),
            json!("2020-03-15T00:00:00.000Z")
        );
        let info = call(
            "DATE_UTCTOLOCAL",
            &[
                json!("2021-07-01T12:00:00Z"),
                json!("Europe/Paris"),
                json!(true),
            ],
        );
        assert_eq!(info["zoneInfo"]["dst"], json!(true));
        assert_eq!(info["zoneInfo"]["offset"], json!(7200));
        assert!(evaluate("DATE_UTCTOLOCAL", &[json!("2021-07-01"), json!(1)]).is_err());
        let zones = call("DATE_TIMEZONES", &[]);
        assert!(zones
            .as_array()
            .unwrap()
            .iter()
            .any(|z| z == "Europe/Paris"));
        assert!(call("DATE_TIMEZONE", &[]).is_string());
    }

    #[test]
    fn timezone_on_components() {
        assert_eq!(
            call(
                "DATE_HOUR",
                &[json!("2024-01-01T15:00:00Z"), json!("America/New_York")]
            ),
            json!(10)
        );
        assert_eq!(
            call(
                "DATE_DAY",
                &[json!("2024-01-01T23:30:00Z"), json!("Asia/Tokyo")]
            ),
            json!(2)
        );
        assert!(evaluate("DATE_HOUR", &[json!("2024-01-01T15:00:00Z"), json!(5)]).is_err());
        assert!(evaluate(
            "DATE_FORMAT",
            &[json!("2024-01-01"), json!("%Y"), json!(true)]
        )
        .is_err());
    }

    #[test]
    fn iso_durations() {
        assert_eq!(
            call(
                "DATE_ADD",
                &[json!("2024-01-31T00:00:00Z"), json!("P1M2DT3H")]
            ),
            json!("2024-03-02T03:00:00.000Z")
        );
        assert_eq!(
            call(
                "DATE_SUBTRACT",
                &[json!("2024-01-01T00:00:00Z"), json!("PT1.5S")]
            ),
            json!("2023-12-31T23:59:58.500Z")
        );
        assert_eq!(
            call("DATE_ADD", &[json!("2024-01-01T00:00:00Z"), json!("-P1W")]),
            json!("2023-12-25T00:00:00.000Z")
        );
        for bad in ["P", "PT", "P1H", "1D", "P1.5D", "PT1M1H", "P1DT"] {
            assert!(
                evaluate("DATE_ADD", &[json!("2024-01-01"), json!(bad)]).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn components_form() {
        assert_eq!(
            call(
                "DATE_ISO8601",
                &[json!(2024), json!(2), json!(29), json!(13), json!(5)]
            ),
            json!("2024-02-29T13:05:00.000Z")
        );
        assert_eq!(
            call("DATE_TIMESTAMP", &[json!(1970), json!(1), json!(2)]),
            json!(86_400_000)
        );
        assert!(evaluate("DATE_ISO8601", &[json!(2023), json!(2), json!(29)]).is_err());
    }

    #[test]
    fn round_aql_form_and_legacy_truncate() {
        assert_eq!(
            call(
                "DATE_ROUND",
                &[json!("2024-06-15T12:37:10Z"), json!(15), json!("minutes")]
            ),
            json!("2024-06-15T12:30:00.000Z")
        );
        assert_eq!(
            call("DATE_ROUND", &[json!("2024-06-15T12:30:00Z"), json!("day")]),
            json!("2024-06-15T00:00:00.000Z")
        );
        assert!(evaluate(
            "DATE_ROUND",
            &[json!("2024-06-15"), json!(1), json!("month")]
        )
        .is_err());
    }

    #[test]
    fn now_iso_has_ms_and_z() {
        let s = call("NOW_ISO", &[]);
        let s = s.as_str().unwrap();
        assert!(s.ends_with('Z'), "{s}");
        assert_eq!(s.len(), "2024-01-01T00:00:00.000Z".len());
    }

    #[test]
    fn time_bucket_units_and_options() {
        assert_eq!(
            call("TIME_BUCKET", &[json!(1234), json!("500ms")]),
            json!(1000)
        );
        // 1970-01-08 (Thursday) → week starting Monday 1970-01-05.
        assert_eq!(
            call("TIME_BUCKET", &[json!("1970-01-08T12:00:00Z"), json!("1w")]),
            json!("1970-01-05T00:00:00.000Z")
        );
        assert_eq!(
            call(
                "TIME_BUCKET",
                &[json!("2024-05-17T12:00:00Z"), json!("1mo")]
            ),
            json!("2024-05-01T00:00:00.000Z")
        );
        assert_eq!(
            call(
                "TIME_BUCKET",
                &[json!("2024-05-17T12:00:00Z"), json!("3mo")]
            ),
            json!("2024-04-01T00:00:00.000Z")
        );
        assert_eq!(
            call("TIME_BUCKET", &[json!("2024-05-17T12:00:00Z"), json!("1y")]),
            json!("2024-01-01T00:00:00.000Z")
        );
        assert_eq!(
            call(
                "TIME_BUCKET",
                &[
                    json!("2024-05-17T01:00:00Z"),
                    json!("1d"),
                    json!({"timezone": "America/New_York"})
                ]
            ),
            json!("2024-05-16T04:00:00.000Z")
        );
        assert_eq!(
            call(
                "TIME_BUCKET",
                &[json!(17_000), json!("10s"), json!({"origin": 5_000})]
            ),
            json!(15_000)
        );
        assert!(evaluate("TIME_BUCKET", &[json!(1), json!("é")]).is_err());
        assert!(evaluate("TIME_BUCKET", &[json!(1), json!("0s")]).is_err());
    }

    #[test]
    fn human_time_parses_string_now() {
        assert_eq!(
            call(
                "HUMAN_TIME",
                &[json!("2025-06-15T14:30:00Z"), json!("2025-06-15T14:35:00Z")]
            ),
            json!("5 minutes ago")
        );
        assert_eq!(
            call(
                "HUMAN_TIME",
                &[json!("2025-06-15T16:40:00Z"), json!("2025-06-15T14:35:00Z")]
            ),
            json!("in 2 hours")
        );
    }

    #[test]
    fn date_add_overflow_is_an_error_not_a_panic() {
        // Audit A9: each of these used to panic inside chrono.
        for unit in [
            "day", "week", "hour", "minute", "second", "ms", "month", "year",
        ] {
            let r = evaluate(
                "DATE_ADD",
                &[json!("2024-01-01T00:00:00Z"), json!(1e17), json!(unit)],
            );
            assert!(r.is_err(), "unit {unit} should overflow");
        }
        let r = evaluate(
            "DATE_ADD",
            &[json!("2024-01-01T00:00:00Z"), json!(i64::MAX), json!("day")],
        );
        assert!(r.is_err());
        // `amount * sign` with i64::MIN.
        let r = evaluate(
            "DATE_SUBTRACT",
            &[json!("2024-01-01T00:00:00Z"), json!(i64::MIN), json!("ms")],
        );
        assert!(r.is_err());
        let r = evaluate(
            "DATE_ADD",
            &[
                json!("2024-01-01T00:00:00Z"),
                json!(i64::MAX),
                json!("year"),
            ],
        );
        assert!(r.is_err());
    }

    #[test]
    fn date_add_normal_amounts_still_work() {
        let r = call(
            "DATE_ADD",
            &[json!("2024-01-01T00:00:00Z"), json!(10), json!("day")],
        );
        assert!(r.as_str().unwrap().starts_with("2024-01-11"));
        let r = call(
            "DATE_SUBTRACT",
            &[json!("2024-01-01T00:00:00Z"), json!(1), json!("hour")],
        );
        assert!(r.as_str().unwrap().starts_with("2023-12-31T23:00:00"));
        assert!(evaluate(
            "DATE_ADD",
            &[json!("2024-01-01"), json!(1), json!("fortnight")]
        )
        .is_err());
    }

    #[test]
    fn other_overflows_are_errors() {
        assert!(evaluate("DATE_FORMAT", &[json!("2024-01-01T00:00:00Z"), json!("%Q")]).is_err());
        // Must not panic, whichever way it rounds.
        let _ = evaluate("TIME_BUCKET", &[json!(i64::MIN), json!("7d")]);
        assert!(evaluate(
            "TIME_BUCKET",
            &[json!(1_700_000_000_000_i64), json!("99999999999999999d")]
        )
        .is_err());
        assert!(evaluate(
            "HUMAN_TIME",
            &[json!("2024-01-01T00:00:00Z"), json!(i64::MIN)]
        )
        .is_err());
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
