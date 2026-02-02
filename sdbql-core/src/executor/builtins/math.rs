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

        "DEGREES" => {
            check_args(name, args, 1)?;
            let radians = get_number(&args[0], name)?;
            Some(Value::Number(num_from_f64(radians.to_degrees())))
        }

        "RADIANS" => {
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

        _ => None,
    };

    Ok(result)
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
    fn test_constants() {
        let pi = call("PI", &[]).unwrap().unwrap();
        assert!(pi.as_f64().unwrap() > 3.14 && pi.as_f64().unwrap() < 3.15);

        let e = call("E", &[]).unwrap().unwrap();
        assert!(e.as_f64().unwrap() > 2.71 && e.as_f64().unwrap() < 2.72);
    }
}
