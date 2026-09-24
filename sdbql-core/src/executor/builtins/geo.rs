//! Geospatial builtin functions (pure math, no index).
//!
//! Coordinates follow GeoJSON: a bare `[a, b]` array is `[lon, lat]`, as
//! are GeoJSON `coordinates`. Objects may use `lat`/`lon` (or `lng`,
//! `latitude`, `longitude`). Distances are Haversine metres on a sphere of
//! radius 6 371 km; areas use the spherical-excess formula.

use serde_json::{json, Value};

use super::common::{check_arity, err};
use crate::error::SdbqlResult;
use crate::executor::helpers::number_from_f64;

const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// (lon, lat)
type P = (f64, f64);

enum Geom {
    Point(P),
    MultiPoint(Vec<P>),
    Line(Vec<P>),
    MultiLine(Vec<Vec<P>>),
    /// Rings: the first is the outer boundary, the rest are holes.
    Polygon(Vec<Vec<P>>),
    MultiPolygon(Vec<Vec<Vec<P>>>),
}

fn num_pair(v: &Value) -> Option<P> {
    let a = v.as_array()?;
    if a.len() < 2 {
        return None;
    }
    Some((a[0].as_f64()?, a[1].as_f64()?))
}

fn points(v: &Value) -> Option<Vec<P>> {
    v.as_array()?.iter().map(num_pair).collect()
}

fn rings(v: &Value) -> Option<Vec<Vec<P>>> {
    v.as_array()?.iter().map(points).collect()
}

fn object_point(o: &serde_json::Map<String, Value>) -> Option<P> {
    let lat = o.get("lat").or_else(|| o.get("latitude"))?.as_f64()?;
    let lon = o
        .get("lon")
        .or_else(|| o.get("lng"))
        .or_else(|| o.get("longitude"))?
        .as_f64()?;
    Some((lon, lat))
}

/// Array depth of a coordinates value: `[x, y]` is 1, `[[x, y], …]` is 2.
fn depth(v: &Value) -> usize {
    match v {
        Value::Array(a) => 1 + a.first().map(depth).unwrap_or(0),
        _ => 0,
    }
}

fn parse_geom(v: &Value) -> Option<Geom> {
    match v {
        Value::Object(o) => {
            if let Some(ty) = o.get("type").and_then(Value::as_str) {
                let c = o.get("coordinates")?;
                return Some(match ty {
                    "Point" => Geom::Point(num_pair(c)?),
                    "MultiPoint" => Geom::MultiPoint(points(c)?),
                    "LineString" => Geom::Line(points(c)?),
                    "MultiLineString" => Geom::MultiLine(rings(c)?),
                    "Polygon" => Geom::Polygon(rings(c)?),
                    "MultiPolygon" => {
                        Geom::MultiPolygon(c.as_array()?.iter().map(rings).collect::<Option<_>>()?)
                    }
                    _ => return None,
                });
            }
            object_point(o).map(Geom::Point)
        }
        // Bare arrays: a point, a ring of points, or polygon rings.
        Value::Array(_) => match depth(v) {
            1 => num_pair(v).map(Geom::Point),
            2 => {
                let pts = points(v)?;
                Some(if pts.len() >= 3 {
                    Geom::Polygon(vec![pts])
                } else {
                    Geom::Line(pts)
                })
            }
            3 => rings(v).map(Geom::Polygon),
            _ => None,
        },
        _ => None,
    }
}

fn as_point(v: &Value) -> Option<P> {
    match parse_geom(v)? {
        Geom::Point(p) => Some(p),
        _ => None,
    }
}

fn haversine(a: P, b: P) -> f64 {
    let (lat1, lat2) = (a.1.to_radians(), b.1.to_radians());
    let dlat = (b.1 - a.1).to_radians();
    let dlon = (b.0 - a.0).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    // Rounding can push `h` just past 1, and asin(sqrt(>1)) is NaN.
    2.0 * EARTH_RADIUS_M * h.clamp(0.0, 1.0).sqrt().asin()
}

fn vertices(g: &Geom) -> Vec<P> {
    match g {
        Geom::Point(p) => vec![*p],
        Geom::MultiPoint(ps) | Geom::Line(ps) => ps.clone(),
        Geom::MultiLine(ls) | Geom::Polygon(ls) => ls.iter().flatten().copied().collect(),
        Geom::MultiPolygon(ps) => ps.iter().flatten().flatten().copied().collect(),
    }
}

