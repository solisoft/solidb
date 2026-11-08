//! `FILTER … SORT … LIMIT` read in SORT order from an index.
//!
//! `FOR v IN c FILTER v.tenant == x [AND v.archived == null …] SORT v.created
//! DESC LIMIT 25` is the list page of a multi-tenant application. Without this
//! path the executor reads every document of the tenant, sorts them all and
//! keeps 25. With an index `[tenant, archived, created]` the entries under the
//! prefix `tenant == x, archived == null` are already in `created` order, so
//! reading that range from the right end and stopping at the 25th row that
//! passes the FILTER returns the same rows.
//!
//! `FOR o IN orders FILTER o.created_at >= @lo AND o.created_at < @hi SORT
//! o.created_at DESC LIMIT 20` is the same read with nothing fixed: a
//! single-field index on `created_at` is walked over the window `[lo, hi)`
//! from its upper end. Range conditions on the sort field bound the walk of a
//! compound index too. [`QueryExecutor::plan_sorted_read`] decides which
//! index, if any, can be trusted with the query.
//!
//! Same guards as the index count — the protected-collection check, row
//! policies, search views, columnar and sharded collections — and anything
//! the index cannot serve is handed back to the ordinary path.

use super::super::index_opt::{conjunction, leading_filters, SortedReadPlan};
use super::super::types::{Context, MutationStats, QueryExecutionResult};
use super::super::window::contains_window_functions;
use super::super::QueryExecutor;
use crate::error::DbResult;
use crate::sdbql::ast::{BodyClause, Expression, ForClause, LetClause, Query};
use crate::storage::{Collection, Document};

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

/// An index-ordered read as it ran, for EXPLAIN.
pub(in crate::sdbql::executor) struct SortedRead {
    /// The index walked, and its type.
    pub index: String,
    pub index_type: String,
    /// The rows kept, in SORT order: the LIMIT's offset and count at most.
    pub docs: Vec<Document>,
    /// The documents read from the index's range.
    pub read: usize,
    /// For each FILTER after the FOR, in order: the rows it was evaluated on
    /// and the rows that passed it.
    pub filter_counts: Vec<(usize, usize)>,
    /// The index entries in the range walked — what the walk could read at
    /// most — counted up to one past the row ceiling.
    pub entries: usize,
}

/// What [`QueryExecutor::sorted_read_target`] finds: the query's parts, the
/// collection, the read planned and the LIMIT.
struct SortedTarget<'q> {
    plan: SortPlan<'q>,
    collection: Collection,
    read: SortedReadPlan,
    offset: usize,
    count: usize,
}

