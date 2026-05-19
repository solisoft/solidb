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

    /// Use index for a condition lookup
    pub(super) fn use_index_for_condition(
        &self,
        collection: &Collection,
        condition: &IndexableCondition,
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
                collection.index_lookup_gt(&condition.field, &normalized_value)
            }
            BinaryOperator::GreaterThanOrEqual => {
                collection.index_lookup_gte(&condition.field, &normalized_value)
            }
            BinaryOperator::LessThan => {
                collection.index_lookup_lt(&condition.field, &normalized_value)
            }
            BinaryOperator::LessThanOrEqual => {
                collection.index_lookup_lte(&condition.field, &normalized_value)
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
