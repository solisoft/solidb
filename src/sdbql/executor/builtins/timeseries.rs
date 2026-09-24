//! Time-series array helpers: DELTA, RATE, FILL, RESAMPLE, plus the single
//! interval parser shared with TIME_BUCKET, ASOF JOIN tolerances and
//! MATCH_SEQ `within`.
//!
//! Timestamps here are always milliseconds: the seconds heuristic of the
//! date functions (`utils::SECONDS_EPOCH_THRESHOLD`) does not apply.

use crate::error::{DbError, DbResult};
use serde_json::{json, Value};

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "DELTA" => Ok(Some(delta(args)?)),
        "RATE" => Ok(Some(rate(args)?)),
        "FILL" => Ok(Some(fill(args)?)),
        "RESAMPLE" => Ok(Some(resample(args)?)),
        _ => Ok(None),
    }
}

/// A parsed interval string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interval {
    /// A fixed length in milliseconds (`ms`, `s`, `m`, `h`, `d`, `w`).
    Fixed(i64),
    /// A calendar length in months (`mo`, `y`); only TIME_BUCKET accepts it.
    Months(i64),
}

const INTERVAL_HELP: &str = "expected a positive integer and a unit, like '500ms', '30s', '5m', \
     '1h', '1d', '1w' (TIME_BUCKET also takes '1mo' and '1y')";