impl QueryExecutor<'_> {
    /// The query's result, or `Ok(None)` when it has to be run the ordinary
    /// way.
    pub(super) fn try_index_sort_limit(
        &self,
        query: &Query,
        bindings: &Context,
    ) -> DbResult<Option<QueryExecutionResult>> {
        let Some(target) = self.sorted_read_target(query, bindings) else {
            return Ok(None);
        };
        let offset = target.offset;
        let Some((plan, docs, _)) = self.walk_sorted(target, bindings)? else {
            return Ok(None);
        };
        let var = &plan.for_clause.variable;
        let mut results = Vec::with_capacity(docs.len().saturating_sub(offset));
        for doc in docs.iter().skip(offset) {
            let mut ctx = bindings.clone();
            ctx.insert(var.clone(), doc.to_value());
            for let_clause in plan.lets.iter().copied().chain(&query.post_limit_lets) {
                self.bind_let(let_clause, &mut ctx)?;
            }
            if let Some(ref ret) = query.return_clause {
                results.push(self.evaluate_expr_with_context(&ret.expression, &ctx)?);
            }
        }
        Ok(Some(QueryExecutionResult {
            results,
            mutations: MutationStats::new(),
        }))
    }

    /// The index-ordered read of `query` as [`Self::try_index_sort_limit`]
    /// makes it, with what it read, what each FILTER kept and the entries of
    /// the range it walked; `Ok(None)` when the query takes the ordinary
    /// path.
    pub(in crate::sdbql::executor) fn index_sorted_read(
        &self,
        query: &Query,
        bindings: &Context,
    ) -> DbResult<Option<SortedRead>> {
        let Some(target) = self.sorted_read_target(query, bindings) else {
            return Ok(None);
        };
        let cap = self.max_intermediate_rows().saturating_add(1);
        let entries = target.read.entries(&target.collection, cap);
        let index = target.read.index.name.clone();
        let index_type = format!("{:?}", target.read.index.index_type);
        Ok(self
            .walk_sorted(target, bindings)?
            .map(|(_, docs, (read, filter_counts))| SortedRead {
                index,
                index_type,
                docs,
                read,
                filter_counts,
                entries,
            }))
    }

    /// For a plan-only EXPLAIN: the index [`Self::index_sorted_read`] would
    /// walk and the entries in its range, decided without reading a
    /// document. The run can still fall back (a sort value that is an array
    /// or object, more entries than the row ceiling).
    pub(in crate::sdbql::executor) fn planned_sorted_read(
        &self,
        query: &Query,
        bindings: &Context,
    ) -> Option<(String, usize)> {
        let target = self.sorted_read_target(query, bindings)?;
        let cap = self.max_intermediate_rows().saturating_add(1);
        let entries = target.read.entries(&target.collection, cap);
        Some((target.read.index.name, entries))
    }

    /// Everything that decides whether `query` is read in index order — its
    /// shape, the collection's guards, the LIMIT and the index — with no
    /// document read (the planner may count index entries).
    fn sorted_read_target<'q>(
        &self,
        query: &'q Query,
        bindings: &Context,
    ) -> Option<SortedTarget<'q>> {
        let plan = sort_plan(query)?;
        let name = &plan.for_clause.collection;
        // A LET-bound array of that name is iterated, not the collection.
        if bindings.contains_key(name) || self.row_policy_applies(name) {
            return None;
        }
        let db_name = self.database.as_ref()?;
        let database = self.storage.get_database(db_name).ok()?;
        if self.is_columnar(&database, name)
            || !matches!(self.resolve_search_view_collection(name), Ok(None))
        {
            return None;
        }
        // Also the protected-collection guard: an error here is reported by
        // the ordinary path.
        let collection = self.get_collection(name).ok()?;
        if collection
            .get_shard_config()
            .is_some_and(|c| c.num_shards > 0)
        {
            return None;
        }

        let (offset, Some(count)) = self.eval_limit(query.limit_clause.as_ref()?, bindings) else {
            return None;
        };
        let want = offset.checked_add(count)?;
        if want > self.max_intermediate_rows() {
            return None;
        }

        let filter = conjunction(&plan.filters)?;
        let read = self.plan_sorted_read(
            &collection,
            &filter,
            &plan.for_clause.variable,
            bindings,
            plan.sort_field,
        )?;
        Some(SortedTarget {
            plan,
            collection,
            read,
            offset,
            count,
        })
    }

    /// Walk the planned index range, keeping the first `offset + count` rows
    /// that pass every FILTER: the query's parts, those rows, the documents
    /// read and what each FILTER kept. `Ok(None)` when the walk gives up.
    #[allow(clippy::type_complexity)]
    fn walk_sorted<'q>(
        &self,
        target: SortedTarget<'q>,
        bindings: &Context,
    ) -> DbResult<Option<(SortPlan<'q>, Vec<Document>, (usize, Vec<(usize, usize)>))>> {
        let SortedTarget {
            plan,
            collection,
            read,
            offset,
            count,
        } = target;
        let var = &plan.for_clause.variable;

        // Each candidate is checked against every FILTER, in order, as the
        // scan does (an evaluation error rejects the row there too).
        let mut filter_counts = vec![(0usize, 0usize); plan.filters.len()];
        let mut accept = |doc: &Document| {
            let mut ctx = bindings.clone();
            ctx.insert(var.clone(), doc.to_value());
            for (filter, (checked, kept)) in plan.filters.iter().zip(filter_counts.iter_mut()) {
                *checked += 1;
                if !self
                    .evaluate_filter_with_context(filter, &ctx)
                    .unwrap_or(false)
                {
                    return false;
                }
                *kept += 1;
            }
            true
        };
        let (lower, upper) = read.bounds();
        let Some((docs, scanned)) = collection.index_sorted_range(
            &read.index,
            &read.eq_values,
            lower,
            upper,
            plan.ascending,
            offset + count,
            self.max_intermediate_rows(),
            &mut accept,
        ) else {
            return Ok(None);
        };
        self.check_budget(docs.len())?;
        Ok(Some((plan, docs, (scanned, filter_counts))))
    }
}
