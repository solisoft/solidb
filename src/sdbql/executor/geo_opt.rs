//! Geo-index optimizer rules (audit P11).
//!
//! Two shapes are served from a collection's geo index instead of a full
//! document scan:
//!
//! - `FOR d IN c FILTER DISTANCE(d.loc.lat, d.loc.lon, lat0, lon0) <= r` (and
//!   `GEO_DISTANCE(d.loc, point) <= r`, `<`, or the mirrored `r >= …`), as one
//!   conjunct of the FILTER. The index returns a candidate *superset*; the
//!   caller re-evaluates the whole FILTER on it, as it does for every index
//!   read, so exactness never depends on this module's distance maths.
//! - `FOR d IN c [LET …]* SORT DISTANCE(…) ASC LIMIT [o,] n RETURN …`: the
//!   index yields the `o + n` nearest entries; every document within that
//!   distance (plus a float epsilon) is fetched, and the query's own SORT and
//!   LIMIT run on that set. Ties therefore break exactly as on the scan path
//!   (both see documents in key order, and the sort is stable).
//!
//! When a document is absent from the geo index its location field is null or
//! missing. The FILTER rule probes the conjunct on an empty document and bails
//! if that would pass (so a future `DISTANCE(null, …) → null` and
//! `null <= r → true` cannot silently drop rows); the SORT rule requires every
//! document to be indexed with an unambiguous point.
//!
//! The geo index stores the raw field value per document key (no geohash
//! cells), so the index read is still linear in the number of indexed
//! documents — but over small entries, with no document decode, and with a
//! bounded heap for the nearest-`n` case.

use std::collections::BinaryHeap;

use serde_json::Value;

use super::types::{Context, MutationStats, QueryExecutionResult};
use super::window::contains_window_functions;
use super::QueryExecutor;
use crate::error::DbResult;
use crate::sdbql::ast::*;
use crate::storage::geo::distance_meters;
use crate::storage::{Collection, Document};

/// Relative slack added to a radius or nearest-`n` threshold before comparing,
/// so a last-bit difference between this module's haversine and the builtin's
/// can only add candidates, never drop one.
const REL_EPS: f64 = 1e-9;
const ABS_EPS_M: f64 = 1e-3;

/// How the document side of a distance call reads the indexed field.
#[derive(Debug, Clone)]
enum DocSide {
    /// `DISTANCE(d.F.<lat_key>, d.F.<lon_key>, …)`
    Keys { lat_key: String, lon_key: String },
    /// `GEO_DISTANCE(d.F, …)`: the builtin parses the whole value as a point.
    Point,
}

/// A `DISTANCE`/`GEO_DISTANCE` call whose one side is an indexed field of the
/// loop variable and whose other side is constant for the row.
#[derive(Debug, Clone)]
pub(super) struct GeoDistanceCall {
    /// Field path of the geo index (`loc` in `d.loc.lat`)
    field: String,
    doc: DocSide,
    /// Possible readings of the constant point as `(lat, lon)`: two when it is
    /// a bare `[a, b]` array, whose axis order is interpreted by the builtin.
    refs: Vec<(f64, f64)>,
}

/// Readings of a stored or constant point as `(lat, lon)`. `None` when the
/// value is not recognisably a point (the caller then treats it as unknown).
fn point_readings(v: &Value) -> Option<Vec<(f64, f64)>> {
    match v {
        Value::Object(obj) => {
            if obj.get("type").and_then(Value::as_str) == Some("Point") {
                let c = obj.get("coordinates")?.as_array()?;
                if c.len() >= 2 {
                    return Some(vec![(c[1].as_f64()?, c[0].as_f64()?)]);
                }
                return None;
            }
            let lat = obj.get("lat").or(obj.get("latitude"))?.as_f64()?;
            let lon = obj
                .get("lon")
                .or(obj.get("lng"))
                .or(obj.get("longitude"))?
                .as_f64()?;
            Some(vec![(lat, lon)])
        }
        Value::Array(a) if a.len() == 2 => {
            let (x, y) = (a[0].as_f64()?, a[1].as_f64()?);
            Some(vec![(x, y), (y, x)])
        }
        _ => None,
    }
}

