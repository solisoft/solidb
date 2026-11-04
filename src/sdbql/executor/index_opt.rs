//! Index optimization for SDBQL executor.
//!
//! This module contains index-related optimizations:
//! - extract_indexable_condition: Extract conditions that can use indexes
//! - extract_field_path: Extract field path from expression
//! - use_index_for_condition: Try to use index for condition lookup
//!
//! Beyond `==` and ranges, a FILTER conjunct can be served by an index when it
//! is `field IN <array>` (one equality lookup per distinct key), or
//! `field LIKE "abc%"` / `STARTS_WITH(field, "abc")` (a range scan over
//! `[abc, abd)` in the order-preserving index key space). Every index read
//! returns candidates that the caller re-checks against the full FILTER, so a
//! condition only needs to select a superset of its matches.
//!
//! `FOR … OPTIONS { indexHint, forceIndexHint }` is honoured through
//! [`IndexHint`] and [`QueryExecutor::lookup_index_for_filter_hinted`].
//!
//! A stage those leave to the scan gets a second, narrower try:
//! [`QueryExecutor::try_subquery_index_stage`] serves `==` / `IN` whose value
//! comes from a reusable subquery, only where the result equals the scan's.

use std::borrow::Cow;
use std::collections::HashSet;

use crate::storage::keyspace::KsDbExt;
use crate::storage::kv::{Direction, IterMode};
use serde_json::Value;

use super::types::{Context, IndexableCondition};
use super::QueryExecutor;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::*;
use crate::storage::index::{IndexSpec, IndexType};
use crate::storage::Collection;

pub(super) const AUTO_INDEX_CAP: usize = 16;

/// Largest `IN` list turned into per-key index lookups; a longer list is
/// cheaper as a scan with the (hashed) `IN` set.
const MAX_INDEXED_IN_KEYS: usize = 10_000;

/// Index hint from `FOR … OPTIONS { indexHint: …, forceIndexHint: … }`.
///
/// `names` are tried in order; the first that can serve a FILTER conjunct is
/// used. With `force`, a FILTER that none of them can serve is an error (as in
/// AQL) instead of falling back to the optimizer's own choice.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IndexHint {
    pub names: Vec<String>,
    pub force: bool,
}

impl IndexHint {
    /// Build from the parsed `FOR` options; `None` when no hint was given.
    pub fn from_options(index_hint: Option<&[String]>, force: bool) -> Option<Self> {
        let names: Vec<String> = index_hint.unwrap_or_default().to_vec();
        if names.is_empty() {
            return None;
        }
        Some(Self { names, force })
    }
}

/// The literal prefix of a LIKE pattern: the characters before the first
/// wildcard (`%`, `_`) or escape (`\`). `None` when that prefix is empty.
pub(super) fn like_literal_prefix(pattern: &str) -> Option<String> {
    let prefix: String = pattern
        .chars()
        .take_while(|c| !matches!(c, '%' | '_' | '\\'))
        .collect();
    (!prefix.is_empty()).then_some(prefix)
}

/// The smallest string greater than every string starting with `prefix`:
/// the prefix with its last incrementable character bumped (`abc` → `abd`).
/// `None` when every character is `char::MAX`.
pub(super) fn prefix_successor(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(c) = chars.pop() {
        let mut next = c as u32 + 1;
        while next <= char::MAX as u32 {
            if let Some(n) = char::from_u32(next) {
                chars.push(n);
                return Some(chars.into_iter().collect());
            }
            next += 1; // skip the surrogate gap
        }
    }
    None
}

/// An `IN` right-hand side that per-key lookups can serve: an array of
/// bounded length. A `null` in it is served only by an index that holds
/// nulls (a non-sparse one); `index_lookup_in` checks that.
fn in_list_is_indexable(value: &Value) -> bool {
    matches!(value, Value::Array(items) if items.len() <= MAX_INDEXED_IN_KEYS)
}

fn is_range_op(op: &BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::LessThan
            | BinaryOperator::LessThanOrEqual
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterThanOrEqual
    )
}

/// One AND'd conjunct as the composite planner reads it.
enum CompositeTerm {
    Eq(Value),
    /// `IN` list; unlike the single-field path it may hold `null`, which a
    /// composite index stores like any other value.
    In(Vec<Value>),
    Range(BinaryOperator, Value),
    /// `!=` (one value) or `NOT IN` (several).
    Not(Vec<Value>),
}

/// A bound of a key interval: `(value, inclusive)`, `None` when open.
type IntervalBound = Option<(Value, bool)>;

/// A read of a multi-field index planned by [`composite_plan`].
struct CompositePlan {
    index: crate::storage::index::Index,
    /// The values each leading field may take, distinct by index encoding.
    eq_lists: Vec<Vec<Value>>,
    /// Disjoint, ordered intervals of the field after them, from its range
    /// and `!=` / `NOT IN` conditions. `None` when that field has none (one
    /// unbounded read); empty when they leave nothing (no read at all).
    next: Option<Vec<(IntervalBound, IntervalBound)>>,
}

impl CompositePlan {
    fn fields_used(&self) -> usize {
        self.eq_lists.len() + usize::from(self.next.is_some())
    }
}

fn complex(v: &Value) -> bool {
    matches!(v, Value::Array(_) | Value::Object(_))
}

/// `a` against `b` in index order (the key codec's byte order).
fn key_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    crate::storage::codec::encode_key(a).cmp(&crate::storage::codec::encode_key(b))
}

/// Does `v` lie within `(lo, hi)`?
fn within(v: &Value, lo: &IntervalBound, hi: &IntervalBound) -> bool {
    use std::cmp::Ordering::*;
    let above = match lo {
        None => true,
        Some((l, inc)) => match key_cmp(v, l) {
            Greater => true,
            Equal => *inc,
            Less => false,
        },
    };
    let below = match hi {
        None => true,
        Some((h, inc)) => match key_cmp(v, h) {
            Less => true,
            Equal => *inc,
            Greater => false,
        },
    };
    above && below
}

/// An interval no value lies in.
fn empty_interval(lo: &IntervalBound, hi: &IntervalBound) -> bool {
    match (lo, hi) {
        (Some((l, li)), Some((h, hi))) => match key_cmp(l, h) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Equal => !(*li && *hi),
            std::cmp::Ordering::Less => false,
        },
        _ => false,
    }
}

/// The intervals of the field after the fixed ones: the tightest range
/// bounds, split around every excluded value. `None` when the field has no
/// usable condition.
fn next_field_intervals(
    field: &str,
    terms: &[(String, CompositeTerm)],
) -> Option<Vec<(IntervalBound, IntervalBound)>> {
    use std::cmp::Ordering::*;
    let mut lower: IntervalBound = None;
    let mut upper: IntervalBound = None;
    let mut excluded: Vec<&Value> = Vec::new();
    let mut constrained = false;
    for (f, term) in terms {
        if f != field {
            continue;
        }
        match term {
            CompositeTerm::Range(op, v) if !complex(v) => {
                constrained = true;
                let inclusive = matches!(
                    op,
                    BinaryOperator::GreaterThanOrEqual | BinaryOperator::LessThanOrEqual
                );
                let is_lower = matches!(
                    op,
                    BinaryOperator::GreaterThan | BinaryOperator::GreaterThanOrEqual
                );
                // Keep the tighter of two bounds; on a tie, the exclusive one.
                let slot = if is_lower { &mut lower } else { &mut upper };
                let tighter = match slot {
                    None => true,
                    Some((cur, cur_inc)) => match (key_cmp(v, cur), is_lower) {
                        (Greater, true) | (Less, false) => true,
                        (Equal, _) => *cur_inc && !inclusive,
                        _ => false,
                    },
                };
                if tighter {
                    *slot = Some((v.clone(), inclusive));
                }
            }
            CompositeTerm::Not(values) => {
                for v in values.iter().filter(|v| !complex(v)) {
                    constrained = true;
                    excluded.push(v);
                }
            }
            _ => {}
        }
    }
    if !constrained {
        return None;
    }
    excluded.sort_by(|a, b| key_cmp(a, b));
    excluded.dedup_by(|a, b| key_cmp(a, b) == Equal);
    let mut intervals = vec![(lower, upper)];
    if empty_interval(&intervals[0].0, &intervals[0].1) {
        return Some(Vec::new());
    }
    for x in excluded {
        let mut split = Vec::with_capacity(intervals.len() + 1);
        for (lo, hi) in intervals {
            if within(x, &lo, &hi) {
                let left = (lo, Some((x.clone(), false)));
                let right = (Some((x.clone(), false)), hi);
                for part in [left, right] {
                    if !empty_interval(&part.0, &part.1) {
                        split.push(part);
                    }
                }
            } else {
                split.push((lo, hi));
            }
        }
        intervals = split;
    }
    Some(intervals)
}

