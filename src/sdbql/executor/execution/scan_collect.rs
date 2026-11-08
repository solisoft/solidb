//! A full-scan `COLLECT`, folded while the collection is read.
//!
//! `FOR x IN c [FILTER … | LET …]* COLLECT …` read the whole collection into
//! rows — one `Context` and one fully decoded document per document — and
//! only then filtered them and folded them into groups. Memory grew with the
//! collection, not with the result: 100,000 orders of 500 bytes were enough
//! for a 1 GB container to be OOM-killed answering a five-group query, and a
//! collection past the row ceiling could not be aggregated at all.
//!
//! Here each document is decoded, run through the FILTER and LET clauses,
//! folded into its group ([`CollectFold`], the fold the materialising path
//! uses too) and dropped before the next one is read. Memory is O(groups)
//! — plus whatever `INTO` or a list aggregate keeps, which the row ceiling
//! still bounds — and the ceiling no longer counts documents that were never
//! held. When the clauses only read fields of the loop variable, only those
//! fields are decoded (`storage::raw_rows::project`).
//!
//! Taken only when the rows are exactly what the ordinary scan would produce:
//! a plain local collection (not columnar, not a search view, not sharded —
//! shards are read by scatter-gather — and no `SYSTEM_TIME` / valid-time
//! filter), between the FOR and the COLLECT nothing but FILTER and LET, none
//! of them writing (a write in the middle of a scan could be seen by it) and
//! none using a window function (which needs every row at once). A row
//! policy is applied per document, as the scan applies it. Anything else
//! runs the ordinary way. `QueryExecutor::with_streaming_collect(false)`
//! turns the path off, so tests can compare the two.
//!
//! Two accelerations on top (#776), neither of which can change an answer:
//!
//! - When the keys, aggregate arguments and FILTERs only read fields of the
//!   loop variable, they are read from the stored bytes (`typed_scan`): no
//!   document is decoded, and one that joins an existing group allocates
//!   nothing. A document that reader cannot read is decoded and evaluated
//!   the ordinary way, into the same fold.
//! - Such a fold over a large collection is split into key ranges read on
//!   several threads at one snapshot and merged (`fold_partitions`),
//!   when the engine has snapshots (RocksDB; solistore 0.2 does not) and
//!   every aggregate merges exactly. `with_typed_collect` and
//!   `with_scan_threads` turn them off for tests.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde_json::Value;

use super::super::types::Context;
use super::super::{contains_window_functions, QueryExecutor};
use super::clauses::BUDGET_CHECK_INTERVAL;
use super::collect::CollectFold;
use super::scan_threads::Helpers;
use super::typed_scan::{Read, TypedPlan, TypedReader};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{expression_mutates, BodyClause, CollectClause, ForClause, Query};
use crate::storage::serializer::{deserialize_doc_as_value, deserialize_doc_projected};
use crate::storage::Collection;

/// The body clauses a streaming COLLECT replaces: the loop, the FILTER and
/// LET clauses up to the COLLECT, and the COLLECT.
struct Plan<'q> {
    for_clause: &'q ForClause,
    /// The FILTER and LET clauses between the FOR and the COLLECT, in order.
    stages: &'q [BodyClause],
    collect: &'q CollectClause,
    /// The loop variable's fields those clauses read; `None` when they need
    /// the whole document.
    fields: Option<Vec<String>>,
}

/// What a streaming COLLECT did, for the pipeline and for EXPLAIN.
pub(in crate::sdbql::executor) struct ScanCollectRun {
    /// One row per group, as the COLLECT returns them.
    pub(in crate::sdbql::executor) rows: Vec<Context>,
    /// How many body clauses this replaced, from the FOR to the COLLECT.
    pub(in crate::sdbql::executor) consumed: usize,
    /// Documents read.
    pub(in crate::sdbql::executor) scanned: usize,
    /// For each FILTER folded in, in order: the rows it saw, and kept.
    pub(in crate::sdbql::executor) filters: Vec<(usize, usize)>,
    /// What EXPLAIN says about it.
    pub(in crate::sdbql::executor) note: String,
}

