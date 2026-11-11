//! Queries answered by copying stored bytes (see `storage::raw_rows`).
//!
//! `FOR v IN c [FILTER v.f == x] [LIMIT …] RETURN v | v.f | {k: v.f, …}` over
//! a plain local collection needs no evaluation at all: every row is a
//! document, or fields of it, in the MessagePack the driver sends anyway. The
//! FILTER is taken only when a single-field index serves it, whose entries are
//! then exactly the documents it accepts. This runs such a query without
//! decoding a single value, under the same guards as the ordinary
//! path — the protected-collection check, row policies, search views,
//! columnar and sharded collections, the row and time budget — and hands
//! anything else, or any document it cannot copy, back to that path.
//!
//! `FOR v IN c FILTER … SORT v.f LIMIT n RETURN <the same shapes>`, when an
//! index serves the SORT (`index_sort`), is read the same way: the walk in
//! index order is the ordinary one, each document checked against the
//! FILTERs on a decode of the fields they read, and the rows it keeps copied.

use super::super::types::Context;
use super::super::QueryExecutor;
use super::typed_scan::{Check, TypedFilters};
use crate::error::DbResult;
use crate::sdbql::ast::{BinaryOperator, BodyClause, Expression, ForClause, Query};
use crate::storage::raw_rows::RawShape;

