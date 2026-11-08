//! Aggregation functions for SDBQL executor.
//!
//! This module contains aggregation logic:
//! - AggregateAccumulator: streaming COUNT/SUM/AVG/MIN/MAX/... for COLLECT groups
//! - try_columnar_aggregation: Optimized columnar aggregation path

use std::borrow::Cow;
use std::collections::HashMap;

use serde_json::Value;

use super::builtins::array::as_int;
use super::builtins::math::{median_of, Welford};
use super::datum::Datum;
use super::exact_sum::ExactSum;
use super::types::Context;
use super::{compare_values, hash_value, values_equal, QueryExecutor};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::*;
use crate::storage::{AggregateOp, ColumnarCollection};

impl<'a> QueryExecutor<'a> {
    pub(super) fn try_columnar_aggregation(
        &self,
        query: &Query,
        _initial_bindings: &Context,
    ) -> DbResult<Option<Vec<Value>>> {
        // Must have a database context
        let db_name = match &self.database {
            Some(name) => name,
            None => return Ok(None),
        };

        // Get database to check if collection is columnar
        let database = match self.storage.get_database(db_name) {
            Ok(db) => db,
            Err(_) => return Ok(None),
        };

        // Check pattern: FOR clause on collection, COLLECT with AGGREGATE, RETURN
        if query.body_clauses.len() != 2 {
            return Ok(None);
        }

        // First clause must be FOR on a collection
        let for_clause = match &query.body_clauses[0] {
            BodyClause::For(fc)
                if fc.source_expression.is_none() && fc.source_variable.is_none() =>
            {
                fc
            }
            _ => return Ok(None),
        };

        // Second clause must be COLLECT with AGGREGATE. Checked before the
        // columnar lookup, which is a storage read, so the common shapes
        // never pay for it.
        let collect_clause = match &query.body_clauses[1] {
            BodyClause::Collect(cc) if !cc.aggregates.is_empty() => cc,
            _ => return Ok(None),
        };

        // Check if collection is columnar
        let collection_name = &for_clause.collection;
        if !self.is_columnar(&database, collection_name) {
            return Ok(None);
        }

        // Must have a return clause
        if query.return_clause.is_none() {
            return Ok(None);
        }

        // Only the ungrouped, plain-AGGREGATE shape is answered from the
        // column store. INTO and WITH COUNT INTO need the rows themselves,
        // and grouping went through storage's string group keys (numbers
        // came back as floats, strings that looked like numbers came back as
        // numbers); the row path reads columnar collections too and gets
        // those right.
        if !collect_clause.group_vars.is_empty()
            || collect_clause.into_var.is_some()
            || collect_clause.count_var.is_some()
            || for_clause.system_time.is_some()
            || for_clause.valid_time.is_some()
        {
            return Ok(None);
        }

        // Load columnar collection
        let columnar =
            match ColumnarCollection::load(collection_name.clone(), db_name, database.db_arc()) {
                Ok(c) => c,
                Err(_) => return Ok(None),
            };
        let meta = match columnar.metadata() {
            Ok(m) => m,
            Err(_) => return Ok(None),
        };

        let mut result_obj: serde_json::Map<String, Value> = serde_json::Map::new();

        for agg in &collect_clause.aggregates {
            // Unknown names fall through to the row path, which reports them.
            let Some(kind) = AggKind::resolve(&agg.function, agg.argument.is_some()) else {
                return Ok(None);
            };

            let field = match &agg.argument {
                Some(Expression::FieldAccess(base, field)) => match base.as_ref() {
                    Expression::Variable(var) if var == &for_clause.variable => Some(field),
                    _ => return Ok(None),
                },
                None => None,
                // A bare variable is the whole document, not a column.
                Some(_) => return Ok(None),
            };
            let column = field.and_then(|f| meta.columns.iter().find(|c| &c.name == f));

            // Each storage operation is used only where its result is the
            // one the row path would produce for the same data.
            use crate::storage::columnar::ColumnType;
            let op = match (kind, field, column) {
                (AggKind::CountRows, _, _) => {
                    result_obj.insert(agg.variable.clone(), Value::Number(meta.row_count.into()));
                    continue;
                }
                // Storage counts stored cells; with no nulls that is the
                // non-null count.
                (AggKind::CountNonNull, Some(_), Some(c)) if !c.nullable => AggregateOp::Count,
                // Numeric-only in both paths.
                (AggKind::Sum, Some(_), _) => AggregateOp::Sum,
                (AggKind::Avg, Some(_), _) => AggregateOp::Avg,
                // Storage compares as f64 and returns a float: identical only
                // for a float column.
                (AggKind::Min, Some(_), Some(c)) if c.data_type == ColumnType::Float64 => {
                    AggregateOp::Min
                }
                (AggKind::Max, Some(_), Some(c)) if c.data_type == ColumnType::Float64 => {
                    AggregateOp::Max
                }
                (AggKind::CountDistinct, Some(_), Some(c))
                    if !c.nullable && c.data_type != ColumnType::Json =>
                {
                    AggregateOp::CountDistinct
                }
                _ => return Ok(None),
            };
            let field = field.expect("every columnar op above has a field");
            match columnar.aggregate(field, op) {
                Ok(value) => {
                    result_obj.insert(agg.variable.clone(), value);
                }
                Err(_) => return Ok(None),
            }
        }

        // Run the single aggregate row through the query's RETURN clause.
        let return_expr = &query
            .return_clause
            .as_ref()
            .expect("checked above")
            .expression;

        let mut ctx: Context = _initial_bindings.clone();
        for (k, v) in result_obj {
            ctx.insert(k, v);
        }
        Ok(Some(vec![
            self.evaluate_expr_with_context(return_expr, &ctx)?
        ]))
    }
}

