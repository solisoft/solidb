//! Query explanation and profiling for SDBQL executor.
//!
//! This module contains the EXPLAIN functionality:
//! - explain: Generate query execution plan with timing information

use std::collections::HashMap;
use std::time::Instant;

use serde_json::Value;

use super::types::{CollectionAccess, Context, ExecutionTiming, FilterInfo, LetBinding, LimitInfo, QueryExplain, SortInfo};
use super::{compare_values, QueryExecutor};
use super::format_expression;
use crate::error::DbResult;
use crate::sdbql::ast::*;

impl<'a> QueryExecutor<'a> {
    pub fn explain(&self, query: &Query) -> DbResult<QueryExplain> {
        let total_start = Instant::now();
        let warnings: Vec<String> = Vec::new();
        let mut collections_info: Vec<CollectionAccess> = Vec::new();
        let mut let_bindings_info: Vec<LetBinding> = Vec::new();
        let mut filters_info: Vec<FilterInfo> = Vec::new();

        // Timing accumulators
        let mut collection_scan_us: u64 = 0;
        let mut filter_us: u64 = 0;
        let mut sort_us: u64 = 0;
        let mut limit_us: u64 = 0;
        let mut return_projection_us: u64 = 0;

        // First, evaluate all LET clauses
        let let_start = Instant::now();
        let mut initial_bindings: Context = HashMap::new();

        for (key, value) in &self.bind_vars {
            initial_bindings.insert(format!("@{}", key), value.clone());
        }

        for let_clause in &query.let_clauses {
            let clause_start = Instant::now();
            let is_subquery = matches!(let_clause.expression, Expression::Subquery(_));
            let value =
                self.evaluate_expr_with_context(&let_clause.expression, &initial_bindings)?;
            initial_bindings.insert(let_clause.variable.clone(), value);
            let clause_time = clause_start.elapsed();

            let_bindings_info.push(LetBinding {
                variable: let_clause.variable.clone(),
                is_subquery,
                time_us: clause_time.as_micros() as u64,
            });
        }
        let let_clauses_time = let_start.elapsed();
        let mut let_clauses_us = let_clauses_time.as_micros() as u64;

        // Execution Phase - Measure everything

        let mut total_docs_scanned = 0usize;
        let mut rows: Vec<Context> = vec![initial_bindings.clone()];

        // Iterate through body clauses (FOR, FILTER, etc.)
        let clauses = if !query.body_clauses.is_empty() {
            &query.body_clauses
        } else {
            // Fallback for empty body clauses (legacy path not fully instrumented here)
            &query.body_clauses
        };

        let mut i = 0;
        while i < clauses.len() {
            match &clauses[i] {
                BodyClause::For(for_clause) => {
                    let scan_start = Instant::now();

                    // Optimization: Check for Index Usage (Index Scan)
                    let mut used_index = false;
                    let mut index_name: Option<String> = None;
                    let mut index_type: Option<String> = None;

                    // Check if next clause is a FILTER that can use an index
                    if i + 1 < clauses.len() {
                        if let BodyClause::Filter(filter_clause) = &clauses[i + 1] {
                            let is_collection = if let Some(src) = &for_clause.source_variable {
                                src == &for_clause.collection
                            } else {
                                !for_clause.collection.is_empty()
                            };

                            if is_collection {
                                if let Ok(collection) = self.get_collection(&for_clause.collection)
                                {
                                    if let Some(condition) = self.extract_indexable_condition(
                                        &filter_clause.expression,
                                        &for_clause.variable,
                                    ) {
                                        if let Some(docs) =
                                            self.use_index_for_condition(&collection, &condition)
                                        {
                                            // Found index usage!
                                            used_index = true;
                                            // Identify index name
                                            for idx in collection.list_indexes() {
                                                if idx.field == condition.field {
                                                    index_name = Some(idx.name.clone());
                                                    index_type =
                                                        Some(format!("{:?}", idx.index_type));
                                                    break;
                                                }
                                            }

                                            let mut new_rows = Vec::new();
                                            for ctx in &rows {
                                                for doc in &docs {
                                                    let mut new_ctx = ctx.clone();
                                                    new_ctx.insert(
                                                        for_clause.variable.clone(),
                                                        doc.to_value(),
                                                    );
                                                    new_rows.push(new_ctx);
                                                }
                                            }
                                            rows = new_rows;
                                            total_docs_scanned += docs.len();
                                            i += 2; // Skip FOR and FILTER
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !used_index {
                        // Full Scan or Range
                        let mut new_rows = Vec::new();
                        let mut clause_docs_scanned = 0;

                        for ctx in &rows {
                            // Measure iterator creation/fetching time as part of scan
                            let docs = self.get_for_source_docs(for_clause, ctx, None)?;
                            clause_docs_scanned += docs.len();
                            for doc in docs {
                                let mut new_ctx = ctx.clone();
                                new_ctx.insert(for_clause.variable.clone(), doc);
                                new_rows.push(new_ctx);
                            }
                        }
                        rows = new_rows;
                        total_docs_scanned += clause_docs_scanned;
                        i += 1;
                    }

                    collection_scan_us += scan_start.elapsed().as_micros() as u64;

                    // Record Collection Info
                    collections_info.push(CollectionAccess {
                        name: for_clause.collection.clone(),
                        variable: for_clause.variable.clone(),
                        access_type: if used_index {
                            "index_lookup".to_string()
                        } else {
                            "full_scan".to_string()
                        },
                        index_used: index_name,
                        index_type,
                        documents_count: if used_index { 0 } else { 0 }, // Simplified
                    });
                }
                BodyClause::Filter(filter_clause) => {
                    let filter_start = Instant::now();
                    let before_count = rows.len();
                    rows.retain(|ctx| {
                        self.evaluate_filter_with_context(&filter_clause.expression, ctx)
                            .unwrap_or(false)
                    });
                    let after_count = rows.len();
                    let duration = filter_start.elapsed().as_micros() as u64;
                    filter_us += duration;

                    filters_info.push(FilterInfo {
                        expression: format_expression(&filter_clause.expression),
                        index_candidate: None,
                        can_use_index: false, // Already checked in FOR loop optimization
                        documents_before: before_count,
                        documents_after: after_count,
                        time_us: duration,
                    });
                    i += 1;
                }
                BodyClause::Let(let_clause) => {
                    let let_start = Instant::now();
                    for ctx in &mut rows {
                        let value = self.evaluate_expr_with_context(&let_clause.expression, ctx)?;
                        ctx.insert(let_clause.variable.clone(), value);
                    }
                    let_clauses_us += let_start.elapsed().as_micros() as u64;
                    i += 1;
                }
                BodyClause::Insert(_) | BodyClause::Update(_) | BodyClause::Remove(_) => {
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }

        // Apply SORT
        if let Some(sort) = &query.sort_clause {
            let sort_start = Instant::now();
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
            sort_us = sort_start.elapsed().as_micros() as u64;
        }

        // Apply LIMIT
        let mut documents_returned = rows.len();
        let mut limit_offset_val: usize = 0;
        let mut limit_count_val: usize = 0;
        if let Some(limit) = &query.limit_clause {
            let limit_start = Instant::now();
            limit_offset_val = self
                .evaluate_expr_with_context(&limit.offset, &initial_bindings)
                .ok()
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0);
            limit_count_val = self
                .evaluate_expr_with_context(&limit.count, &initial_bindings)
                .ok()
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0);

            let start = limit_offset_val.min(rows.len());
            let end = (start + limit_count_val).min(rows.len());
            rows = rows[start..end].to_vec();
            documents_returned = rows.len();
            limit_us = limit_start.elapsed().as_micros() as u64;
        }

        // Apply RETURN Projection
        if let Some(ref return_clause) = query.return_clause {
            let proj_start = Instant::now();
            for ctx in &rows {
                let _ = self.evaluate_expr_with_context(&return_clause.expression, ctx);
            }
            return_projection_us = proj_start.elapsed().as_micros() as u64;
        }

        let total_us = total_start.elapsed().as_micros() as u64;

        Ok(QueryExplain {
            collections: collections_info,
            let_bindings: let_bindings_info,
            filters: filters_info,
            sort: query.sort_clause.as_ref().map(|s| SortInfo {
                field: s
                    .fields
                    .iter()
                    .map(|(e, _)| format_expression(e))
                    .collect::<Vec<_>>()
                    .join(", "),
                direction: if s.fields.first().map(|(_, asc)| *asc).unwrap_or(true) {
                    "ASC".to_string()
                } else {
                    "DESC".to_string()
                },
                time_us: sort_us,
            }),
            limit: query.limit_clause.as_ref().map(|_l| LimitInfo {
                offset: limit_offset_val,
                count: limit_count_val,
            }),
            timing: ExecutionTiming {
                total_us,
                let_clauses_us,
                collection_scan_us,
                filter_us,
                sort_us,
                limit_us,
                return_projection_us,
            },
            documents_scanned: total_docs_scanned,
            documents_returned,
            warnings,
        })
    }

}
