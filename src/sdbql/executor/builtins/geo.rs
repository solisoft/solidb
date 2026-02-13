//! Geospatial functions for SDBQL
//!
//! DISTANCE, GEO_DISTANCE, GEO_WITHIN, etc.

use crate::error::{DbError, DbResult};
use crate::storage::{distance_meters, GeoPoint};
use serde_json::Value;

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "DISTANCE" => {
            if args.len() != 4 {
                return Err(DbError::ExecutionError(
                    "DISTANCE requires 4 arguments: lat1, lon1, lat2, lon2".to_string(),
                ));
            }
            let lat1 = args[0].as_f64().ok_or_else(|| {
                DbError::ExecutionError("DISTANCE: lat1 must be a number".to_string())
            })?;
            let lon1 = args[1].as_f64().ok_or_else(|| {
                DbError::ExecutionError("DISTANCE: lon1 must be a number".to_string())
            })?;
            let lat2 = args[2].as_f64().ok_or_else(|| {
                DbError::ExecutionError("DISTANCE: lat2 must be a number".to_string())
            })?;
            let lon2 = args[3].as_f64().ok_or_else(|| {
                DbError::ExecutionError("DISTANCE: lon2 must be a number".to_string())
            })?;

            let dist = distance_meters(lat1, lon1, lat2, lon2);
            Ok(Some(Value::Number(
                serde_json::Number::from_f64(dist).unwrap_or(serde_json::Number::from(0)),
            )))
        }
        "GEO_DISTANCE" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "GEO_DISTANCE requires 2 arguments: point1, point2".to_string(),
                ));
            }
            let p1 = GeoPoint::from_value(&args[0]).ok_or_else(|| {
                DbError::ExecutionError(
                    "GEO_DISTANCE: first argument must be a geo point".to_string(),
                )
            })?;
            let p2 = GeoPoint::from_value(&args[1]).ok_or_else(|| {
                DbError::ExecutionError(
                    "GEO_DISTANCE: second argument must be a geo point".to_string(),
                )
            })?;

            let dist = distance_meters(p1.lat, p1.lon, p2.lat, p2.lon);
            Ok(Some(Value::Number(
                serde_json::Number::from_f64(dist).unwrap_or(serde_json::Number::from(0)),
            )))
        }
        _ => Ok(None),
    }
}
