//! Extended time functions for Lua (time namespace)

use mlua::Lua;
use crate::error::DbError;

/// Setup additional time namespace functions
pub fn setup_time_ext_globals(lua: &Lua) -> Result<(), DbError> {
    let globals = lua.globals();
    
    let time = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create time table: {}", e)))?;

    // time.now() -> float (seconds)
    let now_fn = lua
        .create_function(|_, ()| {
            let now = chrono::Utc::now();
            let ts = now.timestamp() as f64 + now.timestamp_subsec_micros() as f64 / 1_000_000.0;
            Ok(ts)
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.now function: {}", e)))?;
    time.set("now", now_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.now: {}", e)))?;

    // time.now_ms() -> int (milliseconds)
    let now_ms_fn = lua
        .create_function(|_, ()| Ok(chrono::Utc::now().timestamp_millis()))
        .map_err(|e| DbError::InternalError(format!("Failed to create time.now_ms function: {}", e)))?;
    time.set("now_ms", now_ms_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.now_ms: {}", e)))?;

    // time.now_ns() -> int (nanoseconds)  
    let now_ns_fn = lua
        .create_function(|_, ()| Ok(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)))
        .map_err(|e| DbError::InternalError(format!("Failed to create time.now_ns function: {}", e)))?;
    time.set("now_ns", now_ns_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.now_ns: {}", e)))?;

    // time.sleep(ms) -> void
    let sleep_fn = lua
        .create_async_function(|_, ms: u64| async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(ms)).await;
            Ok(())
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.sleep function: {}", e)))?;
    time.set("sleep", sleep_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.sleep: {}", e)))?;

    // time.format(ts, format) -> string
    let format_fn = lua
        .create_function(|_, (ts, fmt): (f64, String)| {
            let secs = ts.trunc() as i64;
            let nsecs = (ts.fract() * 1_000_000_000.0) as u32;
            let dt = chrono::DateTime::from_timestamp(secs, nsecs)
                .ok_or(mlua::Error::RuntimeError("Invalid timestamp".into()))?;
            Ok(dt.format(&fmt).to_string())
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.format function: {}", e)))?;
    time.set("format", format_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.format: {}", e)))?;

    // time.parse(iso) -> float
    let parse_fn = lua
        .create_function(|_, iso: String| {
            let dt = chrono::DateTime::parse_from_rfc3339(&iso)
                .map_err(|e| mlua::Error::RuntimeError(format!("Parse error: {}", e)))?;
            let ts = dt.timestamp() as f64 + dt.timestamp_subsec_micros() as f64 / 1_000_000.0;
            Ok(ts)
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.parse function: {}", e)))?;
    time.set("parse", parse_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.parse: {}", e)))?;

    // time.add(ts, value, unit) -> float
    let add_fn = lua
        .create_function(|_, (ts, val, unit): (f64, f64, String)| {
            let added_seconds = match unit.as_str() {
                "ms" => val / 1000.0,
                "s" => val,
                "m" => val * 60.0,
                "h" => val * 3600.0,
                "d" => val * 86400.0,
                _ => return Err(mlua::Error::RuntimeError(format!("Unknown unit: {}", unit))),
            };
            Ok(ts + added_seconds)
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.add function: {}", e)))?;
    time.set("add", add_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.add: {}", e)))?;

    // time.subtract(ts, value, unit) -> float
    let sub_fn = lua
        .create_function(|_, (ts, val, unit): (f64, f64, String)| {
            let sub_seconds = match unit.as_str() {
                "ms" => val / 1000.0,
                "s" => val,
                "m" => val * 60.0,
                "h" => val * 3600.0,
                "d" => val * 86400.0,
                _ => return Err(mlua::Error::RuntimeError(format!("Unknown unit: {}", unit))),
            };
            Ok(ts - sub_seconds)
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.subtract function: {}", e)))?;
    time.set("subtract", sub_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.subtract: {}", e)))?;

    // time.iso(timestamp?) -> ISO 8601 string
    let iso_fn = lua
        .create_function(|_, timestamp: Option<f64>| {
            let dt = if let Some(ts) = timestamp {
                let secs = ts.trunc() as i64;
                let nsecs = (ts.fract() * 1_000_000_000.0) as u32;
                chrono::DateTime::from_timestamp(secs, nsecs)
                    .ok_or(mlua::Error::RuntimeError("Invalid timestamp".into()))?
            } else {
                chrono::Utc::now()
            };
            Ok(dt.to_rfc3339())
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.iso function: {}", e)))?;
    time.set("iso", iso_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.iso: {}", e)))?;

    globals.set("time", time).map_err(|e| DbError::InternalError(format!("Failed to set time global: {}", e)))?;

    Ok(())
}
