//! A COLLECT over a time series collection, folded from its column blocks.
//!
//! `FOR p IN c FILTER … COLLECT … AGGREGATE …` over a collection that keeps
//! column blocks (`storage::blocks`) reads the rows of the blocks instead of
//! the documents, when the query stays within what the blocks hold:
//!
//! - every FILTER is a conjunction of comparisons of the time field with a
//!   number, and of equalities of another field with a string;
//! - every group key is a field, or `TIME_BUCKET(time, "<fixed interval>")`;
//! - every aggregate has no argument, or a field;
//! - all those fields are the definition's.
//!
//! The rows go into the same fold as documents would
//! ([`CollectFold::push_datums`]), with the values the documents hold (an
//! integer stays one), so the aggregates are the ones the ordinary path
//! computes. The documents no block covers — the newest, and any range
//! whose block was thrown away or has no columns — are read and folded the
//! ordinary way, FILTERs included. One difference can show: when the
//! smallest (or largest) value of a `MIN` / `MAX` is held as `5` by one
//! point and `5.0` by another, which of the two is returned follows the
//! order the rows are read in, and that order is not the ordinary path's.
//!
//! Anything else, and a collection whose blocks cover too little, is left
//! to the ordinary path.

use std::sync::Arc;

use serde_json::Value;

use super::super::aggregation::{AggKind, AggregateAccumulator, Slice};
use super::super::datum::Datum;
use super::super::index_opt::{expression_references_var, push_conjuncts, IndexHint};
use super::super::types::Context;
use super::super::QueryExecutor;
use super::collect::CollectFold;
use super::scan_collect::{bind_in_place, plan, Plan, ScanCollectRun};
use crate::error::DbResult;
use crate::sdbql::ast::{BinaryOperator, BodyClause, Expression};
use crate::sdbql::executor::builtins::timeseries::{parse_interval, Interval};
use crate::storage::blocks::{self, BlockSet, Column, Columns, Segment, Total, CHUNK_ROWS};
use crate::storage::serializer::{deserialize_doc_as_value, deserialize_doc_projected};
use crate::storage::Collection;

/// Documents outside the blocks a query accepts to read one by one, whatever
/// the blocks hold: past it (and a quarter of what they hold) the ordinary
/// path, which has the indexes, is the better one.
const UNSEALED_READ: u64 = 16_384;

/// A column of the rows: the time, or a field of the definition by position.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    Time,
    Field(usize),
}

/// A group key.
enum Key<'q> {
    Column(Source),
    /// `TIME_BUCKET(time, "<interval>")`: the interval in milliseconds, and
    /// the expression, evaluated once per bucket.
    Bucket(i64, &'q Expression),
}

/// What the query asks of the rows.
struct BlockPlan<'q> {
    /// Times in `[from, to)`.
    from: i64,
    to: i64,
    timed: bool,
    /// For each field of the definition, the string it must equal.
    equals: Vec<Option<String>>,
    keys: Vec<Key<'q>>,
    /// For each aggregate, its argument.
    args: Vec<Option<Source>>,
    kinds: Vec<AggKind>,
    /// Every aggregate takes whole columns: rows that go to one group
    /// are folded together.
    slices: bool,
}

/// A whole number of milliseconds from a comparison's constant.
fn bound(value: &Value, up: bool) -> Option<i64> {
    let n = value.as_number()?;
    if let Some(i) = n.as_i64() {
        return Some(i);
    }
    let f = n.as_f64().filter(|f| f.is_finite() && f.abs() < 9.0e18)?;
    Some(if up { f.ceil() } else { f.floor() } as i64)
}

