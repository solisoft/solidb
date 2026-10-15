//! Counts answered from an index alone.
//!
//! `FOR v IN c FILTER v.f == x COLLECT WITH COUNT INTO n RETURN …`, and
//! `LENGTH(FOR v IN c FILTER v.f == x RETURN 1)` (how the Soli ORM counts a
//! `where`), need no document when a single-field index serves `v.f`: the
//! index holds one entry per matching document, so counting the entries under
//! `x` is the answer.
//! The ordinary path reads every matching document and evaluates the FILTER
//! on it again; at 5,000 matches that is 5,000 reads and decodes for one
//! number. Same guards as that path — the protected-collection check, row
//! policies, search views, columnar and sharded collections, the deadline —
//! and anything else is handed back to it.

use serde_json::Value;

use super::super::types::{Context, MutationStats, QueryExecutionResult};
use super::super::QueryExecutor;
use crate::error::DbResult;
use crate::sdbql::ast::{BinaryOperator, BodyClause, Expression, ForClause, Query};

/// True when the query block is nothing but its body and RETURN.
fn bare(query: &Query) -> bool {
    query.set_operations.is_empty()
        && query.join_clauses.is_empty()
        && query.with_clause.is_none()
        && query.window_clause.is_none()
        && query.create_stream_clause.is_none()
        && query.create_materialized_view_clause.is_none()
        && query.refresh_materialized_view_clause.is_none()
        && query.let_clauses.is_empty()
        && query.post_limit_lets.is_empty()
        && query.sort_clause.is_none()
        && query.limit_clause.is_none()
}

/// `FOR v IN c FILTER <one equality>` over a collection, no options.
fn indexed_loop(for_clause: &ForClause, filter: &Expression) -> bool {
    for_clause.source_expression.is_none()
        && for_clause.system_time.is_none()
        && for_clause.valid_time.is_none()
        && for_clause.options.is_none()
        && for_clause
            .source_variable
            .as_ref()
            .is_none_or(|s| s == &for_clause.collection)
        // One equality, nothing else: the index entries are then exactly
        // the rows the FILTER accepts.
        && matches!(
            filter,
            Expression::BinaryOp {
                op: BinaryOperator::Equal,
                ..
            }
        )
}

/// The loop, the FILTER expression and the count variable of
/// `FOR … FILTER … COLLECT WITH COUNT INTO n RETURN …`.
fn count_plan(query: &Query) -> Option<(&ForClause, &Expression, &str)> {
    if !bare(query) {
        return None;
    }
    let [BodyClause::For(for_clause), BodyClause::Filter(filter), BodyClause::Collect(collect)] =
        query.body_clauses.as_slice()
    else {
        return None;
    };
    if !indexed_loop(for_clause, &filter.expression)
        || !collect.group_vars.is_empty()
        || collect.into_var.is_some()
        || !collect.keep_vars.is_empty()
        || !collect.aggregates.is_empty()
        || collect.into_expr.is_some()
    {
        return None;
    }
    let count_var = collect.count_var.as_deref()?;
    query.return_clause.as_ref()?;
    Some((for_clause, &filter.expression, count_var))
}

/// The loop and the FILTER expression of a `FOR … FILTER … RETURN e` whose
/// row count is all that is asked for: one row per document, whatever `e`
/// is, as long as `e` cannot fail — a literal, the loop variable, or a field
/// of it.
fn rows_plan(query: &Query) -> Option<(&ForClause, &Expression)> {
    if !bare(query) {
        return None;
    }
    let [BodyClause::For(for_clause), BodyClause::Filter(filter)] = query.body_clauses.as_slice()
    else {
        return None;
    };
    if !indexed_loop(for_clause, &filter.expression) {
        return None;
    }
    let ret = query.return_clause.as_ref()?;
    let var = for_clause.variable.as_str();
    let infallible = match &ret.expression {
        Expression::Literal(_) => true,
        Expression::Variable(v) => v == var,
        Expression::FieldAccess(base, _) | Expression::OptionalFieldAccess(base, _) => {
            matches!(base.as_ref(), Expression::Variable(v) if v == var)
        }
        _ => false,
    };
    (infallible && !ret.distinct).then_some((for_clause, &filter.expression))
}

/// A whole number as an integer (`30.0` → `30`), anything else as it is: the
/// form an index lookup tries first.
pub(super) fn whole_number_as_integer(value: &Value) -> Value {
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.is_finite() => {
                Value::Number(serde_json::Number::from(f as i64))
            }
            _ => value.clone(),
        },
        _ => value.clone(),
    }
}

