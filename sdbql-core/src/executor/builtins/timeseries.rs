//! Time-series array helpers: DELTA, RATE, FILL, RESAMPLE, and MATCH_SEQ.
//!
//! Series are arrays of numbers (the index is the time) or of
//! `{t|ts|time, v|value}` objects. Nulls are gaps.

use serde_json::{json, Map, Value};

use super::common::{check_arity, err, parse_interval_ms};
use crate::error::SdbqlResult;
use crate::executor::helpers::values_equal;

fn time_of(o: &Map<String, Value>) -> Option<i64> {
    o.get("t")
        .or_else(|| o.get("ts"))
        .or_else(|| o.get("time"))
        .and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)))
}

fn value_of(o: &Map<String, Value>) -> Option<f64> {
    o.get("v")
        .or_else(|| o.get("value"))
        .and_then(Value::as_f64)
}

/// (t, v) points, gaps dropped, sorted by time (skipping the sort when the
/// input is already ordered).
fn series_points(name: &str, v: &Value) -> SdbqlResult<Vec<(i64, f64)>> {
    let arr = v
        .as_array()
        .ok_or_else(|| err(format!("{}: series must be an array", name)))?;
    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        match item {
            Value::Number(n) => {
                let y = n
                    .as_f64()
                    .ok_or_else(|| err(format!("{}: invalid number", name)))?;
                out.push((i as i64, y));
            }
            Value::Object(o) => {
                let y = value_of(o).ok_or_else(|| {
                    err(format!("{}: series objects need a numeric v/value", name))
                })?;
                out.push((time_of(o).unwrap_or(i as i64), y));
            }
            Value::Null => {}
            _ => {
                return Err(err(format!(
                    "{}: series items must be numbers or {{t, v}} objects",
                    name
                )))
            }
        }
    }
    if !out.windows(2).all(|w| w[0].0 <= w[1].0) {
        out.sort_by_key(|(t, _)| *t);
    }
    Ok(out)
}

fn delta(args: &[Value]) -> SdbqlResult<Value> {
    check_arity("DELTA", args, 1, 1)?;
    if args[0].is_null() {
        return Ok(Value::Null);
    }
    let pts = series_points("DELTA", &args[0])?;
    Ok(Value::Array(
        pts.windows(2)
            .map(|w| json!({ "t": w[1].0, "v": w[1].1 - w[0].1 }))
            .collect(),
    ))
}

fn rate(args: &[Value]) -> SdbqlResult<Value> {
    check_arity("RATE", args, 2, 2)?;
    if args[0].is_null() {
        return Ok(Value::Null);
    }
    let unit_ms = parse_interval_ms(
        args[1]
            .as_str()
            .ok_or_else(|| err("RATE: interval must be a string"))?,
    )? as f64;
    let pts = series_points("RATE", &args[0])?;
    Ok(Value::Array(
        pts.windows(2)
            .map(|w| {
                // i128: two timestamps far apart can overflow i64.
                let dt = (i128::from(w[1].0) - i128::from(w[0].0)) as f64;
                let r = if dt == 0.0 {
                    0.0
                } else {
                    (w[1].1 - w[0].1) / dt * unit_ms
                };
                json!({ "t": w[1].0, "v": r })
            })
            .collect(),
    ))
}

/// FILL(series, "prev" | "locf" | "interp" | constant): one pass, with
/// linear interpolation on time for "interp".
fn fill(args: &[Value]) -> SdbqlResult<Value> {
    check_arity("FILL", args, 2, 2)?;
    if args[0].is_null() {
        return Ok(Value::Null);
    }
    let arr = args[0]
        .as_array()
        .ok_or_else(|| err("FILL: series must be an array"))?;
    let mode = args[1].as_str().unwrap_or("");
    let constant = args[1].as_f64().unwrap_or(0.0);

    let items: Vec<(i64, Option<f64>)> = arr
        .iter()
        .enumerate()
        .map(|(i, item)| match item {
            Value::Number(n) => (i as i64, n.as_f64()),
            Value::Object(o) => (time_of(o).unwrap_or(i as i64), value_of(o)),
            _ => (i as i64, None),
        })
        .collect();

    // next_known[i]: the first known point at or after i.
    let mut next_known: Vec<Option<(i64, f64)>> = vec![None; items.len()];
    let mut upcoming = None;
    for i in (0..items.len()).rev() {
        if let (t, Some(v)) = items[i] {
            upcoming = Some((t, v));
        }
        next_known[i] = upcoming;
    }

    let mut last: Option<(i64, f64)> = None;
    let mut out = Vec::with_capacity(items.len());
    for (i, &(t, maybe_v)) in items.iter().enumerate() {
        let v = match maybe_v {
            Some(x) => {
                last = Some((t, x));
                x
            }
            None => match mode {
                "prev" | "locf" => last.map(|(_, v)| v).unwrap_or(0.0),
                "interp" | "linear" => match (last, next_known[i]) {
                    (Some((ta, a)), Some((tb, b))) if tb != ta => {
                        // In f64: `t - ta` can overflow i64 for extreme times.
                        a + (b - a) * (t as f64 - ta as f64) / (tb as f64 - ta as f64)
                    }
                    (Some((_, a)), Some((_, b))) => (a + b) / 2.0,
                    (Some((_, a)), None) => a,
                    (None, Some((_, b))) => b,
                    (None, None) => 0.0,
                },
                _ => constant,
            },
        };
        out.push(json!({ "t": t, "v": v }));
    }
    Ok(Value::Array(out))
}

