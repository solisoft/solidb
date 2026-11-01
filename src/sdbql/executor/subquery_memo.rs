//! Reuse a subquery's rows while the bindings it can read are unchanged.
//!
//! `FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER ... RETURN c._key)`
//! used to execute the inner `FOR` for every comment, and a second level of
//! nesting multiplied again: 2,022 comments × 1,079 cards × 168 columns ran for
//! minutes on a blocking thread, where the same query with the subquery bound
//! by a `LET` first took 0.18 s. A subquery's result depends only on the
//! context values it can read, so each subquery node keeps its last result
//! and the bindings it was computed under, and reuses it while those are the
//! same. That covers the uncorrelated case (no binding read: one run per
//! query) and a subquery that reads only an outer loop's row
//! (`FOR col ... FOR m ... (FOR c IN cards FILTER c.column_id == col._key ...)`
//! runs once per column, not once per comment: 7.2 s to 12 ms on the board).
//! `x IN (subquery)` also gets the hash-set lookup that `x IN @bindVar` has.
//!
//! A subquery is reused only when all of these hold. Anything else runs per
//! row, as before.
//! - The top-level query writes nothing (set by `execute_with_stats`): a
//!   per-row INSERT/UPDATE/REMOVE can change what a later evaluation reads.
//! - The subquery itself writes nothing.
//! - Every function it calls is deterministic: a pure builtin, or a read
//!   (`DOCUMENT`, `COLLECTION_COUNT`). No `RAND`, `UUID`, `NOW`, user
//!   functions, and no window functions.
//! - It holds no `COLLECT ... INTO g` without `KEEP` or a projection, at any
//!   depth: each group member is then every variable in scope, the outer row
//!   included, so the result depends on bindings no name in it mentions.
//! - Every binding it can read is the same allocation as last time. What it
//!   can read is every string in its serialised AST, plus `@name` for each
//!   bind variable (a backtick identifier, `` LET `@ids` = ... ``, binds the
//!   context key a missing bind var falls back to). The check runs on the
//!   serialised AST rather than on `Expression::Variable` nodes alone,
//!   because a row variable can also be reached by name from a clause
//!   (`FOR x IN outerVar`, a JOIN source, `KEEP`): the FOR source is looked
//!   up in the row context before it is taken as a collection. Over-matching
//!   (a string literal equal to a variable name) only means fewer hits. One
//!   name is dropped: a subquery whose first FOR rebinds an outer name
//!   (`(FOR doc IN cards ...)` inside `FOR doc IN comments`, as the ORM writes
//!   every query) does not read the outer `doc`.
//!
//! The bindings are recorded as `Weak` references and compared by address.
//! A `Weak` keeps the allocation reserved, so no later binding can be given
//! that address; and while it exists a `Context` write cannot change the
//! value in place (`Arc::make_mut` moves it to a new allocation, `try_unwrap`
//! moves it out). So an unchanged address means an unchanged value, and since
//! the memo holds no strong reference, a lambda parameter rebound in place or
//! a `RETURN` field move never copies because of it.
//!
//! Storing is adaptive. A result is kept after each miss, but after three
//! misses in a row (a subquery that really is per row) nothing is stored and
//! no binding is kept alive; storing resumes when a row's bindings repeat the
//! previous miss's.
//!
//! Lazy: the subquery runs on first use, so a subquery in a branch that is
//! never taken is never run, and an error is reported exactly as before
//! (and not cached).
//!
//! The index planner reads the same entries: a subquery the analysis shows
//! does not read a FOR's variable can be that FOR's lookup key
//! (`index_opt.rs`, `try_subquery_index_stage`). Only there is a failure
//! remembered, so the planner does not re-run it under the same bindings.

use std::sync::{Arc, Weak};

use serde_json::Value;

use super::const_fold::is_pure_function;
use super::types::Context;
use super::{QueryExecutor, ValueSet};
use crate::error::DbResult;
use crate::sdbql::ast::{query_any_expression, BodyClause, Expression, Query};

/// Read builtins whose result is fixed for the duration of one query.
const DETERMINISTIC_READS: &[&str] = &["DOCUMENT", "COLLECTION_COUNT"];

/// Below this many rows a linear `IN` scan is as fast as hashing.
const MIN_SET_LEN: usize = 16;

/// Misses in a row after which a result is no longer stored.
const STORED_MISSES: u32 = 3;

