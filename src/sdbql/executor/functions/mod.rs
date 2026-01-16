use serde_json::Value;
use crate::error::DbResult;

pub mod date;
pub mod id;
pub mod math;
pub mod string;

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    // Try each module
    if let Some(val) = date::evaluate(name, args)? {
        return Ok(Some(val));
    }
    if let Some(val) = id::evaluate(name, args)? {
        return Ok(Some(val));
    }
    if let Some(val) = math::evaluate(name, args)? {
        return Ok(Some(val));
    }
    if let Some(val) = string::evaluate(name, args)? {
        return Ok(Some(val));
    }

    Ok(None)
}
