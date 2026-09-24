//! Approximate distinct / percentile / top-k sketches.

use std::collections::HashMap;

use super::array::as_int;
use crate::error::{DbError, DbResult};
use crate::sdbql::executor::helpers::{hash_value, values_equal};
use crate::sdbql::executor::utils::number_from_f64;
use serde_json::{json, Value};

const HLL_P: u8 = 14;
const HLL_M: usize = 1 << HLL_P; // 16384

/// Ceiling on `APPROX_TOP_K`'s k — the same bound as the search functions'
/// result counts. k sizes the counter table (4k entries).
const MAX_TOP_K: usize = 10_000;

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

/// `APPROX_COUNT_DISTINCT(array | sketch, options?)`.
///
/// Returns the estimate. The HyperLogLog sketch itself — what `SKETCH_MERGE`
/// combines — is only built into the result when asked for with
/// `{sketch: true}`: it is 16 384 registers, and returning it on every call
/// made the common case pay for the rare one.
fn approx_count_distinct(args: &[Value]) -> DbResult<Value> {
    if args.is_empty() || args.len() > 2 {
        return Err(DbError::ExecutionError(
            "APPROX_COUNT_DISTINCT requires 1-2 arguments: array, [options]".to_string(),
        ));
    }
    let want_sketch = match args.get(1) {
        None | Some(Value::Null) => false,
        Some(Value::Object(o)) => o.get("sketch").and_then(Value::as_bool).unwrap_or(false),
        Some(_) => {
            return Err(DbError::ExecutionError(
                "APPROX_COUNT_DISTINCT: options must be an object".to_string(),
            ))
        }
    };
    let regs = if let Some(regs) = hll_regs(&args[0]) {
        regs
    } else {
        let arr = args[0].as_array().ok_or_else(|| {
            DbError::ExecutionError(
                "APPROX_COUNT_DISTINCT expects an array or HLL sketch".to_string(),
            )
        })?;
        let mut regs = vec![0u8; HLL_M];
        for v in arr {
            hll_add(&mut regs, hash_value(v));
        }
        regs
    };
    if want_sketch {
        Ok(hll_sketch(&regs))
    } else {
        Ok(json!(hll_estimate(&regs)))
    }
}

/// Exact, despite the name: the p-th percentile (0-100) of the numbers in
/// the array, at index `round(p/100 · (n-1))` of the sorted values. Found by
/// selection, not a full sort.
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
    let idx = (((p / 100.0) * (xs.len() - 1) as f64).round() as usize).min(xs.len() - 1);
    let v = *xs.select_nth_unstable_by(idx, f64::total_cmp).1;
    Ok(Value::Number(number_from_f64(v)))
}

/// Misra-Gries heavy hitters over a hashed counter table of 4k entries.
///
/// The old table was a Vec searched linearly for every element (O(n·k)) with
/// a user-chosen k, and `k * 4` could overflow. Counts are exact while the
/// number of distinct values fits the table, and lower bounds after that.
fn approx_top_k(args: &[Value]) -> DbResult<Value> {
    if args.len() != 2 {
        return Err(DbError::ExecutionError(
            "APPROX_TOP_K requires 2 arguments: array, k".to_string(),
        ));
    }
    let k = as_int(&args[1]).unwrap_or(0);
    if k <= 0 {
        return Ok(json!([]));
    }
    let k = usize::try_from(k).unwrap_or(usize::MAX).min(MAX_TOP_K);
    let arr = args[0]
        .as_array()
        .ok_or_else(|| DbError::ExecutionError("APPROX_TOP_K expects an array".to_string()))?;
    let capacity = k.saturating_mul(4);

    let mut counts: Vec<(&Value, u64)> = Vec::new();
    let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
    for v in arr {
        let h = hash_value(v);
        let found = index
            .get(&h)
            .and_then(|b| b.iter().copied().find(|&i| values_equal(counts[i].0, v)));
        if let Some(i) = found {
            counts[i].1 = counts[i].1.saturating_add(1);
        } else if counts.len() < capacity {
            index.entry(h).or_default().push(counts.len());
            counts.push((v, 1));
        } else {
            // Table full: every counter pays one, the newcomer is dropped.
            // Each pass is paid for by `capacity` earlier increments, so the
            // whole run stays O(n) amortised.
            for c in counts.iter_mut() {
                c.1 -= 1;
            }
            counts.retain(|&(_, c)| c > 0);
            index.clear();
            for (i, (val, _)) in counts.iter().enumerate() {
                index.entry(hash_value(val)).or_default().push(i);
            }
        }
    }
    // Stable: equal counts keep first-seen order.
    counts.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
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
            Ok(hll_sketch(&a))
        }
        _ => Err(DbError::ExecutionError(
            "SKETCH_MERGE: unsupported sketch type".to_string(),
        )),
    }
}