/// The plan for `clauses[i..]`, from the shape of the query alone.
fn plan(clauses: &[BodyClause], i: usize) -> Option<Plan<'_>> {
    let BodyClause::For(for_clause) = clauses.get(i)? else {
        return None;
    };
    if for_clause.source_expression.is_some()
        || for_clause.system_time.is_some()
        || for_clause.valid_time.is_some()
        || for_clause.collection.is_empty()
        || for_clause
            .source_variable
            .as_ref()
            .is_some_and(|s| s != &for_clause.collection)
    {
        return None;
    }
    let rest = &clauses[i + 1..];
    let k = rest
        .iter()
        .position(|c| !matches!(c, BodyClause::Filter(_) | BodyClause::Let(_)))?;
    let BodyClause::Collect(collect) = &rest[k] else {
        return None;
    };
    let stages = &rest[..k];
    let stage_exprs = stages.iter().map(|c| match c {
        BodyClause::Filter(f) => &f.expression,
        BodyClause::Let(l) => &l.expression,
        _ => unreachable!("only FILTER and LET are taken above"),
    });
    let collect_exprs = collect
        .group_vars
        .iter()
        .map(|(_, e)| e)
        .chain(
            collect
                .aggregates
                .iter()
                .filter_map(|a| a.argument.as_ref()),
        )
        .chain(collect.into_expr.iter());
    if stage_exprs
        .chain(collect_exprs)
        .any(|e| contains_window_functions(e) || expression_mutates(e))
    {
        return None;
    }
    Some(Plan {
        for_clause,
        stages,
        collect,
        fields: projection(&for_clause.variable, stages, collect),
    })
}

/// The fields of `var` read between the FOR and the end of the COLLECT,
/// sorted; `None` when something needs the whole document. After the COLLECT
/// the loop variable is out of scope, so nothing later can read it — except
/// through `INTO`, whose members hold every variable (or the KEEP ones).
fn projection(var: &str, stages: &[BodyClause], collect: &CollectClause) -> Option<Vec<String>> {
    use super::super::projection::collect as field_reads;
    // A `` `@x` `` loop variable is what a missing bind var `@x` reads, and
    // the walk takes a bind variable for a constant.
    if var.starts_with('@') {
        return None;
    }
    let mut fields = Vec::new();
    for stage in stages {
        match stage {
            BodyClause::Filter(f) => field_reads(&f.expression, var, &mut fields)?,
            // A LET that rebinds the name makes later reads refer to it.
            BodyClause::Let(l) if l.variable != var => {
                field_reads(&l.expression, var, &mut fields)?
            }
            _ => return None,
        }
    }
    for (_, expr) in &collect.group_vars {
        field_reads(expr, var, &mut fields)?;
    }
    for arg in collect
        .aggregates
        .iter()
        .filter_map(|a| a.argument.as_ref())
    {
        field_reads(arg, var, &mut fields)?;
    }
    if collect.into_var.is_some() {
        match &collect.into_expr {
            Some(expr) => field_reads(expr, var, &mut fields)?,
            None if collect.keep_vars.is_empty() || collect.keep_vars.iter().any(|k| k == var) => {
                return None
            }
            None => {}
        }
    }
    fields.sort_unstable();
    fields.dedup();
    Some(fields)
}

/// What a scan split into key ranges gave: the merged fold, and the counts
/// a sequential scan keeps.
struct Partitioned<'c> {
    fold: CollectFold<'c>,
    scanned: usize,
    passes: Vec<(usize, usize)>,
    partitions: usize,
}

/// The parts of a plan the typed path needs for one document.
#[derive(Clone, Copy)]
struct TypedDoc<'p> {
    stages: &'p [BodyClause],
    var: &'p str,
    /// What the ordinary decode reads, for a document the typed reader
    /// cannot: `None` for the whole document.
    fields: Option<&'p [String]>,
}

