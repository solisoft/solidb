//! Query explanation and profiling for SDBQL executor.
//!
//! This module contains the EXPLAIN functionality:
//! - explain: Generate query execution plan with timing information
//!
//! The plan is built from the constant-folded query (`const_fold.rs`), so the
//! FILTER text shows folded literals and the index choice matches execution.
//! Optimizer notes (folded expressions, the geo SORT rule) are reported as
//! `optimizer: …` entries in `warnings`.

use std::time::Instant;

use serde_json::Value;

use super::format_expression;
use super::index_opt::{conjunction, leading_filters, IndexHint};
use super::types::{
    CollectionAccess, Context, ExecutionTiming, FilterInfo, LetBinding, LimitInfo, QueryExplain,
    SortInfo,
};
use super::QueryExecutor;
use crate::error::DbResult;
use crate::sdbql::ast::*;

/// Subqueries nested deeper than this are not explained.
const MAX_EXPLAIN_DEPTH: usize = 4;

/// The subqueries under `expr` (not inside another subquery), each with
/// whether it is the argument of `LENGTH` / `COUNT`, which may count it from
/// an index without running it.
fn direct_subqueries(expr: &Expression, out: &mut Vec<(Query, bool)>) {
    match expr {
        Expression::FunctionCall { name, args }
            if (name == "LENGTH" || name == "COUNT")
                && args.len() == 1
                && matches!(args[0], Expression::Subquery(_)) =>
        {
            if let Expression::Subquery(q) = &args[0] {
                out.push(((**q).clone(), true));
            }
        }
        Expression::Subquery(q) => out.push(((**q).clone(), false)),
        other => other.for_each_child(&mut |child| direct_subqueries(child, out)),
    }
}

impl<'a> QueryExecutor<'a> {
    pub fn explain(&self, query: &Query) -> DbResult<QueryExplain> {
        self.explain_in(query, &Context::default(), 0)
    }

