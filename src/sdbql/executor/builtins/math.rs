//! Math functions for SDBQL.
//!
//! FLOOR, CEIL, ROUND, ABS, SQRT, POW, LOG, SIN, COS, TAN, etc., plus the
//! array statistics (SUM, AVG, MEDIAN, PERCENTILE, VARIANCE, STDDEV, ...).
//!
//! A result that is not a finite number (`POW(10, 400)`, `POW(-8, 1/3)`) is
//! `null`: JSON has no representation for it, and the old `f as i64`
//! fallback turned it into `i64::MAX` or `0`.

use super::array::as_int;
use crate::error::{DbError, DbResult};
use crate::sdbql::executor::compare_values;
use serde_json::Value;

/// Past ±308 decimals the scale factor under- or overflows an f64, so a
/// larger `ROUND` precision cannot change the result.
const MAX_ROUND_DECIMALS: i64 = 308;

/// Evaluate math functions
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "FLOOR" => unary(name, args, f64::floor),
        "CEIL" | "CEILING" => unary(name, args, f64::ceil),
        "TRUNC" | "TRUNCATE" => unary(name, args, f64::trunc),
        "ROUND" => {
            if args.is_empty() || args.len() > 2 {
                return Err(DbError::ExecutionError(
                    "ROUND requires 1-2 arguments".to_string(),
                ));
            }
            let x = get_number(&args[0], name)?;
            let decimals = match args.get(1) {
                None | Some(Value::Null) => 0,
                Some(v) => as_int(v).ok_or_else(|| {
                    DbError::ExecutionError("ROUND: decimals must be an integer".to_string())
                })?,
            }
            .clamp(-MAX_ROUND_DECIMALS, MAX_ROUND_DECIMALS);
            Ok(Some(num(round_to(x, decimals))))
        }
        "ABS" => unary(name, args, f64::abs),
        "SIGN" => {
            check_args(name, args, 1)?;
            let x = get_number(&args[0], name)?;
            let sign: i64 = if x > 0.0 {
                1
            } else if x < 0.0 {
                -1
            } else {
                0
            };
            Ok(Some(Value::Number(sign.into())))
        }
        "SQRT" => {
            check_args(name, args, 1)?;
            let x = get_number(&args[0], name)?;
            if x < 0.0 {
                return Err(DbError::ExecutionError(
                    "SQRT: argument must be non-negative".to_string(),
                ));
            }
            Ok(Some(num(x.sqrt())))
        }
        "POW" | "POWER" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "POW requires 2 arguments: base, exponent".to_string(),
                ));
            }
            let base = get_number(&args[0], name)?;
            let exp = get_number(&args[1], name)?;
            Ok(Some(num(base.powf(exp))))
        }
        "LOG" | "LN" => positive_log(name, args, "LOG", f64::ln),
        "LOG10" => positive_log(name, args, "LOG10", f64::log10),
        "LOG2" => positive_log(name, args, "LOG2", f64::log2),
        "EXP" => unary(name, args, f64::exp),
        "EXP2" => unary(name, args, f64::exp2),
        "SIN" => unary(name, args, f64::sin),
        "COS" => unary(name, args, f64::cos),
        "TAN" => unary(name, args, f64::tan),
        "ASIN" | "ACOS" => {
            check_args(name, args, 1)?;
            let x = get_number(&args[0], name)?;
            if !(-1.0..=1.0).contains(&x) {
                return Err(DbError::ExecutionError(format!(
                    "{}: argument must be between -1 and 1",
                    name
                )));
            }
            Ok(Some(num(if name == "ASIN" { x.asin() } else { x.acos() })))
        }
        "ATAN" => unary(name, args, f64::atan),
        "ATAN2" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "ATAN2 requires 2 arguments: y, x".to_string(),
                ));
            }
            let y = get_number(&args[0], name)?;
            let x = get_number(&args[1], name)?;
            Ok(Some(num(y.atan2(x))))
        }
        "DEGREES" | "DEG" => unary(name, args, f64::to_degrees),
        "RADIANS" | "RAD" => unary(name, args, f64::to_radians),
        "PI" => Ok(Some(num(std::f64::consts::PI))),
        "E" => Ok(Some(num(std::f64::consts::E))),
        "MOD" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "MOD requires 2 arguments: a, b".to_string(),
                ));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let a = get_number(&args[0], name)?;
            let b = get_number(&args[1], name)?;
            if b == 0.0 {
                return Err(DbError::ExecutionError(
                    "MOD: divisor cannot be 0".to_string(),
                ));
            }
            Ok(Some(num(a % b)))
        }
        "BIT_AND" => bit_binop(args, "BIT_AND", |a, b| a & b),
        "BIT_OR" => bit_binop(args, "BIT_OR", |a, b| a | b),
        "BIT_XOR" => bit_binop(args, "BIT_XOR", |a, b| a ^ b),
        "BIT_NEGATE" | "BIT_NOT" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "BIT_NEGATE requires 1 argument".to_string(),
                ));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let a = as_int(&args[0]).ok_or_else(|| {
                DbError::ExecutionError("BIT_NEGATE: argument must be an integer".to_string())
            })?;
            Ok(Some(Value::Number(serde_json::Number::from(!a))))
        }
        "BIT_SHIFT_LEFT" => bit_binop(args, "BIT_SHIFT_LEFT", |a, b| {
            a.checked_shl(b.clamp(0, 63) as u32).unwrap_or(0)
        }),
        "BIT_SHIFT_RIGHT" => bit_binop(args, "BIT_SHIFT_RIGHT", |a, b| {
            a.checked_shr(b.clamp(0, 63) as u32).unwrap_or(0)
        }),
        "CLAMP" => {
            if args.len() != 3 {
                return Err(DbError::ExecutionError(
                    "CLAMP requires 3 arguments: value, min, max".to_string(),
                ));
            }
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let v = get_number(&args[0], name)?;
            let lo = get_number(&args[1], name)?;
            let hi = get_number(&args[2], name)?;
            Ok(Some(num(v.clamp(lo.min(hi), lo.max(hi)))))
        }
        "MIN" if args.len() >= 2 && args.iter().all(|v| v.is_number() || v.is_null()) => {
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let min = args
                .iter()
                .filter_map(|v| v.as_f64())
                .fold(f64::INFINITY, f64::min);
            Ok(Some(num(min)))
        }
        "MAX" if args.len() >= 2 && args.iter().all(|v| v.is_number() || v.is_null()) => {
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let max = args
                .iter()
                .filter_map(|v| v.as_f64())
                .fold(f64::NEG_INFINITY, f64::max);
            Ok(Some(num(max)))
        }
        // AQL: the smallest / largest non-null element in the type order
        // (null < bool < number < string < array < object). Numbers keep
        // coming back as floats, as they always have.
        "MIN" | "MINIMUM" | "MAX" | "MAXIMUM" if args.len() == 1 && args[0].is_array() => {
            let arr = args[0].as_array().unwrap();
            let want_min = name.starts_with("MIN");
            let mut best: Option<&Value> = None;
            for v in arr.iter().filter(|v| !v.is_null()) {
                let replace = match best {
                    None => true,
                    Some(cur) => {
                        let ord = compare_values(v, cur);
                        if want_min {
                            ord == std::cmp::Ordering::Less
                        } else {
                            ord == std::cmp::Ordering::Greater
                        }
                    }
                };
                if replace {
                    best = Some(v);
                }
            }
            Ok(Some(match best {
                None => Value::Null,
                Some(Value::Number(n)) => n.as_f64().map(num).unwrap_or(Value::Null),
                Some(v) => v.clone(),
            }))
        }
        "SUM" if args.len() == 1 && args[0].is_array() => {
            let arr = args[0].as_array().unwrap();
            let sum: f64 = arr.iter().filter_map(|v| v.as_f64()).sum();
            Ok(Some(num(sum)))
        }
        "PRODUCT" => {
            // AQL: product of the numbers in the array; null is skipped and
            // the empty product is 1.
            check_args(name, args, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let arr = args[0].as_array().ok_or_else(|| {
                DbError::ExecutionError("PRODUCT: argument must be an array".to_string())
            })?;
            let product: f64 = arr.iter().filter_map(|v| v.as_f64()).product();
            Ok(Some(num(product)))
        }
        "AVG" | "AVERAGE" if args.len() == 1 && args[0].is_array() => {
            let arr = args[0].as_array().unwrap();
            let (sum, n) = arr
                .iter()
                .filter_map(|v| v.as_f64())
                .fold((0.0f64, 0u64), |(s, n), x| (s + x, n + 1));
            if n == 0 {
                Ok(Some(Value::Null))
            } else {
                Ok(Some(num(sum / n as f64)))
            }
        }
        "RAND" | "RANDOM" if args.is_empty() => {
            use rand::Rng;
            let r: f64 = rand::thread_rng().gen();
            Ok(Some(num(r)))
        }
        "RANDOM_INT" | "RAND_INT" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "RANDOM_INT requires 2 arguments: min, max".to_string(),
                ));
            }
            use rand::Rng;
            let bound = |v: &Value| {
                as_int(v).ok_or_else(|| {
                    DbError::ExecutionError("RANDOM_INT: bounds must be integers".to_string())
                })
            };
            let (a, b) = (bound(&args[0])?, bound(&args[1])?);
            // `gen_range(min..=max)` panics when min > max; the bounds are
            // an unordered pair.
            let (min, max) = (a.min(b), a.max(b));
            let r: i64 = rand::thread_rng().gen_range(min..=max);
            Ok(Some(Value::Number(serde_json::Number::from(r))))
        }
        "MEDIAN" if args.len() == 1 && args[0].is_array() => {
            let mut nums = numbers_of(&args[0]);
            Ok(Some(median_of(&mut nums).map(num).unwrap_or(Value::Null)))
        }
        "PERCENTILE" | "QUANTILE" if (args.len() == 2 || args.len() == 3) && args[0].is_array() => {
            // Percentile rank must be within [0, 100]. Non-numeric or
            // out-of-range values yield Null (mirrors the degenerate-input
            // handling of the other statistical aggregates).
            let p = match args[1].as_f64() {
                Some(p) if (0.0..=100.0).contains(&p) => p,
                _ => return Ok(Some(Value::Null)),
            };
            // Optional third argument selects the method:
            //   "rank"          -> nearest-rank (default)
            //   "interpolation" -> linear interpolation between closest ranks
            let method = args.get(2).and_then(|v| v.as_str()).unwrap_or("rank");
            let mut nums = numbers_of(&args[0]);
            let interpolate = method.eq_ignore_ascii_case("interpolation");
            Ok(Some(
                percentile_of(&mut nums, p, interpolate)
                    .map(num)
                    .unwrap_or(Value::Null),
            ))
        }
        "VARIANCE"
        | "VAR_POP"
        | "VARIANCE_POPULATION"
        | "VAR_SAMP"
        | "VARIANCE_SAMPLE"
        | "STDDEV"
        | "STDDEV_POP"
        | "STDDEV_POPULATION"
        | "STDDEV_SAMP"
        | "STDDEV_SAMPLE"
            if args.len() == 1 && args[0].is_array() =>
        {
            let mut w = Welford::default();
            for x in args[0].as_array().unwrap().iter().filter_map(Value::as_f64) {
                w.push(x);
            }
            let sample = name.ends_with("SAMP") || name.ends_with("SAMPLE");
            let result = w.variance(sample).map(|v| {
                if name.starts_with("STDDEV") {
                    v.sqrt()
                } else {
                    v
                }
            });
            Ok(Some(result.map(num).unwrap_or(Value::Null)))
        }
        "COUNT_DISTINCT" | "COUNT_UNIQUE" | "UNIQUE_COUNT"
            if args.len() == 1 && args[0].is_array() =>
        {
            let arr = args[0].as_array().unwrap();
            let mut seen = crate::sdbql::executor::ValueSet::with_capacity(arr.len());
            let count = arr.iter().filter(|v| seen.insert(v)).count();
            Ok(Some(Value::Number(serde_json::Number::from(count))))
        }
        "DECAY_GAUSS" | "DECAY_EXP" | "DECAY_LINEAR" => decay(name, args),
        _ => Ok(None),
    }
}