impl TypedDoc<'_> {
    /// Fold the stored document `bytes`: read typed, or — when the reader
    /// cannot read it — decoded and evaluated the ordinary way, into the
    /// same fold. Returns whether the document counted as read (the
    /// ordinary scan skips one that does not decode).
    fn fold(
        &self,
        ex: &QueryExecutor<'_>,
        bytes: &[u8],
        reader: &mut TypedReader<'_>,
        fold: &mut CollectFold<'_>,
        scratch: &mut Context,
        passes: &mut [(usize, usize)],
    ) -> DbResult<bool> {
        let read = reader.read(bytes, passes, |key, args| {
            let ordinal = fold.next_ordinal();
            fold.push_datums(ex, key, args, ordinal)
        })?;
        if !matches!(read, Read::Fallback) {
            return Ok(true);
        }
        let decoded = match self.fields {
            Some(fields) => deserialize_doc_projected(bytes, fields),
            None => deserialize_doc_as_value(bytes),
        };
        let Ok(doc) = decoded else {
            return Ok(false);
        };
        bind_in_place(scratch, self.var, doc);
        if ex.run_stages(self.stages, scratch, passes)? {
            fold.push_ref(ex, scratch)?;
        }
        Ok(true)
    }
}

impl QueryExecutor<'_> {
    /// Run `clauses[i]`, a FOR, and the clauses up to its COLLECT as one
    /// streaming fold over the collection, for each of `rows`; `Ok(None)`
    /// when the ordinary path has to run them.
    pub(in crate::sdbql::executor) fn try_scan_collect(
        &self,
        clauses: &[BodyClause],
        i: usize,
        rows: &[Context],
    ) -> DbResult<Option<ScanCollectRun>> {
        if !self.stream_collect {
            return Ok(None);
        }
        let Some(plan) = plan(clauses, i) else {
            return Ok(None);
        };
        let Some(collection) = self.scan_collect_source(plan.for_clause, rows) else {
            return Ok(None);
        };
        let for_clause = plan.for_clause;
        let var = &for_clause.variable;
        // A row policy reads the whole document; the scan filters with it
        // before anything else sees the row (audit H2).
        let gate = self.row_policy_gate(&for_clause.collection);
        let fields = if gate.is_some() {
            None
        } else {
            plan.fields.as_deref()
        };
        // With FILTERs alone and no INTO, nothing of a row outlives it: one
        // context is reused, its document replaced in place.
        let reuse = plan
            .stages
            .iter()
            .all(|s| matches!(s, BodyClause::Filter(_)))
            && plan.collect.into_var.is_none();
        let filter_count = plan
            .stages
            .iter()
            .filter(|s| matches!(s, BodyClause::Filter(_)))
            .count();
        let mut passes = vec![(0usize, 0usize); filter_count];

        // Keys, arguments and FILTERs read from the stored bytes, when they
        // are all fields of the loop variable (`typed_scan`).
        let typed = if gate.is_none() && self.scan.typed {
            TypedPlan::compile(self, var, plan.stages, plan.collect)
        } else {
            None
        };

        let mut fold = CollectFold::new(plan.collect);
        let mut scanned = 0usize;
        let mut partitions = 1;
        for outer in rows {
            // One collection read per row, as `get_for_source_docs` counts.
            self.caches.count_collection_scan();
            if let Some(typed) = &typed {
                // A scan of its own for one outer row: split across threads
                // when the engine and the collection allow it.
                if rows.len() == 1 {
                    if let Some(run) =
                        self.fold_partitions(&plan, typed, &collection, outer, fields)?
                    {
                        fold = run.fold;
                        scanned = run.scanned;
                        passes = run.passes;
                        partitions = run.partitions;
                        continue;
                    }
                }
                let mut reader = TypedReader::new(typed);
                let mut scratch = outer.clone();
                let mut visited = 0usize;
                collection.for_each_doc_bytes(|bytes| {
                    visited += 1;
                    if visited.is_multiple_of(BUDGET_CHECK_INTERVAL) {
                        self.check_budget(fold.held())?;
                    }
                    let doc = TypedDoc {
                        stages: plan.stages,
                        var,
                        fields,
                    };
                    if doc.fold(
                        self,
                        bytes,
                        &mut reader,
                        &mut fold,
                        &mut scratch,
                        &mut passes,
                    )? {
                        scanned += 1;
                    }
                    Ok(true)
                })?;
                continue;
            }
            let mut scratch = outer.clone();
            collection.for_each_value(fields, |doc| {
                scanned += 1;
                if scanned.is_multiple_of(BUDGET_CHECK_INTERVAL) {
                    self.check_budget(fold.held())?;
                }
                if let Some(gate) = &gate {
                    if !self.row_policy_allows(gate, &doc, outer) {
                        return Ok(());
                    }
                }
                if reuse {
                    bind_in_place(&mut scratch, var, doc);
                    if !self.run_stages(plan.stages, &mut scratch, &mut passes)? {
                        return Ok(());
                    }
                    return fold.push_ref(self, &scratch);
                }
                let mut ctx = outer.clone();
                ctx.insert(var.clone(), doc);
                if !self.run_stages(plan.stages, &mut ctx, &mut passes)? {
                    return Ok(());
                }
                fold.push(self, ctx)
            })?;
        }
        let groups = fold.finish()?;
        self.check_budget(groups.len())?;

        let decoded = match (&gate, &plan.fields, &typed) {
            (_, _, Some(typed)) => {
                format!("reading only {} from the stored bytes", typed.describe())
            }
            (None, Some(fields), None) if fields.is_empty() => "decoding no field".to_string(),
            (None, Some(fields), None) => format!("decoding only {}", fields.join(", ")),
            _ => "decoding whole documents".to_string(),
        };
        let decoded = if partitions > 1 {
            format!("{decoded}; {partitions} key ranges read in parallel")
        } else {
            decoded
        };
        Ok(Some(ScanCollectRun {
            rows: groups,
            consumed: plan.stages.len() + 2,
            scanned,
            filters: passes,
            note: format!(
                "optimizer: COLLECT folded into the scan of '{}': documents are grouped as they \
                 are read, never held as rows ({decoded})",
                for_clause.collection
            ),
        }))
    }

    /// The collection a streaming COLLECT would read for `for_clause`, or
    /// `None` when the ordinary scan has to: the same guards, in the same
    /// order, as `get_for_source_docs`.
    fn scan_collect_source(
        &self,
        for_clause: &ForClause,
        rows: &[Context],
    ) -> Option<crate::storage::Collection> {
        let name = &for_clause.collection;
        // A LET-bound array of that name is iterated, not the collection.
        if rows.iter().any(|r| r.contains_key(name)) {
            return None;
        }
        if let Some(db_name) = &self.database {
            let database = self.storage.get_database(db_name).ok()?;
            if self.is_columnar(&database, name) {
                return None;
            }
        }
        // Also the protected-collection guard, and what tells a search view
        // (not a collection) apart: the ordinary path resolves or reports
        // those.
        let collection = self.get_collection(name).ok()?;
        if collection
            .get_shard_config()
            .is_some_and(|c| c.num_shards > 0)
        {
            return None;
        }
        Some(collection)
    }

    /// How many key ranges a typed fold of `collection` would be read in,
    /// in parallel: 1 when it stays on one thread — the clause's aggregates
    /// cannot be merged exactly, the engine has no snapshot to read the
    /// ranges at together (solistore 0.2), or the collection is too small.
    fn planned_partitions(&self, collect: &CollectClause, collection: &Collection) -> usize {
        if !CollectFold::mergeable(collect) || !collection.supports_parallel_scan() {
            return 1;
        }
        self.scan.wanted(collection.count())
    }

    /// The typed fold of `collection`, split into key ranges read on
    /// several threads at one snapshot and merged; `Ok(None)` when it is to
    /// run on this thread alone (see [`Self::planned_partitions`]; also when
    /// the process's scan threads are all taken, or when the folds would
    /// hold more than the row ceiling between them — the sequential fold
    /// then decides, holding fewer).
    ///
    /// The result is the sequential fold's, exactly: each range is folded
    /// in key order with its rows numbered after every earlier range's, and
    /// the folds are merged with only aggregates whose partial results
    /// combine exactly ([`CollectFold::mergeable`]): counts, exact sums
    /// (`SUM`/`AVG` add without rounding, see `exact_sum`), bit operations,
    /// distinct counts, and MIN/MAX — of equal extremes, and of the values
    /// `1` and `1.0` that make one group, the earliest row's is kept, as one
    /// pass keeps it. Errors are reported for the earliest range that had
    /// one. Each range's fold holds O(groups).
    fn fold_partitions<'c>(
        &self,
        plan: &Plan<'c>,
        typed: &TypedPlan,
        collection: &Collection,
        outer: &Context,
        fields: Option<&[String]>,
    ) -> DbResult<Option<Partitioned<'c>>> {
        let wanted = self.planned_partitions(plan.collect, collection);
        if wanted < 2 {
            return Ok(None);
        }
        let helpers = Helpers::take(wanted - 1);
        if helpers.count() == 0 {
            return Ok(None);
        }
        let ranges = collection.doc_partitions(helpers.count() + 1);
        if ranges.len() < 2 {
            return Ok(None);
        }
        let Some(snapshot) = collection.scan_snapshot() else {
            return Ok(None);
        };
        let doc = TypedDoc {
            stages: plan.stages,
            var: &plan.for_clause.variable,
            fields,
        };
        // Set by the first range to fail, or to find the folds too large:
        // the others stop at their next check.
        let stop = AtomicBool::new(false);
        let over_ceiling = AtomicBool::new(false);
        // Values the range folds hold between them, for the row ceiling.
        let held = AtomicUsize::new(0);
        let collect = plan.collect;
        let run = |p: usize, (lower, upper): &(Vec<u8>, Vec<u8>)| -> DbResult<_> {
            let mut fold = CollectFold::for_partition(collect, (p as u64) << 40);
            let mut reader = TypedReader::new(typed);
            let mut scratch = outer.clone();
            let mut passes = vec![(0usize, 0usize); typed.filter_count()];
            let (mut scanned, mut visited, mut published) = (0usize, 0usize, 0usize);
            let result = collection.for_each_doc_bytes_in(lower, upper, Some(&snapshot), |bytes| {
                visited += 1;
                if visited.is_multiple_of(BUDGET_CHECK_INTERVAL) {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(false);
                    }
                    let mine = fold.held();
                    let total = if mine >= published {
                        held.fetch_add(mine - published, Ordering::Relaxed) + (mine - published)
                    } else {
                        held.fetch_sub(published - mine, Ordering::Relaxed) - (published - mine)
                    };
                    published = mine;
                    if total > self.max_intermediate_rows {
                        over_ceiling.store(true, Ordering::Relaxed);
                        stop.store(true, Ordering::Relaxed);
                        return Ok(false);
                    }
                    // The deadline.
                    self.check_budget(0)?;
                }
                if doc.fold(
                    self,
                    bytes,
                    &mut reader,
                    &mut fold,
                    &mut scratch,
                    &mut passes,
                )? {
                    scanned += 1;
                }
                Ok(true)
            });
            if result.is_err() {
                stop.store(true, Ordering::Relaxed);
            }
            result.map(|_| (fold, scanned, passes))
        };
        let results: Vec<DbResult<_>> = std::thread::scope(|s| {
            let others: Vec<_> = ranges[1..]
                .iter()
                .enumerate()
                .map(|(i, range)| {
                    std::thread::Builder::new()
                        .name("solidb-scan".to_string())
                        .spawn_scoped(s, move || run(i + 1, range))
                })
                .collect();
            let mut results = vec![run(0, &ranges[0])];
            for handle in others {
                results.push(match handle {
                    Ok(h) => h.join().unwrap_or_else(|_| {
                        Err(DbError::InternalError("a scan thread panicked".to_string()))
                    }),
                    // No thread to be had: read that range here.
                    Err(_) => {
                        let p = results.len();
                        run(p, &ranges[p])
                    }
                });
            }
            results
        });
        drop(helpers);
        if over_ceiling.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let mut merged: Option<Partitioned<'c>> = None;
        for result in results {
            let (fold, scanned, passes) = result?;
            match &mut merged {
                None => {
                    merged = Some(Partitioned {
                        fold,
                        scanned,
                        passes,
                        partitions: 1,
                    })
                }
                Some(m) => {
                    m.fold.merge(fold);
                    m.scanned += scanned;
                    for (total, part) in m.passes.iter_mut().zip(passes) {
                        total.0 += part.0;
                        total.1 += part.1;
                    }
                    m.partitions += 1;
                }
            }
        }
        // Over the ceiling between them (each range checks its share only
        // every few thousand documents): the one-thread fold decides, and
        // reports it as it always has.
        if merged
            .as_ref()
            .is_some_and(|m| m.fold.held() > self.max_intermediate_rows)
        {
            return Ok(None);
        }
        self.check_budget(0)?;
        Ok(merged)
    }

    /// Run the FILTER and LET `stages` on `ctx`, in order, as the
    /// materialising pipeline runs them on every row: a FILTER whose
    /// condition fails to evaluate drops the row, a LET's error is the
    /// query's. Returns whether the row survived; `passes` counts, per
    /// FILTER, the rows it saw and kept.
    fn run_stages(
        &self,
        stages: &[BodyClause],
        ctx: &mut Context,
        passes: &mut [(usize, usize)],
    ) -> DbResult<bool> {
        let mut filter = 0;
        for stage in stages {
            match stage {
                BodyClause::Filter(f) => {
                    passes[filter].0 += 1;
                    if !self
                        .evaluate_filter_with_context(&f.expression, ctx)
                        .unwrap_or(false)
                    {
                        return Ok(false);
                    }
                    passes[filter].1 += 1;
                    filter += 1;
                }
                BodyClause::Let(l) => self.bind_let(l, ctx)?,
                _ => unreachable!("only FILTER and LET are planned"),
            }
        }
        Ok(true)
    }

    /// What EXPLAIN without analysis says about `query`'s first loop, when
    /// its scan would be folded into a COLLECT.
    pub(in crate::sdbql::executor) fn scan_collect_note(&self, query: &Query) -> Option<String> {
        if !self.stream_collect {
            return None;
        }
        let first = query
            .body_clauses
            .iter()
            .position(|c| matches!(c, BodyClause::For(_)))?;
        let plan = plan(&query.body_clauses, first)?;
        // The names the run's first row binds: a LET or a CTE of the
        // collection's name is iterated instead of it.
        let mut bound = Context::default();
        let ctes = query.with_clause.iter().flat_map(|w| &w.ctes);
        for name in query
            .let_clauses
            .iter()
            .map(|l| &l.variable)
            .chain(ctes.map(|c| &c.name))
        {
            bound.insert(name.clone(), Value::Null);
        }
        let collection = self.scan_collect_source(plan.for_clause, &[bound])?;
        // As the run decides: typed reads unless a row policy needs whole
        // documents, and the key ranges it would read in parallel.
        let gate = self.row_policy_gate(&plan.for_clause.collection);
        let typed = if gate.is_none() && self.scan.typed {
            TypedPlan::compile(self, &plan.for_clause.variable, plan.stages, plan.collect)
        } else {
            None
        };
        let how = match &typed {
            Some(typed) => {
                let wanted = self.planned_partitions(plan.collect, &collection);
                let ranges = if wanted > 1 {
                    collection.doc_partitions(wanted).len()
                } else {
                    1
                };
                let read = format!("reading only {} from the stored bytes", typed.describe());
                if ranges > 1 {
                    format!(" ({read}; {ranges} key ranges read in parallel)")
                } else {
                    format!(" ({read})")
                }
            }
            None => String::new(),
        };
        Some(format!(
            "optimizer: COLLECT folded into the scan of '{}': documents are grouped as they \
             are read, never held as rows{how}",
            plan.for_clause.collection
        ))
    }
}

