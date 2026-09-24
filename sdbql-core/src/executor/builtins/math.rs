//! Math builtin functions.

use serde_json::Value;

use crate::error::{SdbqlError, SdbqlResult};

/// Call a math function. Returns None if function not found.
pub fn call(name: &str, args: &[Value]) -> SdbqlResult<Option<Value>> {
    let result = match name {
        "FLOOR" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Some(Value::Number(num_from_f64(num.floor())))
        }

        "CEIL" | "CEILING" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Some(Value::Number(num_from_f64(num.ceil())))
        }

        "ROUND" => {
            if args.is_empty() {
                return Err(SdbqlError::ExecutionError(
                    "ROUND requires 1-2 arguments".to_string(),
                ));
            }
            let num = get_number(&args[0], name)?;
            let decimals = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0);
            let multiplier = 10f64.powi(decimals as i32);
            let rounded = (num * multiplier).round() / multiplier;
            Some(Value::Number(num_from_f64(rounded)))
        }

        "ABS" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Some(Value::Number(num_from_f64(num.abs())))
        }

        "SQRT" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            if num < 0.0 {
                return Err(SdbqlError::ExecutionError(
                    "SQRT: argument must be non-negative".to_string(),
                ));
            }
            Some(Value::Number(num_from_f64(num.sqrt())))
        }

        "POW" | "POWER" => {
            if args.len() != 2 {
                return Err(SdbqlError::ExecutionError(
                    "POW requires 2 arguments: base, exponent".to_string(),
                ));
            }
            let base = get_number(&args[0], name)?;
            let exp = get_number(&args[1], name)?;
            Some(Value::Number(num_from_f64(base.powf(exp))))
        }

        "LOG" | "LN" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            if num <= 0.0 {
                return Err(SdbqlError::ExecutionError(
                    "LOG: argument must be positive".to_string(),
                ));
            }
            Some(Value::Number(num_from_f64(num.ln())))
        }

        "LOG10" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            if num <= 0.0 {
                return Err(SdbqlError::ExecutionError(
                    "LOG10: argument must be positive".to_string(),
                ));
            }
            Some(Value::Number(num_from_f64(num.log10())))
        }

        "LOG2" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            if num <= 0.0 {
                return Err(SdbqlError::ExecutionError(
                    "LOG2: argument must be positive".to_string(),
                ));
            }
            Some(Value::Number(num_from_f64(num.log2())))
        }

        "EXP" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Some(Value::Number(num_from_f64(num.exp())))
        }

        "SIN" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Some(Value::Number(num_from_f64(num.sin())))
        }

        "COS" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Some(Value::Number(num_from_f64(num.cos())))
        }

        "TAN" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Some(Value::Number(num_from_f64(num.tan())))
        }

        "ASIN" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            if !(-1.0..=1.0).contains(&num) {
                return Err(SdbqlError::ExecutionError(
                    "ASIN: argument must be between -1 and 1".to_string(),
                ));
            }
            Some(Value::Number(num_from_f64(num.asin())))
        }

        "ACOS" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            if !(-1.0..=1.0).contains(&num) {
                return Err(SdbqlError::ExecutionError(
                    "ACOS: argument must be between -1 and 1".to_string(),
                ));
            }
            Some(Value::Number(num_from_f64(num.acos())))
        }

        "ATAN" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Some(Value::Number(num_from_f64(num.atan())))
        }

        "ATAN2" => {
            if args.len() != 2 {
                return Err(SdbqlError::ExecutionError(
                    "ATAN2 requires 2 arguments: y, x".to_string(),
                ));
            }
            let y = get_number(&args[0], name)?;
            let x = get_number(&args[1], name)?;
            Some(Value::Number(num_from_f64(y.atan2(x))))
        }

        "DEGREES" | "DEG" => {
            check_args(name, args, 1)?;
            let radians = get_number(&args[0], name)?;
            Some(Value::Number(num_from_f64(radians.to_degrees())))
        }

        "RADIANS" | "RAD" => {
            check_args(name, args, 1)?;
            let degrees = get_number(&args[0], name)?;
            Some(Value::Number(num_from_f64(degrees.to_radians())))
        }

        "PI" => Some(Value::Number(num_from_f64(std::f64::consts::PI))),

        "E" => Some(Value::Number(num_from_f64(std::f64::consts::E))),

        "SIGN" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            let sign = if num > 0.0 {
                1
            } else if num < 0.0 {
                -1
            } else {
                0
            };
            Some(Value::Number(serde_json::Number::from(sign)))
        }

        "TRUNCATE" | "TRUNC" => {
            check_args(name, args, 1)?;
            let num = get_number(&args[0], name)?;
            Some(Value::Number(num_from_f64(num.trunc())))
        }

        "MOD" => {
            if args.len() != 2 {
                return Err(SdbqlError::ExecutionError(
                    "MOD requires 2 arguments: dividend, divisor".to_string(),
                ));
            }
            let a = get_number(&args[0], name)?;
            let b = get_number(&args[1], name)?;
            if b == 0.0 {
                return Err(SdbqlError::ExecutionError(
                    "MOD: division by zero".to_string(),
                ));
            }
            Some(Value::Number(num_from_f64(a % b)))
        }

        "CLAMP" => {
            if args.len() != 3 {
                return Err(SdbqlError::ExecutionError(
                    "CLAMP requires 3 arguments: value, min, max".to_string(),
                ));
            }
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let v = get_number(&args[0], name)?;
            let lo = get_number(&args[1], name)?;
            let hi = get_number(&args[2], name)?;
            Some(Value::Number(num_from_f64(v.clamp(lo.min(hi), lo.max(hi)))))
        }

        "MEDIAN" => {
            let Some(mut nums) = stats_input(name, args, 1, 1)? else {
                return Ok(Some(Value::Null));
            };
            if nums.is_empty() {
                return Ok(Some(Value::Null));
            }
            nums.sort_by(f64::total_cmp);
            let mid = nums.len() / 2;
            let median = if nums.len().is_multiple_of(2) {
                (nums[mid - 1] + nums[mid]) / 2.0
            } else {
                nums[mid]
            };
            Some(Value::Number(num_from_f64(median)))
        }

        "PERCENTILE" | "QUANTILE" => {
            // PERCENTILE(array, p, method): p in [0, 100], method "rank"
            // (nearest rank, default) or "interpolation".
            let Some(mut nums) = stats_input(name, args, 2, 3)? else {
                return Ok(Some(Value::Null));
            };
            let p = match args[1].as_f64() {
                Some(p) if (0.0..=100.0).contains(&p) => p,
                _ => return Ok(Some(Value::Null)),
            };
            if nums.is_empty() {
                return Ok(Some(Value::Null));
            }
            nums.sort_by(f64::total_cmp);
            let n = nums.len();
            let method = args.get(2).and_then(Value::as_str).unwrap_or("rank");
            let result = if method.eq_ignore_ascii_case("interpolation") {
                let pos = (p / 100.0) * (n - 1) as f64;
                let lower = (pos.floor() as usize).min(n - 1);
                let frac = pos - lower as f64;
                if lower + 1 < n {
                    nums[lower] + frac * (nums[lower + 1] - nums[lower])
                } else {
                    nums[lower]
                }
            } else {
                let rank = ((p / 100.0) * n as f64).ceil() as usize;
                nums[rank.saturating_sub(1).min(n - 1)]
            };
            Some(Value::Number(num_from_f64(result)))
        }

        // AQL: VARIANCE and STDDEV are the population forms.
        "VARIANCE"
        | "VARIANCE_POPULATION"
        | "VAR_POP"
        | "VARIANCE_SAMPLE"
        | "VAR_SAMP"
        | "STDDEV"
        | "STDDEV_POPULATION"
        | "STDDEV_POP"
        | "STDDEV_SAMPLE"
        | "STDDEV_SAMP" => {
            let Some(nums) = stats_input(name, args, 1, 1)? else {
                return Ok(Some(Value::Null));
            };
            let sample = name.ends_with("_SAMPLE") || name.ends_with("_SAMP");
            match variance(&nums, sample) {
                None => Some(Value::Null),
                Some(var) => {
                    let out = if name.starts_with("STDDEV") {
                        var.sqrt()
                    } else {
                        var
                    };
                    Some(Value::Number(num_from_f64(out)))
                }
            }
        }

        _ => None,
    };

    Ok(result)
}