impl QueryExecutor<'_> {
    /// The query's result, or `Ok(None)` when it has to be run the ordinary
    /// way.
    pub(super) fn try_index_count(
        &self,
        query: &Query,
        bindings: &Context,
    ) -> DbResult<Option<QueryExecutionResult>> {
        let Some((for_clause, filter, count_var)) = count_plan(query) else {
            return Ok(None);
        };
        let Some(count) = self.index_count(for_clause, filter, bindings)? else {
            return Ok(None);
        };
        // After COLLECT, the row holds the count variable and nothing else.
        let mut ctx = Context::default();
        ctx.insert(count_var.to_string(), Value::Number(count.into()));
        let Some(ref ret) = query.return_clause else {
            return Ok(None);
        };
        let value = self.evaluate_expr_with_context(&ret.expression, &ctx)?;
        Ok(Some(QueryExecutionResult {
            results: vec![value],
            mutations: MutationStats::new(),
        }))
    }

    /// How many rows `query`, a subquery evaluated against `ctx`, returns —
    /// what `LENGTH(…)` of it is — or `Ok(None)` when it has to be run.
    pub(in crate::sdbql::executor) fn try_index_count_rows(
        &self,
        query: &Query,
        ctx: &Context,
    ) -> DbResult<Option<u64>> {
        let Some((for_clause, filter)) = rows_plan(query) else {
            return Ok(None);
        };
        self.index_count(for_clause, filter, ctx)
    }

    /// The documents of `FOR v IN c FILTER v.f == x`, counted in the index on
    /// `f`; `Ok(None)` when no such index serves it, or when a guard sends the
    /// query down the ordinary path.
    fn index_count(
        &self,
        for_clause: &ForClause,
        filter: &Expression,
        bindings: &Context,
    ) -> DbResult<Option<u64>> {
        let name = &for_clause.collection;
        // A LET-bound array of that name is iterated, not the collection.
        if bindings.contains_key(name) || self.row_policy_applies(name) {
            return Ok(None);
        }
        let Some(ref db_name) = self.database else {
            return Ok(None);
        };
        let Ok(database) = self.storage.get_database(db_name) else {
            return Ok(None);
        };
        if database.is_columnar_collection(name)
            || !matches!(self.resolve_search_view_collection(name), Ok(None))
        {
            return Ok(None);
        }
        // Also the protected-collection guard: an error here is reported by
        // the ordinary path.
        let Ok(collection) = self.get_collection(name) else {
            return Ok(None);
        };
        if collection
            .get_shard_config()
            .is_some_and(|c| c.num_shards > 0)
        {
            return Ok(None);
        }
        let Some(cond) = self.extract_indexable_condition(filter, &for_clause.variable, bindings)
        else {
            return Ok(None);
        };
        // `_key` and `_id` are served by the primary key, not an index.
        if cond.op != BinaryOperator::Equal || cond.field == "_key" || cond.field == "_id" {
            return Ok(None);
        }
        // The ordinary path looks a whole number up as an integer first, then
        // as written (see `use_index_for_condition`); so does this one.
        let normalized = whole_number_as_integer(&cond.value);
        let check = || self.check_budget(0);
        Ok(
            match collection.index_count_eq(&cond.field, &normalized, check)? {
                Some(0) if normalized != cond.value => {
                    collection.index_count_eq(&cond.field, &cond.value, check)?
                }
                other => other,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdbql::parser::parse;

    fn plans(q: &str) -> bool {
        count_plan(&parse(q).unwrap()).is_some()
    }

    fn counts_rows(q: &str) -> bool {
        rows_plan(&parse(q).unwrap()).is_some()
    }

    #[test]
    fn row_counts_it_answers() {
        for q in [
            "FOR doc IN c FILTER doc.status == @s RETURN 1",
            "FOR doc IN c FILTER doc.status == 'x' RETURN doc",
            "FOR doc IN c FILTER doc.status == 'x' RETURN doc.name",
        ] {
            assert!(counts_rows(q), "{q}");
        }
        for q in [
            "FOR doc IN c FILTER doc.status == 'x' RETURN DISTINCT 1",
            "FOR doc IN c FILTER doc.status == 'x' RETURN doc.a / 0",
            "FOR doc IN c FILTER doc.status == 'x' RETURN other",
            "FOR doc IN c FILTER doc.status == 'x' LIMIT 3 RETURN 1",
            "FOR doc IN c FILTER doc.status != 'x' RETURN 1",
            "FOR doc IN c RETURN 1",
        ] {
            assert!(!counts_rows(q), "{q}");
        }
    }

    #[test]
    fn shapes_it_answers() {
        assert!(plans(
            "FOR d IN c FILTER d.g == @g COLLECT WITH COUNT INTO n RETURN n"
        ));
        assert!(plans(
            "FOR d IN c FILTER 3 == d.a.b COLLECT WITH COUNT INTO n RETURN {total: n}"
        ));
    }

    #[test]
    fn everything_else_is_left_to_the_ordinary_path() {
        for q in [
            "FOR d IN c COLLECT WITH COUNT INTO n RETURN n",
            "FOR d IN c FILTER d.g > 1 COLLECT WITH COUNT INTO n RETURN n",
            "FOR d IN c FILTER d.g == 1 AND d.h == 2 COLLECT WITH COUNT INTO n RETURN n",
            "FOR d IN c FILTER d.g == 1 FILTER d.h == 2 COLLECT WITH COUNT INTO n RETURN n",
            "FOR d IN c FILTER d.g == 1 COLLECT a = d.a WITH COUNT INTO n RETURN n",
            "FOR d IN c FILTER d.g == 1 COLLECT AGGREGATE s = SUM(d.x) RETURN s",
            "FOR d IN c FILTER d.g == 1 COLLECT WITH COUNT INTO n SORT n LIMIT 1 RETURN n",
            "FOR d IN c FILTER d.g == 1 LET x = 1 COLLECT WITH COUNT INTO n RETURN n",
            "LET x = 1 FOR d IN c FILTER d.g == x COLLECT WITH COUNT INTO n RETURN n",
            "FOR d IN c OPTIONS {indexHint: 'i'} FILTER d.g == 1 COLLECT WITH COUNT INTO n RETURN n",
            "FOR d IN 1..3 FILTER d == 1 COLLECT WITH COUNT INTO n RETURN n",
            "FOR d IN c FILTER d.g == 1 RETURN d",
        ] {
            assert!(!plans(q), "{q}");
        }
    }
}