/// AQL `DECAY_GAUSS` / `DECAY_EXP` / `DECAY_LINEAR(value, origin, scale,
/// offset, decay)`: 1 within `offset` of `origin`, and `decay` at `scale`
/// beyond that. `value` may be a number or an array of numbers.
fn decay(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() != 5 {
        return Err(DbError::ExecutionError(format!(
            "{} requires 5 arguments: value, origin, scale, offset, decay",
            name
        )));
    }
    let origin = get_number(&args[1], name)?;
    let scale = get_number(&args[2], name)?;
    let offset = get_number(&args[3], name)?;
    let decay = get_number(&args[4], name)?;
    if scale <= 0.0 {
        return Err(DbError::ExecutionError(format!(
            "{}: scale must be greater than 0",
            name
        )));
    }
    if offset < 0.0 {
        return Err(DbError::ExecutionError(format!(
            "{}: offset must be 0 or greater",
            name
        )));
    }
    if !(decay > 0.0 && decay < 1.0) {
        return Err(DbError::ExecutionError(format!(
            "{}: decay must be between 0 and 1 (exclusive)",
            name
        )));
    }
    let score = |v: f64| -> f64 {
        let d = ((v - origin).abs() - offset).max(0.0);
        match name {
            "DECAY_GAUSS" => decay.powf(d * d / (scale * scale)),
            "DECAY_EXP" => decay.powf(d / scale),
            _ => {
                let s = scale / (1.0 - decay);
                ((s - d) / s).max(0.0)
            }
        }
    };
    match &args[0] {
        Value::Array(values) => {
            let mut out = Vec::with_capacity(values.len());
            for v in values {
                out.push(num(score(get_number(v, name)?)));
            }
            Ok(Some(Value::Array(out)))
        }
        v => Ok(Some(num(score(get_number(v, name)?)))),
    }
}