/// An `AGGREGATE` function, resolved from its name.
///
/// This is the one table of aggregate names. The row path (`COLLECT`) and
/// the columnar fast path both resolve through it, so a name cannot mean one
/// thing on one storage layout and another thing on the other — which is how
/// `LENGTH` used to be a distinct count on rows and a row count on columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AggKind {
    /// `COUNT()`, `LENGTH(...)` — every row.
    CountRows,
    /// `COUNT(expr)` — rows whose value is not null.
    CountNonNull,
    Sum,
    Avg,
    Min,
    Max,
    /// `COUNT_DISTINCT(expr)` — number of distinct values (null included).
    CountDistinct,
    /// `UNIQUE(expr)` — the distinct values, in first-seen order.
    Unique,
    /// `SORTED_UNIQUE(expr)` — the distinct values, sorted.
    SortedUnique,
    /// `COLLECT_LIST(expr)` / `PUSH(expr)` — every value.
    List,
    Variance {
        sample: bool,
    },
    Stddev {
        sample: bool,
    },
    Median,
    BitAnd,
    BitOr,
    BitXor,
}

impl AggKind {
    pub(super) fn resolve(function: &str, has_argument: bool) -> Option<Self> {
        Some(match function.to_ascii_uppercase().as_str() {
            "COUNT" if !has_argument => Self::CountRows,
            "COUNT" => Self::CountNonNull,
            // AQL: LENGTH counts rows, like COUNT().
            "LENGTH" => Self::CountRows,
            "SUM" => Self::Sum,
            "AVG" | "AVERAGE" => Self::Avg,
            "MIN" | "MINIMUM" => Self::Min,
            "MAX" | "MAXIMUM" => Self::Max,
            "COUNT_DISTINCT" | "COUNT_UNIQUE" => Self::CountDistinct,
            "UNIQUE" => Self::Unique,
            "SORTED_UNIQUE" => Self::SortedUnique,
            "COLLECT_LIST" | "COLLECT" | "PUSH" => Self::List,
            "VARIANCE" | "VARIANCE_POPULATION" | "VAR_POP" => Self::Variance { sample: false },
            "VARIANCE_SAMPLE" | "VAR_SAMP" => Self::Variance { sample: true },
            "STDDEV" | "STDDEV_POPULATION" | "STDDEV_POP" => Self::Stddev { sample: false },
            "STDDEV_SAMPLE" | "STDDEV_SAMP" => Self::Stddev { sample: true },
            "MEDIAN" => Self::Median,
            "BIT_AND" => Self::BitAnd,
            "BIT_OR" => Self::BitOr,
            "BIT_XOR" => Self::BitXor,
            _ => return None,
        })
    }
}

