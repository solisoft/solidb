//! A subquery over an array already in memory:
//! `(FOR x IN <array> [FILTER …]* RETURN <expr>)`.
//!
//! Such a subquery went through the whole pipeline, as a query over a
//! collection does: the parent's bindings copied, every fast path asked
//! whether it applies (a registry read for some), the array copied out of
//! its binding, one `Context` built per element, the FILTERs run on those
//! rows and the RETURN on the survivors. For
//! `AVG((FOR x IN ds FILTER x.v[p] != null RETURN x.v[p]))` over three
//! documents, run 10,000 times by one query, that was 5 µs a run, most of
//! it copying the three documents and four maps of bindings.
//!
//! Here the elements are read where they are and bound, one after the
//! other, in a single copy of the parent's bindings. The rows, their
//! order, the FILTERs' meaning (`filter_row`) and the errors are those of
//! the pipeline. Taken only for exactly that shape: one FOR over an
//! expression or over a variable of the row, FILTERs, a RETURN that is not
//! DISTINCT; no window function and nothing that writes. A name that is
//! also a collection of the database is left to the pipeline, which
//! decides which of the two it reads.

use std::sync::Arc;

use serde_json::Value;

use super::super::types::Context;
use super::super::{contains_window_functions, QueryExecutor};
use crate::error::DbResult;
use crate::sdbql::ast::{expression_mutates, BodyClause, Expression, ForClause, Query};

/// A binding that holds an array, and that array's elements each behind an
/// `Arc` (see [`QueryExecutor::shared_elements`]).
pub(in crate::sdbql::executor) type SharedElements = (Arc<Value>, Arc<[Arc<Value>]>);

/// The parts of a subquery [`QueryExecutor::try_array_subquery`] runs.
struct Plan<'q> {
    for_clause: &'q ForClause,
    /// The clauses after the FOR: FILTERs only.
    filters: &'q [BodyClause],
    returns: &'q Expression,
}

/// The plan for `query`, from its shape alone.
fn plan(query: &Query) -> Option<Plan<'_>> {
    let return_clause = query.return_clause.as_ref()?;
    if return_clause.distinct
        || query.with_clause.is_some()
        || !query.let_clauses.is_empty()
        || !query.join_clauses.is_empty()
        || query.sort_clause.is_some()
        || query.limit_clause.is_some()
        || query.create_stream_clause.is_some()
        || query.create_materialized_view_clause.is_some()
        || query.refresh_materialized_view_clause.is_some()
        || query.window_clause.is_some()
        || !query.post_limit_lets.is_empty()
        || !query.set_operations.is_empty()
        || query.needs_full_pipeline()
    {
        return None;
    }
    let (first, filters) = query.body_clauses.split_first()?;
    let BodyClause::For(for_clause) = first else {
        return None;
    };
    if for_clause.system_time.is_some()
        || for_clause.valid_time.is_some()
        || for_clause.options.is_some()
    {
        return None;
    }
    let plain = |e: &Expression| !contains_window_functions(e) && !expression_mutates(e);
    let filters_plain = filters.iter().all(|c| match c {
        BodyClause::Filter(f) => plain(&f.expression),
        _ => false,
    });
    if !filters_plain || !plain(&return_clause.expression) {
        return None;
    }
    if for_clause
        .source_expression
        .as_ref()
        .is_some_and(|e| !plain(e))
    {
        return None;
    }
    Some(Plan {
        for_clause,
        filters,
        returns: &return_clause.expression,
    })
}

/// The elements a FOR iterates.
enum Elements<'v> {
    /// Where they are: in a binding, a bind variable or the query's text.
    Lent(&'v [Value]),
    /// Each behind its own `Arc`, shared by every run over the same array.
    Shared(Arc<[Arc<Value>]>),
    /// Computed for this run.
    Owned(Vec<Value>),
}

impl Elements<'_> {
    fn len(&self) -> usize {
        match self {
            Elements::Lent(items) => items.len(),
            Elements::Shared(items) => items.len(),
            Elements::Owned(items) => items.len(),
        }
    }

    /// Bind element `i` to `var` in `row`.
    fn bind(&self, i: usize, var: &str, row: &mut Context) {
        match self {
            Elements::Lent(items) => row.set_shared(var, Arc::new(items[i].clone())),
            Elements::Shared(items) => row.set_shared(var, items[i].clone()),
            Elements::Owned(items) => row.set_shared(var, Arc::new(items[i].clone())),
        }
    }
}

