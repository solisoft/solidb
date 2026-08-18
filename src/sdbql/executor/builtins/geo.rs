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
        "GEO_EQUALS" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "GEO_EQUALS requires 2 arguments: point1, point2".to_string(),
                ));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let p1 = GeoPoint::from_value(&args[0]).ok_or_else(|| {
                DbError::ExecutionError(
                    "GEO_EQUALS: first argument must be a geo point".to_string(),
                )
            })?;
            let p2 = GeoPoint::from_value(&args[1]).ok_or_else(|| {
                DbError::ExecutionError(
                    "GEO_EQUALS: second argument must be a geo point".to_string(),
                )
            })?;
            const EPS: f64 = 1e-9;
            Ok(Some(Value::Bool(
                (p1.lat - p2.lat).abs() < EPS && (p1.lon - p2.lon).abs() < EPS,
            )))
        }
        "GEO_WITHIN" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "GEO_WITHIN requires 2 arguments: point, polygon".to_string(),
                ));
            }
            let point = GeoPoint::from_value(&args[0]).ok_or_else(|| {
                DbError::ExecutionError(
                    "GEO_WITHIN: first argument must be a geo point".to_string(),
                )
            })?;

            let polygon = args[1].as_array().ok_or_else(|| {
                DbError::ExecutionError(
                    "GEO_WITHIN: second argument must be an array of points".to_string(),
                )
            })?;

            if polygon.len() < 3 {
                return Err(DbError::ExecutionError(
                    "GEO_WITHIN: polygon must have at least 3 points".to_string(),
                ));
            }

            let inside = point_in_polygon(point.lat, point.lon, polygon);
            Ok(Some(Value::Bool(inside)))
        }
        "GEO_POINT" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "GEO_POINT requires lat, lon".to_string(),
                ));
            }
            let lat = args[0].as_f64().ok_or_else(|| {
                DbError::ExecutionError("GEO_POINT: lat must be a number".to_string())
            })?;
            let lon = args[1].as_f64().ok_or_else(|| {
                DbError::ExecutionError("GEO_POINT: lon must be a number".to_string())
            })?;
            Ok(Some(serde_json::json!({
                "type": "Point",
                "coordinates": [lon, lat]
            })))
        }
        "GEO_LINESTRING" => Ok(Some(geo_construct("LineString", args)?)),
        "GEO_POLYGON" => Ok(Some(geo_construct("Polygon", args)?)),
        "GEO_MULTIPOINT" => Ok(Some(geo_construct("MultiPoint", args)?)),
        "GEO_MULTILINESTRING" => Ok(Some(geo_construct("MultiLineString", args)?)),
        "GEO_MULTIPOLYGON" => Ok(Some(geo_construct("MultiPolygon", args)?)),
        "GEO_CONTAINS" => {
            check_geo_arity("GEO_CONTAINS", args, 2)?;
            Ok(Some(Value::Bool(geo_contains(&args[0], &args[1]))))
        }
        "GEO_INTERSECTS" => {
            check_geo_arity("GEO_INTERSECTS", args, 2)?;
            Ok(Some(Value::Bool(
                geo_contains(&args[0], &args[1])
                    || geo_contains(&args[1], &args[0])
                    || geo_line_intersect(&args[0], &args[1]),
            )))
        }
        "GEO_IN_RANGE" => {
            if args.len() != 4 {
                return Err(DbError::ExecutionError(
                    "GEO_IN_RANGE requires point, origin, low_m, high_m".to_string(),
                ));
            }
            let p = GeoPoint::from_value(&args[0]).ok_or_else(|| {
                DbError::ExecutionError("GEO_IN_RANGE: point required".to_string())
            })?;
            let o = GeoPoint::from_value(&args[1]).ok_or_else(|| {
                DbError::ExecutionError("GEO_IN_RANGE: origin required".to_string())
            })?;
            let lo = args[2].as_f64().unwrap_or(0.0);
            let hi = args[3].as_f64().unwrap_or(0.0);
            let d = distance_meters(p.lat, p.lon, o.lat, o.lon);
            Ok(Some(Value::Bool(d >= lo && d <= hi)))
        }
        "GEO_AREA" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "GEO_AREA requires a polygon".to_string(),
                ));
            }
            Ok(Some(Value::Number(
                serde_json::Number::from_f64(geo_area(&args[0])).unwrap_or_else(|| 0.into()),
            )))
        }
        _ => Ok(None),
    }
}

fn check_geo_arity(name: &str, args: &[Value], n: usize) -> DbResult<()> {
    if args.len() != n {
        return Err(DbError::ExecutionError(format!(
            "{name} requires {n} arguments"
        )));
    }
    Ok(())
}

