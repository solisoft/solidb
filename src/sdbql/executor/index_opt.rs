//! Index optimization for SDBQL executor.
//!
//! This module contains index-related optimizations:
//! - extract_indexable_condition: Extract conditions that can use indexes
//! - extract_field_path: Extract field path from expression
//! - use_index_for_condition: Try to use index for condition lookup

use serde_json::Value;

use super::types::IndexableCondition;
use super::QueryExecutor;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::*;
use crate::storage::Collection;

impl<'a> QueryExecutor<'a> {
    pub(super) fn extract_indexable_condition(
        &self,
        expr: &Expression,
        var_name: &str,
    ) -> Option<IndexableCondition> {
        if let Expression::BinaryOp { left, op, right } = expr {
            match op {
                BinaryOperator::Equal
                | BinaryOperator::LessThan
                | BinaryOperator::LessThanOrEqual
                | BinaryOperator::GreaterThan
                | BinaryOperator::GreaterThanOrEqual => {
                    // Try left = field access, right = literal OR bind param
                    if let Some(field) = self.extract_field_path(left, var_name) {
                        let value_opt = match right.as_ref() {
                            Expression::Literal(v) => Some(v.clone()),
                            Expression::BindVariable(name) => self.bind_vars.get(name).cloned(),
                            _ => None,
                        };

                        if let Some(value) = value_opt {
                            return Some(IndexableCondition {
                                field,
                                op: op.clone(),
                                value,
                            });
                        }
                    }
                    // Try right = field access, left = literal OR bind param
                    if let Some(field) = self.extract_field_path(right, var_name) {
                        let value_opt = match left.as_ref() {
                            Expression::Literal(v) => Some(v.clone()),
                            Expression::BindVariable(name) => self.bind_vars.get(name).cloned(),
                            _ => None,
                        };

                        if let Some(value) = value_opt {
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
                    if let Some(cond) = self.extract_indexable_condition(left, var_name) {
                        return Some(cond);
                    }
                    return self.extract_indexable_condition(right, var_name);
                }
                _ => {}
            }
        }
        None
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
}
