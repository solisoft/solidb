use chrono::{Datelike, TimeZone, Timelike, Utc, Duration, NaiveDate, NaiveDateTime};
use chrono_tz::Tz;
use serde_json::Value;

use super::super::utils::{number_from_f64, parse_datetime};
use crate::error::{DbError, DbResult};

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "DATE_NOW" => {
            if !args.is_empty() {
                return Err(DbError::ExecutionError(
                    "DATE_NOW requires 0 arguments".to_string(),
                ));
            }
            let timestamp = Utc::now().timestamp_millis();
            Ok(Some(Value::Number(serde_json::Number::from(timestamp))))
        }
        "DATE_ISO8601" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "DATE_ISO8601 requires 1 argument: timestamp in milliseconds".to_string(),
                ));
            }

            let timestamp_ms = match &args[0] {
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        i
                    } else if let Some(f) = n.as_f64() {
                        f as i64
                    } else {
                        return Err(DbError::ExecutionError("DATE_ISO8601: argument must be a number (timestamp in milliseconds)".to_string()));
                    }
                }
                _ => {
                    return Err(DbError::ExecutionError(
                        "DATE_ISO8601: argument must be a number (timestamp in milliseconds)"
                            .to_string(),
                    ))
                }
            };

            let timestamp_secs = timestamp_ms / 1000;
            let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;

            let datetime = match Utc.timestamp_opt(timestamp_secs, nanos) {
                chrono::LocalResult::Single(dt) => dt,
                _ => {
                    return Err(DbError::ExecutionError(format!(
                        "DATE_ISO8601: invalid timestamp: {}",
                        timestamp_ms
                    )))
                }
            };
            Ok(Some(Value::String(datetime.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))))
        }
        "DATE_TIMESTAMP" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "DATE_TIMESTAMP requires 1 argument: ISO 8601 date string".to_string(),
                ));
            }

            let date_str = args[0].as_str().ok_or_else(|| {
                DbError::ExecutionError(
                    "DATE_TIMESTAMP: argument must be a string (ISO 8601 date)".to_string(),
                )
            })?;

            let datetime = chrono::DateTime::parse_from_rfc3339(date_str).map_err(|e| {
                DbError::ExecutionError(format!(
                    "DATE_TIMESTAMP: invalid ISO 8601 date '{}': {}",
                    date_str, e
                ))
            })?;

            let timestamp_ms = datetime.timestamp_millis();
            Ok(Some(Value::Number(serde_json::Number::from(timestamp_ms))))
        }
        "DATE_YEAR" => {
             if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "DATE_YEAR requires 1 argument".to_string(),
                ));
            }
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.year()))))
        }
        "DATE_MONTH" => {
             if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "DATE_MONTH requires 1 argument".to_string(),
                ));
            }
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.month()))))
        }
        "DATE_DAY" => {
             if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "DATE_DAY requires 1 argument".to_string(),
                ));
            }
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.day()))))
        }
        "DATE_HOUR" => {
             if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "DATE_HOUR requires 1 argument".to_string(),
                ));
            }
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.hour()))))
        }
        "DATE_MINUTE" => {
             if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "DATE_MINUTE requires 1 argument".to_string(),
                ));
            }
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.minute()))))
        }
        "DATE_SECOND" => {
             if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "DATE_SECOND requires 1 argument".to_string(),
                ));
            }
            let dt = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(dt.second()))))
        }
        "DATE_DAYOFWEEK" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "DATE_DAYOFWEEK requires 1 argument".to_string(),
                ));
            }
            let dt = parse_datetime(&args[0])?;
            let weekday = dt.weekday().num_days_from_sunday();
            Ok(Some(Value::Number(serde_json::Number::from(weekday))))
        }
        "DATE_QUARTER" => {
             if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "DATE_QUARTER requires 1 argument".to_string(),
                ));
            }
            let dt = parse_datetime(&args[0])?;
            let quarter = (dt.month() - 1) / 3 + 1;
            Ok(Some(Value::Number(serde_json::Number::from(quarter))))
        }
        "DATE_TRUNC" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "DATE_TRUNC requires 2-3 arguments: date, unit, [timezone]".to_string(),
                ));
            }

            let datetime_utc = parse_datetime(&args[0])?;
            let unit = args[1].as_str().ok_or_else(|| {
                DbError::ExecutionError("DATE_TRUNC: unit must be a string".to_string())
            })?.to_lowercase();

            let tz: Tz = if args.len() == 3 {
                let tz_str = args[2].as_str().ok_or_else(|| {
                    DbError::ExecutionError("DATE_TRUNC: timezone must be a string".to_string())
                })?;
                tz_str.parse().map_err(|_| {
                    DbError::ExecutionError(format!("DATE_TRUNC: unknown timezone '{}'", tz_str))
                })?
            } else {
                chrono_tz::UTC
            };

            let datetime_tz = datetime_utc.with_timezone(&tz);

            let truncated = match unit.as_str() {
                "y" | "year" | "years" => {
                     let naive = NaiveDateTime::new(
                        NaiveDate::from_ymd_opt(datetime_tz.year(), 1, 1).unwrap(),
                        chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
                    );
                    tz.from_local_datetime(&naive).single().unwrap()
                }
                "m" | "month" | "months" => {
                    let naive = NaiveDateTime::new(
                        NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), 1).unwrap(),
                        chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
                    );
                    tz.from_local_datetime(&naive).single().unwrap()
                }
                "d" | "day" | "days" => {
                    let naive = NaiveDateTime::new(
                        NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), datetime_tz.day()).unwrap(),
                        chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
                    );
                    tz.from_local_datetime(&naive).single().unwrap()
                }
                "h" | "hour" | "hours" => {
                    let naive = NaiveDateTime::new(
                        NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), datetime_tz.day()).unwrap(),
                        chrono::NaiveTime::from_hms_opt(datetime_tz.hour(), 0, 0).unwrap()
                    );
                    tz.from_local_datetime(&naive).single().unwrap()
                }
                "i" | "minute" | "minutes" => {
                    let naive = NaiveDateTime::new(
                        NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), datetime_tz.day()).unwrap(),
                        chrono::NaiveTime::from_hms_opt(datetime_tz.hour(), datetime_tz.minute(), 0).unwrap()
                    );
                    tz.from_local_datetime(&naive).single().unwrap()
                }
                "s" | "second" | "seconds" => {
                    let naive = NaiveDateTime::new(
                        NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), datetime_tz.day()).unwrap(),
                        chrono::NaiveTime::from_hms_opt(datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second()).unwrap()
                    );
                    tz.from_local_datetime(&naive).single().unwrap()
                }
                "f" | "millisecond" | "milliseconds" => datetime_tz,
                _ => return Err(DbError::ExecutionError(
                    format!("DATE_TRUNC: unknown unit '{}'", unit)
                )),
            };

            let truncated_utc = truncated.with_timezone(&Utc);
            Ok(Some(Value::String(truncated_utc.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))))
        }
        "DATE_DAYS_IN_MONTH" => {
            if args.is_empty() || args.len() > 2 {
                return Err(DbError::ExecutionError(
                    "DATE_DAYS_IN_MONTH requires 1-2 arguments: date, [timezone]".to_string(),
                ));
            }
            let datetime_utc = parse_datetime(&args[0])?;
            let (year, month) = if args.len() == 2 {
                let tz_str = args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("DATE_DAYS_IN_MONTH: timezone must be a string".to_string())
                })?;
                let tz: Tz = tz_str.parse().map_err(|_| {
                    DbError::ExecutionError(format!("DATE_DAYS_IN_MONTH: unknown timezone '{}'", tz_str))
                })?;
                let dt_tz = datetime_utc.with_timezone(&tz);
                (dt_tz.year(), dt_tz.month())
            } else {
                (datetime_utc.year(), datetime_utc.month())
            };

            let days_in_month = if month == 12 {
                NaiveDate::from_ymd_opt(year + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(year, month + 1, 1)
            }
            .and_then(|next_month| {
                NaiveDate::from_ymd_opt(year, month, 1)
                    .map(|this_month| (next_month - this_month).num_days())
            })
            .unwrap_or(30) as u32;

            Ok(Some(Value::Number(serde_json::Number::from(days_in_month))))
        }
        "DATE_DAYOFYEAR" => {
            if args.is_empty() || args.len() > 2 {
                 return Err(DbError::ExecutionError(
                    "DATE_DAYOFYEAR requires 1-2 arguments: date, [timezone]".to_string(),
                ));
            }
            let datetime_utc = parse_datetime(&args[0])?;
            let day_of_year = if args.len() == 2 {
                let tz_str = args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("DATE_DAYOFYEAR: timezone must be a string".to_string())
                })?;
                let tz: Tz = tz_str.parse().map_err(|_| {
                    DbError::ExecutionError(format!("DATE_DAYOFYEAR: unknown timezone '{}'", tz_str))
                })?;
                datetime_utc.with_timezone(&tz).ordinal()
            } else {
                datetime_utc.ordinal()
            };
            Ok(Some(Value::Number(serde_json::Number::from(day_of_year))))
        }
        "DATE_ISOWEEK" => {
            if args.len() != 1 {
                 return Err(DbError::ExecutionError(
                    "DATE_ISOWEEK requires 1 argument".to_string(),
                ));
            }
            let datetime_utc = parse_datetime(&args[0])?;
            Ok(Some(Value::Number(serde_json::Number::from(datetime_utc.iso_week().week()))))
        }
        "DATE_FORMAT" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "DATE_FORMAT requires 2-3 arguments: date, format, [timezone]".to_string(),
                ));
            }
            let datetime_utc = parse_datetime(&args[0])?;
            let format_str = args[1].as_str().ok_or_else(|| {
                DbError::ExecutionError("DATE_FORMAT: format must be a string".to_string())
            })?;
            let tz: Tz = if args.len() == 3 {
                let tz_str = args[2].as_str().ok_or_else(|| {
                    DbError::ExecutionError("DATE_FORMAT: timezone must be a string".to_string())
                })?;
                tz_str.parse().map_err(|_| {
                    DbError::ExecutionError(format!("DATE_FORMAT: unknown timezone '{}'", tz_str))
                })?
            } else {
                chrono_tz::UTC
            };
            let datetime_tz = datetime_utc.with_timezone(&tz);
            Ok(Some(Value::String(datetime_tz.format(format_str).to_string())))
        }
        "DATE_ADD" => {
            if args.len() < 3 || args.len() > 4 {
                return Err(DbError::ExecutionError(
                    "DATE_ADD requires 3-4 arguments: date, amount, unit, [timezone]".to_string(),
                ));
            }
            let datetime_utc = parse_datetime(&args[0])?;
            let amount = args[1].as_i64().or_else(|| args[1].as_f64().map(|f| f as i64)).ok_or_else(|| {
                DbError::ExecutionError("DATE_ADD: amount must be a number".to_string())
            })?;
            let unit = args[2].as_str().ok_or_else(|| {
                DbError::ExecutionError("DATE_ADD: unit must be a string".to_string())
            })?.to_lowercase();
            
            let tz: Tz = if args.len() > 3 {
                let tz_str = args[3].as_str().ok_or_else(|| {
                     DbError::ExecutionError("DATE_ADD: timezone must be a string".to_string())
                })?;
                tz_str.parse().map_err(|_| {
                    DbError::ExecutionError(format!("DATE_ADD: unknown timezone '{}'", tz_str))
                })?
            } else {
                chrono_tz::UTC
            };

            let datetime_tz = datetime_utc.with_timezone(&tz);
            
            // Implementation of date addition logic
            // Simplified using Duration for standard units, special logic for year/month
             let result_tz = match unit.as_str() {
                "y" | "year" | "years" => {
                    let new_year = datetime_tz.year() + amount as i32;
                    let naive = NaiveDateTime::new(
                        NaiveDate::from_ymd_opt(new_year, datetime_tz.month(), datetime_tz.day())
                            .unwrap_or_else(|| {
                                // Handle Feb 29 -> Feb 28
                                NaiveDate::from_ymd_opt(new_year, datetime_tz.month(), 28).unwrap()
                            }),
                        chrono::NaiveTime::from_hms_milli_opt(
                            datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second(),
                            datetime_tz.timestamp_subsec_millis()
                        ).unwrap()
                    );
                    tz.from_local_datetime(&naive).single().ok_or_else(|| DbError::ExecutionError("DATE_ADD: invalid datetime".to_string()))?
                }
                "m" | "month" | "months" => {
                    let total_months = datetime_tz.year() * 12 + datetime_tz.month() as i32 - 1 + amount as i32;
                    let new_year = total_months / 12;
                    let new_month = (total_months % 12 + 1) as u32;
                    let max_day = NaiveDate::from_ymd_opt(new_year, new_month + 1, 1)
                        .unwrap_or_else(|| NaiveDate::from_ymd_opt(new_year + 1, 1, 1).unwrap())
                        .pred_opt().unwrap().day();
                    let new_day = datetime_tz.day().min(max_day);
                     let naive = NaiveDateTime::new(
                        NaiveDate::from_ymd_opt(new_year, new_month, new_day).unwrap(),
                        chrono::NaiveTime::from_hms_milli_opt(
                            datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second(),
                            datetime_tz.timestamp_subsec_millis()
                        ).unwrap()
                    );
                    tz.from_local_datetime(&naive).single().ok_or_else(|| DbError::ExecutionError("DATE_ADD: invalid datetime".to_string()))?
                }
                "w" | "week" | "weeks" => datetime_tz + Duration::weeks(amount),
                "d" | "day" | "days" => datetime_tz + Duration::days(amount),
                "h" | "hour" | "hours" => datetime_tz + Duration::hours(amount),
                "i" | "minute" | "minutes" => datetime_tz + Duration::minutes(amount),
                "s" | "second" | "seconds" => datetime_tz + Duration::seconds(amount),
                "f" | "millisecond" | "milliseconds" => datetime_tz + Duration::milliseconds(amount),
                _ => return Err(DbError::ExecutionError(format!("DATE_ADD: unknown unit '{}'", unit))),
            };

            let result_utc = result_tz.with_timezone(&Utc);
            Ok(Some(Value::String(result_utc.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))))
        }
        "DATE_SUBTRACT" => {
            // DATE_SUBTRACT is just negation of amount in DATE_ADD
            // Implemented by calling our own DATE_ADD logic or recursively?
            // Recursive call is hard because we are inside the match.
            // But we can just duplicate logic or create internal helper.
            // For now, I'll return None and let Executor handle it? 
            // NO, executor delegates to us.
            // I'll implement it by copying logic but negating amount.
             if args.len() < 3 || args.len() > 4 {
                return Err(DbError::ExecutionError(
                    "DATE_SUBTRACT requires 3-4 arguments".to_string(),
                ));
            }
             let amount = args[1].as_i64().or_else(|| args[1].as_f64().map(|f| f as i64)).ok_or_else(|| {
                DbError::ExecutionError("DATE_SUBTRACT: amount must be a number".to_string())
            })?;
            
            // Construct new args with negated amount
            let mut new_args = args.to_vec();
            new_args[1] = Value::Number(serde_json::Number::from(-amount));
            evaluate("DATE_ADD", &new_args)
        }
        "DATE_DIFF" => {
             if args.len() < 3 || args.len() > 6 {
                return Err(DbError::ExecutionError(
                    "DATE_DIFF requires 3-6 arguments".to_string(),
                ));
            }
            let datetime1_utc = parse_datetime(&args[0])?;
            let datetime2_utc = parse_datetime(&args[1])?;
            let unit = args[2].as_str().ok_or_else(|| {
                DbError::ExecutionError("DATE_DIFF: unit must be a string".to_string())
            })?.to_lowercase();
            let as_float = if args.len() >= 4 { args[3].as_bool().unwrap_or(false) } else { false };
            
            let (tz1, tz2) = if args.len() >= 5 {
                let tz1_str = args[4].as_str();
                let tz1 = if let Some(s) = tz1_str { s.parse::<Tz>().unwrap_or(chrono_tz::UTC) } else { chrono_tz::UTC };
                 let tz2 = if args.len() >= 6 {
                    let tz2_str = args[5].as_str();
                    if let Some(s) = tz2_str { s.parse::<Tz>().unwrap_or(chrono_tz::UTC) } else { tz1 }
                 } else { tz1 };
                 (tz1, tz2)
            } else {
                (chrono_tz::UTC, chrono_tz::UTC)
            };

            let dt1 = datetime1_utc.with_timezone(&tz1);
            let dt2 = datetime2_utc.with_timezone(&tz2);

            let diff: f64 = match unit.as_str() {
                "y" | "year" | "years" => {
                    let year_diff = dt2.year() - dt1.year();
                    if as_float {
                         year_diff as f64 + (dt2.month() as f64 - dt1.month() as f64) / 12.0
                    } else {
                        year_diff as f64
                    }
                }
                "m" | "month" | "months" => {
                    let months = (dt2.year() * 12 + dt2.month() as i32) - (dt1.year() * 12 + dt1.month() as i32);
                     if as_float {
                         months as f64 + (dt2.day() as f64 - dt1.day() as f64) / 30.0
                    } else {
                        months as f64
                    }
                }
                "d" | "day" | "days" => {
                    let diff_ms = dt2.timestamp_millis() - dt1.timestamp_millis();
                    let days = diff_ms as f64 / (24.0 * 3600.0 * 1000.0);
                    if as_float { days } else { days.trunc() }
                }
                 "h" | "hour" | "hours" => {
                    let diff_ms = dt2.timestamp_millis() - dt1.timestamp_millis();
                    let vals = diff_ms as f64 / (3600.0 * 1000.0);
                    if as_float { vals } else { vals.trunc() }
                }
                 "i" | "minute" | "minutes" => {
                    let diff_ms = dt2.timestamp_millis() - dt1.timestamp_millis();
                    let vals = diff_ms as f64 / (60.0 * 1000.0);
                    if as_float { vals } else { vals.trunc() }
                }
                 "s" | "second" | "seconds" => {
                    let diff_ms = dt2.timestamp_millis() - dt1.timestamp_millis();
                    let vals = diff_ms as f64 / 1000.0;
                    if as_float { vals } else { vals.trunc() }
                }
                 "f" | "millisecond" | "milliseconds" => {
                    (dt2.timestamp_millis() - dt1.timestamp_millis()) as f64
                }
                _ => return Err(DbError::ExecutionError(format!("DATE_DIFF: unknown unit '{}'", unit))),
            };
            
            Ok(Some(Value::Number(number_from_f64(diff))))
        }
        // ... I'll add more dates in update steps ...
        _ => Ok(None),

    }
}