impl GeoDistanceCall {
    /// Readings of one index entry; `None` = cannot tell, keep as candidate.
    fn entry_points(&self, v: &Value) -> Option<Vec<(f64, f64)>> {
        match &self.doc {
            DocSide::Keys { lat_key, lon_key } => {
                let lat = v.get(lat_key)?.as_f64()?;
                let lon = v.get(lon_key)?.as_f64()?;
                Some(vec![(lat, lon)])
            }
            DocSide::Point => point_readings(v),
        }
    }

    /// Smallest distance over every (entry reading, reference reading) pair.
    /// `None` when the entry cannot be read or a distance is NaN.
    fn min_distance(&self, v: &Value) -> Option<f64> {
        let points = self.entry_points(v)?;
        let mut best = f64::INFINITY;
        for (lat, lon) in &points {
            for (rlat, rlon) in &self.refs {
                let d = distance_meters(*lat, *lon, *rlat, *rlon);
                if d.is_nan() {
                    return None;
                }
                best = best.min(d);
            }
        }
        Some(best)
    }

    /// Exact single distance, for SORT: only when both sides have exactly one
    /// reading.
    fn exact_distance(&self, v: &Value) -> Option<f64> {
        if self.refs.len() != 1 {
            return None;
        }
        let points = self.entry_points(v)?;
        if points.len() != 1 {
            return None;
        }
        let d = distance_meters(points[0].0, points[0].1, self.refs[0].0, self.refs[0].1);
        (!d.is_nan()).then_some(d)
    }
}

fn slack(d: f64) -> f64 {
    d + d.abs() * REL_EPS + ABS_EPS_M
}

/// Totally ordered distance for the nearest-`n` heap.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Dist(f64);
impl Eq for Dist {}
impl PartialOrd for Dist {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Dist {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// Split a field path `loc.lat` into `("loc", "lat")`.
fn split_leaf(path: &str) -> Option<(&str, &str)> {
    path.rsplit_once('.')
        .filter(|(p, l)| !p.is_empty() && !l.is_empty())
}

/// Top-level AND conjuncts of a FILTER expression.
fn conjuncts<'e>(expr: &'e Expression, out: &mut Vec<&'e Expression>) {
    if let Expression::BinaryOp {
        left,
        op: BinaryOperator::And,
        right,
    } = expr
    {
        conjuncts(left, out);
        conjuncts(right, out);
    } else {
        out.push(expr);
    }
}

impl<'a> QueryExecutor<'a> {
    /// Recognise `DISTANCE(...)` / `GEO_DISTANCE(...)` with one side on
    /// `var_name` and the other constant against `ctx`.
    pub(super) fn parse_geo_distance_call(
        &self,
        expr: &Expression,
        var_name: &str,
        ctx: &Context,
    ) -> Option<GeoDistanceCall> {
        let Expression::FunctionCall { name, args } = expr else {
            return None;
        };
        let num = |e: &Expression| -> Option<f64> {
            self.extract_indexable_value(e, var_name, ctx)?.as_f64()
        };
        if name.eq_ignore_ascii_case("DISTANCE") && args.len() == 4 {
            for (d, c) in [(0usize, 2usize), (2, 0)] {
                let (Some(lat_path), Some(lon_path)) = (
                    self.extract_field_path(&args[d], var_name),
                    self.extract_field_path(&args[d + 1], var_name),
                ) else {
                    continue;
                };
                let (Some((f1, lat_key)), Some((f2, lon_key))) =
                    (split_leaf(&lat_path), split_leaf(&lon_path))
                else {
                    continue;
                };
                if f1 != f2 {
                    continue;
                }
                let (Some(clat), Some(clon)) = (num(&args[c]), num(&args[c + 1])) else {
                    continue;
                };
                return Some(GeoDistanceCall {
                    field: f1.to_string(),
                    doc: DocSide::Keys {
                        lat_key: lat_key.to_string(),
                        lon_key: lon_key.to_string(),
                    },
                    refs: vec![(clat, clon)],
                });
            }
            return None;
        }
        if name.eq_ignore_ascii_case("GEO_DISTANCE") && args.len() == 2 {
            for (d, c) in [(0usize, 1usize), (1, 0)] {
                let Some(field) = self.extract_field_path(&args[d], var_name) else {
                    continue;
                };
                let Some(point) = self.extract_indexable_value(&args[c], var_name, ctx) else {
                    continue;
                };
                let refs = point_readings(&point)?;
                return Some(GeoDistanceCall {
                    field,
                    doc: DocSide::Point,
                    refs,
                });
            }
        }
        None
    }