/// Whether the AND'd `terms` reject every row whose `field` is null or
/// missing — some term on it is false for null. A sparse index has no entry
/// for such rows, so a read of it returns every match only then.
fn rejects_null(field: &str, terms: &[(String, CompositeTerm)]) -> bool {
    terms
        .iter()
        .filter(|(f, _)| f == field)
        .any(|(_, term)| match term {
            CompositeTerm::Eq(v) => !v.is_null(),
            CompositeTerm::In(items) => !items.iter().any(Value::is_null),
            CompositeTerm::Range(op, v) => !matches!(
                super::helpers::evaluate_binary_op(&Value::Null, op, v),
                Ok(Value::Bool(true))
            ),
            CompositeTerm::Not(values) => values.iter().any(Value::is_null),
        })
}

/// Plan a read of `index` from the AND'd `terms`: the longest run of leading
/// fields each fixed by an `==` or an `IN`, then the range and `!=` / `NOT
/// IN` conditions on the next field as key intervals.
///
/// `None` when the index is not a regular one, when the reads multiply out
/// past [`MAX_INDEXED_IN_KEYS`], or when the index cannot be trusted with the
/// condition. A sparse index leaves out rows whose index fields are all null,
/// so for one the plan must fix at least one field, every combination of the
/// fixed values must have a non-null value — every row it matches is then
/// indexed whatever its other fields hold, which is what lets `!=` and a
/// range that admits null be read — and the index must have several fields
/// (a single-field one is the single-field path's). A non-sparse index holds
/// every document, so none of that applies: `== null`, `IN [null]` and a
/// `!=` on its first field are read from it.
fn composite_plan(
    index: crate::storage::index::Index,
    terms: &[(String, CompositeTerm)],
) -> Option<CompositePlan> {
    if !matches!(index.index_type, IndexType::Persistent | IndexType::Hash)
        || (index.sparse && index.fields.len() < 2)
    {
        return None;
    }
    let mut eq_lists: Vec<Vec<Value>> = Vec::new();
    for field in &index.fields {
        let on_field = || terms.iter().filter(|(f, _)| f == field).map(|(_, t)| t);
        let list = if let Some(CompositeTerm::Eq(v)) =
            on_field().find(|t| matches!(t, CompositeTerm::Eq(v) if !complex(v)))
        {
            vec![v.clone()]
        } else if let Some(CompositeTerm::In(items)) =
            on_field().find(|t| matches!(t, CompositeTerm::In(items) if !items.iter().any(complex)))
        {
            let mut seen = HashSet::with_capacity(items.len());
            items
                .iter()
                .filter(|v| seen.insert(crate::storage::codec::encode_key(v)))
                .cloned()
                .collect()
        } else {
            break;
        };
        eq_lists.push(list);
    }
    if index.sparse
        && (eq_lists.is_empty() || eq_lists.iter().all(|l| l.iter().any(Value::is_null)))
    {
        return None;
    }
    let next = index
        .fields
        .get(eq_lists.len())
        .and_then(|field| next_field_intervals(field, terms));
    // Nothing fixed and nothing bounded would read the whole index.
    if eq_lists.is_empty() && next.is_none() {
        return None;
    }
    let reads = eq_lists
        .iter()
        .map(Vec::len)
        .chain(next.as_ref().map(Vec::len))
        .try_fold(1usize, |acc, n| acc.checked_mul(n.max(1)))?;
    if reads > MAX_INDEXED_IN_KEYS {
        return None;
    }
    Some(CompositePlan {
        index,
        eq_lists,
        next,
    })
}

/// Single-field index on `field` whose entries live in the `idx:` key space.
fn regular_index_on(collection: &Collection, field: &str) -> Option<crate::storage::index::Index> {
    collection.get_all_indexes().into_iter().find(|i| {
        i.fields.len() == 1
            && i.fields[0] == field
            && !matches!(i.index_type, IndexType::Fulltext | IndexType::Vector)
    })
}

/// Range scan `[prefix, successor(prefix))` over a single-field index: every
/// string value that starts with `prefix`. `None` when no such index exists.
fn index_prefix_scan(
    collection: &Collection,
    field: &str,
    prefix: &str,
    cap: usize,
) -> Option<Vec<crate::storage::Document>> {
    let index = regular_index_on(collection, field)?;
    let upper = prefix_successor(prefix)?;
    let encode = |s: &str| {
        hex::encode(crate::storage::codec::encode_key(&Value::String(
            s.to_string(),
        )))
    };
    let base = format!("{}{}:", crate::storage::collection::IDX_PREFIX, index.name);
    let lo = format!("{}{}", base, encode(prefix));
    let hi = format!("{}{}", base, encode(&upper));

    let db = &collection.db;
    let cf = collection.ks.handle(db)?;
    let mut doc_keys: Vec<Vec<u8>> = Vec::new();
    let iter = db.iterator_ks(&cf, IterMode::From(lo.as_bytes(), Direction::Forward));
    for (k, v) in iter.flatten() {
        if !k.starts_with(base.as_bytes()) || k.as_ref() >= hi.as_bytes() {
            break;
        }
        doc_keys.push(Collection::doc_key(&String::from_utf8_lossy(&v)));
        if doc_keys.len() >= cap {
            break;
        }
    }
    if doc_keys.is_empty() {
        return Some(Vec::new());
    }
    let docs = db
        .multi_get_ks(&cf, doc_keys.iter().map(|k| k.as_slice()))
        .into_iter()
        .filter_map(|r| r.ok())
        .flatten()
        .filter_map(|bytes| crate::storage::serializer::deserialize_doc(&bytes).ok())
        .collect();
    Some(docs)
}

/// Documents above which a collection is never auto-indexed: the backfill runs
/// inside the query that triggered it, so an unbounded one stalls that request
/// for as long as the scan takes. Override with `SOLIDB_AUTO_INDEX_MAX_DOCS`
/// (`0` disables the ceiling).
const AUTO_INDEX_MAX_DOCS_DEFAULT: usize = 1_000_000;

fn auto_index_max_docs() -> usize {
    match std::env::var("SOLIDB_AUTO_INDEX_MAX_DOCS") {
        Ok(v) => v.trim().parse().unwrap_or(AUTO_INDEX_MAX_DOCS_DEFAULT),
        Err(_) => AUTO_INDEX_MAX_DOCS_DEFAULT,
    }
}

fn auto_index_name(field: &str) -> String {
    format!("_auto_{field}")
}

/// An index this feature created (or would create): `_auto_{field}` over that
/// one field. A hand-made index that merely starts with `_auto_` is not one,
/// so it cannot silently consume a slot of [`AUTO_INDEX_CAP`].
fn is_auto_index(index: &crate::storage::index::Index) -> bool {
    index.fields.len() == 1 && index.name == auto_index_name(&index.fields[0])
}

pub(super) fn field_is_auto_indexable(field: &str) -> bool {
    if matches!(field, "_key" | "_id" | "_rev" | "") {
        return false;
    }
    let mut parts = field.split('.');
    parts.all(|p| {
        let mut chars = p.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            }
            _ => false,
        }
    }) && !field.contains("..")
        && !field.starts_with('.')
        && !field.ends_with('.')
}

/// The FILTERs that directly follow a FOR — `clauses[from..]` up to the first
/// clause that is not a FILTER. Consecutive FILTERs are one conjunction, so
/// the index planner considers all of them, not just the first.
pub(super) fn leading_filters(clauses: &[BodyClause], from: usize) -> Vec<&Expression> {
    clauses
        .get(from..)
        .unwrap_or_default()
        .iter()
        .map_while(|c| match c {
            BodyClause::Filter(f) => Some(&f.expression),
            _ => None,
        })
        .collect()
}

/// `a AND b AND …` over `filters`, borrowed when there is only one. Used to
/// *choose* an index; rows are still re-checked FILTER by FILTER.
pub(super) fn conjunction<'e>(filters: &[&'e Expression]) -> Option<Cow<'e, Expression>> {
    let (first, rest) = filters.split_first()?;
    if rest.is_empty() {
        return Some(Cow::Borrowed(*first));
    }
    Some(Cow::Owned(rest.iter().fold((*first).clone(), |acc, e| {
        Expression::BinaryOp {
            left: Box::new(acc),
            op: BinaryOperator::And,
            right: Box::new((*e).clone()),
        }
    })))
}

fn collection_bare_name(cf_name: &str) -> &str {
    cf_name.rsplit(':').next().unwrap_or(cf_name)
}