/// Every edge (lines and polygon rings, rings closed).
fn segments(g: &Geom) -> Vec<(P, P)> {
    fn open(pts: &[P], out: &mut Vec<(P, P)>) {
        out.extend(pts.windows(2).map(|w| (w[0], w[1])));
    }
    fn closed(pts: &[P], out: &mut Vec<(P, P)>) {
        open(pts, out);
        if let (Some(&f), Some(&l)) = (pts.first(), pts.last()) {
            if pts.len() > 2 && f != l {
                out.push((l, f));
            }
        }
    }
    let mut out = Vec::new();
    match g {
        Geom::Point(_) | Geom::MultiPoint(_) => {}
        Geom::Line(ps) => open(ps, &mut out),
        Geom::MultiLine(ls) => ls.iter().for_each(|l| open(l, &mut out)),
        Geom::Polygon(rs) => rs.iter().for_each(|r| closed(r, &mut out)),
        Geom::MultiPolygon(ps) => ps.iter().flatten().for_each(|r| closed(r, &mut out)),
    }
    out
}

fn polygons(g: &Geom) -> Vec<&Vec<Vec<P>>> {
    match g {
        Geom::Polygon(rs) => vec![rs],
        Geom::MultiPolygon(ps) => ps.iter().collect(),
        _ => Vec::new(),
    }
}

