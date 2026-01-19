//! Mathematical functions for SDBQL

use serde_json::Value;

use crate::error::{DbError, DbResult};
use crate::sdbql::executor::utils::number_from_f64;

#[inline]
fn make_number(f: f64) -> Value {
    Value::Number(number_from_f64(f))
}

/// Evaluate mathematical functions
pub fn evaluate_math_fn(name: &str, args: &[Value]) -> DbResult<Value> {
    match name {
        // Rounding functions
        "ROUND" | "FLOOR" | "CEIL" | "CEILING" => {
            if args.len() != 1 && args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "ROUND/FLOOR/CEIL requires 1-2 arguments".to_string(),
                ));
            }
            let value = match &args[0] {
                Value::Number(n) => n.as_f64().unwrap_or(0.0),
                Value::String(s) => s.parse().unwrap_or(0.0),
                _ => 0.0,
            };
            let decimals = if args.len() > 1 {
                args[1].as_u64().unwrap_or(0) as usize
            } else {
                0
            };
            let multiplier = 10f64.powi(decimals as i32);
            let result = match name {
                "ROUND" => (value * multiplier).round() / multiplier,
                "FLOOR" | "CEIL" | "CEILING" => (value * multiplier).floor() / multiplier,
                _ => value,
            };
            Ok(make_number(result))
        }

        "ABS" | "ABSOLUTE" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError("ABS requires 1 argument".to_string()));
            }
            let value = match &args[0] {
                Value::Number(n) => n.as_f64().unwrap_or(0.0),
                Value::String(s) => s.parse().unwrap_or(0.0),
                _ => 0.0,
            };
            Ok(make_number(value.abs()))
        }

        "CLAMP" => {
            if args.len() != 3 {
                return Err(DbError::ExecutionError(
                    "CLAMP requires 3 arguments: value, min, max".to_string(),
                ));
            }
            let value = match &args[0] {
                Value::Number(n) => n.as_f64().unwrap_or(0.0),
                _ => 0.0,
            };
            let min = match &args[1] {
                Value::Number(n) => n.as_f64().unwrap_or(0.0),
                _ => 0.0,
            };
            let max = match &args[2] {
                Value::Number(n) => n.as_f64().unwrap_or(0.0),
                _ => 0.0,
            };
            Ok(make_number(value.clamp(min, max)))
        }

        "SIGN" | "SIGNUM" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError("SIGN requires 1 argument".to_string()));
            }
            let value = match &args[0] {
                Value::Number(n) => n.as_f64().unwrap_or(0.0),
                _ => 0.0,
            };
            Ok(make_number(value.signum() as f64))
        }

        "MOD" | "MODULO" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "MOD requires 2 arguments".to_string(),
                ));
            }
            let a = match &args[0] {
                Value::Number(n) => n.as_f64().unwrap_or(0.0),
                _ => 0.0,
            };
            let b = match &args[1] {
                Value::Number(n) => n.as_f64().unwrap_or(0.0),
                _ => 0.0,
            };
            if b == 0.0 {
                return Err(DbError::ExecutionError("MOD: division by zero".to_string()));
            }
            Ok(make_number(a % b))
        }

        "RANDOM_INT" | "RAND_INT" => {
            let min = if args.len() > 0 {
                match &args[0] {
                    Value::Number(n) => n.as_i64().unwrap_or(0),
                    _ => 0,
                }
            } else {
                0
            };
            let max = if args.len() > 1 {
                match &args[1] {
                    Value::Number(n) => n.as_i64().unwrap_or(i64::MAX),
                    _ => i64::MAX,
                }
            } else {
                i64::MAX
            };
            let range = max - min;
            let random_value = if range > 0 {
                min + (rand::random::<u64>() % range as u64) as i64
            } else {
                min
            };
            Ok(make_number(random_value as f64))
        }

        "RANGE" => {
            if args.len() < 1 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "RANGE requires 1-3 arguments".to_string(),
                ));
            }
            let start = match args.get(0).and_then(|v| v.as_i64()) {
                Some(n) => n,
                None => {
                    return Err(DbError::ExecutionError(
                        "RANGE: first argument must be an integer".to_string(),
                    ));
                }
            };
            let end = match args.get(1).and_then(|v| v.as_i64()) {
                Some(n) => n,
                None => start + 10,
            };
            let step = match args.get(2).and_then(|v| v.as_i64()) {
                Some(n) => n,
                None => 1,
            };

            if step == 0 {
                return Err(DbError::ExecutionError(
                    "RANGE: step cannot be zero".to_string(),
                ));
            }

            let values: Vec<Value> = if step > 0 {
                (start..end).step_by(step as usize).map(|n| make_number(n as f64)).collect()
            } else {
                (start..end).step_by((-step) as usize).map(|n| make_number(n as f64)).collect()
            };
            Ok(Value::Array(values))
        }

        _ => Err(DbError::ExecutionError(format!(
            "Unknown math function: {}",
            name
        ))),
    }
}
