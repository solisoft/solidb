//! COLLECT (GROUP BY) execution.
//!
//! Groups are built in one pass over the incoming rows ([`CollectFold`]).
//! Each row is folded into its group's accumulators and dropped, so
//! `COLLECT ... AGGREGATE` / `WITH COUNT INTO` hold O(groups) memory however
//! many rows flow through. Only `INTO` and `COLLECT_LIST` retain per-row data,
//! because that data is the output; `INTO ... KEEP` projects each row down to
//! the kept variables *before* storing it rather than after.
//!
//! The rows come either from a materialised stage
//! ([`QueryExecutor::execute_collect`]) or straight from a collection scan,
//! one document at a time (`scan_collect`). Both feed the same fold, so the
//! two cannot disagree on a group, an aggregate or the output order.
//!
//! The previous implementation pushed a clone of every row into a per-group
//! `Vec<Context>` and evaluated the aggregates over those lists at the end.
//! The row ceiling (`SOLIDB_MAX_INTERMEDIATE_ROWS`) counted the input once,
//! but the clause held it twice, then a third time while building the `INTO`
//! arrays — a `COLLECT` at the ceiling was the query shape most likely to be
//! the one the OOM killer answered.

use std::borrow::Cow;
use std::collections::HashMap;

use serde_json::Value;

use super::super::aggregation::{AggKind, AggregateAccumulator};
use super::super::datum::{key_hash, value_key_hash, Datum, PreHashedBuild};
use super::super::types::Context;
use super::super::{compare_values, QueryExecutor};
use super::clauses::BUDGET_CHECK_INTERVAL;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{CollectClause, Expression};

/// One group under construction.
struct Group {
    /// The group key: one value per group variable, in declaration order —
    /// those of the group's first row (`1` and `1.0` are one group, which
    /// shows the one it met first).
    key: Vec<Value>,
    /// The ordinal of that first row.
    first: u64,
    count: i64,
    /// Projected rows for `INTO`; stays empty without one.
    members: Vec<Value>,
    aggregates: Vec<AggregateAccumulator>,
}

/// A `COLLECT` in progress: rows go in one at a time with [`Self::push`] (or
/// [`Self::push_ref`], or [`Self::push_datums`] for keys and arguments read
/// without a row), and [`Self::finish`] returns one row per group.
pub(super) struct CollectFold<'c> {
    collect: &'c CollectClause,
    // Groups are found by hashing their key values and confirmed with
    // `values_equal` — the equality `==` uses — rather than by comparing
    // JSON serialisations, which split `1` from `1.0` and cost a string
    // allocation per group variable per row. The hash is computed once per
    // row (`datum::key_hash`); the table does not hash it again.
    groups: Vec<Group>,
    index: HashMap<u64, Vec<usize>, PreHashedBuild>,
    // Per-row values still alive in `groups` (INTO members, COLLECT_LIST
    // items). Over materialised rows it never exceeds what the previous
    // stage already passed the ceiling with; over a scan it is the only
    // thing the ceiling has to bound. Either way the periodic check is also
    // what lets the deadline interrupt a long fold.
    retained: usize,
    seen: usize,
    /// Ordinal of this fold's first row: rows are numbered `base + seen`.
    /// A fold over one partition of a scan starts where no earlier
    /// partition's numbers reach, so that merged folds still know which of
    /// two rows came first.
    base: u64,
    /// Whether the periodic check bounds the values held; off for a fold
    /// over one partition, whose caller bounds them across partitions.
    check_held: bool,
    keep_validated: bool,
    /// Buffers for a row's key and arguments, kept between rows (emptied,
    /// they hold nothing borrowed).
    key_buf: Vec<Datum<'static>>,
    arg_buf: Vec<Option<Datum<'static>>>,
}

/// An empty buffer of `Datum<'b>` from one of `Datum<'a>`, keeping its
/// allocation: a row's key and arguments borrow from that row, so the buffer
/// cannot outlive it with its lifetime, only emptied.
fn relifetime<T, U>(mut buf: Vec<T>) -> Vec<U> {
    buf.clear();
    // Same size and alignment (one type at two lifetimes): the standard
    // library collects in place, so no allocation happens.
    buf.into_iter().map(|_| unreachable!("emptied")).collect()
}

