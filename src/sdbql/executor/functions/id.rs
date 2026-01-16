use crate::error::{DbError, DbResult};
use serde_json::Value;

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "UUID" | "UUID_V4" => {
            if !args.is_empty() {
                return Err(DbError::ExecutionError(
                    "UUID takes no arguments".to_string(),
                ));
            }
            let id = uuid::Uuid::new_v4().to_string();
            Ok(Some(Value::String(id)))
        }
        "UUID_V7" => {
            if !args.is_empty() {
                return Err(DbError::ExecutionError(
                    "UUID_V7 takes no arguments".to_string(),
                ));
            }
            let id = uuid::Uuid::now_v7().to_string();
            Ok(Some(Value::String(id)))
        }
        "ULID" => {
            if !args.is_empty() {
                return Err(DbError::ExecutionError(
                    "ULID takes no arguments".to_string(),
                ));
            }
            let id = ulid::Ulid::new().to_string();
            Ok(Some(Value::String(id)))
        }
        "NANOID" => {
            let size = if args.is_empty() {
                21 // Default nanoid size
            } else if args.len() == 1 {
                args[0].as_u64().ok_or_else(|| {
                    DbError::ExecutionError("NANOID: size must be a positive integer".to_string())
                })? as usize
            } else {
                return Err(DbError::ExecutionError(
                    "NANOID takes 0 or 1 argument (size)".to_string(),
                ));
            };
            if size == 0 || size > 256 {
                return Err(DbError::ExecutionError(
                    "NANOID: size must be between 1 and 256".to_string(),
                ));
            }
            let id = nanoid::nanoid!(size);
            Ok(Some(Value::String(id)))
        }
        _ => Ok(None),
    }
}
