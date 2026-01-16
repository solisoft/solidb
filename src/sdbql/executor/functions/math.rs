use serde_json::Value;

use super::super::utils::number_from_f64;
use crate::error::{DbError, DbResult};

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "SQRT" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "SQRT requires 1 argument".to_string(),
                ));
            }
            let num = args[0].as_f64().ok_or_else(|| {
                DbError::ExecutionError("SQRT: argument must be a number".to_string())
            })?;
            if num < 0.0 {
                return Err(DbError::ExecutionError(
                    "SQRT: cannot take square root of negative number".to_string(),
                ));
            }
            Ok(Some(Value::Number(number_from_f64(num.sqrt()))))
        }

        "POW" | "POWER" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "POW requires 2 arguments".to_string(),
                ));
            }
            let base = args[0].as_f64().ok_or_else(|| {
                DbError::ExecutionError("POW: base must be a number".to_string())
            })?;
            let exp = args[1].as_f64().ok_or_else(|| {
                DbError::ExecutionError("POW: exponent must be a number".to_string())
            })?;

            Ok(Some(Value::Number(number_from_f64(base.powf(exp)))))
        }

        "RANDOM" => {
            if !args.is_empty() {
                return Err(DbError::ExecutionError(
                    "RANDOM takes no arguments".to_string(),
                ));
            }
            use rand::Rng;
            let random_val: f64 = rand::thread_rng().gen();
            Ok(Some(Value::Number(number_from_f64(random_val))))
        }
        
        // ROUND, ABS, FLOOR, CEIL were in evaluate_function_with_values
        "ROUND" => {
            if args.is_empty() || args.len() > 2 {
                return Err(DbError::ExecutionError(
                    "ROUND requires 1-2 arguments".to_string(),
                ));
            }
            let num = args[0].as_f64().ok_or_else(|| {
                DbError::ExecutionError("ROUND: first argument must be a number".to_string())
            })?;
            let precision = if args.len() > 1 {
                args[1].as_i64().unwrap_or(0) as i32
            } else {
                0
            };
            let factor = 10_f64.powi(precision);
            let rounded = (num * factor).round() / factor;
            Ok(Some(Value::Number(number_from_f64(rounded))))
        }
        "ABS" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "ABS requires 1 argument".to_string(),
                ));
            }
            let num = args[0].as_f64().ok_or_else(|| {
                DbError::ExecutionError("ABS: argument must be a number".to_string())
            })?;
            Ok(Some(Value::Number(number_from_f64(num.abs()))))
        }
        "FLOOR" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "FLOOR requires 1 argument".to_string(),
                ));
            }
            let num = args[0].as_f64().ok_or_else(|| {
                DbError::ExecutionError("FLOOR: argument must be a number".to_string())
            })?;
            Ok(Some(Value::Number(number_from_f64(num.floor()))))
        }
        "CEIL" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "CEIL requires 1 argument".to_string(),
                ));
            }
            let num = args[0].as_f64().ok_or_else(|| {
                DbError::ExecutionError("CEIL: argument must be a number".to_string())
            })?;
            Ok(Some(Value::Number(number_from_f64(num.ceil()))))
        }

        _ => Ok(None),
    }
}
