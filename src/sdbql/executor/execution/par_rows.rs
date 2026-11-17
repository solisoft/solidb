//! A LET or a RETURN evaluated for many rows on several threads.
//!
//! The pipeline works a clause at a time: a `LET v = (subquery)` is
//! evaluated for every row, then the next clause for every row, and the
//! RETURN last. Each row's value depends on that row alone, yet they were
//! computed one after the other on the query's thread:
//! `FOR m IN @metrics LET ds = DOCUMENT(…) RETURN ZIP(…, (FOR p IN … RETURN
//! AVG((FOR x IN ds …))))` spent 25 ms on 500 rows with seven cores idle.
//!
//! Here the rows are cut into as many slices as the scan threads allow
//! (`scan_threads`: `SOLIDB_SCAN_THREADS`, shared by every query of the
//! process) and each slice is evaluated on its own thread, in row order.
//! The values are those of one thread, in the same order, because this is
//! only done for an expression that
//!
//! - writes nothing, here or in a subquery;
//! - calls only pure builtins and reads (`DOCUMENT`, `COLLECTION_COUNT`),
//!   the functions a subquery may be memoised with: no `RAND`, no user
//!   function, no window function;
//! - holds a subquery or a `DOCUMENT` call, for at least [`MIN_ROWS`]
//!   rows: anything lighter costs less than the threads;
//!
//! and never inside a transaction, whose reads are tracked and staged per
//! query. An error is the first row's that fails, as on one thread. A
//! FILTER inside a subquery that meets a mistake of the query records it
//! (`filter_row`), and which of two mistakes is met first depends on the
//! order: when a run on several threads records one, it is forgotten and
//! the clause is run again on this thread, which then decides.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use super::super::subquery_memo::is_nondeterministic;
use super::super::types::Context;
use super::super::QueryExecutor;
use super::scan_threads::{configured_threads, Helpers};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{expression_any, Expression, LetClause};

/// Fewer rows than this are evaluated on the query's thread.
const MIN_ROWS: usize = 256;

/// Fewer rows than this per thread are not worth a thread.
const MIN_ROWS_PER_THREAD: usize = 64;

/// Whether `expr` may be evaluated for different rows at once, and is
/// worth it (see the module).
fn spreads(expr: &Expression) -> bool {
    let mut heavy = false;
    let tied = expression_any(expr, &mut |e| {
        match e {
            Expression::Subquery(q) => {
                heavy = true;
                if q.has_mutations() {
                    return true;
                }
            }
            Expression::FunctionCall { name, .. } if name.eq_ignore_ascii_case("DOCUMENT") => {
                heavy = true;
            }
            _ => {}
        }
        is_nondeterministic(e)
    });
    heavy && !tied
}

impl QueryExecutor<'_> {
    /// The helper threads to evaluate `expr` for `rows` rows with; `None`
    /// when this thread evaluates them all.
    fn spread(&self, rows: usize, expr: &Expression) -> Option<Helpers> {
        if rows < MIN_ROWS || self.tx_writer.is_some() {
            return None;
        }
        let threads = self
            .scan
            .threads
            .unwrap_or_else(configured_threads)
            .min(rows / MIN_ROWS_PER_THREAD);
        if threads < 2 || !spreads(expr) {
            return None;
        }
        let helpers = Helpers::take(threads - 1);
        (helpers.count() > 0).then_some(helpers)
    }

    /// Whether no FILTER has recorded a mistake of the query yet.
    fn no_mistake_yet(&self) -> bool {
        self.caches.filter_mistake.lock().is_none()
    }

    /// Run `work` on each slice of `rows` (cut for `helpers`), the first
    /// one here and the others on their own threads, and return what each
    /// gave, in order. `None` when the run cannot be trusted and the
    /// caller must do the work on this thread: a thread could not be
    /// started, or a mistake of the query was recorded meanwhile (and is
    /// forgotten here).
    fn on_slices<S: Send, T: Send>(
        &self,
        slices: Vec<S>,
        helpers: Helpers,
        work: impl Fn(S, &AtomicBool) -> DbResult<T> + Sync,
    ) -> Option<Vec<DbResult<T>>> {
        let clean = self.no_mistake_yet();
        let stop = AtomicBool::new(false);
        let results: Option<Vec<DbResult<T>>> = std::thread::scope(|s| {
            let (work, stop) = (&work, &stop);
            let run = move |slice: S| {
                let result = work(slice, stop);
                if result.is_err() {
                    stop.store(true, Ordering::Relaxed);
                }
                result
            };
            let mut slices = slices.into_iter();
            let first = slices.next()?;
            let others: Vec<_> = slices
                .map(|slice| {
                    std::thread::Builder::new()
                        .name("solidb-scan".to_string())
                        .spawn_scoped(s, move || run(slice))
                })
                .collect();
            let mut results = vec![run(first)];
            let mut started = true;
            for handle in others {
                match handle {
                    Ok(h) => results.push(h.join().unwrap_or_else(|_| {
                        Err(DbError::InternalError("a scan thread panicked".to_string()))
                    })),
                    Err(_) => started = false,
                }
            }
            started.then_some(results)
        });
        drop(helpers);
        if clean && !self.no_mistake_yet() {
            *self.caches.filter_mistake.lock() = None;
            return None;
        }
        results
    }

    /// Evaluate `let_clause` into each of `rows`, as `bind_let` row after
    /// row does; on several threads when the expression allows it.
    pub(super) fn bind_let_rows(
        &self,
        let_clause: &LetClause,
        rows: &mut [Context],
    ) -> DbResult<()> {
        if let Some(helpers) = self.spread(rows.len(), &let_clause.expression) {
            let size = rows.len().div_ceil(helpers.count() + 1);
            let slices: Vec<&mut [Context]> = rows.chunks_mut(size).collect();
            let done = self.on_slices(slices, helpers, |slice, stop| {
                for ctx in slice {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    self.bind_let(let_clause, ctx)?;
                }
                Ok(())
            });
            if let Some(results) = done {
                // The first row that failed, in row order.
                return results.into_iter().collect();
            }
        }
        for ctx in rows {
            self.bind_let(let_clause, ctx)?;
        }
        Ok(())
    }

    /// `expr` evaluated for each of `rows`, in order, on several threads;
    /// `None` when they are to be evaluated on this thread.
    pub(super) fn return_rows(
        &self,
        expr: &Expression,
        rows: &[Context],
    ) -> Option<DbResult<Vec<Value>>> {
        let helpers = self.spread(rows.len(), expr)?;
        let size = rows.len().div_ceil(helpers.count() + 1);
        let slices: Vec<&[Context]> = rows.chunks(size).collect();
        let results = self.on_slices(slices, helpers, |slice, stop| {
            let mut out = Vec::with_capacity(slice.len());
            for ctx in slice {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                out.push(self.evaluate_expr_with_context(expr, ctx)?);
            }
            Ok(out)
        })?;
        let mut out = Vec::with_capacity(rows.len());
        for result in results {
            match result {
                Ok(mut values) => out.append(&mut values),
                Err(e) => return Some(Err(e)),
            }
        }
        Some(Ok(out))
    }
}
