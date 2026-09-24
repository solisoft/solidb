//! COLLECT (GROUP BY) execution.
//!
//! Groups are built in one pass over the incoming rows. Each row is consumed —
//! moved, never cloned — and folded into its group's accumulators, so
//! `COLLECT ... AGGREGATE` / `WITH COUNT INTO` hold O(groups) memory however
//! many rows flow through. Only `INTO` and `COLLECT_LIST` retain per-row data,
//! because that data is the output; `INTO ... KEEP` projects each row down to
//! the kept variables *before* storing it rather than after.
//!
//! The previous implementation pushed a clone of every row into a per-group
//! `Vec<Context>` and evaluated the aggregates over those lists at the end.
//! The row ceiling (`SOLIDB_MAX_INTERMEDIATE_ROWS`) counted the input once,
//! but the clause held it twice, then a third time while building the `INTO`
//! arrays — a `COLLECT` at the ceiling was the query shape most likely to be
//! the one the OOM killer answered.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use serde_json::Value;

use super::super::aggregation::AggregateAccumulator;
use super::super::types::Context;
use super::super::{compare_values, hash_value, values_equal, QueryExecutor};
use super::clauses::BUDGET_CHECK_INTERVAL;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::CollectClause;

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

impl<'a> QueryExecutor<'a> {
    /// Fold `rows` into groups and return one context per group.
    pub(super) fn execute_collect(
        &self,
        collect: &CollectClause,
        rows: Vec<Context>,
    ) -> DbResult<Vec<Context>> {
        // Groups are found by hashing their key values and confirmed with
        // `values_equal` — the equality `==` uses — rather than by comparing
        // JSON serialisations, which split `1` from `1.0` and cost a string
        // allocation per group variable per row.
        let mut groups: Vec<Group> = Vec::new();
        let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
        // Per-row values still alive in `groups` (INTO members, COLLECT_LIST
        // items). Never more than the rows we were handed, so on its own it
        // cannot trip the ceiling the previous stage already passed; the
        // periodic check is what lets the deadline interrupt a long fold.
        let mut retained = 0usize;
        let mut keep_validated = collect.into_var.is_none() || collect.keep_vars.is_empty();

        for (seen, ctx) in rows.into_iter().enumerate() {
            let mut key = Vec::with_capacity(collect.group_vars.len());
            for (_, expr) in &collect.group_vars {
                key.push(self.evaluate_expr_with_context(expr, &ctx)?);
            }
            let key_hash = group_key_hash(&key);

            // Evaluate the aggregate arguments while `ctx` is still whole;
            // the INTO projection below takes it apart.
            let mut agg_values = Vec::with_capacity(collect.aggregates.len());
            for agg in &collect.aggregates {
                agg_values.push(match &agg.argument {
                    Some(expr) => Some(self.evaluate_expr_with_context(expr, &ctx)?),
                    None => None,
                });
            }

            if !keep_validated {
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
                keep_validated = true;
            }

            let bucket = index.entry(key_hash).or_default();
            let found = bucket
                .iter()
                .copied()
                .find(|&i| keys_equal(&groups[i].key, &key));
            let gi = match found {
                Some(i) => i,
                None => {
                    bucket.push(groups.len());
                    groups.push(self.new_group(collect, key)?);
                    groups.len() - 1
                }
            };
            let group = &mut groups[gi];

            group.count += 1;
            for (acc, value) in group.aggregates.iter_mut().zip(agg_values) {
                if acc.push(value) {
                    retained += 1;
                }
            }
            if collect.into_var.is_some() {
                // `INTO g = expr` keeps the projection; plain `INTO g` keeps
                // the row's variables (or just the KEEP ones).
                let member = match &collect.into_expr {
                    Some(expr) => self.evaluate_expr_with_context(expr, &ctx)?,
                    None => project_into(&collect.keep_vars, ctx),
                };
                group.members.push(member);
                retained += 1;
            }

            if (seen + 1) % BUDGET_CHECK_INTERVAL == 0 {
                self.check_budget(retained.max(groups.len()))?;
            }
        }

        // AQL: with no group variables, COLLECT always yields exactly one
        // row — over empty input `WITH COUNT INTO n` is 0 and the aggregates
        // are their empty values, not an empty result.
        if collect.group_vars.is_empty() && groups.is_empty() {
            groups.push(self.new_group(collect, Vec::new())?);
        }

        // Groups come out ordered by key (AQL's default sorted COLLECT), not
        // in hash order.
        groups.sort_by(|a, b| {
            a.key
                .iter()
                .zip(&b.key)
                .map(|(x, y)| compare_values(x, y))
                .find(|o| o.is_ne())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut out = Vec::with_capacity(groups.len());
        for group in groups {
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

    fn new_group(&self, collect: &CollectClause, key: Vec<Value>) -> DbResult<Group> {
        let mut ctx = Context::with_capacity(
            collect.group_vars.len()
                + collect.aggregates.len()
                + usize::from(collect.into_var.is_some())
                + usize::from(collect.count_var.is_some()),
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
}

fn group_key_hash(key: &[Value]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for v in key {
        hash_value(v).hash(&mut h);
    }
    h.finish()
}

fn keys_equal(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| values_equal(x, y))
}

/// The object stored for one row under `INTO`: every variable in scope, or
/// only those named by `KEEP`. Takes the row apart rather than copying it.
fn project_into(keep_vars: &[String], ctx: Context) -> Value {
    let obj: serde_json::Map<String, Value> = if keep_vars.is_empty() {
        ctx.into_iter().collect()
    } else {
        ctx.into_iter()
            .filter(|(k, _)| keep_vars.contains(k))
            .collect()
    };
    Value::Object(obj)
}
