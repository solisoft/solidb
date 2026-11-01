//! Run an uncorrelated subquery once per query instead of once per row.
//!
//! `FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER ... RETURN c._key)`
//! used to execute the inner `FOR` for every comment, and a second level of
//! nesting multiplied again: 2,022 comments × 1,079 cards × 168 columns ran for
//! minutes on a blocking thread, where the same query with the subquery bound
//! by a `LET` first took 0.18 s. A subquery that reads nothing from the row
//! it is evaluated in has the same result for every row, so it is executed the
//! first time it is needed and the rows are reused. `x IN (subquery)` also
//! gets the hash-set lookup that `x IN @bindVar` has.
//!
//! A subquery is memoised only when all of these hold. Anything else runs per
//! row, as before.
//! - The top-level query writes nothing (set by `execute_with_stats`): a
//!   per-row INSERT/UPDATE/REMOVE can change what a later evaluation reads.
//! - The subquery itself writes nothing.
//! - Every function it calls is deterministic: a pure builtin, or a read
//!   (`DOCUMENT`, `COLLECTION_COUNT`). No `RAND`, `UUID`, `NOW`, user
//!   functions, and no window functions.
//! - No string in its AST names a variable of the current row. The check runs
//!   on the serialised AST rather than on `Expression::Variable` nodes alone,
//!   because a row variable can also be reached by name from a clause
//!   (`FOR x IN outerVar`, a JOIN source, `KEEP`): the FOR source is looked up
//!   in the row context before it is taken as a collection. Over-matching (a
//!   string literal equal to a variable name) only means "not memoised".
//!
//! Lazy: the subquery runs on first use, so a subquery in a branch that is
//! never taken is never run, and an error is reported exactly as before
//! (and not cached).

use std::sync::Arc;

use serde_json::Value;

use super::const_fold::is_pure_function;
use super::types::Context;
use super::{QueryExecutor, ValueSet};
use crate::error::DbResult;
use crate::sdbql::ast::{query_any_expression, Expression, Query};

/// Read builtins whose result is fixed for the duration of one query.
const DETERMINISTIC_READS: &[&str] = &["DOCUMENT", "COLLECTION_COUNT"];

/// Below this many rows a linear `IN` scan is as fast as hashing.
const MIN_SET_LEN: usize = 16;

/// What is known about one subquery node during one query.
pub(super) struct SubqueryMemo {
    /// The node itself, compared on every hit so another AST reusing the
    /// address can never read this entry.
    query: Query,
    /// `None` when the subquery can never be memoised; otherwise every
    /// string in its AST, any of which may name an outer variable.
    names: Option<Arc<[String]>>,
    rows: Option<Arc<Vec<Value>>>,
    set: Option<Arc<ValueSet>>,
}

fn collect_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|i| collect_strings(i, out)),
        Value::Object(map) => map.values().for_each(|i| collect_strings(i, out)),
        _ => {}
    }
}

/// The names a subquery may read from its context, or `None` when it must
/// not be memoised at all.
fn analyse(query: &Query) -> Option<Arc<[String]>> {
    if query.has_mutations() {
        return None;
    }
    let nondeterministic = query_any_expression(query, &mut |e| match e {
        Expression::FunctionCall { name, args } => {
            !(is_pure_function(name, args.len())
                || DETERMINISTIC_READS
                    .iter()
                    .any(|f| name.eq_ignore_ascii_case(f)))
        }
        Expression::WindowFunctionCall { .. } => true,
        _ => false,
    });
    if nondeterministic {
        return None;
    }
    let ast = serde_json::to_value(query).ok()?;
    let mut names = Vec::new();
    collect_strings(&ast, &mut names);
    names.sort_unstable();
    names.dedup();
    Some(names.into())
}

fn reads_context(names: &[String], ctx: &Context) -> bool {
    names
        .iter()
        .any(|n| ctx.contains_key(n) || ctx.contains_key(&format!("@{n}")))
}

impl QueryExecutor<'_> {
    /// The rows of `query` evaluated in `ctx`, from the memo when it is
    /// uncorrelated. `Ok(None)` means "run it the ordinary way".
    pub(super) fn memoized_subquery(
        &self,
        query: &Query,
        ctx: &Context,
    ) -> DbResult<Option<Arc<Vec<Value>>>> {
        if !self.caches.subquery_memo_enabled() {
            return Ok(None);
        }
        let addr = query as *const Query as usize;
        {
            let mut memos = self.caches.subqueries.lock();
            let entry = memos.entry(addr).or_insert_with(|| SubqueryMemo {
                query: query.clone(),
                names: analyse(query),
                rows: None,
                set: None,
            });
            if entry.query != *query {
                // Address reused by a different AST: start over for it.
                *entry = SubqueryMemo {
                    query: query.clone(),
                    names: analyse(query),
                    rows: None,
                    set: None,
                };
            }
            let Some(names) = &entry.names else {
                return Ok(None);
            };
            if reads_context(names, ctx) {
                return Ok(None);
            }
            if let Some(rows) = &entry.rows {
                return Ok(Some(rows.clone()));
            }
        }
        // Run outside the lock: the subquery's own subqueries take it too.
        let rows = Arc::new(self.execute_with_parent_context(query, ctx)?);
        if let Some(entry) = self.caches.subqueries.lock().get_mut(&addr) {
            if entry.query == *query {
                entry.rows = Some(rows.clone());
            }
        }
        Ok(Some(rows))
    }

    /// A hash set over the rows of an uncorrelated subquery, for
    /// `x IN (subquery)`. `Ok(None)` when it is correlated or small.
    pub(super) fn memoized_subquery_set(
        &self,
        query: &Query,
        ctx: &Context,
    ) -> DbResult<Option<Arc<ValueSet>>> {
        let Some(rows) = self.memoized_subquery(query, ctx)? else {
            return Ok(None);
        };
        if rows.len() < MIN_SET_LEN {
            return Ok(None);
        }
        let addr = query as *const Query as usize;
        let mut memos = self.caches.subqueries.lock();
        let Some(entry) = memos.get_mut(&addr) else {
            return Ok(None);
        };
        if let Some(set) = &entry.set {
            return Ok(Some(set.clone()));
        }
        let set = Arc::new(ValueSet::from_values(&rows));
        entry.set = Some(set.clone());
        Ok(Some(set))
    }
}
