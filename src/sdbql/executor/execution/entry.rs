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
use crate::error::DbResult;
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

        // Evaluate CTEs (Common Table Expressions) and store results in initial_bindings
        // CTEs are evaluated sequentially so later CTEs can reference earlier ones
        if let Some(ref with_clause) = query.with_clause {
            for cte in &with_clause.ctes {
                // Execute CTE query with access to previously computed CTEs
                let cte_results = self.execute_cte_with_context(&cte.query, &initial_bindings)?;
                tracing::debug!("CTE '{}' returned {} results", cte.name, cte_results.len());
                // Store CTE results as an array in the context so FOR ... IN cte_name can iterate
                initial_bindings.insert(cte.name.clone(), Value::Array(cte_results));
            }
        }

        for let_clause in &query.let_clauses {
            let value =
                self.evaluate_expr_with_context(&let_clause.expression, &initial_bindings)?;
            initial_bindings.insert(let_clause.variable.clone(), value);
        }

        self.execute_with_initial_bindings(query, initial_bindings)
    }

    /// Execute the optimization + body-clause + sort/limit/return pipeline against a
    /// fully-built initial binding set. Shared by top-level `execute_with_stats` and
    /// the subquery executor so that subqueries get the same fast paths
    /// (e.g. the `_key` index-sorted shortcut for SORT+LIMIT).
    pub(super) fn execute_with_initial_bindings(
        &self,
        query: &Query,
        initial_bindings: Context,
    ) -> DbResult<QueryExecutionResult> {
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
        // Pattern: FOR var IN collection [LET v = ...]* SORT var.field LIMIT n RETURN ...
        // The body must be a single FOR (on a real collection) optionally followed by
        // LET clauses. LET clauses are evaluated *after* limiting so a per-row
        // correlated subquery (e.g. `LET org = (FOR ... FILTER ... RETURN ...)`)
        // runs at most `offset + count` times instead of once per document.
        if let (Some(sort), Some(limit)) = (&query.sort_clause, &query.limit_clause) {
            if sort.fields.len() == 1 {
                let body = query.body_clauses.as_slice();
                let head_is_for = matches!(body.first(), Some(BodyClause::For(_)));
                let tail_is_lets = body.iter().skip(1).all(|c| matches!(c, BodyClause::Let(_)));
                if head_is_for && tail_is_lets {
                    if let Some(BodyClause::For(for_clause)) = body.first() {
                        // Only when the FOR iterates a collection (no source_expression)
                        // and matches against a real collection name (not a LET-bound array).
                        let iterates_collection = for_clause.source_expression.is_none()
                            && for_clause
                                .source_variable
                                .as_ref()
                                .is_none_or(|s| s == &for_clause.collection)
                            && !initial_bindings.contains_key(&for_clause.collection);

                        let (sort_expr, sort_asc) = &sort.fields[0];
                        if iterates_collection {
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

                            // Check for overflow in limit_offset + limit_count
                            let max_fetch = match limit_offset.checked_add(limit_count) {
                                Some(sum) => sum,
                                None => {
                                    return Ok(QueryExecutionResult {
                                        results: vec![],
                                        mutations: MutationStats::new(),
                                    });
                                }
                            };

                            // Check if sort expression is a simple field access on the loop variable
                            if let Expression::FieldAccess(base, field) = sort_expr {
                                if let Expression::Variable(var) = base.as_ref() {
                                    if var == &for_clause.variable {
                                        if let Ok(collection) =
                                            self.get_collection(&for_clause.collection)
                                        {
                                            if let Some(docs) = collection.index_sorted(
                                                field,
                                                *sort_asc,
                                                Some(max_fetch),
                                            ) {
                                                let start = limit_offset.min(docs.len());
                                                let end = match start.checked_add(limit_count) {
                                                    Some(sum) => sum.min(docs.len()),
                                                    None => docs.len(),
                                                };
                                                let docs = &docs[start..end];

                                                // Collect trailing LET clauses (already
                                                // validated above as the only non-FOR body
                                                // entries).
                                                let lets: Vec<&LetClause> = body
                                                    .iter()
                                                    .skip(1)
                                                    .filter_map(|c| match c {
                                                        BodyClause::Let(l) => Some(l),
                                                        _ => None,
                                                    })
                                                    .collect();

                                                let results = if let Some(ref return_clause) =
                                                    query.return_clause
                                                {
                                                    let results: DbResult<Vec<Value>> = docs
                                                        .iter()
                                                        .map(|doc| {
                                                            let mut ctx = initial_bindings.clone();
                                                            ctx.insert(
                                                                for_clause.variable.clone(),
                                                                doc.to_value(),
                                                            );
                                                            for let_clause in &lets {
                                                                let v = self
                                                                    .evaluate_expr_with_context(
                                                                        &let_clause.expression,
                                                                        &ctx,
                                                                    )?;
                                                                ctx.insert(
                                                                    let_clause.variable.clone(),
                                                                    v,
                                                                );
                                                            }
                                                            self.evaluate_expr_with_context(
                                                                &return_clause.expression,
                                                                &ctx,
                                                            )
                                                        })
                                                        .collect();
                                                    results?
                                                } else {
                                                    vec![]
                                                };
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
            }
        }

        // Optimization: Use index for SORT (without LIMIT) if available
        // Check if query is: FOR var IN collection SORT var.field RETURN ...
        if let Some(sort) = &query.sort_clause {
            if query.limit_clause.is_none() && query.body_clauses.len() == 1 {
                if let Some(BodyClause::For(for_clause)) = query.body_clauses.first() {
                    if sort.fields.len() == 1 {
                        let (sort_expr, sort_asc) = &sort.fields[0];

                        if let Expression::FieldAccess(base, field) = sort_expr {
                            if let Expression::Variable(var) = base.as_ref() {
                                if var == &for_clause.variable {
                                    if let Ok(collection) =
                                        self.get_collection(&for_clause.collection)
                                    {
                                        if let Some(docs) =
                                            collection.index_sorted(field, *sort_asc, None)
                                        {
                                            let results = if let Some(ref return_clause) =
                                                query.return_clause
                                            {
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
                                                docs.iter().map(|doc| doc.to_value()).collect()
                                            };
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
        }

        // Optimization: Direct scan for simple FOR + [LIMIT] + RETURN var
        // Pattern: FOR var IN collection [LIMIT n] RETURN var
        // Skips the entire Context/HashMap machinery
        if query.body_clauses.len() == 1
            && query.sort_clause.is_none()
            && query.let_clauses.is_empty()
        {
            if let Some(BodyClause::For(for_clause)) = query.body_clauses.first() {
                if for_clause.source_expression.is_none() {
                    if let Some(ref return_clause) = query.return_clause {
                        if let Expression::Variable(ref var) = return_clause.expression {
                            if var == &for_clause.variable {
                                let source_name = for_clause
                                    .source_variable
                                    .as_ref()
                                    .unwrap_or(&for_clause.collection);

                                if !initial_bindings.contains_key(source_name) {
                                    let scan_limit = query.limit_clause.as_ref().map(|l| {
                                        let offset = self
                                            .evaluate_expr_with_context(
                                                &l.offset,
                                                &initial_bindings,
                                            )
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
                                        (offset, count)
                                    });

                                    let collection = self.get_collection(&for_clause.collection)?;
                                    let results = if let Some((offset, count)) = scan_limit {
                                        collection.scan_values_range(offset, Some(count))
                                    } else {
                                        collection.scan_values(None)
                                    };

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
                query.limit_clause.as_ref().and_then(|l| {
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
                    offset.checked_add(count)
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
            if start > 0 {
                rows.drain(0..start);
            }
            rows.truncate(end - start);
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

    /// Execute a CTE query with access to a provided context (for CTE chaining)
    fn execute_cte_with_context(
        &self,
        query: &Query,
        initial_bindings: &Context,
    ) -> DbResult<Vec<Value>> {
        // CTEs don't support nested CTEs, so we just use the provided context
        // Evaluate LET clauses first
        let mut ctx = initial_bindings.clone();
        for let_clause in &query.let_clauses {
            let value = self.evaluate_expr_with_context(&let_clause.expression, &ctx)?;
            ctx.insert(let_clause.variable.clone(), value);
        }

        // Execute body clauses
        let (rows, _) = self.execute_body_clauses(&query.body_clauses, &ctx, None)?;

        // Apply RETURN projection
        let results = if let Some(ref return_clause) = query.return_clause {
            let results: DbResult<Vec<Value>> = rows
                .iter()
                .map(|r| self.evaluate_expr_with_context(&return_clause.expression, r))
                .collect();
            results?
        } else {
            vec![]
        };

        Ok(results)
    }
}