/// Subquery nodes tracked per query; a node past this is run per row.
const MAX_ENTRIES: usize = 1024;

/// For each index into `names` that the context binds, that binding.
type Bindings = Vec<(usize, Weak<Value>)>;

/// What is known about one subquery node during one query.
pub(super) struct SubqueryMemo {
    /// The node itself, compared on every lookup so another AST reusing the
    /// address can never read this entry.
    query: Query,
    /// `None` when the subquery can never be reused; otherwise every name
    /// it may read from its context.
    names: Option<Arc<[String]>>,
    last: Option<Computed>,
    misses_in_a_row: u32,
    /// The bindings of the last miss that was not stored.
    seen: Bindings,
    /// The bindings an execution for the index planner failed under, so the
    /// planner does not run it again for them (evaluation paths ignore it
    /// and report or swallow the error as before). Cleared when a result is
    /// stored.
    failed: Option<Bindings>,
}

/// A result and the bindings it was computed under.
struct Computed {
    bound: Bindings,
    /// Always a `Value::Array`, shared with the rows it is bound into.
    rows: Arc<Value>,
    set: Option<Arc<ValueSet>>,
}

impl SubqueryMemo {
    fn new(query: &Query) -> Self {
        Self {
            query: query.clone(),
            names: analyse(query),
            last: None,
            misses_in_a_row: 0,
            seen: Vec::new(),
            failed: None,
        }
    }
}

fn collect_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|i| collect_strings(i, out)),
        Value::Object(map) => {
            // A bind var not passed in reads the "@name" context key.
            if let Some(Value::String(n)) = map.get("BindVariable") {
                out.push(format!("@{n}"));
            }
            map.values().for_each(|i| collect_strings(i, out))
        }
        _ => {}
    }
}

fn strings_of(v: &impl serde::Serialize) -> Option<Vec<String>> {
    let ast = serde_json::to_value(v).ok()?;
    let mut out = Vec::new();
    collect_strings(&ast, &mut out);
    Some(out)
}

/// A call whose result can differ between two evaluations in one query.
pub(super) fn is_nondeterministic(e: &Expression) -> bool {
    match e {
        Expression::FunctionCall { name, args } => {
            !(is_pure_function(name, args.len())
                || DETERMINISTIC_READS
                    .iter()
                    .any(|f| name.eq_ignore_ascii_case(f)))
        }
        Expression::WindowFunctionCall { .. } => true,
        _ => false,
    }
}

/// A `COLLECT ... INTO g` whose members are the whole scope, anywhere in
/// `query`: its own body, set-operation operands, CTE bodies and nested
/// subqueries (`query_any_expression` reaches their expressions, not their
/// clauses, so the predicate recurses).
fn has_bare_into(query: &Query) -> bool {
    query.body_clauses.iter().any(|c| {
        matches!(c, BodyClause::Collect(c)
            if c.into_var.is_some() && c.keep_vars.is_empty() && c.into_expr.is_none())
    }) || query
        .set_operations
        .iter()
        .any(|op| has_bare_into(&op.query))
        || query
            .with_clause
            .as_ref()
            .is_some_and(|with| with.ctes.iter().any(|cte| has_bare_into(&cte.query)))
        || query_any_expression(
            query,
            &mut |e| matches!(e, Expression::Subquery(q) if has_bare_into(q)),
        )
}

/// The first FOR's variable, when the block binds it before anything can
/// read the outer binding of that name. The FOR's own source, `SYSTEM_TIME`
/// and valid-time are evaluated in the outer row; CTEs and pre-FOR LETs are
/// bound before it; LIMIT is evaluated with the block's starting bindings;
/// set operands run on the outer bindings. Everything else runs in the rows
/// the FOR produces (or after a COLLECT, which starts from an empty context).
fn rebound_first_for_var(query: &Query) -> Option<&str> {
    let Some(BodyClause::For(f)) = query.body_clauses.first() else {
        return None;
    };
    let v = f.variable.as_str();
    if f.collection == v || f.source_variable.as_deref() == Some(v) {
        return None;
    }
    let outside = strings_of(&(
        &f.source_expression,
        &f.system_time,
        &f.valid_time,
        &query.with_clause,
        &query.let_clauses,
        &query.limit_clause,
        &query.set_operations,
    ))?;
    (!outside.iter().any(|s| s == v)).then_some(v)
}

