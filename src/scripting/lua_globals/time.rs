//! Time-related Lua globals

use crate::error::DbError;
use mlua::Lua;

/// Setup the time table with date/time functions
pub fn setup_time_globals(lua: &Lua) -> Result<(), DbError> {
    let globals = lua.globals();

    let time_table = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create time table: {}", e)))?;

    // time.now() - current Unix timestamp in seconds
    let now_fn = lua
        .create_function(|_, ()| {
            use std::time::{SystemTime, UNIX_EPOCH};
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(mlua::Error::external)?;
            Ok(now.as_secs() as i64)
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.now: {}", e)))?;
    time_table
        .set("now", now_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.now: {}", e)))?;

    // time.millis() - current Unix timestamp in milliseconds
    let millis_fn = lua
        .create_function(|_, ()| {
            use std::time::{SystemTime, UNIX_EPOCH};
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(mlua::Error::external)?;
            Ok(now.as_millis() as i64)
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.millis: {}", e)))?;
    time_table
        .set("millis", millis_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.millis: {}", e)))?;

    // time.date(format, timestamp?) - format a timestamp (or current time)
    let date_fn = lua
        .create_function(|_, (format, timestamp): (Option<String>, Option<i64>)| {
            use chrono::{DateTime, TimeZone, Utc};
            let fmt = format.unwrap_or_else(|| "%Y-%m-%d %H:%M:%S".to_string());
            let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                Utc.timestamp_opt(ts, 0)
                    .single()
                    .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
            } else {
                Utc::now()
            };
            Ok(dt.format(&fmt).to_string())
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.date: {}", e)))?;
    time_table
        .set("date", date_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.date: {}", e)))?;

    // time.parse(str, format?) - parse a date string to timestamp
    let parse_fn = lua
        .create_function(|_, (date_str, format): (String, Option<String>)| {
            use chrono::{DateTime, NaiveDateTime, Utc};
            let fmt = format.unwrap_or_else(|| "%Y-%m-%d %H:%M:%S".to_string());
            let naive = NaiveDateTime::parse_from_str(&date_str, &fmt)
                .map_err(|e| mlua::Error::external(format!("Date parse error: {}", e)))?;
            let dt: DateTime<Utc> = DateTime::from_naive_utc_and_offset(naive, Utc);
            Ok(dt.timestamp())
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.parse: {}", e)))?;
    time_table
        .set("parse", parse_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.parse: {}", e)))?;

    // time.iso(timestamp?) - format as ISO 8601 string
    let iso_fn = lua
        .create_function(|_, timestamp: Option<i64>| {
            use chrono::{DateTime, TimeZone, Utc};
            let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                Utc.timestamp_opt(ts, 0)
                    .single()
                    .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
            } else {
                Utc::now()
            };
            Ok(dt.to_rfc3339())
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.iso: {}", e)))?;
    time_table
        .set("iso", iso_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.iso: {}", e)))?;

    // time.diff(t1, t2) - difference in seconds
    let diff_fn = lua
        .create_function(|_, (t1, t2): (i64, i64)| Ok(t1 - t2))
        .map_err(|e| DbError::InternalError(format!("Failed to create time.diff: {}", e)))?;
    time_table
        .set("diff", diff_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.diff: {}", e)))?;

    // time.add(timestamp, seconds) - add seconds to timestamp
    let add_fn = lua
        .create_function(|_, (timestamp, seconds): (i64, i64)| Ok(timestamp + seconds))
        .map_err(|e| DbError::InternalError(format!("Failed to create time.add: {}", e)))?;
    time_table
        .set("add", add_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.add: {}", e)))?;

    // time.year(timestamp?)
    let year_fn = lua
        .create_function(|_, timestamp: Option<i64>| {
            use chrono::{DateTime, Datelike, TimeZone, Utc};
            let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                Utc.timestamp_opt(ts, 0)
                    .single()
                    .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
            } else {
                Utc::now()
            };
            Ok(dt.year())
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.year: {}", e)))?;
    time_table
        .set("year", year_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.year: {}", e)))?;

    // time.month(timestamp?)
    let month_fn = lua
        .create_function(|_, timestamp: Option<i64>| {
            use chrono::{DateTime, Datelike, TimeZone, Utc};
            let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                Utc.timestamp_opt(ts, 0)
                    .single()
                    .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
            } else {
                Utc::now()
            };
            Ok(dt.month() as i32)
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.month: {}", e)))?;
    time_table
        .set("month", month_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.month: {}", e)))?;

    // time.day(timestamp?)
    let day_fn = lua
        .create_function(|_, timestamp: Option<i64>| {
            use chrono::{DateTime, Datelike, TimeZone, Utc};
            let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                Utc.timestamp_opt(ts, 0)
                    .single()
                    .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
            } else {
                Utc::now()
            };
            Ok(dt.day() as i32)
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.day: {}", e)))?;
    time_table
        .set("day", day_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.day: {}", e)))?;

    // time.hour(timestamp?)
    let hour_fn = lua
        .create_function(|_, timestamp: Option<i64>| {
            use chrono::{DateTime, TimeZone, Timelike, Utc};
            let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                Utc.timestamp_opt(ts, 0)
                    .single()
                    .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
            } else {
                Utc::now()
            };
            Ok(dt.hour() as i32)
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.hour: {}", e)))?;
    time_table
        .set("hour", hour_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.hour: {}", e)))?;

    // time.minute(timestamp?)
    let minute_fn = lua
        .create_function(|_, timestamp: Option<i64>| {
            use chrono::{DateTime, TimeZone, Timelike, Utc};
            let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                Utc.timestamp_opt(ts, 0)
                    .single()
                    .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
            } else {
                Utc::now()
            };
            Ok(dt.minute() as i32)
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.minute: {}", e)))?;
    time_table
        .set("minute", minute_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.minute: {}", e)))?;

    // time.second(timestamp?)
    let second_fn = lua
        .create_function(|_, timestamp: Option<i64>| {
            use chrono::{DateTime, TimeZone, Timelike, Utc};
            let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                Utc.timestamp_opt(ts, 0)
                    .single()
                    .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
            } else {
                Utc::now()
            };
            Ok(dt.second() as i32)
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.second: {}", e)))?;
    time_table
        .set("second", second_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.second: {}", e)))?;

    // time.weekday(timestamp?) - day of week (1=Monday, 7=Sunday)
    let weekday_fn = lua
        .create_function(|_, timestamp: Option<i64>| {
            use chrono::{DateTime, Datelike, TimeZone, Utc};
            let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                Utc.timestamp_opt(ts, 0)
                    .single()
                    .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
            } else {
                Utc::now()
            };
            Ok(dt.weekday().num_days_from_monday() as i32 + 1)
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create time.weekday: {}", e)))?;
    time_table
        .set("weekday", weekday_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set time.weekday: {}", e)))?;

    globals
        .set("time", time_table)
        .map_err(|e| DbError::InternalError(format!("Failed to set time global: {}", e)))?;

    Ok(())
}
