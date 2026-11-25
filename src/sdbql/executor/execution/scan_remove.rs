//! `FOR x IN c [FILTER …]* REMOVE x IN c`, without a row per document.
//!
//! The ordinary pipeline read every document of the collection whole into
//! a `Context`, ran the FILTERs on those rows, evaluated the selector on
//! each survivor (a copy of the document, to take its `_key`) and handed
//! the keys to one batch delete: 90,500 documents out of 182,500 took 1.1 s,
//! most of it building, copying and dropping documents nobody read.
//!
//! Here the FILTERs are checked on the stored bytes (`typed_scan`), on
//! several threads when the collection is large enough, and only the keys
//! of the documents they keep are held. The delete is the same one
//! (`Collection::delete_batch_strict`), so index entries, foreign keys,
//! change events and the replication log are what they were.
//!
//! Taken only when the rows are exactly what the ordinary scan would hand
//! to the REMOVE: a plain local collection (the guards of
//! `scan_collect_source`, no row policy, no index hint, no `SYSTEM_TIME` /
//! valid-time), FILTERs that compile to typed predicates, the selector the
//! loop variable or its `_key`, the same collection on both sides, no
//! OPTIONS, no `OLD`, no transaction, and nothing after the REMOVE. A
//! document the typed reader cannot read is decoded and its FILTERs
//! evaluated the ordinary way. Anything else runs as before.

use super::super::types::Context;
use super::super::QueryExecutor;
use super::clauses::BUDGET_CHECK_INTERVAL;
use super::scan_collect::bind_in_place;
use super::scan_threads::Helpers;
use super::typed_scan::{Check, TypedFilters};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{BodyClause, Expression, ForClause, MutationOptions, Query, RemoveClause};
use crate::storage::serializer::deserialize_doc_as_value;
use crate::storage::Collection;
use crate::sync::Operation;

/// The parts of a query [`QueryExecutor::try_scan_remove`] runs.
struct Plan<'q> {
    for_clause: &'q ForClause,
    filters: Vec<&'q Expression>,
    remove: &'q RemoveClause,
}

/// The plan for `query`, from its shape alone.
fn plan(query: &Query) -> Option<Plan<'_>> {
    if query.needs_full_pipeline()
        || query.with_clause.is_some()
        || !query.join_clauses.is_empty()
        || query.sort_clause.is_some()
        || query.limit_clause.is_some()
        || query.return_clause.is_some()
        || query.create_stream_clause.is_some()
        || query.create_materialized_view_clause.is_some()
        || query.refresh_materialized_view_clause.is_some()
        || query.window_clause.is_some()
        || !query.post_limit_lets.is_empty()
        || !query.set_operations.is_empty()
    {
        return None;
    }
    let (first, rest) = query.body_clauses.split_first()?;
    let (last, between) = rest.split_last()?;
    let (BodyClause::For(for_clause), BodyClause::Remove(remove)) = (first, last) else {
        return None;
    };
    if for_clause.source_expression.is_some()
        || for_clause.system_time.is_some()
        || for_clause.valid_time.is_some()
        || for_clause.options.is_some()
        || for_clause.collection.is_empty()
        || for_clause
            .source_variable
            .as_ref()
            .is_some_and(|s| s != &for_clause.collection)
        || remove.collection != for_clause.collection
        || remove.options != MutationOptions::default()
        || remove.binds_old
    {
        return None;
    }
    let var = &for_clause.variable;
    let selects_row = match &remove.selector {
        Expression::Variable(v) => v == var,
        Expression::FieldAccess(base, field) => {
            field == "_key" && matches!(base.as_ref(), Expression::Variable(v) if v == var)
        }
        _ => false,
    };
    if !selects_row {
        return None;
    }
    let filters = between
        .iter()
        .map(|c| match c {
            BodyClause::Filter(f) => Some(&f.expression),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Plan {
        for_clause,
        filters,
        remove,
    })
}