impl<'c> CollectFold<'c> {
    pub(super) fn new(collect: &'c CollectClause) -> Self {
        Self {
            collect,
            groups: Vec::new(),
            index: HashMap::default(),
            retained: 0,
            seen: 0,
            base: 0,
            check_held: true,
            keep_validated: collect.into_var.is_none() || collect.keep_vars.is_empty(),
            key_buf: Vec::new(),
            arg_buf: Vec::new(),
        }
    }

    /// A fold over one partition of a scan, numbering its rows from `base`,
    /// whose caller bounds the values held (across all partitions) itself.
    pub(super) fn for_partition(collect: &'c CollectClause, base: u64) -> Self {
        Self {
            base,
            check_held: false,
            ..Self::new(collect)
        }
    }

    /// Values the fold is holding: one per group, or every per-row value an
    /// `INTO` or a list aggregate keeps, whichever is more. What the row
    /// ceiling bounds.
    pub(super) fn held(&self) -> usize {
        self.retained.max(self.groups.len())
    }

    /// Fold in one row, taking it: `INTO` stores (a projection of) it.
    pub(super) fn push(&mut self, ex: &QueryExecutor<'_>, ctx: Context) -> DbResult<()> {
        let gi = self.fold_row(ex, &ctx)?;
        if self.collect.into_var.is_some() {
            // `INTO g = expr` keeps the projection; plain `INTO g` keeps
            // the row's variables (or just the KEEP ones).
            let member = match &self.collect.into_expr {
                Some(expr) => ex.evaluate_expr_with_context(expr, &ctx)?,
                None => project_into(&self.collect.keep_vars, ctx),
            };
            self.groups[gi].members.push(member);
            self.retained += 1;
        }
        self.tick(ex)
    }

    /// Fold in one row the caller keeps (to reuse for the next one). Nothing
    /// of the row is needed once its key and aggregate arguments are read,
    /// unless there is an `INTO`, which takes a copy.
    pub(super) fn push_ref(&mut self, ex: &QueryExecutor<'_>, ctx: &Context) -> DbResult<()> {
        if self.collect.into_var.is_some() {
            return self.push(ex, ctx.clone());
        }
        self.fold_row(ex, ctx)?;
        self.tick(ex)
    }

    /// Fold in one row given as its group key and aggregate arguments, read
    /// by the caller (the typed scan): what [`Self::push_ref`] does with the
    /// values it evaluates. `ordinal` is the row's position in the input
    /// (see [`Self::next_ordinal`]). Not for a COLLECT with `INTO`.
    pub(super) fn push_datums<'r>(
        &mut self,
        ex: &QueryExecutor<'_>,
        key: &[Datum<'r>],
        args: impl Iterator<Item = Option<Datum<'r>>>,
        ordinal: u64,
    ) -> DbResult<()> {
        debug_assert!(self.collect.into_var.is_none());
        self.fold_datums(key, args, ordinal)?;
        self.tick(ex)
    }

    /// The ordinal the next row pushed without one gets: rows are numbered
    /// in the order they arrive.
    pub(super) fn next_ordinal(&self) -> u64 {
        self.base + self.seen as u64
    }

    /// The deadline and the values held, every `BUDGET_CHECK_INTERVAL` rows.
    fn tick(&mut self, ex: &QueryExecutor<'_>) -> DbResult<()> {
        self.seen += 1;
        if self.seen.is_multiple_of(BUDGET_CHECK_INTERVAL) {
            ex.check_budget(if self.check_held { self.held() } else { 0 })?;
        }
        Ok(())
    }