/// Distinct values in first-seen order, hashed with `hash_value` and
/// compared with `values_equal` — the same equality as `UNIQUE()` and
/// `COUNT_DISTINCT()` on arrays. (It used to be the JSON serialisation, so
/// `1` and `1.0` were two values.)
#[derive(Default)]
pub(super) struct DistinctValues {
    index: HashMap<u64, Vec<usize>>,
    items: Vec<Value>,
}

impl DistinctValues {
    /// Returns whether `v` was new (and is now held). Copied only then.
    fn insert(&mut self, v: Cow<'_, Value>) -> bool {
        let bucket = self.index.entry(hash_value(&v)).or_default();
        if bucket.iter().any(|&i| values_equal(&self.items[i], &v)) {
            return false;
        }
        bucket.push(self.items.len());
        self.items.push(v.into_owned());
        true
    }
}

/// Running state of one `AGGREGATE` function inside one `COLLECT` group.
///
/// `COLLECT` used to keep every row of every group alive until the end of the
/// clause and then compute each aggregate over that list, so `COLLECT
/// AGGREGATE total = SUM(doc.amount)` over 5M rows held 5M cloned contexts —
/// the intermediate row ceiling counted them once, memory paid for them twice.
/// Folding each row in as it arrives keeps the clause at O(groups) for every
/// function except those whose output is (or needs) the values themselves:
/// `COLLECT_LIST`, `UNIQUE`, `SORTED_UNIQUE`, `COUNT_DISTINCT`, `MEDIAN`.
///
/// `SUM` and `AVG` add exactly ([`ExactSum`]) and round once, so their result
/// does not depend on the order the rows arrive in — which is what lets a
/// scan read on several threads ([`Self::merge`]) give the answer of one.
pub(super) enum AggregateAccumulator {
    CountRows(i64),
    CountNonNull(i64),
    Sum(ExactSum),
    Avg {
        sum: ExactSum,
        count: i64,
    },
    /// The extreme so far, and the ordinal of the row it came from: of equal
    /// extremes the earliest row's is kept, which a merge of partial results
    /// read out of order can still tell.
    Min(Option<(Value, u64)>),
    Max(Option<(Value, u64)>),
    CountDistinct(DistinctValues),
    Unique {
        values: DistinctValues,
        sorted: bool,
    },
    List(Vec<Value>),
    Stats {
        acc: Welford,
        sample: bool,
        sqrt: bool,
    },
    Median(Vec<f64>),
    /// Null values are skipped; any other non-integer makes the result null.
    Bits {
        kind: AggKind,
        acc: Option<i64>,
        invalid: bool,
    },
}

impl AggregateAccumulator {
    pub(super) fn new(function: &str, has_argument: bool) -> DbResult<Self> {
        let kind = AggKind::resolve(function, has_argument).ok_or_else(|| {
            DbError::ExecutionError(format!("Unknown aggregate function: {}", function))
        })?;
        Ok(Self::of_kind(kind))
    }

