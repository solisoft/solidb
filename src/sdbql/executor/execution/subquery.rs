//! Subquery execution for SDBQL executor.
//!
//! This module contains execute_with_parent_context for correlated subqueries.

use std::collections::HashMap;

use serde_json::Value;

use super::super::types::Context;
use super::super::{compare_values, QueryExecutor};
use crate::error::DbResult;
use crate::sdbql::ast::*;

impl<'a> QueryExecutor<'a> {
    /// Execute query with parent context for correlated subqueries
    pub(in crate::sdbql::executor) fn execute_with_parent_context(
        &self,
        query: &Query,
        parent_ctx: &Context,
    ) -> DbResult<Vec<Value>> {
        // Start with parent context (enables correlation with outer query)
        let mut initial_bindings = parent_ctx.clone();

        // Add bind variables
        for (key, value) in &self.bind_vars {
            initial_bindings.insert(format!("@{}", key), value.clone());
        }

        // Evaluate initial LET clauses (before FOR)
        for let_clause in &query.let_clauses {
            let value =
                self.evaluate_expr_with_context(&let_clause.expression, &initial_bindings)?;
            initial_bindings.insert(let_clause.variable.clone(), value);
        }

        // Process body_clauses in order
        let rows = if !query.body_clauses.is_empty() {
            let (r, _) = self.execute_body_clauses(&query.body_clauses, &initial_bindings, None)?;
            r
        } else {
            let mut rows =
                self.build_row_combinations_with_context(&query.for_clauses, &initial_bindings)?;
            for filter in &query.filter_clauses {
                rows.retain(|ctx| {
                    self.evaluate_filter_with_context(&filter.expression, ctx)
                        .unwrap_or(false)
                });
            }
            rows
        };

        let mut rows = rows;

        // Apply SORT
        if let Some(sort) = &query.sort_clause {
            rows.sort_by(|a, b| {
                for (expr, ascending) in &sort.fields {
                    let a_val = self
                        .evaluate_expr_with_context(expr, a)
                        .unwrap_or(Value::Null);
                    let b_val = self
                        .evaluate_expr_with_context(expr, b)
                        .unwrap_or(Value::Null);

                    let cmp = compare_values(&a_val, &b_val);
                    if cmp != std::cmp::Ordering::Equal {
                        return if *ascending { cmp } else { cmp.reverse() };
                    }
                }
                std::cmp::Ordering::Equal
            });
        }

        // Apply LIMIT
        if let Some(limit) = &query.limit_clause {
            let offset = self
                .evaluate_expr_with_context(&limit.offset, &initial_bindings)
                .ok()
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0);
            let count = self
                .evaluate_expr_with_context(&limit.count, &initial_bindings)
                .ok()
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0);

            let start = offset.min(rows.len());
            let end = (start + count).min(rows.len());
            rows = rows[start..end].to_vec();
        }

        // Apply RETURN projection (if present)
        if let Some(ref return_clause) = query.return_clause {
            let results: DbResult<Vec<Value>> = rows
                .iter()
                .map(|ctx| self.evaluate_expr_with_context(&return_clause.expression, ctx))
                .collect();
            results
        } else {
            Ok(vec![])
        }
    }
}
