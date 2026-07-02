//! Index optimization for SDBQL executor.
//!
//! This module contains index-related optimizations:
//! - extract_indexable_condition: Extract conditions that can use indexes
//! - extract_field_path: Extract field path from expression
//! - use_index_for_condition: Try to use index for condition lookup

use serde_json::Value;

use super::types::{Context, IndexableCondition};
use super::QueryExecutor;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::*;
use crate::storage::Collection;

impl<'a> QueryExecutor<'a> {
    pub(super) fn extract_indexable_condition(
        &self,
        expr: &Expression,
        var_name: &str,
        ctx: &Context,
    ) -> Option<IndexableCondition> {
        if let Expression::BinaryOp { left, op, right } = expr {
            match op {
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

        // 2. Single-field fallback
        let cond = self.extract_indexable_condition(filter, var_name, ctx)?;
        let docs = self.use_index_for_condition(collection, &cond, limit)?;
        let (name, type_str) = collection
            .get_all_indexes()
            .into_iter()
            .find(|i| i.fields.len() == 1 && i.fields[0] == cond.field)
            .map(|i| (i.name, format!("{:?}", i.index_type)))
            .unwrap_or_default();
        Some((docs, name, type_str))
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
    fn extract_indexable_value(
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
            BinaryOperator::GreaterThan => {
                collection.index_lookup_gt(&condition.field, &normalized_value, limit)
            }
            BinaryOperator::GreaterThanOrEqual => {
                collection.index_lookup_gte(&condition.field, &normalized_value, limit)
            }
            BinaryOperator::LessThan => {
                collection.index_lookup_lt(&condition.field, &normalized_value, limit)
            }
            BinaryOperator::LessThanOrEqual => {
                collection.index_lookup_lte(&condition.field, &normalized_value, limit)
            }
            _ => None,
        }
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
}

/// Returns true if `expr` references `var_name` anywhere (conservative: lambda
/// parameter shadowing is ignored, which only ever produces false positives — at
/// worst, we forgo the index optimization and fall back to a scan).
fn expression_references_var(expr: &Expression, var_name: &str) -> bool {
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