    pub(super) fn of_kind(kind: AggKind) -> Self {
        match kind {
            AggKind::CountRows => Self::CountRows(0),
            AggKind::CountNonNull => Self::CountNonNull(0),
            AggKind::Sum => Self::Sum(ExactSum::default()),
            AggKind::Avg => Self::Avg {
                sum: ExactSum::default(),
                count: 0,
            },
            AggKind::Min => Self::Min(None),
            AggKind::Max => Self::Max(None),
            AggKind::CountDistinct => Self::CountDistinct(DistinctValues::default()),
            AggKind::Unique => Self::Unique {
                values: DistinctValues::default(),
                sorted: false,
            },
            AggKind::SortedUnique => Self::Unique {
                values: DistinctValues::default(),
                sorted: true,
            },
            AggKind::List => Self::List(Vec::new()),
            AggKind::Variance { sample } => Self::Stats {
                acc: Welford::default(),
                sample,
                sqrt: false,
            },
            AggKind::Stddev { sample } => Self::Stats {
                acc: Welford::default(),
                sample,
                sqrt: true,
            },
            AggKind::Median => Self::Median(Vec::new()),
            AggKind::BitAnd | AggKind::BitOr | AggKind::BitXor => Self::Bits {
                kind,
                acc: None,
                invalid: false,
            },
        }
    }

    /// Fold one row in. `value` is `None` when the aggregate has no argument,
    /// in which case only the row counts have anything to count. A borrowed
    /// value is copied only when it is kept (a new extreme, a list item, a
    /// distinct value not seen before).
    ///
    /// Returns whether the value was retained (as opposed to reduced), so the
    /// caller can budget the memory this clause is holding on to. `ordinal`
    /// is the row's position in the input, increasing.
    #[inline]
    pub(super) fn push(&mut self, value: Option<Datum<'_>>, ordinal: u64) -> bool {
        match self {
            Self::CountRows(n) => {
                *n += 1;
                false
            }
            Self::CountNonNull(n) => {
                if matches!(value, Some(ref v) if !v.is_null()) {
                    *n += 1;
                }
                false
            }
            Self::Sum(sum) => {
                if let Some(n) = value.as_ref().and_then(Datum::as_f64) {
                    sum.add(n);
                }
                false
            }
            Self::Avg { sum, count } => {
                if let Some(n) = value.as_ref().and_then(Datum::as_f64) {
                    sum.add(n);
                    *count += 1;
                }
                false
            }
            // AQL order via compare_values; null is skipped and the first of
            // equal extremes is kept.
            Self::Min(best) => {
                if let Some(val) = value.filter(|v| !v.is_null()) {
                    if best
                        .as_ref()
                        .is_none_or(|(b, _)| val.cmp_value(b) == std::cmp::Ordering::Less)
                    {
                        *best = Some((val.into_value(), ordinal));
                    }
                }
                false
            }
            Self::Max(best) => {
                if let Some(val) = value.filter(|v| !v.is_null()) {
                    if best
                        .as_ref()
                        .is_none_or(|(b, _)| val.cmp_value(b) == std::cmp::Ordering::Greater)
                    {
                        *best = Some((val.into_value(), ordinal));
                    }
                }
                false
            }
            Self::CountDistinct(values) | Self::Unique { values, .. } => match value {
                Some(v) => values.insert(v.into_cow()),
                None => false,
            },
            Self::List(list) => match value {
                Some(v) => {
                    list.push(v.into_value());
                    true
                }
                None => false,
            },
            Self::Stats { acc, .. } => {
                if let Some(n) = value.as_ref().and_then(Datum::as_f64) {
                    acc.push(n);
                }
                false
            }
            Self::Median(nums) => match value.as_ref().and_then(Datum::as_f64) {
                Some(n) => {
                    nums.push(n);
                    true
                }
                None => false,
            },
            Self::Bits { kind, acc, invalid } => {
                match &value {
                    None => {}
                    Some(v) if v.is_null() => {}
                    Some(v) => match v.as_value().as_deref().and_then(as_int) {
                        Some(n) => *acc = Some(bit_combine(*kind, *acc, n)),
                        None => *invalid = true,
                    },
                }
                false
            }
        }
    }

