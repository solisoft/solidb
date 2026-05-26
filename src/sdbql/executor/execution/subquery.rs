//! Subquery execution for SDBQL executor.
//!
//! This module contains execute_with_parent_context for correlated subqueries.

use serde_json::Value;

use super::super::types::Context;
use super::super::QueryExecutor;
use crate::error::DbResult;
use crate::sdbql::ast::*;

impl<'a> QueryExecutor<'a> {
    /// Execute query with parent context for correlated subqueries.
    ///
    /// Builds an initial binding set seeded by the parent context (for correlation),
    /// adds bind variables, evaluates pre-FOR LET clauses, then dispatches to the
    /// shared `execute_with_initial_bindings` pipeline so the subquery benefits from
    /// the same optimizations as a top-level query (index-sorted SORT+LIMIT,
    /// LIMIT pushdown, direct-scan, etc.).
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

        Ok(self
            .execute_with_initial_bindings(query, initial_bindings)?
            .results)
    }
}
