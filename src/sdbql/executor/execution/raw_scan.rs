//! Queries answered by copying stored bytes (see `storage::raw_rows`).
//!
//! `FOR v IN c [LIMIT …] RETURN v | v.f | {k: v.f, …}` over a plain local
//! collection needs no evaluation at all: every row is a document, or fields
//! of it, in the MessagePack the driver sends anyway. This runs such a query
//! without decoding a single value, under the same guards as the ordinary
//! path — the protected-collection check, row policies, search views,
//! columnar and sharded collections, the row and time budget — and hands
//! anything else, or any document it cannot copy, back to that path.

use super::super::types::Context;
use super::super::QueryExecutor;
use crate::error::DbResult;
use crate::sdbql::ast::{BodyClause, Expression, ForClause, Query};
use crate::storage::raw_rows::RawShape;

/// The loop and the row shape of a query this path can answer.
fn raw_plan(query: &Query) -> Option<(&ForClause, RawShape)> {
    if !query.set_operations.is_empty()
        || !query.join_clauses.is_empty()
        || query.with_clause.is_some()
        || query.window_clause.is_some()
        || query.create_stream_clause.is_some()
        || query.create_materialized_view_clause.is_some()
        || query.refresh_materialized_view_clause.is_some()
        || !query.let_clauses.is_empty()
        || !query.post_limit_lets.is_empty()
        || query.sort_clause.is_some()
    {
        return None;
    }
    let [BodyClause::For(for_clause)] = query.body_clauses.as_slice() else {
        return None;
    };
    if for_clause.source_expression.is_some()
        || for_clause.system_time.is_some()
        || for_clause.valid_time.is_some()
        || for_clause.options.is_some()
        || for_clause
            .source_variable
            .as_ref()
            .is_some_and(|s| s != &for_clause.collection)
    {
        return None;
    }
    let ret = query.return_clause.as_ref()?;
    if ret.distinct {
        return None;
    }
    let var = for_clause.variable.as_str();
    let field_of = |expr: &Expression| -> Option<String> {
        match expr {
            Expression::FieldAccess(base, field) | Expression::OptionalFieldAccess(base, field)
                if !field.contains('.')
                    && matches!(base.as_ref(), Expression::Variable(v) if v == var) =>
            {
                Some(field.clone())
            }
            _ => None,
        }
    };
    let shape = match &ret.expression {
        Expression::Variable(v) if v == var => RawShape::Whole,
        Expression::Object(pairs) if !pairs.is_empty() => {
            let mut fields: Vec<(String, String)> = Vec::with_capacity(pairs.len());
            for (key, value) in pairs {
                // A repeated key keeps its last value when evaluated.
                if fields.iter().any(|(k, _)| k == key) {
                    return None;
                }
                fields.push((key.clone(), field_of(value)?));
            }
            fields.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            RawShape::Fields(fields)
        }
        other => RawShape::Single(field_of(other)?),
    };
    Some((for_clause, shape))
}

impl QueryExecutor<'_> {
    /// The query's result as an encoded MessagePack array, or `Ok(None)` when
    /// it has to be run the ordinary way (the caller then does so).
    pub fn execute_raw_msgpack(&self, query: &Query) -> DbResult<Option<Vec<u8>>> {
        let Some((for_clause, shape)) = raw_plan(query) else {
            return Ok(None);
        };
        self.reset_query_caches();
        let Some(ref db_name) = self.database else {
            return Ok(None);
        };
        let name = &for_clause.collection;
        if self.row_policy_applies(name) {
            return Ok(None);
        }
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

        let (offset, count) = match &query.limit_clause {
            Some(limit) => self.eval_limit(limit, &Context::default()),
            None => (0, None),
        };
        let ceiling = self.max_intermediate_rows.saturating_add(1);
        let limit = Some(count.map_or(ceiling, |n| n.min(ceiling)));
        let Some((rows, body)) =
            collection.scan_raw(offset, limit, &shape, |n| self.check_budget(n))?
        else {
            return Ok(None);
        };
        // Over the ceiling: the ordinary path reports it.
        if rows > self.max_intermediate_rows {
            return Ok(None);
        }
        let mut out = Vec::with_capacity(body.len() + 5);
        if rmp::encode::write_array_len(&mut out, rows as u32).is_err() {
            return Ok(None);
        }
        out.extend_from_slice(&body);
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdbql::parser::parse;

    fn plan(q: &str) -> Option<String> {
        let query = parse(q).unwrap();
        raw_plan(&query).map(|(_, shape)| format!("{shape:?}"))
    }

    #[test]
    fn shapes_it_answers() {
        assert_eq!(plan("FOR d IN c RETURN d").as_deref(), Some("Whole"));
        assert_eq!(
            plan("FOR d IN c LIMIT 5 RETURN d").as_deref(),
            Some("Whole")
        );
        assert_eq!(
            plan("FOR d IN c RETURN d.title").as_deref(),
            Some("Single(\"title\")")
        );
        assert_eq!(
            plan("FOR d IN c LIMIT @o, @n RETURN {z: d.a, b: d._key}").as_deref(),
            Some("Fields([(\"b\", \"_key\"), (\"z\", \"a\")])")
        );
    }

    #[test]
    fn everything_else_is_left_to_the_ordinary_path() {
        for q in [
            "FOR d IN c FILTER d.a > 1 RETURN d",
            "FOR d IN c SORT d.a RETURN d",
            "FOR d IN c RETURN DISTINCT d.a",
            "FOR d IN c RETURN {a: d.a, a: d.b}",
            "FOR d IN c RETURN {a: d.a.b}",
            "FOR d IN c RETURN {a: d.a, n: 1}",
            "FOR d IN c RETURN MERGE(d, {x: 1})",
            "FOR d IN c LET x = 1 RETURN d",
            "LET x = 1 FOR d IN c RETURN d",
            "FOR d IN c FOR e IN c RETURN d",
            "FOR d IN 1..3 RETURN d",
            "FOR d IN c COLLECT a = d.a RETURN a",
            "FOR d IN c RETURN e",
        ] {
            assert_eq!(plan(q), None, "{q}");
        }
    }
}
