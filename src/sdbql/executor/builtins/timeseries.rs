//! Time-series array helpers: DELTA, RATE, FILL, RESAMPLE.

use crate::error::{DbError, DbResult};
use crate::sdbql::executor::utils::number_from_f64;
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

pub fn parse_interval_ms(interval_str: &str) -> DbResult<i64> {
    if interval_str.len() < 2 {
        return Err(DbError::ExecutionError(
            "interval: expected form like '5m', '1h', '30s', '1d'".to_string(),
        ));
    }
    let (num, unit) = interval_str.split_at(interval_str.len() - 1);
    let val: i64 = num.parse().map_err(|_| {
        DbError::ExecutionError(format!("interval: invalid number in '{interval_str}'"))
    })?;
    let ms = match unit {
        "s" => val.saturating_mul(1000),
        "m" => val.saturating_mul(60_000),
        "h" => val.saturating_mul(3_600_000),
        "d" => val.saturating_mul(86_400_000),
        _ => {
            return Err(DbError::ExecutionError(
                "interval: valid units are s, m, h, d".to_string(),
            ))
        }
    };
    if ms == 0 {
        return Err(DbError::ExecutionError("interval cannot be 0".to_string()));
    }
    Ok(ms)
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
                let t = o
                    .get("t")
                    .or_else(|| o.get("ts"))
                    .or_else(|| o.get("time"))
                    .and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)))
                    .unwrap_or(i as i64);
                let y = o
                    .get("v")
                    .or_else(|| o.get("value"))
                    .and_then(Value::as_f64)
                    .ok_or_else(|| {
                        DbError::ExecutionError(
                            "time-series object needs numeric v/value".to_string(),
                        )
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
    out.sort_by_key(|(t, _)| *t);
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
        let r = if dt == 0.0 {
            0.0
        } else {
            (w[1].1 - w[0].1) / dt * unit_ms
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
    let mut last: Option<f64> = None;
    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let (t, maybe_v) = match item {
            Value::Null => (i as i64, None),
            Value::Number(n) => (i as i64, n.as_f64()),
            Value::Object(o) => {
                let t = o
                    .get("t")
                    .or_else(|| o.get("ts"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(i as i64);
                let v = o
                    .get("v")
                    .or_else(|| o.get("value"))
                    .and_then(Value::as_f64);
                (t, v)
            }
            _ => (i as i64, None),
        };
        let v = match maybe_v {
            Some(x) => {
                last = Some(x);
                x
            }
            None => match mode {
                "prev" | "locf" => last.unwrap_or(0.0),
                "interp" => {
                    let next = arr.iter().skip(i + 1).find_map(|it| match it {
                        Value::Number(n) => n.as_f64(),
                        Value::Object(o) => o
                            .get("v")
                            .or_else(|| o.get("value"))
                            .and_then(Value::as_f64),
                        _ => None,
                    });
                    match (last, next) {
                        (Some(a), Some(b)) => (a + b) / 2.0,
                        (Some(a), None) => a,
                        (None, Some(b)) => b,
                        _ => 0.0,
                    }
                }
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
    let mut out = Vec::new();
    let mut cur_b = pts[0].0 / bucket * bucket;
    let mut last = pts[0].1;
    let mut sum = 0.0;
    let mut n = 0i64;
    for (t, v) in pts {
        let b = t / bucket * bucket;
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
    let _ = number_from_f64;
    Ok(Value::Array(out))
}