impl QueryExecutor<'_> {
    /// The field of the loop variable `expr` reads, as a column.
    fn block_source(expr: &Expression, var: &str, set: &BlockSet) -> Option<Source> {
        let Expression::FieldAccess(base, field) = expr else {
            return None;
        };
        if !matches!(base.as_ref(), Expression::Variable(v) if v == var) {
            return None;
        }
        if field == &set.def.time {
            return Some(Source::Time);
        }
        set.def
            .fields
            .iter()
            .position(|f| f == field)
            .map(Source::Field)
    }

    /// The plan for `found` over `set`, or `None` when the query asks for
    /// more than the rows hold.
    fn block_plan<'q>(
        &self,
        found: &Plan<'q>,
        set: &BlockSet,
        outer: &Context,
    ) -> Option<BlockPlan<'q>> {
        let var = found.for_clause.variable.as_str();
        let mut plan = BlockPlan {
            from: i64::MIN,
            to: i64::MAX,
            timed: false,
            equals: vec![None; set.def.fields.len()],
            keys: Vec::new(),
            args: Vec::new(),
            kinds: Vec::new(),
            slices: false,
        };
        let mut conjuncts = Vec::new();
        for stage in found.stages {
            let BodyClause::Filter(filter) = stage else {
                return None;
            };
            push_conjuncts(&filter.expression, &mut conjuncts);
        }
        for conjunct in conjuncts {
            let Expression::BinaryOp { left, op, right } = conjunct else {
                return None;
            };
            let source = Self::block_source(left, var, set)?;
            if expression_references_var(right, var) {
                return None;
            }
            let value = self.evaluate_expr_with_context(right, outer).ok()?;
            match (source, op) {
                (Source::Time, BinaryOperator::GreaterThanOrEqual) => {
                    plan.from = plan.from.max(bound(&value, true)?);
                }
                (Source::Time, BinaryOperator::GreaterThan) => {
                    plan.from = plan.from.max(bound(&value, false)?.checked_add(1)?);
                }
                (Source::Time, BinaryOperator::LessThan) => {
                    plan.to = plan.to.min(bound(&value, true)?);
                }
                (Source::Time, BinaryOperator::LessThanOrEqual) => {
                    plan.to = plan.to.min(bound(&value, false)?.checked_add(1)?);
                }
                (Source::Field(at), BinaryOperator::Equal) => {
                    let wanted = value.as_str()?.to_string();
                    match &plan.equals[at] {
                        Some(held) if held != &wanted => return None,
                        _ => plan.equals[at] = Some(wanted),
                    }
                }
                _ => return None,
            }
            plan.timed |= source == Source::Time;
        }
        for (_, expr) in &found.collect.group_vars {
            plan.keys.push(match expr {
                Expression::FunctionCall { name, args }
                    if name.eq_ignore_ascii_case("TIME_BUCKET") && args.len() == 2 =>
                {
                    if Self::block_source(&args[0], var, set)? != Source::Time {
                        return None;
                    }
                    let Expression::Literal(Value::String(interval)) = &args[1] else {
                        return None;
                    };
                    // Weeks are counted from a Monday, months have no fixed
                    // length: the builtin's own business.
                    let unit = interval.trim_end().to_ascii_lowercase();
                    if unit.ends_with('w') || unit.ends_with("week") || unit.ends_with("weeks") {
                        return None;
                    }
                    match parse_interval(interval).ok()? {
                        Interval::Fixed(step) if step > 0 => Key::Bucket(step, expr),
                        _ => return None,
                    }
                }
                other => Key::Column(Self::block_source(other, var, set)?),
            });
        }
        for aggregate in &found.collect.aggregates {
            plan.args.push(match &aggregate.argument {
                None => None,
                Some(expr) => Some(Self::block_source(expr, var, set)?),
            });
            // An unknown aggregate is the ordinary path's to report.
            plan.kinds.push(AggKind::resolve(
                &aggregate.function,
                aggregate.argument.is_some(),
            )?);
        }
        plan.slices = plan
            .kinds
            .iter()
            .all(|kind| AggregateAccumulator::takes_slices(*kind));
        Some(plan)
    }

    /// What [`Self::try_block_collect`] would fold for `clauses[i]`.
    fn plan_block_collect<'q>(
        &self,
        clauses: &'q [BodyClause],
        i: usize,
        rows: &[Context],
    ) -> Option<(Plan<'q>, Collection, Arc<BlockSet>, BlockPlan<'q>)> {
        if !self.stream_collect {
            return None;
        }
        let [outer] = rows else {
            return None;
        };
        let found = plan(clauses, i)?;
        let for_clause = found.for_clause;
        if found.collect.into_var.is_some()
            || IndexHint::of(for_clause).is_some()
            || self.row_policy_gate(&for_clause.collection).is_some()
        {
            return None;
        }
        let collection = self.scan_collect_source(for_clause, rows)?;
        let set = collection.column_blocks()?;
        let block_plan = self.block_plan(&found, &set, outer)?;
        // Worth it only when the blocks hold most of the collection: what
        // they do not hold is read as documents, every one of them.
        let unsealed = set.unsealed(&collection);
        let sealed = (collection.count() as u64).saturating_sub(unsealed);
        if sealed == 0 || unsealed > UNSEALED_READ.max(sealed / 4) {
            // After a restart the small blocks are gone and nothing was
            // written yet: have the sealing thread catch up for the next
            // query. This one takes the ordinary path.
            collection.seal_column_blocks_behind(&set, false);
            return None;
        }
        Some((found, collection, set, block_plan))
    }

    /// What EXPLAIN says when [`Self::try_block_collect`] would run
    /// `clauses[i]`.
    pub(in crate::sdbql::executor) fn block_collect_note(
        &self,
        clauses: &[BodyClause],
        i: usize,
        rows: &[Context],
    ) -> Option<String> {
        let (found, ..) = self.plan_block_collect(clauses, i, rows)?;
        Some(format!(
            "optimizer: COLLECT folded from the column blocks of '{}': the rows of its \
             blocks are grouped, and only the documents no block covers are read",
            found.for_clause.collection
        ))
    }

    /// The newest row of one series, for an index-ordered read that asks
    /// for it (`FILTER p.series == x SORT p.time DESC LIMIT 1`), from the
    /// column blocks: the row an index walk would keep, bound as that walk
    /// binds it, or no row. `None` when the walk has to run.
    ///
    /// The blocks answer only when they hold every document (nothing waits
    /// outside them, each has its columns in memory and was checked) and
    /// when the answer is one row beyond doubt: the series is the blocks'
    /// one string column, so that its rows are a run ordered by time, and
    /// no other row of the series has the newest instant — which of two
    /// such documents comes first is decided by their keys, which a block
    /// does not hold. Anything else, a FILTER the row does not pass
    /// included, is the index's to answer.
    pub(super) fn newest_from_blocks(
        &self,
        target: &super::index_sort::SortedTarget<'_>,
        bindings: &Context,
        fields: &[String],
    ) -> Option<Vec<Context>> {
        let plan = &target.plan;
        if plan.ascending || target.offset != 0 || target.count != 1 {
            return None;
        }
        let set = target.collection.column_blocks()?;
        let def = &set.def;
        if plan.sort_field != def.time {
            return None;
        }
        // The read planned: every row of one string value, by time.
        let [series, time] = target.read.index.fields.as_slice() else {
            return None;
        };
        let [Value::String(wanted)] = target.read.eq_values.as_slice() else {
            return None;
        };
        if time != &def.time || !matches!(target.read.bounds(), (None, None)) {
            return None;
        }
        let series_at = def.fields.iter().position(|f| f == series)?;
        if !fields
            .iter()
            .all(|f| f == &def.time || def.fields.contains(f))
        {
            return None;
        }
        // Asked before the blocks are listed, as the fold does.
        if !set.tail_is_empty() {
            return None;
        }
        let segments = set.segments(&target.collection);
        // The newest row of the series, and whether another has its time.
        let mut best: Option<(i64, Arc<Columns>, usize)> = None;
        let mut tied = false;
        for segment in segments.iter().rev() {
            let entry = match segment {
                Segment::Block(entry) => entry,
                Segment::Documents(_, None) => continue,
                Segment::Documents(..) => return None,
            };
            if entry.opaque || !set.verified(&target.collection, entry) {
                return None;
            }
            if best.as_ref().is_some_and(|(t, ..)| entry.max_time < *t) {
                continue;
            }
            let columns = set.cached_columns(entry)?;
            let strings = columns
                .cols
                .iter()
                .filter(|c| matches!(c, Column::Str { .. }))
                .count();
            let Column::Str { dict, codes } = columns.cols.get(series_at)? else {
                return None;
            };
            if strings != 1 {
                return None;
            }
            let Ok(code) = dict.binary_search_by(|d| d.as_str().cmp(wanted.as_str())) else {
                continue;
            };
            // Rows are ordered by the string then by time: the series is a
            // run, its newest row the last.
            let code = code as u32;
            let from = codes.partition_point(|c| *c < code);
            let to = codes.partition_point(|c| *c <= code);
            if from == to {
                continue;
            }
            let at = to - 1;
            let t = *columns.time.get(at)?;
            let twice = at > from && columns.time[at - 1] == t;
            match &best {
                Some((held, ..)) if *held > t => {}
                Some((held, ..)) if *held == t => tied = true,
                _ => {
                    tied = twice;
                    best = Some((t, columns.clone(), at));
                }
            }
        }
        if tied {
            return None;
        }
        // It did not come from the stored documents: a transaction that
        // checks its reads holds it to the range rule.
        target.collection.note_range_read();
        let answered = || {
            blocks::NEWEST_FROM_BLOCKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        };
        let Some((t, columns, at)) = best else {
            answered();
            return Some(Vec::new());
        };
        let mut row = serde_json::Map::new();
        for field in fields {
            let value = if field == &def.time {
                Value::from(t)
            } else {
                let column = columns
                    .cols
                    .get(def.fields.iter().position(|f| f == field)?)?;
                match column {
                    Column::Int(values) => Value::from(*values.get(at)?),
                    Column::Float(values) => Value::from(*values.get(at)?),
                    Column::Mixed { values, ints } => {
                        let v = *values.get(at)?;
                        if *ints.get(at)? {
                            Value::from(v as i64)
                        } else {
                            Value::from(v)
                        }
                    }
                    Column::Str { dict, codes } => {
                        Value::String(dict.get(*codes.get(at)? as usize)?.clone())
                    }
                }
            };
            row.insert(field.clone(), value);
        }
        let mut ctx = bindings.clone();
        ctx.insert(plan.for_clause.variable.clone(), Value::Object(row));
        // A FILTER beyond the equality, which this row may not pass while
        // an older one does.
        if !plan
            .filters
            .iter()
            .all(|filter| self.filter_row(filter, &ctx))
        {
            return None;
        }
        answered();
        Some(vec![ctx])
    }

    /// Run `clauses[i]`, a FOR, and the clauses up to its COLLECT from the
    /// column blocks of its collection; `Ok(None)` when the ordinary path
    /// has to run them (see the module's notes).
    pub(in crate::sdbql::executor) fn try_block_collect(
        &self,
        clauses: &[BodyClause],
        i: usize,
        rows: &[Context],
    ) -> DbResult<Option<ScanCollectRun>> {
        let Some((found, collection, set, block_plan)) = self.plan_block_collect(clauses, i, rows)
        else {
            return Ok(None);
        };
        let outer = &rows[0];
        let var = found.for_clause.variable.as_str();
        // A transaction that checks its reads holds this one to the range
        // rule: it did not come from the stored documents.
        collection.note_range_read();

        let mut fold = CollectFold::new(found.collect);
        let mut passes = vec![(0usize, 0usize); found.stages.len()];
        let mut scanned = 0usize;
        let mut scratch = outer.clone();
        let mut documents = |lo: &str, hi: Option<&str>, fold: &mut CollectFold<'_>| {
            let mut failed = None;
            blocks::for_each_doc(&collection, lo, hi, None, |_, bytes| {
                let decoded = match found.fields.as_deref() {
                    Some(fields) => deserialize_doc_projected(bytes, fields),
                    None => deserialize_doc_as_value(bytes),
                };
                let Ok(doc) = decoded else {
                    return true;
                };
                scanned += 1;
                bind_in_place(&mut scratch, var, doc);
                let folded = self
                    .run_stages(found.stages, &mut scratch, &mut passes)
                    .and_then(|kept| match kept {
                        true => fold.push_ref(self, &scratch),
                        false => Ok(()),
                    });
                match folded {
                    Ok(()) => true,
                    Err(e) => {
                        failed = Some(e);
                        false
                    }
                }
            })?;
            failed.map_or(Ok(()), Err)
        };

        let mut rows_read = 0usize;
        // Asked before the blocks are listed: a write after it is one the
        // query may or may not see, as with any read.
        let tail_empty = set.tail_is_empty();
        let mut state = Buckets::new(&block_plan, &set.def.time, var, outer);
        for segment in set.segments(&collection).iter() {
            match segment {
                // Past the last block, when a seal found nothing there and
                // nothing was written since: nothing to open.
                Segment::Documents(_, None) if tail_empty => {}
                Segment::Documents(lo, hi) => documents(lo, hi.as_deref(), &mut fold)?,
                Segment::Block(entry) => {
                    let usable = !entry.opaque && set.verified(&collection, entry);
                    // No row of it in the span of time asked for.
                    if usable
                        && block_plan.timed
                        && (block_plan.to <= entry.min_time || block_plan.from > entry.max_time)
                    {
                        continue;
                    }
                    let columns = usable.then(|| set.columns(&collection, entry)).flatten();
                    match columns {
                        Some(columns) => {
                            rows_read +=
                                self.fold_block(&block_plan, &columns, &mut state, &mut fold)?;
                        }
                        None => documents(&entry.lo, Some(&entry.hi), &mut fold)?,
                    }
                }
            }
        }
        let groups = fold.finish()?;
        self.check_budget(groups.len())?;
        Ok(Some(ScanCollectRun {
            rows: groups,
            consumed: found.stages.len() + 2,
            scanned: scanned + rows_read,
            filters: passes,
            note: String::new(),
        }))
    }

    /// Fold the rows `wanted` of `columns` into the one group of a COLLECT
    /// without keys, their values gathered into columns of their own.
    fn fold_rows(
        &self,
        plan: &BlockPlan<'_>,
        columns: &Columns,
        wanted: &[u32],
        fold: &mut CollectFold<'_>,
    ) -> DbResult<()> {
        enum Held {
            Int(Vec<i64>),
            Float(Vec<f64>),
            Mixed(Vec<f64>, Vec<bool>),
            Other,
        }
        let rows = || wanted.iter().map(|r| *r as usize);
        let held: Vec<Option<Held>> = plan
            .args
            .iter()
            .map(|arg| {
                arg.map(|source| match source {
                    Source::Time => Held::Int(rows().map(|r| columns.time[r]).collect()),
                    Source::Field(at) => match &columns.cols[at] {
                        Column::Int(values) => Held::Int(rows().map(|r| values[r]).collect()),
                        Column::Float(values) => Held::Float(rows().map(|r| values[r]).collect()),
                        Column::Mixed { values, ints } => Held::Mixed(
                            rows().map(|r| values[r]).collect(),
                            rows().map(|r| ints[r]).collect(),
                        ),
                        Column::Str { .. } => Held::Other,
                    },
                })
            })
            .collect();
        let args: Vec<Option<Slice<'_>>> = held
            .iter()
            .map(|h| {
                h.as_ref().map(|h| match h {
                    Held::Int(values) => Slice::Int(values),
                    Held::Float(values) => Slice::Float(values),
                    Held::Mixed(values, ints) => Slice::Mixed(values, ints),
                    Held::Other => Slice::Other,
                })
            })
            .collect();
        fold.push_slices(self, &[], wanted.len(), &args)
    }

    /// Fold the rows of `columns` the plan selects; how many.
    fn fold_block(
        &self,
        plan: &BlockPlan<'_>,
        columns: &Columns,
        state: &mut Buckets<'_>,
        fold: &mut CollectFold<'_>,
    ) -> DbResult<usize> {
        let total = columns.time.len();
        if total == 0
            || (plan.timed && (plan.to <= columns.min_time || plan.from > columns.max_time))
        {
            return Ok(0);
        }
        // The rows are ordered by the string columns, then by time: while
        // every string column so far is fixed, the rows wanted are a run.
        let (mut start, mut end) = (0usize, total);
        let mut run = true;
        let mut residual: Vec<(&[u32], u32)> = Vec::new();
        for (at, column) in columns.cols.iter().enumerate() {
            let wanted = plan.equals[at].as_deref();
            let Column::Str { dict, codes } = column else {
                // A string never equals a number.
                if wanted.is_some() {
                    return Ok(0);
                }
                continue;
            };
            let Some(wanted) = wanted else {
                run = false;
                continue;
            };
            let Ok(code) = dict.binary_search_by(|d| d.as_str().cmp(wanted)) else {
                return Ok(0);
            };
            let code = code as u32;
            if run {
                let held = &codes[start..end];
                let first = held.partition_point(|c| *c < code);
                let past = held.partition_point(|c| *c <= code);
                (start, end) = (start + first, start + past);
            } else {
                residual.push((codes.as_slice(), code));
            }
        }
        let mut check_time =
            plan.timed && !(plan.from <= columns.min_time && columns.max_time < plan.to);
        if check_time && run {
            let held = &columns.time[start..end];
            let first = held.partition_point(|t| *t < plan.from);
            let past = held.partition_point(|t| *t < plan.to);
            (start, end) = (start + first, start + past);
            check_time = false;
        }

        // Aggregates that take whole columns: the rows wanted are folded
        // together wherever they go to one group (strings can be counted,
        // nothing more).
        let counted = |kind: &AggKind| matches!(kind, AggKind::CountRows | AggKind::CountNonNull);
        let in_slices = plan.slices
            && plan.args.iter().zip(&plan.kinds).all(|(arg, kind)| {
                !matches!(arg, Some(Source::Field(at))
                    if matches!(columns.cols[*at], Column::Str { .. }) && !counted(kind))
            });
        let contiguous = !check_time && residual.is_empty();
        let slices = |from: usize, to: usize| -> Vec<Option<Slice<'_>>> {
            plan.args
                .iter()
                .map(|arg| {
                    arg.map(|source| match source {
                        Source::Time => Slice::Int(&columns.time[from..to]),
                        Source::Field(at) => match &columns.cols[at] {
                            Column::Int(values) => Slice::Int(&values[from..to]),
                            Column::Float(values) => Slice::Float(&values[from..to]),
                            Column::Mixed { values, ints } => {
                                Slice::Mixed(&values[from..to], &ints[from..to])
                            }
                            Column::Str { .. } => Slice::Other,
                        },
                    })
                })
                .collect()
        };
        if in_slices && plan.keys.is_empty() && contiguous {
            // The whole block: its totals, computed when it was sealed.
            let args = if start == 0 && end == total {
                plan.args
                    .iter()
                    .map(|arg| {
                        arg.map(|source| match source {
                            Source::Time => Slice::Int(&columns.time),
                            Source::Field(at) => match (&columns.cols[at], &columns.totals[at]) {
                                (_, Some(total)) => total_slice(total),
                                (Column::Int(values), None) => Slice::Int(values),
                                (Column::Float(values), None) => Slice::Float(values),
                                (Column::Mixed { values, ints }, None) => {
                                    Slice::Mixed(values, ints)
                                }
                                (Column::Str { .. }, None) => Slice::Other,
                            },
                        })
                    })
                    .collect()
            } else {
                slices(start, end)
            };
            fold.push_slices(self, &[], end - start, &args)?;
            return Ok(end - start);
        }
        // Every series, a span of time: the rows in time order, whole
        // chunks of them by their subtotals.
        if in_slices && plan.keys.is_empty() && residual.is_empty() && start == 0 && end == total {
            let order = &columns.by_time;
            let first = order.partition_point(|r| columns.time[*r as usize] < plan.from);
            let past = order.partition_point(|r| columns.time[*r as usize] < plan.to);
            let mut at = first;
            while at < past {
                let chunk = at / CHUNK_ROWS;
                let next = ((chunk + 1) * CHUNK_ROWS).min(past);
                let whole =
                    at == chunk * CHUNK_ROWS && (next == (chunk + 1) * CHUNK_ROWS || next == total);
                let totals: Option<Vec<Option<Slice<'_>>>> = whole
                    .then(|| {
                        plan.args
                            .iter()
                            .map(|arg| match arg {
                                None => Some(None),
                                Some(Source::Time) => None,
                                Some(Source::Field(f)) => {
                                    match (&columns.cols[*f], &columns.chunks[*f][chunk]) {
                                        (_, Some(total)) => Some(Some(total_slice(total))),
                                        (Column::Str { .. }, None) => Some(Some(Slice::Other)),
                                        (_, None) => None,
                                    }
                                }
                            })
                            .collect()
                    })
                    .flatten();
                match totals {
                    Some(args) => fold.push_slices(self, &[], next - at, &args)?,
                    None => self.fold_rows(plan, columns, &order[at..next], fold)?,
                }
                at = next;
            }
            return Ok(past - first);
        }
        if in_slices && plan.keys.is_empty() {
            let wanted: Vec<u32> = (start..end)
                .filter(|&row| {
                    let t = columns.time[row];
                    (!check_time || (t >= plan.from && t < plan.to))
                        && residual.iter().all(|(codes, code)| codes[row] == *code)
                })
                .map(|row| row as u32)
                .collect();
            self.fold_rows(plan, columns, &wanted, fold)?;
            return Ok(wanted.len());
        }

        let number =
            |source: Source, row: usize| -> Datum<'_> {
                match source {
                    Source::Time => Datum::Num(columns.time[row].into()),
                    Source::Field(at) => match &columns.cols[at] {
                        Column::Int(values) => Datum::Num(values[row].into()),
                        Column::Float(values) => serde_json::Number::from_f64(values[row])
                            .map_or(Datum::Null, Datum::Num),
                        Column::Mixed { values, ints } if ints[row] => {
                            Datum::Num((values[row] as i64).into())
                        }
                        Column::Mixed { values, .. } => serde_json::Number::from_f64(values[row])
                            .map_or(Datum::Null, Datum::Num),
                        Column::Str { dict, codes } => Datum::Str(&dict[codes[row] as usize]),
                    },
                }
            };
        // A run of one series, in time order, grouped by buckets (and by
        // fields the FILTER fixed): each stretch of rows in one bucket is
        // folded together.
        let constant = |key: &Key<'_>| match key {
            Key::Bucket(..) => true,
            Key::Column(Source::Field(at)) => {
                plan.equals[*at].is_some() && matches!(columns.cols[*at], Column::Str { .. })
            }
            Key::Column(Source::Time) => false,
        };
        if in_slices && run && contiguous && plan.keys.iter().all(constant) {
            let mut at = start;
            while at < end {
                let mut next = at + 1;
                let same = |row: usize| {
                    plan.keys.iter().all(|key| match key {
                        Key::Bucket(step, _) => {
                            columns.time[row].div_euclid(*step)
                                == columns.time[at].div_euclid(*step)
                        }
                        Key::Column(_) => true,
                    })
                };
                while next < end && same(next) {
                    next += 1;
                }
                state.bucket_values(self, plan, columns.time[at])?;
                let key: Vec<Datum<'_>> = plan
                    .keys
                    .iter()
                    .enumerate()
                    .map(|(i, key)| match key {
                        Key::Column(source) => number(*source, at),
                        Key::Bucket(..) => state.datum(i),
                    })
                    .collect();
                fold.push_slices(self, &key, next - at, &slices(at, next))?;
                at = next;
            }
            return Ok(end - start);
        }

        let mut folded = 0usize;
        for row in start..end {
            let t = columns.time[row];
            if check_time && (t < plan.from || t >= plan.to) {
                continue;
            }
            if residual.iter().any(|(codes, code)| codes[row] != *code) {
                continue;
            }
            state.bucket_values(self, plan, t)?;
            let key: Vec<Datum<'_>> = plan
                .keys
                .iter()
                .enumerate()
                .map(|(i, key)| match key {
                    Key::Column(source) => number(*source, row),
                    Key::Bucket(..) => state.datum(i),
                })
                .collect();
            let ordinal = fold.next_ordinal();
            fold.push_datums(
                self,
                &key,
                plan.args
                    .iter()
                    .map(|arg| arg.map(|source| number(source, row))),
                ordinal,
            )?;
            folded += 1;
        }
        Ok(folded)
    }
}

