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
    /// Seeds the bindings with the parent context (for correlation) and the bind
    /// variables, then dispatches to `execute_query_with_bindings` so a subquery
    /// is executed exactly like a top-level query: its `WITH` prelude, pre-FOR
    /// `LET`s and set operations all apply, and it gets the same optimizations
    /// (index-sorted SORT+LIMIT, LIMIT pushdown, direct-scan, ...).
    pub(in crate::sdbql::executor) fn execute_with_parent_context(
        &self,
        query: &Query,
        parent_ctx: &Context,
    ) -> DbResult<Vec<Value>> {
        // Start with parent context (enables correlation with outer query).
        // Bind variables stay in `self.bind_vars`; see `execute_with_stats`.
        let initial_bindings = parent_ctx.clone();

        Ok(self
            .execute_query_with_bindings(query, initial_bindings)?
            .results)
    }
}