impl<'a> QueryExecutor<'a> {
    pub(super) fn extract_indexable_condition(
        &self,
        expr: &Expression,
        var_name: &str,
        ctx: &Context,
    ) -> Option<IndexableCondition> {
        // STARTS_WITH(var.field, "abc") -> prefix range
        if let Expression::FunctionCall { name, args } = expr {
            if name.eq_ignore_ascii_case("STARTS_WITH") && args.len() == 2 {
                let field = self.extract_field_path(&args[0], var_name)?;
                let prefix = match self.extract_indexable_value(&args[1], var_name, ctx)? {
                    Value::String(p) if !p.is_empty() => p,
                    _ => return None,
                };
                return Some(IndexableCondition {
                    field,
                    op: BinaryOperator::Like,
                    value: Value::String(prefix),
                });
            }
            return None;
        }
        if let Expression::BinaryOp { left, op, right } = expr {
            match op {
                BinaryOperator::In => {
                    let field = self.extract_field_path(left, var_name)?;
                    let value = self.extract_indexable_value(right, var_name, ctx)?;
                    if !in_list_is_indexable(&value) {
                        return None;
                    }
                    return Some(IndexableCondition {
                        field,
                        op: BinaryOperator::In,
                        value,
                    });
                }
                BinaryOperator::Like => {
                    // The condition carries the literal *prefix*, not the
                    // pattern: the index returns every value starting with it
                    // and the FILTER re-check applies the rest of the pattern.
                    let field = self.extract_field_path(left, var_name)?;
                    let prefix = match self.extract_indexable_value(right, var_name, ctx)? {
                        Value::String(p) => like_literal_prefix(&p)?,
                        _ => return None,
                    };
                    return Some(IndexableCondition {
                        field,
                        op: BinaryOperator::Like,
                        value: Value::String(prefix),
                    });
                }
                BinaryOperator::Equal
                | BinaryOperator::LessThan
                | BinaryOperator::LessThanOrEqual
                | BinaryOperator::GreaterThan
                | BinaryOperator::GreaterThanOrEqual => {
                    // Try left = field access, right = value-side expression
                    if let Some(field) = self.extract_field_path(left, var_name) {
                        if let Some(value) = self.extract_indexable_value(right, var_name, ctx) {
                            return Some(IndexableCondition {
                                field,
                                op: op.clone(),
                                value,
                            });
                        }
                    }
                    // Try right = field access, left = value-side expression
                    if let Some(field) = self.extract_field_path(right, var_name) {
                        if let Some(value) = self.extract_indexable_value(left, var_name, ctx) {
                            let reversed_op = match op {
                                BinaryOperator::LessThan => BinaryOperator::GreaterThan,
                                BinaryOperator::LessThanOrEqual => {
                                    BinaryOperator::GreaterThanOrEqual
                                }
                                BinaryOperator::GreaterThan => BinaryOperator::LessThan,
                                BinaryOperator::GreaterThanOrEqual => {
                                    BinaryOperator::LessThanOrEqual
                                }
                                other => other.clone(),
                            };
                            return Some(IndexableCondition {
                                field,
                                op: reversed_op,
                                value,
                            });
                        }
                    }
                }
                BinaryOperator::And => {
                    if let Some(cond) = self.extract_indexable_condition(left, var_name, ctx) {
                        return Some(cond);
                    }
                    return self.extract_indexable_condition(right, var_name, ctx);
                }
                _ => {}
            }
        }
        None
    }

    /// Collect every top-level equality condition on `var_name` from an AND
    /// chain. Used to pick a composite index when multiple AND'd `field == val`
    /// terms are present (e.g. `FILTER doc.city == 'Paris' AND doc.age == 10`).
    /// Non-equality terms are skipped — they can't extend a composite-equality
    /// lookup prefix.
    pub(super) fn extract_equality_conditions(
        &self,
        expr: &Expression,
        var_name: &str,
        ctx: &Context,
    ) -> Vec<IndexableCondition> {
        let mut out = Vec::new();
        self.collect_equality_conditions(expr, var_name, ctx, &mut out);
        out
    }

    /// Every indexable conjunct of an AND chain, in order.
    pub(super) fn extract_indexable_conditions(
        &self,
        expr: &Expression,
        var_name: &str,
        ctx: &Context,
    ) -> Vec<IndexableCondition> {
        fn walk(
            exec: &QueryExecutor<'_>,
            expr: &Expression,
            var_name: &str,
            ctx: &Context,
            out: &mut Vec<IndexableCondition>,
        ) {
            if let Expression::BinaryOp {
                left,
                op: BinaryOperator::And,
                right,
            } = expr
            {
                walk(exec, left, var_name, ctx, out);
                walk(exec, right, var_name, ctx, out);
            } else if let Some(c) = exec.extract_indexable_condition(expr, var_name, ctx) {
                out.push(c);
            }
        }
        let mut out = Vec::new();
        walk(self, expr, var_name, ctx, &mut out);
        out
    }

    /// Conditions to try against single-field indexes, best first: equality
    /// and `IN` from any conjunct, then prefix scans, then — only when it is
    /// the condition the planner always picked — a range.
    ///
    /// A range from a later conjunct is not tried. That restriction dates from
    /// when `index_range_scan` silently stopped an unlimited read at 1000 keys
    /// (the request in the E2 report); the read is now bounded by the row
    /// ceiling and fails loudly past it, so lifting the restriction is a
    /// planner choice, no longer a correctness one.
    fn ordered_index_candidates(
        &self,
        filter: &Expression,
        var_name: &str,
        ctx: &Context,
    ) -> Vec<IndexableCondition> {
        let all = self.extract_indexable_conditions(filter, var_name, ctx);
        let first = self.extract_indexable_condition(filter, var_name, ctx);
        let (mut out, rest): (Vec<IndexableCondition>, Vec<IndexableCondition>) = all
            .into_iter()
            .partition(|c| matches!(c.op, BinaryOperator::Equal | BinaryOperator::In));
        out.extend(
            rest.into_iter()
                .filter(|c| matches!(c.op, BinaryOperator::Like)),
        );
        if let Some(first) = first.filter(|c| is_range_op(&c.op)) {
            out.push(first);
        }
        out
    }