    /// The plan of `query` without running it (`analyze: false`): each loop
    /// over a collection, its subqueries' included, gets the access the
    /// planner chooses for its FILTER — values read from other rows standing
    /// in as placeholders — marked `estimated`; an index-ordered SORT is named
    /// when one applies. No document is read and nothing is written; timings
    /// and document counts are zero. The data can still change the real run:
    /// a sparse index's ordered read falls back when short of rows, a trigram
    /// read when its term is too common.
    pub fn explain_plan(&self, query: &Query) -> DbResult<QueryExplain> {
        let mut folded_query = query.clone();
        self.fold_constants(&mut folded_query);
        let query = &folded_query;
        let bindings = Context::default();
        let mut warnings = vec![
            "plan only (analyze: false): nothing was run and no document was read; every access \
             is estimated from the shape of the query"
                .to_string(),
        ];
        let mut collections = Vec::new();
        self.estimate_loops(query, None, 0, &mut collections, &mut warnings)?;

        // As the run decides it: a FILTER + SORT + LIMIT read in index order
        // (with the entries of the range it would walk), else a bare SORT.
        let (sort_index, sorted_entries) = if super::execution::has_window_functions(query) {
            (None, None)
        } else {
            match self.planned_sorted_read(query, &bindings) {
                Some((name, entries)) => (Some(name), Some(entries)),
                None => (self.bare_sort_index(query, &bindings), None),
            }
        };
        if let (Some(name), Some(first)) = (&sort_index, collections.first_mut()) {
            if first.subquery.is_none() {
                first.access_type = "index_sort".to_string();
                first.index_used = Some(name.clone());
                first.estimated_rows = sorted_entries;
            }
        }

        let filters = query
            .body_clauses
            .iter()
            .filter_map(|c| match c {
                BodyClause::Filter(f) => Some(FilterInfo {
                    expression: format_expression(&f.expression),
                    index_candidate: None,
                    can_use_index: false,
                    documents_before: 0,
                    documents_after: 0,
                    time_us: 0,
                }),
                _ => None,
            })
            .collect();
        let limit = query.limit_clause.as_ref().map(|l| {
            let (offset, count) = self.eval_limit(l, &bindings);
            LimitInfo { offset, count }
        });
        Ok(QueryExplain {
            collections,
            let_bindings: query
                .let_clauses
                .iter()
                .map(|l| LetBinding {
                    variable: l.variable.clone(),
                    is_subquery: matches!(l.expression, Expression::Subquery(_)),
                    time_us: 0,
                })
                .collect(),
            filters,
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
                time_us: 0,
                index: sort_index,
            }),
            limit,
            timing: ExecutionTiming::default(),
            documents_scanned: 0,
            documents_returned: 0,
            warnings,
            plan_only: true,
        })
    }

    /// The plans of the subqueries under `expr`, evaluated against `ctx`
    /// (the row they would see first), added to `out` labelled with `label`.
    /// A subquery that fails is reported in `warnings`, not as an error: the
    /// plan of the rest still stands.
    #[allow(clippy::too_many_arguments)]
    fn explain_subqueries(
        &self,
        expr: &Expression,
        ctx: Option<&Context>,
        label: &str,
        depth: usize,
        out: &mut Vec<CollectionAccess>,
        warnings: &mut Vec<String>,
    ) {
        let mut subs = Vec::new();
        direct_subqueries(expr, &mut subs);
        if subs.is_empty() {
            return;
        }
        if depth >= MAX_EXPLAIN_DEPTH {
            warnings.push(format!(
                "{label}: subqueries nested more than {MAX_EXPLAIN_DEPTH} deep are not explained"
            ));
            return;
        }
        let Some(ctx) = ctx else {
            warnings.push(format!(
                "{label}: no row reached the subquery; its plan is estimated from the shape of \
                 its FILTER"
            ));
            for (sub, _) in subs {
                if let Err(e) = self.estimate_loops(&sub, Some(label), depth + 1, out, warnings) {
                    warnings.push(format!("{label}: subquery not explained: {e}"));
                }
            }
            return;
        };
        for (sub, under_length) in subs {
            let counted =
                under_length && matches!(self.try_index_count_rows(&sub, ctx), Ok(Some(_)));
            match self.explain_in(&sub, ctx, depth + 1) {
                Ok(plan) => {
                    for (n, mut access) in plan.collections.into_iter().enumerate() {
                        access.subquery = Some(match access.subquery.take() {
                            Some(inner) => format!("{label} > {inner}"),
                            None => label.to_string(),
                        });
                        if counted && n == 0 {
                            access.access_type = "index_count".to_string();
                        }
                        out.push(access);
                    }
                    warnings.extend(plan.warnings.into_iter().map(|w| format!("{label}: {w}")));
                }
                Err(e) => warnings.push(format!("{label}: subquery not explained: {e}")),
            }
        }
    }

    /// The loops of a subquery no row reached, planned without running it:
    /// each `FOR` over a collection gets the access [`Self::plan_index_read`]
    /// chooses for its FILTER, values read from outer rows replaced by a
    /// placeholder, marked `estimated`. Subqueries inside it are estimated
    /// the same way.
    fn estimate_loops(
        &self,
        sub: &Query,
        label: Option<&str>,
        depth: usize,
        out: &mut Vec<CollectionAccess>,
        warnings: &mut Vec<String>,
    ) -> DbResult<()> {
        use super::index_opt::{with_placeholder_values, PlannedRead};
        let clauses = &sub.body_clauses;
        for (i, clause) in clauses.iter().enumerate() {
            let BodyClause::For(for_clause) = clause else {
                continue;
            };
            let iterates_collection = for_clause.source_expression.is_none()
                && !for_clause.collection.is_empty()
                && for_clause
                    .source_variable
                    .as_ref()
                    .is_none_or(|s| s == &for_clause.collection);
            if !iterates_collection {
                continue;
            }
            let filters = leading_filters(clauses, i + 1);
            let hint = IndexHint::of(for_clause);
            let plan = match (
                self.get_collection(&for_clause.collection),
                conjunction(&filters),
            ) {
                (Ok(collection), Some(filter))
                    if !self.row_policy_applies(&for_clause.collection) =>
                {
                    let filter = with_placeholder_values(&filter, &for_clause.variable);
                    let plan = match self.plan_index_read_hinted(
                        &collection,
                        &filter,
                        &for_clause.variable,
                        &Context::default(),
                        hint.as_ref(),
                    ) {
                        Ok(plan) => plan,
                        // A forced hint that cannot serve: the run fails at
                        // the query's own loop, and at a subquery's only if
                        // a row reaches it.
                        Err(e) => match label {
                            None => return Err(e),
                            Some(label) => {
                                warnings.push(format!("{label}: a row reaching it fails: {e}"));
                                PlannedRead::Scan
                            }
                        },
                    };
                    let used = match &plan {
                        PlannedRead::Index(name) => Some(name.as_str()),
                        _ => None,
                    };
                    if plan != PlannedRead::Maybe {
                        warnings.extend(
                            hint.as_ref()
                                .and_then(|h| h.unserved_warning(&for_clause.collection, used))
                                .map(|w| match label {
                                    Some(label) => format!("{label}: {w}"),
                                    None => w,
                                }),
                        );
                    }
                    plan
                }
                _ => PlannedRead::Scan,
            };
            let mut estimated_rows = None;
            let (access_type, index_used) = match plan {
                PlannedRead::Index(name) if hint.as_ref().is_some_and(|h| h.served_by(&name)) => {
                    ("index_lookup", Some(name)) // a hinted read was not compared
                }
                PlannedRead::Index(name) => {
                    // A count of a placeholder standing in for an outer
                    // value says nothing about the rows: only a FILTER that
                    // needed none is counted.
                    let filter = conjunction(&filters)
                        .filter(|f| with_placeholder_values(f, &for_clause.variable) == **f);
                    if let (Ok(collection), Some(filter)) =
                        (self.get_collection(&for_clause.collection), filter)
                    {
                        estimated_rows = self.chosen_read_estimate(
                            &collection,
                            &filter,
                            &for_clause.variable,
                            &Context::default(),
                            &name,
                        );
                    }
                    ("index_lookup", Some(name))
                }
                PlannedRead::Maybe => {
                    warnings.push(format!(
                        "{}: `{}` may be read from a geo index, depending on the rows",
                        label.unwrap_or("FOR"),
                        for_clause.collection
                    ));
                    ("full_scan", None)
                }
                PlannedRead::Scan => ("full_scan", None),
            };
            out.push(CollectionAccess {
                name: for_clause.collection.clone(),
                variable: for_clause.variable.clone(),
                access_type: access_type.to_string(),
                index_used,
                index_type: None,
                documents_count: 0,
                auto_index_candidate: None,
                subquery: label.map(str::to_string),
                estimated: Some(true),
                estimated_rows,
            });
        }

        // Subqueries inside it, labelled as a run would label them.
        if depth >= MAX_EXPLAIN_DEPTH {
            return Ok(());
        }
        let lets = sub
            .let_clauses
            .iter()
            .chain(clauses.iter().filter_map(|c| match c {
                BodyClause::Let(l) => Some(l),
                _ => None,
            }));
        let mut nested: Vec<(String, &Expression)> = lets
            .chain(&sub.post_limit_lets)
            .map(|l| (format!("LET {}", l.variable), &l.expression))
            .collect();
        nested.extend(clauses.iter().filter_map(|c| match c {
            BodyClause::Filter(f) => Some(("FILTER".to_string(), &f.expression)),
            _ => None,
        }));
        if let Some(ret) = &sub.return_clause {
            nested.push(("RETURN".to_string(), &ret.expression));
        }
        for (inner, expr) in nested {
            let mut subs = Vec::new();
            direct_subqueries(expr, &mut subs);
            for (q, _) in subs {
                let nested = match label {
                    Some(label) => format!("{label} > {inner}"),
                    None => inner.clone(),
                };
                if let Err(e) = self.estimate_loops(&q, Some(&nested), depth + 1, out, warnings) {
                    warnings.push(format!("{nested}: subquery not explained: {e}"));
                }
            }
        }
        Ok(())
    }

    fn explain_in(&self, query: &Query, outer: &Context, depth: usize) -> DbResult<QueryExplain> {
        let total_start = Instant::now();
        let mut warnings: Vec<String> = Vec::new();

        // Plan what execution runs: the constant-folded tree.
        let mut folded_query = query.clone();
        let folded = self.fold_constants(&mut folded_query);
        let query = &folded_query;
        // Reuse as execution does, so the plan and its timings are the work
        // the query actually does. A subquery's plan keeps the caches of the
        // query that holds it.
        if depth == 0 {
            self.reset_query_caches();
            self.caches
                .enable_reuse(self.reuse_within_query && !query.has_mutations());
        }
        if folded > 0 {
            warnings.push(format!(
                "optimizer: folded {} constant expression(s)",
                folded
            ));
        }
        let mut collections_info: Vec<CollectionAccess> = Vec::new();
        // Accesses made by subqueries, listed after the query's own loops.
        let mut subquery_accesses: Vec<CollectionAccess> = Vec::new();
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
        // Bind variables stay in `self.bind_vars`; see `execute_with_stats`.
        let mut initial_bindings = outer.clone();

        for let_clause in &query.let_clauses {
            let clause_start = Instant::now();
            let is_subquery = matches!(let_clause.expression, Expression::Subquery(_));
            self.bind_let(let_clause, &mut initial_bindings)?;
            let clause_time = clause_start.elapsed();

            let_bindings_info.push(LetBinding {
                variable: let_clause.variable.clone(),
                is_subquery,
                time_us: clause_time.as_micros() as u64,
            });
        }
        let let_clauses_time = let_start.elapsed();
        let mut let_clauses_us = let_clauses_time.as_micros() as u64;
        for let_clause in &query.let_clauses {
            self.explain_subqueries(
                &let_clause.expression,
                Some(&initial_bindings),
                &format!("LET {}", let_clause.variable),
                depth,
                &mut subquery_accesses,
                &mut warnings,
            );
        }

        // Paths that answer the whole query from an index before the
        // pipeline below runs: a count read from index entries, or rows read
        // in SORT order with an early stop.
        let counted = matches!(self.try_index_count(query, &initial_bindings), Ok(Some(_)));
        // A FILTER + SORT + LIMIT read in index order is reported from the
        // read itself: what it read, what each FILTER kept, no sort.
        let sorted_start = Instant::now();
        let mut sorted_read = if counted || super::execution::has_window_functions(query) {
            None
        } else {
            self.index_sorted_read(query, &initial_bindings)
                .ok()
                .flatten()
        };
        let sorted_read_us = sorted_start.elapsed().as_micros() as u64;
        let sort_index = if counted || super::execution::has_window_functions(query) {
            None
        } else {
            match &sorted_read {
                Some(read) => Some(read.index.clone()),
                None => self.bare_sort_index(query, &initial_bindings),
            }
        };
        let read_in_sort_order = sorted_read.is_some();
        if let Some(name) = &sort_index {
            warnings.push(format!(
                "optimizer: rows read in SORT order from index '{name}'"
            ));
        }

        // `FOR … SORT DISTANCE(…) LIMIT n` served by a geo index (P11).
        let geo_sort_index = self
            .geo_sort_candidates(query, &initial_bindings)
            .map(|(_, name)| name);
        if let Some(name) = &geo_sort_index {
            warnings.push(format!(
                "optimizer: SORT DISTANCE(...) LIMIT served by geo index '{}'",
                name
            ));
        }

        // Execution Phase - Measure everything

        let mut total_docs_scanned = 0usize;
        let mut rows: Vec<Context> = vec![initial_bindings.clone()];

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
                    let mut estimated_rows: Option<usize> = None;
                    let mut auto_index_candidate: Option<String> = None;
                    let mut walked = false;

                    // Check if the FILTERs after the FOR can use an index
                    // (all of them, as execution does).
                    let filters = leading_filters(clauses, i + 1);
                    let sorted_here = if collections_info.is_empty() {
                        sorted_read.take()
                    } else {
                        None
                    };
                    if let Some(read) = sorted_here {
                        // The first loop, read in SORT order: its rows are
                        // those the walk kept, every FILTER already checked
                        // on each document it read.
                        used_index = true;
                        walked = true;
                        index_type = Some(read.index_type);
                        estimated_rows = Some(read.entries);
                        total_docs_scanned += read.read;
                        collection_scan_us += sorted_read_us;
                        rows = read
                            .docs
                            .iter()
                            .map(|doc| {
                                let mut ctx = initial_bindings.clone();
                                ctx.insert(for_clause.variable.clone(), doc.to_value());
                                ctx
                            })
                            .collect();
                        for (filter, (before, after)) in filters.iter().zip(read.filter_counts) {
                            self.explain_subqueries(
                                filter,
                                rows.first(),
                                "FILTER",
                                depth,
                                &mut subquery_accesses,
                                &mut warnings,
                            );
                            filters_info.push(FilterInfo {
                                expression: format_expression(filter),
                                index_candidate: None,
                                can_use_index: false,
                                documents_before: before,
                                documents_after: after,
                                time_us: 0,
                            });
                        }
                        i += 1 + filters.len();
                    } else if let Some(index_filter) = conjunction(&filters) {
                        let is_collection = if let Some(src) = &for_clause.source_variable {
                            src == &for_clause.collection
                        } else {
                            !for_clause.collection.is_empty()
                        };

                        // Mirror execution: a row-policy collection is
                        // scanned, not index-read (audit H2).
                        if is_collection && !self.row_policy_applies(&for_clause.collection) {
                            if let Ok(collection) = self.get_collection(&for_clause.collection) {
                                // EXPLAIN uses the first row's context (or
                                // an empty one) — sufficient for the
                                // structural decision of whether an index
                                // would be used at runtime.
                                let probe_ctx = rows.first().cloned().unwrap_or_default();
                                // As the run reads it: the loop's indexHint
                                // first, and a forced one that cannot serve
                                // fails here as it fails there.
                                let hint = IndexHint::of(for_clause);
                                if let Some((docs, name, ty)) = self
                                    .lookup_index_for_filter_hinted(
                                        &collection,
                                        &index_filter,
                                        &for_clause.variable,
                                        &probe_ctx,
                                        None,
                                        hint.as_ref(),
                                    )?
                                {
                                    used_index = true;
                                    // A hinted read was not compared.
                                    if hint.as_ref().is_none_or(|h| !h.served_by(&name)) {
                                        estimated_rows = self.chosen_read_estimate(
                                            &collection,
                                            &index_filter,
                                            &for_clause.variable,
                                            &probe_ctx,
                                            &name,
                                        );
                                    }
                                    index_name = Some(name);
                                    index_type = Some(ty);

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
                                    // The FILTERs run next on these rows, as
                                    // the run re-checks every one of them.
                                    i += 1;
                                } else if let Some(cond) = self.extract_indexable_condition(
                                    &index_filter,
                                    &for_clause.variable,
                                    &probe_ctx,
                                ) {
                                    if self.would_auto_index(
                                        &collection,
                                        &cond.field,
                                        Some(&cond.value),
                                    ) {
                                        auto_index_candidate = Some(cond.field);
                                    }
                                }
                                warnings.extend(hint.and_then(|h| {
                                    h.unserved_warning(
                                        &for_clause.collection,
                                        index_name.as_deref(),
                                    )
                                }));
                                if !used_index {
                                    warnings.extend(self.or_scan_reasons(
                                        &collection,
                                        &index_filter,
                                        &for_clause.variable,
                                        &probe_ctx,
                                    ));
                                }
                            }
                        }
                    }

                    // The second pass execution makes when the planner
                    // leaves the stage to the scan.
                    if !used_index {
                        if let Some(stage) =
                            self.try_subquery_index_stage(for_clause, &filters, &rows, scan_limit)?
                        {
                            used_index = true;
                            index_name = Some(stage.index_name);
                            index_type = Some(stage.index_type);
                            rows = stage.rows;
                            total_docs_scanned += stage.docs_read;
                            i += 1 + filters.len();
                        }
                    }

                    if !used_index {
                        // Full Scan or Range
                        let mut new_rows = Vec::new();
                        let mut clause_docs_scanned = 0;

                        for ctx in &rows {
                            // Measure iterator creation/fetching time as part of scan
                            let docs = self.get_for_source_docs(for_clause, ctx, scan_limit)?;
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

                    // The geo SORT rule replaces the scan of the (single) FOR.
                    let geo_sorted =
                        !used_index && collections_info.is_empty() && geo_sort_index.is_some();
                    if geo_sorted {
                        index_name = geo_sort_index.clone();
                        index_type = Some("Geo".to_string());
                    }

                    // `FOR v IN NEAR(...)` already read the geo index inside
                    // the function. Name that access; an empty collection
                    // string would otherwise be reported as a full scan.
                    let near = self.near_source(for_clause);
                    let mut access_name = for_clause.collection.clone();
                    if let Some((coll, idx)) = &near {
                        if !coll.is_empty() {
                            access_name = coll.clone();
                        }
                        if let Some(idx) = idx {
                            index_name = Some(idx.clone());
                            index_type = Some("Geo".to_string());
                            warnings.push(format!("NEAR served by geo index '{idx}'"));
                        }
                    }

                    // The first loop is the one an index count or an
                    // index-ordered read answers.
                    let first = collections_info.is_empty();
                    let sorted_by_index = first && !counted && sort_index.is_some();
                    if sorted_by_index {
                        index_name = sort_index.clone();
                    }

                    // Record Collection Info
                    collections_info.push(CollectionAccess {
                        name: access_name,
                        variable: for_clause.variable.clone(),
                        access_type: if near.is_some() {
                            "geo_near".to_string()
                        } else if geo_sorted {
                            "geo_index_sort".to_string()
                        } else if first && counted {
                            "index_count".to_string()
                        } else if sorted_by_index {
                            "index_sort".to_string()
                        } else if used_index {
                            "index_lookup".to_string()
                        } else if scan_limit.is_some() {
                            "limited_scan".to_string()
                        } else {
                            "full_scan".to_string()
                        },
                        index_used: index_name,
                        index_type,
                        documents_count: 0, // Simplified
                        auto_index_candidate,
                        subquery: None,
                        estimated: None,
                        // What the FILTER's index read chose, or the
                        // entries of the range an index-ordered read walked;
                        // an index count or a bare index SORT reports none.
                        estimated_rows: if walked {
                            estimated_rows
                        } else {
                            estimated_rows
                                .filter(|_| used_index && !sorted_by_index && !(first && counted))
                        },
                    });
                }
                BodyClause::Filter(filter_clause) => {
                    self.explain_subqueries(
                        &filter_clause.expression,
                        rows.first(),
                        "FILTER",
                        depth,
                        &mut subquery_accesses,
                        &mut warnings,
                    );
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
                    self.explain_subqueries(
                        &let_clause.expression,
                        rows.first(),
                        &format!("LET {}", let_clause.variable),
                        depth,
                        &mut subquery_accesses,
                        &mut warnings,
                    );
                    let let_start = Instant::now();
                    for ctx in &mut rows {
                        self.bind_let(let_clause, ctx)?;
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

        // Apply SORT, unless the rows were read in its order.
        if let Some(sort) = query.sort_clause.as_ref().filter(|_| !read_in_sort_order) {
            let sort_start = Instant::now();
            rows = self.sort_rows(rows, &sort.fields);
            sort_us = sort_start.elapsed().as_micros() as u64;
        }

        // Apply LIMIT
        let mut documents_returned = rows.len();
        let mut limit_offset_val: usize = 0;
        let mut limit_count_val: Option<usize> = None;
        if let Some(limit) = &query.limit_clause {
            let limit_start = Instant::now();
            let (offset, count) = self.eval_limit(limit, &initial_bindings);
            limit_offset_val = offset;
            limit_count_val = count;

            let start = limit_offset_val.min(rows.len());
            let end = match limit_count_val {
                Some(count) => start.saturating_add(count).min(rows.len()),
                None => rows.len(),
            };
            rows = rows[start..end].to_vec();
            documents_returned = rows.len();
            limit_us = limit_start.elapsed().as_micros() as u64;
        }

        // LETs after LIMIT, then RETURN: their subqueries see the first row
        // left.
        for let_clause in &query.post_limit_lets {
            self.explain_subqueries(
                &let_clause.expression,
                rows.first(),
                &format!("LET {}", let_clause.variable),
                depth,
                &mut subquery_accesses,
                &mut warnings,
            );
        }
        if let Some(ref return_clause) = query.return_clause {
            let first_row = rows.first().or_else(|| {
                // A query with no loop returns one row: its bindings.
                query
                    .body_clauses
                    .iter()
                    .all(|c| !matches!(c, BodyClause::For(_)))
                    .then_some(&initial_bindings)
            });
            self.explain_subqueries(
                &return_clause.expression,
                first_row,
                "RETURN",
                depth,
                &mut subquery_accesses,
                &mut warnings,
            );
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

        collections_info.extend(subquery_accesses);
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
                index: sort_index.clone(),
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
            plan_only: false,
        })
    }

    /// `FOR v IN NEAR(collection, …, field)`: the collection name, and the
    /// geo index when both the collection and the field are a literal or a
    /// bind variable. `None` when the source is not a `NEAR` call.
    fn near_source(&self, for_clause: &ForClause) -> Option<(String, Option<String>)> {
        let Expression::FunctionCall { name, args } = for_clause.source_expression.as_ref()? else {
            return None;
        };
        if !name.eq_ignore_ascii_case("NEAR") {
            return None;
        }
        let collection = args.first().and_then(|a| self.static_string(a));
        let field = args.get(3).and_then(|a| self.static_string(a));
        let index = match (&collection, &field) {
            (Some(coll), Some(field)) => self.get_collection(coll).ok().and_then(|c| {
                c.get_all_geo_indexes()
                    .into_iter()
                    .find(|g| g.field == *field)
                    .map(|g| g.name)
            }),
            _ => None,
        };
        Some((collection.unwrap_or_default(), index))
    }

    /// A string literal, or a bind variable bound to a string.
    fn static_string(&self, expr: &Expression) -> Option<String> {
        match expr {
            Expression::Literal(Value::String(s)) => Some(s.clone()),
            Expression::BindVariable(name) => self
                .bind_vars
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_string),
            _ => None,
        }
    }
}