    /// Fold `ctx`'s group key and aggregate arguments into its group, and
    /// return the group's index.
    fn fold_row(&mut self, ex: &QueryExecutor<'_>, ctx: &Context) -> DbResult<usize> {
        let collect = self.collect;
        // Keys and arguments are borrowed from the row when they are plain
        // variable or field reads, so a row that lands in an existing group
        // copies nothing; a new group takes its key's values.
        let mut key: Vec<Datum<'_>> = relifetime(std::mem::take(&mut self.key_buf));
        for (_, expr) in &collect.group_vars {
            key.push(Datum::of(eval_cow(ex, expr, ctx)?));
        }

        // Every aggregate argument is evaluated before the group is looked
        // up: an argument's error is reported ahead of an unknown aggregate
        // function's, as it always was.
        let mut args: Vec<Option<Datum<'_>>> = relifetime(std::mem::take(&mut self.arg_buf));
        for agg in &collect.aggregates {
            args.push(match &agg.argument {
                Some(expr) => Some(Datum::of(eval_cow(ex, expr, ctx)?)),
                None => None,
            });
        }

        if !self.keep_validated {
            // A KEEP naming a variable that is not in scope would
            // silently store `{}` for every group item — say so instead.
            for keep in &collect.keep_vars {
                if !ctx.contains_key(keep) {
                    return Err(DbError::ExecutionError(format!(
                        "KEEP variable '{}' is not in scope at COLLECT",
                        keep
                    )));
                }
            }
            self.keep_validated = true;
        }

        let ordinal = self.next_ordinal();
        let gi = self.fold_datums(&key, args.drain(..), ordinal)?;
        self.key_buf = relifetime(key);
        self.arg_buf = relifetime(args);
        Ok(gi)
    }

    /// Fold one row's key and arguments into its group; the group's index.
    #[inline]
    fn fold_datums<'r>(
        &mut self,
        key: &[Datum<'r>],
        args: impl Iterator<Item = Option<Datum<'r>>>,
        ordinal: u64,
    ) -> DbResult<usize> {
        let hash = key_hash(key);
        let bucket = self.index.entry(hash).or_default();
        let groups = &mut self.groups;
        let gi = match bucket
            .iter()
            .copied()
            .find(|&i| keys_equal(&groups[i].key, key))
        {
            Some(i) => i,
            None => {
                bucket.push(groups.len());
                let key = key.iter().map(|d| d.clone().into_value()).collect();
                groups.push(new_group(self.collect, key, ordinal)?);
                groups.len() - 1
            }
        };
        let group = &mut groups[gi];
        group.count += 1;
        for (acc, value) in group.aggregates.iter_mut().zip(args) {
            if acc.push(value, ordinal) {
                self.retained += 1;
            }
        }
        Ok(gi)
    }

    /// Whether two folds of this COLLECT over different rows can be
    /// [merged](Self::merge) into exactly what one fold over all of them
    /// would hold: no `INTO` (its members are kept in row order), and only
    /// aggregates whose partial results combine exactly in any order.
    pub(super) fn mergeable(collect: &CollectClause) -> bool {
        collect.into_var.is_none()
            && collect.aggregates.iter().all(|a| {
                AggKind::resolve(&a.function, a.argument.is_some())
                    .is_some_and(AggregateAccumulator::mergeable)
            })
    }

    /// Fold in `other`, a fold of the same COLLECT over other rows, numbered
    /// in the same sequence: the result is the fold of all the rows, in
    /// whatever order or interleaving the two saw them. A group's key is the
    /// one of its earliest row. Only for [`Self::mergeable`] clauses.
    pub(super) fn merge(&mut self, other: CollectFold<'_>) {
        self.seen += other.seen;
        for group in other.groups {
            let hash = value_key_hash(&group.key);
            let bucket = self.index.entry(hash).or_default();
            let groups = &mut self.groups;
            let found = bucket
                .iter()
                .copied()
                .find(|&i| values_keys_equal(&groups[i].key, &group.key));
            let Some(i) = found else {
                bucket.push(groups.len());
                groups.push(group);
                continue;
            };
            let mine = &mut groups[i];
            if group.first < mine.first {
                mine.key = group.key;
                mine.first = group.first;
            }
            mine.count += group.count;
            for (acc, theirs) in mine.aggregates.iter_mut().zip(group.aggregates) {
                let dropped = acc.merge(theirs);
                self.retained = self.retained.saturating_sub(dropped);
            }
        }
        self.retained += other.retained;
    }