    fn collect_equality_conditions(
        &self,
        expr: &Expression,
        var_name: &str,
        ctx: &Context,
        out: &mut Vec<IndexableCondition>,
    ) {
        if let Expression::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } = expr
        {
            self.collect_equality_conditions(left, var_name, ctx, out);
            self.collect_equality_conditions(right, var_name, ctx, out);
            return;
        }
        if let Some(cond) = self.extract_indexable_condition(expr, var_name, ctx) {
            if matches!(cond.op, BinaryOperator::Equal) {
                out.push(cond);
            }
        }
    }

    /// Resolve the best index for a FILTER expression: composite first (when
    /// 2+ AND'd equality terms cover all of an index's fields), otherwise the
    /// existing single-field path. Returns `(docs, index_name, index_type)` so
    /// EXPLAIN and the executor can report what was used without re-scanning
    /// the index list.
    pub(super) fn lookup_index_for_filter(
        &self,
        collection: &Collection,
        filter: &Expression,
        var_name: &str,
        ctx: &Context,
    ) -> Option<(Vec<crate::storage::Document>, String, String)> {
        self.lookup_index_for_filter_limited(collection, filter, var_name, ctx, None)
    }

    /// [`lookup_index_for_filter`] with an optional cap on the number of
    /// documents fetched. Callers must only pass `Some` when the FILTER is
    /// fully satisfied by the index condition (see
    /// [`Self::filter_fully_covered_by_index`]) — a residual conjunct could
    /// otherwise reject fetched rows and silently under-fill the LIMIT.
    pub(super) fn lookup_index_for_filter_limited(
        &self,
        collection: &Collection,
        filter: &Expression,
        var_name: &str,
        ctx: &Context,
        limit: Option<usize>,
    ) -> Option<(Vec<crate::storage::Document>, String, String)> {
        // A sharded collection's documents live in its physical shards; the
        // logical collection holds none, so an index (or `_key`) lookup here
        // would answer "no rows". Fall back to the scan, which reads the shards.
        if collection
            .get_shard_config()
            .is_some_and(|c| c.num_shards > 0)
        {
            return None;
        }

        // 1. Composite path: `==` / `IN` on leading fields, then the next
        //    field's range or exclusions — two fields at least.
        if let Some(hit) = self.lookup_composite(collection, filter, var_name, ctx, None, 2, limit)
        {
            return Some(hit);
        }

        // 2. Single-field: the first conjunct an index can serve, equalities
        //    and `IN` first. Between those and the prefix and range reads, a
        //    composite index fixed on its first field alone — the only index
        //    on that field, often.
        let (equalities, others): (Vec<_>, Vec<_>) = self
            .ordered_index_candidates(filter, var_name, ctx)
            .into_iter()
            .partition(|c| matches!(c.op, BinaryOperator::Equal | BinaryOperator::In));
        for cond in &equalities {
            if let Some(hit) = self.single_field_hit(collection, cond, limit) {
                return Some(hit);
            }
        }
        if let Some(hit) = self.lookup_composite(collection, filter, var_name, ctx, None, 1, limit)
        {
            return Some(hit);
        }
        let terms = self.composite_terms(filter, var_name, ctx);
        for cond in &others {
            if !self.range_read_is_exact(collection, cond, &terms) {
                continue;
            }
            if let Some(hit) = self.single_field_hit(collection, cond, limit) {
                return Some(hit);
            }
        }

        // 3. Geo index: `DISTANCE(...) <= r` / `GEO_DISTANCE(...) <= r`
        if let Some(hit) =
            self.geo_lookup_for_filter(collection, filter, var_name, ctx, None, limit)
        {
            return Some(hit);
        }

        // 4. An `OR` whose every branch an index serves: the union of them.
        self.lookup_or_union(collection, filter, var_name, ctx)
    }

    /// Whether reading the range `cond` from the single-field index on its
    /// field returns every row the FILTER (`terms`) can accept. A sparse
    /// index has no entry for a null or missing value, which `<` and `<=`
    /// (and `>=` null) accept, so it serves such a range only when another
    /// term rejects null. Anything but a range is decided elsewhere.
    fn range_read_is_exact(
        &self,
        collection: &Collection,
        cond: &IndexableCondition,
        terms: &[(String, CompositeTerm)],
    ) -> bool {
        if !is_range_op(&cond.op) {
            return true;
        }
        match collection.get_single_field_index_for_field(&cond.field) {
            Some(index) if index.sparse => rejects_null(&cond.field, terms),
            _ => true,
        }
    }

    /// [`Self::use_index_for_condition`] with the name and type of the
    /// single-field index it read, for EXPLAIN.
    fn single_field_hit(
        &self,
        collection: &Collection,
        cond: &IndexableCondition,
        limit: Option<usize>,
    ) -> Option<(Vec<crate::storage::Document>, String, String)> {
        let docs = self.use_index_for_condition(collection, cond, limit)?;
        if cond.field == "_key" || cond.field == "_id" {
            return Some((docs, "primary key".to_string(), "Primary".to_string()));
        }
        let (name, type_str) = collection
            .get_all_indexes()
            .into_iter()
            .find(|i| i.fields.len() == 1 && i.fields[0] == cond.field)
            .map(|i| (i.name, format!("{:?}", i.index_type)))
            .unwrap_or_default();
        Some((docs, name, type_str))
    }

    /// A FILTER that is an `OR`, or an AND with an `OR` conjunct, served as
    /// the union of what an index returns for each branch. Every branch must
    /// be served — one that is not would need the scan anyway. The rows are
    /// a superset of the matches (each branch's are), deduplicated by key;
    /// the caller re-checks every FILTER.
    fn lookup_or_union(
        &self,
        collection: &Collection,
        filter: &Expression,
        var_name: &str,
        ctx: &Context,
    ) -> Option<(Vec<crate::storage::Document>, String, String)> {
        let mut conjuncts = Vec::new();
        push_conjuncts(filter, &mut conjuncts);
        conjuncts.into_iter().find_map(|conjunct| {
            let mut branches = Vec::new();
            push_disjuncts(conjunct, &mut branches);
            if branches.len() < 2 {
                return None;
            }
            let mut seen: HashSet<String> = HashSet::new();
            let mut out = Vec::new();
            let mut names: Vec<String> = Vec::new();
            for branch in branches {
                let (docs, name, _) =
                    self.lookup_index_for_filter_limited(collection, branch, var_name, ctx, None)?;
                if !names.contains(&name) {
                    names.push(name);
                }
                out.extend(docs.into_iter().filter(|d| seen.insert(d.key.clone())));
                if out.len() > self.max_intermediate_rows() {
                    break; // the caller's budget check reports it
                }
            }
            Some((out, names.join(" | "), "Union".to_string()))
        })
    }

    /// Every AND'd conjunct of `filter` the composite planner can use:
    /// `==` and ranges (as [`Self::extract_indexable_condition`] reads them,
    /// the field on either side), `IN` lists (nulls kept), `!=` and `NOT IN`.
    fn composite_terms(
        &self,
        filter: &Expression,
        var_name: &str,
        ctx: &Context,
    ) -> Vec<(String, CompositeTerm)> {
        let mut conjuncts = Vec::new();
        push_conjuncts(filter, &mut conjuncts);
        let mut out = Vec::new();
        for conjunct in conjuncts {
            let Expression::BinaryOp { left, op, right } = conjunct else {
                continue;
            };
            match op {
                BinaryOperator::In | BinaryOperator::NotIn => {
                    let Some(field) = self.extract_field_path(left, var_name) else {
                        continue;
                    };
                    let Some(Value::Array(items)) =
                        self.extract_indexable_value(right, var_name, ctx)
                    else {
                        continue;
                    };
                    if items.len() > MAX_INDEXED_IN_KEYS {
                        continue;
                    }
                    let term = if matches!(op, BinaryOperator::In) {
                        CompositeTerm::In(items)
                    } else {
                        CompositeTerm::Not(items)
                    };
                    out.push((field, term));
                }
                BinaryOperator::NotEqual => {
                    let side = |f: &Expression, v: &Expression| {
                        Some((
                            self.extract_field_path(f, var_name)?,
                            self.extract_indexable_value(v, var_name, ctx)?,
                        ))
                    };
                    if let Some((field, value)) = side(left, right).or_else(|| side(right, left)) {
                        out.push((field, CompositeTerm::Not(vec![value])));
                    }
                }
                _ => {
                    let Some(c) = self.extract_indexable_condition(conjunct, var_name, ctx) else {
                        continue;
                    };
                    let term = match c.op {
                        BinaryOperator::Equal => CompositeTerm::Eq(c.value),
                        ref r if is_range_op(r) => CompositeTerm::Range(c.op, c.value),
                        _ => continue,
                    };
                    out.push((c.field, term));
                }
            }
        }
        out
    }

    /// Serve `filter` from a multi-field index (see [`composite_plan`]):
    /// one bounded read per combination of the fixed fields' values and
    /// interval of the next field.
    ///
    /// Without `only`, the index whose plan uses the most fields wins, if it
    /// uses at least `min_fields`. With `only`, that index alone is tried.
    /// The rows returned are a superset of the matches; the caller re-checks
    /// every FILTER.
    #[allow(clippy::too_many_arguments)]
    fn lookup_composite(
        &self,
        collection: &Collection,
        filter: &Expression,
        var_name: &str,
        ctx: &Context,
        only: Option<&str>,
        min_fields: usize,
        limit: Option<usize>,
    ) -> Option<(Vec<crate::storage::Document>, String, String)> {
        let terms = self.composite_terms(filter, var_name, ctx);
        if terms.is_empty() {
            return None;
        }
        let plan = collection
            .get_all_indexes()
            .into_iter()
            .filter(|i| only.is_none_or(|name| i.name == name))
            .filter_map(|i| composite_plan(i, &terms))
            .filter(|p| p.fields_used() >= min_fields)
            .max_by_key(|p| (p.fields_used(), std::cmp::Reverse(p.index.fields.len())))?;

        let cap = limit
            .or(self.scan_cap())
            .unwrap_or(usize::MAX)
            .min(self.max_intermediate_rows().saturating_add(1));
        let unbounded = [(None, None)];
        let intervals: &[(IntervalBound, IntervalBound)] = match &plan.next {
            Some(intervals) => intervals,
            None => &unbounded,
        };
        let mut out = Vec::new();
        // Odometer over the IN lists: one read per combination and interval.
        let mut at = vec![0usize; plan.eq_lists.len()];
        if plan.eq_lists.iter().all(|l| !l.is_empty()) {
            'combos: loop {
                let combo: Vec<Value> = plan
                    .eq_lists
                    .iter()
                    .zip(&at)
                    .map(|(l, &i)| l[i].clone())
                    .collect();
                for (lo, hi) in intervals {
                    let docs = collection.index_scan_composite(
                        &plan.index,
                        &combo,
                        lo.as_ref().map(|(v, inc)| (v, *inc)),
                        hi.as_ref().map(|(v, inc)| (v, *inc)),
                        cap - out.len(),
                    )?;
                    out.extend(docs);
                    if out.len() >= cap {
                        break 'combos; // the caller's budget check reports an overflow
                    }
                }
                let mut d = at.len();
                while d > 0 {
                    d -= 1;
                    at[d] += 1;
                    if at[d] < plan.eq_lists[d].len() {
                        break;
                    }
                    at[d] = 0;
                }
                if at.iter().all(|&i| i == 0) {
                    break;
                }
            }
        }
        let type_str = format!("{:?}", plan.index.index_type);
        Some((out, plan.index.name, type_str))
    }

    /// [`Self::lookup_index_for_filter_limited`] honouring an index hint.
    ///
    /// The hinted indexes are tried first, in order. When none can serve the
    /// FILTER, a forced hint is an error; otherwise the optimizer's own choice
    /// applies.
    pub(super) fn lookup_index_for_filter_hinted(
        &self,
        collection: &Collection,
        filter: &Expression,
        var_name: &str,
        ctx: &Context,
        limit: Option<usize>,
        hint: Option<&IndexHint>,
    ) -> DbResult<Option<(Vec<crate::storage::Document>, String, String)>> {
        let Some(hint) = hint.filter(|h| !h.names.is_empty()) else {
            return Ok(
                self.lookup_index_for_filter_limited(collection, filter, var_name, ctx, limit)
            );
        };
        if let Some(hit) = self.lookup_with_hint(collection, filter, var_name, ctx, limit, hint) {
            return Ok(Some(hit));
        }
        if hint.force {
            return Err(DbError::ExecutionError(format!(
                "could not use index hint to serve query: none of [{}] on '{}' can serve the \
                 FILTER (forceIndexHint is set)",
                hint.names.join(", "),
                collection
                    .name
                    .rsplit(':')
                    .next()
                    .unwrap_or(&collection.name)
            )));
        }
        Ok(self.lookup_index_for_filter_limited(collection, filter, var_name, ctx, limit))
    }

    fn lookup_with_hint(
        &self,
        collection: &Collection,
        filter: &Expression,
        var_name: &str,
        ctx: &Context,
        limit: Option<usize>,
        hint: &IndexHint,
    ) -> Option<(Vec<crate::storage::Document>, String, String)> {
        let indexes = collection.get_all_indexes();
        let candidates = self.ordered_index_candidates(filter, var_name, ctx);
        let terms = self.composite_terms(filter, var_name, ctx);
        for name in &hint.names {
            if let Some(index) = indexes.iter().find(|i| i.name == *name) {
                let type_str = format!("{:?}", index.index_type);
                if index.fields.len() == 1 {
                    for cond in candidates.iter().filter(|c| {
                        c.field == index.fields[0]
                            && self.range_read_is_exact(collection, c, &terms)
                    }) {
                        if let Some(docs) = self.use_index_for_condition(collection, cond, limit) {
                            return Some((docs, index.name.clone(), type_str));
                        }
                    }
                } else if let Some(hit) =
                    self.lookup_composite(collection, filter, var_name, ctx, Some(name), 1, limit)
                {
                    return Some(hit);
                }
            } else if let Some(hit) =
                self.geo_lookup_for_filter(collection, filter, var_name, ctx, Some(name), limit)
            {
                return Some(hit);
            }
        }
        None
    }

    /// True when the FILTER expression is exactly one indexable comparison —
    /// i.e. the index lookup returns precisely the rows the FILTER accepts,
    /// so a LIMIT can be pushed into the lookup. AND/OR trees are excluded:
    /// only one conjunct feeds the index and the rest re-filter afterwards.
    pub(super) fn filter_fully_covered_by_index(
        &self,
        expr: &Expression,
        var_name: &str,
        ctx: &Context,
    ) -> bool {
        match expr {
            Expression::BinaryOp { op, .. } => {
                matches!(
                    op,
                    BinaryOperator::Equal
                        | BinaryOperator::LessThan
                        | BinaryOperator::LessThanOrEqual
                        | BinaryOperator::GreaterThan
                        | BinaryOperator::GreaterThanOrEqual
                        | BinaryOperator::In
                ) && self
                    .extract_indexable_condition(expr, var_name, ctx)
                    .is_some()
            }
            _ => false,
        }
    }

    /// Split a JOIN condition into an equi-join term `var.field == key_expr`
    /// where `key_expr` does not reference `var` (it is evaluated against the
    /// left row instead). Returns the field path on `var`, the key expression
    /// for the other side, and whether the term is the *entire* condition —
    /// when it was pulled out of an AND, the remaining conjuncts must be
    /// re-checked per matched pair.
    pub(super) fn extract_equi_join_term<'e>(
        &self,
        condition: &'e Expression,
        var_name: &str,
    ) -> Option<(String, &'e Expression, bool)> {
        match condition {
            Expression::BinaryOp {
                left,
                op: BinaryOperator::Equal,
                right,
            } => {
                if let Some(field) = self.extract_field_path(left, var_name) {
                    if !expression_references_var(right, var_name) {
                        return Some((field, right.as_ref(), true));
                    }
                }
                if let Some(field) = self.extract_field_path(right, var_name) {
                    if !expression_references_var(left, var_name) {
                        return Some((field, left.as_ref(), true));
                    }
                }
                None
            }
            Expression::BinaryOp {
                left,
                op: BinaryOperator::And,
                right,
            } => self
                .extract_equi_join_term(left, var_name)
                .or_else(|| self.extract_equi_join_term(right, var_name))
                .map(|(field, expr, _)| (field, expr, false)),
            _ => None,
        }
    }

    /// Extract a concrete value from the non-field side of a comparison.
    ///
    /// Accepts literals, bind variables, and any expression that can be
    /// evaluated against `ctx` without referencing `var_name` (the FOR-loop
    /// variable being filtered). This is what allows correlated subqueries
    /// like `FILTER rel._key == doc.organisation_id` to use an index lookup:
    /// `doc.organisation_id` evaluates fine against the parent context, and
    /// the result is fed to the index path.
    pub(super) fn extract_indexable_value(
        &self,
        expr: &Expression,
        var_name: &str,
        ctx: &Context,
    ) -> Option<Value> {
        match expr {
            Expression::Literal(v) => Some(v.clone()),
            Expression::BindVariable(name) => self.bind_vars.get(name).cloned(),
            _ => {
                // Don't evaluate expressions that reference the FOR variable
                // (those depend on the row being filtered, not on parent state).
                if expression_references_var(expr, var_name) {
                    return None;
                }
                self.evaluate_expr_with_context(expr, ctx).ok()
            }
        }
    }

    /// Extract field path from an expression
    #[allow(clippy::only_used_in_recursion)]
    pub(super) fn extract_field_path(&self, expr: &Expression, var_name: &str) -> Option<String> {
        match expr {
            Expression::FieldAccess(base, field) => {
                if let Expression::Variable(name) = base.as_ref() {
                    if name == var_name {
                        return Some(field.clone());
                    }
                }
                if let Some(base_path) = self.extract_field_path(base, var_name) {
                    return Some(format!("{}.{}", base_path, field));
                }
                None
            }
            _ => None,
        }
    }

    /// Extract a vector (array of f32) from a JSON value
    pub(super) fn extract_vector_arg(value: &Value, context: &str) -> DbResult<Vec<f32>> {
        match value {
            Value::Array(arr) => arr
                .iter()
                .map(|v| {
                    v.as_f64().map(|f| f as f32).ok_or_else(|| {
                        DbError::ExecutionError(format!("{} must be an array of numbers", context))
                    })
                })
                .collect(),
            _ => Err(DbError::ExecutionError(format!(
                "{} must be an array",
                context
            ))),
        }
    }

    /// Use index for a condition lookup. `limit` caps the number of fetched
    /// documents (LIMIT pushdown); pass `None` for the full result.
    pub(super) fn use_index_for_condition(
        &self,
        collection: &Collection,
        condition: &IndexableCondition,
        limit: Option<usize>,
    ) -> Option<Vec<crate::storage::Document>> {
        // Fast-path: `_key` is the primary key, served by a direct RocksDB get()
        // instead of a full prefix scan + in-memory filter.
        // `_id` is `"<collection>/<key>"`, so `_id == x` / `_id IN [..]` is a
        // primary-key lookup on the part after the collection prefix.
        if condition.field == "_id" {
            return self.id_fast_path(collection, condition);
        }
        if condition.field == "_key" {
            return self.key_fast_path(collection, condition);
        }

        // Normalize the value for index lookup
        // If it's a float that's actually an integer (e.g., 30.0), convert to integer
        // This handles the case where SDBQL parses "30" as 30.0 but data has integer 30
        let normalized_value = if let Value::Number(n) = &condition.value {
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f.is_finite() {
                    // It's a whole number, try as integer first
                    Value::Number(serde_json::Number::from(f as i64))
                } else {
                    condition.value.clone()
                }
            } else {
                condition.value.clone()
            }
        } else {
            condition.value.clone()
        };

        match condition.op {
            BinaryOperator::Equal => {
                if let Some(k) = limit {
                    // Same normalized-then-original two-try as the unlimited path
                    if let Some(docs) =
                        collection.index_lookup_eq_limit(&condition.field, &normalized_value, k)
                    {
                        if !docs.is_empty() {
                            return Some(docs);
                        }
                    }
                    return collection.index_lookup_eq_limit(&condition.field, &condition.value, k);
                }
                // Try with normalized value first
                if let Some(docs) = collection.index_lookup_eq(&condition.field, &normalized_value)
                {
                    if !docs.is_empty() {
                        return Some(docs);
                    }
                }
                // Fall back to original value
                collection.index_lookup_eq(&condition.field, &condition.value)
            }
            // A range read is bounded by the row ceiling, one past it, so the
            // budget check that follows rejects an oversized range instead of
            // the scan quietly stopping short.
            BinaryOperator::GreaterThan => collection.index_lookup_gt(
                &condition.field,
                &normalized_value,
                limit.or(self.scan_cap()),
            ),
            BinaryOperator::GreaterThanOrEqual => collection.index_lookup_gte(
                &condition.field,
                &normalized_value,
                limit.or(self.scan_cap()),
            ),
            BinaryOperator::LessThan => collection.index_lookup_lt(
                &condition.field,
                &normalized_value,
                limit.or(self.scan_cap()),
            ),
            BinaryOperator::LessThanOrEqual => collection.index_lookup_lte(
                &condition.field,
                &normalized_value,
                limit.or(self.scan_cap()),
            ),
            BinaryOperator::In => self.index_lookup_in(collection, condition, limit),
            BinaryOperator::Like => {
                let prefix = condition.value.as_str()?;
                let cap = limit.unwrap_or(self.max_intermediate_rows().saturating_add(1));
                index_prefix_scan(collection, &condition.field, prefix, cap)
            }
            _ => None,
        }
    }

    /// `field IN [k1, k2, …]`: one equality lookup per distinct key. Keys are
    /// deduplicated by their index encoding (numbers compare as f64 there,
    /// like `==`), so each matching document is returned once.
    fn index_lookup_in(
        &self,
        collection: &Collection,
        condition: &IndexableCondition,
        limit: Option<usize>,
    ) -> Option<Vec<crate::storage::Document>> {
        let Value::Array(items) = &condition.value else {
            return None;
        };
        // Establish that an index exists before claiming the lookup, so an
        // empty list does not report an index it never touched. A sparse
        // index holds no null entries, so a list with null needs the scan.
        let index = regular_index_on(collection, &condition.field)?;
        if index.sparse && items.iter().any(Value::is_null) {
            return None;
        }
        let mut seen: HashSet<Vec<u8>> = HashSet::with_capacity(items.len());
        let mut out = Vec::new();
        for item in items {
            if !seen.insert(crate::storage::codec::encode_key(item)) {
                continue;
            }
            let docs = match limit {
                Some(k) => {
                    let remaining = k.saturating_sub(out.len());
                    if remaining == 0 {
                        break;
                    }
                    collection.index_lookup_eq_limit(&condition.field, item, remaining)?
                }
                None => collection.index_lookup_eq(&condition.field, item)?,
            };
            out.extend(docs);
            if out.len() > self.max_intermediate_rows() {
                break; // the caller's budget check reports it
            }
        }
        Some(out)
    }

    /// Primary-key point-lookup for `doc._key == <expr>`.
    /// Returns `Some(Vec)` for equality (treated as an indexed lookup so the
    /// scan path is skipped) and `None` for non-equality ops so range filters
    /// fall through to the scan path.
    fn id_fast_path(
        &self,
        collection: &Collection,
        condition: &IndexableCondition,
    ) -> Option<Vec<crate::storage::Document>> {
        // Ask `Document` for the prefix rather than assuming its format: the
        // stored collection name can carry the database.
        let probe =
            crate::storage::Document::with_key(&collection.name, "\u{0}".into(), Value::Null);
        let prefix = probe.id.strip_suffix('\u{0}')?.to_string();
        let to_key = |v: &Value| {
            v.as_str()?
                .strip_prefix(prefix.as_str())
                .map(str::to_string)
        };

        let keys = match (&condition.op, &condition.value) {
            // An `_id` of another collection, or a non-string, matches nothing.
            (BinaryOperator::Equal, v) => {
                Value::Array(to_key(v).map(Value::String).into_iter().collect())
            }
            (BinaryOperator::In, Value::Array(ids)) => {
                Value::Array(ids.iter().filter_map(to_key).map(Value::String).collect())
            }
            _ => return None,
        };
        self.key_fast_path(
            collection,
            &IndexableCondition {
                field: "_key".to_string(),
                op: BinaryOperator::In,
                value: keys,
            },
        )
    }

    fn key_fast_path(
        &self,
        collection: &Collection,
        condition: &IndexableCondition,
    ) -> Option<Vec<crate::storage::Document>> {
        if let (BinaryOperator::In, Value::Array(keys)) = (&condition.op, &condition.value) {
            let mut seen: HashSet<&str> = HashSet::with_capacity(keys.len());
            let mut out = Vec::new();
            for key in keys.iter().filter_map(Value::as_str) {
                if !seen.insert(key) {
                    continue;
                }
                match collection.get(key) {
                    Ok(doc) => out.push(doc),
                    Err(DbError::DocumentNotFound(_)) => {}
                    Err(_) => return None,
                }
            }
            return Some(out);
        }
        if !matches!(condition.op, BinaryOperator::Equal) {
            return None;
        }
        // `_key` is always a string at insert time; a non-string literal
        // cannot match any document.
        let Some(key) = condition.value.as_str() else {
            return Some(Vec::new());
        };
        match collection.get(key) {
            Ok(doc) => Some(vec![doc]),
            Err(DbError::DocumentNotFound(_)) => Some(Vec::new()),
            Err(_) => None,
        }
    }

    /// Whether this FILTER miss is allowed to create `_auto_{field}`.
    /// Does not create. EXPLAIN uses the same predicate.
    ///
    /// Cheap checks first: this runs on the FILTER path of every query against
    /// a collection, so the storage reads must sit behind the in-memory ones.
    /// The opt-in flag comes next: it is off by default, so it ends nearly
    /// every call, and it is read once per query (a correlated subquery calls
    /// this once per run). Then the shard config, the document count, and the
    /// index list, borrowed from the cached snapshot (`get_all_indexes` would
    /// clone every definition).
    pub(super) fn would_auto_index(
        &self,
        collection: &Collection,
        field: &str,
        value: Option<&Value>,
    ) -> bool {
        if value.is_some_and(Value::is_null) {
            return false;
        }
        if !field_is_auto_indexable(field) {
            return false;
        }
        // Creating an index is a write, and the query routes that reach here
        // are classified Read (`/cursor`, `/sql`, `/nl`, `/explain`, the
        // driver's Query op, ...). Absence of a principal is *not* permission:
        // an executor built without one — internal refreshes, jobs, stream
        // tasks — does not auto-index either.
        match &self.principal {
            Some(p) if p.can_write || p.can_admin => {}
            _ => return false,
        }
        if crate::storage::is_protected_collection(&collection.name)
            || collection_bare_name(&collection.name).starts_with('_')
        {
            return false;
        }
        if !self.auto_index_enabled(collection) {
            return false;
        }
        if collection
            .get_shard_config()
            .is_some_and(|c| c.num_shards > 0)
        {
            return false;
        }
        let max_docs = auto_index_max_docs();
        if max_docs > 0 && collection.count() > max_docs {
            return false;
        }
        let meta = collection.index_meta();
        let indexes = meta.as_deref().map_or(&[][..], |m| m.indexes.as_slice());
        if indexes
            .iter()
            .any(|i| i.fields.len() == 1 && i.fields[0] == field)
        {
            return false;
        }
        indexes.iter().filter(|i| is_auto_index(i)).count() < AUTO_INDEX_CAP
    }

    /// Create a persistent `_auto_{field}` index when allowed. `true` only if
    /// `create_index` succeeded and the index can actually serve a lookup — a
    /// failed backfill must not look ready.
    pub(super) fn maybe_auto_index(
        &self,
        collection: &Collection,
        field: &str,
        value: Option<&Value>,
    ) -> bool {
        if !self.would_auto_index(collection, field, value) {
            return false;
        }
        let name = auto_index_name(field);
        let spec = IndexSpec::Regular {
            name: name.clone(),
            fields: vec![field.to_string()],
            index_type: IndexType::Persistent,
            unique: false,
            sparse: true,
        };
        match collection.create_index(
            name.clone(),
            vec![field.to_string()],
            IndexType::Persistent,
            false,
        ) {
            Ok(stats) if stats.indexed_documents == 0 => {
                // No document carries the field — a misspelled or absent name.
                // Keeping the index would burn one of the 16 slots and add
                // write amplification to every later insert for a lookup that
                // can never match.
                if let Err(e) = collection.drop_index(&name) {
                    tracing::warn!(
                        collection = %collection.name,
                        field,
                        error = %e,
                        "could not drop the empty auto-index"
                    );
                }
                false
            }
            Ok(_) => {
                self.propagate_auto_index(collection, &spec);
                true
            }
            Err(e) => {
                tracing::warn!(
                    collection = %collection.name,
                    field,
                    error = %e,
                    "auto-index create failed; falling back to scan"
                );
                false
            }
        }
    }

    fn propagate_auto_index(&self, collection: &Collection, spec: &IndexSpec) {
        let Some(db) = self.database.as_deref() else {
            return;
        };
        if let Some(repl) = self.replication {
            let payload = serde_json::to_vec(spec).ok();
            let target = collection_bare_name(&collection.name).to_string();
            repl.append(crate::sync::log::LogEntry::new_op(
                db,
                target,
                crate::sync::protocol::Operation::CreateIndex,
                spec.name().to_string(),
                payload,
            ));
        }
    }
}