    /// Whether [`Self::merge`] gives exactly what one accumulator would have
    /// computed from all the rows, whichever rows each partial result saw
    /// and in whatever order they are merged. Counts, exact sums and bit
    /// operations do not depend on order; MIN and MAX keep their row's
    /// ordinal to break ties as one pass would. The others keep values in
    /// arrival order (lists, first-seen distinct values, the median's
    /// `-0.0`/`0.0`) or round in it (the variance family).
    pub(super) fn mergeable(kind: AggKind) -> bool {
        matches!(
            kind,
            AggKind::CountRows
                | AggKind::CountNonNull
                | AggKind::Sum
                | AggKind::Avg
                | AggKind::Min
                | AggKind::Max
                | AggKind::CountDistinct
                | AggKind::BitAnd
                | AggKind::BitOr
                | AggKind::BitXor
        )
    }

    /// Fold in `other`, a partial result over other rows of the same group
    /// (see [`Self::mergeable`]). Returns how many retained values `other`
    /// held that are now dropped as duplicates.
    pub(super) fn merge(&mut self, other: Self) -> usize {
        use std::cmp::Ordering::{Equal, Greater, Less};
        // Of two equal extremes, the one from the earlier row.
        let better = |mine: &Option<(Value, u64)>, theirs: &(Value, u64), wanted| match mine {
            None => true,
            Some((v, at)) => match compare_values(&theirs.0, v) {
                Equal => theirs.1 < *at,
                o => o == wanted,
            },
        };
        match (self, other) {
            (Self::CountRows(a), Self::CountRows(b))
            | (Self::CountNonNull(a), Self::CountNonNull(b)) => {
                *a += b;
                0
            }
            (Self::Sum(a), Self::Sum(b)) => {
                a.merge(&b);
                0
            }
            (
                Self::Avg { sum, count },
                Self::Avg {
                    sum: other_sum,
                    count: other_count,
                },
            ) => {
                sum.merge(&other_sum);
                *count += other_count;
                0
            }
            (Self::Min(a), Self::Min(Some(b))) => {
                if better(a, &b, Less) {
                    *a = Some(b);
                }
                0
            }
            (Self::Max(a), Self::Max(Some(b))) => {
                if better(a, &b, Greater) {
                    *a = Some(b);
                }
                0
            }
            (Self::Min(_), Self::Min(None)) | (Self::Max(_), Self::Max(None)) => 0,
            (Self::CountDistinct(a), Self::CountDistinct(b)) => {
                let held = b.items.len();
                let kept = b
                    .items
                    .into_iter()
                    .filter(|v| a.insert(Cow::Borrowed(v)))
                    .count();
                held - kept
            }
            (
                Self::Bits { kind, acc, invalid },
                Self::Bits {
                    acc: other_acc,
                    invalid: other_invalid,
                    ..
                },
            ) => {
                if let Some(n) = other_acc {
                    *acc = Some(bit_combine(*kind, *acc, n));
                }
                *invalid |= other_invalid;
                0
            }
            _ => unreachable!("only mergeable accumulators of one kind are merged"),
        }
    }

    pub(super) fn finish(self) -> Value {
        match self {
            Self::CountRows(n) | Self::CountNonNull(n) => Value::Number(n.into()),
            Self::Sum(sum) => float_value(sum.value()),
            Self::Avg { count: 0, .. } => Value::Null,
            Self::Avg { sum, count } => float_value(sum.value() / count as f64),
            Self::Min(v) | Self::Max(v) => v.map_or(Value::Null, |(v, _)| v),
            Self::CountDistinct(values) => Value::Number((values.items.len() as i64).into()),
            Self::Unique { values, sorted } => {
                let mut items = values.items;
                if sorted {
                    items.sort_by(compare_values);
                }
                Value::Array(items)
            }
            Self::List(list) => Value::Array(list),
            Self::Stats { acc, sample, sqrt } => match acc.variance(sample) {
                Some(v) => float_value(if sqrt { v.sqrt() } else { v }),
                None => Value::Null,
            },
            Self::Median(mut nums) => median_of(&mut nums).map(float_value).unwrap_or(Value::Null),
            Self::Bits { acc, invalid, .. } => match (acc, invalid) {
                (Some(n), false) => Value::Number(n.into()),
                _ => Value::Null,
            },
        }
    }
}