fn geo_construct(ty: &str, args: &[Value]) -> DbResult<Value> {
    if args.len() != 1 {
        return Err(DbError::ExecutionError(format!(
            "GEO_{} requires coordinates",
            ty.to_uppercase()
        )));
    }
    Ok(serde_json::json!({ "type": ty, "coordinates": args[0] }))
}

fn ring_of(v: &Value) -> Vec<(f64, f64)> {
    match v {
        Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("Polygon") => o
            .get("coordinates")
            .and_then(Value::as_array)
            .and_then(|r| r.first())
            .and_then(Value::as_array)
            .map(|pts| pts.iter().map(get_coords).collect())
            .unwrap_or_default(),
        Value::Array(a) => {
            if a.first().and_then(Value::as_array).is_some()
                && a.first()
                    .and_then(|x| x.as_array())
                    .map(|p| p.first().and_then(Value::as_array).is_some())
                    .unwrap_or(false)
            {
                a.first()
                    .and_then(Value::as_array)
                    .map(|pts| pts.iter().map(get_coords).collect())
                    .unwrap_or_default()
            } else {
                a.iter().map(get_coords).collect()
            }
        }
        _ => Vec::new(),
    }
}

fn as_point(v: &Value) -> Option<(f64, f64)> {
    if let Some(p) = GeoPoint::from_value(v) {
        return Some((p.lon, p.lat));
    }
    if let Value::Object(o) = v {
        if o.get("type").and_then(Value::as_str) == Some("Point") {
            if let Some(c) = o.get("coordinates").and_then(Value::as_array) {
                if c.len() >= 2 {
                    return Some((c[0].as_f64()?, c[1].as_f64()?));
                }
            }
        }
    }
    None
}

fn geo_contains(outer: &Value, inner: &Value) -> bool {
    let ring = ring_of(outer);
    if ring.len() < 3 {
        return false;
    }
    let poly: Vec<Value> = ring
        .iter()
        .map(|(lon, lat)| serde_json::json!([lon, lat]))
        .collect();
    if let Some((lon, lat)) = as_point(inner) {
        return point_in_polygon(lat, lon, &poly);
    }
    let inner_ring = ring_of(inner);
    !inner_ring.is_empty()
        && inner_ring
            .iter()
            .all(|(lon, lat)| point_in_polygon(*lat, *lon, &poly))
}

fn segs_intersect(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    fn cross(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    }
    let d1 = cross(a, b, c);
    let d2 = cross(a, b, d);
    let d3 = cross(c, d, a);
    let d4 = cross(c, d, b);
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

fn geo_line_intersect(a: &Value, b: &Value) -> bool {
    let ra = ring_of(a);
    let rb = ring_of(b);
    if ra.len() < 2 || rb.len() < 2 {
        return false;
    }
    for w in ra.windows(2) {
        for z in rb.windows(2) {
            if segs_intersect(w[0], w[1], z[0], z[1]) {
                return true;
            }
        }
    }
    false
}

fn geo_area(poly: &Value) -> f64 {
    let ring = ring_of(poly);
    if ring.len() < 3 {
        return 0.0;
    }
    let mut s = 0.0;
    for i in 0..ring.len() {
        let (x1, y1) = ring[i];
        let (x2, y2) = ring[(i + 1) % ring.len()];
        s += x1 * y2 - x2 * y1;
    }
    // deg² → m² (rough, 111_320 m/deg)
    (s.abs() / 2.0) * 111_320.0 * 111_320.0
}

fn point_in_polygon(lat: f64, lon: f64, polygon: &[Value]) -> bool {
    let mut inside = false;
    let n = polygon.len();
    let mut j = n - 1;

    for i in 0..n {
        let pi = &polygon[i];
        let pj = &polygon[j];

        let (xi, yi) = get_coords(pi);
        let (xj, yj) = get_coords(pj);

        let intersect =
            ((yi > lat) != (yj > lat)) && (lon < (xj - xi) * (lat - yi) / (yj - yi) + xi);

        if intersect {
            inside = !inside;
        }
        j = i;
    }

    inside
}

fn get_coords(point: &Value) -> (f64, f64) {
    if let Some(arr) = point.as_array() {
        if arr.len() >= 2 {
            let lon = arr[0].as_f64().unwrap_or(0.0);
            let lat = arr[1].as_f64().unwrap_or(0.0);
            return (lon, lat);
        }
    }
    if let Some(obj) = point.as_object() {
        let lon = obj
            .get("lon")
            .or_else(|| obj.get("lng"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let lat = obj.get("lat").and_then(|v| v.as_f64()).unwrap_or(0.0);
        return (lon, lat);
    }
    (0.0, 0.0)
}