/// A FILTER conjunct the subquery pass can serve: `v.f == S`, `S == v.f` or
/// `v.f IN S`, where `S` holds a subquery and does not read `v`, and `f` has
/// an index that answers exactly what the scan's `==` does.
struct SubqueryKeyTerm<'e> {
    field: String,
    in_list: bool,
    /// `S`, borrowed from the FILTER the re-check evaluates, so planner and
    /// re-check share one memo entry. A copy (what [`conjunction`] makes of
    /// two FILTERs) is a new node address, hence a new entry and a second
    /// run of the subquery on every run of the stage.
    value: &'e Expression,
}

/// A FOR stage served by [`QueryExecutor::try_subquery_index_stage`].
pub(super) struct SubqueryIndexStage {
    /// The stage's rows, every leading FILTER applied.
    pub rows: Vec<Context>,
    pub index_name: String,
    pub index_type: String,
    /// Documents read from the index, before the FILTER re-check.
    pub docs_read: usize,
}

/// `filter`'s AND chain, in order, borrowed.
/// The branches of an `OR` chain, in order; any other expression is one
/// branch.
fn push_disjuncts<'e>(expr: &'e Expression, out: &mut Vec<&'e Expression>) {
    match expr {
        Expression::BinaryOp {
            left,
            op: BinaryOperator::Or,
            right,
        } => {
            push_disjuncts(left, out);
            push_disjuncts(right, out);
        }
        other => out.push(other),
    }
}

