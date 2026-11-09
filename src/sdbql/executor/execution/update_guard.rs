//! `UPDATE` / `REPLACE` guarded by the revision the query read (card #796).
//!
//! `FOR d IN c FILTER … UPDATE d WITH {n: d.n + 1} IN c` evaluates `WITH`
//! against the document as the `FOR` read it, then writes under the key's
//! lock. A write that landed in between used to be overwritten by a value
//! computed from the older version: ten clients appending with `PUSH` on a
//! few hot keys lost about one append in four.
//!
//! When the `UPDATE` names a document the query read (`UPDATE d …` or
//! `UPDATE d._key …`, `d` a stored document of the collection being
//! written), its `_rev` goes with the write and is compared under the lock.
//! If the document moved on, the row is evaluated again, this time holding
//! the key's write lock, as PostgreSQL waits for the row and re-evaluates it:
//! the `FILTER`s, `SEARCH`es and `LET`s between the `FOR` that bound `d` and
//! the `UPDATE` run again on the current version, then `WITH`, and the
//! result is written before the lock is released, so nothing can overtake
//! it. A row that no longer passes its filters is left alone (two workers
//! claiming `FILTER d.status == 'pending'` cannot both take the same job). A
//! pipeline that cannot be re-run for one row — a nested `FOR`, a `COLLECT`,
//! a join, an expression that writes — fails the statement with a conflict
//! rather than lose the write. Inside a transaction the storage refuses the
//! stale write itself (`update_tx_if_unchanged`).

use serde_json::Value;

use super::super::types::Context;
use super::super::{contains_window_functions, QueryExecutor};
use super::mutations::{selector_key, RowGuard, RowWrite};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{expression_mutates, BodyClause, Expression, UpdateClause};
use crate::storage::Collection;

/// The variable and the `_rev` of the document an `UPDATE`'s selector
/// names, when the query read it: `UPDATE d …` or `UPDATE d._key …` with `d`
/// bound to a stored document of `collection` whose key is `key`.
pub(crate) fn update_guard(
    selector: &Expression,
    ctx: &Context,
    key: &str,
    collection: &Collection,
) -> Option<(String, String)> {
    let var = match selector {
        Expression::Variable(v) => v,
        Expression::FieldAccess(base, field) | Expression::OptionalFieldAccess(base, field)
            if field == "_key" =>
        {
            match base.as_ref() {
                Expression::Variable(v) => v,
                _ => return None,
            }
        }
        _ => return None,
    };
    let doc = ctx.get(var)?.as_object()?;
    if doc.get("_key").and_then(Value::as_str) != Some(key) {
        return None;
    }
    let id = doc.get("_id").and_then(Value::as_str)?;
    if !id
        .strip_prefix(collection.name.as_str())
        .is_some_and(|rest| rest.starts_with('/'))
    {
        return None;
    }
    let rev = doc.get("_rev").and_then(Value::as_str)?;
    Some((var.clone(), rev.to_string()))
}