impl QueryExecutor<'_> {
    /// The rows of the subquery `query` under `parent`, when it only walks
    /// an array in memory (see the module); `Ok(None)` when the pipeline
    /// has to run it.
    pub(super) fn try_array_subquery(
        &self,
        query: &Query,
        parent: &Context,
    ) -> DbResult<Option<Vec<Value>>> {
        if !self.array_subqueries {
            return Ok(None);
        }
        let Some(plan) = plan(query) else {
            return Ok(None);
        };
        let for_clause = plan.for_clause;
        let elements = match &for_clause.source_expression {
            Some(expr) => match self.resolve_ref(expr, parent) {
                Some(Value::Array(items)) => Elements::Lent(items),
                Some(other) => Elements::Lent(std::slice::from_ref(other)),
                // Not a value that is held somewhere: computed, with the
                // error it may raise.
                None => match self.evaluate_expr_with_context(expr, parent)? {
                    Value::Array(items) => Elements::Owned(items),
                    other => Elements::Owned(vec![other]),
                },
            },
            None => {
                let name = for_clause
                    .source_variable
                    .as_ref()
                    .unwrap_or(&for_clause.collection);
                let Some(binding) = parent.get_shared(name) else {
                    // A collection.
                    return Ok(None);
                };
                // `FOR x IN name` where a collection has that name too: the
                // pipeline asks some of its index and columnar reads before
                // it looks at the row.
                if name == &for_clause.collection && self.names_a_collection(name) {
                    return Ok(None);
                }
                match binding.as_ref() {
                    Value::Array(items) => match self.shared_elements(binding, items) {
                        Some(shared) => Elements::Shared(shared),
                        None => Elements::Lent(items),
                    },
                    other => Elements::Lent(std::slice::from_ref(other)),
                }
            }
        };
        // As many rows as the pipeline would have built.
        self.check_budget(elements.len())?;

        let var = for_clause.variable.as_str();
        let mut row = parent.clone();
        let mut out = Vec::with_capacity(elements.len());
        let filter = |c: &BodyClause, row: &Context| match c {
            BodyClause::Filter(f) => self.filter_row(&f.expression, row),
            _ => unreachable!("only FILTERs are planned"),
        };
        if plan.filters.len() <= 1 {
            for i in 0..elements.len() {
                elements.bind(i, var, &mut row);
                if plan.filters.iter().all(|c| filter(c, &row)) {
                    out.push(self.evaluate_expr_with_context(plan.returns, &row)?);
                }
            }
        } else {
            // Clause by clause, as the pipeline runs them: which FILTER
            // meets a mistake of the query first depends on it.
            let mut kept: Vec<usize> = (0..elements.len()).collect();
            for clause in plan.filters {
                kept.retain(|&i| {
                    elements.bind(i, var, &mut row);
                    filter(clause, &row)
                });
            }
            for i in kept {
                elements.bind(i, var, &mut row);
                out.push(self.evaluate_expr_with_context(plan.returns, &row)?);
            }
        }
        Ok(Some(out))
    }

    /// The elements of the array `items` held by `binding`, each behind an
    /// `Arc`, when copying them for every run would cost: an array of
    /// documents iterated by one subquery after another (`FOR x IN ds`,
    /// once per perimeter). `None` for an array of scalars, which are
    /// cheaper to copy than to share.
    fn shared_elements(&self, binding: &Arc<Value>, items: &[Value]) -> Option<Arc<[Arc<Value>]>> {
        /// Arrays remembered at once: nested loops walk a few in turn.
        const KEPT: usize = 4;
        if !items.iter().any(|v| v.is_object() || v.is_array()) {
            return None;
        }
        let slots = &self.caches.array_elements;
        let mut kept = slots[thread_slot() % slots.len()].lock();
        if let Some((_, elements)) = kept.iter().find(|(held, _)| Arc::ptr_eq(held, binding)) {
            return Some(elements.clone());
        }
        let elements: Arc<[Arc<Value>]> = items.iter().cloned().map(Arc::new).collect();
        if kept.len() == KEPT {
            kept.remove(0);
        }
        // The binding is held with them: its address names this array for
        // as long as the entry lives, and a write to it elsewhere copies.
        kept.push((binding.clone(), elements.clone()));
        Some(elements)
    }
}

/// A number for the running thread, the same at every call.
fn thread_slot() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static SLOT: usize = NEXT.fetch_add(1, Ordering::Relaxed);
    }
    SLOT.with(|slot| *slot)
}
