//! `FILTER … SORT … LIMIT` read in order from a composite index.
//!
//! `FOR v IN c FILTER v.tenant == x [AND v.archived == null …] SORT v.created
//! DESC LIMIT 25` is the list page of a multi-tenant application. Without this
//! path the executor reads every document of the tenant, sorts them all and
//! keeps 25. With an index `[tenant, archived, created]` the entries under the
//! prefix `tenant == x, archived == null` are already in `created` order, so
//! reading that range from the right end and stopping at the 25th row that
//! passes the FILTER returns the same rows.
//!
//! Same guards as the index count — the protected-collection check, row
//! policies, search views, columnar and sharded collections — and anything
//! the index cannot serve is handed back to the ordinary path.

use super::super::index_opt::{conjunction, leading_filters};
use super::super::types::{Context, MutationStats, QueryExecutionResult};
use super::super::window::contains_window_functions;
use super::super::QueryExecutor;
use crate::error::DbResult;
use crate::sdbql::ast::{BodyClause, Expression, ForClause, LetClause, Query};

/// The parts of `FOR v IN c FILTER … [LET …]* SORT v.f LIMIT …` this path
/// serves: the loop, its FILTERs, the LETs after them, the sort field and
/// direction.
struct SortPlan<'q> {
    for_clause: &'q ForClause,
    filters: Vec<&'q Expression>,
    lets: Vec<&'q LetClause>,
    sort_field: &'q str,
    ascending: bool,
}

fn sort_plan(query: &Query) -> Option<SortPlan<'_>> {
    if !query.set_operations.is_empty()
        || !query.join_clauses.is_empty()
        || query.with_clause.is_some()
        || query.window_clause.is_some()
        || query.create_stream_clause.is_some()
        || query.create_materialized_view_clause.is_some()
        || query.refresh_materialized_view_clause.is_some()
        || query.limit_clause.as_ref()?.count.is_none()
    {
        return None;
    }
    let sort = query.sort_clause.as_ref()?;
    let [(sort_expr, ascending)] = sort.fields.as_slice() else {
        return None;
    };
    let (BodyClause::For(for_clause), rest) = query.body_clauses.split_first()? else {
        return None;
    };
    if for_clause.source_expression.is_some()
        || for_clause.system_time.is_some()
        || for_clause.valid_time.is_some()
        || for_clause.options.is_some()
        || for_clause
            .source_variable
            .as_ref()
            .is_some_and(|s| s != &for_clause.collection)
    {
        return None;
    }
    let Expression::FieldAccess(base, sort_field) = sort_expr else {
        return None;
    };
    if !matches!(base.as_ref(), Expression::Variable(v) if v == &for_clause.variable) {
        return None;
    }

    // FILTERs straight after the FOR, then LETs only: a LET evaluated per row
    // can then wait until after LIMIT, as in the plain SORT + LIMIT path.
    let filters = leading_filters(&query.body_clauses, 1);
    if filters.is_empty() {
        return None;
    }
    let lets = rest[filters.len()..]
        .iter()
        .map(|c| match c {
            BodyClause::Let(l) if !contains_window_functions(&l.expression) => Some(l),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    if query
        .post_limit_lets
        .iter()
        .any(|l| contains_window_functions(&l.expression))
        || query
            .return_clause
            .as_ref()
            .is_some_and(|rc| contains_window_functions(&rc.expression))
    {
        return None;
    }
    Some(SortPlan {
        for_clause,
        filters,
        lets,
        sort_field,
        ascending: *ascending,
    })
}

impl QueryExecutor<'_> {
    /// The query's result, or `Ok(None)` when it has to be run the ordinary
    /// way.
    pub(super) fn try_index_sort_limit(
        &self,
        query: &Query,
        bindings: &Context,
    ) -> DbResult<Option<QueryExecutionResult>> {
        Ok(self
            .index_sort_limit(query, bindings)?
            .map(|(result, _)| result))
    }

    /// [`Self::try_index_sort_limit`] with the name of the index it read, for
    /// EXPLAIN.
    pub(in crate::sdbql::executor) fn index_sort_limit(
        &self,
        query: &Query,
        bindings: &Context,
    ) -> DbResult<Option<(QueryExecutionResult, String)>> {
        let Some(plan) = sort_plan(query) else {
            return Ok(None);
        };
        let name = &plan.for_clause.collection;
        // A LET-bound array of that name is iterated, not the collection.
        if bindings.contains_key(name) || self.row_policy_applies(name) {
            return Ok(None);
        }
        let Some(ref db_name) = self.database else {
            return Ok(None);
        };
        let Ok(database) = self.storage.get_database(db_name) else {
            return Ok(None);
        };
        if self.is_columnar(&database, name)
            || !matches!(self.resolve_search_view_collection(name), Ok(None))
        {
            return Ok(None);
        }
        // Also the protected-collection guard: an error here is reported by
        // the ordinary path.
        let Ok(collection) = self.get_collection(name) else {
            return Ok(None);
        };
        if collection
            .get_shard_config()
            .is_some_and(|c| c.num_shards > 0)
        {
            return Ok(None);
        }

        let var = &plan.for_clause.variable;
        let Some(filter) = conjunction(&plan.filters) else {
            return Ok(None);
        };
        let conditions: Vec<(String, serde_json::Value)> = self
            .extract_equality_conditions(&filter, var, bindings)
            .into_iter()
            .map(|c| (c.field, c.value))
            .collect();
        if conditions.is_empty() {
            return Ok(None);
        }

        let Some(limit) = query.limit_clause.as_ref() else {
            return Ok(None);
        };
        let (offset, Some(count)) = self.eval_limit(limit, bindings) else {
            return Ok(None);
        };
        let Some(want) = offset.checked_add(count) else {
            return Ok(None);
        };
        let max_scan = self.max_intermediate_rows();
        if want > max_scan {
            return Ok(None);
        }

        // Each candidate is checked against every FILTER, as the scan does
        // (an evaluation error rejects the row there too).
        let row = |doc: &crate::storage::Document| {
            let mut ctx = bindings.clone();
            ctx.insert(var.clone(), doc.to_value());
            ctx
        };
        let mut accept = |doc: &crate::storage::Document| {
            let ctx = row(doc);
            plan.filters
                .iter()
                .all(|f| self.evaluate_filter_with_context(f, &ctx).unwrap_or(false))
        };
        let Some((index, docs)) = collection.index_sorted_with_prefix(
            &conditions,
            plan.sort_field,
            plan.ascending,
            want,
            max_scan,
            &mut accept,
        ) else {
            return Ok(None);
        };
        self.check_budget(docs.len())?;

        let mut results = Vec::with_capacity(count.min(docs.len()));
        for doc in docs.iter().skip(offset) {
            let mut ctx = row(doc);
            for let_clause in plan.lets.iter().copied().chain(&query.post_limit_lets) {
                self.bind_let(let_clause, &mut ctx)?;
            }
            if let Some(ref ret) = query.return_clause {
                results.push(self.evaluate_expr_with_context(&ret.expression, &ctx)?);
            }
        }
        Ok(Some((
            QueryExecutionResult {
                results,
                mutations: MutationStats::new(),
            },
            index.name,
        )))
    }
}
