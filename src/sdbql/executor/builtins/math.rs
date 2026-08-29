//! Math functions for SDBQL.
//!
//! FLOOR, CEIL, ROUND, ABS, SQRT, POW, LOG, SIN, COS, TAN, etc.

use crate::error::{DbError, DbResult};
use serde_json::Value;

/// Evaluate math functions
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "FLOOR" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Ok(Some(Value::Number(num_from_f64(num.floor()))))
        }
        "CEIL" | "CEILING" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Ok(Some(Value::Number(num_from_f64(num.ceil()))))
        }
        "ROUND" => {
            if args.is_empty() {
                return Err(DbError::ExecutionError(
                    "ROUND requires 1-2 arguments".to_string(),
                ));
            }
            let num = get_number(&args[0], name)?;
            let decimals = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0);
            let multiplier = 10f64.powi(decimals as i32);
            let rounded = (num * multiplier).round() / multiplier;
            Ok(Some(Value::Number(num_from_f64(rounded))))
        }
        "ABS" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Ok(Some(Value::Number(num_from_f64(num.abs()))))
        }
        "SQRT" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            if num < 0.0 {
                return Err(DbError::ExecutionError(
                    "SQRT: argument must be non-negative".to_string(),
                ));
            }
            Ok(Some(Value::Number(num_from_f64(num.sqrt()))))
        }
        "POW" | "POWER" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "POW requires 2 arguments: base, exponent".to_string(),
                ));
            }
            let base = get_number(&args[0], name)?;
            let exp = get_number(&args[1], name)?;
            Ok(Some(Value::Number(num_from_f64(base.powf(exp)))))
        }
        "LOG" | "LN" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            if num <= 0.0 {
                return Err(DbError::ExecutionError(
                    "LOG: argument must be positive".to_string(),
                ));
            }
            Ok(Some(Value::Number(num_from_f64(num.ln()))))
        }
        "LOG10" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            if num <= 0.0 {
                return Err(DbError::ExecutionError(
                    "LOG10: argument must be positive".to_string(),
                ));
            }
            Ok(Some(Value::Number(num_from_f64(num.log10()))))
        }
        "LOG2" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            if num <= 0.0 {
                return Err(DbError::ExecutionError(
                    "LOG2: argument must be positive".to_string(),
                ));
            }
            Ok(Some(Value::Number(num_from_f64(num.log2()))))
        }
        "EXP" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Ok(Some(Value::Number(num_from_f64(num.exp()))))
        }
        "SIN" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Ok(Some(Value::Number(num_from_f64(num.sin()))))
        }
        "COS" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Ok(Some(Value::Number(num_from_f64(num.cos()))))
        }
        "TAN" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Ok(Some(Value::Number(num_from_f64(num.tan()))))
        }
        "ASIN" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            if !(-1.0..=1.0).contains(&num) {
                return Err(DbError::ExecutionError(
                    "ASIN: argument must be between -1 and 1".to_string(),
                ));
            }
            Ok(Some(Value::Number(num_from_f64(num.asin()))))
        }
        "ACOS" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            if !(-1.0..=1.0).contains(&num) {
                return Err(DbError::ExecutionError(
                    "ACOS: argument must be between -1 and 1".to_string(),
                ));
            }
            Ok(Some(Value::Number(num_from_f64(num.acos()))))
        }
        "ATAN" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Ok(Some(Value::Number(num_from_f64(num.atan()))))
        }
        "ATAN2" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "ATAN2 requires 2 arguments: y, x".to_string(),
                ));
            }
            let y = get_number(&args[0], name)?;
            let x = get_number(&args[1], name)?;
            Ok(Some(Value::Number(num_from_f64(y.atan2(x)))))
        }
        "DEGREES" => {
            check_args(name, args, 1)?;
            let radians = get_number(&args[0], name)?;
            Ok(Some(Value::Number(num_from_f64(radians.to_degrees()))))
        }
        "RADIANS" => {
            check_args(name, args, 1)?;
            let degrees = get_number(&args[0], name)?;
            Ok(Some(Value::Number(num_from_f64(degrees.to_radians()))))
        }
        "PI" => Ok(Some(Value::Number(num_from_f64(std::f64::consts::PI)))),
        "E" => Ok(Some(Value::Number(num_from_f64(std::f64::consts::E)))),
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
            Ok(Some(Value::Number(num_from_f64(a % b))))
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
            let a = args[0].as_i64().ok_or_else(|| {
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
            Ok(Some(Value::Number(num_from_f64(
                v.clamp(lo.min(hi), lo.max(hi)),
            ))))
        }
        "MIN" if args.len() >= 2 && args.iter().all(|v| v.is_number() || v.is_null()) => {
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let min = args
                .iter()
                .filter_map(|v| v.as_f64())
                .fold(f64::INFINITY, f64::min);
            Ok(Some(Value::Number(num_from_f64(min))))
        }
        "MAX" if args.len() >= 2 && args.iter().all(|v| v.is_number() || v.is_null()) => {
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let max = args
                .iter()
                .filter_map(|v| v.as_f64())
                .fold(f64::NEG_INFINITY, f64::max);
            Ok(Some(Value::Number(num_from_f64(max))))
        }
        "MIN" if args.len() == 1 && args[0].is_array() => {
            let arr = args[0].as_array().unwrap();
            let min = arr
                .iter()
                .filter_map(|v| v.as_f64())
                .fold(f64::INFINITY, f64::min);
            if min.is_infinite() {
                Ok(Some(Value::Null))
            } else {
                Ok(Some(Value::Number(num_from_f64(min))))
            }
        }
        "MAX" if args.len() == 1 && args[0].is_array() => {
            let arr = args[0].as_array().unwrap();
            let max = arr
                .iter()
                .filter_map(|v| v.as_f64())
                .fold(f64::NEG_INFINITY, f64::max);
            if max.is_infinite() {
                Ok(Some(Value::Null))
            } else {
                Ok(Some(Value::Number(num_from_f64(max))))
            }
        }
        "SUM" if args.len() == 1 && args[0].is_array() => {
            let arr = args[0].as_array().unwrap();
            let sum: f64 = arr.iter().filter_map(|v| v.as_f64()).sum();
            Ok(Some(Value::Number(num_from_f64(sum))))
        }
        "AVG" | "AVERAGE" if args.len() == 1 && args[0].is_array() => {
            let arr = args[0].as_array().unwrap();
            let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
            if nums.is_empty() {
                Ok(Some(Value::Null))
            } else {
                let avg = nums.iter().sum::<f64>() / nums.len() as f64;
                Ok(Some(Value::Number(num_from_f64(avg))))
            }
        }
        "RAND" | "RANDOM" if args.is_empty() => {
            use rand::Rng;
            let r: f64 = rand::thread_rng().gen();
            Ok(Some(Value::Number(num_from_f64(r))))
        }
        "RANDOM_INT" | "RAND_INT" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "RANDOM_INT requires 2 arguments: min, max".to_string(),
                ));
            }
            use rand::Rng;
            let min = args[0].as_i64().unwrap_or(0);
            let max = args[1].as_i64().unwrap_or(100);
            let r: i64 = rand::thread_rng().gen_range(min..=max);
            Ok(Some(Value::Number(serde_json::Number::from(r))))
        }
        "MEDIAN" if args.len() == 1 && args[0].is_array() => {
            let arr = args[0].as_array().unwrap();
            let mut nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
            if nums.is_empty() {
                return Ok(Some(Value::Null));
            }
            nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mid = nums.len() / 2;
            let median = if nums.len().is_multiple_of(2) {
                (nums[mid - 1] + nums[mid]) / 2.0
            } else {
                nums[mid]
            };
            Ok(Some(Value::Number(num_from_f64(median))))
        }
        "PERCENTILE" | "QUANTILE" if (args.len() == 2 || args.len() == 3) && args[0].is_array() => {
            let arr = args[0].as_array().unwrap();
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
            let mut nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
            if nums.is_empty() {
                return Ok(Some(Value::Null));
            }
            nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let n = nums.len();
            let result = if method.eq_ignore_ascii_case("interpolation") {
                // Linear interpolation (Excel PERCENTILE.INC / numpy "linear").
                // Coincides with MEDIAN at p = 50.
                let pos = (p / 100.0) * (n - 1) as f64;
                let lower = pos.floor() as usize;
                let frac = pos - lower as f64;
                if lower + 1 < n {
                    nums[lower] + frac * (nums[lower + 1] - nums[lower])
                } else {
                    nums[lower]
                }
            } else {
                // Nearest-rank method: rank = ceil(p/100 * n), 1-indexed.
                let rank = ((p / 100.0) * n as f64).ceil() as usize;
                let idx = rank.saturating_sub(1).min(n - 1);
                nums[idx]
            };
            Ok(Some(Value::Number(num_from_f64(result))))
        }
        "VARIANCE" | "VAR_POP" | "VAR_SAMP" if args.len() == 1 && args[0].is_array() => {
            let arr = args[0].as_array().unwrap();
            let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
            if nums.len() < 2 {
                return Ok(Some(Value::Null));
            }
            let mean = nums.iter().sum::<f64>() / nums.len() as f64;
            let denom = if name == "VAR_SAMP" {
                (nums.len() - 1) as f64
            } else {
                nums.len() as f64
            };
            let variance = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / denom;
            Ok(Some(Value::Number(num_from_f64(variance))))
        }
        "STDDEV" | "STDDEV_POP" | "STDDEV_SAMP" | "STDDEV_POPULATION"
            if args.len() == 1 && args[0].is_array() =>
        {
            let arr = args[0].as_array().unwrap();
            let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
            if nums.len() < 2 {
                return Ok(Some(Value::Null));
            }
            let mean = nums.iter().sum::<f64>() / nums.len() as f64;
            let denom = if name == "STDDEV_SAMP" {
                (nums.len() - 1) as f64
            } else {
                nums.len() as f64
            };
            let variance = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / denom;
            let stddev = variance.sqrt();
            Ok(Some(Value::Number(num_from_f64(stddev))))
        }
        "COUNT_DISTINCT" | "COUNT_UNIQUE" | "UNIQUE_COUNT"
            if args.len() == 1 && args[0].is_array() =>
        {
            let arr = args[0].as_array().unwrap();
            let mut seen = crate::sdbql::executor::ValueSet::with_capacity(arr.len());
            let count = arr.iter().filter(|v| seen.insert(v)).count();
            Ok(Some(Value::Number(serde_json::Number::from(count))))
        }
        _ => Ok(None),
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
    let a = args[0]
        .as_i64()
        .ok_or_else(|| DbError::ExecutionError(format!("{}: arguments must be integers", name)))?;
    let b = args[1]
        .as_i64()
        .ok_or_else(|| DbError::ExecutionError(format!("{}: arguments must be integers", name)))?;
    Ok(Some(Value::Number(serde_json::Number::from(op(a, b)))))
}

fn get_number(v: &Value, func_name: &str) -> DbResult<f64> {
    v.as_f64()
        .ok_or_else(|| DbError::ExecutionError(format!("{}: argument must be a number", func_name)))
}

fn num_from_f64(f: f64) -> serde_json::Number {
    serde_json::Number::from_f64(f).unwrap_or_else(|| serde_json::Number::from(f as i64))
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
