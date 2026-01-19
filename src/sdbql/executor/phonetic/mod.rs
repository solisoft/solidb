use crate::error::DbResult;
use serde_json::Value;

#[allow(clippy::module_name_repetitions)]
pub mod date;
pub mod id;
pub mod math;
#[allow(clippy::module_inception)]
pub mod phonetic;
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
    if let Some(val) = phonetic::evaluate(name, args)? {
        return Ok(Some(val));
    }
    if let Some(val) = string::evaluate(name, args)? {
        return Ok(Some(val));
    }

    Ok(None)
}