/// The loop, its FILTER and the row shape of a query this path can answer.
fn raw_plan(query: &Query) -> Option<(&ForClause, Option<&Expression>, RawShape)> {
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
    let (for_clause, filter) = match query.body_clauses.as_slice() {
        [BodyClause::For(f)] => (f, None),
        // One equality, nothing else: the index entries are then exactly
        // the rows the FILTER accepts.
        [BodyClause::For(f), BodyClause::Filter(filter)]
            if matches!(
                &filter.expression,
                Expression::BinaryOp {
                    op: BinaryOperator::Equal,
                    ..
                }
            ) =>
        {
            (f, Some(&filter.expression))
        }
        _ => return None,
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
    let shape = return_shape(query, &for_clause.variable)?;
    Some((for_clause, filter, shape))
}

/// The row shape of `FOR v IN c FILTER …+ SORT v.f [DESC] LIMIT … RETURN v |
/// v.f | {k: v.f, …}` — FILTERs and nothing else between the FOR and the
/// SORT — which [`QueryExecutor::sorted_read_target`] may then read in index
/// order. The rest of the shape (one sort field, a LIMIT count, a plain
/// collection loop) is that function's to check.
fn raw_sorted_shape(query: &Query) -> Option<RawShape> {
    if !query.let_clauses.is_empty()
        || !query.post_limit_lets.is_empty()
        || query.sort_clause.is_none()
    {
        return None;
    }
    let (BodyClause::For(for_clause), filters) = query.body_clauses.split_first()? else {
        return None;
    };
    if filters.is_empty() || !filters.iter().all(|c| matches!(c, BodyClause::Filter(_))) {
        return None;
    }
    return_shape(query, &for_clause.variable)
}

/// What each row of `query`'s RETURN is, if it is `var`, a field of it, or
/// an object of its fields.
fn return_shape(query: &Query, var: &str) -> Option<RawShape> {
    let ret = query.return_clause.as_ref()?;
    if ret.distinct {
        return None;
    }
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
    Some(shape)
}

/// What [`QueryExecutor::execute_raw_msgpack_small`] made of a query.
#[derive(Debug)]
pub enum SmallRaw {
    /// The rows, an encoded MessagePack array.
    Rows(Vec<u8>),
    /// A raw query, but a scan, or an index read of more rows than asked:
    /// nothing was fetched. Run it the usual way.
    NotSmall,
    /// Not for this path (see [`QueryExecutor::execute_raw_msgpack`]).
    Ordinary,
}

impl QueryExecutor<'_> {
    /// The query's result as an encoded MessagePack array, or `Ok(None)` when
    /// it has to be run the ordinary way (the caller then does so).
    pub fn execute_raw_msgpack(&self, query: &Query) -> DbResult<Option<Vec<u8>>> {
        Ok(match self.raw_msgpack(query, None)? {
            SmallRaw::Rows(rows) => Some(rows),
            SmallRaw::NotSmall | SmallRaw::Ordinary => None,
        })
    }

    /// [`Self::execute_raw_msgpack`], but only for a FILTER read through an
    /// index whose entries name at most `max_rows` documents, or a SORT read
    /// in index order whose LIMIT keeps at most that many: a handful of
    /// point reads, cheap enough to answer where a single-document get is
    /// answered. The index is read first, and nothing is fetched when it
    /// names more (`NotSmall`).
    pub fn execute_raw_msgpack_small(&self, query: &Query, max_rows: usize) -> DbResult<SmallRaw> {
        self.raw_msgpack(query, Some(max_rows))
    }

    fn raw_msgpack(&self, query: &Query, small: Option<usize>) -> DbResult<SmallRaw> {
        let Some((for_clause, filter, shape)) = raw_plan(query) else {
            return match raw_sorted_shape(query) {
                Some(shape) => self.raw_sorted(query, &shape, small),
                None => Ok(SmallRaw::Ordinary),
            };
        };
        if small.is_some() && filter.is_none() {
            return Ok(SmallRaw::NotSmall);
        }
        self.reset_query_caches();
        let Some(ref db_name) = self.database else {
            return Ok(SmallRaw::Ordinary);
        };
        let name = &for_clause.collection;
        if self.row_policy_applies(name) {
            return Ok(SmallRaw::Ordinary);
        }
        let Ok(database) = self.storage.get_database(db_name) else {
            return Ok(SmallRaw::Ordinary);
        };
        if self.is_columnar(&database, name)
            || !matches!(self.resolve_search_view_collection(name), Ok(None))
        {
            return Ok(SmallRaw::Ordinary);
        }
        // Also the protected-collection guard: an error here is reported by
        // the ordinary path.
        let Ok(collection) = self.get_collection(name) else {
            return Ok(SmallRaw::Ordinary);
        };
        if collection
            .get_shard_config()
            .is_some_and(|c| c.num_shards > 0)
        {
            return Ok(SmallRaw::Ordinary);
        }

        let (offset, count) = match &query.limit_clause {
            Some(limit) => self.eval_limit(limit, &Context::default()),
            None => (0, None),
        };
        let ceiling = self.max_intermediate_rows.saturating_add(1);
        let limit = Some(count.map_or(ceiling, |n| n.min(ceiling)));
        let scanned = match filter {
            None => collection.scan_raw(offset, limit, &shape, |n| self.check_budget(n))?,
            Some(filter) => {
                let Some(limit) = limit else {
                    return Ok(SmallRaw::Ordinary);
                };
                let keys = match self.index_eq_keys(
                    &collection,
                    for_clause,
                    filter,
                    offset,
                    limit,
                    small,
                )? {
                    IndexKeys::Keys(keys) => keys,
                    IndexKeys::NotSmall => return Ok(SmallRaw::NotSmall),
                    IndexKeys::NoIndex => return Ok(SmallRaw::Ordinary),
                };
                collection
                    .raw_rows_of(&keys, &shape)?
                    .map(|body| (keys.len(), body))
            }
        };
        let Some((rows, body)) = scanned else {
            return Ok(SmallRaw::Ordinary);
        };
        // Over the ceiling: the ordinary path reports it.
        if rows > self.max_intermediate_rows {
            return Ok(SmallRaw::Ordinary);
        }
        let mut out = Vec::with_capacity(body.len() + 5);
        if rmp::encode::write_array_len(&mut out, rows as u32).is_err() {
            return Ok(SmallRaw::Ordinary);
        }
        out.extend_from_slice(&body);
        Ok(SmallRaw::Rows(out))
    }

    /// `FOR v IN c FILTER … SORT v.f LIMIT … RETURN <shape>` read in SORT
    /// order from an index (see `index_sort`), each row copied from the
    /// stored bytes. Each document read is checked against every FILTER as
    /// the ordinary walk checks it, on a decode of just the fields the
    /// FILTERs and the SORT read. With `small`, the LIMIT must keep at most
    /// that many rows, and the walk gives up (`NotSmall`) past four times as
    /// many documents read.
    fn raw_sorted(
        &self,
        query: &Query,
        shape: &RawShape,
        small: Option<usize>,
    ) -> DbResult<SmallRaw> {
        self.reset_query_caches();
        let bindings = Context::default();
        let Some(target) = self.sorted_read_target(query, &bindings) else {
            return Ok(SmallRaw::Ordinary);
        };
        let plan = &target.plan;
        let var = plan.for_clause.variable.as_str();
        // A `@x` loop variable is what a missing bind var `@x` reads (see
        // `projection`).
        if !plan.lets.is_empty() || var.starts_with('@') {
            return Ok(SmallRaw::Ordinary);
        }
        let want = target.offset + target.count;
        let (max_scan, gave_up) = match small {
            Some(cap) if want > cap => return Ok(SmallRaw::NotSmall),
            Some(cap) => (cap.saturating_mul(4), SmallRaw::NotSmall),
            None => (self.max_intermediate_rows(), SmallRaw::Ordinary),
        };
        let mut fields = vec![plan.sort_field.to_string()];
        for filter in &plan.filters {
            if super::super::projection::collect(filter, var, &mut fields).is_none() {
                return Ok(SmallRaw::Ordinary);
            }
        }
        fields.sort_unstable();
        fields.dedup();
        // The FILTERs checked on the stored bytes when they compile (field
        // compared with a constant or bind variable, AND / OR / NOT …), with
        // no row built; any document that cannot be read that way, and any
        // query whose FILTERs do not compile, the ordinary way below.
        let typed = TypedFilters::compile(self, var, &plan.filters, plan.sort_field);

        let mut out = Vec::new();
        let mut kept = 0usize;
        let (lower, upper) = target.read.bounds();
        let walked = target.collection.index_sorted_range_by(
            &target.read.index,
            &target.read.eq_values,
            lower,
            upper,
            plan.ascending,
            want,
            max_scan,
            &mut |bytes| {
                let passed = match typed.as_ref().map(|t| t.check(bytes)) {
                    Some(Check::Pass) => true,
                    Some(Check::Fail) => return Some(None),
                    Some(Check::Stop) => return None,
                    Some(Check::Fallback) | None => false,
                };
                if passed {
                    if kept >= target.offset {
                        crate::storage::raw_rows::write_row(bytes, shape, &mut out)?;
                    }
                    kept += 1;
                    return Some(Some(()));
                }
                let row = crate::storage::raw_rows::project(bytes, &fields)?;
                // The ordinary walk gives up on these too.
                if matches!(
                    row.get(plan.sort_field),
                    Some(serde_json::Value::Array(_) | serde_json::Value::Object(_))
                ) {
                    return None;
                }
                let mut ctx = Context::default();
                ctx.insert(var.to_string(), row);
                for filter in &plan.filters {
                    if !self.filter_row(filter, &ctx) {
                        return Some(None);
                    }
                }
                if kept >= target.offset {
                    crate::storage::raw_rows::write_row(bytes, shape, &mut out)?;
                }
                kept += 1;
                Some(Some(()))
            },
        );
        let Some((rows, _)) = walked else {
            return Ok(gave_up);
        };
        self.check_budget(rows.len())?;
        let rows = rows.len().saturating_sub(target.offset);
        let mut body = Vec::with_capacity(out.len() + 5);
        if rmp::encode::write_array_len(&mut body, rows as u32).is_err() {
            return Ok(SmallRaw::Ordinary);
        }
        body.extend_from_slice(&out);
        Ok(SmallRaw::Rows(body))
    }

    /// The storage keys of the rows of `FOR v IN c FILTER v.f == x`, read
    /// through the index on `f`. With `small`, `NotSmall` as soon as the
    /// index names more than that many rows.
    fn index_eq_keys(
        &self,
        collection: &crate::storage::Collection,
        for_clause: &ForClause,
        filter: &Expression,
        offset: usize,
        limit: usize,
        small: Option<usize>,
    ) -> DbResult<IndexKeys> {
        let Some(cond) =
            self.extract_indexable_condition(filter, &for_clause.variable, &Context::default())
        else {
            return Ok(IndexKeys::NoIndex);
        };
        // `_key` and `_id` are served by the primary key, not an index.
        if cond.op != BinaryOperator::Equal || cond.field == "_key" || cond.field == "_id" {
            return Ok(IndexKeys::NoIndex);
        }
        // One past the cap tells a small read from a large one.
        let walk = small.map_or(limit, |cap| limit.min(cap.saturating_add(1)));
        // The ordinary path looks a whole number up as an integer first, and
        // as written only when the index holds nothing under the integer
        // (see `use_index_for_condition`).
        let normalized = super::index_count::whole_number_as_integer(&cond.value);
        let check = |n| self.check_budget(n);
        let first = collection.index_eq_doc_keys(&cond.field, &normalized, offset, walk, check)?;
        let keys = match first {
            Some((_, false)) if normalized != cond.value => collection
                .index_eq_doc_keys(&cond.field, &cond.value, offset, walk, check)?
                .map(|(keys, _)| keys),
            other => other.map(|(keys, _)| keys),
        };
        Ok(match keys {
            None => IndexKeys::NoIndex,
            Some(keys) if small.is_some_and(|cap| keys.len() > cap) => IndexKeys::NotSmall,
            Some(keys) => IndexKeys::Keys(keys),
        })
    }
}