/// RESAMPLE(series, interval): last value and average per bucket. Buckets
/// use `div_euclid`, so negative timestamps land in the right bucket.
fn resample(args: &[Value]) -> SdbqlResult<Value> {
    check_arity("RESAMPLE", args, 2, 2)?;
    if args[0].is_null() {
        return Ok(Value::Null);
    }
    let bucket = parse_interval_ms(
        args[1]
            .as_str()
            .ok_or_else(|| err("RESAMPLE: interval must be a string"))?,
    )?;
    let pts = series_points("RESAMPLE", &args[0])?;
    // Saturating: the bucket start of a timestamp near i64::MIN underflows.
    let start = |t: i64| t.saturating_sub(t.rem_euclid(bucket));
    let mut out = Vec::new();
    let mut iter = pts.into_iter();
    let Some((t0, v0)) = iter.next() else {
        return Ok(json!([]));
    };
    let (mut cur, mut last, mut sum, mut n) = (start(t0), v0, v0, 1usize);
    for (t, v) in iter {
        let b = start(t);
        if b != cur {
            out.push(json!({ "t": cur, "v": last, "avg": sum / n as f64 }));
            cur = b;
            sum = 0.0;
            n = 0;
        }
        last = v;
        sum += v;
        n += 1;
    }
    out.push(json!({ "t": cur, "v": last, "avg": sum / n as f64 }));
    Ok(Value::Array(out))
}

fn event_ts(e: &Value) -> i64 {
    e.as_object().and_then(time_of).unwrap_or(0)
}

fn step_matches(ev: &Value, step: &Value) -> bool {
    if let Some(ty) = step.get("type").and_then(Value::as_str) {
        let ev_ty = ev
            .get("type")
            .or_else(|| ev.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if ev_ty != ty {
            return false;
        }
    }
    if let (Some(field), Some(eq)) = (
        step.get("field").and_then(Value::as_str),
        step.get("equals"),
    ) {
        if !ev.get(field).is_some_and(|v| values_equal(v, eq)) {
            return false;
        }
    }
    true
}

fn match_one_key(evs: &[&Value], steps: &[Value], within: &[Option<i64>]) -> Option<Value> {
    let mut found = Vec::with_capacity(steps.len());
    let mut idx = 0usize;
    let mut last_ts: Option<i64> = None;
    for (step, &limit) in steps.iter().zip(within) {
        let mut hit = None;
        while idx < evs.len() {
            let ev = evs[idx];
            idx += 1;
            if !step_matches(ev, step) {
                continue;
            }
            let ts = event_ts(ev);
            if let (Some(prev), Some(w)) = (last_ts, limit) {
                if ts.saturating_sub(prev) > w {
                    return None;
                }
            }
            last_ts = Some(ts);
            let label = step.get("as").and_then(Value::as_str).unwrap_or("step");
            hit = Some(json!({ "as": label, "event": ev, "ts": ts }));
            break;
        }
        found.push(hit?);
    }
    Some(Value::Array(found))
}

/// MATCH_SEQ(events, key_field, steps): per key, the first ordered run of
/// events matching every step (`{as, type, field, equals, within}`).
fn match_seq(args: &[Value]) -> SdbqlResult<Value> {
    check_arity("MATCH_SEQ", args, 3, 3)?;
    let events = args[0]
        .as_array()
        .ok_or_else(|| err("MATCH_SEQ: events must be an array"))?;
    let key_field = args[1]
        .as_str()
        .ok_or_else(|| err("MATCH_SEQ: key_field must be a string"))?;
    let steps = args[2]
        .as_array()
        .ok_or_else(|| err("MATCH_SEQ: steps must be an array"))?;
    if steps.is_empty() {
        return Ok(Value::Array(vec![]));
    }
    // Parse every `within` once, and reject a bad one instead of ignoring it.
    let within = steps
        .iter()
        .map(|s| match s.get("within") {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(w)) => parse_interval_ms(w).map(Some),
            Some(_) => Err(err("MATCH_SEQ: within must be an interval string")),
        })
        .collect::<SdbqlResult<Vec<_>>>()?;

    let mut by_key: std::collections::BTreeMap<String, Vec<&Value>> = Default::default();
    for ev in events {
        let k = ev
            .get(key_field)
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_else(|| "null".into());
        by_key.entry(k).or_default().push(ev);
    }

    // BTreeMap: keys come out in a stable order.
    let mut matches = Vec::new();
    for (key, mut evs) in by_key {
        evs.sort_by_key(|e| event_ts(e));
        if let Some(hit) = match_one_key(&evs, steps, &within) {
            matches.push(json!({ "key": key, "steps": hit }));
        }
    }
    Ok(Value::Array(matches))
}

