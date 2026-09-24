//! Geospatial functions for SDBQL
//!
//! DISTANCE, GEO_DISTANCE, GEO_WITHIN, GEO_CONTAINS, GEO_INTERSECTS, etc.
//!
//! Conventions (GeoJSON / AQL):
//! - A bare two-number array is **`[lon, lat]`**. `{lat, lon}` objects (also
//!   `latitude`/`longitude`/`lng`) and GeoJSON geometries are accepted too.
//!   The geo *index* keeps its historical `[lat, lon]` reading of stored
//!   arrays — see `GeoPoint::from_value`.
//! - Each argument is parsed once into a [`Geometry`], then the predicate
//!   runs on plain coordinates.
//! - Containment and intersection are planar in lon/lat (edges are straight
//!   lines on the map, not great circles), with the boundary counted as
//!   part of a polygon. Polygon holes are honoured.
//! - Distances are Haversine on a sphere of radius 6,371 km; areas use the
//!   spherical-excess formula on the same sphere.

use crate::error::{DbError, DbResult};
use crate::storage::{distance_meters, GeoPoint};
use serde_json::Value;

/// `(lon, lat)`
type Pt = (f64, f64);
/// A ring without its closing duplicate vertex.
type Ring = Vec<Pt>;
/// `[outer, hole, hole, …]`
type Poly = Vec<Ring>;

