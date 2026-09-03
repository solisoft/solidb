//! Main execution entry points for SDBQL executor.
//!
//! This module contains:
//! - execute: Main query execution
//! - execute_with_stats: Query execution with mutation statistics

use std::collections::HashMap;

use serde_json::Value;

use super::super::types::{Context, MutationStats, QueryExecutionResult};
use super::super::window::contains_window_functions;
use super::super::{QueryExecutor, ValueSet};
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

        // Bind variables are the only bindings a top-level query starts with;
        // everything else (CTEs, pre-FOR LETs) is part of the query prelude.
        let mut bindings: Context = HashMap::new();
        for (key, value) in &self.bind_vars {
            bindings.insert(format!("@{}", key), value.clone());
        }

        self.execute_query_with_bindings(query, bindings)
    }

    /// Execute a query block against a set of outer bindings.
    ///
    /// This is the single entry point that understands a *whole* query: set
    /// operations, the `WITH` prelude, pre-FOR `LET`s, and then the execution
    /// pipeline. Anything that runs a nested query block (CTE bodies, set
    /// operation operands, recursive steps, correlated subqueries) goes through
    /// here, so none of them can silently lose a clause.
    pub(in crate::sdbql::executor) fn execute_query_with_bindings(
        &self,
        query: &Query,
        bindings: Context,
    ) -> DbResult<QueryExecutionResult> {
        // Set operations: this query block is combined with further blocks
        // (`q1 UNION q2`, `q1 INTERSECT q2`, `q1 EXCEPT q2`, ...). Each operand
        // is executed independently, then combined in list order.
        if !query.set_operations.is_empty() {
            return self.execute_set_operations(query, bindings);
        }

        let mut bindings = bindings;
        self.bind_ctes(query, &mut bindings)?;
        self.bind_pre_for_lets(query, &mut bindings)?;
        self.execute_with_initial_bindings(query, bindings)
    }

    /// Evaluate this block's `WITH` clause into `bindings`.
    ///
    /// CTEs are evaluated sequentially so later ones can reference earlier ones.
    fn bind_ctes(&self, query: &Query, bindings: &mut Context) -> DbResult<()> {
        let Some(ref with_clause) = query.with_clause else {
            return Ok(());
        };

        for cte in &with_clause.ctes {
            let rows = if cte.recursive {
                self.execute_recursive_cte(cte, bindings)?
            } else {
                self.execute_query_with_bindings(&cte.query, bindings.clone())?
                    .results
            };
            tracing::debug!("CTE '{}' returned {} results", cte.name, rows.len());
            // Stored as an array so `FOR x IN cte_name` can iterate it
            bindings.insert(cte.name.clone(), Value::Array(rows));
        }

        Ok(())
    }

    /// Evaluate the `LET`s that precede the first `FOR` (evaluated once).
    fn bind_pre_for_lets(&self, query: &Query, bindings: &mut Context) -> DbResult<()> {
        for let_clause in &query.let_clauses {
            let value = self.evaluate_expr_with_context(&let_clause.expression, bindings)?;
            bindings.insert(let_clause.variable.clone(), value);
        }
        Ok(())
    }

    /// Evaluate a LIMIT clause to `(offset, count)`. A `None` count means the
    /// query has no upper bound (`OFFSET n` without `LIMIT`) — callers must not
    /// stand in a maximum, because the count reaches storage as a scan size.
    pub(in crate::sdbql::executor) fn eval_limit(
        &self,
        limit: &LimitClause,
        ctx: &Context,
    ) -> (usize, Option<usize>) {
        let eval = |expr| {
            self.evaluate_expr_with_context(expr, ctx)
                .ok()
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0)
        };
        (eval(&limit.offset), limit.count.as_ref().map(eval))
    }

    /// Execute the optimization + body-clause + sort/limit/return pipeline against a
    /// fully-built initial binding set, then apply `RETURN DISTINCT`.
    ///
    /// Deduplication lives here rather than in the pipeline because the pipeline
    /// returns from a dozen fast paths, and every one of them must honour
    /// `DISTINCT` (a columnar collection used to ignore it).
    pub(super) fn execute_with_initial_bindings(
        &self,
        query: &Query,
        initial_bindings: Context,
    ) -> DbResult<QueryExecutionResult> {
        let mut result = self.execute_pipeline(query, initial_bindings)?;
        if query.return_clause.as_ref().is_some_and(|rc| rc.distinct) {
            result.results = dedupe_values(result.results);
        }
        Ok(result)
    }

    /// The execution pipeline itself. Shared by top-level queries and the
    /// subquery executor so subqueries get the same fast paths (e.g. the
    /// `_key` index-sorted shortcut for SORT+LIMIT).
    fn execute_pipeline(
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
                            // Evaluate limit expressions. An unbounded count
                            // (`OFFSET n` with no `LIMIT`) fetches everything.
                            let (limit_offset, limit_count) =
                                self.eval_limit(limit, &initial_bindings);

                            // Check for overflow in limit_offset + limit_count
                            let max_fetch = match limit_count {
                                Some(count) => match limit_offset.checked_add(count) {
                                    Some(sum) => Some(sum),
                                    None => {
                                        return Ok(QueryExecutionResult {
                                            results: vec![],
                                            mutations: MutationStats::new(),
                                        });
                                    }
                                },
                                None => None,
                            };

                            // Check if sort expression is a simple field access on the loop variable
                            if let Expression::FieldAccess(base, field) = sort_expr {
                                if let Expression::Variable(var) = base.as_ref() {
                                    if var == &for_clause.variable {
                                        if let Ok(collection) =
                                            self.get_collection(&for_clause.collection)
                                        {
                                            // `OFFSET n` alone leaves `max_fetch` unbounded;
                                            // the ceiling bounds it instead.
                                            let max_fetch =
                                                Some(max_fetch.map_or(
                                                    self.max_intermediate_rows() + 1,
                                                    |n| n.min(self.max_intermediate_rows() + 1),
                                                ));
                                            if let Some(docs) =
                                                collection.index_sorted(field, *sort_asc, max_fetch)
                                            {
                                                self.check_budget(docs.len())?;
                                                let start = limit_offset.min(docs.len());
                                                let end = match limit_count {
                                                    Some(count) => {
                                                        start.saturating_add(count).min(docs.len())
                                                    }
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
                                        if let Some(docs) = collection.index_sorted(
                                            field,
                                            *sort_asc,
                                            self.scan_cap(),
                                        ) {
                                            self.check_budget(docs.len())?;
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
                                    let scan_limit = query
                                        .limit_clause
                                        .as_ref()
                                        .map(|l| self.eval_limit(l, &initial_bindings));

                                    // This fast path bypasses `get_for_source_docs`,
                                    // so it needs its own columnar check —
                                    // otherwise the simplest query of all,
                                    // `FOR x IN c RETURN x`, is the one shape
                                    // that still reports CollectionNotFound.
                                    let cap = self.max_intermediate_rows() + 1;
                                    let fetch = scan_limit
                                        .and_then(|(offset, count)| {
                                            count.map(|count| offset.saturating_add(count))
                                        })
                                        .map_or(cap, |n| n.min(cap));
                                    if let Some(rows) = self
                                        .columnar_source_rows(&for_clause.collection, Some(fetch))?
                                    {
                                        self.check_budget(rows.len())?;
                                        let results = match scan_limit {
                                            Some((offset, _)) => {
                                                rows.into_iter().skip(offset).collect()
                                            }
                                            None => rows,
                                        };
                                        return Ok(QueryExecutionResult {
                                            results,
                                            mutations: MutationStats::new(),
                                        });
                                    }

                                    // This fast path bypasses `get_for_source_docs`
                                    // and with it the capped scan, so it caps
                                    // its own: `FOR d IN huge RETURN d` was the
                                    // one shape that still read everything.
                                    let collection = self.get_collection(&for_clause.collection)?;
                                    let results = match scan_limit {
                                        Some((offset, count)) => collection.scan_values_range(
                                            offset,
                                            Some(count.map_or(cap, |n| n.min(cap))),
                                        ),
                                        None => collection.scan_values(self.scan_cap()),
                                    };
                                    self.check_budget(results.len())?;

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
                    let (offset, count) = self.eval_limit(l, &initial_bindings);
                    // No count means no upper bound: nothing to push down.
                    offset.checked_add(count?)
                })
            } else {
                None
            }
        } else {
            None
        };

        // Optimization: push LIMIT into the index lookup when the body is
        // exactly FOR + FILTER, nothing reorders or regroups rows afterwards,
        // and the FILTER is fully satisfied by the index condition (that last
        // part is checked per-row in the index branch, where the row context
        // needed for condition extraction is available).
        let indexed_filter_limit = if query.sort_clause.is_none()
            && query.body_clauses.len() == 2
            && matches!(query.body_clauses[0], BodyClause::For(_))
            && matches!(query.body_clauses[1], BodyClause::Filter(_))
            && query
                .return_clause
                .as_ref()
                .is_none_or(|rc| !contains_window_functions(&rc.expression))
        {
            query.limit_clause.as_ref().and_then(|l| {
                let (offset, count) = self.eval_limit(l, &initial_bindings);
                // Fetch offset+count — the LIMIT below still applies the offset
                offset.checked_add(count?)
            })
        } else {
            None
        };

        // Process body_clauses in order (supports correlated subqueries)
        // If body_clauses is empty, fall back to legacy behavior
        let (rows, mutation_stats) = if !query.body_clauses.is_empty() {
            self.execute_body_clauses(
                &query.body_clauses,
                &initial_bindings,
                scan_limit,
                indexed_filter_limit,
            )?
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
            // When a LIMIT follows and no window functions need the full
            // sorted set, only the first offset+count rows survive — use the
            // top-k path (identical output order) instead of a full sort.
            let top_k = query.limit_clause.as_ref().and_then(|limit| {
                let no_windows = query
                    .return_clause
                    .as_ref()
                    .is_none_or(|rc| !contains_window_functions(&rc.expression));
                if !no_windows {
                    return None;
                }
                let (offset, count) = self.eval_limit(limit, &initial_bindings);
                // Unbounded: every row survives, so top-k buys nothing.
                let k = offset.checked_add(count?)?;
                (k.saturating_mul(4) < rows.len()).then_some(k)
            });
            rows = match top_k {
                Some(k) => self.sort_rows_top_k(rows, &sort.fields, k),
                None => self.sort_rows(rows, &sort.fields),
            };
        }

        // Apply window functions if RETURN clause contains any
        if let Some(ref return_clause) = query.return_clause {
            if contains_window_functions(&return_clause.expression) {
                rows = self.apply_window_functions(rows, &return_clause.expression)?;
            }
        }

        // Apply LIMIT (and/or a standalone OFFSET, which has no count)
        if let Some(limit) = &query.limit_clause {
            let (offset, count) = self.eval_limit(limit, &initial_bindings);

            let start = offset.min(rows.len());
            if start > 0 {
                rows.drain(0..start);
            }
            if let Some(count) = count {
                rows.truncate(count);
            }
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

    /// Combine this query block with its set-operation operands
    /// (`UNION [ALL]` / `INTERSECT` / `EXCEPT`).
    ///
    /// The operand list is applied left to right; SQL's tighter binding for
    /// `INTERSECT` is already expressed by the parser, which nests such an
    /// operand inside the one before it.
    fn execute_set_operations(
        &self,
        query: &Query,
        bindings: Context,
    ) -> DbResult<QueryExecutionResult> {
        let mut left = query.clone();
        let ops = std::mem::take(&mut left.set_operations);

        // The `WITH` clause sits on the left block syntactically but binds for
        // the whole combined query, so evaluate it once here and share it with
        // every operand (evaluating it per operand would re-run a CTE body,
        // and re-apply its mutations).
        let mut shared = bindings;
        self.bind_ctes(&left, &mut shared)?;

        let mut left_bindings = shared.clone();
        self.bind_pre_for_lets(&left, &mut left_bindings)?;
        let mut acc = self.execute_with_initial_bindings(&left, left_bindings)?;

        for op in &ops {
            let rhs = self.execute_query_with_bindings(&op.query, shared.clone())?;
            acc.mutations.documents_inserted += rhs.mutations.documents_inserted;
            acc.mutations.documents_updated += rhs.mutations.documents_updated;
            acc.mutations.documents_removed += rhs.mutations.documents_removed;

            // Each operand is bounded on its own; the accumulation was not,
            // so `q UNION ALL q UNION ALL ...` compounded past the ceiling.
            self.check_budget(acc.results.len().saturating_add(rhs.results.len()))?;

            match op.op {
                SetOperator::UnionAll => acc.results.extend(rhs.results),
                SetOperator::Union => {
                    // Duplicates go, wherever they came from: the left side is
                    // deduplicated too, not just the incoming rows.
                    let mut seen = ValueSet::with_capacity(acc.results.len() + rhs.results.len());
                    acc.results.retain(|value| seen.insert(value));
                    for value in rhs.results {
                        if seen.insert(&value) {
                            acc.results.push(value);
                        }
                    }
                }
                SetOperator::Intersect | SetOperator::Except => {
                    let keep_when_present = op.op == SetOperator::Intersect;
                    let mut right = ValueSet::with_capacity(rhs.results.len());
                    for value in &rhs.results {
                        right.insert(value);
                    }
                    let mut seen = ValueSet::with_capacity(acc.results.len());
                    acc.results.retain(|value| {
                        right.contains(value) == keep_when_present && seen.insert(value)
                    });
                }
            }
        }

        Ok(acc)
    }

    /// Execute a recursive CTE: `WITH RECURSIVE t AS (<anchor> UNION ALL <step>)`.
    ///
    /// The anchor runs once. Then the step queries run repeatedly; inside them
    /// the CTE name is bound to the rows produced by the *previous* iteration
    /// (`FOR x IN t ...` sees the last batch). Iteration stops when a batch is
    /// empty or the safety limits are hit.
    pub(super) fn execute_recursive_cte(
        &self,
        cte: &CteClause,
        initial_bindings: &Context,
    ) -> DbResult<Vec<Value>> {
        const MAX_ITERATIONS: usize = 1000;
        const MAX_ROWS: usize = 1_000_000;

        if cte.query.set_operations.is_empty() {
            return Err(DbError::ExecutionError(format!(
                "Recursive CTE '{}' requires a body of the form \
                 `<anchor query> UNION ALL <recursive step>`",
                cte.name
            )));
        }

        // Split into the anchor (the CTE body without set operations) and the
        // iterative steps (every UNION ALL operand).
        let mut anchor = (*cte.query).clone();
        let steps = std::mem::take(&mut anchor.set_operations);
        for op in &steps {
            if op.op != SetOperator::UnionAll {
                return Err(DbError::ExecutionError(format!(
                    "Recursive CTE '{}' only supports UNION ALL between anchor and recursive steps",
                    cte.name
                )));
            }
        }

        // The anchor and the steps are full query blocks: they may carry their
        // own `WITH`, pre-FOR `LET`s and nested set operations.
        let mut accumulated = self
            .execute_query_with_bindings(&anchor, initial_bindings.clone())?
            .results;

        let mut batch = accumulated.clone();
        let mut iterations = 0usize;
        while !batch.is_empty() {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                return Err(DbError::ExecutionError(format!(
                    "Recursive CTE '{}' exceeded {} iterations (possible infinite recursion)",
                    cte.name, MAX_ITERATIONS
                )));
            }

            // Bind the CTE name to the previous batch for this iteration. The
            // batch is moved in, not cloned — it is not needed again.
            let mut ctx = initial_bindings.clone();
            ctx.insert(cte.name.clone(), Value::Array(std::mem::take(&mut batch)));

            let mut next = Vec::new();
            for step in &steps {
                let result = self.execute_query_with_bindings(&step.query, ctx.clone())?;
                next.extend(result.results);
                // Bound the batch as it grows, not after every step has run.
                self.check_budget(accumulated.len().saturating_add(next.len()))?;
            }

            if next.is_empty() {
                break;
            }
            if accumulated.len().saturating_add(next.len()) > MAX_ROWS {
                return Err(DbError::ExecutionError(format!(
                    "Recursive CTE '{}' produced more than {} rows",
                    cte.name, MAX_ROWS
                )));
            }

            // Continue iterating with the newly produced rows
            accumulated.extend(next.iter().cloned());
            batch = next;
        }

        Ok(accumulated)
    }
}

/// Remove duplicate values preserving first-occurrence order.
///
/// Row identity is `ValueSet`'s — the same value equality the `UNION()` and
/// `INTERSECTION()` array builtins use, so `1` and `1.0` are one row here too.
pub(super) fn dedupe_values(values: Vec<Value>) -> Vec<Value> {
    let mut seen = ValueSet::with_capacity(values.len());
    values.into_iter().filter(|v| seen.insert(v)).collect()
}