pub(super) fn push_conjuncts<'e>(filter: &'e Expression, out: &mut Vec<&'e Expression>) {
    match filter {
        Expression::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            push_conjuncts(left, out);
            push_conjuncts(right, out);
        }
        other => out.push(other),
    }
}

/// `v.a.b` as the path `a.b`: a chain of plain field accesses on `var`, no
/// segment holding a dot. The index splits its field on dots, so a `"a.b"`
/// segment would be read there as `a` then `b`.
fn plain_field_path(expr: &Expression, var: &str) -> Option<String> {
    let mut segments = Vec::new();
    let mut cur = expr;
    loop {
        match cur {
            Expression::FieldAccess(base, field) if !field.contains('.') => {
                segments.push(field.as_str());
                cur = base;
            }
            Expression::Variable(name) if name == var && !segments.is_empty() => break,
            _ => return None,
        }
    }
    segments.reverse();
    Some(segments.join("."))
}

/// A key for which an index lookup returns every document the scan's `==`
/// accepts. Not null: null is never indexed. Not a numeric zero: `encode_key`
/// keeps `-0.0` apart from `0`, which `==` equates. Not an array or object:
/// those are encoded as JSON text, where `1` and `1.0` differ. Every other
/// number is encoded as its f64, so `2` and `2.0` share a key.
fn exact_index_key(value: &Value) -> bool {
    match value {
        Value::Null | Value::Array(_) | Value::Object(_) => false,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::Bool(_) | Value::String(_) => true,
    }
}