/// Call a time-series function. Returns None if function not found.
pub fn call(name: &str, args: &[Value]) -> SdbqlResult<Option<Value>> {
    let result = match name {
        "DELTA" => delta(args)?,
        "RATE" => rate(args)?,
        "FILL" => fill(args)?,
        "RESAMPLE" => resample(args)?,
        "MATCH_SEQ" => match_seq(args)?,
        _ => return Ok(None),
    };
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(name: &str, args: &[Value]) -> Value {
        call(name, args).unwrap().unwrap()
    }

    #[test]
    fn delta_and_rate() {
        assert_eq!(
            ok("DELTA", &[json!([{"t": 0, "v": 1}, {"t": 10, "v": 4}])]),
            json!([{"t": 10, "v": 3.0}])
        );
        assert_eq!(
            ok(
                "RATE",
                &[json!([{"t": 0, "v": 0}, {"t": 1000, "v": 10}]), json!("1s")]
            ),
            json!([{"t": 1000, "v": 10.0}])
        );
        assert!(call("RATE", &[json!([1, 2]), json!("5µ")]).is_err());
        assert!(call("RATE", &[json!([1, 2]), json!("-5s")]).is_err());
        assert_eq!(
            ok(
                "RATE",
                &[
                    json!([{"t": i64::MIN, "v": 0}, {"t": i64::MAX, "v": 1}]),
                    json!("1s")
                ]
            )
            .as_array()
            .unwrap()
            .len(),
            1
        );
    }

    #[test]
    fn fill_interpolates_linearly() {
        assert_eq!(
            ok(
                "FILL",
                &[
                    json!([{"t": 0, "v": 0}, {"t": 1, "v": null}, {"t": 2, "v": null}, {"t": 3, "v": 3}]),
                    json!("interp")
                ]
            ),
            json!([{"t": 0, "v": 0.0}, {"t": 1, "v": 1.0}, {"t": 2, "v": 2.0}, {"t": 3, "v": 3.0}])
        );
        assert_eq!(
            ok("FILL", &[json!([1, null, 3]), json!("prev")]),
            json!([{"t": 0, "v": 1.0}, {"t": 1, "v": 1.0}, {"t": 2, "v": 3.0}])
        );
        assert_eq!(
            ok("FILL", &[json!([null, 2]), json!(9)]),
            json!([{"t": 0, "v": 9.0}, {"t": 1, "v": 2.0}])
        );
    }

    #[test]
    fn resample_uses_euclidean_buckets() {
        let r = ok(
            "RESAMPLE",
            &[json!([{"t": -1, "v": 1}, {"t": 1, "v": 3}]), json!("1s")],
        );
        // -1 ms belongs to the bucket starting at -1000, not 0.
        assert_eq!(
            r,
            json!([{"t": -1000, "v": 1.0, "avg": 1.0}, {"t": 0, "v": 3.0, "avg": 3.0}])
        );
        assert_eq!(ok("RESAMPLE", &[json!([]), json!("1m")]), json!([]));
    }

    #[test]
    fn match_seq_finds_ordered_steps() {
        let events = json!([
            {"user": "a", "type": "signup", "ts": 0},
            {"user": "a", "type": "pay", "ts": 1000},
            {"user": "b", "type": "signup", "ts": 0},
            {"user": "b", "type": "pay", "ts": 999_999_999},
        ]);
        let steps = json!([
            {"as": "s", "type": "signup"},
            {"as": "p", "type": "pay", "within": "7d"}
        ]);
        let r = ok("MATCH_SEQ", &[events.clone(), json!("user"), steps]);
        let arr = r.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["key"], json!("a"));
        // A non-ASCII `within` used to panic in the interval parser.
        let bad = json!([{"type": "signup"}, {"type": "pay", "within": "7é"}]);
        assert!(call("MATCH_SEQ", &[events, json!("user"), bad]).is_err());
    }
}
