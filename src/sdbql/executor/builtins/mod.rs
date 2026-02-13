//! Built-in function modules for SDBQL.
//!
//! This module organizes built-in functions into categories:
//! - type_check: IS_ARRAY, IS_STRING, IS_NULL, etc.
//! - string: UPPER, LOWER, TRIM, SPLIT, etc.
//! - array: FIRST, LAST, SORTED, UNIQUE, etc.
//! - math: FLOOR, CEIL, ROUND, SIN, COS, etc.
//! - crypto: MD5, SHA256, BASE64, ARGON2, etc.
//! - datetime: NOW, DATE_*, TIME_BUCKET, etc.
//! - geo: DISTANCE, GEO_DISTANCE, etc.
//! - json: JSON_PARSE, JSON_STRINGIFY
//! - misc: UUID, TYPEOF, COALESCE, etc.

pub mod array;
pub mod crypto;
pub mod datetime;
pub mod geo;
pub mod json;
pub mod math;
pub mod misc;
pub mod string;
pub mod type_check;

use crate::error::DbResult;
use serde_json::Value;

/// Try to evaluate a function using the built-in modules.
/// Returns Ok(Some(value)) if the function was handled,
/// Ok(None) if the function is not a built-in,
/// or Err if there was an error.
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    let name_upper = name.to_uppercase();
    let name = name_upper.as_str();

    // Try each module in order
    if let Some(v) = type_check::evaluate(name, args)? {
        return Ok(Some(v));
    }
    if let Some(v) = string::evaluate(name, args)? {
        return Ok(Some(v));
    }
    if let Some(v) = array::evaluate(name, args)? {
        return Ok(Some(v));
    }
    if let Some(v) = math::evaluate(name, args)? {
        return Ok(Some(v));
    }
    if let Some(v) = crypto::evaluate(name, args)? {
        return Ok(Some(v));
    }
    if let Some(v) = datetime::evaluate(name, args)? {
        return Ok(Some(v));
    }
    if let Some(v) = geo::evaluate(name, args)? {
        return Ok(Some(v));
    }
    if let Some(v) = json::evaluate(name, args)? {
        return Ok(Some(v));
    }
    if let Some(v) = misc::evaluate(name, args)? {
        return Ok(Some(v));
    }

    Ok(None)
}