/// The one interval parser. Splits on characters, not bytes, so a non-ASCII
/// unit is an error rather than a panic, and rejects a value ≤ 0.
pub fn parse_interval(interval_str: &str) -> DbResult<Interval> {
    let s = interval_str.trim();
    let split = s
        .char_indices()
        .find(|(i, c)| !(c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+'))))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let unit = unit.trim();
    if num.is_empty() || unit.is_empty() {
        return Err(DbError::ExecutionError(format!(
            "interval '{interval_str}': {INTERVAL_HELP}"
        )));
    }
    let val: i64 = num.parse().map_err(|_| {
        DbError::ExecutionError(format!(
            "interval '{interval_str}': invalid number ({INTERVAL_HELP})"
        ))
    })?;
    if val <= 0 {
        return Err(DbError::ExecutionError(format!(
            "interval '{interval_str}': must be greater than 0"
        )));
    }
    let too_large =
        || DbError::ExecutionError(format!("interval '{interval_str}': value is too large"));
    let fixed = |per: i64| {
        val.checked_mul(per)
            .map(Interval::Fixed)
            .ok_or_else(too_large)
    };
    // Short units are case-sensitive ('m' is minutes); long names are not.
    match unit {
        "ms" => return fixed(1),
        "s" => return fixed(1_000),
        "m" => return fixed(60_000),
        "h" => return fixed(3_600_000),
        "d" => return fixed(86_400_000),
        "w" => return fixed(604_800_000),
        "mo" => return Ok(Interval::Months(val)),
        "y" => {
            return val
                .checked_mul(12)
                .map(Interval::Months)
                .ok_or_else(too_large)
        }
        _ => {}
    }
    match unit.to_ascii_lowercase().as_str() {
        "millisecond" | "milliseconds" => fixed(1),
        "sec" | "secs" | "second" | "seconds" => fixed(1_000),
        "min" | "mins" | "minute" | "minutes" => fixed(60_000),
        "hour" | "hours" => fixed(3_600_000),
        "day" | "days" => fixed(86_400_000),
        "week" | "weeks" => fixed(604_800_000),
        "month" | "months" => Ok(Interval::Months(val)),
        "year" | "years" => val
            .checked_mul(12)
            .map(Interval::Months)
            .ok_or_else(too_large),
        _ => Err(DbError::ExecutionError(format!(
            "interval '{interval_str}': unknown unit '{unit}' ({INTERVAL_HELP})"
        ))),
    }
}

/// A fixed-length interval in milliseconds. Calendar units are refused:
/// a month has no fixed length.
pub fn parse_interval_ms(interval_str: &str) -> DbResult<i64> {
    match parse_interval(interval_str)? {
        Interval::Fixed(ms) => Ok(ms),
        Interval::Months(_) => Err(DbError::ExecutionError(format!(
            "interval '{interval_str}': months and years have no fixed length here; \
             use days or weeks"
        ))),
    }
}

/// `t` / `ts` / `time` of a series object, as integer milliseconds (floats
/// are floored).
fn point_time(o: &serde_json::Map<String, Value>) -> Option<i64> {
    o.get("t")
        .or_else(|| o.get("ts"))
        .or_else(|| o.get("time"))
        .and_then(|x| {
            x.as_i64().or_else(|| {
                x.as_f64()
                    .filter(|f| f.is_finite())
                    .map(|f| f.floor() as i64)
            })
        })
}

fn point_value(o: &serde_json::Map<String, Value>) -> Option<f64> {
    o.get("v")
        .or_else(|| o.get("value"))
        .and_then(Value::as_f64)
}

fn series_points(v: &Value) -> DbResult<Vec<(i64, f64)>> {
    let arr = v.as_array().ok_or_else(|| {
        DbError::ExecutionError("time-series function expects an array".to_string())
    })?;
    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        match item {
            Value::Number(n) => {
                let y = n.as_f64().ok_or_else(|| {
                    DbError::ExecutionError("time-series: invalid number".to_string())
                })?;
                out.push((i as i64, y));
            }
            Value::Object(o) => {
                let t = point_time(o).unwrap_or(i as i64);
                let y = point_value(o).ok_or_else(|| {
                    DbError::ExecutionError("time-series object needs numeric v/value".to_string())
                })?;
                out.push((t, y));
            }
            Value::Null => {}
            _ => {
                return Err(DbError::ExecutionError(
                    "time-series items must be numbers or {t,v} objects".to_string(),
                ))
            }
        }
    }
    // Series usually arrive in time order; only sort when they don't.
    if !out.windows(2).all(|w| w[0].0 <= w[1].0) {
        out.sort_by_key(|(t, _)| *t);
    }
    Ok(out)
}

fn delta(args: &[Value]) -> DbResult<Value> {
    if args.len() != 1 {
        return Err(DbError::ExecutionError(
            "DELTA requires 1 argument".to_string(),
        ));
    }
    let pts = series_points(&args[0])?;
    if pts.len() < 2 {
        return Ok(json!([]));
    }
    let mut out = Vec::with_capacity(pts.len() - 1);
    for w in pts.windows(2) {
        out.push(json!({ "t": w[1].0, "v": w[1].1 - w[0].1 }));
    }
    Ok(Value::Array(out))
}

fn rate(args: &[Value]) -> DbResult<Value> {
    if args.len() != 2 {
        return Err(DbError::ExecutionError(
            "RATE requires 2 arguments: series, interval".to_string(),
        ));
    }
    let unit_ms =
        parse_interval_ms(args[1].as_str().ok_or_else(|| {
            DbError::ExecutionError("RATE: interval must be a string".to_string())
        })?)? as f64;
    let pts = series_points(&args[0])?;
    if pts.len() < 2 {
        return Ok(json!([]));
    }
    let mut out = Vec::with_capacity(pts.len() - 1);
    for w in pts.windows(2) {
        let dt = (w[1].0 - w[0].0) as f64;
        // Two samples at the same instant have no rate; 0 would claim a flat
        // series, so report null.
        let r = if dt == 0.0 {
            Value::Null
        } else {
            json!((w[1].1 - w[0].1) / dt * unit_ms)
        };
        out.push(json!({ "t": w[1].0, "v": r }));
    }
    Ok(Value::Array(out))
}

fn fill(args: &[Value]) -> DbResult<Value> {
    if args.len() != 2 {
        return Err(DbError::ExecutionError(
            "FILL requires 2 arguments: series, mode|value".to_string(),
        ));
    }
    let arr = args[0]
        .as_array()
        .ok_or_else(|| DbError::ExecutionError("FILL: series must be an array".to_string()))?;
    let mode = args[1].as_str().unwrap_or("");
    let const_fill = args[1].as_f64();
    if args[1].is_string()
        && !matches!(
            mode,
            "prev" | "locf" | "next" | "nocb" | "interp" | "linear"
        )
    {
        return Err(DbError::ExecutionError(format!(
            "FILL: unknown mode '{mode}' (prev, next, interp, or a number)"
        )));
    }

    let pts: Vec<(i64, Option<f64>)> = arr
        .iter()
        .enumerate()
        .map(|(i, item)| match item {
            Value::Number(n) => (i as i64, n.as_f64()),
            Value::Object(o) => (point_time(o).unwrap_or(i as i64), point_value(o)),
            _ => (i as i64, None),
        })
        .collect();

    // Next known point for every index, filled in one backward pass so that
    // interpolation and next-fill stay O(n).
    let needs_next = matches!(mode, "next" | "nocb" | "interp" | "linear");
    let mut next_known: Vec<Option<(i64, f64)>> = Vec::new();
    if needs_next {
        next_known = vec![None; pts.len()];
        let mut nxt = None;
        for i in (0..pts.len()).rev() {
            next_known[i] = nxt;
            if let (t, Some(v)) = pts[i] {
                nxt = Some((t, v));
            }
        }
    }

    let mut last: Option<(i64, f64)> = None;
    let mut out = Vec::with_capacity(pts.len());
    for (i, &(t, maybe_v)) in pts.iter().enumerate() {
        let v = match maybe_v {
            Some(x) => {
                last = Some((t, x));
                x
            }
            None => match mode {
                "prev" | "locf" => last.map(|(_, v)| v).unwrap_or(0.0),
                "next" | "nocb" => next_known[i].map(|(_, v)| v).unwrap_or(0.0),
                "interp" | "linear" => match (last, next_known[i]) {
                    (Some((ta, a)), Some((tb, b))) => {
                        if tb == ta {
                            a
                        } else {
                            a + (b - a) * (t - ta) as f64 / (tb - ta) as f64
                        }
                    }
                    (Some((_, a)), None) => a,
                    (None, Some((_, b))) => b,
                    _ => 0.0,
                },
                _ => const_fill.unwrap_or(0.0),
            },
        };
        out.push(json!({ "t": t, "v": v }));
    }
    Ok(Value::Array(out))
}

fn resample(args: &[Value]) -> DbResult<Value> {
    if args.len() != 2 {
        return Err(DbError::ExecutionError(
            "RESAMPLE requires 2 arguments: series, interval".to_string(),
        ));
    }
    let bucket = parse_interval_ms(args[1].as_str().ok_or_else(|| {
        DbError::ExecutionError("RESAMPLE: interval must be a string".to_string())
    })?)?;
    let pts = series_points(&args[0])?;
    if pts.is_empty() {
        return Ok(json!([]));
    }
    // div_euclid: a negative timestamp belongs to the bucket below it, not
    // the one truncation toward zero would pick.
    let bucket_of = |t: i64| t.div_euclid(bucket) * bucket;
    let mut out = Vec::new();
    let mut cur_b = bucket_of(pts[0].0);
    let mut last = pts[0].1;
    let mut sum = 0.0;
    let mut n = 0i64;
    for (t, v) in pts {
        let b = bucket_of(t);
        if b != cur_b {
            out.push(json!({
                "t": cur_b,
                "v": last,
                "avg": if n > 0 { sum / n as f64 } else { last }
            }));
            cur_b = b;
            sum = 0.0;
            n = 0;
        }
        last = v;
        sum += v;
        n += 1;
    }
    out.push(json!({
        "t": cur_b,
        "v": last,
        "avg": if n > 0 { sum / n as f64 } else { last }
    }));
    Ok(Value::Array(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_parser_is_char_safe_and_rejects_non_positive() {
        assert!(parse_interval("é").is_err());
        assert!(parse_interval("5µ").is_err());
        assert!(parse_interval("5é").is_err());
        assert!(parse_interval("0s").is_err());
        assert!(parse_interval("-5m").is_err());
        assert!(parse_interval("m").is_err());
        assert!(parse_interval("5").is_err());
        assert_eq!(parse_interval_ms("5m").unwrap(), 300_000);
        assert_eq!(parse_interval_ms("250ms").unwrap(), 250);
        assert_eq!(parse_interval_ms("2w").unwrap(), 1_209_600_000);
        assert_eq!(parse_interval_ms("3 hours").unwrap(), 10_800_000);
        assert_eq!(parse_interval("2mo").unwrap(), Interval::Months(2));
        assert_eq!(parse_interval("1y").unwrap(), Interval::Months(12));
        assert!(parse_interval_ms("1mo").is_err());
        assert!(parse_interval("99999999999999999d").is_err());
    }

    #[test]
    fn fill_interpolates_linearly_by_time() {
        let r = fill(&[
            json!([{"t":0,"v":0},{"t":1,"v":null},{"t":3,"v":null},{"t":4,"v":8}]),
            json!("interp"),
        ])
        .unwrap();
        assert_eq!(r[1]["v"], json!(2.0));
        assert_eq!(r[2]["v"], json!(6.0));
        let r = fill(&[
            json!([{"ts":0.0,"v":1},{"ts":1.0,"v":null},{"ts":2.0,"v":3}]),
            json!("interp"),
        ])
        .unwrap();
        assert_eq!(r[1]["t"], json!(1));
        assert_eq!(r[1]["v"], json!(2.0));
        let r = fill(&[json!([null, 5]), json!("next")]).unwrap();
        assert_eq!(r[0]["v"], json!(5.0));
        assert!(fill(&[json!([1]), json!("bogus")]).is_err());
    }

    #[test]
    fn resample_negative_timestamps_floor() {
        let r = resample(&[json!([{"t":-1,"v":1},{"t":1,"v":2}]), json!("1s")]).unwrap();
        assert_eq!(r[0]["t"], json!(-1000));
        assert_eq!(r[1]["t"], json!(0));
    }

    #[test]
    fn rate_same_instant_is_null() {
        let r = rate(&[json!([{"t":0,"v":0},{"t":0,"v":5}]), json!("1s")]).unwrap();
        assert_eq!(r[0]["v"], Value::Null);
    }
}