impl QueryExecutor<'_> {
    /// Run `query`, a filtered scan of a collection that removes what it
    /// keeps, and return how many documents were removed; `Ok(None)` when
    /// the ordinary pipeline has to run it (see the module).
    pub(super) fn try_scan_remove(&self, query: &Query, ctx: &Context) -> DbResult<Option<usize>> {
        if self.tx_writer.is_some() || !self.stream_collect || !self.scan.typed {
            return Ok(None);
        }
        let Some(plan) = plan(query) else {
            return Ok(None);
        };
        let name = &plan.for_clause.collection;
        let var = &plan.for_clause.variable;
        if self.row_policy_applies(name) {
            return Ok(None);
        }
        let Some(source) = self.scan_collect_source(plan.for_clause, std::slice::from_ref(ctx))
        else {
            return Ok(None);
        };
        let Some(typed) = TypedFilters::compile_unsorted(self, var, &plan.filters) else {
            return Ok(None);
        };
        // Whatever the ordinary path would refuse (a protected collection),
        // it reports.
        let Ok(target) = self.get_collection_for_write(&plan.remove.collection) else {
            return Ok(None);
        };
        if target.get_shard_config().is_some_and(|c| c.num_shards > 0) {
            return Ok(None);
        }

        self.caches.count_collection_scan();
        let keys = self.keys_kept(&source, &typed, &plan, ctx)?;
        // As many rows as the ordinary path would have held.
        self.check_budget(keys.len())?;
        if keys.is_empty() {
            return Ok(Some(0));
        }
        let removed = target.delete_batch_strict(keys.clone())?;
        // Log to replication (keys only for deletes).
        for key in &keys {
            self.log_mutation(&plan.remove.collection, Operation::Delete, key, None);
        }
        Ok(Some(removed))
    }

    /// The keys of the documents of `collection` every FILTER of `plan`
    /// keeps, in key order: read on several threads at one snapshot when
    /// the collection is large enough and the engine has snapshots.
    fn keys_kept(
        &self,
        collection: &Collection,
        typed: &TypedFilters,
        plan: &Plan<'_>,
        outer: &Context,
    ) -> DbResult<Vec<String>> {
        let var = plan.for_clause.variable.as_str();
        let read = |lower: &[u8], upper: &[u8], snapshot| -> DbResult<Vec<String>> {
            let mut keys = Vec::new();
            let mut scratch: Option<Context> = None;
            let mut visited = 0usize;
            collection.for_each_doc_in(lower, upper, snapshot, |key, bytes| {
                visited += 1;
                if visited.is_multiple_of(BUDGET_CHECK_INTERVAL) {
                    self.check_budget(keys.len())?;
                }
                let kept = match typed.check(bytes) {
                    Check::Pass => true,
                    Check::Fail | Check::Stop => false,
                    // Not readable there: decoded and filtered as a row. One
                    // that does not decode is passed over, as the scan does.
                    Check::Fallback => match deserialize_doc_as_value(bytes) {
                        Ok(doc) => {
                            let row = scratch.get_or_insert_with(|| outer.clone());
                            bind_in_place(row, var, doc);
                            plan.filters.iter().all(|f| self.filter_row(f, row))
                        }
                        Err(_) => false,
                    },
                };
                if kept {
                    keys.push(String::from_utf8_lossy(key).into_owned());
                }
                Ok(true)
            })?;
            Ok(keys)
        };

        let wanted = if collection.supports_parallel_scan() {
            self.scan.wanted(collection.count())
        } else {
            1
        };
        let helpers = Helpers::take(wanted.saturating_sub(1));
        let snapshot = if helpers.count() > 0 {
            collection.scan_snapshot()
        } else {
            None
        };
        let ranges = collection.doc_partitions_at(
            if snapshot.is_some() {
                helpers.count() + 1
            } else {
                1
            },
            snapshot.as_ref(),
        );
        if ranges.len() < 2 {
            let (lower, upper) = &ranges[0];
            return read(lower, upper, None);
        }
        let results: Vec<DbResult<Vec<String>>> = std::thread::scope(|s| {
            let (read, snapshot) = (&read, snapshot.as_ref());
            let others: Vec<_> = ranges[1..]
                .iter()
                .map(|(lower, upper)| {
                    std::thread::Builder::new()
                        .name("solidb-scan".to_string())
                        .spawn_scoped(s, move || read(lower, upper, snapshot))
                })
                .collect();
            let mut results = vec![read(&ranges[0].0, &ranges[0].1, snapshot)];
            for handle in others {
                results.push(match handle {
                    Ok(h) => h.join().unwrap_or_else(|_| {
                        Err(DbError::InternalError("a scan thread panicked".to_string()))
                    }),
                    // No thread to be had: read that range here.
                    Err(_) => {
                        let (lower, upper) = &ranges[results.len()];
                        read(lower, upper, snapshot)
                    }
                });
            }
            results
        });
        drop(helpers);
        let mut keys = Vec::new();
        for result in results {
            keys.append(&mut result?);
        }
        Ok(keys)
    }
}