/// Whether the first single-field index on `field` (the one the equality
/// lookup reads) is exact. Bloom and Cuckoo answer a definite miss from a
/// filter of `value.to_string()`, where `2` and `2.0` differ, and that filter
/// is filled only when the index is built, so a miss can be wrong; the other
/// types do not serve `==`.
fn exactly_indexed(indexes: &[crate::storage::index::Index], field: &str) -> bool {
    if matches!(field, "_key" | "_id") {
        return true;
    }
    indexes
        .iter()
        .find(|i| i.fields.len() == 1 && i.fields[0] == field)
        .is_some_and(|i| matches!(i.index_type, IndexType::Persistent | IndexType::Hash))
}

impl QueryExecutor<'_> {
    /// Subquery-valued equality and `IN` served by an index: a second pass, tried
    /// only after the planner above has given the stage to the scan.
    ///
    /// The planner never evaluates a subquery (`expression_references_var`
    /// counts every subquery as reading the loop variable), so
    /// `FOR m IN comments FILTER m.card_id IN (FOR c IN cards ... RETURN c._key)`
    /// scanned all 2,022 comments where its hoisted `LET ids = (...)` form read 8
    /// from the index on `card_id`. A subquery the memo can reuse, and that does
    /// not read the loop variable, has one value for the whole stage row, so it
    /// can be a lookup key like a `LET` variable.
    ///
    /// This pass only ever replaces a scan, and returns what the scan returns,
    /// order included: each row's documents are sorted by `_key`, the scan's
    /// `doc:<key>` order, and no LIMIT is pushed into the lookup (which would keep
    /// different rows than the scan without a SORT). So it is limited to what an
    /// index answers exactly as the scan's comparison does: `==` and `IN` (a range
    /// read misses documents whose field is null or missing, and returns index
    /// order), on `_key`, `_id` or a single-field Persistent or Hash index, with
    /// keys [`exact_index_key`] accepts. Servability is decided from the AST and
    /// the index list before anything is evaluated, so a stage that will scan
    /// never runs the subquery here. Every row of the stage is served, or none.
    pub(super) fn try_subquery_index_stage(
        &self,
        for_clause: &ForClause,
        filters: &[&Expression],
        rows: &[Context],
        scan_limit: Option<usize>,
    ) -> DbResult<Option<SubqueryIndexStage>> {
        let name = &for_clause.collection;
        // Writing queries keep reading per row, so the planner never runs a
        // subquery that could write. A pushed-down scan limit applies before
        // the FILTER, which no lookup can reproduce.
        if !self.caches.reuse_enabled()
            || filters.is_empty()
            || rows.is_empty()
            || scan_limit.is_some()
            || name.is_empty()
            || for_clause.source_expression.is_some()
            || for_clause
                .source_variable
                .as_ref()
                .is_some_and(|s| s != name)
            || for_clause.system_time.is_some()
            || for_clause.valid_time.is_some()
        {
            return Ok(None);
        }
        // A LET-bound array of that name is iterated, not the collection; a
        // row policy, a columnar layout or shards all change what the scan
        // reads.
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

        let var = for_clause.variable.as_str();
        let meta = collection.index_meta();
        let indexes = meta.as_deref().map_or(&[][..], |m| m.indexes.as_slice());
        let mut parts = Vec::new();
        for filter in filters {
            push_conjuncts(filter, &mut parts);
        }
        let mut terms: Vec<SubqueryKeyTerm<'_>> = parts
            .into_iter()
            .filter_map(|part| self.subquery_key_term(part, var))
            .filter(|t| exactly_indexed(indexes, &t.field))
            .collect();
        if terms.is_empty() {
            return Ok(None);
        }
        // `==` first: one key, where an `IN` list is a lookup per key.
        terms.sort_by_key(|t| t.in_list);

        let mut new_rows = Vec::new();
        let mut used: Option<&str> = None;
        let mut docs_read = 0;
        for ctx in rows {
            let Some((field, mut docs)) = self.serve_subquery_key_row(&collection, &terms, ctx)
            else {
                return Ok(None);
            };
            used.get_or_insert(field);
            docs.sort_by(|a, b| a.key.cmp(&b.key));
            docs.dedup_by(|a, b| a.key == b.key);
            docs_read += docs.len();
            for doc in docs {
                let mut new_ctx = ctx.clone();
                new_ctx.insert(for_clause.variable.clone(), doc.into_value());
                new_rows.push(new_ctx);
                if new_rows.len() % super::execution::BUDGET_CHECK_INTERVAL == 0 {
                    self.check_budget(new_rows.len())?;
                }
            }
        }
        new_rows.retain(|ctx| {
            filters
                .iter()
                .all(|f| self.evaluate_filter_with_context(f, ctx).unwrap_or(false))
        });
        // Reported as the planner reports a single-field lookup.
        let (index_name, index_type) = used
            .and_then(|field| {
                indexes
                    .iter()
                    .find(|i| i.fields.len() == 1 && i.fields[0] == field)
            })
            .map(|i| (i.name.clone(), format!("{:?}", i.index_type)))
            .unwrap_or_default();
        Ok(Some(SubqueryIndexStage {
            rows: new_rows,
            index_name,
            index_type,
            docs_read,
        }))
    }

    /// `part` as a [`SubqueryKeyTerm`], from the AST alone.
    fn subquery_key_term<'e>(
        &self,
        part: &'e Expression,
        var: &str,
    ) -> Option<SubqueryKeyTerm<'e>> {
        let Expression::BinaryOp { left, op, right } = part else {
            return None;
        };
        let (field, value, in_list) = match op {
            BinaryOperator::Equal => match plain_field_path(left, var) {
                Some(field) => (field, right.as_ref(), false),
                None => (plain_field_path(right, var)?, left.as_ref(), false),
            },
            BinaryOperator::In => (plain_field_path(left, var)?, right.as_ref(), true),
            _ => return None,
        };
        let holds_subquery = expression_any(value, &mut |e| matches!(e, Expression::Subquery(_)));
        // Evaluated once per stage row where the scan evaluates it per
        // document: it must give the same value each time.
        if !holds_subquery
            || expression_any(value, &mut super::subquery_memo::is_nondeterministic)
            || self.value_reads_var(value, var)
        {
            return None;
        }
        Some(SubqueryKeyTerm {
            field,
            in_list,
            value,
        })
    }

    /// Whether `expr` may read `var`: a reference outside any subquery, or a
    /// subquery that the memo's analysis says may read it (or cannot tell).
    pub(super) fn value_reads_var(&self, expr: &Expression, var: &str) -> bool {
        match expr {
            Expression::Variable(name) => name == var,
            Expression::Subquery(q) => self.subquery_may_read(q, var),
            other => {
                let mut reads = false;
                other.for_each_child(&mut |child| {
                    reads = reads || self.value_reads_var(child, var);
                });
                reads
            }
        }
    }

    /// The documents for one stage row from the first term that can serve
    /// it, and that term's field. A bare subquery's rows come from the memo,
    /// shared; a value that is unusable or fails to evaluate passes to the
    /// next term.
    fn serve_subquery_key_row<'t>(
        &self,
        collection: &Collection,
        terms: &'t [SubqueryKeyTerm<'_>],
        ctx: &Context,
    ) -> Option<(&'t str, Vec<crate::storage::Document>)> {
        for term in terms {
            let value = match term.value {
                Expression::Subquery(q) => self.planner_subquery_rows(q, ctx),
                other => self
                    .evaluate_expr_with_context(other, ctx)
                    .ok()
                    .map(std::sync::Arc::new),
            };
            let Some(value) = value else {
                continue;
            };
            let usable = if term.in_list {
                matches!(&*value, Value::Array(items)
                    if items.len() <= MAX_INDEXED_IN_KEYS && items.iter().all(exact_index_key))
            } else {
                exact_index_key(&value)
            };
            if !usable {
                continue;
            }
            // The one copy of a memoised list, made only for a list about to
            // be read key by key, which costs more than copying it.
            let condition = IndexableCondition {
                field: term.field.clone(),
                op: if term.in_list {
                    BinaryOperator::In
                } else {
                    BinaryOperator::Equal
                },
                value: std::sync::Arc::unwrap_or_clone(value),
            };
            if let Some(docs) = self.use_index_for_condition(collection, &condition, None) {
                return Some((term.field.as_str(), docs));
            }
        }
        None
    }
}

