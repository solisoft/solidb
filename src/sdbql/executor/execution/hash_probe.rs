//! A correlated FILTER no index serves, answered from a per-query hash.
//!
//! `FOR card IN cards LET ms = (FOR m IN comments FILTER m.card_id == card._key
//! RETURN m)` with no index on `card_id` read all 2,022 comments from storage
//! once per card: 1.86 s on the board's 1,079 cards, where one scan takes
//! about a millisecond. When the same FOR clause would scan its collection
//! again in this query (a second row of its stage, or a second run of the
//! subquery it sits in), the collection is read once, its documents are
//! bucketed by the hash of the filtered field, and each row only re-checks
//! the documents in its key's bucket.
//!
//! The probe replaces the scan only where it returns what the scan returns,
//! order included:
//! - Candidates are a superset of the row's matches: buckets are keyed by
//!   [`hash_value`], which agrees with `==` (`1` and `1.0`, `-0.0` and `0`
//!   hash alike, a missing field hashes as `null`), and every leading FILTER
//!   is re-checked on each candidate, as the index path does.
//! - Each row's candidates are visited in scan order, and the rows in order.
//! - The documents bound are the scan's own: the FOR's recorded scan
//!   projection, or whole documents, read by the same storage call.
//! - The field is read segment by segment as the evaluator reads a field
//!   access chain (`v.a.b` is `a` then `b`, `v["a.b"]` is one segment, looked
//!   up literally first). A chain is never joined into a dotted string, which
//!   would make those two the same field.
//! - The key must not read the loop variable, and must be deterministic: it
//!   is evaluated once per row to choose candidates, then again per candidate
//!   by the re-check. With `FLOOR(RAND() * 4)` a document would survive only
//!   when two independent draws both matched it, 1/16 where the scan gives
//!   1/4.
//!
//! Several conjuncts can be keys (`m.commentable_type == "card" AND
//! m.commentable_id == card._key`, the board query's shape). Each row probes
//! the one with the fewest candidates: a constant discriminator first in the
//! FILTER would otherwise hand every row ~1,600 of the 2,022 comments.
//!
//! What the probe holds is bounded with the rest of the query: every cached
//! document vector counts against `max_intermediate_rows`, and a build that
//! would not fit is declined (the FOR scans as before) rather than failing
//! the query. A FOR whose stage has one row triggers only from its second
//! run, so a subquery that runs once keeps its one projected scan.
//!
//! The probe evaluates keys and FILTERs, which can run subqueries whose own
//! FORs reach this code again on the same executor. `parking_lot::Mutex` is
//! not reentrant, so no `ExecCaches` guard is ever held across an
//! evaluation: each access clones what it needs out and unlocks, and a built
//! table is inserted before it is probed, so a nested FOR reuses it.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;

use super::super::index_opt::push_conjuncts;
use super::super::subquery_memo::is_nondeterministic;
use super::super::types::Context;
use super::super::{get_field_ref, hash_value, QueryExecutor};
use super::BUDGET_CHECK_INTERVAL;
use crate::error::DbResult;
use crate::sdbql::ast::{expression_any, BinaryOperator, Expression, ForClause};

/// A collection and the scan projection its documents were read with
/// (`None`: whole documents).
type VectorKey = (String, Option<Arc<[String]>>);

/// Documents in scan order, shared by every row bound to one of them.
type DocVector = Arc<[Arc<Value>]>;

/// What the hash probe has read and built during one query.
#[derive(Default)]
pub(in crate::sdbql::executor) struct ProbeCache {
    /// Per FOR clause (by address): its collection and key fields, compared
    /// on lookup like a scan projection, and how often it scanned without
    /// the probe.
    scans: HashMap<usize, (String, Vec<Vec<String>>, u32)>,
    vectors: HashMap<VectorKey, DocVector>,
    /// Vectors that did not fit the row ceiling: not tried again.
    declined: HashSet<VectorKey>,
    /// Buckets per vector and field path, indexing into that vector.
    buckets: HashMap<(VectorKey, Vec<String>), Arc<Buckets>>,
    /// Documents held across every cached vector.
    held: usize,
}