/// Bind `doc` to `var` in `ctx`, overwriting the previous document in place
/// when nothing else refers to it. A binding the subquery memo has recorded
/// (it holds a `Weak`) is replaced by a new allocation instead, so the memo
/// never sees a changed value at an unchanged address.
fn bind_in_place(ctx: &mut Context, var: &str, doc: Value) {
    match ctx.get_mut_unique(var) {
        Some(slot) => *slot = doc,
        None => {
            ctx.insert(var.to_string(), doc);
        }
    }
}

/// Whether `clauses` holds a COLLECT, which a LIMIT must never be pushed
/// past: it would cap the documents grouped, not the groups returned.
pub(in crate::sdbql::executor) fn has_collect(clauses: &[BodyClause]) -> bool {
    clauses.iter().any(|c| matches!(c, BodyClause::Collect(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdbql::parser::parse;

    /// The decoded fields of the first FOR's streaming plan, `Some(None)`
    /// for whole documents, `None` when the query is not streamed.
    fn fields(q: &str) -> Option<Option<Vec<String>>> {
        let query = parse(q).unwrap();
        plan(&query.body_clauses, 0).map(|p| p.fields)
    }

    fn names(list: &[&str]) -> Option<Option<Vec<String>>> {
        Some(Some(list.iter().map(|s| s.to_string()).collect()))
    }

    #[test]
    fn shapes_it_streams() {
        assert_eq!(
            fields(
                "FOR o IN orders COLLECT status = o.status \
                 AGGREGATE n = COUNT(), total = SUM(o.amount) RETURN {status, n, total}"
            ),
            names(&["amount", "status"])
        );
        assert_eq!(
            fields("FOR o IN orders COLLECT WITH COUNT INTO n RETURN n"),
            names(&[])
        );
        assert_eq!(
            fields(
                "FOR o IN orders FILTER o.amount > 10 LET m = SUBSTRING(o.date, 0, 7) \
                 COLLECT month = m, s = o.address.city INTO g = o._key SORT month RETURN g"
            ),
            names(&["_key", "address", "amount", "date"])
        );
        assert_eq!(
            fields("FOR o IN orders LET s = o.status COLLECT x = s INTO g KEEP s RETURN g"),
            names(&["status"])
        );
        assert_eq!(
            fields("FOR o IN orders FILTER o[\"a.b\"] == 1 COLLECT WITH COUNT INTO n RETURN n"),
            names(&["a", "a.b"])
        );
    }

    #[test]
    fn whole_documents_when_the_clauses_need_them() {
        for q in [
            "FOR o IN orders COLLECT s = o.status INTO g RETURN g",
            "FOR o IN orders COLLECT s = o.status INTO g KEEP o RETURN g",
            "FOR o IN orders COLLECT x = o RETURN x",
            "FOR o IN orders FILTER HAS(o, 'a') COLLECT WITH COUNT INTO n RETURN n",
            "FOR o IN orders LET o = 1 COLLECT x = o RETURN x",
            "FOR o IN orders COLLECT AGGREGATE n = COUNT(o[@f]) RETURN n",
            "FOR o IN orders FILTER (FOR x IN other FILTER x.k == o.k RETURN 1)[0] == 1 \
             COLLECT WITH COUNT INTO n RETURN n",
            "FOR `@o` IN orders COLLECT s = `@o`.status RETURN s",
        ] {
            assert_eq!(fields(q), Some(None), "{q}");
        }
    }

    /// (`FOR o IN l` over a LET-bound `l` is planned like a collection
    /// loop; the row holding `l` is what sends it down the ordinary path.)
    #[test]
    fn everything_else_is_left_to_the_ordinary_path() {
        for q in [
            "FOR o IN orders RETURN o",
            "FOR o IN orders FILTER o.a == 1 RETURN o",
            "FOR o IN 1..10 COLLECT x = o RETURN x",
            "FOR o IN orders FOR i IN o.items COLLECT x = i.sku RETURN x",
            "FOR o IN orders SEARCH o.a == 1 COLLECT WITH COUNT INTO n RETURN n",
            "FOR o IN orders LET r = ROW_NUMBER() OVER (ORDER BY o.a) \
             COLLECT x = r RETURN x",
            "FOR o IN orders LET w = (INSERT {a: o.a} INTO log RETURN NEW) \
             COLLECT WITH COUNT INTO n RETURN n",
            "FOR o IN orders SYSTEM_TIME AS OF 0 COLLECT WITH COUNT INTO n RETURN n",
        ] {
            let query = match parse(q) {
                Ok(query) => query,
                Err(_) => continue,
            };
            assert!(plan(&query.body_clauses, 0).is_none(), "{q}");
        }
    }

    #[test]
    fn a_later_loop_streams_into_its_own_collect() {
        let query = parse(
            "FOR c IN customers FOR o IN orders FILTER o.customer == c._key \
             COLLECT n = c.name AGGREGATE t = SUM(o.amount) RETURN {n, t}",
        )
        .unwrap();
        assert!(plan(&query.body_clauses, 0).is_none());
        // The inner loop folds; the outer one's variable is not projected.
        assert_eq!(
            plan(&query.body_clauses, 1).map(|p| p.fields),
            Some(Some(vec!["amount".to_string(), "customer".to_string()]))
        );
    }
}