/// The names a subquery may read from its context, or `None` when it must
/// not be reused at all.
fn analyse(query: &Query) -> Option<Arc<[String]>> {
    if query.has_mutations()
        || query_any_expression(query, &mut is_nondeterministic)
        || has_bare_into(query)
    {
        return None;
    }
    let mut names = strings_of(query)?;
    if let Some(v) = rebound_first_for_var(query) {
        names.retain(|n| n != v);
    }
    names.sort_unstable();
    names.dedup();
    Some(names.into())
}

/// Whether `ctx` binds exactly the recorded names, each to the recorded
/// allocation.
fn same_bindings(names: &[String], ctx: &Context, recorded: &[(usize, Weak<Value>)]) -> bool {
    let mut recorded = recorded.iter();
    for (i, name) in names.iter().enumerate() {
        if let Some(cur) = ctx.get_shared(name) {
            match recorded.next() {
                Some((j, w)) if *j == i && std::ptr::eq(w.as_ptr(), Arc::as_ptr(cur)) => {}
                _ => return false,
            }
        }
    }
    recorded.next().is_none()
}

fn record_bindings(names: &[String], ctx: &Context, out: &mut Bindings) {
    out.clear();
    for (i, name) in names.iter().enumerate() {
        if let Some(cur) = ctx.get_shared(name) {
            out.push((i, Arc::downgrade(cur)));
        }
    }
}

/// The entry for the node at `addr`, created (or reset, when another AST
/// reuses the address) as needed. `None` when the table is full.
fn entry_for<'m>(
    memos: &'m mut std::collections::HashMap<usize, SubqueryMemo>,
    addr: usize,
    query: &Query,
) -> Option<&'m mut SubqueryMemo> {
    if memos.len() >= MAX_ENTRIES && !memos.contains_key(&addr) {
        return None;
    }
    let entry = memos
        .entry(addr)
        .or_insert_with(|| SubqueryMemo::new(query));
    if entry.query != *query {
        *entry = SubqueryMemo::new(query);
    }
    Some(entry)
}