impl ProbeCache {
    /// A cached vector `projection` can be served from: the same fields,
    /// whole documents, or a superset (the projection analysis proves the
    /// extra fields are never read). Never a projected vector for a FOR that
    /// reads whole documents.
    fn usable_vector(
        &self,
        name: &str,
        projection: Option<&Arc<[String]>>,
    ) -> Option<(VectorKey, DocVector)> {
        let mut found: Option<(&VectorKey, &DocVector)> = None;
        for (key, docs) in &self.vectors {
            if key.0 != name {
                continue;
            }
            let usable = match (&key.1, projection) {
                (None, _) => true,
                (Some(have), Some(want)) => want.iter().all(|f| have.contains(f)),
                (Some(_), None) => false,
            };
            if !usable {
                continue;
            }
            if key.1.as_ref() == projection {
                found = Some((key, docs));
                break;
            }
            if found.is_none() || key.1.is_none() {
                found = Some((key, docs));
            }
        }
        found.map(|(key, docs)| (key.clone(), docs.clone()))
    }
}

/// Indices into a [`DocVector`], ascending, by the hash of one field.
pub(in crate::sdbql::executor) struct Buckets(HashMap<u64, Vec<u32>, foldhash::fast::RandomState>);

impl Buckets {
    fn build(docs: &[Arc<Value>], segments: &[String]) -> Self {
        let mut map: HashMap<u64, Vec<u32>, _> = HashMap::default();
        for (i, doc) in docs.iter().enumerate() {
            map.entry(hash_value(field_of(doc, segments)))
                .or_default()
                .push(i as u32);
        }
        Self(map)
    }

    fn bucket(&self, value: &Value) -> Option<&[u32]> {
        self.0.get(&hash_value(value)).map(Vec::as_slice)
    }

    /// The buckets that hold every document `field == key` (or `field IN
    /// key`) can accept.
    fn lists(&self, key: &Value, in_list: bool) -> Vec<&[u32]> {
        if !in_list {
            return self.bucket(key).into_iter().collect();
        }
        match key {
            Value::Array(items) => items.iter().filter_map(|v| self.bucket(v)).collect(),
            // `x IN {…}` holds when x is a string naming one of its keys.
            Value::Object(map) => map
                .keys()
                .filter_map(|k| self.bucket(&Value::String(k.clone())))
                .collect(),
            // Never true.
            _ => Vec::new(),
        }
    }
}

/// `doc` read through `segments` as `resolve_ref` reads a field access
/// chain: a missing segment, and anything below it, is `null`.
fn field_of<'v>(doc: &'v Value, segments: &[String]) -> &'v Value {
    static NULL: Value = Value::Null;
    segments
        .iter()
        .fold(doc, |cur, seg| get_field_ref(cur, seg).unwrap_or(&NULL))
}

/// `v.a.b` as `["a", "b"]`, `v["a.b"]` as `["a.b"]`: a chain of plain field
/// accesses on `var`, segments as written.
fn field_segments(expr: &Expression, var: &str) -> Option<Vec<String>> {
    let mut segments = Vec::new();
    let mut cur = expr;
    loop {
        match cur {
            Expression::FieldAccess(base, field) => {
                segments.push(field.clone());
                cur = base;
            }
            Expression::Variable(name) if name == var && !segments.is_empty() => break,
            _ => return None,
        }
    }
    segments.reverse();
    Some(segments)
}

/// A FILTER conjunct the probe can key on: `v.f == key`, `key == v.f` or
/// `v.f IN key`.
struct ProbeTerm<'e> {
    segments: Vec<String>,
    in_list: bool,
    key: &'e Expression,
}

