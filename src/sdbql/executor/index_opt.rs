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

use std::collections::HashSet;

use rust_rocksdb::{Direction, IteratorMode};
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

/// An `IN` right-hand side that per-key lookups can serve: an array without
/// `null` (index entries never hold null) and of bounded length.
fn in_list_is_indexable(value: &Value) -> bool {
    matches!(value, Value::Array(items)
        if items.len() <= MAX_INDEXED_IN_KEYS && !items.iter().any(Value::is_null))
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
    let cf = db.cf_handle(&collection.name)?;
    let mut doc_keys: Vec<Vec<u8>> = Vec::new();
    let iter = db.iterator_cf(&cf, IteratorMode::From(lo.as_bytes(), Direction::Forward));
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
        .multi_get_cf(doc_keys.iter().map(|k| (&cf, k.as_slice())))
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
        // 1. Composite path
        let eq_conditions = self.extract_equality_conditions(filter, var_name, ctx);
        if eq_conditions.len() >= 2 {
            let pairs: Vec<(String, Value)> = eq_conditions
                .iter()
                .map(|c| (c.field.clone(), c.value.clone()))
                .collect();
            if let Some((index, docs)) = collection.index_lookup_eq_composite(&pairs) {
                let type_str = format!("{:?}", index.index_type);
                return Some((docs, index.name, type_str));
            }
        }

        // 2. Single-field: the first conjunct an index can serve
        for cond in self.ordered_index_candidates(filter, var_name, ctx) {
            if let Some(docs) = self.use_index_for_condition(collection, &cond, limit) {
                let (name, type_str) = collection
                    .get_all_indexes()
                    .into_iter()
                    .find(|i| i.fields.len() == 1 && i.fields[0] == cond.field)
                    .map(|i| (i.name, format!("{:?}", i.index_type)))
                    .unwrap_or_default();
                return Some((docs, name, type_str));
            }
        }

        // 3. Geo index: `DISTANCE(...) <= r` / `GEO_DISTANCE(...) <= r`
        self.geo_lookup_for_filter(collection, filter, var_name, ctx, None, limit)
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
        for name in &hint.names {
            if let Some(index) = indexes.iter().find(|i| i.name == *name) {
                let type_str = format!("{:?}", index.index_type);
                if index.fields.len() == 1 {
                    for cond in candidates.iter().filter(|c| c.field == index.fields[0]) {
                        if let Some(docs) = self.use_index_for_condition(collection, cond, limit) {
                            return Some((docs, index.name.clone(), type_str));
                        }
                    }
                } else {
                    let pairs: Vec<(String, Value)> = self
                        .extract_equality_conditions(filter, var_name, ctx)
                        .into_iter()
                        .map(|c| (c.field, c.value))
                        .collect();
                    if let Some((used, docs)) = collection.index_lookup_eq_composite(&pairs) {
                        if used.name == *name {
                            return Some((docs, used.name, type_str));
                        }
                    }
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
        // TODO(_id fast-path): handle `doc._id == "coll/key"` similarly.
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
        // empty list does not report an index it never touched.
        regular_index_on(collection, &condition.field)?;
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
    /// a collection, so the storage reads (`auto_index_enabled`, the index
    /// list, the shard config) must sit behind the in-memory ones.
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
        if !collection.auto_index_enabled() {
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
        let indexes = collection.get_all_indexes();
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
    fn in_lists_with_null_or_too_many_keys_are_not_indexed() {
        assert!(in_list_is_indexable(&json!([1, "a"])));
        assert!(in_list_is_indexable(&json!([])));
        assert!(!in_list_is_indexable(&json!([1, null])));
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
