//! `FOR … SAMPLE n` and `FOR … SAMPLE p PERCENT`.
//!
//! The sample is of the loop's source, before FILTER, SORT and LIMIT. The
//! whole source is read first: a count sample then keeps `n` rows by a
//! partial shuffle, and a percent sample keeps each row on its own. Neither
//! stops the scan early. What follows the FOR sees only the rows kept.

use rand::seq::SliceRandom;
use rand::Rng;
use serde_json::Value;

use super::super::types::Context;
use super::super::QueryExecutor;
use super::BUDGET_CHECK_INTERVAL;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{ForClause, SampleSpec};

impl<'a> QueryExecutor<'a> {
    /// One pass of `FOR`, then the sample that clause asked for.
    pub(super) fn for_then_sample(
        &self,
        for_clause: &ForClause,
        rows: &[Context],
    ) -> DbResult<Vec<Context>> {
        let Some(spec) = &for_clause.sample else {
            return Ok(rows.to_vec());
        };
        // A columnar collection is read through `columnar_for_rows`, which
        // applies its row policy: `get_for_source_docs` reads its rows
        // without one.
        let is_collection = for_clause
            .source_variable
            .as_ref()
            .is_none_or(|src| src == &for_clause.collection);
        let mut new_rows = Vec::new();
        for ctx in rows {
            // No scan limit: SAMPLE reads the source, and LIMIT comes after.
            let columnar = if is_collection {
                self.columnar_for_rows(for_clause, &[], ctx, None)?
            } else {
                None
            };
            let docs = match columnar {
                Some(docs) => docs,
                None => self.get_for_source_docs(for_clause, ctx, None)?,
            };
            // A scan stops one past the row ceiling: drawn from, that cut
            // would give a sample of the first rows only. The source is held
            // whole here, so it is what the ceiling bounds, as for a FOR.
            self.check_budget(docs.len())?;
            let sampled = take_sample(self, spec, docs, ctx)?;
            for doc in sampled {
                let mut new_ctx = ctx.clone();
                new_ctx.insert(for_clause.variable.clone(), doc);
                new_rows.push(new_ctx);
                if new_rows.len() % BUDGET_CHECK_INTERVAL == 0 {
                    self.check_budget(new_rows.len())?;
                }
            }
        }
        self.check_budget(new_rows.len())?;
        Ok(new_rows)
    }
}

fn take_sample(
    ex: &QueryExecutor<'_>,
    spec: &SampleSpec,
    mut docs: Vec<Value>,
    ctx: &Context,
) -> DbResult<Vec<Value>> {
    match spec {
        SampleSpec::Count(expr) => {
            let n = whole_count(&ex.evaluate_expr_with_context(expr, ctx)?)?;
            if n == 0 {
                return Ok(Vec::new());
            }
            if n >= docs.len() {
                return Ok(docs);
            }
            let mut rng = rand::thread_rng();
            // rand 0.8 draws the `n` rows into the *end* of the slice (and
            // returns that part first). Keeping the first `n` of `docs` kept
            // rows the draw had mostly left alone: not uniform.
            let rest = docs.len() - n;
            docs.partial_shuffle(&mut rng, n);
            Ok(docs.split_off(rest))
        }
        SampleSpec::Percent(expr) => {
            let p = percent(&ex.evaluate_expr_with_context(expr, ctx)?)?;
            if p == 0.0 {
                return Ok(Vec::new());
            }
            // `gen` is in `[0, 1)`, so 100 keeps every row.
            if p >= 100.0 {
                return Ok(docs);
            }
            let mut rng = rand::thread_rng();
            let threshold = p / 100.0;
            docs.retain(|_| rng.gen::<f64>() < threshold);
            Ok(docs)
        }
    }
}

/// A non-negative whole number. `1.0` is 1. A fraction, a negative, or a
/// non-number fails the query.
fn whole_count(value: &Value) -> DbResult<usize> {
    let Some(n) = value.as_f64() else {
        return Err(DbError::ExecutionError(format!(
            "SAMPLE count must be a non-negative whole number, got {value}"
        )));
    };
    if !n.is_finite() || n < 0.0 || n.fract() != 0.0 || n > usize::MAX as f64 {
        return Err(DbError::ExecutionError(format!(
            "SAMPLE count must be a non-negative whole number, got {value}"
        )));
    }
    Ok(n as usize)
}

fn percent(value: &Value) -> DbResult<f64> {
    let Some(n) = value.as_f64() else {
        return Err(DbError::ExecutionError(format!(
            "SAMPLE percent must be a number from 0 to 100, got {value}"
        )));
    };
    if !n.is_finite() || !(0.0..=100.0).contains(&n) {
        return Err(DbError::ExecutionError(format!(
            "SAMPLE percent must be a number from 0 to 100, got {value}"
        )));
    }
    Ok(n)
}
