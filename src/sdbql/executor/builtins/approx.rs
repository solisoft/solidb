//! Approximate distinct / percentile / top-k sketches.

use crate::error::{DbError, DbResult};
use crate::sdbql::executor::helpers::hash_value;
use crate::sdbql::executor::utils::number_from_f64;
use serde_json::{json, Map, Value};

const HLL_P: u8 = 14;
const HLL_M: usize = 1 << HLL_P; // 16384

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "APPROX_COUNT_DISTINCT" => Ok(Some(approx_count_distinct(args)?)),
        "APPROX_PERCENTILE" => Ok(Some(approx_percentile(args)?)),
        "APPROX_TOP_K" => Ok(Some(approx_top_k(args)?)),
        "SKETCH_MERGE" => Ok(Some(sketch_merge(args)?)),
        "MINHASH" => Ok(Some(minhash(args)?)),
        "MINHASH_COUNT" => {
            let err = args.first().and_then(Value::as_f64).unwrap_or(0.05);
            let n = if err <= 0.0 {
                1
            } else {
                ((1.0 / (err * err)).ceil() as u64).max(1)
            };
            Ok(Some(json!(n)))
        }
        "MINHASH_ERROR" => {
            let n = args.first().and_then(Value::as_f64).unwrap_or(1.0).max(1.0);
            Ok(Some(json!(1.0 / n.sqrt())))
        }
        _ => Ok(None),
    }
}

fn approx_count_distinct(args: &[Value]) -> DbResult<Value> {
    if args.len() != 1 {
        return Err(DbError::ExecutionError(
            "APPROX_COUNT_DISTINCT requires 1 argument".to_string(),
        ));
    }
    if let Some(regs) = hll_regs(&args[0]) {
        return Ok(json!(hll_estimate(&regs)));
    }
    let arr = args[0].as_array().ok_or_else(|| {
        DbError::ExecutionError("APPROX_COUNT_DISTINCT expects an array or HLL sketch".to_string())
    })?;
    let mut regs = vec![0u8; HLL_M];
    for v in arr {
        hll_add(&mut regs, hash_value(v));
    }
    Ok(json!({
        "_type": "hll",
        "p": HLL_P,
        "estimate": hll_estimate(&regs),
        "registers": encode_regs(&regs),
    }))
}