/// SplitMix64 finaliser: a bijective, non-linear 64-bit mix.
#[inline]
fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
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
    let n = as_int(&args[1]).unwrap_or(1).clamp(1, 1024) as usize;
    // Each slot needs its own hash function. `h * C + i` differed between
    // slots only by a constant, so every slot picked the same minimum element
    // and the signature carried one hash's worth of information. Mixing the
    // element hash with a per-slot seed gives independent orderings.
    let seeds: Vec<u64> = (0..n as u64)
        .map(|i| mix64(i.wrapping_add(0x9E37_79B9_7F4A_7C15)))
        .collect();
    let mut sig = vec![u64::MAX; n];
    for v in arr {
        let h0 = hash_value(v);
        for (slot, seed) in sig.iter_mut().zip(&seeds) {
            let hi = mix64(h0 ^ seed);
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

fn hll_sketch(regs: &[u8]) -> Value {
    json!({
        "_type": "hll",
        "p": HLL_P,
        "estimate": hll_estimate(regs),
        "registers": encode_regs(regs),
    })
}

/// Registers as a hex string: two characters per register instead of one
/// JSON number (a `Value` each) per register.
fn encode_regs(regs: &[u8]) -> Value {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(regs.len() * 2);
    for &r in regs {
        s.push(HEX[(r >> 4) as usize] as char);
        s.push(HEX[(r & 0xf) as usize] as char);
    }
    Value::String(s)
}

/// Decode a sketch's registers: the hex string written now, or the array of
/// numbers written by earlier versions (sketches may have been stored).
fn hll_regs(v: &Value) -> Option<Vec<u8>> {
    if v.get("_type").and_then(Value::as_str) != Some("hll") {
        return None;
    }
    let regs: Vec<u8> = match v.get("registers")? {
        Value::String(s) => {
            let bytes = s.as_bytes();
            if !bytes.len().is_multiple_of(2) {
                return None;
            }
            let nibble = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[hi, lo]| Some((nibble(hi)? << 4) | nibble(lo)?))
                .collect::<Option<Vec<u8>>>()?
        }
        Value::Array(a) => a
            .iter()
            .map(|x| x.as_u64().unwrap_or(0).min(64) as u8)
            .collect(),
        _ => return None,
    };
    // hll_add masks with len - 1 and every sketch this module writes has
    // HLL_M registers; anything else is not one of ours.
    (regs.len() == HLL_M).then_some(regs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_distinct_returns_the_estimate_by_default() {
        let v = approx_count_distinct(&[json!([1, 1, 2, 3, 3, 3])]).unwrap();
        assert_eq!(v, json!(3.0));
        let empty = approx_count_distinct(&[json!([])]).unwrap();
        assert_eq!(empty, json!(0.0));
    }

    #[test]
    fn sketch_is_opt_in_and_round_trips() {
        let a = approx_count_distinct(&[json!([1, 2, 3]), json!({"sketch": true})]).unwrap();
        assert_eq!(a["_type"], json!("hll"));
        assert!(a["registers"].is_string());
        let b = approx_count_distinct(&[json!([3, 4, 5]), json!({"sketch": true})]).unwrap();
        let merged = sketch_merge(&[a.clone(), b]).unwrap();
        assert_eq!(merged["estimate"], json!(5.0));
        // A sketch as input yields its estimate.
        assert_eq!(approx_count_distinct(&[a]).unwrap(), json!(3.0));
        // Sketches stored by earlier versions (register arrays) still load.
        let legacy = json!({"_type": "hll", "p": 14, "registers": vec![0u8; HLL_M]});
        assert_eq!(approx_count_distinct(&[legacy]).unwrap(), json!(0.0));
    }

    #[test]
    fn top_k_counts_and_caps() {
        let top = approx_top_k(&[json!(["a", "a", "b", "a", "c", "b"]), json!(2)]).unwrap();
        assert_eq!(
            top,
            json!([{"value": "a", "count": 3}, {"value": "b", "count": 2}])
        );
        assert_eq!(approx_top_k(&[json!(["a"]), json!(0)]).unwrap(), json!([]));
        // k far beyond the cap and float k both work.
        let top = approx_top_k(&[json!([1, 1, 2]), json!(1e15)]).unwrap();
        assert_eq!(top.as_array().unwrap().len(), 2);
        let top = approx_top_k(&[json!([1, 1, 2]), json!(1.0)]).unwrap();
        assert_eq!(top, json!([{"value": 1, "count": 2}]));
    }

    #[test]
    fn top_k_finds_a_heavy_hitter_past_capacity() {
        let mut items: Vec<Value> = (0..1000).map(|i| json!(i)).collect();
        items.extend(std::iter::repeat_n(json!("hot"), 600));
        let top = approx_top_k(&[Value::Array(items), json!(1)]).unwrap();
        assert_eq!(top[0]["value"], json!("hot"));
    }

    #[test]
    fn minhash_slots_are_independent() {
        let sig = minhash(&[json!(["a", "b", "c", "d", "e", "f", "g", "h"]), json!(64)]).unwrap();
        let sig = sig.as_array().unwrap();
        // With h*C + i every slot's minimum came from the same element, so
        // consecutive slots differed by exactly 1.
        let values: Vec<u64> = sig
            .iter()
            .map(|s| u64::from_str_radix(s.as_str().unwrap(), 16).unwrap())
            .collect();
        let off_by_one = values
            .windows(2)
            .filter(|w| w[1] == w[0].wrapping_add(1))
            .count();
        assert!(off_by_one < 4, "slots look linearly related");

        // Similar sets give similar signatures, disjoint ones do not.
        let s1 = minhash(&[json!(["a", "b", "c", "d"]), json!(128)]).unwrap();
        let s2 = minhash(&[json!(["a", "b", "c", "e"]), json!(128)]).unwrap();
        let s3 = minhash(&[json!(["w", "x", "y", "z"]), json!(128)]).unwrap();
        let agree = |x: &Value, y: &Value| {
            x.as_array()
                .unwrap()
                .iter()
                .zip(y.as_array().unwrap())
                .filter(|(a, b)| a == b)
                .count()
        };
        assert!(agree(&s1, &s2) > agree(&s1, &s3));
    }

    #[test]
    fn approx_percentile_selects() {
        let v = approx_percentile(&[json!([5, 1, 4, 2, 3]), json!(50)]).unwrap();
        assert_eq!(v, json!(3.0));
    }
}