/// What [`QueryExecutor::index_eq_keys`] read.
enum IndexKeys {
    Keys(Vec<Vec<u8>>),
    NotSmall,
    NoIndex,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdbql::parser::parse;

    fn plan(q: &str) -> Option<String> {
        let query = parse(q).unwrap();
        raw_plan(&query).map(|(_, _, shape)| format!("{shape:?}"))
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
        assert_eq!(
            plan("FOR d IN c FILTER d.g == @g RETURN {n: d.name}").as_deref(),
            Some("Fields([(\"n\", \"name\")])")
        );
    }

    #[test]
    fn sorted_shapes_it_answers() {
        let shape = |q: &str| raw_sorted_shape(&parse(q).unwrap()).map(|s| format!("{s:?}"));
        assert_eq!(
            shape("FOR d IN c FILTER d.t >= @lo SORT d.t DESC LIMIT 20 RETURN d").as_deref(),
            Some("Whole")
        );
        assert_eq!(
            shape("FOR d IN c FILTER d.a == 1 FILTER d.t < 9 SORT d.t LIMIT 5 RETURN d.x")
                .as_deref(),
            Some("Single(\"x\")")
        );
        for q in [
            "FOR d IN c SORT d.t LIMIT 5 RETURN d",
            "FOR d IN c FILTER d.t > 1 RETURN d",
            "FOR d IN c FILTER d.t > 1 LET x = 1 SORT d.t LIMIT 5 RETURN d",
            "FOR d IN c FILTER d.t > 1 SORT d.t LIMIT 5 LET x = 1 RETURN d",
            "LET x = 1 FOR d IN c FILTER d.t > 1 SORT d.t LIMIT 5 RETURN d",
            "FOR d IN c FILTER d.t > 1 SORT d.t LIMIT 5 RETURN MERGE(d, {})",
        ] {
            assert_eq!(shape(q), None, "{q}");
        }
    }

    #[test]
    fn everything_else_is_left_to_the_ordinary_path() {
        for q in [
            "FOR d IN c FILTER d.a > 1 RETURN d",
            "FOR d IN c FILTER d.a == 1 AND d.b == 2 RETURN d",
            "FOR d IN c FILTER d.a == 1 FILTER d.b == 2 RETURN d",
            "FOR d IN c FILTER d.a == 1 SORT d.b RETURN d",
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