/// One step of BIT_AND / BIT_OR / BIT_XOR: associative and commutative, so
/// partial results combine in any grouping.
#[inline]
fn bit_combine(kind: AggKind, acc: Option<i64>, n: i64) -> i64 {
    match (acc, kind) {
        (None, _) => n,
        (Some(a), AggKind::BitAnd) => a & n,
        (Some(a), AggKind::BitOr) => a | n,
        (Some(a), _) => a ^ n,
    }
}

/// A float result; non-finite (an overflowing SUM) is null, as JSON has no
/// representation for it.
fn float_value(f: f64) -> Value {
    serde_json::Number::from_f64(f)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fold(function: &str, values: &[Value]) -> Value {
        let mut acc = AggregateAccumulator::new(function, true).unwrap();
        for (i, v) in values.iter().enumerate() {
            acc.push(Some(Datum::of(Cow::Borrowed(v))), i as u64);
        }
        acc.finish()
    }

    #[test]
    fn one_name_table() {
        assert_eq!(AggKind::resolve("length", true), Some(AggKind::CountRows));
        assert_eq!(AggKind::resolve("COUNT", false), Some(AggKind::CountRows));
        assert_eq!(AggKind::resolve("COUNT", true), Some(AggKind::CountNonNull));
        assert_eq!(AggKind::resolve("AVERAGE", true), Some(AggKind::Avg));
        assert_eq!(AggKind::resolve("MINIMUM", true), Some(AggKind::Min));
        assert_eq!(AggKind::resolve("MAXIMUM", true), Some(AggKind::Max));
        assert_eq!(AggKind::resolve("UNIQUE", true), Some(AggKind::Unique));
        assert_eq!(AggKind::resolve("NOPE", true), None);
        assert!(AggregateAccumulator::new("NOPE", true).is_err());
    }

    #[test]
    fn length_counts_rows_and_unique_is_an_array() {
        let vals = [json!(1), json!(1.0), json!(null), json!("a")];
        assert_eq!(fold("LENGTH", &vals), json!(4));
        assert_eq!(fold("COUNT", &vals), json!(3));
        assert_eq!(fold("UNIQUE", &vals), json!([1, null, "a"]));
        assert_eq!(fold("COUNT_DISTINCT", &vals), json!(3));
        assert_eq!(
            fold("SORTED_UNIQUE", &[json!(3), json!(1), json!(3), json!(2)]),
            json!([1, 2, 3])
        );
    }

    #[test]
    fn min_max_use_the_aql_order() {
        let vals = [json!("b"), json!(5), json!(null), json!("a")];
        assert_eq!(fold("MIN", &vals), json!(5));
        assert_eq!(fold("MAX", &vals), json!("b"));
    }

    #[test]
    fn statistics() {
        let vals: Vec<Value> = [1, 2, 3, 4, 5].iter().map(|n| json!(n)).collect();
        assert_eq!(fold("VARIANCE", &vals), json!(2.0));
        assert_eq!(fold("VARIANCE_SAMPLE", &vals), json!(2.5));
        assert_eq!(fold("STDDEV_POPULATION", &vals), json!(2f64.sqrt()));
        assert_eq!(fold("MEDIAN", &vals), json!(3.0));
        assert_eq!(fold("VARIANCE", &[json!(7)]), json!(0.0));
        assert_eq!(fold("STDDEV_SAMPLE", &[json!(7)]), Value::Null);
        assert_eq!(fold("MEDIAN", &[]), Value::Null);
        assert_eq!(fold("AVERAGE", &vals), json!(3.0));
    }

    #[test]
    fn bit_aggregates() {
        let vals = [json!(12), json!(10.0), json!(null)];
        assert_eq!(fold("BIT_AND", &vals), json!(8));
        assert_eq!(fold("BIT_OR", &vals), json!(14));
        assert_eq!(fold("BIT_XOR", &vals), json!(6));
        assert_eq!(fold("BIT_OR", &[json!(1), json!("x")]), Value::Null);
        assert_eq!(fold("BIT_AND", &[]), Value::Null);
    }
}