/// Ray casting on (lon, lat) as a plane.
fn in_ring(p: P, ring: &[P]) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = ring[i];
        let (xj, yj) = ring[j];
        if (yi > p.1) != (yj > p.1) && p.0 < (xj - xi) * (p.1 - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Inside the outer ring and outside every hole.
fn in_polygon(p: P, rings: &[Vec<P>]) -> bool {
    match rings.split_first() {
        Some((outer, holes)) => in_ring(p, outer) && !holes.iter().any(|h| in_ring(p, h)),
        None => false,
    }
}

fn cross(o: P, a: P, b: P) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

fn on_segment(p: P, a: P, b: P) -> bool {
    cross(a, b, p).abs() < 1e-12
        && p.0 >= a.0.min(b.0) - 1e-12
        && p.0 <= a.0.max(b.0) + 1e-12
        && p.1 >= a.1.min(b.1) - 1e-12
        && p.1 <= a.1.max(b.1) + 1e-12
}

/// Segments cross or touch.
fn segs_touch(a: P, b: P, c: P, d: P) -> bool {
    let (d1, d2, d3, d4) = (
        cross(a, b, c),
        cross(a, b, d),
        cross(c, d, a),
        cross(c, d, b),
    );
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    on_segment(c, a, b) || on_segment(d, a, b) || on_segment(a, c, d) || on_segment(b, c, d)
}

/// Segments cross properly (not merely touching at an end or along a side).
fn segs_cross(a: P, b: P, c: P, d: P) -> bool {
    let (d1, d2, d3, d4) = (
        cross(a, b, c),
        cross(a, b, d),
        cross(c, d, a),
        cross(c, d, b),
    );
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

fn geo_contains(outer: &Geom, inner: &Geom) -> bool {
    let polys = polygons(outer);
    if polys.is_empty() {
        // Only a point "contains" an equal point.
        return match (outer, inner) {
            (Geom::Point(a), Geom::Point(b)) => a == b,
            _ => false,
        };
    }
    let verts = vertices(inner);
    if verts.is_empty() {
        return false;
    }
    let inside = |p: P| polys.iter().any(|rs| in_polygon(p, rs));
    if !verts.iter().all(|&p| inside(p)) {
        return false;
    }
    // A line or polygon with every vertex inside can still leave through a
    // concave edge or around a hole.
    let inner_segs = segments(inner);
    let outer_segs = segments(outer);
    if inner_segs
        .iter()
        .any(|&(a, b)| outer_segs.iter().any(|&(c, d)| segs_cross(a, b, c, d)))
    {
        return false;
    }
    // A hole of `outer` lying inside `inner` is not contained.
    let inner_polys = polygons(inner);
    !polys.iter().flat_map(|rs| rs.iter().skip(1)).any(|hole| {
        hole.iter()
            .any(|&p| inner_polys.iter().any(|rs| in_polygon(p, rs)))
    })
}

fn geo_intersects(a: &Geom, b: &Geom) -> bool {
    let (va, vb) = (vertices(a), vertices(b));
    let (pa, pb) = (polygons(a), polygons(b));
    let (sa, sb) = (segments(a), segments(b));
    let point_hits = |p: P, polys: &[&Vec<Vec<P>>], segs: &[(P, P)], verts: &[P]| {
        polys.iter().any(|rs| in_polygon(p, rs))
            || segs.iter().any(|&(c, d)| on_segment(p, c, d))
            || verts.contains(&p)
    };
    va.iter().any(|&p| point_hits(p, &pb, &sb, &vb))
        || vb.iter().any(|&p| point_hits(p, &pa, &sa, &va))
        || sa
            .iter()
            .any(|&(p, q)| sb.iter().any(|&(r, s)| segs_touch(p, q, r, s)))
}

/// Spherical area of a ring in m² (Chamberlain & Duquette).
fn ring_area(ring: &[P]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let mut s = 0.0;
    for i in 0..n {
        let (lon1, lat1) = ring[i];
        let (lon2, lat2) = ring[(i + 1) % n];
        s += (lon2 - lon1).to_radians() * (2.0 + lat1.to_radians().sin() + lat2.to_radians().sin());
    }
    (s * EARTH_RADIUS_M * EARTH_RADIUS_M / 2.0).abs()
}

fn geo_area(g: &Geom) -> f64 {
    polygons(g)
        .iter()
        .map(|rs| match rs.split_first() {
            Some((outer, holes)) => {
                (ring_area(outer) - holes.iter().map(|h| ring_area(h)).sum::<f64>()).max(0.0)
            }
            None => 0.0,
        })
        .sum()
}

fn geom_arg(name: &str, args: &[Value], i: usize) -> SdbqlResult<Geom> {
    parse_geom(&args[i]).ok_or_else(|| {
        err(format!(
            "{}: argument {} must be a GeoJSON geometry, a coordinate array or a {{lat, lon}} object",
            name,
            i + 1
        ))
    })
}

fn point_arg(name: &str, args: &[Value], i: usize) -> SdbqlResult<P> {
    as_point(&args[i])
        .ok_or_else(|| err(format!("{}: argument {} must be a geo point", name, i + 1)))
}

fn num_arg(name: &str, args: &[Value], i: usize) -> SdbqlResult<f64> {
    args[i]
        .as_f64()
        .ok_or_else(|| err(format!("{}: argument {} must be a number", name, i + 1)))
}

fn metres(v: f64) -> Value {
    Value::Number(number_from_f64(v))
}

/// Call a geo function. Returns None if function not found.
pub fn call(name: &str, args: &[Value]) -> SdbqlResult<Option<Value>> {
    let result = match name {
        "DISTANCE" => {
            // DISTANCE(lat1, lon1, lat2, lon2), as documented and as in AQL.
            check_arity(name, args, 4, 4)?;
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let a = (num_arg(name, args, 1)?, num_arg(name, args, 0)?);
            let b = (num_arg(name, args, 3)?, num_arg(name, args, 2)?);
            Some(metres(haversine(a, b)))
        }
        "GEO_DISTANCE" => {
            check_arity(name, args, 2, 2)?;
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            Some(metres(haversine(
                point_arg(name, args, 0)?,
                point_arg(name, args, 1)?,
            )))
        }
        "GEO_EQUALS" => {
            check_arity(name, args, 2, 2)?;
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let a = geom_arg(name, args, 0)?;
            let b = geom_arg(name, args, 1)?;
            let (va, vb) = (vertices(&a), vertices(&b));
            const EPS: f64 = 1e-9;
            Some(Value::Bool(
                va.len() == vb.len()
                    && va
                        .iter()
                        .zip(&vb)
                        .all(|(p, q)| (p.0 - q.0).abs() < EPS && (p.1 - q.1).abs() < EPS),
            ))
        }
        "GEO_WITHIN" => {
            // GEO_WITHIN(geometry, polygon) = GEO_CONTAINS(polygon, geometry)
            check_arity(name, args, 2, 2)?;
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let inner = geom_arg(name, args, 0)?;
            let outer = geom_arg(name, args, 1)?;
            if polygons(&outer).is_empty() {
                return Err(err("GEO_WITHIN: second argument must be a polygon"));
            }
            Some(Value::Bool(geo_contains(&outer, &inner)))
        }
        "GEO_CONTAINS" => {
            check_arity(name, args, 2, 2)?;
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let outer = geom_arg(name, args, 0)?;
            let inner = geom_arg(name, args, 1)?;
            Some(Value::Bool(geo_contains(&outer, &inner)))
        }
        "GEO_INTERSECTS" => {
            check_arity(name, args, 2, 2)?;
            if args.iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let a = geom_arg(name, args, 0)?;
            let b = geom_arg(name, args, 1)?;
            Some(Value::Bool(geo_intersects(&a, &b)))
        }
        "GEO_IN_RANGE" => {
            // GEO_IN_RANGE(point, origin, low, high, includeLow, includeHigh)
            check_arity(name, args, 4, 6)?;
            if args[..4].iter().any(Value::is_null) {
                return Ok(Some(Value::Null));
            }
            let d = haversine(point_arg(name, args, 0)?, point_arg(name, args, 1)?);
            let lo = num_arg(name, args, 2)?;
            let hi = num_arg(name, args, 3)?;
            let inc_lo = args.get(4).and_then(Value::as_bool).unwrap_or(true);
            let inc_hi = args.get(5).and_then(Value::as_bool).unwrap_or(true);
            let above = if inc_lo { d >= lo } else { d > lo };
            let below = if inc_hi { d <= hi } else { d < hi };
            Some(Value::Bool(above && below))
        }
        "GEO_AREA" => {
            check_arity(name, args, 1, 2)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            Some(metres(geo_area(&geom_arg(name, args, 0)?)))
        }
        "GEO_POINT" => {
            // SoliDB documents GEO_POINT(lat, lon); it stores [lon, lat].
            check_arity(name, args, 2, 2)?;
            let lat = num_arg(name, args, 0)?;
            let lon = num_arg(name, args, 1)?;
            Some(json!({ "type": "Point", "coordinates": [lon, lat] }))
        }
        "GEO_LINESTRING"
        | "GEO_POLYGON"
        | "GEO_MULTIPOINT"
        | "GEO_MULTILINESTRING"
        | "GEO_MULTIPOLYGON" => {
            check_arity(name, args, 1, 1)?;
            let ty = match name {
                "GEO_LINESTRING" => "LineString",
                "GEO_POLYGON" => "Polygon",
                "GEO_MULTIPOINT" => "MultiPoint",
                "GEO_MULTILINESTRING" => "MultiLineString",
                _ => "MultiPolygon",
            };
            let value = json!({ "type": ty, "coordinates": args[0] });
            // Validate the coordinates' shape now rather than returning a
            // geometry every predicate would reject.
            if parse_geom(&value).is_none() {
                return Err(err(format!("{}: invalid coordinates", name)));
            }
            Some(value)
        }
        _ => None,
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(name: &str, args: &[Value]) -> Value {
        call(name, args).unwrap().unwrap()
    }

    fn square(n: f64) -> Value {
        json!([[[0.0, 0.0], [n, 0.0], [n, n], [0.0, n], [0.0, 0.0]]])
    }

    #[test]
    fn distance_paris_london() {
        let d = ok(
            "DISTANCE",
            &[
                json!(48.8566),
                json!(2.3522),
                json!(51.5074),
                json!(-0.1278),
            ],
        );
        let km = d.as_f64().unwrap() / 1000.0;
        assert!(km > 340.0 && km < 350.0, "{km}");
        let d2 = ok(
            "GEO_DISTANCE",
            &[
                json!({"lat": 48.8566, "lon": 2.3522}),
                json!([-0.1278, 51.5074]),
            ],
        );
        assert!((d2.as_f64().unwrap() - d.as_f64().unwrap()).abs() < 1e-6);
        // Antipodes: haversine used to be able to produce NaN.
        let far = ok("GEO_DISTANCE", &[json!([0, 0]), json!([180, 0])]);
        assert!(far.as_f64().unwrap() > 20_000_000.0);
    }

    #[test]
    fn bare_arrays_are_lon_lat() {
        let poly = json!({"type": "Polygon", "coordinates": square(10.0)});
        // [lon, lat] = [5, 1] is inside; read as [lat, lon] it would be too,
        // so use an asymmetric box.
        let strip = json!({"type": "Polygon",
            "coordinates": [[[0, 0], [20, 0], [20, 2], [0, 2], [0, 0]]]});
        assert_eq!(
            ok("GEO_CONTAINS", &[strip.clone(), json!([15, 1])]),
            json!(true)
        );
        assert_eq!(ok("GEO_CONTAINS", &[strip, json!([1, 15])]), json!(false));
        assert_eq!(
            ok(
                "GEO_CONTAINS",
                &[poly, ok("GEO_POINT", &[json!(5), json!(5)])]
            ),
            json!(true)
        );
    }

    #[test]
    fn contains_lines_polygons_and_holes() {
        let with_hole = json!({"type": "Polygon", "coordinates": [
            [[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]],
            [[4, 4], [6, 4], [6, 6], [4, 6], [4, 4]]
        ]});
        assert_eq!(
            ok("GEO_CONTAINS", &[with_hole.clone(), json!([5, 5])]),
            json!(false)
        );
        assert_eq!(
            ok("GEO_CONTAINS", &[with_hole.clone(), json!([2, 2])]),
            json!(true)
        );
        let line = ok("GEO_LINESTRING", &[json!([[1, 1], [2, 2]])]);
        assert_eq!(ok("GEO_CONTAINS", &[with_hole.clone(), line]), json!(true));
        // Crosses the hole.
        let through = ok("GEO_LINESTRING", &[json!([[3, 5], [7, 5]])]);
        assert_eq!(
            ok("GEO_CONTAINS", &[with_hole.clone(), through.clone()]),
            json!(false)
        );
        assert_eq!(ok("GEO_INTERSECTS", &[with_hole, through]), json!(true));
        let mp = ok("GEO_MULTIPOINT", &[json!([[1, 1], [2, 2]])]);
        let sq = json!({"type": "Polygon", "coordinates": square(10.0)});
        assert_eq!(ok("GEO_CONTAINS", &[sq.clone(), mp]), json!(true));
        let small = json!({"type": "Polygon", "coordinates": [[[1, 1], [2, 1], [2, 2], [1, 1]]]});
        assert_eq!(ok("GEO_CONTAINS", &[sq, small]), json!(true));
    }

    #[test]
    fn intersects_lines() {
        let a = ok("GEO_LINESTRING", &[json!([[0, 0], [10, 10]])]);
        let b = ok("GEO_LINESTRING", &[json!([[0, 10], [10, 0]])]);
        let c = ok("GEO_LINESTRING", &[json!([[20, 20], [30, 30]])]);
        assert_eq!(ok("GEO_INTERSECTS", &[a.clone(), b]), json!(true));
        assert_eq!(ok("GEO_INTERSECTS", &[a, c]), json!(false));
    }

    #[test]
    fn area_is_spherical() {
        // 1°×1° at the equator is about 12 364 km².
        let eq = ok(
            "GEO_AREA",
            &[json!({"type": "Polygon", "coordinates": square(1.0)})],
        );
        let km2 = eq.as_f64().unwrap() / 1e6;
        assert!((km2 - 12_364.0).abs() < 50.0, "{km2}");
        // At 60°N the same box is about half as large (cos 60° = 0.5).
        let north = json!({"type": "Polygon",
            "coordinates": [[[0, 60], [1, 60], [1, 61], [0, 61], [0, 60]]]});
        let n_km2 = ok("GEO_AREA", &[north]).as_f64().unwrap() / 1e6;
        assert!(n_km2 < km2 * 0.52 && n_km2 > km2 * 0.45, "{n_km2}");
    }

    #[test]
    fn in_range_and_errors() {
        let o = json!({"lat": 0, "lon": 0});
        let p = json!([0.001, 0]);
        assert_eq!(
            ok(
                "GEO_IN_RANGE",
                &[p.clone(), o.clone(), json!(0), json!(1000)]
            ),
            json!(true)
        );
        assert_eq!(
            ok(
                "GEO_IN_RANGE",
                &[p, o, json!(0), json!(50), json!(true), json!(false)]
            ),
            json!(false)
        );
        assert!(call("GEO_DISTANCE", &[json!("x"), json!([0, 0])]).is_err());
        assert!(call("GEO_POLYGON", &[json!("x")]).is_err());
        assert_eq!(
            ok("GEO_DISTANCE", &[Value::Null, json!([0, 0])]),
            Value::Null
        );
        // An empty ring must not underflow `n - 1`.
        assert_eq!(
            ok(
                "GEO_CONTAINS",
                &[
                    json!({"type": "Polygon", "coordinates": [[]]}),
                    json!([0, 0])
                ]
            ),
            json!(false)
        );
    }
}
