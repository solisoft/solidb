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

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use serde_json::Value;

use super::super::aggregation::AggregateAccumulator;
use super::super::types::Context;
use super::super::QueryExecutor;
use super::clauses::BUDGET_CHECK_INTERVAL;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::CollectClause;

/// One group under construction.
struct Group {
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
        let mut groups: HashMap<String, Group> = HashMap::new();
        // Per-row values still alive in `groups` (INTO members, COLLECT_LIST
        // items). Never more than the rows we were handed, so on its own it
        // cannot trip the ceiling the previous stage already passed; the
        // periodic check is what lets the deadline interrupt a long fold.
        let mut retained = 0usize;
        let mut keep_validated = collect.into_var.is_none() || collect.keep_vars.is_empty();

        for (seen, ctx) in rows.into_iter().enumerate() {
            let mut key_parts = Vec::with_capacity(collect.group_vars.len());
            let mut group_vals = Vec::with_capacity(collect.group_vars.len());
            for (var_name, expr) in &collect.group_vars {
                let val = self.evaluate_expr_with_context(expr, &ctx)?;
                key_parts.push(serde_json::to_string(&val).unwrap_or_default());
                group_vals.push((var_name, val));
            }
            let group_key = key_parts.join("|");

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

            let group = match groups.entry(group_key) {
                Entry::Occupied(e) => e.into_mut(),
                Entry::Vacant(e) => {
                    let mut group_ctx = Context::with_capacity(
                        collect.group_vars.len()
                            + collect.aggregates.len()
                            + usize::from(collect.into_var.is_some())
                            + usize::from(collect.count_var.is_some()),
                    );
                    for (name, val) in group_vals {
                        group_ctx.insert(name.clone(), val);
                    }
                    let aggregates = collect
                        .aggregates
                        .iter()
                        .map(|a| AggregateAccumulator::new(&a.function, a.argument.is_some()))
                        .collect::<DbResult<Vec<_>>>()?;
                    e.insert(Group {
                        ctx: group_ctx,
                        count: 0,
                        members: Vec::new(),
                        aggregates,
                    })
                }
            };

            group.count += 1;
            for (acc, value) in group.aggregates.iter_mut().zip(agg_values) {
                if acc.push(value) {
                    retained += 1;
                }
            }
            if collect.into_var.is_some() {
                group.members.push(project_into(&collect.keep_vars, ctx));
                retained += 1;
            }

            if (seen + 1) % BUDGET_CHECK_INTERVAL == 0 {
                self.check_budget(retained.max(groups.len()))?;
            }
        }

        let mut out = Vec::with_capacity(groups.len());
        for group in groups.into_values() {
            let Group {
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
