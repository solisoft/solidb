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
use std::hash::{Hash, Hasher};

use serde_json::Value;

use super::super::aggregation::AggregateAccumulator;
use super::super::types::Context;
use super::super::{compare_values, hash_value, values_equal, QueryExecutor};
use super::clauses::BUDGET_CHECK_INTERVAL;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{CollectClause, Expression};

/// One group under construction.
struct Group {
    /// The group key: one value per group variable, in declaration order.
    key: Vec<Value>,
    /// The group variables, later extended with INTO / COUNT / AGGREGATE.
    ctx: Context,
    count: i64,
    /// Projected rows for `INTO`; stays empty without one.
    members: Vec<Value>,
    aggregates: Vec<AggregateAccumulator>,
}

/// A `COLLECT` in progress: rows go in one at a time with [`Self::push`] (or
/// [`Self::push_ref`]), and [`Self::finish`] returns one row per group.
pub(super) struct CollectFold<'c> {
    collect: &'c CollectClause,
    // Groups are found by hashing their key values and confirmed with
    // `values_equal` — the equality `==` uses — rather than by comparing
    // JSON serialisations, which split `1` from `1.0` and cost a string
    // allocation per group variable per row.
    groups: Vec<Group>,
    index: HashMap<u64, Vec<usize>>,
    // Per-row values still alive in `groups` (INTO members, COLLECT_LIST
    // items). Over materialised rows it never exceeds what the previous
    // stage already passed the ceiling with; over a scan it is the only
    // thing the ceiling has to bound. Either way the periodic check is also
    // what lets the deadline interrupt a long fold.
    retained: usize,
    seen: usize,
    keep_validated: bool,
}

impl<'c> CollectFold<'c> {
    pub(super) fn new(collect: &'c CollectClause) -> Self {
        Self {
            collect,
            groups: Vec::new(),
            index: HashMap::new(),
            retained: 0,
            seen: 0,
            keep_validated: collect.into_var.is_none() || collect.keep_vars.is_empty(),
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

    /// The deadline and the values held, every `BUDGET_CHECK_INTERVAL` rows.
    fn tick(&mut self, ex: &QueryExecutor<'_>) -> DbResult<()> {
        self.seen += 1;
        if self.seen.is_multiple_of(BUDGET_CHECK_INTERVAL) {
            ex.check_budget(self.held())?;
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
        let mut key: Vec<Cow<'_, Value>> = Vec::with_capacity(collect.group_vars.len());
        for (_, expr) in &collect.group_vars {
            key.push(eval_cow(ex, expr, ctx)?);
        }
        let key_hash = group_key_hash(&key);

        // Every aggregate argument is evaluated before the group is looked
        // up: an argument's error is reported ahead of an unknown aggregate
        // function's, as it always was.
        let mut agg_values = Vec::with_capacity(collect.aggregates.len());
        for agg in &collect.aggregates {
            agg_values.push(match &agg.argument {
                Some(expr) => Some(eval_cow(ex, expr, ctx)?),
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

        let bucket = self.index.entry(key_hash).or_default();
        let found = bucket
            .iter()
            .copied()
            .find(|&i| keys_equal(&self.groups[i].key, &key));
        let gi = match found {
            Some(i) => i,
            None => {
                bucket.push(self.groups.len());
                let key = key.into_iter().map(Cow::into_owned).collect();
                self.groups.push(new_group(collect, key)?);
                self.groups.len() - 1
            }
        };
        let group = &mut self.groups[gi];
        group.count += 1;
        for (acc, value) in group.aggregates.iter_mut().zip(agg_values) {
            if acc.push(value) {
                self.retained += 1;
            }
        }
        Ok(gi)
    }

    /// One row per group, ordered by key.
    pub(super) fn finish(mut self) -> DbResult<Vec<Context>> {
        let collect = self.collect;
        // AQL: with no group variables, COLLECT always yields exactly one
        // row — over empty input `WITH COUNT INTO n` is 0 and the aggregates
        // are their empty values, not an empty result.
        if collect.group_vars.is_empty() && self.groups.is_empty() {
            self.groups.push(new_group(collect, Vec::new())?);
        }

        // Groups come out ordered by key (AQL's default sorted COLLECT), not
        // in hash order. The sort is stable: groups that compare equal keep
        // the order their first rows arrived in.
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
                key: _,
                mut ctx,
                count,
                members,
                aggregates,
            } = group;
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

fn new_group(collect: &CollectClause, key: Vec<Value>) -> DbResult<Group> {
    let mut ctx = Context::with_capacity_and_hasher(
        collect.group_vars.len()
            + collect.aggregates.len()
            + usize::from(collect.into_var.is_some())
            + usize::from(collect.count_var.is_some()),
        Default::default(),
    );
    for ((name, _), val) in collect.group_vars.iter().zip(&key) {
        ctx.insert(name.clone(), val.clone());
    }
    let aggregates = collect
        .aggregates
        .iter()
        .map(|a| AggregateAccumulator::new(&a.function, a.argument.is_some()))
        .collect::<DbResult<Vec<_>>>()?;
    Ok(Group {
        key,
        ctx,
        count: 0,
        members: Vec::new(),
        aggregates,
    })
}

fn group_key_hash(key: &[Cow<'_, Value>]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for v in key {
        hash_value(v).hash(&mut h);
    }
    h.finish()
}

fn keys_equal(a: &[Value], b: &[Cow<'_, Value>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| values_equal(x, y))
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