    /// One row per group, ordered by key.
    pub(super) fn finish(mut self) -> DbResult<Vec<Context>> {
        let collect = self.collect;
        // AQL: with no group variables, COLLECT always yields exactly one
        // row — over empty input `WITH COUNT INTO n` is 0 and the aggregates
        // are their empty values, not an empty result.
        if collect.group_vars.is_empty() && self.groups.is_empty() {
            self.groups.push(new_group(collect, Vec::new(), 0)?);
        }

        // Groups come out ordered by key (AQL's default sorted COLLECT), not
        // in hash order. No two groups compare equal (`compare_values` is
        // `Equal` exactly when `values_equal` holds, and such keys are one
        // group), so the order the groups were created in cannot show.
        self.groups.sort_by(|a, b| {
            a.key
                .iter()
                .zip(&b.key)
                .map(|(x, y)| compare_values(x, y))
                .find(|o| o.is_ne())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut out = Vec::with_capacity(self.groups.len());
        for group in self.groups {
            let Group {
                key,
                first: _,
                count,
                members,
                aggregates,
            } = group;
            let mut ctx = Context::with_capacity_and_hasher(
                collect.group_vars.len()
                    + collect.aggregates.len()
                    + usize::from(collect.into_var.is_some())
                    + usize::from(collect.count_var.is_some()),
                Default::default(),
            );
            for ((name, _), val) in collect.group_vars.iter().zip(key) {
                ctx.insert(name.clone(), val);
            }
            if let Some(into_var) = &collect.into_var {
                ctx.insert(into_var.clone(), Value::Array(members));
            }
            if let Some(count_var) = &collect.count_var {
                ctx.insert(count_var.clone(), Value::Number(count.into()));
            }
            for (agg, acc) in collect.aggregates.iter().zip(aggregates) {
                ctx.insert(agg.variable.clone(), acc.finish());
            }
            out.push(ctx);
        }
        Ok(out)
    }
}

impl<'a> QueryExecutor<'a> {
    /// Fold `rows` into groups and return one context per group.
    pub(in crate::sdbql::executor) fn execute_collect(
        &self,
        collect: &CollectClause,
        rows: Vec<Context>,
    ) -> DbResult<Vec<Context>> {
        let mut fold = CollectFold::new(collect);
        for ctx in rows {
            fold.push(self, ctx)?;
        }
        fold.finish()
    }
}

/// `expr`'s value in `ctx`, borrowed when it is a plain read of a binding:
/// the same value, and the same error, as evaluating it.
fn eval_cow<'v>(
    ex: &'v QueryExecutor<'_>,
    expr: &'v Expression,
    ctx: &'v Context,
) -> DbResult<Cow<'v, Value>> {
    Ok(match ex.resolve_ref(expr, ctx) {
        Some(v) => Cow::Borrowed(v),
        None => Cow::Owned(ex.evaluate_expr_with_context(expr, ctx)?),
    })
}

fn new_group(collect: &CollectClause, key: Vec<Value>, first: u64) -> DbResult<Group> {
    let aggregates = collect
        .aggregates
        .iter()
        .map(|a| AggregateAccumulator::new(&a.function, a.argument.is_some()))
        .collect::<DbResult<Vec<_>>>()?;
    Ok(Group {
        key,
        first,
        count: 0,
        members: Vec::new(),
        aggregates,
    })
}

fn keys_equal(a: &[Value], b: &[Datum<'_>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| y.eq_value(x))
}

fn values_keys_equal(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| super::super::values_equal(x, y))
}

/// The object stored for one row under `INTO`: every variable in scope, or
/// only those named by `KEEP`. Takes the row apart rather than copying it;
/// with `KEEP`, only the kept bindings are taken, so a shared one it drops (a
/// memoised subquery's rows) is never copied.
fn project_into(keep_vars: &[String], mut ctx: Context) -> Value {
    let obj: serde_json::Map<String, Value> = if keep_vars.is_empty() {
        ctx.into_iter().collect()
    } else {
        let mut obj = serde_json::Map::with_capacity(keep_vars.len());
        for k in keep_vars {
            // A name kept twice was taken the first time.
            if let Some(v) = ctx.remove(k) {
                obj.insert(k.clone(), v);
            }
        }
        obj
    };
    Value::Object(obj)
}