/// Running mean and variance (Welford). One pass, no buffer, and none of the
/// cancellation that `Σx² − n·mean²` suffers on large values.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Welford {
    n: u64,
    mean: f64,
    m2: f64,
}

impl Welford {
    pub(crate) fn push(&mut self, x: f64) {
        self.n += 1;
        let delta = x - self.mean;
        self.mean += delta / self.n as f64;
        self.m2 += delta * (x - self.mean);
    }

    /// Population variance (0 for a single value) or sample variance (needs
    /// two values). `None` when there is not enough data.
    pub(crate) fn variance(&self, sample: bool) -> Option<f64> {
        if sample {
            (self.n >= 2).then(|| self.m2 / (self.n - 1) as f64)
        } else {
            (self.n >= 1).then(|| self.m2 / self.n as f64)
        }
    }
}

/// The numeric elements of an array value; anything else is skipped.
fn numbers_of(v: &Value) -> Vec<f64> {
    v.as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

/// Median by selection rather than a full sort. Reorders `nums`.
pub(crate) fn median_of(nums: &mut [f64]) -> Option<f64> {
    if nums.is_empty() {
        return None;
    }
    let mid = nums.len() / 2;
    let upper = *nums.select_nth_unstable_by(mid, f64::total_cmp).1;
    if !nums.len().is_multiple_of(2) {
        Some(upper)
    } else {
        let lower = nums[..mid]
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        Some((lower + upper) / 2.0)
    }
}

/// p-th percentile (0-100) by selection: nearest-rank, or linear
/// interpolation between the closest ranks. Reorders `nums`.
pub(crate) fn percentile_of(nums: &mut [f64], p: f64, interpolate: bool) -> Option<f64> {
    let n = nums.len();
    if n == 0 {
        return None;
    }
    if interpolate {
        // Excel PERCENTILE.INC / numpy "linear"; coincides with MEDIAN at 50.
        let pos = (p / 100.0) * (n - 1) as f64;
        let lower = (pos.floor() as usize).min(n - 1);
        let frac = pos - lower as f64;
        let lo = *nums.select_nth_unstable_by(lower, f64::total_cmp).1;
        if lower + 1 < n {
            let hi = nums[lower + 1..]
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min);
            Some(lo + frac * (hi - lo))
        } else {
            Some(lo)
        }
    } else {
        // Nearest-rank: rank = ceil(p/100 * n), 1-indexed.
        let rank = ((p / 100.0) * n as f64).ceil() as usize;
        let idx = rank.saturating_sub(1).min(n - 1);
        Some(*nums.select_nth_unstable_by(idx, f64::total_cmp).1)
    }
}