/// The numbers of the first (array) argument; `None` when it is null.
fn stats_input(
    name: &str,
    args: &[Value],
    min: usize,
    max: usize,
) -> SdbqlResult<Option<Vec<f64>>> {
    if args.len() < min || args.len() > max {
        return Err(SdbqlError::ExecutionError(format!(
            "{} requires {}-{} argument(s)",
            name, min, max
        )));
    }
    match &args[0] {
        Value::Null => Ok(None),
        Value::Array(arr) => Ok(Some(arr.iter().filter_map(Value::as_f64).collect())),
        _ => Err(SdbqlError::ExecutionError(format!(
            "{}: first argument must be an array",
            name
        ))),
    }
}

/// Welford's one-pass variance. Population needs one value, sample two.
fn variance(nums: &[f64], sample: bool) -> Option<f64> {
    let mut n = 0usize;
    let mut mean = 0.0;
    let mut m2 = 0.0;
    for &x in nums {
        n += 1;
        let d = x - mean;
        mean += d / n as f64;
        m2 += d * (x - mean);
    }
    if sample {
        (n >= 2).then(|| m2 / (n - 1) as f64)
    } else {
        (n >= 1).then(|| m2 / n as f64)
    }
}

fn get_number(v: &Value, func_name: &str) -> SdbqlResult<f64> {
    v.as_f64().ok_or_else(|| {
        SdbqlError::ExecutionError(format!("{}: argument must be a number", func_name))
    })
}