impl QueryExecutor<'_> {
    /// The rows of `query` evaluated in `ctx` (a `Value::Array`): the
    /// remembered ones when its bindings are unchanged, otherwise executed
    /// here and remembered. `Ok(None)` means "not reusable this time": the
    /// caller must then run it with `execute_with_parent_context`, never by
    /// evaluating the expression again, since a second lookup for the same
    /// row would look like a repeat and resume storing.
    pub(super) fn memoized_subquery(
        &self,
        query: &Query,
        ctx: &Context,
    ) -> DbResult<Option<Arc<Value>>> {
        if !self.caches.reuse_enabled() {
            return Ok(None);
        }
        let addr = query as *const Query as usize;
        let bound = {
            let mut memos = self.caches.subqueries.lock();
            let Some(entry) = entry_for(&mut memos, addr, query) else {
                return Ok(None);
            };
            let Some(names) = entry.names.clone() else {
                return Ok(None);
            };
            if let Some(last) = &entry.last {
                if same_bindings(&names, ctx, &last.bound) {
                    entry.misses_in_a_row = 0;
                    return Ok(Some(last.rows.clone()));
                }
            }
            if entry.misses_in_a_row >= STORED_MISSES {
                if !same_bindings(&names, ctx, &entry.seen) {
                    record_bindings(&names, ctx, &mut entry.seen);
                    return Ok(None);
                }
                // The previous row's bindings again: worth storing.
                entry.misses_in_a_row = 0;
            }
            let mut bound = Vec::new();
            record_bindings(&names, ctx, &mut bound);
            bound
        };
        // Run outside the lock: the subquery's own subqueries take it too.
        let rows = Arc::new(Value::Array(self.execute_with_parent_context(query, ctx)?));
        if let Some(entry) = self.caches.subqueries.lock().get_mut(&addr) {
            if entry.query == *query {
                entry.last = Some(Computed {
                    bound,
                    rows: rows.clone(),
                    set: None,
                });
                entry.misses_in_a_row = entry.misses_in_a_row.saturating_add(1);
                entry.failed = None;
            }
        }
        Ok(Some(rows))
    }

    /// A hash set over `rows`, which [`Self::memoized_subquery`] just
    /// returned for `query`, built once per stored result. `None` when the
    /// rows are too few to be worth hashing (or are no longer the stored
    /// ones): test membership on the array.
    pub(super) fn memoized_subquery_set(
        &self,
        query: &Query,
        rows: &Arc<Value>,
    ) -> Option<Arc<ValueSet>> {
        let Value::Array(items) = rows.as_ref() else {
            return None;
        };
        if items.len() < MIN_SET_LEN {
            return None;
        }
        let addr = query as *const Query as usize;
        let mut memos = self.caches.subqueries.lock();
        // Same rows means same entry: no need to compare the query again.
        let last = memos
            .get_mut(&addr)?
            .last
            .as_mut()
            .filter(|last| Arc::ptr_eq(&last.rows, rows))?;
        Some(
            last.set
                .get_or_insert_with(|| Arc::new(ValueSet::from_values(items)))
                .clone(),
        )
    }

    /// Whether `query` may read the binding `var` (a loop variable), from the
    /// analysis this query already made of it; nothing is executed. `true`
    /// whenever reuse is off, the subquery is never reusable, or the table is
    /// full: a caller may treat the subquery as independent of `var` (e.g.
    /// plan it as an index lookup value) only on `false`, and then gets its
    /// rows from [`Self::memoized_subquery`].
    pub(super) fn subquery_may_read(&self, query: &Query, var: &str) -> bool {
        if !self.caches.reuse_enabled() {
            return true;
        }
        let addr = query as *const Query as usize;
        let mut memos = self.caches.subqueries.lock();
        match entry_for(&mut memos, addr, query).and_then(|e| e.names.as_deref()) {
            Some(names) => names.iter().any(|n| n == var),
            None => true,
        }
    }

    /// The rows of `query` for the index planner, which has already found it
    /// reusable (see [`Self::subquery_may_read`]): the memoised ones, shared,
    /// or executed once. `None` when it fails, now or (recorded here) earlier
    /// under the same bindings: the planner then leaves the stage to the scan,
    /// whose FILTER evaluates it and treats the error as it always has, and a
    /// stage run again with those bindings (inside a correlated subquery)
    /// does not pay for the failing run a second time.
    pub(super) fn planner_subquery_rows(&self, query: &Query, ctx: &Context) -> Option<Arc<Value>> {
        let addr = query as *const Query as usize;
        {
            let mut memos = self.caches.subqueries.lock();
            let entry = entry_for(&mut memos, addr, query)?;
            let names = entry.names.clone()?;
            if entry
                .failed
                .as_ref()
                .is_some_and(|failed| same_bindings(&names, ctx, failed))
            {
                return None;
            }
        }
        let rows = match self.memoized_subquery(query, ctx) {
            Ok(Some(rows)) => Ok(rows),
            Ok(None) => self
                .execute_with_parent_context(query, ctx)
                .map(|rows| Arc::new(Value::Array(rows))),
            Err(e) => Err(e),
        };
        match rows {
            Ok(rows) => Some(rows),
            Err(_) => {
                let mut memos = self.caches.subqueries.lock();
                if let Some(entry) = memos.get_mut(&addr).filter(|e| e.query == *query) {
                    if let Some(names) = entry.names.clone() {
                        let mut failed = Vec::new();
                        record_bindings(&names, ctx, &mut failed);
                        entry.failed = Some(failed);
                    }
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdbql::parser::parse;
    use crate::storage::StorageEngine;
    use serde_json::json;

    fn subquery(q: &str) -> Query {
        let query = parse(q).unwrap();
        match &query.return_clause.as_ref().unwrap().expression {
            Expression::Subquery(sq) => (**sq).clone(),
            other => panic!("not a subquery: {other:?}"),
        }
    }

    fn names(q: &str) -> Option<Vec<String>> {
        analyse(&subquery(q)).map(|n| n.to_vec())
    }

    #[test]
    fn bare_collect_into_is_never_reused() {
        for q in [
            "RETURN (FOR y IN [1] COLLECT a = y INTO g RETURN g)",
            "RETURN (FOR y IN [1] RETURN (FOR z IN [y] COLLECT a = z INTO g RETURN g))",
            "RETURN (FOR y IN [1] RETURN y UNION FOR z IN [2] COLLECT a = z INTO g RETURN g)",
        ] {
            assert_eq!(names(q), None, "{q}");
        }
        // A CTE body (a `WITH` only opens a top-level query).
        let with =
            parse("WITH t AS (FOR y IN [1] COLLECT a = y INTO g RETURN g) FOR r IN t RETURN r");
        assert!(analyse(&with.unwrap()).is_none());
        for q in [
            "RETURN (FOR y IN [1] COLLECT a = y INTO g KEEP y RETURN g)",
            "RETURN (FOR y IN [1] COLLECT a = y INTO g = y RETURN g)",
        ] {
            assert!(names(q).is_some(), "{q}");
        }
    }

    #[test]
    fn bind_variables_are_read_as_at_names() {
        let n = names("RETURN (FOR d IN 1..3 FILTER d IN @ids RETURN d)").unwrap();
        assert!(n.contains(&"@ids".to_string()), "{n:?}");
    }

    #[test]
    fn a_rebound_first_for_variable_is_not_read() {
        let n = names("RETURN (FOR doc IN cards FILTER doc.c == 1 RETURN doc._key)").unwrap();
        assert!(!n.contains(&"doc".to_string()), "{n:?}");
        for q in [
            "RETURN (FOR doc IN cards SORT doc._key LIMIT doc.n RETURN doc._key)",
            "RETURN (FOR doc IN cards RETURN doc._key UNION FOR c IN cards FILTER c._key == doc.k RETURN c._key)",
            "RETURN (FOR doc IN doc.items RETURN doc)",
            "RETURN (FOR doc IN doc RETURN doc)",
            "RETURN (FOR doc IN cards SYSTEM_TIME AS OF doc.ts RETURN doc._key)",
            "RETURN (LET x = doc.k FOR doc IN cards FILTER doc.k == x RETURN doc._key)",
        ] {
            let n = names(q).unwrap();
            assert!(n.contains(&"doc".to_string()), "{q}: {n:?}");
        }
    }

    /// The memo records bindings without keeping them alive: a binding's
    /// strong count is the same before and after every kind of lookup, so an
    /// in-place write to it never has to copy.
    #[test]
    fn memo_holds_no_strong_reference_to_a_binding() {
        let dir = tempfile::TempDir::new().unwrap();
        let engine = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
        let executor = QueryExecutor::new(&engine);
        executor.caches.enable_reuse(true);
        let q = subquery("RETURN (FOR i IN [x] RETURN i)");
        let row = |n: i64| {
            let mut ctx = Context::default();
            ctx.insert("x".to_string(), json!(n));
            ctx
        };
        let strong = |ctx: &Context| Arc::strong_count(ctx.get_shared("x").unwrap());

        // Stored misses, a hit (which resets the run of misses), stored
        // misses again, then misses that are not stored.
        let stored = |from: i64| {
            let mut last = None;
            for n in from..from + STORED_MISSES as i64 {
                let ctx = row(n);
                let got = executor.memoized_subquery(&q, &ctx).unwrap();
                assert_eq!(got.as_deref(), Some(&json!([n])));
                assert_eq!(strong(&ctx), 1);
                last = Some(ctx);
            }
            last.unwrap()
        };
        let last = stored(0);
        assert_eq!(
            executor.memoized_subquery(&q, &last).unwrap().as_deref(),
            Some(&json!([2]))
        );
        assert_eq!(strong(&last), 1);
        stored(10);
        for n in 20..23 {
            let ctx = row(n);
            assert!(executor.memoized_subquery(&q, &ctx).unwrap().is_none());
            assert_eq!(strong(&ctx), 1);
        }
        assert_eq!(executor.subquery_runs(), 2 * STORED_MISSES as u64);
    }

    /// A value rebound in place (a lambda parameter) moves away from a
    /// recorded address, so it never reads as unchanged.
    #[test]
    fn an_in_place_write_never_looks_unchanged() {
        let dir = tempfile::TempDir::new().unwrap();
        let engine = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
        let executor = QueryExecutor::new(&engine);
        executor.caches.enable_reuse(true);
        let q = subquery("RETURN (FOR i IN [x] RETURN i)");
        let mut ctx = Context::default();
        ctx.insert("x".to_string(), json!(1));
        for n in 1..10 {
            *ctx.get_mut("x").unwrap() = json!(n);
            let got = match executor.memoized_subquery(&q, &ctx).unwrap() {
                Some(rows) => (*rows).clone(),
                None => Value::Array(executor.execute_with_parent_context(&q, &ctx).unwrap()),
            };
            assert_eq!(got, json!([n]));
        }
    }
}