fn unary(name: &str, args: &[Value], f: impl Fn(f64) -> f64) -> DbResult<Option<Value>> {
    check_args(name, args, 1)?;
    let x = get_number(&args[0], name)?;
    Ok(Some(num(f(x))))
}

fn positive_log(
    name: &str,
    args: &[Value],
    label: &str,
    f: impl Fn(f64) -> f64,
) -> DbResult<Option<Value>> {
    check_args(name, args, 1)?;
    let x = get_number(&args[0], name)?;
    if x <= 0.0 {
        return Err(DbError::ExecutionError(format!(
            "{}: argument must be positive",
            label
        )));
    }
    Ok(Some(num(f(x))))
}

fn round_to(x: f64, decimals: i64) -> f64 {
    if decimals == 0 {
        return x.round();
    }
    if decimals < 0 {
        // Divide by an exact power of ten rather than multiply by an inexact
        // 0.01, so ROUND(1234.5, -2) is exactly 1200.
        let m = 10f64.powi((-decimals) as i32);
        let r = (x / m).round() * m;
        return if r.is_finite() { r } else { x };
    }
    let m = 10f64.powi(decimals as i32);
    let scaled = x * m;
    // So many decimals that scaling overflows: nothing left to round.
    if !scaled.is_finite() || m == 0.0 {
        return x;
    }
    let r = scaled.round() / m;
    if r.is_finite() {
        r
    } else {
        x
    }
}