/// A row's key: borrowed from the row or the AST where it can be, a
/// memoised subquery's rows shared.
enum ProbeKey<'v> {
    Ref(&'v Value),
    Owned(Value),
    Shared(Arc<Value>),
}

impl ProbeKey<'_> {
    fn get(&self) -> &Value {
        match self {
            ProbeKey::Ref(v) => v,
            ProbeKey::Owned(v) => v,
            ProbeKey::Shared(v) => v,
        }
    }
}

impl QueryExecutor<'_> {
    /// The rows of a FOR stage and its leading FILTERs, from the hash probe,
    /// or `None` to leave the stage to the scan. Tried after both index
    /// passes have declined.
    pub(in crate::sdbql::executor) fn try_hash_probe(
        &self,
        for_clause: &ForClause,
        filters: &[&Expression],
        rows: &[Context],
        scan_limit: Option<usize>,
    ) -> DbResult<Option<Vec<Context>>> {
        let name = &for_clause.collection;
        let var = for_clause.variable.as_str();
        // Writing queries read per row. A pushed-down scan limit applies
        // before the FILTER. OPTIONS hint the index path, which declined.
        // A `` `@x` `` loop variable is what a missing bind var `@x` reads.
        if !self.caches.reuse_enabled()
            || filters.is_empty()
            || rows.is_empty()
            || scan_limit.is_some()
            || name.is_empty()
            || var.starts_with('@')
            || for_clause.source_expression.is_some()
            || for_clause
                .source_variable
                .as_ref()
                .is_some_and(|s| s != name)
            || for_clause.system_time.is_some()
            || for_clause.valid_time.is_some()
            || for_clause.options.is_some()
        {
            return Ok(None);
        }
        let terms = self.probe_terms(filters, var);
        if terms.is_empty() {
            return Ok(None);
        }
        // A LET-bound array of that name is iterated, not the collection; a
        // row policy, a columnar layout or shards all change what the scan
        // reads. A name `get_collection` resolves is never a search view.
        if rows.iter().any(|r| r.contains_key(name)) || self.row_policy_applies(name) {
            return Ok(None);
        }
        if let Some(database) = self
            .database
            .as_deref()
            .and_then(|db| self.storage.get_database(db).ok())
        {
            if self.is_columnar(&database, name) {
                return Ok(None);
            }
        }
        let Ok(collection) = self.get_collection(name) else {
            return Ok(None);
        };
        if collection
            .get_shard_config()
            .is_some_and(|c| c.num_shards > 0)
        {
            return Ok(None);
        }

        let projection = self.scan_projection_for(for_clause);
        let paths: Vec<Vec<String>> = terms.iter().map(|t| t.segments.clone()).collect();
        let addr = for_clause as *const ForClause as usize;
        let (cached, held) = {
            let mut cache = self.caches.hash_probe.lock();
            let cached = cache.usable_vector(name, projection.as_ref());
            if cached.is_none() {
                let scanned_before = cache
                    .scans
                    .get(&addr)
                    .is_some_and(|(c, p, n)| c == name && *p == paths && *n > 0);
                // One row, first run: scan, and count it, so the next run of
                // this FOR (and only this one) builds the table.
                if rows.len() < 2 && !scanned_before {
                    let entry = cache
                        .scans
                        .entry(addr)
                        .or_insert_with(|| (name.clone(), Vec::new(), 0));
                    if entry.0 != *name || entry.1 != paths {
                        *entry = (name.clone(), paths, 0);
                    }
                    entry.2 += 1;
                    return Ok(None);
                }
                if cache.declined.contains(&(name.clone(), projection.clone())) {
                    return Ok(None);
                }
            }
            (cached, cache.held)
        };

        let (vkey, docs) = match cached {
            Some(found) => found,
            None => {
                let key = (name.clone(), projection.clone());
                // Bucket indices are u32.
                let room = self
                    .max_intermediate_rows
                    .saturating_sub(held)
                    .min(u32::MAX as usize);
                if collection.count() > room {
                    self.caches.hash_probe.lock().declined.insert(key);
                    return Ok(None);
                }
                self.caches.count_collection_scan();
                let cap = Some(room.saturating_add(1));
                let docs = match &projection {
                    Some(fields) => collection.scan_values_projected(cap, fields),
                    None => collection.scan_values(cap),
                };
                // The count was stale: drop the scan rather than exceed.
                if docs.len() > room {
                    self.caches.hash_probe.lock().declined.insert(key);
                    return Ok(None);
                }
                let docs: DocVector = docs.into_iter().map(Arc::new).collect();
                let mut cache = self.caches.hash_probe.lock();
                let docs = match cache.vectors.get(&key) {
                    Some(existing) => existing.clone(),
                    None => {
                        cache.held += docs.len();
                        cache.vectors.insert(key.clone(), docs.clone());
                        docs
                    }
                };
                let held = cache.held;
                drop(cache);
                self.check_budget(held)?;
                (key, docs)
            }
        };
        if docs.is_empty() {
            return Ok(Some(Vec::new()));
        }

        let mut buckets = Vec::with_capacity(terms.len());
        for term in &terms {
            let bkey = (vkey.clone(), term.segments.clone());
            let found = self.caches.hash_probe.lock().buckets.get(&bkey).cloned();
            let b = match found {
                Some(b) => b,
                None => {
                    let built = Arc::new(Buckets::build(&docs, &term.segments));
                    let mut cache = self.caches.hash_probe.lock();
                    cache.buckets.entry(bkey).or_insert(built).clone()
                }
            };
            buckets.push(b);
        }

        let mut out = Vec::new();
        // Candidates re-checked since the last budget check.
        let mut checked = 0;
        for ctx in rows {
            let Some(candidates) = self.probe_row(&terms, &buckets, ctx) else {
                continue;
            };
            for &idx in candidates.iter() {
                let mut new_ctx = ctx.clone();
                new_ctx.insert_shared(for_clause.variable.clone(), docs[idx as usize].clone());
                checked += 1;
                if filters.iter().all(|f| {
                    self.evaluate_filter_with_context(f, &new_ctx)
                        .unwrap_or(false)
                }) {
                    out.push(new_ctx);
                }
                if checked == BUDGET_CHECK_INTERVAL {
                    self.caches.count_probe_candidates(checked as u64);
                    checked = 0;
                    self.check_budget(out.len())?;
                }
            }
        }
        self.caches.count_probe_candidates(checked as u64);
        Ok(Some(out))
    }

    /// Every conjunct of the leading FILTERs the probe can key on, in order.
    fn probe_terms<'e>(&self, filters: &[&'e Expression], var: &str) -> Vec<ProbeTerm<'e>> {
        let mut parts = Vec::new();
        for filter in filters {
            push_conjuncts(filter, &mut parts);
        }
        parts
            .into_iter()
            .filter_map(|part| self.probe_term(part, var))
            .collect()
    }

    fn probe_term<'e>(&self, part: &'e Expression, var: &str) -> Option<ProbeTerm<'e>> {
        let Expression::BinaryOp { left, op, right } = part else {
            return None;
        };
        let (segments, key, in_list) = match op {
            BinaryOperator::Equal => match field_segments(left, var) {
                Some(segments) => (segments, right.as_ref(), false),
                None => (field_segments(right, var)?, left.as_ref(), false),
            },
            BinaryOperator::In => (field_segments(left, var)?, right.as_ref(), true),
            _ => return None,
        };
        if expression_any(key, &mut is_nondeterministic) || self.value_reads_var(key, var) {
            return None;
        }
        Some(ProbeTerm {
            segments,
            in_list,
            key,
        })
    }

    /// The candidates for one row, ascending, from the term with the fewest
    /// (ties to the earlier term). `None` when some term rules out every
    /// document: its key failed to evaluate (so the FILTER fails for every
    /// document on the scan too), or matches nothing.
    fn probe_row<'b>(
        &self,
        terms: &[ProbeTerm<'_>],
        buckets: &'b [Arc<Buckets>],
        ctx: &Context,
    ) -> Option<Cow<'b, [u32]>> {
        let mut best: Option<(usize, Vec<&'b [u32]>)> = None;
        for (term, b) in terms.iter().zip(buckets) {
            let key = self.probe_key(term.key, ctx)?;
            let lists = b.lists(key.get(), term.in_list);
            let total: usize = lists.iter().map(|l| l.len()).sum();
            if total == 0 {
                return None;
            }
            if best.as_ref().is_none_or(|(n, _)| total < *n) {
                best = Some((total, lists));
            }
        }
        let (_, lists) = best?;
        if let [one] = lists[..] {
            return Some(Cow::Borrowed(one));
        }
        let mut all = lists.concat();
        all.sort_unstable();
        all.dedup();
        Some(Cow::Owned(all))
    }

    /// `key` in `ctx`, `None` on an error. A bare subquery's rows come from
    /// the memo, shared; on a miss it runs here, once.
    fn probe_key<'v>(&'v self, key: &'v Expression, ctx: &'v Context) -> Option<ProbeKey<'v>> {
        if let Expression::Subquery(q) = key {
            return match self.memoized_subquery(q, ctx) {
                Ok(Some(rows)) => Some(ProbeKey::Shared(rows)),
                Ok(None) => self
                    .execute_with_parent_context(q, ctx)
                    .ok()
                    .map(|rows| ProbeKey::Owned(Value::Array(rows))),
                Err(_) => None,
            };
        }
        if let Some(v) = self.resolve_ref(key, ctx) {
            return Some(ProbeKey::Ref(v));
        }
        self.evaluate_expr_with_context(key, ctx)
            .ok()
            .map(ProbeKey::Owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn segs(path: &[&str]) -> Vec<String> {
        path.iter().map(|s| s.to_string()).collect()
    }

    /// A field is read per segment: a dotted segment literally first, then
    /// split; a missing segment, and what is below it, is null.
    #[test]
    fn fields_are_read_segment_by_segment() {
        let doc = json!({"a.b": 1, "a": {"b": 2}, "c": {"b.d": 4, "b": {"d": 5}}});
        assert_eq!(field_of(&doc, &segs(&["a", "b"])), &json!(2));
        assert_eq!(field_of(&doc, &segs(&["a.b"])), &json!(1));
        assert_eq!(field_of(&doc, &segs(&["c", "b.d"])), &json!(4));
        assert_eq!(
            field_of(&json!({"a": {"b": 3}}), &segs(&["a.b"])),
            &json!(3)
        );
        assert_eq!(field_of(&doc, &segs(&["x", "y"])), &Value::Null);
    }

    /// Values `==` equates share a bucket; `IN` reads the buckets of an
    /// array's elements or an object's keys.
    #[test]
    fn buckets_agree_with_equality() {
        let docs: Vec<Arc<Value>> = [
            json!({"k": 1}),
            json!({"k": 1.0}),
            json!({"k": -0.0}),
            json!({"k": 0}),
            json!({"k": null}),
            json!({}),
            json!({"k": "x"}),
        ]
        .into_iter()
        .map(Arc::new)
        .collect();
        let b = Buckets::build(&docs, &segs(&["k"]));
        assert_eq!(b.lists(&json!(1), false), vec![&[0u32, 1][..]]);
        assert_eq!(b.lists(&json!(0.0), false), vec![&[2u32, 3][..]]);
        assert_eq!(b.lists(&Value::Null, false), vec![&[4u32, 5][..]]);
        assert_eq!(
            b.lists(&json!([1, "x"]), true),
            vec![&[0u32, 1][..], &[6][..]]
        );
        assert_eq!(b.lists(&json!({"x": 0}), true), vec![&[6u32][..]]);
        assert!(b.lists(&json!("x"), true).is_empty());
    }
}