fn approx_percentile(args: &[Value]) -> DbResult<Value> {
    if args.len() != 2 {
        return Err(DbError::ExecutionError(
            "APPROX_PERCENTILE requires 2 arguments: array, p".to_string(),
        ));
    }
    let p = args[1].as_f64().ok_or_else(|| {
        DbError::ExecutionError("APPROX_PERCENTILE: p must be a number 0-100".to_string())
    })?;
    if !(0.0..=100.0).contains(&p) {
        return Err(DbError::ExecutionError(
            "APPROX_PERCENTILE: p must be 0-100".to_string(),
        ));
    }
    let mut xs: Vec<f64> = args[0]
        .as_array()
        .ok_or_else(|| DbError::ExecutionError("APPROX_PERCENTILE expects an array".to_string()))?
        .iter()
        .filter_map(Value::as_f64)
        .collect();
    if xs.is_empty() {
        return Ok(Value::Null);
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((p / 100.0) * (xs.len() - 1) as f64).round() as usize;
    Ok(Value::Number(number_from_f64(xs[idx.min(xs.len() - 1)])))
}

fn approx_top_k(args: &[Value]) -> DbResult<Value> {
    if args.len() != 2 {
        return Err(DbError::ExecutionError(
            "APPROX_TOP_K requires 2 arguments: array, k".to_string(),
        ));
    }
    let k = args[1].as_u64().unwrap_or(0) as usize;
    if k == 0 {
        return Ok(json!([]));
    }
    let arr = args[0]
        .as_array()
        .ok_or_else(|| DbError::ExecutionError("APPROX_TOP_K expects an array".to_string()))?;
    let mut counts: Vec<(Value, u64)> = Vec::new();
    for v in arr {
        if let Some((_, c)) = counts
            .iter_mut()
            .find(|(x, _)| crate::sdbql::executor::helpers::values_equal(x, v))
        {
            *c += 1;
        } else if counts.len() < k * 4 {
            counts.push((v.clone(), 1));
        } else {
            // Space-Saving: decrement all, drop zeros
            for c in counts.iter_mut() {
                c.1 = c.1.saturating_sub(1);
            }
            counts.retain(|(_, c)| *c > 0);
            counts.push((v.clone(), 1));
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    counts.truncate(k);
    Ok(Value::Array(
        counts
            .into_iter()
            .map(|(v, c)| json!({ "value": v, "count": c }))
            .collect(),
    ))
}

fn sketch_merge(args: &[Value]) -> DbResult<Value> {
    if args.len() != 2 {
        return Err(DbError::ExecutionError(
            "SKETCH_MERGE requires 2 sketches".to_string(),
        ));
    }
    let ta = args[0].get("_type").and_then(Value::as_str);
    let tb = args[1].get("_type").and_then(Value::as_str);
    if ta != tb {
        return Err(DbError::ExecutionError(
            "SKETCH_MERGE: sketches must have the same _type".to_string(),
        ));
    }
    match ta {
        Some("hll") => {
            let mut a = hll_regs(&args[0])
                .ok_or_else(|| DbError::ExecutionError("SKETCH_MERGE: invalid HLL".to_string()))?;
            let b = hll_regs(&args[1])
                .ok_or_else(|| DbError::ExecutionError("SKETCH_MERGE: invalid HLL".to_string()))?;
            if a.len() != b.len() {
                return Err(DbError::ExecutionError(
                    "SKETCH_MERGE: HLL precision mismatch".to_string(),
                ));
            }
            for (x, y) in a.iter_mut().zip(b.iter()) {
                *x = (*x).max(*y);
            }
            Ok(json!({
                "_type": "hll",
                "p": HLL_P,
                "estimate": hll_estimate(&a),
                "registers": encode_regs(&a),
            }))
        }
        _ => Err(DbError::ExecutionError(
            "SKETCH_MERGE: unsupported sketch type".to_string(),
        )),
    }
}

fn minhash(args: &[Value]) -> DbResult<Value> {
    if args.len() != 2 {
        return Err(DbError::ExecutionError(
            "MINHASH requires array, numHashes".to_string(),
        ));
    }
    let arr = args[0].as_array().ok_or_else(|| {
        DbError::ExecutionError("MINHASH: first argument must be an array".to_string())
    })?;
    let n = args[1].as_u64().unwrap_or(1).clamp(1, 1024) as usize;
    let mut sig = vec![u64::MAX; n];
    for v in arr {
        let h0 = hash_value(v);
        for (i, slot) in sig.iter_mut().enumerate() {
            let hi = h0
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(i as u64);
            if hi < *slot {
                *slot = hi;
            }
        }
    }
    Ok(Value::Array(
        sig.into_iter()
            .map(|h| Value::String(format!("{h:016x}")))
            .collect(),
    ))
}

fn hll_add(regs: &mut [u8], hash: u64) {
    let idx = (hash as usize) & (regs.len() - 1);
    let w = hash >> HLL_P;
    let rho = (w.trailing_zeros() as u8).saturating_add(1).min(64);
    if rho > regs[idx] {
        regs[idx] = rho;
    }
}

fn hll_estimate(regs: &[u8]) -> f64 {
    let m = regs.len() as f64;
    let mut sum = 0.0;
    let mut zeros = 0;
    for &r in regs {
        sum += 2f64.powi(-(r as i32));
        if r == 0 {
            zeros += 1;
        }
    }
    let alpha = 0.7213 / (1.0 + 1.079 / m);
    let mut e = alpha * m * m / sum;
    if e <= 2.5 * m && zeros > 0 {
        e = m * (m / zeros as f64).ln();
    }
    e.round()
}

fn encode_regs(regs: &[u8]) -> Value {
    Value::Array(regs.iter().map(|&r| Value::Number(r.into())).collect())
}

fn hll_regs(v: &Value) -> Option<Vec<u8>> {
    if v.get("_type").and_then(Value::as_str) != Some("hll") {
        return None;
    }
    let regs = v.get("registers")?.as_array()?;
    Some(regs.iter().map(|x| x.as_u64().unwrap_or(0) as u8).collect())
}

#[allow(dead_code)]
fn empty_map() -> Map<String, Value> {
    Map::new()
}