fn bit_binop(
    args: &[Value],
    name: &str,
    op: impl FnOnce(i64, i64) -> i64,
) -> DbResult<Option<Value>> {
    if args.len() != 2 {
        return Err(DbError::ExecutionError(format!(
            "{} requires 2 arguments",
            name
        )));
    }
    if args[0].is_null() || args[1].is_null() {
        return Ok(Some(Value::Null));
    }
    let a = as_int(&args[0])
        .ok_or_else(|| DbError::ExecutionError(format!("{}: arguments must be integers", name)))?;
    let b = as_int(&args[1])
        .ok_or_else(|| DbError::ExecutionError(format!("{}: arguments must be integers", name)))?;
    Ok(Some(Value::Number(serde_json::Number::from(op(a, b)))))
}

fn get_number(v: &Value, func_name: &str) -> DbResult<f64> {
    v.as_f64()
        .ok_or_else(|| DbError::ExecutionError(format!("{}: argument must be a number", func_name)))
}

/// A finite result as a JSON number; anything else is `null`.
fn num(f: f64) -> Value {
    serde_json::Number::from_f64(f)
        .map(Value::Number)
        .unwrap_or(Value::Null)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, args: &[Value]) -> Value {
        evaluate(name, args)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .unwrap_or_else(|| panic!("{name} not handled"))
    }

    fn approx(v: &Value, expected: f64) -> bool {
        v.as_f64().is_some_and(|x| (x - expected).abs() < 1e-9)
    }

    #[test]
    fn non_finite_results_are_null() {
        assert_eq!(call("POW", &[json!(10), json!(400)]), Value::Null);
        assert_eq!(call("POW", &[json!(-8), json!(1.0 / 3.0)]), Value::Null);
        assert_eq!(call("EXP", &[json!(1000)]), Value::Null);
    }

    #[test]
    fn round_clamps_and_reads_float_decimals() {
        assert_eq!(call("ROUND", &[json!(1.5), json!(400)]), json!(1.5));
        assert_eq!(call("ROUND", &[json!(1.23456), json!(2.0)]), json!(1.23));
        assert_eq!(call("ROUND", &[json!(1234.5), json!(-2)]), json!(1200.0));
        assert_eq!(call("ROUND", &[json!(1234.5), json!(-400)]), json!(0.0));
        assert!(evaluate("ROUND", &[json!(1.0), json!(0.5)]).is_err());
    }

    #[test]
    fn random_int_accepts_reversed_bounds() {
        for _ in 0..50 {
            let r = call("RANDOM_INT", &[json!(10), json!(1)]).as_i64().unwrap();
            assert!((1..=10).contains(&r));
        }
        let r = call("RANDOM_INT", &[json!(3.0), json!(3.0)]);
        assert_eq!(r, json!(3));
        assert!(evaluate("RANDOM_INT", &[json!("a"), json!(3)]).is_err());
    }

    #[test]
    fn sign_trunc_exp2_aliases() {
        assert_eq!(call("SIGN", &[json!(-5)]), json!(-1));
        assert_eq!(call("SIGN", &[json!(0)]), json!(0));
        assert_eq!(call("SIGN", &[json!(0.1)]), json!(1));
        assert_eq!(call("TRUNC", &[json!(-2.7)]), json!(-2.0));
        assert_eq!(call("TRUNCATE", &[json!(2.7)]), json!(2.0));
        assert_eq!(call("EXP2", &[json!(10)]), json!(1024.0));
        assert!(approx(&call("DEG", &[json!(std::f64::consts::PI)]), 180.0));
        assert!(approx(&call("RAD", &[json!(180)]), std::f64::consts::PI));
        assert_eq!(call("PRODUCT", &[json!([2, 3, null, 4])]), json!(24.0));
        assert_eq!(call("PRODUCT", &[json!([])]), json!(1.0));
    }

    #[test]
    fn statistics_population_and_sample() {
        let a = json!([1, 2, 3, 4, 5]);
        assert!(approx(&call("VARIANCE", std::slice::from_ref(&a)), 2.0));
        assert!(approx(
            &call("VARIANCE_POPULATION", std::slice::from_ref(&a)),
            2.0
        ));
        assert!(approx(
            &call("VARIANCE_SAMPLE", std::slice::from_ref(&a)),
            2.5
        ));
        assert!(approx(
            &call("STDDEV", std::slice::from_ref(&a)),
            2f64.sqrt()
        ));
        assert!(approx(
            &call("STDDEV_SAMPLE", std::slice::from_ref(&a)),
            2.5f64.sqrt()
        ));
        assert!(approx(&call("STDDEV_POPULATION", &[a]), 2f64.sqrt()));
        // One value: population variance is 0, sample variance undefined.
        assert_eq!(call("VARIANCE", &[json!([7])]), json!(0.0));
        assert_eq!(call("VAR_SAMP", &[json!([7])]), Value::Null);
        assert_eq!(call("STDDEV", &[json!([])]), Value::Null);
    }

    #[test]
    fn median_and_percentile_by_selection() {
        assert_eq!(call("MEDIAN", &[json!([5, 1, 3])]), json!(3.0));
        assert_eq!(call("MEDIAN", &[json!([4, 1, 3, 2])]), json!(2.5));
        assert_eq!(
            call("PERCENTILE", &[json!([5, 4, 3, 2, 1]), json!(100)]),
            json!(5.0)
        );
        assert_eq!(
            call("PERCENTILE", &[json!([5, 4, 3, 2, 1]), json!(0)]),
            json!(1.0)
        );
        assert_eq!(
            call(
                "PERCENTILE",
                &[json!([4, 3, 2, 1]), json!(50), json!("interpolation")]
            ),
            json!(2.5)
        );
    }

    #[test]
    fn min_max_use_the_aql_order() {
        assert_eq!(call("MIN", &[json!([5, 2, 8])]), json!(2.0));
        assert_eq!(call("MAX", &[json!(["b", "a", null])]), json!("b"));
        assert_eq!(call("MIN", &[json!(["b", "a"])]), json!("a"));
        assert_eq!(call("MIN", &[json!([null])]), Value::Null);
    }

    #[test]
    fn decay_functions() {
        assert_eq!(
            call(
                "DECAY_GAUSS",
                &[json!(41), json!(40), json!(5), json!(5), json!(0.5)]
            ),
            json!(1.0)
        );
        assert!(approx(
            &call(
                "DECAY_GAUSS",
                &[json!(20), json!(40), json!(5), json!(5), json!(0.5)]
            ),
            0.5f64.powi(9)
        ));
        assert!(approx(
            &call(
                "DECAY_EXP",
                &[json!(2), json!(0), json!(10), json!(0), json!(0.2)]
            ),
            0.2f64.powf(0.2)
        ));
        assert!(approx(
            &call(
                "DECAY_LINEAR",
                &[json!(9.8), json!(0), json!(10), json!(0), json!(0.2)]
            ),
            0.216
        ));
        let arr = call(
            "DECAY_LINEAR",
            &[json!([0, 100]), json!(0), json!(10), json!(0), json!(0.5)],
        );
        assert_eq!(arr, json!([1.0, 0.0]));
        assert!(evaluate(
            "DECAY_EXP",
            &[json!(1), json!(0), json!(0), json!(0), json!(0.5)]
        )
        .is_err());
        assert!(evaluate(
            "DECAY_EXP",
            &[json!(1), json!(0), json!(1), json!(0), json!(1.0)]
        )
        .is_err());
    }

    #[test]
    fn bit_functions_accept_integral_floats() {
        assert_eq!(call("BIT_AND", &[json!(12.0), json!(10)]), json!(8));
        assert!(evaluate("BIT_AND", &[json!(1.5), json!(1)]).is_err());
    }

    #[test]
    fn welford_matches_two_pass() {
        let mut w = Welford::default();
        for x in [1e9 + 4.0, 1e9 + 7.0, 1e9 + 13.0, 1e9 + 16.0] {
            w.push(x);
        }
        assert!((w.variance(true).unwrap() - 30.0).abs() < 1e-6);
    }
}