/// A stored total as what an aggregate folds.
fn total_slice(total: &Total) -> Slice<'_> {
    Slice::Total {
        sum: &total.sum,
        min: total.min.number(),
        max: total.max.number(),
    }
}

/// The value of each bucket key for the bucket the last row fell in, kept
/// from one block to the next: `TIME_BUCKET` is evaluated once per bucket.
struct Buckets<'a> {
    values: Vec<Option<(i64, Value)>>,
    /// The name of the time field, the loop variable and the outer row:
    /// what the expression is evaluated on.
    time: &'a str,
    var: &'a str,
    outer: &'a Context,
    scratch: Option<Context>,
}

impl<'a> Buckets<'a> {
    fn new(plan: &BlockPlan<'_>, time: &'a str, var: &'a str, outer: &'a Context) -> Self {
        Buckets {
            values: plan.keys.iter().map(|_| None).collect(),
            time,
            var,
            outer,
            scratch: None,
        }
    }

    /// Make the values those of a point at `time`.
    fn bucket_values(
        &mut self,
        ex: &QueryExecutor<'_>,
        plan: &BlockPlan<'_>,
        time: i64,
    ) -> DbResult<()> {
        for (at, key) in plan.keys.iter().enumerate() {
            let Key::Bucket(step, expr) = key else {
                continue;
            };
            let id = time.div_euclid(*step);
            if self.values[at]
                .as_ref()
                .is_some_and(|(held, _)| *held == id)
            {
                continue;
            }
            let outer = self.outer;
            let ctx = self.scratch.get_or_insert_with(|| outer.clone());
            let mut point = serde_json::Map::new();
            point.insert(self.time.to_string(), Value::from(time));
            bind_in_place(ctx, self.var, Value::Object(point));
            self.values[at] = Some((id, ex.evaluate_expr_with_context(expr, ctx)?));
        }
        Ok(())
    }

    /// The value of bucket key `at`, after [`Self::bucket_values`].
    fn datum(&self, at: usize) -> Datum<'_> {
        match &self.values[at] {
            Some((_, value)) => Datum::of(std::borrow::Cow::Borrowed(value)),
            None => Datum::Null,
        }
    }
}
