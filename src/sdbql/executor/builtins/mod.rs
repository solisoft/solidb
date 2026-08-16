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

pub mod approx;
pub mod array;
pub mod crypto;
pub mod datetime;
pub mod geo;
pub mod json;
pub mod math;
pub mod misc;
pub mod string;
pub mod timeseries;
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

    // One module per prefix so a string function does not scan DATE_*/GEO_*.
    if name.starts_with("IS_") {
        return type_check::evaluate(name, args);
    }
    if name.starts_with("DATE_")
        || name.starts_with("NOW")
        || name == "TIME_BUCKET"
        || name == "HUMAN_TIME"
        || name == "UUIDV4"
        || name == "UUIDV7"
    {
        return datetime::evaluate(name, args);
    }
    if matches!(
        name,
        "DELTA" | "RATE" | "FILL" | "RESAMPLE" | "MATCH_SEQ" | "SEMANTIC"
    ) {
        if name == "MATCH_SEQ" {
            return Ok(Some(match_seq(args)?));
        }
        if name == "SEMANTIC" {
            return Ok(Some(semantic(args)?));
        }
        return timeseries::evaluate(name, args);
    }
    if name.starts_with("APPROX_")
        || name == "SKETCH_MERGE"
        || name.starts_with("MINHASH")
    {
        return approx::evaluate(name, args);
    }
    if name.starts_with("GEO_") || name == "DISTANCE" {
        return geo::evaluate(name, args);
    }
    if name.starts_with("JSON_") || name == "PARSE_JSON" || name == "TO_JSON" {
        return json::evaluate(name, args);
    }
    if matches!(
        name,
        "MD5"
            | "SHA256"
            | "SHA512"
            | "BASE64_ENCODE"
            | "TO_BASE64"
            | "BASE64_DECODE"
            | "FROM_BASE64"
            | "HEX_ENCODE"
            | "TO_HEX"
            | "HEX_DECODE"
            | "FROM_HEX"
            | "ARGON2_HASH"
            | "ARGON2_VERIFY"
            | "HMAC_SHA256"
    ) {
        return crypto::evaluate(name, args);
    }
    if matches!(
        name,
        "BIT_AND"
            | "BIT_OR"
            | "BIT_XOR"
            | "BIT_NEGATE"
            | "BIT_NOT"
            | "BIT_SHIFT_LEFT"
            | "BIT_SHIFT_RIGHT"
    ) {
        return math::evaluate(name, args);
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
    misc::evaluate(name, args)
}

fn semantic(args: &[Value]) -> DbResult<Value> {
    if args.len() < 2 {
        return Err(crate::error::DbError::ExecutionError(
            "SEMANTIC requires doc, query, [options]".to_string(),
        ));
    }
    let field = args
        .get(2)
        .and_then(|o| o.get("field"))
        .and_then(Value::as_str)
        .unwrap_or("body");
    let text = match &args[0] {
        Value::String(s) => s.clone(),
        Value::Object(o) => o
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    };
    let q = args[1].as_str().unwrap_or("");
    let score = crate::sdbql::executor::helpers::trigram_similarity(&text, q);
    Ok(serde_json::json!({
        "score": score,
        "match": score >= 0.45,
        "field": field
    }))
}

fn match_seq(args: &[Value]) -> DbResult<Value> {
    if args.len() != 3 {
        return Err(crate::error::DbError::ExecutionError(
            "MATCH_SEQ requires events, key_field, steps".to_string(),
        ));
    }
    let events = args[0].as_array().ok_or_else(|| {
        crate::error::DbError::ExecutionError("MATCH_SEQ: events must be an array".to_string())
    })?;
    let key_field = args[1].as_str().ok_or_else(|| {
        crate::error::DbError::ExecutionError("MATCH_SEQ: key_field must be a string".to_string())
    })?;
    let steps = args[2].as_array().ok_or_else(|| {
        crate::error::DbError::ExecutionError("MATCH_SEQ: steps must be an array".to_string())
    })?;
    if steps.is_empty() {
        return Ok(Value::Array(vec![]));
    }

    use std::collections::HashMap;
    let mut by_key: HashMap<String, Vec<&Value>> = HashMap::new();
    for ev in events {
        let k = ev
            .get(key_field)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".into());
        by_key.entry(k).or_default().push(ev);
    }

    let mut matches = Vec::new();
    for (key, mut evs) in by_key {
        evs.sort_by_key(|e| {
            e.get("ts")
                .or_else(|| e.get("t"))
                .or_else(|| e.get("time"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        });
        if let Some(hit) = match_one_key(&evs, steps) {
            let mut obj = serde_json::Map::new();
            obj.insert("key".into(), Value::String(key));
            obj.insert("steps".into(), hit);
            matches.push(Value::Object(obj));
        }
    }
    Ok(Value::Array(matches))
}

fn event_ts(e: &Value) -> i64 {
    e.get("ts")
        .or_else(|| e.get("t"))
        .or_else(|| e.get("time"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

fn step_matches(ev: &Value, step: &Value) -> bool {
    if let Some(ty) = step.get("type").and_then(Value::as_str) {
        let ev_ty = ev
            .get("type")
            .or_else(|| ev.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if ev_ty != ty {
            return false;
        }
    }
    if let Some(field) = step.get("field").and_then(Value::as_str) {
        if let Some(eq) = step.get("equals") {
            if ev.get(field) != Some(eq) {
                return false;
            }
        }
    }
    true
}

fn match_one_key(evs: &[&Value], steps: &[Value]) -> Option<Value> {
    let mut found: Vec<Value> = Vec::new();
    let mut idx = 0usize;
    let mut last_ts: Option<i64> = None;
    for step in steps {
        let within = step
            .get("within")
            .and_then(Value::as_str)
            .and_then(|s| timeseries::parse_interval_ms(s).ok());
        let mut hit = None;
        while idx < evs.len() {
            let ev = evs[idx];
            idx += 1;
            if !step_matches(ev, step) {
                continue;
            }
            let ts = event_ts(ev);
            if let (Some(prev), Some(w)) = (last_ts, within) {
                if ts - prev > w {
                    return None;
                }
            }
            last_ts = Some(ts);
            let name = step
                .get("as")
                .and_then(Value::as_str)
                .unwrap_or("step")
                .to_string();
            hit = Some(serde_json::json!({ "as": name, "event": ev, "ts": ts }));
            break;
        }
        found.push(hit?);
    }
    Some(Value::Array(found))
}