    /// `DISTANCE(...) <= r` / `< r` / `r >= DISTANCE(...)` / `r > ...`.
    fn parse_geo_radius_conjunct(
        &self,
        expr: &Expression,
        var_name: &str,
        ctx: &Context,
    ) -> Option<(GeoDistanceCall, f64)> {
        let Expression::BinaryOp { left, op, right } = expr else {
            return None;
        };
        let (call_side, radius_side) = match op {
            BinaryOperator::LessThan | BinaryOperator::LessThanOrEqual => (left, right),
            BinaryOperator::GreaterThan | BinaryOperator::GreaterThanOrEqual => (right, left),
            _ => return None,
        };
        let call = self.parse_geo_distance_call(call_side, var_name, ctx)?;
        let radius = self
            .extract_indexable_value(radius_side, var_name, ctx)?
            .as_f64()?;
        Some((call, radius))
    }

    /// True when `expr` reads `var_name` only through paths at or under
    /// `field` (and reads it at least once). Such a conjunct can be evaluated
    /// on a document missing from the geo index.
    fn depends_only_on_field(&self, expr: &Expression, var_name: &str, field: &str) -> bool {
        fn walk(
            this: &QueryExecutor<'_>,
            e: &Expression,
            var_name: &str,
            field: &str,
            seen: &mut bool,
        ) -> bool {
            if let Some(path) = this.extract_field_path(e, var_name) {
                let ok = path == field || path.starts_with(&format!("{field}."));
                *seen |= ok;
                return ok;
            }
            // A bare reference, or a subquery (not descended into), could
            // read anything.
            if matches!(e, Expression::Variable(n) if n == var_name)
                || matches!(e, Expression::Subquery(_))
            {
                return false;
            }
            let mut ok = true;
            e.for_each_child(&mut |c| {
                if ok && !walk(this, c, var_name, field, seen) {
                    ok = false;
                }
            });
            ok
        }
        let mut seen = false;
        walk(self, expr, var_name, field, &mut seen) && seen
    }

    /// Serve a FILTER from a geo index. Returns the candidate documents (a
    /// superset of the matches of the geo conjunct), the geo index name and
    /// `"Geo"`. `only_index` restricts the rule to one geo index (index
    /// hints); `limit` stops the index read early (existence probes).
    pub(super) fn geo_lookup_for_filter(
        &self,
        collection: &Collection,
        filter: &Expression,
        var_name: &str,
        ctx: &Context,
        only_index: Option<&str>,
        limit: Option<usize>,
    ) -> Option<(Vec<Document>, String, String)> {
        let geo_indexes = collection.get_all_geo_indexes();
        if geo_indexes.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        conjuncts(filter, &mut parts);
        for part in parts {
            let Some((call, radius)) = self.parse_geo_radius_conjunct(part, var_name, ctx) else {
                continue;
            };
            let Some(index) = geo_indexes.iter().find(|g| g.field == call.field) else {
                continue;
            };
            if only_index.is_some_and(|n| n != index.name) {
                continue;
            }
            // A document missing from the index reads the geo field as null,
            // like an empty document, and `DISTANCE(null, …) <= r` is true
            // (null sorts below numbers). The index can serve the FILTER only
            // if the conjuncts that depend on nothing but the geo field reject
            // such a document — e.g. `IS_NUMBER(p.loc.lat) AND DISTANCE(…) <= r`.
            let mut probe = ctx.clone();
            probe.insert(var_name.to_string(), Value::Object(serde_json::Map::new()));
            let mut guard_parts = Vec::new();
            conjuncts(filter, &mut guard_parts);
            let admits_missing = guard_parts
                .into_iter()
                .filter(|c| self.depends_only_on_field(c, var_name, &call.field))
                .all(|c| {
                    self.evaluate_filter_with_context(c, &probe)
                        .unwrap_or(false)
                });
            if admits_missing {
                continue;
            }

            let bound = slack(radius);
            let cap = limit.unwrap_or(self.max_intermediate_rows().saturating_add(1));
            let mut keys: Vec<String> = Vec::new();
            let name = collection.geo_index_scan(&call.field, |doc_key, v| {
                let keep = match call.min_distance(v) {
                    Some(d) => d <= bound,
                    None => true, // unreadable: let the FILTER decide
                };
                if keep {
                    keys.push(doc_key.to_string());
                }
                keys.len() < cap
            })?;
            let docs: Vec<Document> = keys.iter().filter_map(|k| collection.get(k).ok()).collect();
            return Some((docs, name, "Geo".to_string()));
        }
        None
    }