/// Returns true if `expr` references `var_name` anywhere (conservative: lambda
/// parameter shadowing is ignored, which only ever produces false positives — at
/// worst, we forgo the index optimization and fall back to a scan).
pub(super) fn expression_references_var(expr: &Expression, var_name: &str) -> bool {
    match expr {
        Expression::Variable(name) => name == var_name,
        Expression::BindVariable(_) | Expression::Literal(_) => false,
        Expression::FieldAccess(base, _) | Expression::OptionalFieldAccess(base, _) => {
            expression_references_var(base, var_name)
        }
        Expression::DynamicFieldAccess(base, key) => {
            expression_references_var(base, var_name) || expression_references_var(key, var_name)
        }
        Expression::ArrayAccess(base, idx) => {
            expression_references_var(base, var_name) || expression_references_var(idx, var_name)
        }
        Expression::ArraySpreadAccess(base, _) => expression_references_var(base, var_name),
        Expression::BinaryOp { left, right, .. } => {
            expression_references_var(left, var_name) || expression_references_var(right, var_name)
        }
        Expression::UnaryOp { operand, .. } => expression_references_var(operand, var_name),
        Expression::Object(fields) => fields
            .iter()
            .any(|(_, e)| expression_references_var(e, var_name)),
        Expression::Array(items) => items.iter().any(|e| expression_references_var(e, var_name)),
        Expression::Range(a, b) => {
            expression_references_var(a, var_name) || expression_references_var(b, var_name)
        }
        Expression::FunctionCall { args, .. } => {
            args.iter().any(|e| expression_references_var(e, var_name))
        }
        Expression::Subquery(_) => {
            // Conservative: assume any subquery may correlate on var_name.
            true
        }
        Expression::Ternary {
            condition,
            true_expr,
            false_expr,
        } => {
            expression_references_var(condition, var_name)
                || expression_references_var(true_expr, var_name)
                || expression_references_var(false_expr, var_name)
        }
        Expression::Case {
            operand,
            when_clauses,
            else_clause,
        } => {
            operand
                .as_deref()
                .is_some_and(|e| expression_references_var(e, var_name))
                || when_clauses.iter().any(|(c, r)| {
                    expression_references_var(c, var_name) || expression_references_var(r, var_name)
                })
                || else_clause
                    .as_deref()
                    .is_some_and(|e| expression_references_var(e, var_name))
        }
        Expression::Pipeline { left, right } => {
            expression_references_var(left, var_name) || expression_references_var(right, var_name)
        }
        Expression::Lambda { body, .. } => expression_references_var(body, var_name),
        Expression::ArrayComparison {
            quantifier,
            left,
            right,
            ..
        } => {
            expression_references_var(left, var_name)
                || expression_references_var(right, var_name)
                || matches!(quantifier, crate::sdbql::ast::ArrayQuantifier::AtLeast(n)
                    if expression_references_var(n, var_name))
        }
        Expression::ArrayInline {
            base,
            filter,
            limit,
            projection,
            ..
        } => {
            expression_references_var(base, var_name)
                || filter
                    .as_deref()
                    .is_some_and(|e| expression_references_var(e, var_name))
                || limit.as_ref().is_some_and(|(o, c)| {
                    expression_references_var(o, var_name) || expression_references_var(c, var_name)
                })
                || projection
                    .as_deref()
                    .is_some_and(|e| expression_references_var(e, var_name))
        }
        Expression::WindowFunctionCall {
            arguments,
            over_clause,
            ..
        } => {
            arguments
                .iter()
                .any(|e| expression_references_var(e, var_name))
                || over_clause
                    .partition_by
                    .iter()
                    .any(|e| expression_references_var(e, var_name))
                || over_clause
                    .order_by
                    .iter()
                    .any(|(e, _)| expression_references_var(e, var_name))
        }
        Expression::TemplateString { parts } => parts.iter().any(|p| match p {
            TemplateStringPart::Expression(e) => expression_references_var(e, var_name),
            TemplateStringPart::Literal(_) => false,
        }),
    }
}

#[cfg(test)]
mod optimizer_helper_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn like_prefix_stops_at_wildcards_and_escapes() {
        assert_eq!(like_literal_prefix("abc%"), Some("abc".to_string()));
        assert_eq!(like_literal_prefix("ab_c%"), Some("ab".to_string()));
        assert_eq!(like_literal_prefix("ab\\%c"), Some("ab".to_string()));
        assert_eq!(like_literal_prefix("exact"), Some("exact".to_string()));
        assert_eq!(like_literal_prefix("%abc"), None);
        assert_eq!(like_literal_prefix(""), None);
    }

    #[test]
    fn prefix_successor_bounds_every_extension() {
        assert_eq!(prefix_successor("abc").as_deref(), Some("abd"));
        assert_eq!(prefix_successor("a\u{10FFFF}").as_deref(), Some("b"));
        // U+D7FF is followed by the surrogate gap; the next scalar is U+E000.
        assert_eq!(prefix_successor("\u{D7FF}").as_deref(), Some("\u{E000}"));
        assert_eq!(prefix_successor("\u{10FFFF}"), None);
        let upper = prefix_successor("User1").unwrap();
        for s in ["User1", "User1\u{10FFFF}z", "User19"] {
            assert!(s >= "User1" && s < upper.as_str(), "{s}");
        }
        assert!("User2" >= upper.as_str());
    }

    #[test]
    fn in_lists_of_too_many_keys_are_not_indexed() {
        assert!(in_list_is_indexable(&json!([1, "a"])));
        assert!(in_list_is_indexable(&json!([])));
        // Kept: only a non-sparse index serves the null (`index_lookup_in`).
        assert!(in_list_is_indexable(&json!([1, null])));
        assert!(!in_list_is_indexable(&json!({"a": 1})));
        let long: Vec<Value> = (0..=MAX_INDEXED_IN_KEYS).map(|i| json!(i)).collect();
        assert!(!in_list_is_indexable(&Value::Array(long)));
    }

    #[test]
    fn index_hint_from_options() {
        assert_eq!(IndexHint::from_options(None, true), None);
        assert_eq!(IndexHint::from_options(Some(&[][..]), false), None);
        let names = vec!["a".to_string()];
        assert_eq!(
            IndexHint::from_options(Some(names.as_slice()), true),
            Some(IndexHint {
                names: names.clone(),
                force: true
            })
        );
    }
}