const EARTH_RADIUS_M: f64 = 6_371_000.0;
/// Radius of the sphere with the WGS84 ellipsoid's surface area.
const AUTHALIC_RADIUS_M: f64 = 6_371_007.2;
const EPS: f64 = 1e-12;
const POINT_EQ_EPS: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
enum Geometry {
    Point(Pt),
    MultiPoint(Vec<Pt>),
    LineString(Vec<Pt>),
    MultiLineString(Vec<Vec<Pt>>),
    Polygon(Poly),
    MultiPolygon(Vec<Poly>),
}

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "DISTANCE" => {
            if args.len() != 4 {
                return Err(DbError::ExecutionError(
                    "DISTANCE requires 4 arguments: lat1, lon1, lat2, lon2".to_string(),
                ));
            }
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let mut n = [0.0; 4];
            for (i, label) in ["lat1", "lon1", "lat2", "lon2"].iter().enumerate() {
                n[i] = args[i].as_f64().ok_or_else(|| {
                    DbError::ExecutionError(format!("DISTANCE: {label} must be a number"))
                })?;
            }
            let (lat1, lon1, lat2, lon2) = (n[0], n[1], n[2], n[3]);
            if !valid_lat_lon(lat1, lon1) || !valid_lat_lon(lat2, lon2) {
                return Ok(Some(Value::Null));
            }
            Ok(Some(meters(distance_meters(lat1, lon1, lat2, lon2))))
        }
        "GEO_DISTANCE" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "GEO_DISTANCE requires 2 arguments: point1, point2".to_string(),
                ));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let a = geometry_arg("GEO_DISTANCE", &args[0])?;
            let b = geometry_arg("GEO_DISTANCE", &args[1])?;
            // Non-point geometries are measured between their centroids, as AQL does.
            let (lon1, lat1) = centroid(&a);
            let (lon2, lat2) = centroid(&b);
            if !valid_lat_lon(lat1, lon1) || !valid_lat_lon(lat2, lon2) {
                return Ok(Some(Value::Null));
            }
            Ok(Some(meters(distance_meters(lat1, lon1, lat2, lon2))))
        }
        "GEO_EQUALS" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "GEO_EQUALS requires 2 arguments: geometry1, geometry2".to_string(),
                ));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let a = geometry_arg("GEO_EQUALS", &args[0])?;
            let b = geometry_arg("GEO_EQUALS", &args[1])?;
            Ok(Some(Value::Bool(geometry_eq(&a, &b))))
        }
        "GEO_WITHIN" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "GEO_WITHIN requires 2 arguments: point, polygon".to_string(),
                ));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let point = match geometry_arg("GEO_WITHIN", &args[0])? {
                Geometry::Point(p) => p,
                _ => {
                    return Err(DbError::ExecutionError(
                        "GEO_WITHIN: first argument must be a geo point".to_string(),
                    ))
                }
            };
            if let Some(a) = args[1].as_array() {
                if a.len() < 3 && a.first().and_then(position).is_some() {
                    return Err(DbError::ExecutionError(
                        "GEO_WITHIN: polygon must have at least 3 points".to_string(),
                    ));
                }
            }
            let polys = match geometry_arg("GEO_WITHIN", &args[1])? {
                Geometry::Polygon(p) => vec![p],
                Geometry::MultiPolygon(ps) => ps,
                _ => {
                    return Err(DbError::ExecutionError(
                        "GEO_WITHIN: second argument must be a polygon (ring array or GeoJSON \
                         Polygon/MultiPolygon)"
                            .to_string(),
                    ))
                }
            };
            Ok(Some(Value::Bool(
                polys_locate(point, &polys) != Loc::Outside,
            )))
        }
        "GEO_POINT" => {
            // SoliDB's documented argument order is (lat, lon); AQL's is
            // (lon, lat). The stored coordinates are GeoJSON [lon, lat] either way.
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
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let a = geometry_arg("GEO_CONTAINS", &args[0])?;
            let b = geometry_arg("GEO_CONTAINS", &args[1])?;
            Ok(Some(Value::Bool(contains(&a, &b))))
        }
        "GEO_INTERSECTS" => {
            check_geo_arity("GEO_INTERSECTS", args, 2)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let a = geometry_arg("GEO_INTERSECTS", &args[0])?;
            let b = geometry_arg("GEO_INTERSECTS", &args[1])?;
            Ok(Some(Value::Bool(intersects(&a, &b))))
        }
        "GEO_IN_RANGE" => {
            if args.len() < 4 || args.len() > 6 {
                return Err(DbError::ExecutionError(
                    "GEO_IN_RANGE requires point, origin, low_m, high_m, [includeLow], \
                     [includeHigh]"
                        .to_string(),
                ));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let p = point_arg("GEO_IN_RANGE", "point", &args[0])?;
            let o = point_arg("GEO_IN_RANGE", "origin", &args[1])?;
            let lo = finite_number("GEO_IN_RANGE", "low", &args[2])?;
            let hi = finite_number("GEO_IN_RANGE", "high", &args[3])?;
            let include_lo = opt_bool("GEO_IN_RANGE", "includeLow", args.get(4), true)?;
            let include_hi = opt_bool("GEO_IN_RANGE", "includeHigh", args.get(5), true)?;
            if !valid_lat_lon(p.lat, p.lon) || !valid_lat_lon(o.lat, o.lon) {
                return Ok(Some(Value::Null));
            }
            let d = distance_meters(p.lat, p.lon, o.lat, o.lon);
            let above = if include_lo { d >= lo } else { d > lo };
            let below = if include_hi { d <= hi } else { d < hi };
            Ok(Some(Value::Bool(above && below)))
        }
        "GEO_AREA" => {
            if args.is_empty() || args.len() > 2 {
                return Err(DbError::ExecutionError(
                    "GEO_AREA requires a polygon and an optional ellipsoid ('sphere' or 'wgs84')"
                        .to_string(),
                ));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let radius = match args.get(1) {
                None | Some(Value::Null) => EARTH_RADIUS_M,
                Some(Value::String(s)) if s.eq_ignore_ascii_case("sphere") => EARTH_RADIUS_M,
                Some(Value::String(s)) if s.eq_ignore_ascii_case("wgs84") => AUTHALIC_RADIUS_M,
                Some(_) => {
                    return Err(DbError::ExecutionError(
                        "GEO_AREA: ellipsoid must be 'sphere' or 'wgs84'".to_string(),
                    ))
                }
            };
            let g = geometry_arg("GEO_AREA", &args[0])?;
            Ok(Some(meters(geo_area(&g, radius))))
        }
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

fn meters(d: f64) -> Value {
    serde_json::Number::from_f64(d)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn valid_lat_lon(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
}

fn finite_number(fname: &str, what: &str, v: &Value) -> DbResult<f64> {
    v.as_f64()
        .filter(|f| f.is_finite())
        .ok_or_else(|| DbError::ExecutionError(format!("{fname}: {what} must be a number")))
}

fn opt_bool(fname: &str, what: &str, v: Option<&Value>, default: bool) -> DbResult<bool> {
    match v {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(DbError::ExecutionError(format!(
            "{fname}: {what} must be a boolean"
        ))),
    }
}

fn point_arg(fname: &str, what: &str, v: &Value) -> DbResult<GeoPoint> {
    match parse_geometry(v) {
        Some(Geometry::Point((lon, lat))) => Ok(GeoPoint::new(lat, lon)),
        _ => Err(DbError::ExecutionError(format!(
            "{fname}: {what} must be a geo point"
        ))),
    }
}

fn geometry_arg(fname: &str, v: &Value) -> DbResult<Geometry> {
    parse_geometry(v).ok_or_else(|| {
        DbError::ExecutionError(format!(
            "{fname}: invalid geometry (expected [lon, lat], {{lat, lon}}, a ring array, or GeoJSON)"
        ))
    })
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
    let v = serde_json::json!({ "type": ty, "coordinates": args[0] });
    if parse_geometry(&v).is_none() {
        return Err(DbError::ExecutionError(format!(
            "GEO_{}: invalid coordinates for a {ty}",
            ty.to_uppercase()
        )));
    }
    Ok(v)
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// `[lon, lat, …]` or a `{lat, lon}` / GeoJSON Point object.
fn position(v: &Value) -> Option<Pt> {
    match v {
        Value::Array(a) if a.len() >= 2 => {
            let lon = a[0].as_f64()?;
            let lat = a[1].as_f64()?;
            (lon.is_finite() && lat.is_finite()).then_some((lon, lat))
        }
        Value::Object(_) => GeoPoint::from_value_lonlat(v).map(|p| (p.lon, p.lat)),
        _ => None,
    }
}

fn positions(v: &Value) -> Option<Vec<Pt>> {
    v.as_array()?.iter().map(position).collect()
}

fn line(v: &Value) -> Option<Vec<Pt>> {
    let pts = positions(v)?;
    (pts.len() >= 2).then_some(pts)
}

fn ring(v: &Value) -> Option<Ring> {
    ring_from(positions(v)?)
}

fn ring_from(mut pts: Vec<Pt>) -> Option<Ring> {
    if pts.len() >= 2 && pt_eq(pts[0], pts[pts.len() - 1]) {
        pts.pop();
    }
    (pts.len() >= 3).then_some(pts)
}

fn polygon(v: &Value) -> Option<Poly> {
    let rings: Option<Vec<Ring>> = v.as_array()?.iter().map(ring).collect();
    rings.filter(|r| !r.is_empty())
}

fn parse_geometry(v: &Value) -> Option<Geometry> {
    match v {
        Value::Object(o) => {
            if let Some(ty) = o.get("type").and_then(Value::as_str) {
                let c = o.get("coordinates");
                return match ty {
                    "Point" => position(c?).map(Geometry::Point),
                    "MultiPoint" => positions(c?)
                        .filter(|p| !p.is_empty())
                        .map(Geometry::MultiPoint),
                    "LineString" => line(c?).map(Geometry::LineString),
                    "MultiLineString" => c?
                        .as_array()?
                        .iter()
                        .map(line)
                        .collect::<Option<Vec<_>>>()
                        .filter(|l| !l.is_empty())
                        .map(Geometry::MultiLineString),
                    "Polygon" => polygon(c?).map(Geometry::Polygon),
                    "MultiPolygon" => c?
                        .as_array()?
                        .iter()
                        .map(polygon)
                        .collect::<Option<Vec<_>>>()
                        .filter(|p| !p.is_empty())
                        .map(Geometry::MultiPolygon),
                    "Feature" => parse_geometry(o.get("geometry")?),
                    _ => None,
                };
            }
            position(v).map(Geometry::Point)
        }
        Value::Array(a) => {
            if let Some(p) = position(v).filter(|_| a.iter().all(Value::is_number)) {
                return Some(Geometry::Point(p));
            }
            let first = a.first()?;
            if position(first).is_some() {
                // A list of positions: a ring (the legacy GEO_WITHIN form), or a
                // segment when there are only two.
                let pts = positions(v)?;
                return match pts.len() {
                    1 => Some(Geometry::Point(pts[0])),
                    2 => Some(Geometry::LineString(pts)),
                    _ => ring_from(pts).map(|r| Geometry::Polygon(vec![r])),
                };
            }
            let second_level = first.as_array()?.first()?;
            if position(second_level).is_some() {
                return polygon(v).map(Geometry::Polygon);
            }
            a.iter()
                .map(polygon)
                .collect::<Option<Vec<_>>>()
                .map(Geometry::MultiPolygon)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Planar primitives
// ---------------------------------------------------------------------------

fn pt_eq(a: Pt, b: Pt) -> bool {
    (a.0 - b.0).abs() < POINT_EQ_EPS && (a.1 - b.1).abs() < POINT_EQ_EPS
}

fn cross(o: Pt, a: Pt, b: Pt) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

fn sign(x: f64) -> i8 {
    if x > EPS {
        1
    } else if x < -EPS {
        -1
    } else {
        0
    }
}

/// `p` lies on the closed segment `a`–`b`.
fn on_segment(p: Pt, a: Pt, b: Pt) -> bool {
    sign(cross(a, b, p)) == 0
        && p.0 >= a.0.min(b.0) - EPS
        && p.0 <= a.0.max(b.0) + EPS
        && p.1 >= a.1.min(b.1) - EPS
        && p.1 <= a.1.max(b.1) + EPS
}

/// Closed segments intersect, touching and collinear overlap included.
fn segs_intersect(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let d1 = sign(cross(c, d, a));
    let d2 = sign(cross(c, d, b));
    let d3 = sign(cross(a, b, c));
    let d4 = sign(cross(a, b, d));
    if d1 * d2 < 0 && d3 * d4 < 0 {
        return true;
    }
    on_segment(a, c, d) || on_segment(b, c, d) || on_segment(c, a, b) || on_segment(d, a, b)
}

fn ring_edges(r: &Ring) -> impl Iterator<Item = (Pt, Pt)> + '_ {
    (0..r.len()).map(move |i| (r[i], r[(i + 1) % r.len()]))
}

fn line_segments(l: &[Pt]) -> impl Iterator<Item = (Pt, Pt)> + '_ {
    l.windows(2).map(|w| (w[0], w[1]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Loc {
    Inside,
    Boundary,
    Outside,
}

fn ring_locate(p: Pt, r: &Ring) -> Loc {
    let mut inside = false;
    for (a, b) in ring_edges(r) {
        if on_segment(p, a, b) {
            return Loc::Boundary;
        }
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
    }
    if inside {
        Loc::Inside
    } else {
        Loc::Outside
    }
}

fn poly_locate(p: Pt, poly: &Poly) -> Loc {
    match ring_locate(p, &poly[0]) {
        Loc::Outside => return Loc::Outside,
        Loc::Boundary => return Loc::Boundary,
        Loc::Inside => {}
    }
    for hole in &poly[1..] {
        match ring_locate(p, hole) {
            Loc::Inside => return Loc::Outside,
            Loc::Boundary => return Loc::Boundary,
            Loc::Outside => {}
        }
    }
    Loc::Inside
}

fn polys_locate(p: Pt, polys: &[Poly]) -> Loc {
    let mut best = Loc::Outside;
    for poly in polys {
        match poly_locate(p, poly) {
            Loc::Inside => return Loc::Inside,
            Loc::Boundary => best = Loc::Boundary,
            Loc::Outside => {}
        }
    }
    best
}

fn lerp(a: Pt, b: Pt, t: f64) -> Pt {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Parameter of `q` projected on `a`–`b`, clamped to [0, 1].
fn project(q: Pt, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 <= EPS {
        return 0.0;
    }
    (((q.0 - a.0) * dx + (q.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
}

/// Split `a`–`b` wherever it meets one of `edges`, and check that the middle
/// of every piece satisfies `ok`. Handles concave polygons, and segments that
/// touch or run along the boundary.
fn segment_pieces_ok(a: Pt, b: Pt, edges: &[(Pt, Pt)], ok: impl Fn(Pt) -> bool) -> bool {
    if !ok(a) || !ok(b) {
        return false;
    }
    let mut ts = vec![0.0, 1.0];
    for &(c, d) in edges {
        if !segs_intersect(a, b, c, d) {
            continue;
        }
        let r = (b.0 - a.0, b.1 - a.1);
        let s = (d.0 - c.0, d.1 - c.1);
        let denom = r.0 * s.1 - r.1 * s.0;
        if denom.abs() > EPS {
            let t = ((c.0 - a.0) * s.1 - (c.1 - a.1) * s.0) / denom;
            ts.push(t.clamp(0.0, 1.0));
        } else {
            ts.push(project(c, a, b));
            ts.push(project(d, a, b));
        }
    }
    ts.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    ts.windows(2)
        .filter(|w| w[1] - w[0] > 1e-9)
        .all(|w| ok(lerp(a, b, (w[0] + w[1]) / 2.0)))
}

fn poly_set_edges(polys: &[Poly]) -> Vec<(Pt, Pt)> {
    polys
        .iter()
        .flat_map(|p| p.iter())
        .flat_map(ring_edges)
        .collect()
}

fn segment_in_polys(a: Pt, b: Pt, polys: &[Poly], edges: &[(Pt, Pt)]) -> bool {
    segment_pieces_ok(a, b, edges, |m| polys_locate(m, polys) != Loc::Outside)
}

fn point_on_lines(p: Pt, lines: &[Vec<Pt>]) -> bool {
    lines
        .iter()
        .any(|l| line_segments(l).any(|(a, b)| on_segment(p, a, b)))
}

// ---------------------------------------------------------------------------
// Predicates
// ---------------------------------------------------------------------------

/// Points, segments and polygons a geometry is made of.
struct Parts {
    points: Vec<Pt>,
    segments: Vec<(Pt, Pt)>,
    polys: Vec<Poly>,
}

fn parts(g: &Geometry) -> Parts {
    let mut p = Parts {
        points: Vec::new(),
        segments: Vec::new(),
        polys: Vec::new(),
    };
    match g {
        Geometry::Point(x) => p.points.push(*x),
        Geometry::MultiPoint(xs) => p.points.extend(xs),
        Geometry::LineString(l) => {
            p.points.extend(l);
            p.segments.extend(line_segments(l));
        }
        Geometry::MultiLineString(ls) => {
            for l in ls {
                p.points.extend(l);
                p.segments.extend(line_segments(l));
            }
        }
        Geometry::Polygon(poly) => {
            p.points.extend(&poly[0]);
            p.segments.extend(poly.iter().flat_map(ring_edges));
            p.polys.push(poly.clone());
        }
        Geometry::MultiPolygon(polys) => {
            for poly in polys {
                p.points.extend(&poly[0]);
                p.segments.extend(poly.iter().flat_map(ring_edges));
            }
            p.polys.extend(polys.iter().cloned());
        }
    }
    p
}

fn point_touches(pt: Pt, other: &Parts) -> bool {
    other.points.iter().any(|q| pt_eq(pt, *q))
        || other.segments.iter().any(|&(a, b)| on_segment(pt, a, b))
        || (!other.polys.is_empty() && polys_locate(pt, &other.polys) != Loc::Outside)
}

fn intersects(a: &Geometry, b: &Geometry) -> bool {
    let pa = parts(a);
    let pb = parts(b);
    pa.points.iter().any(|p| point_touches(*p, &pb))
        || pb.points.iter().any(|p| point_touches(*p, &pa))
        || pa
            .segments
            .iter()
            .any(|&(p, q)| pb.segments.iter().any(|&(r, s)| segs_intersect(p, q, r, s)))
}

fn contains(outer: &Geometry, inner: &Geometry) -> bool {
    let pi = parts(inner);
    match outer {
        Geometry::Point(p) => pi.points.iter().all(|q| pt_eq(*p, *q)),
        Geometry::MultiPoint(ps) => {
            pi.segments.is_empty() && pi.points.iter().all(|q| ps.iter().any(|p| pt_eq(*p, *q)))
        }
        Geometry::LineString(l) => lines_contain(std::slice::from_ref(l), inner, &pi),
        Geometry::MultiLineString(ls) => lines_contain(ls, inner, &pi),
        Geometry::Polygon(poly) => polys_contain(std::slice::from_ref(poly), &pi),
        Geometry::MultiPolygon(polys) => polys_contain(polys, &pi),
    }
}

fn lines_contain(lines: &[Vec<Pt>], inner: &Geometry, pi: &Parts) -> bool {
    if matches!(inner, Geometry::Polygon(_) | Geometry::MultiPolygon(_)) {
        return false;
    }
    if !pi.points.iter().all(|p| point_on_lines(*p, lines)) {
        return false;
    }
    let edges: Vec<(Pt, Pt)> = lines.iter().flat_map(|l| line_segments(l)).collect();
    pi.segments
        .iter()
        .all(|&(a, b)| segment_pieces_ok(a, b, &edges, |m| point_on_lines(m, lines)))
}

fn polys_contain(polys: &[Poly], pi: &Parts) -> bool {
    if !pi
        .points
        .iter()
        .all(|p| polys_locate(*p, polys) != Loc::Outside)
    {
        return false;
    }
    let edges = poly_set_edges(polys);
    if !pi
        .segments
        .iter()
        .all(|&(a, b)| segment_in_polys(a, b, polys, &edges))
    {
        return false;
    }
    // Every edge of the inner polygon stays inside, but one of our holes
    // could still sit in its interior.
    for inner_poly in &pi.polys {
        for hole in polys.iter().flat_map(|p| p.iter().skip(1)) {
            let hole_inside = ring_edges(hole).any(|(a, b)| {
                poly_locate(a, inner_poly) == Loc::Inside
                    || poly_locate(lerp(a, b, 0.5), inner_poly) == Loc::Inside
            });
            if hole_inside {
                return false;
            }
        }
    }
    true
}

fn geometry_eq(a: &Geometry, b: &Geometry) -> bool {
    fn seq_eq(x: &[Pt], y: &[Pt]) -> bool {
        x.len() == y.len() && x.iter().zip(y).all(|(p, q)| pt_eq(*p, *q))
    }
    fn poly_eq(x: &Poly, y: &Poly) -> bool {
        x.len() == y.len() && x.iter().zip(y).all(|(r, s)| seq_eq(r, s))
    }
    match (a, b) {
        (Geometry::Point(p), Geometry::Point(q)) => pt_eq(*p, *q),
        (Geometry::MultiPoint(x), Geometry::MultiPoint(y))
        | (Geometry::LineString(x), Geometry::LineString(y)) => seq_eq(x, y),
        (Geometry::MultiLineString(x), Geometry::MultiLineString(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(l, m)| seq_eq(l, m))
        }
        (Geometry::Polygon(x), Geometry::Polygon(y)) => poly_eq(x, y),
        (Geometry::MultiPolygon(x), Geometry::MultiPolygon(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| poly_eq(p, q))
        }
        _ => false,
    }
}

/// Vertex average — outer rings only for polygons.
fn centroid(g: &Geometry) -> Pt {
    if let Geometry::Point(p) = g {
        return *p;
    }
    let pts = parts(g).points;
    let n = pts.len().max(1) as f64;
    let (sx, sy) = pts.iter().fold((0.0, 0.0), |(x, y), p| (x + p.0, y + p.1));
    (sx / n, sy / n)
}

/// Spherical ring area (Chamberlain & Duquette), in m².
fn ring_area(r: &Ring, radius: f64) -> f64 {
    let n = r.len();
    if n < 3 {
        return 0.0;
    }
    let mut total = 0.0;
    for i in 0..n {
        let lower = r[i];
        let middle = r[(i + 1) % n];
        let upper = r[(i + 2) % n];
        total += (upper.0.to_radians() - lower.0.to_radians()) * middle.1.to_radians().sin();
    }
    (total * radius * radius / 2.0).abs()
}

fn poly_area(p: &Poly, radius: f64) -> f64 {
    let outer = ring_area(&p[0], radius);
    let holes: f64 = p[1..].iter().map(|h| ring_area(h, radius)).sum();
    (outer - holes).max(0.0)
}

fn geo_area(g: &Geometry, radius: f64) -> f64 {
    match g {
        Geometry::Polygon(p) => poly_area(p, radius),
        Geometry::MultiPolygon(ps) => ps.iter().map(|p| poly_area(p, radius)).sum(),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, args: &[Value]) -> Value {
        evaluate(name, args).unwrap().unwrap()
    }

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
        json!([[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]])
    }

    #[test]
    fn bare_arrays_are_lon_lat() {
        // Paris [lon, lat] inside a box around it.
        let poly = json!({"type":"Polygon","coordinates":[square(2.0, 48.0, 3.0, 49.0)]});
        assert_eq!(
            call("GEO_CONTAINS", &[poly.clone(), json!([2.35, 48.85])]),
            json!(true)
        );
        assert_eq!(
            call("GEO_CONTAINS", &[poly, json!([48.85, 2.35])]),
            json!(false)
        );
        let d = call(
            "GEO_DISTANCE",
            &[json!([2.3522, 48.8566]), json!([-0.1278, 51.5074])],
        );
        let km = d.as_f64().unwrap() / 1000.0;
        assert!(km > 340.0 && km < 350.0, "{km}");
        // {lat, lon} objects still work.
        let d2 = call(
            "GEO_DISTANCE",
            &[
                json!({"lat":48.8566,"lon":2.3522}),
                json!({"lat":51.5074,"lon":-0.1278}),
            ],
        );
        assert!((d2.as_f64().unwrap() - d.as_f64().unwrap()).abs() < 1e-6);
    }

    #[test]
    fn holes_are_honoured() {
        let donut = json!({"type":"Polygon","coordinates":[square(0.0,0.0,10.0,10.0), square(4.0,4.0,6.0,6.0)]});
        assert_eq!(
            call("GEO_CONTAINS", &[donut.clone(), json!([5, 5])]),
            json!(false)
        );
        assert_eq!(
            call("GEO_CONTAINS", &[donut.clone(), json!([2, 2])]),
            json!(true)
        );
        assert_eq!(
            call("GEO_WITHIN", &[json!([5, 5]), donut.clone()]),
            json!(false)
        );
        // A polygon covering the hole is not contained.
        let inner = json!({"type":"Polygon","coordinates":[square(3.0,3.0,7.0,7.0)]});
        assert_eq!(call("GEO_CONTAINS", &[donut.clone(), inner]), json!(false));
        let small = json!({"type":"Polygon","coordinates":[square(1.0,1.0,2.0,2.0)]});
        assert_eq!(call("GEO_CONTAINS", &[donut.clone(), small]), json!(true));
        // A polygon sitting inside the hole does not intersect the donut.
        let in_hole = json!({"type":"Polygon","coordinates":[square(4.5,4.5,5.5,5.5)]});
        assert_eq!(call("GEO_INTERSECTS", &[donut, in_hole]), json!(false));
    }

    #[test]
    fn concave_polygon_segment_leaving_through_the_notch() {
        // A "U": the segment between the two arms crosses the notch.
        let u = json!({"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[7,10],[7,3],[3,3],[3,10],[0,10],[0,0]]]});
        let line = json!({"type":"LineString","coordinates":[[1,8],[9,8]]});
        assert_eq!(
            call("GEO_CONTAINS", &[u.clone(), line.clone()]),
            json!(false)
        );
        assert_eq!(call("GEO_INTERSECTS", &[u.clone(), line]), json!(true));
        let inside = json!({"type":"LineString","coordinates":[[1,1],[9,1]]});
        assert_eq!(call("GEO_CONTAINS", &[u, inside]), json!(true));
    }

    #[test]
    fn collinear_touching_counts() {
        let a = json!({"type":"Polygon","coordinates":[square(0.0,0.0,1.0,1.0)]});
        let b = json!({"type":"Polygon","coordinates":[square(1.0,0.0,2.0,1.0)]});
        assert_eq!(call("GEO_INTERSECTS", &[a.clone(), b]), json!(true));
        // A segment along an edge is contained (boundary inclusive).
        let edge = json!({"type":"LineString","coordinates":[[0,0],[1,0]]});
        assert_eq!(call("GEO_CONTAINS", &[a, edge]), json!(true));
    }

    #[test]
    fn multi_and_line_predicates() {
        let mp = call("GEO_MULTIPOINT", &[json!([[1, 1], [20, 20]])]);
        let poly = call("GEO_POLYGON", &[json!([square(0.0, 0.0, 10.0, 10.0)])]);
        assert_eq!(
            call("GEO_INTERSECTS", &[poly.clone(), mp.clone()]),
            json!(true)
        );
        assert_eq!(call("GEO_CONTAINS", &[poly.clone(), mp]), json!(false));
        let ls = call("GEO_LINESTRING", &[json!([[-5, 5], [15, 5]])]);
        assert_eq!(
            call("GEO_INTERSECTS", &[ls.clone(), poly.clone()]),
            json!(true)
        );
        assert_eq!(
            call("GEO_CONTAINS", &[ls.clone(), json!([0, 5])]),
            json!(true)
        );
        let mpoly = call(
            "GEO_MULTIPOLYGON",
            &[json!([
                [square(0.0, 0.0, 1.0, 1.0)],
                [square(5.0, 5.0, 6.0, 6.0)]
            ])],
        );
        assert_eq!(
            call("GEO_CONTAINS", &[mpoly.clone(), json!([5.5, 5.5])]),
            json!(true)
        );
        assert_eq!(call("GEO_CONTAINS", &[mpoly, json!([3, 3])]), json!(false));
        let mls = call(
            "GEO_MULTILINESTRING",
            &[json!([[[0, 0], [1, 1]], [[5, 0], [5, 9]]])],
        );
        assert_eq!(call("GEO_INTERSECTS", &[mls, ls]), json!(true));
    }

    #[test]
    fn area_is_spherical() {
        let one_deg = call(
            "GEO_AREA",
            &[call("GEO_POLYGON", &[json!([square(0.0, 0.0, 1.0, 1.0)])])],
        );
        let a = one_deg.as_f64().unwrap();
        assert!(a > 1.20e10 && a < 1.25e10, "{a}");
        // The same box at 60°N is about half as large.
        let north = call(
            "GEO_AREA",
            &[call(
                "GEO_POLYGON",
                &[json!([square(0.0, 60.0, 1.0, 61.0)])],
            )],
        );
        let ratio = north.as_f64().unwrap() / a;
        assert!(ratio > 0.45 && ratio < 0.52, "{ratio}");
        let donut = json!({"type":"Polygon","coordinates":[square(0.0,0.0,2.0,2.0), square(0.5,0.5,1.5,1.5)]});
        let d = call("GEO_AREA", &[donut]).as_f64().unwrap();
        assert!((d / a - 3.0).abs() < 0.05, "{}", d / a);
    }

    #[test]
    fn in_range_bounds_and_validation() {
        let p = json!([0, 0]);
        assert_eq!(
            call("GEO_IN_RANGE", &[p.clone(), p.clone(), json!(0), json!(10)]),
            json!(true)
        );
        assert_eq!(
            call(
                "GEO_IN_RANGE",
                &[p.clone(), p.clone(), json!(0), json!(10), json!(false)]
            ),
            json!(false)
        );
        assert!(evaluate(
            "GEO_IN_RANGE",
            &[p.clone(), p.clone(), json!("a"), json!(10)]
        )
        .is_err());
        assert_eq!(
            call(
                "GEO_IN_RANGE",
                &[Value::Null, p.clone(), json!(0), json!(10)]
            ),
            Value::Null
        );
    }

    #[test]
    fn nulls_and_invalid_coordinates() {
        assert_eq!(
            call("GEO_DISTANCE", &[Value::Null, json!([0, 0])]),
            Value::Null
        );
        assert_eq!(
            call("DISTANCE", &[json!(0), json!(0), Value::Null, json!(0)]),
            Value::Null
        );
        assert_eq!(
            call("DISTANCE", &[json!(95), json!(0), json!(0), json!(0)]),
            Value::Null
        );
        assert_eq!(
            call("GEO_CONTAINS", &[Value::Null, json!([0, 0])]),
            Value::Null
        );
        assert!(evaluate("GEO_CONTAINS", &[json!("x"), json!([0, 0])]).is_err());
        let antipodal = call("DISTANCE", &[json!(0), json!(0), json!(0), json!(180)]);
        assert!(antipodal.as_f64().unwrap() > 2.0e7);
    }

    #[test]
    fn legacy_ring_forms_still_work() {
        assert_eq!(
            call(
                "GEO_WITHIN",
                &[
                    json!({"lat": 5, "lon": 5}),
                    json!([[0, 0], [10, 0], [10, 10], [0, 10]])
                ]
            ),
            json!(true)
        );
        assert_eq!(
            call(
                "GEO_WITHIN",
                &[
                    json!({"lat": 15, "lon": 5}),
                    json!([[0, 0], [10, 0], [10, 10], [0, 10]])
                ]
            ),
            json!(false)
        );
        assert!(evaluate("GEO_WITHIN", &[json!([0, 0]), json!([[0, 0], [1, 1]])]).is_err());
    }
}