    /// The candidate documents for `FOR … SORT DISTANCE(…) ASC LIMIT o, n`
    /// when a geo index can serve it, plus the geo index name. `None` when the
    /// query does not have that shape or the index cannot guarantee the
    /// result (a document without an unambiguous indexed point).
    pub(super) fn geo_sort_candidates(
        &self,
        query: &Query,
        initial_bindings: &Context,
    ) -> Option<(Vec<Document>, String)> {
        let sort = query.sort_clause.as_ref()?;
        let limit = query.limit_clause.as_ref()?;
        if sort.fields.len() != 1 || !sort.fields[0].1 {
            return None;
        }
        let body = query.body_clauses.as_slice();
        let Some(BodyClause::For(for_clause)) = body.first() else {
            return None;
        };
        let lets_only = body.iter().skip(1).all(|c| match c {
            BodyClause::Let(l) => l.variable != for_clause.variable,
            _ => false,
        });
        if !lets_only
            || for_clause.source_expression.is_some()
            || for_clause.system_time.is_some()
            || for_clause.valid_time.is_some()
            || for_clause
                .source_variable
                .as_ref()
                .is_some_and(|s| s != &for_clause.collection)
            || initial_bindings.contains_key(&for_clause.collection)
            || self.row_policy_applies(&for_clause.collection)
            || query
                .return_clause
                .as_ref()
                .is_some_and(|rc| contains_window_functions(&rc.expression))
        {
            return None;
        }
        let call = self.parse_geo_distance_call(
            &sort.fields[0].0,
            &for_clause.variable,
            initial_bindings,
        )?;
        if call.refs.len() != 1 {
            return None;
        }
        let (offset, count) = self.eval_limit(limit, initial_bindings);
        let k = offset.checked_add(count?)?;
        if k == 0 {
            return None;
        }
        let collection = self.get_collection(&for_clause.collection).ok()?;
        if collection
            .get_shard_config()
            .is_some_and(|c| c.num_shards > 0)
        {
            return None;
        }

        // Pass 1: the k smallest distances, and proof that every entry has
        // exactly one reading.
        let mut heap: BinaryHeap<Dist> = BinaryHeap::with_capacity(k.min(1 << 16));
        let mut entries = 0usize;
        let mut all_exact = true;
        let name = collection.geo_index_scan(&call.field, |_, v| {
            entries += 1;
            let Some(d) = call.exact_distance(v) else {
                all_exact = false;
                return false;
            };
            if heap.len() < k {
                heap.push(Dist(d));
            } else if heap.peek().is_some_and(|w| d < w.0) {
                heap.pop();
                heap.push(Dist(d));
            }
            true
        })?;
        // Every document must be in the index: one that is not has a null
        // distance, which sorts first.
        if !all_exact || entries != collection.count() {
            return None;
        }
        let threshold = if heap.len() < k {
            f64::INFINITY
        } else {
            slack(heap.peek()?.0)
        };

        // Pass 2: everything within the threshold, in key order.
        let cap = self.max_intermediate_rows().saturating_add(1);
        let mut keys = Vec::new();
        collection.geo_index_scan(&call.field, |doc_key, v| {
            if call.exact_distance(v).is_some_and(|d| d <= threshold) {
                keys.push(doc_key.to_string());
            }
            keys.len() < cap
        })?;
        let docs: Vec<Document> = keys.iter().filter_map(|k| collection.get(k).ok()).collect();
        Some((docs, name))
    }