fn num_from_f64(f: f64) -> serde_json::Number {
    serde_json::Number::from_f64(f).unwrap_or_else(|| serde_json::Number::from(f as i64))
}

fn check_args(name: &str, args: &[Value], expected: usize) -> SdbqlResult<()> {
    if args.len() != expected {
        return Err(SdbqlError::ExecutionError(format!(
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

    #[test]
    fn test_floor_ceil() {
        assert_eq!(call("FLOOR", &[json!(3.7)]).unwrap(), Some(json!(3.0)));
        assert_eq!(call("CEIL", &[json!(3.2)]).unwrap(), Some(json!(4.0)));
    }

    #[test]
    fn test_round() {
        assert_eq!(call("ROUND", &[json!(3.456)]).unwrap(), Some(json!(3.0)));
        assert_eq!(
            call("ROUND", &[json!(3.456), json!(2)]).unwrap(),
            Some(json!(3.46))
        );
    }

    #[test]
    fn test_abs() {
        assert_eq!(call("ABS", &[json!(-5)]).unwrap(), Some(json!(5.0)));
        assert_eq!(call("ABS", &[json!(5)]).unwrap(), Some(json!(5.0)));
    }

    #[test]
    fn test_sqrt() {
        assert_eq!(call("SQRT", &[json!(16)]).unwrap(), Some(json!(4.0)));
    }

    #[test]
    fn test_pow() {
        assert_eq!(
            call("POW", &[json!(2), json!(3)]).unwrap(),
            Some(json!(8.0))
        );
    }

    #[test]
    fn test_trig() {
        assert_eq!(call("SIN", &[json!(0)]).unwrap(), Some(json!(0.0)));
        assert_eq!(call("COS", &[json!(0)]).unwrap(), Some(json!(1.0)));
    }

    #[test]
    fn stats_functions() {
        assert_eq!(
            call("MEDIAN", &[json!([1, 5, 10])]).unwrap(),
            Some(json!(5.0))
        );
        assert_eq!(
            call("MEDIAN", &[json!([1, 2, 3, 4])]).unwrap(),
            Some(json!(2.5))
        );
        assert_eq!(call("MEDIAN", &[json!([])]).unwrap(), Some(Value::Null));
        assert_eq!(call("MEDIAN", &[Value::Null]).unwrap(), Some(Value::Null));
        assert_eq!(
            call(
                "PERCENTILE",
                &[json!([1, 2, 3, 4]), json!(50), json!("interpolation")]
            )
            .unwrap(),
            Some(json!(2.5))
        );
        assert_eq!(
            call("PERCENTILE", &[json!([1, 2, 3, 4]), json!(100)]).unwrap(),
            Some(json!(4.0))
        );
        assert_eq!(
            call(
                "PERCENTILE",
                &[json!([1]), json!(0), json!("interpolation")]
            )
            .unwrap(),
            Some(json!(1.0))
        );
        assert_eq!(
            call("PERCENTILE", &[json!([1]), json!(101)]).unwrap(),
            Some(Value::Null)
        );
        let five = json!([1, 2, 3, 4, 5]);
        assert_eq!(call("VARIANCE", &[five.clone()]).unwrap(), Some(json!(2.0)));
        assert_eq!(
            call("VARIANCE_SAMPLE", &[five.clone()]).unwrap(),
            Some(json!(2.5))
        );
        // AQL: STDDEV is the population deviation.
        let sd = call("STDDEV", &[five.clone()]).unwrap().unwrap();
        assert!((sd.as_f64().unwrap() - 2f64.sqrt()).abs() < 1e-12);
        let sd = call("STDDEV_SAMPLE", &[five]).unwrap().unwrap();
        assert!((sd.as_f64().unwrap() - 2.5f64.sqrt()).abs() < 1e-12);
        assert_eq!(call("VARIANCE", &[json!([7])]).unwrap(), Some(json!(0.0)));
        assert_eq!(
            call("VARIANCE_SAMPLE", &[json!([7])]).unwrap(),
            Some(Value::Null)
        );
    }

    #[test]
    fn clamp_and_aliases() {
        assert_eq!(
            call("CLAMP", &[json!(15), json!(0), json!(10)]).unwrap(),
            Some(json!(10.0))
        );
        assert_eq!(call("SIGN", &[json!(-5)]).unwrap(), Some(json!(-1)));
        assert_eq!(call("DEG", &[json!(0)]).unwrap(), Some(json!(0.0)));
    }

    #[test]
    fn test_constants() {
        let pi = call("PI", &[]).unwrap().unwrap();
        assert!(pi.as_f64().unwrap() > 3.14 && pi.as_f64().unwrap() < 3.15);

        let e = call("E", &[]).unwrap().unwrap();
        assert!(e.as_f64().unwrap() > 2.71 && e.as_f64().unwrap() < 2.72);
    }
}
