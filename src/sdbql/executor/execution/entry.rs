//! Main execution entry points for SDBQL executor.
//!
//! This module contains:
//! - execute: Main query execution
//! - execute_with_stats: Query execution with mutation statistics

use std::collections::HashMap;

use serde_json::Value;

use super::super::types::{Context, MutationStats, QueryExecutionResult};
use super::super::window::contains_window_functions;
use super::super::{compare_values, QueryExecutor};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::*;

impl<'a> QueryExecutor<'a> {
    /// Execute query and return results
    pub fn execute(&self, query: &Query) -> DbResult<Vec<Value>> {
        let result = self.execute_with_stats(query)?;
        Ok(result.results)
    }

    /// Execute query and return full results with mutation statistics
    pub fn execute_with_stats(&self, query: &Query) -> DbResult<QueryExecutionResult> {
        // Handle CREATE MATERIALIZED VIEW
        if let Some(ref clause) = query.create_materialized_view_clause {
            return self.execute_create_materialized_view(clause);
        }

        // Handle REFRESH MATERIALIZED VIEW
        if let Some(ref clause) = query.refresh_materialized_view_clause {
            return self.execute_refresh_materialized_view(clause);
        }

        // First, evaluate initial LET clauses (before any FOR) to create initial binding
        let mut initial_bindings: Context = HashMap::new();

        // Merge bind variables into initial context
        for (key, value) in &self.bind_vars {
            initial_bindings.insert(format!("@{}", key), value.clone());
        }

        for let_clause in &query.let_clauses {
            let value =
                self.evaluate_expr_with_context(&let_clause.expression, &initial_bindings)?;
            initial_bindings.insert(let_clause.variable.clone(), value);
        }

        // Optimization: Streaming bulk INSERT for range-based FOR loops
        // Pattern: FOR i IN start..end INSERT {...} INTO collection [RETURN ...]
        // This avoids materializing millions of row contexts in memory
        if let Some((results, insert_count)) =
            self.try_streaming_bulk_insert(query, &initial_bindings)?
        {
            return Ok(QueryExecutionResult {
                results,
                mutations: MutationStats {
                    documents_inserted: insert_count,
                    documents_updated: 0,
                    documents_removed: 0,
                },
            });
        }

        // Optimization: Columnar aggregation queries
        // Pattern: FOR x IN columnar_collection COLLECT AGGREGATE ... RETURN ...
        if let Some(results) = self.try_columnar_aggregation(query, &initial_bindings)? {
            return Ok(QueryExecutionResult {
                results,
                mutations: MutationStats::new(),
            });
        }

        // Optimization: Use index for SORT + LIMIT if available
        // Check if query is: FOR var IN collection SORT var.field LIMIT n RETURN ...
        if let (Some(sort), Some(limit)) = (&query.sort_clause, &query.limit_clause) {
            // Check if we have a simple FOR loop on a collection
            // Only optimize single field sort for now
            if query.body_clauses.len() == 1 && sort.fields.len() == 1 {
                if let Some(BodyClause::For(for_clause)) = query.body_clauses.first() {
                    let (sort_expr, sort_asc) = &sort.fields[0];

                    // Evaluate limit expressions
                    let limit_offset = self
                        .evaluate_expr_with_context(&limit.offset, &initial_bindings)
                        .ok()
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .unwrap_or(0);
                    let limit_count = self
                        .evaluate_expr_with_context(&limit.count, &initial_bindings)
                        .ok()
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .unwrap_or(0);

                    // Check if the sort field is on the loop variable
                    // Check if sort expression is a simple field access on the loop variable
                    if let Expression::FieldAccess(base, field) = sort_expr {
                        if let Expression::Variable(var) = base.as_ref() {
                            if var == &for_clause.variable {
                                // Try to get collection and check for index
                                if let Ok(collection) = self.get_collection(&for_clause.collection)
                                {
                                    if let Some(docs) = collection.index_sorted(
                                        field,
                                        *sort_asc,
                                        Some(limit_offset + limit_count),
                                    ) {
                                        // Got sorted documents from index! Apply offset and build result
                                        let start = limit_offset.min(docs.len());
                                        let end = (start + limit_count).min(docs.len());
                                        let docs = &docs[start..end];

                                        let results =
                                            if let Some(ref return_clause) = query.return_clause {
                                                let results: DbResult<Vec<Value>> = docs
                                                    .iter()
                                                    .map(|doc| {
                                                        let mut ctx = initial_bindings.clone();
                                                        ctx.insert(
                                                            for_clause.variable.clone(),
                                                            doc.to_value(),
                                                        );
                                                        self.evaluate_expr_with_context(
                                                            &return_clause.expression,
                                                            &ctx,
                                                        )
                                                    })
                                                    .collect();
                                                results?
                                            } else {
                                                // No RETURN clause - return empty array
                                                vec![]
                                            };
                                        // Index-sorted optimization is read-only, no mutations
                                        return Ok(QueryExecutionResult {
                                            results,
                                            mutations: MutationStats::new(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Optimization: Check if we can push LIMIT down to storage scan
        let scan_limit = if query.sort_clause.is_none() {
            let for_count = query
                .body_clauses
                .iter()
                .filter(|c| matches!(c, BodyClause::For(_)))
                .count();
            let filter_count = query
                .body_clauses
                .iter()
                .filter(|c| matches!(c, BodyClause::Filter(_)))
                .count();

            if for_count == 1 && filter_count == 0 {
                query.limit_clause.as_ref().map(|l| {
                    let offset = self
                        .evaluate_expr_with_context(&l.offset, &initial_bindings)
                        .ok()
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .unwrap_or(0);
                    let count = self
                        .evaluate_expr_with_context(&l.count, &initial_bindings)
                        .ok()
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .unwrap_or(0);
                    offset + count
                })
            } else {
                None
            }
        } else {
            None
        };

        // Process body_clauses in order (supports correlated subqueries)
        // If body_clauses is empty, fall back to legacy behavior
        let (rows, mutation_stats) = if !query.body_clauses.is_empty() {
            self.execute_body_clauses(&query.body_clauses, &initial_bindings, scan_limit)?
        } else {
            // Legacy path: use for_clauses and filter_clauses separately
            let mut rows =
                self.build_row_combinations_with_context(&query.for_clauses, &initial_bindings)?;
            for filter in &query.filter_clauses {
                rows.retain(|ctx| {
                    self.evaluate_filter_with_context(&filter.expression, ctx)
                        .unwrap_or(false)
                });
            }
            (rows, MutationStats::new())
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

        // Apply window functions if RETURN clause contains any
        if let Some(ref return_clause) = query.return_clause {
            if contains_window_functions(&return_clause.expression) {
                rows = self.apply_window_functions(rows, &return_clause.expression)?;
            }
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
        let results = if let Some(ref return_clause) = query.return_clause {
            let results: DbResult<Vec<Value>> = rows
                .iter()
                .map(|ctx| self.evaluate_expr_with_context(&return_clause.expression, ctx))
                .collect();
            results?
        } else {
            // No RETURN clause - return empty array (mutations don't need to return anything)
            vec![]
        };

        Ok(QueryExecutionResult {
            results,
            mutations: mutation_stats,
        })
    }

}