    /// Execute `FOR … [LET …]* SORT DISTANCE(…) LIMIT … RETURN …` from the geo
    /// index. `Ok(None)` when the rule does not apply; the caller then runs
    /// the normal pipeline. `RETURN DISTINCT` is applied by the caller.
    pub(crate) fn try_geo_sort_limit(
        &self,
        query: &Query,
        initial_bindings: &Context,
    ) -> DbResult<Option<QueryExecutionResult>> {
        let Some((docs, _index)) = self.geo_sort_candidates(query, initial_bindings) else {
            return Ok(None);
        };
        self.check_budget(docs.len())?;
        let (Some(sort), Some(limit), Some(BodyClause::For(for_clause))) = (
            query.sort_clause.as_ref(),
            query.limit_clause.as_ref(),
            query.body_clauses.first(),
        ) else {
            return Ok(None);
        };

        let rows: Vec<Context> = docs
            .into_iter()
            .map(|doc| {
                let mut ctx = initial_bindings.clone();
                ctx.insert(for_clause.variable.clone(), doc.into_value());
                ctx
            })
            .collect();
        let mut rows = self.sort_rows(rows, &sort.fields);
        let (offset, count) = self.eval_limit(limit, initial_bindings);
        let start = offset.min(rows.len());
        rows.drain(0..start);
        if let Some(count) = count {
            rows.truncate(count);
        }

        // Body LETs run on the surviving rows only (as on the index-sorted
        // path): the SORT key does not depend on them.
        for clause in query.body_clauses.iter().skip(1) {
            if let BodyClause::Let(let_clause) = clause {
                for ctx in &mut rows {
                    let v = self.evaluate_expr_with_context(&let_clause.expression, ctx)?;
                    ctx.insert(let_clause.variable.clone(), v);
                }
            }
        }
        for let_clause in &query.post_limit_lets {
            for ctx in &mut rows {
                let v = self.evaluate_expr_with_context(&let_clause.expression, ctx)?;
                ctx.insert(let_clause.variable.clone(), v);
            }
        }

        let results = match &query.return_clause {
            Some(rc) => rows
                .iter()
                .map(|ctx| self.evaluate_expr_with_context(&rc.expression, ctx))
                .collect::<DbResult<Vec<_>>>()?,
            None => Vec::new(),
        };
        Ok(Some(QueryExecutionResult {
            results,
            mutations: MutationStats::new(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn point_readings_cover_objects_geojson_and_ambiguous_arrays() {
        assert_eq!(
            point_readings(&json!({"lat": 1.0, "lon": 2.0})),
            Some(vec![(1.0, 2.0)])
        );
        assert_eq!(
            point_readings(&json!({"type": "Point", "coordinates": [2.0, 1.0]})),
            Some(vec![(1.0, 2.0)])
        );
        assert_eq!(
            point_readings(&json!([1.0, 2.0])),
            Some(vec![(1.0, 2.0), (2.0, 1.0)])
        );
        assert_eq!(point_readings(&json!("x")), None);
    }

    #[test]
    fn slack_only_grows() {
        assert!(slack(100.0) > 100.0);
        assert!(slack(0.0) > 0.0);
    }

    #[test]
    fn split_leaf_requires_parent() {
        assert_eq!(split_leaf("loc.lat"), Some(("loc", "lat")));
        assert_eq!(split_leaf("a.b.lat"), Some(("a.b", "lat")));
        assert_eq!(split_leaf("lat"), None);
    }
}