impl QueryExecutor<'_> {
    /// Write one row of an `UPDATE` / `REPLACE`: guarded by the revision it
    /// read, and evaluated again under the key's lock when that document has
    /// moved on (or, with `stale`, straight away). `None` when the row no
    /// longer passes its filters, or its document is gone.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn update_row_guarded(
        &self,
        clauses: &[BodyClause],
        at: usize,
        update: &UpdateClause,
        collection: &Collection,
        ctx: Context,
        statement: &str,
        stale: bool,
    ) -> DbResult<Option<(RowWrite, Context)>> {
        let selector_value = self.evaluate_expr_with_context(&update.selector, &ctx)?;
        let key = selector_key(&selector_value, statement)?;
        let guard = update_guard(&update.selector, &ctx, &key, collection);
        if !stale {
            let changes = if update.changes == update.selector {
                selector_value
            } else {
                self.evaluate_expr_with_context(&update.changes, &ctx)?
            };
            let row_guard = match &guard {
                Some((_, rev)) => RowGuard::IfUnchanged(rev),
                None => RowGuard::None,
            };
            if let Some(write) = self.write_update_row(
                collection,
                &update.collection,
                &key,
                changes,
                update.replace,
                &update.options,
                update.binds_old,
                row_guard,
            )? {
                return Ok(Some((write, ctx)));
            }
        }
        // The document moved on since the query read it.
        let Some((var, read_rev)) = guard else {
            return Err(DbError::InternalError(format!(
                "{statement}: an unguarded write reported a conflict"
            )));
        };
        let start = self.rerun_start(clauses, at, update, &var).map_err(|why| {
            DbError::ConflictError(format!(
                "{statement}: document '{key}' of {} changed while this query ran \
                 (read at revision '{read_rev}'), and {why}; retry the query",
                update.collection
            ))
        })?;
        collection.with_write_lock(&key, || {
            let current = match collection.get(&key) {
                Ok(doc) => doc.into_value(),
                Err(DbError::DocumentNotFound(_)) => return Ok(None),
                Err(e) => return Err(e),
            };
            let mut fresh = ctx.clone();
            fresh.insert(var.clone(), current);
            if !self.rerun_row(&clauses[start + 1..at], &mut fresh)? {
                return Ok(None);
            }
            let selector_value = self.evaluate_expr_with_context(&update.selector, &fresh)?;
            if selector_key(&selector_value, statement)? != key {
                return Err(DbError::ConflictError(format!(
                    "{statement}: document '{key}' of {} changed while this query ran, and the \
                     row now names another document; retry the query",
                    update.collection
                )));
            }
            let changes = if update.changes == update.selector {
                selector_value
            } else {
                self.evaluate_expr_with_context(&update.changes, &fresh)?
            };
            let write = self
                .write_update_row(
                    collection,
                    &update.collection,
                    &key,
                    changes,
                    update.replace,
                    &update.options,
                    update.binds_old,
                    RowGuard::Locked,
                )?
                .ok_or_else(|| {
                    DbError::InternalError(format!("{statement}: a locked write did not write"))
                })?;
            Ok(Some((write, fresh)))
        })
    }

    /// Where the row that bound `var` starts: the last `FOR` over it before
    /// clause `at`, when everything between it and the `UPDATE` can be
    /// evaluated again for one row without writing anything. Otherwise why
    /// not.
    fn rerun_start(
        &self,
        clauses: &[BodyClause],
        at: usize,
        update: &UpdateClause,
        var: &str,
    ) -> Result<usize, &'static str> {
        let start = clauses[..at]
            .iter()
            .rposition(|c| matches!(c, BodyClause::For(f) if f.variable == var))
            .ok_or("the variable was not bound by a FOR of this query")?;
        if expression_mutates(&update.selector) || expression_mutates(&update.changes) {
            return Err("its UPDATE expressions write, so they cannot run twice");
        }
        for clause in &clauses[start + 1..at] {
            let ok = match clause {
                BodyClause::Filter(f) | BodyClause::Search(f) => !expression_mutates(&f.expression),
                BodyClause::Let(l) => {
                    !contains_window_functions(&l.expression) && !expression_mutates(&l.expression)
                }
                _ => false,
            };
            if !ok {
                return Err(
                    "the clauses between its FOR and the write cannot be evaluated again for one row",
                );
            }
        }
        Ok(start)
    }

    /// Evaluate the `FILTER`s, `SEARCH`es and `LET`s of `between` on `ctx`
    /// again; `false` when the row no longer passes.
    fn rerun_row(&self, between: &[BodyClause], ctx: &mut Context) -> DbResult<bool> {
        for clause in between {
            match clause {
                BodyClause::Filter(f) => {
                    if !self
                        .evaluate_filter_with_context(&f.expression, ctx)
                        .unwrap_or(false)
                    {
                        return Ok(false);
                    }
                }
                BodyClause::Search(f) => {
                    let value = self.evaluate_expr_with_context(&f.expression, ctx)?;
                    let score = match &value {
                        Value::Bool(b) => f64::from(u8::from(*b)),
                        Value::Number(n) => n.as_f64().unwrap_or(0.0),
                        Value::Null => 0.0,
                        other => f64::from(u8::from(super::super::to_bool(other))),
                    };
                    if score <= 0.0 {
                        return Ok(false);
                    }
                    ctx.insert("__search_score".into(), serde_json::json!(score));
                }
                BodyClause::Let(l) => self.bind_let(l, ctx)?,
                _ => unreachable!("rerun_start admits FILTER, SEARCH and LET only"),
            }
        }
        Ok(true)
    }
}
