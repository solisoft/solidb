//! A COLLECT over a columnar collection, folded from its blocks.
//!
//! `FOR r IN c [FILTER …] COLLECT … AGGREGATE …` over a columnar collection
//! used to build every row as an object, then filter and group the
//! objects. When the query stays within what the arrays answer —
//!
//! - every FILTER is made of comparisons of a column with a constant
//!   (`==`, `!=`, `<`, `<=`, `>`, `>=` with a string, a number, a
//!   boolean or null, on either side; `IN` and `NOT IN` a list of such)
//!   or with another column, joined by `AND`, `OR` and `NOT`, or is any
//!   other condition computed from the columns (below);
//! - every group key is a column, `TIME_BUCKET(column, "<fixed
//!   interval>")` of a column of integers, or an expression computed from
//!   the columns;
//! - every aggregate is one that takes whole columns (`COUNT`, `SUM`,
//!   `AVG`, `MIN`, `MAX`) or `COUNT_DISTINCT`, of a column, of an
//!   expression computed from the columns, or of nothing
//!
//! — the blocks (`storage::columnar_blocks`) are read as arrays, on several
//! threads when they are many: rows are selected by comparing array
//! elements, grouped in a table keyed by the key columns' values, and each
//! group is handed to the same [`CollectFold`] the ordinary path fills,
//! with the exact state of its sums. A column that holds nulls is read
//! with its mask: to a comparison a null is below every value and equal
//! to none, a null key is a group of its own, and an aggregate passes a
//! null over. A block that does not fit (a column with values of several
//! kinds, or compared with a constant of another kind, or that the block
//! was sealed without) and the rows of the tail are read as objects and
//! folded the ordinary way, FILTERs included: the answer is the ordinary
//! path's.
//!
//! An expression computed from the columns (`Derived`: `r.price * r.qty`,
//! `UPPER(r.host)`, `DATE_TRUNC(r.ts, "hour")`, `CONTAINS(r.host, "db")`)
//! is made of the row's columns, constants and functions that give the
//! same value for the same arguments. Its values for a block are computed
//! once, kept as a column for the query and shared by the threads:
//! arithmetic on columns of numbers in the arrays (`Sum`, held to
//! `helpers::arithmetic` value for value by a test), an expression of one
//! column of strings once per string of the block, anything else by the
//! evaluator, for each row, from the columns it reads. Values that are
//! not of one kind send the block to the rows, except integers and
//! floats together under an aggregate (`ColData::Mixed`).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::super::aggregation::{AggKind, AggregateAccumulator, Slice};
use super::super::builtins::approx::Hll;
use super::super::builtins::array::as_int;
use super::super::builtins::math::Moments;
use super::super::datum::Datum;
use super::super::exact_sum::ExactSum;
use super::super::index_opt::{expression_references_var, push_conjuncts, IndexHint};
use super::super::types::Context;
use super::super::QueryExecutor;
use super::collect::CollectFold;
use super::collect::FoldState;
use super::scan_collect::{bind_in_place, plan, Plan, ScanCollectRun};
use super::scan_threads::Helpers;
use crate::error::DbError;
use crate::error::DbResult;
use crate::sdbql::ast::{BinaryOperator, BodyClause, Expression, UnaryOperator};
use crate::sdbql::executor::builtins::datetime::WEEK_ORIGIN_MS;
use crate::sdbql::executor::builtins::timeseries::{parse_interval, Interval};
use crate::sdbql::executor::const_fold::is_pure_function;
use crate::sdbql::executor::helpers::{compare_values, hash_f64, hash_str, hash_value};
use crate::storage::columnar_blocks::{ColBlock, ColData, Column};

/// A comparison of a column with a constant.
struct Cond {
    column: String,
    op: BinaryOperator,
    value: Value,
}

/// What a FILTER keeps: comparisons, joined.
enum Pred {
    Cmp(Cond),
    /// Every one of them (`AND`).
    All(Vec<Pred>),
    /// One of them at least (`OR`, `IN`).
    Any(Vec<Pred>),
    /// The rows it does not keep (`NOT`, `NOT IN`).
    Not(Box<Pred>),
    /// A comparison of two columns.
    Cols {
        left: String,
        op: BinaryOperator,
        right: String,
    },
    /// Any other condition on the columns: the rows where the computed
    /// column `ColPlan::made[n]` is true.
    Made(usize),
}

/// A group key.
enum Key {
    Column(String),
    /// `TIME_BUCKET(column, "<interval>")` for an interval of `step`
    /// milliseconds counted from `origin`: the instant its bucket starts
    /// at, computed here when the column is of integers, and as
    /// `ColPlan::made[made]` otherwise.
    Bucket {
        column: String,
        step: i64,
        origin: i64,
        made: usize,
    },
    /// Any other expression of the columns: `ColPlan::made[n]`.
    Made(usize),
}

/// What an aggregate reads.
enum Source {
    Column(String),
    /// An expression of the columns: `ColPlan::made[n]`.
    Made(usize),
}

/// An expression of the row's columns that the arrays do not compute
/// themselves (`DATE_TRUNC(r.day, "month")`, `r.price * r.qty`,
/// `CONTAINS(r.host, "db")`): its value for every row of a block is
/// computed once, by the evaluator every other query goes through, from
/// the columns it reads and nothing else of the row, and kept as a column
/// of the block for the query.
struct Derived<'q> {
    expr: &'q Expression,
    /// The columns of the row it reads.
    columns: Vec<String>,
    /// Whether integers and floats together make a column: for what an
    /// aggregate adds up or compares, not for a key or a condition.
    numbers: bool,
}

/// The computed columns of one block, each made when first asked for and
/// shared by the threads that read the block.
type Cells = Vec<std::sync::OnceLock<Option<Column>>>;

/// What computing a column needs of the query.
struct Eval<'a, 'e> {
    ex: &'a QueryExecutor<'e>,
    outer: &'a Context,
    var: &'a str,
}

struct ColPlan<'q> {
    made: Vec<Derived<'q>>,
    preds: Vec<Pred>,
    keys: Vec<Key>,
    /// For each aggregate, its argument and what it computes.
    args: Vec<Option<Source>>,
    kinds: Vec<AggKind>,
    /// For each aggregate, the one that keeps the numbers it reads
    /// (`CollectFold::shared_numbers`).
    shared: Vec<usize>,
}

/// A value of a key column.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Atom {
    Int(i64),
    /// The bits of a float.
    Float(u64),
    Str(Arc<str>),
    Bool(bool),
    Null,
}

/// Distinct values kept as they come, then sorted and each kept once
/// when there are twice as many as the last time: no hashing per row, and
/// never more than about twice the distinct values held.
#[derive(Default)]
struct Uniques<T> {
    values: Vec<T>,
    /// The length at which to sort again.
    next: usize,
}

impl<T: Ord + Copy> Uniques<T> {
    #[inline]
    fn push(&mut self, value: T) {
        self.values.push(value);
        if self.values.len() >= self.next.max(1024) {
            self.settle();
        }
    }

    /// Sorted, each value once.
    fn settle(&mut self) {
        self.values.sort_unstable();
        self.values.dedup();
        self.next = self.values.len() * 2;
    }

    fn merge(&mut self, other: &Self) {
        self.values.extend_from_slice(&other.values);
        if self.values.len() >= self.next.max(1024) {
            self.settle();
        }
    }
}

/// What one aggregate holds of one group's numbers.
#[derive(Default)]
struct Acc {
    /// The rows of the group whose argument is null: what the aggregate
    /// passes over.
    missing: u64,
    /// The distinct values of the argument, for `COUNT_DISTINCT`: those of
    /// a column of integers, those of a column of floats (their bits),
    /// and any other.
    ints: Uniques<i64>,
    floats: Uniques<u64>,
    distinct: std::collections::HashSet<Atom, foldhash::fast::RandomState>,
    /// Whether `distinct` holds numbers (a column of integers and floats
    /// together): `1` and `1.0` are then two of its values, and one to
    /// the query.
    mixed: bool,
    sum: ExactSum,
    /// The sum of the squares, for the variance family.
    squares: ExactSum,
    /// The numbers themselves, for a median or a percentile.
    values: Vec<f64>,
    /// The integers combined, for `BIT_AND` and its kin, and whether a
    /// number that is not an integer was met (the result is then null).
    bits: Option<i64>,
    not_bits: bool,
    /// Which of them combined `bits`: what a merge combines with.
    bit_kind: Option<AggKind>,
    /// The sketch of the argument's values, for `APPROX_COUNT_DISTINCT`.
    sketch: Hll,
    /// The extremes, and whether each is held as an integer.
    min: Option<(f64, bool)>,
    max: Option<(f64, bool)>,
}

impl Acc {
    #[inline]
    fn int(&mut self, v: i64) {
        self.sum.add_i64(v);
        self.extreme(v as f64, true);
    }

    #[inline]
    fn float(&mut self, v: f64) {
        self.sum.add(v);
        self.extreme(v, false);
    }

    /// [`Self::float`] of each of `values`.
    fn floats(&mut self, values: &[f64]) {
        self.sum.add_slice(values);
        let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
        for &v in values {
            min = min.min(v);
            max = max.max(v);
        }
        if !values.is_empty() {
            self.extreme(min, false);
            self.extreme(max, false);
        }
    }

    #[inline]
    fn extreme(&mut self, v: f64, int: bool) {
        if self.min.is_none_or(|(held, _)| v < held) {
            self.min = Some((v, int));
        }
        if self.max.is_none_or(|(held, _)| v > held) {
            self.max = Some((v, int));
        }
    }

    fn merge(&mut self, other: &Acc) {
        self.missing += other.missing;
        self.ints.merge(&other.ints);
        self.floats.merge(&other.floats);
        self.mixed |= other.mixed;
        self.distinct.extend(other.distinct.iter().cloned());
        self.sum.merge(&other.sum);
        self.squares.merge(&other.squares);
        self.values.extend_from_slice(&other.values);
        self.sketch.merge(&other.sketch);
        self.not_bits |= other.not_bits;
        if let (Some(n), Some(kind)) = (other.bits, other.bit_kind) {
            self.bits = Some(AggregateAccumulator::bits(kind, self.bits, n));
            self.bit_kind = Some(kind);
        }
        if let Some((v, int)) = other.min {
            if self.min.is_none_or(|(held, _)| v < held) {
                self.min = Some((v, int));
            }
        }
        if let Some((v, int)) = other.max {
            if self.max.is_none_or(|(held, _)| v > held) {
                self.max = Some((v, int));
            }
        }
    }
}

struct Group {
    key: Vec<Atom>,
    count: u64,
    accs: Vec<Acc>,
}

/// The groups met so far, by key.
#[derive(Default)]
struct Table {
    index: HashMap<Vec<Atom>, u32, foldhash::fast::RandomState>,
    /// The groups of a single numeric key, by the bits of its value:
    /// found without building a key.
    numbers: HashMap<u64, u32, foldhash::fast::RandomState>,
    /// The groups of a single integer key of small range, by its distance
    /// from the smallest (`u32::MAX`: none yet).
    dense: Vec<u32>,
    /// The same groups as plain arrays, while their sums can be.
    flat: Option<Flat>,
    groups: Vec<Group>,
}

/// Which rows a thread takes when the groups are shared out by key: those
/// whose key falls to it. Each group is then in one thread's table, and
/// nothing has to be merged.
#[derive(Clone, Copy)]
enum Share {
    /// Every row: the blocks are what is shared out.
    All,
    /// Thread `mine` of `of`, by a hash of the key's bits.
    Hashed { mine: u64, of: u64 },
    /// Thread `mine` of `of`, by where an integer key lies between `base`
    /// and `base + span`: the groups are then found in a table indexed by
    /// that distance.
    Ranged {
        mine: u64,
        /// `(of << 32) / (span + 1)`: a distance times this, shifted
        /// right by 32, is the thread it falls to.
        scale: u64,
        base: i64,
        span: u64,
        /// Whether to try the sums in plain arrays first ([`Flat`]).
        flat: bool,
    },
}

/// The groups of an integer key of small range, as arrays indexed by the
/// key's distance from the smallest: a count, and per aggregate a sum and
/// the extremes. A row then touches a few adjacent numbers, not a group
/// with its own exact sum somewhere in memory.
///
/// The sums are plain numbers that still say the exact sum. A float sum
/// goes with the total of what its additions rounded away (each such
/// error is itself a float, exactly): the two together are the exact sum
/// for as long as adding the errors up rounds nothing. An integer sum is
/// kept while it and its values are integers a float holds exactly. When
/// one of those conditions fails, `ok` goes false and the rows are read
/// again with exact accumulators.
struct Flat {
    counts: Vec<u64>,
    cols: Vec<FlatCol>,
    ok: bool,
}

enum FlatCol {
    /// No argument, or a column that is only counted.
    None,
    Float {
        sum: Vec<f64>,
        /// What the additions of `sum` rounded away, added up: `sum + err`
        /// is the exact sum while adding to `err` rounds nothing itself.
        err: Vec<f64>,
        min: Vec<f64>,
        max: Vec<f64>,
    },
    Int {
        sum: Vec<i64>,
        min: Vec<i64>,
        max: Vec<i64>,
    },
}

impl Flat {
    fn new(groups: usize, args: &[Option<&Column>]) -> Self {
        let cols = args
            .iter()
            .map(|column| match column.map(|c| &c.data) {
                Some(ColData::Float(_)) => FlatCol::Float {
                    sum: vec![0.0; groups],
                    err: vec![0.0; groups],
                    min: vec![f64::INFINITY; groups],
                    max: vec![f64::NEG_INFINITY; groups],
                },
                Some(ColData::Int(_)) => FlatCol::Int {
                    sum: vec![0; groups],
                    min: vec![i64::MAX; groups],
                    max: vec![i64::MIN; groups],
                },
                _ => FlatCol::None,
            })
            .collect();
        Flat {
            counts: vec![0; groups],
            cols,
            ok: true,
        }
    }

    #[inline]
    fn add(&mut self, at: usize, row: usize, args: &[Option<&Column>]) {
        self.counts[at] += 1;
        for (col, column) in self.cols.iter_mut().zip(args) {
            match (col, column.map(|c| &c.data)) {
                (FlatCol::Float { sum, err, min, max }, Some(ColData::Float(values))) => {
                    let x = values[row];
                    let (next, lost) = two_sum(sum[at], x);
                    sum[at] = next;
                    if lost != 0.0 {
                        let (kept, gone) = two_sum(err[at], lost);
                        err[at] = kept;
                        self.ok &= gone == 0.0;
                    }
                    min[at] = min[at].min(x);
                    max[at] = max[at].max(x);
                }
                (FlatCol::Int { sum, min, max }, Some(ColData::Int(values))) => {
                    let x = values[row];
                    match sum[at].checked_add(x) {
                        Some(next) if x.unsigned_abs() <= 1 << 53 => sum[at] = next,
                        _ => self.ok = false,
                    }
                    min[at] = min[at].min(x);
                    max[at] = max[at].max(x);
                }
                (FlatCol::None, _) => {}
                // A column of another kind in this block than in the first.
                _ => self.ok = false,
            }
        }
    }

    /// Add the groups of `other`, a table of the same keys filled from
    /// other rows: sums are added as the rows were, under the same
    /// conditions.
    fn absorb(&mut self, other: &Flat) {
        self.ok &= other.ok && self.cols.len() == other.cols.len();
        for (mine, theirs) in self.counts.iter_mut().zip(&other.counts) {
            *mine += theirs;
        }
        for (mine, theirs) in self.cols.iter_mut().zip(&other.cols) {
            match (mine, theirs) {
                (FlatCol::None, FlatCol::None) => {}
                (
                    FlatCol::Float { sum, err, min, max },
                    FlatCol::Float {
                        sum: more,
                        err: rest,
                        min: lower,
                        max: higher,
                    },
                ) => {
                    for at in 0..sum.len().min(more.len()) {
                        let (next, lost) = two_sum(sum[at], more[at]);
                        sum[at] = next;
                        let (kept, gone) = two_sum(err[at], rest[at]);
                        let (kept, gone_too) = two_sum(kept, lost);
                        err[at] = kept;
                        self.ok &= gone == 0.0 && gone_too == 0.0;
                        min[at] = min[at].min(lower[at]);
                        max[at] = max[at].max(higher[at]);
                    }
                }
                (
                    FlatCol::Int { sum, min, max },
                    FlatCol::Int {
                        sum: more,
                        min: lower,
                        max: higher,
                    },
                ) => {
                    for at in 0..sum.len().min(more.len()) {
                        match sum[at].checked_add(more[at]) {
                            Some(next) => sum[at] = next,
                            None => self.ok = false,
                        }
                        min[at] = min[at].min(lower[at]);
                        max[at] = max[at].max(higher[at]);
                    }
                }
                _ => self.ok = false,
            }
        }
    }

    /// Whether every plain sum is the exact one: no addition lost
    /// anything, and each sum is a number its exact form holds.
    fn valid(&self) -> bool {
        self.ok
            && self.cols.iter().all(|col| match col {
                FlatCol::None => true,
                FlatCol::Float { sum, err, .. } => {
                    sum.iter().all(|s| s.is_finite()) && err.iter().all(|e| e.is_finite())
                }
                FlatCol::Int { sum, .. } => sum.iter().all(|s| s.unsigned_abs() <= 1 << 53),
            })
    }

    /// The positions that hold a group.
    fn present(&self) -> impl Iterator<Item = usize> + '_ {
        self.counts
            .iter()
            .enumerate()
            .filter(|(_, count)| **count > 0)
            .map(|(at, _)| at)
    }

    /// What aggregate `agg`, of kind `kind`, is worth for the group at
    /// `at`, as a float: what a SORT on it compares.
    fn score(&self, at: usize, agg: usize, kind: AggKind) -> f64 {
        let (sum, min, max) = match &self.cols[agg] {
            FlatCol::None => (0.0, f64::NAN, f64::NAN),
            // Rounded once: the exact sum is `sum + err`.
            FlatCol::Float { sum, err, min, max } => (sum[at] + err[at], min[at], max[at]),
            FlatCol::Int { sum, min, max } => (sum[at] as f64, min[at] as f64, max[at] as f64),
        };
        match kind {
            AggKind::Sum => sum,
            AggKind::Avg => sum / self.counts[at] as f64,
            AggKind::Min => min,
            AggKind::Max => max,
            _ => self.counts[at] as f64,
        }
    }

    /// The groups `keep` wants, with exact sums. For a [valid](Self::valid)
    /// table.
    fn groups(&self, base: i64, keep: impl Fn(usize) -> bool) -> Vec<Group> {
        let mut groups = Vec::new();
        for at in self.present().filter(|at| keep(*at)) {
            groups.push(Group {
                key: vec![Atom::Int(base + at as i64)],
                count: self.counts[at],
                accs: self.accs(at),
            });
        }
        groups
    }

    /// The aggregates of the group at `at`, with exact sums. For a
    /// [valid](Self::valid) table.
    fn accs(&self, at: usize) -> Vec<Acc> {
        let mut accs = Vec::with_capacity(self.cols.len());
        for col in &self.cols {
            let mut acc = Acc::default();
            match col {
                FlatCol::None => {}
                FlatCol::Float { sum, err, min, max } => {
                    acc.sum.add(sum[at]);
                    if err[at] != 0.0 {
                        acc.sum.add(err[at]);
                    }
                    acc.min = Some((min[at], false));
                    acc.max = Some((max[at], false));
                }
                FlatCol::Int { sum, min, max } => {
                    acc.sum.add_i64(sum[at]);
                    acc.min = Some((min[at] as f64, true));
                    acc.max = Some((max[at] as f64, true));
                }
            }
            accs.push(acc);
        }
        accs
    }

    /// The rows of a block whose key is a column of strings, the group of
    /// a row being its code: `rows`, or every row. A column at a time,
    /// under the conditions of [`Self::add`].
    fn add_coded(&mut self, codes: &[u32], rows: Option<&[u32]>, args: &[Option<&Column>]) {
        match rows {
            None => codes.iter().for_each(|c| self.counts[*c as usize] += 1),
            Some(rows) => rows
                .iter()
                .for_each(|r| self.counts[codes[*r as usize] as usize] += 1),
        }
        let mut ok = self.ok;
        for (col, column) in self.cols.iter_mut().zip(args) {
            match (col, column.map(|c| &c.data)) {
                (FlatCol::Float { sum, err, min, max }, Some(ColData::Float(values))) => {
                    let mut one = |at: usize, x: f64| {
                        let (next, lost) = two_sum(sum[at], x);
                        sum[at] = next;
                        if lost != 0.0 {
                            let (kept, gone) = two_sum(err[at], lost);
                            err[at] = kept;
                            ok &= gone == 0.0;
                        }
                        min[at] = min[at].min(x);
                        max[at] = max[at].max(x);
                    };
                    match rows {
                        None => codes
                            .iter()
                            .zip(values.iter())
                            .for_each(|(c, x)| one(*c as usize, *x)),
                        Some(rows) => rows
                            .iter()
                            .for_each(|r| one(codes[*r as usize] as usize, values[*r as usize])),
                    }
                }
                (FlatCol::Int { sum, min, max }, Some(ColData::Int(values))) => {
                    let mut one = |at: usize, x: i64| {
                        let (next, over) = sum[at].overflowing_add(x);
                        sum[at] = next;
                        ok &= !over & (x.unsigned_abs() <= 1 << 53);
                        min[at] = min[at].min(x);
                        max[at] = max[at].max(x);
                    };
                    match rows {
                        None => codes
                            .iter()
                            .zip(values.iter())
                            .for_each(|(c, x)| one(*c as usize, *x)),
                        Some(rows) => rows
                            .iter()
                            .for_each(|r| one(codes[*r as usize] as usize, values[*r as usize])),
                    }
                }
                (FlatCol::None, _) => {}
                _ => ok = false,
            }
        }
        self.ok = ok;
    }
}

/// `a + b` as a float, and what that addition rounded away: the two add up
/// to `a + b` exactly (unless it overflows).
#[inline]
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let sum = a + b;
    let virtual_b = sum - a;
    (sum, (a - (sum - virtual_b)) + (b - virtual_b))
}

/// A number below `of` from the bits of a key.
#[inline]
fn owner(bits: u64, of: u64) -> u64 {
    let x = bits.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    ((x >> 32) * of) >> 32
}

impl Table {
    fn group(&mut self, key: Vec<Atom>, accs: usize) -> u32 {
        if let Some(found) = self.index.get(&key) {
            return *found;
        }
        let id = self.groups.len() as u32;
        self.groups.push(Group {
            key: key.clone(),
            count: 0,
            accs: (0..accs).map(|_| Acc::default()).collect(),
        });
        self.index.insert(key, id);
        id
    }

    /// The group of `key`, found where the blocks put the groups of that
    /// kind of key: a single number in `numbers`, anything else in
    /// `index`. A key is then one group in the table, whichever way its
    /// rows came.
    fn slot(&mut self, key: Vec<Atom>, accs: usize) -> u32 {
        let bits = match key.as_slice() {
            [Atom::Int(value)] => *value as u64,
            [Atom::Float(bits)] => *bits,
            _ => return self.group(key, accs),
        };
        if let Some(found) = self.numbers.get(&bits) {
            return *found;
        }
        let id = self.groups.len() as u32;
        self.groups.push(Group {
            key,
            count: 0,
            accs: (0..accs).map(|_| Acc::default()).collect(),
        });
        self.numbers.insert(bits, id);
        id
    }

    fn merge(&mut self, other: Table) {
        for mut group in other.groups {
            let id = self.slot(group.key, group.accs.len()) as usize;
            let mine = &mut self.groups[id];
            mine.count += group.count;
            for (acc, theirs) in mine.accs.iter_mut().zip(&mut group.accs) {
                // Settled, a sum goes in at the cost of an addition.
                theirs.sum.settle();
                theirs.squares.settle();
                acc.merge(theirs);
            }
        }
    }
}

/// A number as `(x, is an integer)` → the JSON number a document holds.
fn number((x, int): (f64, bool)) -> Option<serde_json::Number> {
    if int {
        Some(serde_json::Number::from(x as i64))
    } else {
        serde_json::Number::from_f64(x)
    }
}

/// The rows of `column` the comparison keeps, among `rows` (`None`: all of
/// them), ascending. To SDBQL null is below every other value and equal
/// to none: a null passes `!=`, `<` and `<=`, and nothing else. `None`
/// when the arrays cannot answer as the ordinary comparison would: another
/// kind of value on one side.
fn select(column: &Column, cond: &Cond, rows: Option<Vec<u32>>, len: usize) -> Option<Vec<u32>> {
    use std::cmp::Ordering;
    use BinaryOperator::*;
    let wanted = wanted_of(&cond.op)?;
    let null_passes = matches!(cond.op, NotEqual | LessThan | LessThanOrEqual);
    let nulls = column.nulls.as_deref();
    let keep = |stands: &dyn Fn(usize) -> Ordering| -> Vec<u32> {
        let test = |row: &u32| {
            let row = *row as usize;
            match nulls {
                Some(nulls) if nulls[row] => null_passes,
                _ => wanted(stands(row)),
            }
        };
        match &rows {
            Some(rows) => rows.iter().copied().filter(test).collect(),
            None => (0..len as u32).filter(test).collect(),
        }
    };
    // Against null: a null is equal to it, anything else above it.
    if cond.value.is_null() {
        let test = |row: &u32| {
            wanted(match nulls {
                Some(nulls) if nulls[*row as usize] => Ordering::Equal,
                _ => Ordering::Greater,
            })
        };
        return Some(match &rows {
            Some(rows) => rows.iter().copied().filter(test).collect(),
            None => (0..len as u32).filter(test).collect(),
        });
    }
    match (&column.data, &cond.value) {
        (ColData::Str { dict, codes }, Value::String(constant)) => {
            // The dictionary is sorted: the strings below the constant are
            // the codes below `at`, and the constant itself, if the block
            // holds it, is the code `at`.
            let at = dict.partition_point(|held| held.as_str() < constant.as_str());
            let held = dict.get(at).is_some_and(|held| held == constant);
            // The first comparison, an equality: the rows are listed, not
            // looked for.
            if cond.op == Equal && rows.is_none() && nulls.is_none() {
                return Some(match held {
                    true => column.rows_of(at as u32).to_vec(),
                    false => Vec::new(),
                });
            }
            Some(keep(&|row| match (codes[row] as usize).cmp(&at) {
                Ordering::Equal if !held => Ordering::Greater,
                other => other,
            }))
        }
        (ColData::Bool(values), Value::Bool(constant)) => {
            Some(keep(&|row| values[row].cmp(constant)))
        }
        (ColData::Int(values), Value::Number(n)) => {
            // Against an integer, as integers; against a float, as the
            // floats both are compared as.
            if let Some(c) = n.as_i64() {
                Some(keep(&|row| values[row].cmp(&c)))
            } else {
                let c = n.as_f64()?;
                let exact = |v: i64| v.abs() <= 1 << 53;
                if !values.iter().copied().all(exact) {
                    return None;
                }
                Some(keep(&|row| {
                    (values[row] as f64)
                        .partial_cmp(&c)
                        .unwrap_or(Ordering::Equal)
                }))
            }
        }
        (ColData::Float(values), Value::Number(n)) => {
            let c = match n.as_i64() {
                Some(i) if i.abs() <= 1 << 53 => i as f64,
                Some(_) => return None,
                None => n.as_f64()?,
            };
            Some(keep(&|row| {
                values[row].partial_cmp(&c).unwrap_or(Ordering::Equal)
            }))
        }
        // Values of several kinds, each as JSON text: compared as the
        // values they are, in SDBQL's order of kinds.
        (ColData::Json(values), constant) => Some(keep(&|row| {
            let value = serde_json::from_str::<Value>(&values[row]).unwrap_or(Value::Null);
            compare_values(&value, constant)
        })),
        _ => None,
    }
}

/// What a comparison keeps of how its left side stands to its right.
fn wanted_of(op: &BinaryOperator) -> Option<fn(std::cmp::Ordering) -> bool> {
    use std::cmp::Ordering;
    use BinaryOperator::*;
    Some(match op {
        Equal => |o| o == Ordering::Equal,
        NotEqual => |o| o != Ordering::Equal,
        LessThan => |o| o == Ordering::Less,
        LessThanOrEqual => |o| o != Ordering::Greater,
        GreaterThan => |o| o == Ordering::Greater,
        GreaterThanOrEqual => |o| o != Ordering::Less,
        _ => return None,
    })
}

/// The rows where column `a` stands to column `b` as `op` asks, among
/// `rows` (`None`: all of them), ascending. Two columns of one kind are
/// compared in their arrays; any others value by value, as SDBQL orders
/// values of different kinds. A null is below every value and equal to
/// another null.
fn select_cols(
    a: &Column,
    b: &Column,
    op: &BinaryOperator,
    rows: Option<Vec<u32>>,
    len: usize,
) -> Option<Vec<u32>> {
    use std::cmp::Ordering;
    let wanted = wanted_of(op)?;
    let null = |column: &Column, row: usize| column.nulls.as_ref().is_some_and(|n| n[row]);
    let stands = |row: usize| -> Ordering {
        match (null(a, row), null(b, row)) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            (false, false) => {}
        }
        match (&a.data, &b.data) {
            (ColData::Int(x), ColData::Int(y)) => x[row].cmp(&y[row]),
            (ColData::Float(x), ColData::Float(y)) => {
                x[row].partial_cmp(&y[row]).unwrap_or(Ordering::Equal)
            }
            (ColData::Bool(x), ColData::Bool(y)) => x[row].cmp(&y[row]),
            (ColData::Str { dict: d, codes: x }, ColData::Str { dict: e, codes: y }) => {
                d[x[row] as usize].cmp(&e[y[row] as usize])
            }
            _ => compare_values(&a.value(row), &b.value(row)),
        }
    };
    let test = |row: &u32| wanted(stands(*row as usize));
    Some(match rows {
        Some(rows) => rows.into_iter().filter(test).collect(),
        None => (0..len as u32).filter(test).collect(),
    })
}

/// The column of `derived` for the rows of `block`: its expression
/// evaluated for each row from the columns it reads. `None` when the
/// block lacks one of them, when a row's evaluation fails (the rows then
/// report it, as any query would) or when the values are not of one kind
/// an array holds.
fn derive(ev: &Eval<'_, '_>, derived: &Derived<'_>, block: &ColBlock) -> Option<Column> {
    let columns: Vec<&Column> = derived
        .columns
        .iter()
        .map(|name| block.column(name))
        .collect::<Option<_>>()?;
    if let Some(column) = arithmetic(derived.expr, ev.var, block, derived.numbers) {
        return Some(column);
    }
    let len = block.len();
    let mut ctx = ev.outer.clone();
    let mut value_of = |row: serde_json::Map<String, Value>| -> Option<Value> {
        bind_in_place(&mut ctx, ev.var, Value::Object(row));
        ev.ex.evaluate_expr_with_context(derived.expr, &ctx).ok()
    };
    // One column of strings: once per string the block holds, and once
    // for null.
    if let (
        [name],
        [Column {
            data: ColData::Str { dict, codes },
            nulls,
            ..
        }],
    ) = (derived.columns.as_slice(), columns.as_slice())
    {
        let one = |value: Value| serde_json::Map::from_iter([(name.clone(), value)]);
        let mut per = Vec::with_capacity(dict.len());
        for held in dict {
            per.push(value_of(one(Value::String(held.clone())))?);
        }
        let for_null = match nulls {
            Some(_) => Some(value_of(one(Value::Null))?),
            None => None,
        };
        let values = (0..len).map(|row| match (nulls, &for_null) {
            (Some(nulls), Some(value)) if nulls[row] => Some(value),
            _ => Some(&per[codes[row] as usize]),
        });
        return Column::of_values(values, derived.numbers);
    }
    let mut values = Vec::with_capacity(len);
    for row in 0..len {
        let object = derived
            .columns
            .iter()
            .zip(&columns)
            .map(|(name, column)| (name.clone(), column.value(row)))
            .collect();
        values.push(value_of(object)?);
    }
    Column::of_values(values.iter().map(Some), derived.numbers)
}

/// A number as SDBQL's arithmetic holds it.
#[derive(Clone, Copy)]
enum Num {
    Int(i64),
    Float(f64),
}

/// An arithmetic expression over columns of numbers and numeric literals,
/// ready to be computed row by row in the arrays.
enum Sum<'a> {
    Ints(&'a [i64], Option<&'a [bool]>),
    Floats(&'a [f64], Option<&'a [bool]>),
    Const(Num),
    Op(Box<Sum<'a>>, BinaryOperator, Box<Sum<'a>>),
}

impl<'a> Sum<'a> {
    /// `expr` when it is made of `+ - * / %`, columns of numbers of the
    /// block and numeric literals, and nothing else.
    fn of(expr: &Expression, var: &str, block: &'a ColBlock) -> Option<Self> {
        match expr {
            Expression::FieldAccess(base, field) if matches!(base.as_ref(), Expression::Variable(v) if v == var) =>
            {
                let column = block.column(field)?;
                let nulls = column.nulls.as_deref();
                match &column.data {
                    ColData::Int(values) => Some(Sum::Ints(values, nulls)),
                    ColData::Float(values) => Some(Sum::Floats(values, nulls)),
                    _ => None,
                }
            }
            Expression::Literal(Value::Number(n)) => Some(Sum::Const(match n.as_i64() {
                Some(value) => Num::Int(value),
                None => Num::Float(n.as_f64()?),
            })),
            Expression::BinaryOp { left, op, right }
                if matches!(
                    op,
                    BinaryOperator::Add
                        | BinaryOperator::Subtract
                        | BinaryOperator::Multiply
                        | BinaryOperator::Divide
                        | BinaryOperator::Modulus
                ) =>
            {
                Some(Sum::Op(
                    Box::new(Sum::of(left, var, block)?),
                    op.clone(),
                    Box::new(Sum::of(right, var, block)?),
                ))
            }
            _ => None,
        }
    }

    /// The value for `row`; `None` is null. As `helpers::arithmetic`
    /// computes it, which the tests hold this to: a null operand counts
    /// as the integer 0, two integers give an integer unless that
    /// overflows or a division leaves a remainder, anything else a float,
    /// and a division by zero or a result that is not finite is null.
    fn at(&self, row: usize) -> Option<Num> {
        match self {
            Sum::Ints(values, nulls) => {
                (!nulls.is_some_and(|nulls| nulls[row])).then(|| Num::Int(values[row]))
            }
            Sum::Floats(values, nulls) => {
                (!nulls.is_some_and(|nulls| nulls[row])).then(|| Num::Float(values[row]))
            }
            Sum::Const(value) => Some(*value),
            Sum::Op(left, op, right) => {
                let a = left.at(row).unwrap_or(Num::Int(0));
                let b = right.at(row).unwrap_or(Num::Int(0));
                if let (Num::Int(x), Num::Int(y)) = (a, b) {
                    let exact = match op {
                        BinaryOperator::Add => x.checked_add(y),
                        BinaryOperator::Subtract => x.checked_sub(y),
                        BinaryOperator::Multiply => x.checked_mul(y),
                        BinaryOperator::Divide => {
                            if y == 0 {
                                return None;
                            }
                            match x.checked_rem(y) {
                                Some(0) => x.checked_div(y),
                                _ => None,
                            }
                        }
                        _ => {
                            if y == 0 {
                                return None;
                            }
                            Some(x.checked_rem(y).unwrap_or(0))
                        }
                    };
                    if let Some(value) = exact {
                        return Some(Num::Int(value));
                    }
                }
                let float = |n: Num| match n {
                    Num::Int(i) => i as f64,
                    Num::Float(f) => f,
                };
                let (x, y) = (float(a), float(b));
                let value = match op {
                    BinaryOperator::Add => x + y,
                    BinaryOperator::Subtract => x - y,
                    BinaryOperator::Multiply => x * y,
                    BinaryOperator::Divide => {
                        if y == 0.0 {
                            return None;
                        }
                        x / y
                    }
                    _ => {
                        if y == 0.0 {
                            return None;
                        }
                        x % y
                    }
                };
                value.is_finite().then_some(Num::Float(value))
            }
        }
    }
}

/// Arithmetic over columns of numbers, computed in the arrays: a column
/// of integers, of floats, or (with `numbers`) of both. `None` for any
/// other expression, and when the values are of both kinds where a column
/// cannot be: the evaluator then says what each row is.
fn arithmetic(expr: &Expression, var: &str, block: &ColBlock, numbers: bool) -> Option<Column> {
    let sum = Sum::of(expr, var, block)?;
    if !matches!(sum, Sum::Op(..)) {
        return None;
    }
    let len = block.len();
    let mut values: Vec<f64> = Vec::with_capacity(len);
    let mut whole: Vec<i64> = Vec::with_capacity(len);
    let mut ints: Vec<bool> = Vec::with_capacity(len);
    let mut nulls: Vec<bool> = Vec::with_capacity(len);
    let (mut any_int, mut any_float, mut any_null, mut wide) = (false, false, false, false);
    for row in 0..len {
        let (value, int, is_int, null) = match sum.at(row) {
            Some(Num::Int(i)) => (i as f64, i, true, false),
            Some(Num::Float(f)) => (f, 0, false, false),
            None => (0.0, 0, true, true),
        };
        any_null |= null;
        any_int |= is_int && !null;
        any_float |= !is_int;
        // An integer a float does not hold cannot sit beside floats.
        wide |= is_int && int.unsigned_abs() > 1 << 53;
        values.push(value);
        whole.push(int);
        ints.push(is_int);
        nulls.push(null);
    }
    let nulls = any_null.then_some(nulls);
    match (any_int, any_float) {
        (_, false) => Some(Column::ints(whole, nulls)),
        (false, true) => Some(Column::floats(values, nulls)),
        (true, true) if numbers && !wide => Some(Column::mixed(values, ints, nulls)),
        (true, true) => None,
    }
}

/// `expr` as a computed column of the plan, when it is made of the row's
/// columns, constants and functions that give the same value for the same
/// arguments: its place in `made`.
fn derived<'q>(
    expr: &'q Expression,
    var: &str,
    known: &[String],
    numbers: bool,
    made: &mut Vec<Derived<'q>>,
) -> Option<usize> {
    let mut columns = Vec::new();
    reads(expr, var, known, &mut columns)?;
    columns.sort();
    columns.dedup();
    if columns.is_empty() {
        return None;
    }
    made.push(Derived {
        expr,
        columns,
        numbers,
    });
    Some(made.len() - 1)
}

/// The columns of `var` that `expr` reads, when that is all it reads of
/// the row and nothing in it depends on anything but its arguments.
fn reads(expr: &Expression, var: &str, known: &[String], out: &mut Vec<String>) -> Option<()> {
    use Expression as E;
    match expr {
        E::Literal(_) | E::BindVariable(_) => Some(()),
        // An outer value: the same for every row.
        E::Variable(name) => (name != var).then_some(()),
        E::FieldAccess(base, field) => match base.as_ref() {
            E::Variable(name) if name == var && known.iter().any(|k| k == field) => {
                out.push(field.clone());
                Some(())
            }
            _ => None,
        },
        E::BinaryOp { left, right, .. } => {
            reads(left, var, known, out)?;
            reads(right, var, known, out)
        }
        E::UnaryOp { operand, .. } => reads(operand, var, known, out),
        E::FunctionCall { name, args } if is_pure_function(name, args.len()) => {
            args.iter().try_for_each(|arg| reads(arg, var, known, out))
        }
        E::Ternary {
            condition,
            true_expr,
            false_expr,
        } => {
            reads(condition, var, known, out)?;
            reads(true_expr, var, known, out)?;
            reads(false_expr, var, known, out)
        }
        _ => None,
    }
}

/// `TIME_BUCKET(column, "<interval>")` for an interval of a fixed length:
/// the column, the length in milliseconds and the instant buckets are
/// counted from (a Monday for weeks). Months have no fixed length, and
/// options are the builtin's own business.
fn bucket(
    expr: &Expression,
    column: &dyn Fn(&Expression) -> Option<String>,
) -> Option<(String, i64, i64)> {
    let Expression::FunctionCall { name, args } = expr else {
        return None;
    };
    if !name.eq_ignore_ascii_case("TIME_BUCKET") || args.len() != 2 {
        return None;
    }
    let Expression::Literal(Value::String(interval)) = &args[1] else {
        return None;
    };
    let unit = interval.trim_end().to_ascii_lowercase();
    let weeks = unit.ends_with('w') || unit.ends_with("week") || unit.ends_with("weeks");
    match parse_interval(interval).ok()? {
        Interval::Fixed(step) if step > 0 => Some((
            column(&args[0])?,
            step,
            if weeks { WEEK_ORIGIN_MS } else { 0 },
        )),
        _ => None,
    }
}

/// A block as a fold reads it: with the plan's computed columns, each
/// made the first time it is asked for.
struct Read<'a, 'q, 'e> {
    plan: &'a ColPlan<'q>,
    ev: &'a Eval<'a, 'e>,
    cells: &'a Cells,
    block: &'a ColBlock,
}

impl<'a> Read<'a, '_, '_> {
    /// The computed column `at` of the plan, for this block.
    fn made(&self, at: usize) -> Option<&'a Column> {
        self.cells[at]
            .get_or_init(|| derive(self.ev, &self.plan.made[at], self.block))
            .as_ref()
    }
}

/// The rows of `all` that are not in `out`; all ascending.
fn minus(all: &[u32], out: &[u32]) -> Vec<u32> {
    let mut at = 0;
    all.iter()
        .copied()
        .filter(|row| {
            while at < out.len() && out[at] < *row {
                at += 1;
            }
            at >= out.len() || out[at] != *row
        })
        .collect()
}

/// The rows `pred` keeps among `rows` (`None`: every row of the block),
/// ascending; the outer `None` when the arrays cannot answer for it.
fn kept(
    read: &Read<'_, '_, '_>,
    pred: &Pred,
    rows: Option<Vec<u32>>,
    len: usize,
) -> Option<Option<Vec<u32>>> {
    if rows.as_ref().is_some_and(Vec::is_empty) {
        return Some(rows);
    }
    let block = read.block;
    match pred {
        Pred::Cmp(cond) => {
            let column = block.column(&cond.column)?;
            // The smallest and largest value of the block may say it all.
            if column.nulls.is_none() {
                match settled(column, cond) {
                    Some(true) => return Some(rows),
                    Some(false) => return Some(Some(Vec::new())),
                    None => {}
                }
            }
            Some(Some(select(column, cond, rows, len)?))
        }
        Pred::Cols { left, op, right } => {
            let (a, b) = (block.column(left)?, block.column(right)?);
            Some(Some(select_cols(a, b, op, rows, len)?))
        }
        Pred::Made(at) => {
            // A condition is true or false; anything else is the rows'
            // to turn into one.
            let column = read.made(*at)?;
            let ColData::Bool(values) = &column.data else {
                return None;
            };
            let holds = |row: &u32| {
                let row = *row as usize;
                values[row] && !column.nulls.as_ref().is_some_and(|nulls| nulls[row])
            };
            Some(Some(match rows {
                Some(rows) => rows.into_iter().filter(holds).collect(),
                None => (0..len as u32).filter(holds).collect(),
            }))
        }
        Pred::Not(inner) => {
            let all: Vec<u32> = match &rows {
                Some(rows) => rows.clone(),
                None => (0..len as u32).collect(),
            };
            Some(Some(match kept(read, inner, rows, len)? {
                None => Vec::new(),
                Some(found) => minus(&all, &found),
            }))
        }
        Pred::All(all) => {
            let mut rows = rows;
            for pred in all {
                rows = kept(read, pred, rows, len)?;
            }
            Some(rows)
        }
        Pred::Any(any) => {
            let mut found: Vec<u32> = Vec::new();
            for pred in any {
                match kept(read, pred, rows.clone(), len)? {
                    // Every row there was.
                    None => return Some(None),
                    Some(more) => found = either(&found, &more),
                }
            }
            Some(Some(found))
        }
    }
}

/// The rows of `a` and those of `b`, each once; all three ascending.
fn either(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut rows = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                rows.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                rows.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                rows.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    rows.extend_from_slice(&a[i..]);
    rows.extend_from_slice(&b[j..]);
    rows
}

/// The value of a key column at `row`.
fn atom(column: &Column, row: usize) -> Atom {
    if column.nulls.as_ref().is_some_and(|nulls| nulls[row]) {
        return Atom::Null;
    }
    match &column.data {
        ColData::Int(values) => Atom::Int(values[row]),
        ColData::Float(values) => Atom::Float(values[row].to_bits()),
        ColData::Str { dict, codes } => Atom::Str(Arc::from(dict[codes[row] as usize].as_str())),
        ColData::Bool(values) => Atom::Bool(values[row]),
        ColData::Json(_) => Atom::Bool(false),
        ColData::Mixed { values, ints } if ints[row] => Atom::Int(values[row] as i64),
        ColData::Mixed { values, .. } => Atom::Float(values[row].to_bits()),
    }
}

/// A number that tells a row's key value from the others of the block
/// (nulls aside): a string's code, a number's bits.
fn local(column: &Column, row: usize) -> u64 {
    match &column.data {
        ColData::Int(values) => values[row] as u64,
        ColData::Float(values) => values[row].to_bits(),
        ColData::Str { codes, .. } => u64::from(codes[row]),
        ColData::Bool(values) => u64::from(values[row]),
        ColData::Json(_) => 0,
        ColData::Mixed { values, .. } => values[row].to_bits(),
    }
}

/// Fold rows a row at a time: those whose keys or arguments hold nulls,
/// a null key being a group of its own and a null argument passed over
/// (`Acc::missing`), and those of a query that counts distinct values,
/// which each group keeps (`Acc::distinct`). The groups of a single
/// numeric key are found where the blocks without nulls put them
/// (`Table::numbers`), so that a key is one group in the table.
fn fold_by_row(
    table: &mut Table,
    share: Share,
    keys: &[&Column],
    args: &[Option<&Column>],
    plan: &ColPlan<'_>,
    rows: Option<&[u32]>,
    len: usize,
) -> Option<()> {
    let (kinds, shared) = (plan.kinds.as_slice(), plan.shared.as_slice());
    let (mine, of) = match share {
        Share::All => (0, 1),
        Share::Hashed { mine, of } => (mine, of),
        // Groups by position hold no null.
        Share::Ranged { .. } => return None,
    };
    if keys.len() > 63 {
        return None;
    }
    let accs = args.len();
    let is_null = |column: &Column, row: usize| column.nulls.as_ref().is_some_and(|n| n[row]);
    let by_number = matches!(
        keys,
        [Column {
            data: ColData::Int(_) | ColData::Float(_),
            ..
        }]
    );
    // One key of strings, none of them null: the groups by the block's
    // codes.
    let mut by_code: Vec<u32> = match keys {
        [Column {
            data: ColData::Str { dict, .. },
            nulls: None,
            ..
        }] if of == 1 => vec![u32::MAX; dict.len()],
        _ => Vec::new(),
    };
    // For a sketch of a column of strings: the hash of each string of the
    // block, once.
    let hashes: Vec<Option<Vec<u64>>> = args
        .iter()
        .zip(kinds)
        .map(|(column, kind)| match column.map(|c| &c.data) {
            Some(ColData::Str { dict, .. }) if *kind == AggKind::ApproxDistinct => {
                Some(dict.iter().map(|s| hash_str(s)).collect())
            }
            _ => None,
        })
        .collect();
    let mut seen: HashMap<Vec<u64>, u32, foldhash::fast::RandomState> = HashMap::default();
    let mut scratch: Vec<u64> = Vec::with_capacity(keys.len() + 1);
    let mut each = |row: usize| {
        let group = if !by_code.is_empty() {
            let code = local(keys[0], row) as usize;
            if by_code[code] == u32::MAX {
                by_code[code] = table.group(vec![atom(keys[0], row)], accs);
            }
            by_code[code]
        } else if by_number {
            let key = keys[0];
            if is_null(key, row) {
                // The null key is the first thread's.
                if mine != 0 {
                    return;
                }
                table.group(vec![Atom::Null], accs)
            } else {
                let bits = local(key, row);
                if of > 1 && owner(bits, of) != mine {
                    return;
                }
                match table.numbers.get(&bits) {
                    Some(group) => *group,
                    None => {
                        let group = table.groups.len() as u32;
                        table.groups.push(Group {
                            key: vec![atom(key, row)],
                            count: 0,
                            accs: (0..accs).map(|_| Acc::default()).collect(),
                        });
                        table.numbers.insert(bits, group);
                        group
                    }
                }
            }
        } else {
            // The keys' numbers, then which of them are null.
            scratch.clear();
            let mut nulls = 0u64;
            for (n, key) in keys.iter().enumerate() {
                if is_null(key, row) {
                    nulls |= 1 << n;
                    scratch.push(0);
                } else {
                    scratch.push(local(key, row));
                }
            }
            scratch.push(nulls);
            match seen.get(&scratch) {
                Some(group) => *group,
                None => {
                    let key = keys.iter().map(|key| atom(key, row)).collect();
                    let group = table.group(key, accs);
                    seen.insert(scratch.clone(), group);
                    group
                }
            }
        };
        let group = &mut table.groups[group as usize];
        group.count += 1;
        for (at, (((acc, column), kind), hashes)) in group
            .accs
            .iter_mut()
            .zip(args)
            .zip(kinds)
            .zip(&hashes)
            .enumerate()
        {
            match column {
                // Null is one of the values a sketch holds too: each value
                // by the hash the function gives it.
                Some(column) if *kind == AggKind::ApproxDistinct => {
                    acc.sketch.add(match (&column.data, hashes) {
                        _ if is_null(column, row) => hash_value(&Value::Null),
                        (ColData::Str { codes, .. }, Some(hashes)) => hashes[codes[row] as usize],
                        (ColData::Int(values), _) => hash_f64(values[row] as f64),
                        (ColData::Float(values) | ColData::Mixed { values, .. }, _) => {
                            hash_f64(values[row])
                        }
                        (ColData::Bool(values), _) => hash_value(&Value::Bool(values[row])),
                        _ => return,
                    });
                }
                // Null is one of the values a distinct count holds.
                Some(column) if *kind == AggKind::CountDistinct => match &column.data {
                    ColData::Int(values) if !is_null(column, row) => acc.ints.push(values[row]),
                    // (`-0.0` is `0.0` to the query.)
                    ColData::Float(values) if !is_null(column, row) => {
                        acc.floats.push((values[row] + 0.0).to_bits())
                    }
                    data => {
                        acc.mixed |= matches!(data, ColData::Mixed { .. });
                        let value = atom(column, row);
                        if !acc.distinct.contains(&value) {
                            acc.distinct.insert(value);
                        }
                    }
                },
                Some(column) if is_null(column, row) => acc.missing += 1,
                // The variance family: the number as a float, and its
                // square.
                Some(column) if AggregateAccumulator::takes_moments(*kind) => {
                    let x = match &column.data {
                        ColData::Int(values) => values[row] as f64,
                        ColData::Float(values) | ColData::Mixed { values, .. } => values[row],
                        _ => return,
                    };
                    acc.sum.add(x);
                    acc.squares.add_square(x);
                }
                // Bits: an integer, or a float that is one, as the
                // aggregate reads a value.
                Some(column) if AggregateAccumulator::takes_bits(*kind) => {
                    let n = match &column.data {
                        ColData::Int(values) => Some(values[row]),
                        ColData::Float(values) | ColData::Mixed { values, .. } => {
                            serde_json::Number::from_f64(values[row])
                                .and_then(|n| as_int(&Value::Number(n)))
                        }
                        _ => return,
                    };
                    match n {
                        Some(n) => {
                            acc.bits = Some(AggregateAccumulator::bits(*kind, acc.bits, n));
                            acc.bit_kind = Some(*kind);
                        }
                        None => acc.not_bits = true,
                    }
                }
                // A median, a percentile: the numbers, unless another of
                // the same argument keeps them.
                Some(_) if shared[at] != at => {}
                Some(column) if AggregateAccumulator::takes_numbers(*kind) => {
                    acc.values.push(match &column.data {
                        ColData::Int(values) => values[row] as f64,
                        ColData::Float(values) | ColData::Mixed { values, .. } => values[row],
                        _ => return,
                    });
                }
                Some(column) => match &column.data {
                    ColData::Int(values) => acc.int(values[row]),
                    ColData::Float(values) => acc.float(values[row]),
                    ColData::Mixed { values, ints } if ints[row] => acc.int(values[row] as i64),
                    ColData::Mixed { values, .. } => acc.float(values[row]),
                    _ => {}
                },
                None => {}
            }
        }
    };
    match rows {
        Some(rows) => rows.iter().for_each(|row| each(*row as usize)),
        None => (0..len).for_each(&mut each),
    }
    Some(())
}

/// Whether every row of `column` passes `cond`, or none does, when the
/// extremes of a column of integers compared with an integer tell.
fn settled(column: &Column, cond: &Cond) -> Option<bool> {
    use BinaryOperator::*;
    let c = match &cond.value {
        Value::Number(n) => n.as_i64()?,
        _ => return None,
    };
    let (lo, hi) = column.int_range()?;
    let (all, none) = match cond.op {
        Equal => (lo == c && hi == c, c < lo || c > hi),
        LessThan => (hi < c, lo >= c),
        LessThanOrEqual => (hi <= c, lo > c),
        GreaterThan => (lo > c, hi <= c),
        GreaterThanOrEqual => (lo >= c, hi < c),
        _ => return None,
    };
    match (all, none) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    }
}

/// Fold the rows of `block` the plan keeps into `table`; `None` when the
/// block has to be read as objects.
fn fold_block(read: &Read<'_, '_, '_>, table: &mut Table, share: Share) -> Option<()> {
    let (plan, block) = (read.plan, read.block);
    if let Share::Ranged { span, .. } = share {
        if table.dense.is_empty() {
            table.dense = vec![u32::MAX; span as usize + 1];
        }
    }
    let usable = |name: &str| {
        block
            .column(name)
            .filter(|c| !matches!(c.data, ColData::Json(_)))
    };
    let len = block.len();
    let mut rows: Option<Vec<u32>> = None;
    for pred in &plan.preds {
        rows = kept(read, pred, rows, len)?;
    }
    // A key that is a bucket of time over a column of integers: the
    // instants its buckets start at, as a column of its own. One that
    // overflows is the builtin's to report, from the rows.
    let mut buckets: Vec<Option<Column>> = Vec::with_capacity(plan.keys.len());
    for key in &plan.keys {
        buckets.push(match key {
            Key::Bucket {
                column,
                step,
                origin,
                ..
            } => match usable(column).map(|source| (&source.data, source)) {
                Some((ColData::Int(times), source)) => {
                    let starts = times
                        .iter()
                        .map(|time| {
                            time.checked_sub(*origin)?
                                .div_euclid(*step)
                                .checked_mul(*step)?
                                .checked_add(*origin)
                        })
                        .collect::<Option<Vec<i64>>>()?;
                    Some(Column::ints(starts, source.nulls.clone()))
                }
                _ => None,
            },
            _ => None,
        });
    }
    let keys: Vec<&Column> = plan
        .keys
        .iter()
        .zip(&buckets)
        .map(|(key, bucket)| match (key, bucket) {
            (_, Some(column)) => Some(column),
            (Key::Column(name), None) => usable(name),
            (Key::Bucket { made, .. } | Key::Made(made), None) => read.made(*made),
        })
        .collect::<Option<_>>()?;
    // Numbers for the aggregates that add or compare; a string or a
    // boolean can only be counted.
    let mut args: Vec<Option<&Column>> = Vec::with_capacity(plan.args.len());
    for (arg, kind) in plan.args.iter().zip(&plan.kinds) {
        let column = match arg {
            None => None,
            Some(Source::Column(name)) => Some(usable(name)?),
            Some(Source::Made(made)) => Some(read.made(*made)?),
        };
        let numeric = column.is_none_or(|c| {
            matches!(
                c.data,
                ColData::Int(_) | ColData::Float(_) | ColData::Mixed { .. }
            )
        });
        let counted = matches!(
            kind,
            AggKind::CountNonNull
                | AggKind::CountRows
                | AggKind::CountDistinct
                | AggKind::ApproxDistinct
        );
        if !numeric && !counted {
            return None;
        }
        args.push(column);
    }
    let accs = args.len();
    // A null among the keys or the arguments, distinct values to keep or
    // squares to add: a row at a time. What follows reads whole arrays.
    let by_row =
        plan.kinds
            .iter()
            .any(|kind| AggregateAccumulator::takes_its_own(*kind))
            || keys.iter().chain(args.iter().flatten()).any(|column| {
                column.nulls.is_some() || matches!(column.data, ColData::Mixed { .. })
            });
    if by_row {
        return fold_by_row(table, share, &keys, &args, plan, rows.as_deref(), len);
    }

    // The group of each combination of key values met in this block.
    let mut seen: HashMap<Vec<u64>, u32, foldhash::fast::RandomState> = HashMap::default();
    let mut by_code: Vec<u32> = match keys.as_slice() {
        [Column {
            data: ColData::Str { dict, .. },
            ..
        }] => vec![u32::MAX; dict.len()],
        _ => Vec::new(),
    };
    let whole = (keys.is_empty() && len > 0).then(|| table.group(Vec::new(), accs));
    // No key: the rows kept go to one group, a column at a time.
    if let Some(group) = whole {
        let group = &mut table.groups[group as usize];
        group.count += rows.as_ref().map_or(len, Vec::len) as u64;
        for (acc, column) in group.accs.iter_mut().zip(&args) {
            match (column.map(|c| &c.data), &rows) {
                (Some(ColData::Float(values)), None) => acc.floats(values),
                (Some(ColData::Float(values)), Some(rows)) => {
                    let kept: Vec<f64> = rows.iter().map(|r| values[*r as usize]).collect();
                    acc.floats(&kept);
                }
                (Some(ColData::Int(values)), None) => values.iter().for_each(|v| acc.int(*v)),
                (Some(ColData::Int(values)), Some(rows)) => {
                    rows.iter().for_each(|r| acc.int(values[*r as usize]))
                }
                _ => {}
            }
        }
        return Some(());
    }
    // An integer key of small range, on its own: sums and extremes in
    // arrays indexed by the key (see `Flat`).
    if let (
        Share::Ranged {
            mine,
            scale,
            base,
            span,
            flat: true,
        },
        [Column {
            data: ColData::Int(key),
            ..
        }],
    ) = (share, keys.as_slice())
    {
        let flat = table
            .flat
            .get_or_insert_with(|| Flat::new(span as usize + 1, &args));
        let mut one = |row: usize| {
            let at = key[row].wrapping_sub(base) as u64;
            if at > span || (at * scale) >> 32 != mine {
                return;
            }
            flat.add(at as usize, row, &args);
        };
        match &rows {
            Some(rows) => rows.iter().for_each(|row| one(*row as usize)),
            None => (0..len).for_each(&mut one),
        }
        return Some(());
    }
    // One key of strings: plain sums in arrays indexed by the block's
    // codes, a column at a time, added to the groups when they are exact.
    // Otherwise the rows are read below, one by one.
    if let [Column {
        data: ColData::Str { dict, codes },
        ..
    }] = keys.as_slice()
    {
        let mut flat = Flat::new(dict.len(), &args);
        flat.add_coded(codes, rows.as_deref(), &args);
        if flat.valid() {
            for at in flat.present() {
                let key = vec![Atom::Str(Arc::from(dict[at].as_str()))];
                let id = table.group(key, accs) as usize;
                let group = &mut table.groups[id];
                group.count += flat.counts[at];
                for (acc, mut more) in group.accs.iter_mut().zip(flat.accs(at)) {
                    // Settled, a sum goes in at the cost of an addition.
                    more.sum.settle();
                    acc.merge(&more);
                }
            }
            return Some(());
        }
    }
    let mut scratch: Vec<u64> = Vec::with_capacity(keys.len());

    let mut each = |row: usize| {
        let group = match (whole, keys.as_slice()) {
            (Some(group), _) => group,
            (None, [key]) if !by_code.is_empty() => {
                let code = local(key, row) as usize;
                if by_code[code] == u32::MAX {
                    by_code[code] = table.group(vec![atom(key, row)], accs);
                }
                by_code[code]
            }
            (None, [key]) => {
                let bits = local(key, row);
                match share {
                    Share::All => {}
                    Share::Hashed { mine, of } => {
                        if owner(bits, of) != mine {
                            return;
                        }
                    }
                    Share::Ranged {
                        mine,
                        scale,
                        base,
                        span,
                        ..
                    } => {
                        // The distance from the smallest key says whose
                        // row it is, and where its group is.
                        let at = (bits as i64).wrapping_sub(base) as u64;
                        if at > span || (at * scale) >> 32 != mine {
                            return;
                        }
                        if table.dense[at as usize] == u32::MAX {
                            table.dense[at as usize] = table.groups.len() as u32;
                            table.groups.push(Group {
                                key: vec![atom(key, row)],
                                count: 0,
                                accs: (0..accs).map(|_| Acc::default()).collect(),
                            });
                        }
                        let group = &mut table.groups[table.dense[at as usize] as usize];
                        group.count += 1;
                        for (acc, column) in group.accs.iter_mut().zip(&args) {
                            match column.map(|c| &c.data) {
                                Some(ColData::Int(values)) => acc.int(values[row]),
                                Some(ColData::Float(values)) => acc.float(values[row]),
                                _ => {}
                            }
                        }
                        return;
                    }
                }
                match table.numbers.get(&bits) {
                    Some(group) => *group,
                    None => {
                        let group = table.groups.len() as u32;
                        table.groups.push(Group {
                            key: vec![atom(key, row)],
                            count: 0,
                            accs: (0..accs).map(|_| Acc::default()).collect(),
                        });
                        table.numbers.insert(bits, group);
                        group
                    }
                }
            }
            (None, keys) => {
                scratch.clear();
                scratch.extend(keys.iter().map(|key| local(key, row)));
                match seen.get(&scratch) {
                    Some(group) => *group,
                    None => {
                        let key = keys.iter().map(|key| atom(key, row)).collect();
                        let group = table.group(key, accs);
                        seen.insert(scratch.clone(), group);
                        group
                    }
                }
            }
        };
        let group = &mut table.groups[group as usize];
        group.count += 1;
        for (acc, column) in group.accs.iter_mut().zip(&args) {
            match column.map(|c| &c.data) {
                Some(ColData::Int(values)) => acc.int(values[row]),
                Some(ColData::Float(values)) => acc.float(values[row]),
                _ => {}
            }
        }
    };
    match &rows {
        Some(rows) => rows.iter().for_each(|row| each(*row as usize)),
        None => (0..len).for_each(&mut each),
    }
    Some(())
}

impl QueryExecutor<'_> {
    /// The plan for `found` over the columns `known`, or `None` when the
    /// query asks for more than the arrays answer.
    fn columnar_plan<'q>(
        &self,
        found: &Plan<'q>,
        known: &[String],
        outer: &Context,
    ) -> Option<ColPlan<'q>> {
        let var = found.for_clause.variable.as_str();
        let column = |expr: &Expression| -> Option<String> {
            let Expression::FieldAccess(base, field) = expr else {
                return None;
            };
            (matches!(base.as_ref(), Expression::Variable(v) if v == var)
                && known.iter().any(|k| k == field))
            .then(|| field.clone())
        };
        let mut plan = ColPlan {
            made: Vec::new(),
            preds: Vec::new(),
            keys: Vec::new(),
            args: Vec::new(),
            kinds: Vec::new(),
            shared: CollectFold::shared_numbers(found.collect),
        };
        let mut conjuncts = Vec::new();
        for stage in found.stages {
            let BodyClause::Filter(filter) = stage else {
                return None;
            };
            push_conjuncts(&filter.expression, &mut conjuncts);
        }
        for conjunct in conjuncts {
            // What the arrays compare themselves, or the condition as a
            // computed column of booleans.
            let pred = match self.columnar_pred(conjunct, var, &column, outer) {
                Some(pred) => pred,
                None => Pred::Made(derived(conjunct, var, known, false, &mut plan.made)?),
            };
            plan.preds.push(pred);
        }
        for (_, expr) in &found.collect.group_vars {
            let key = match column(expr) {
                Some(name) => Key::Column(name),
                None => {
                    let made = derived(expr, var, known, false, &mut plan.made)?;
                    match bucket(expr, &column) {
                        Some((column, step, origin)) => Key::Bucket {
                            column,
                            step,
                            origin,
                            made,
                        },
                        None => Key::Made(made),
                    }
                }
            };
            plan.keys.push(key);
        }
        for aggregate in &found.collect.aggregates {
            plan.args.push(match &aggregate.argument {
                None => None,
                Some(expr) => Some(match column(expr) {
                    Some(name) => Source::Column(name),
                    None => Source::Made(derived(expr, var, known, true, &mut plan.made)?),
                }),
            });
            let kind = AggKind::resolve(&aggregate.function, aggregate.argument.is_some())?;
            if !AggregateAccumulator::takes_slices(kind)
                && !AggregateAccumulator::takes_its_own(kind)
            {
                return None;
            }
            plan.kinds.push(kind);
        }
        Some(plan)
    }

    /// One conjunct of a FILTER as what the arrays answer: a comparison of
    /// a column of `var` with a constant, on either side, an `IN` a list
    /// of constants, or such joined by `AND` and `OR`.
    fn columnar_pred(
        &self,
        expr: &Expression,
        var: &str,
        column: &dyn Fn(&Expression) -> Option<String>,
        outer: &Context,
    ) -> Option<Pred> {
        use BinaryOperator::*;
        if let Expression::UnaryOp {
            op: UnaryOperator::Not,
            operand,
        } = expr
        {
            let inner = self.columnar_pred(operand, var, column, outer)?;
            return Some(Pred::Not(Box::new(inner)));
        }
        let Expression::BinaryOp { left, op, right } = expr else {
            return None;
        };
        // A constant the arrays compare with: of one of the kinds a column
        // holds as an array, or null.
        let constant = |expr: &Expression| -> Option<Value> {
            if expression_references_var(expr, var) {
                return None;
            }
            self.evaluate_expr_with_context(expr, outer).ok()
        };
        let comparable = |value: &Value| {
            matches!(
                value,
                Value::Number(_) | Value::String(_) | Value::Bool(_) | Value::Null
            )
        };
        match op {
            And | Or => {
                let sides = vec![
                    self.columnar_pred(left, var, column, outer)?,
                    self.columnar_pred(right, var, column, outer)?,
                ];
                Some(if *op == And {
                    Pred::All(sides)
                } else {
                    Pred::Any(sides)
                })
            }
            NotIn => {
                let within = Expression::BinaryOp {
                    left: left.clone(),
                    op: In,
                    right: right.clone(),
                };
                let within = self.columnar_pred(&within, var, column, outer)?;
                Some(Pred::Not(Box::new(within)))
            }
            In => {
                let name = column(left)?;
                let Value::Array(values) = constant(right)? else {
                    return None;
                };
                // Equal to one of them, as `==` compares.
                values
                    .into_iter()
                    .map(|value| {
                        comparable(&value).then(|| {
                            Pred::Cmp(Cond {
                                column: name.clone(),
                                op: Equal,
                                value,
                            })
                        })
                    })
                    .collect::<Option<Vec<Pred>>>()
                    .map(Pred::Any)
            }
            Equal | NotEqual | LessThan | LessThanOrEqual | GreaterThan | GreaterThanOrEqual => {
                // `column <op> constant`, or the same the other way round,
                // or two columns.
                let (name, value, op) = match (column(left), column(right)) {
                    (Some(left), Some(right)) => {
                        return Some(Pred::Cols {
                            left,
                            op: op.clone(),
                            right,
                        })
                    }
                    (Some(name), None) => (name, constant(right)?, op.clone()),
                    (None, Some(name)) => {
                        let mirrored = match op {
                            LessThan => GreaterThan,
                            LessThanOrEqual => GreaterThanOrEqual,
                            GreaterThan => LessThan,
                            GreaterThanOrEqual => LessThanOrEqual,
                            other => other.clone(),
                        };
                        (name, constant(left)?, mirrored)
                    }
                    _ => return None,
                };
                comparable(&value).then_some(Pred::Cmp(Cond {
                    column: name,
                    op,
                    value,
                }))
            }
            _ => None,
        }
    }

    /// Each group into the fold the ordinary path fills, with the state of
    /// its sums; `kept`, when given, says which groups go. `alone`: no row
    /// is folded beside the groups (no block left to the rows, no tail),
    /// so a group is whole.
    fn push_groups(
        &self,
        fold: &mut CollectFold<'_>,
        col_plan: &ColPlan,
        groups: &mut [Group],
        kept: Option<&[bool]>,
        alone: bool,
    ) -> DbResult<()> {
        if col_plan.kinds.contains(&AggKind::CountDistinct) {
            for acc in groups.iter_mut().flat_map(|group| &mut group.accs) {
                acc.ints.settle();
                acc.floats.settle();
            }
        }
        for (n, group) in groups.iter().enumerate() {
            if kept.is_some_and(|kept| !kept[n]) {
                continue;
            }
            let key: Vec<Datum<'_>> = group
                .key
                .iter()
                .map(|atom| match atom {
                    Atom::Int(v) => Datum::Num((*v).into()),
                    Atom::Float(bits) => serde_json::Number::from_f64(f64::from_bits(*bits))
                        .map_or(Datum::Null, Datum::Num),
                    Atom::Str(s) => Datum::Str(s.as_ref()),
                    Atom::Bool(b) => Datum::Bool(*b),
                    Atom::Null => Datum::Null,
                })
                .collect();
            // What a `COUNT_DISTINCT` kept: how many values, when the group
            // is whole and they are of kinds that cannot be equal to one
            // another; else the values, for the fold to tell apart.
            let distinct: Vec<Option<Result<usize, Vec<Value>>>> = col_plan
                .kinds
                .iter()
                .zip(&group.accs)
                .map(|(kind, acc)| {
                    (*kind == AggKind::CountDistinct).then(|| {
                        let (ints, floats) = (&acc.ints.values, &acc.floats.values);
                        if alone && !acc.mixed && (ints.is_empty() || floats.is_empty()) {
                            return Ok(ints.len() + floats.len() + acc.distinct.len());
                        }
                        let float = |bits: u64| {
                            serde_json::Number::from_f64(f64::from_bits(bits))
                                .map_or(Value::Null, Value::Number)
                        };
                        Err(ints
                            .iter()
                            .map(|v| Value::from(*v))
                            .chain(floats.iter().map(|bits| float(*bits)))
                            .chain(acc.distinct.iter().map(|atom| match atom {
                                Atom::Int(v) => Value::from(*v),
                                Atom::Float(bits) => float(*bits),
                                Atom::Str(s) => Value::String(s.to_string()),
                                Atom::Bool(b) => Value::Bool(*b),
                                Atom::Null => Value::Null,
                            }))
                            .collect())
                    })
                })
                .collect();
            let args: Vec<Option<Slice<'_>>> = col_plan
                .args
                .iter()
                .zip(&group.accs)
                .zip(&distinct)
                .zip(&col_plan.kinds)
                .map(|(((arg, acc), distinct), kind)| {
                    match distinct {
                        Some(Ok(count)) => return Some(Slice::Counted(*count)),
                        Some(Err(values)) => return Some(Slice::Each(values)),
                        None => {}
                    }
                    // The rows of the group whose argument is not null.
                    let rows = group.count.saturating_sub(acc.missing) as usize;
                    if AggregateAccumulator::takes_moments(*kind) {
                        return arg.as_ref().map(|_| Slice::Moments {
                            rows,
                            sum: &acc.sum,
                            squares: &acc.squares,
                        });
                    }
                    if AggregateAccumulator::takes_numbers(*kind) {
                        return arg.as_ref().map(|_| Slice::Float(&acc.values));
                    }
                    if *kind == AggKind::ApproxDistinct {
                        return arg.as_ref().map(|_| Slice::Sketch(&acc.sketch));
                    }
                    if AggregateAccumulator::takes_bits(*kind) {
                        return arg.as_ref().map(|_| Slice::Bits {
                            acc: acc.bits,
                            invalid: acc.not_bits,
                        });
                    }
                    arg.as_ref().map(|_| match (acc.min, acc.max, acc.missing) {
                        (Some(min), Some(max), 0) => Slice::Total {
                            sum: &acc.sum,
                            min: number(min),
                            max: number(max),
                        },
                        // A column of strings or booleans: counted.
                        (_, _, 0) => Slice::Other,
                        (Some(min), Some(max), _) => Slice::TotalOf {
                            rows,
                            sum: &acc.sum,
                            min: number(min),
                            max: number(max),
                        },
                        _ => Slice::OtherOf(rows),
                    })
                })
                .collect();
            fold.push_slices(self, &key, group.count as usize, &args)?;
        }
        Ok(())
    }

    /// [`Self::try_columnar_collect`] for a collection whose blocks are not
    /// kept decoded because they would not fit the budget
    /// (`ColumnarCollection::larger_than_kept`): the blocks are read and
    /// decoded as many at a time as there are threads, each folded into a
    /// table of its own and the tables merged, so that the query holds a
    /// few blocks and its groups, not the collection. The same groups as
    /// the other way; what it gives up is the arrays indexed by an integer
    /// key, which need every block's range first.
    fn columnar_collect_in_windows<'q>(
        &self,
        columnar: &crate::storage::ColumnarCollection,
        found: &Plan<'q>,
        col_plan: &ColPlan,
        known: &[String],
        outer: &Context,
    ) -> DbResult<Folded<'q>> {
        let wanted = self.scan.wanted(columnar.count() / 8);
        let helpers = Helpers::take(wanted.saturating_sub(1));
        let threads = helpers.count() + 1;
        let mut table = Table::default();
        let ev = Eval {
            ex: self,
            outer,
            var: found.for_clause.variable.as_str(),
        };
        // Blocks the arrays cannot answer for, by key: read again below.
        let mut left: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut scanned = 0usize;
        columnar.for_each_blocks(threads, None, |blocks| {
            scanned += blocks.iter().map(|b| b.len()).sum::<usize>();
            let work = |block: &Arc<ColBlock>| {
                let mut table = Table::default();
                let cells: Cells = col_plan.made.iter().map(|_| Default::default()).collect();
                let read = Read {
                    plan: col_plan,
                    ev: &ev,
                    cells: &cells,
                    block,
                };
                fold_block(&read, &mut table, Share::All).map(|()| table)
            };
            let folded: Vec<Option<Table>> = std::thread::scope(|scope| {
                let work = &work;
                let others: Vec<_> = blocks[1..]
                    .iter()
                    .map(|block| scope.spawn(move || work(block)))
                    .collect();
                let mut folded = vec![work(&blocks[0])];
                // (A thread that died: its block is read again, as rows.)
                folded.extend(others.into_iter().map(|other| other.join().ok().flatten()));
                folded
            });
            for (block, theirs) in blocks.iter().zip(folded) {
                match theirs {
                    Some(theirs) => table.merge(theirs),
                    None => {
                        left.insert(block.key.clone());
                    }
                }
            }
            Ok(())
        })?;
        drop(helpers);

        let tail = columnar.tail_rows()?;
        let mut fold = CollectFold::new(found.collect);
        let alone = left.is_empty()
            && tail.is_empty()
            && found.collect.grouping == crate::sdbql::ast::Grouping::None;
        self.push_groups(&mut fold, col_plan, &mut table.groups, None, alone)?;
        let var = found.for_clause.variable.as_str();
        let fields: &[String] = found.fields.as_deref().unwrap_or(known);
        let mut passes = vec![(0usize, 0usize); found.stages.len()];
        let mut scratch = outer.clone();
        let mut one = |row: serde_json::Map<String, Value>, fold: &mut CollectFold<'_>| {
            bind_in_place(&mut scratch, var, Value::Object(row));
            if self.run_stages(found.stages, &mut scratch, &mut passes)? {
                fold.push_ref(self, &scratch)?;
            }
            DbResult::Ok(())
        };
        if !left.is_empty() {
            crate::storage::columnar::BLOCKS_FOLDED_AS_ROWS
                .fetch_add(left.len() as u64, std::sync::atomic::Ordering::Relaxed);
            columnar.for_each_blocks(1, Some(&left), |blocks| {
                for block in blocks {
                    for row in 0..block.len() {
                        let object = fields
                            .iter()
                            .map(|f| {
                                let value = block.column(f).map_or(Value::Null, |c| c.value(row));
                                (f.clone(), value)
                            })
                            .collect();
                        one(object, &mut fold)?;
                    }
                }
                Ok(())
            })?;
        }
        scanned += tail.len();
        for (_, row) in &tail {
            let object = fields
                .iter()
                .map(|f| (f.clone(), row.get(f).cloned().unwrap_or(Value::Null)))
                .collect();
            one(object, &mut fold)?;
        }
        Ok(Folded {
            fold,
            scanned,
            passes,
            windowed: true,
        })
    }

    /// Run `clauses[i]`, a FOR over a columnar collection, and the clauses
    /// up to its COLLECT from the collection's blocks; `Ok(None)` when the
    /// ordinary path has to run them (see the module's notes).
    pub(in crate::sdbql::executor) fn try_columnar_collect(
        &self,
        clauses: &[BodyClause],
        i: usize,
        rows: &[Context],
    ) -> DbResult<Option<ScanCollectRun>> {
        if !self.stream_collect {
            return Ok(None);
        }
        let [outer] = rows else {
            return Ok(None);
        };
        let Some(found) = plan(clauses, i) else {
            return Ok(None);
        };
        let for_clause = found.for_clause;
        let name = &for_clause.collection;
        if found.collect.into_var.is_some()
            || IndexHint::of(for_clause).is_some()
            || outer.contains_key(name)
        {
            return Ok(None);
        }
        let Some(db_name) = &self.database else {
            return Ok(None);
        };
        let Ok(database) = self.storage.get_database(db_name) else {
            return Ok(None);
        };
        if !self.is_columnar(&database, name) || self.row_policy_applies(name) {
            return Ok(None);
        }
        // Before anything is read of it: a transaction that tracks its
        // reads compares the collection's write generation at commit.
        self.note_columnar_read(&database, name)?;
        // A sharded collection whose COLLECT each shard can compute is not
        // gathered: its shards send their folds (card #899).
        let logical =
            crate::storage::ColumnarCollection::load(name.to_string(), db_name, database.db_arc())?;
        let sharded = logical
            .sharding()
            .filter(|_| outer.is_empty() && CollectFold::sent_by_shards(found.collect));
        let columnar = match sharded {
            Some(_) => logical,
            None => self.open_columnar(name, db_name, &database)?,
        };
        let known: Vec<String> = columnar
            .metadata()?
            .columns
            .into_iter()
            .map(|c| c.name)
            .collect();
        let Some(col_plan) = self.columnar_plan(&found, &known, outer) else {
            return Ok(None);
        };
        let folded = match sharded {
            // Computed on each shard, their states merged here (card #899).
            Some(shards) => self.fold_shards(&found, &col_plan, &known, outer, name, &shards)?,
            None => {
                let last = i + found.stages.len() + 2 == clauses.len();
                let top_body = last.then_some(clauses.as_ptr() as usize);
                self.fold_columnar(&columnar, &found, &col_plan, &known, outer, top_body)?
            }
        };
        let windowed = folded.windowed;
        let groups = folded.fold.finish(self)?;
        self.check_budget(groups.len())?;
        Ok(Some(ScanCollectRun {
            rows: groups,
            consumed: found.stages.len() + 2,
            scanned: folded.scanned,
            filters: folded.passes,
            note: if windowed {
                format!(
                    "optimizer: COLLECT folded from the blocks of the columnar collection '{}', \
                     read a few at a time: they are larger than what is kept in memory",
                    name
                )
            } else {
                format!(
                    "optimizer: COLLECT folded from the blocks of the columnar collection '{}'",
                    name
                )
            },
        }))
    }

    /// The fold of `found` over every shard of the sharded collection
    /// `name`, each computed where the shard is and sent as its state
    /// (card #899). Only for a COLLECT whose aggregates send their state
    /// and that reads nothing from outside its loop.
    #[allow(clippy::too_many_arguments)]
    fn fold_shards<'q>(
        &self,
        found: &Plan<'q>,
        col_plan: &ColPlan,
        known: &[String],
        outer: &Context,
        name: &str,
        shards: &crate::storage::columnar::ColumnarShards,
    ) -> DbResult<Folded<'q>> {
        let db_name = self
            .database
            .as_ref()
            .ok_or_else(|| DbError::ExecutionError("no database".to_string()))?;
        let database = self.storage.get_database(db_name)?;
        let mut total = CollectFold::new(found.collect);
        let mut scanned = 0usize;
        let mut passes = vec![(0usize, 0usize); found.stages.len()];
        for shard in 0..shards.num_shards {
            let phys = crate::sharding::columnar::physical(name, shard);
            // The clauses a shard's node runs: the loop over its shard.
            let mut for_clause = found.for_clause.clone();
            // The name is in both places when the loop reads a collection.
            if for_clause.source_variable.as_deref() == Some(name) {
                for_clause.source_variable = Some(phys.clone());
            }
            for_clause.collection = phys.clone();
            let mut clauses = vec![BodyClause::For(for_clause)];
            clauses.extend(found.stages.iter().cloned());
            clauses.push(BodyClause::Collect(found.collect.clone()));
            let request = serde_json::json!({ "clauses": clauses, "binds": self.bind_vars });
            let bytes = crate::sharding::columnar::fold_shard(
                self.shard_coordinator.as_deref(),
                &database,
                name,
                shards,
                shard,
                &request,
                |local| {
                    let folded = self.fold_columnar(local, found, col_plan, known, outer, None)?;
                    for (total, part) in passes.iter_mut().zip(&folded.passes) {
                        total.0 += part.0;
                        total.1 += part.1;
                    }
                    shard_state(&folded)
                },
            )?;
            let (state, rows): (FoldState, usize) = rmp_serde::from_slice(&bytes)
                .map_err(|e| DbError::InternalError(format!("shard fold: {e}")))?;
            total.absorb(state, (u64::from(shard) + 1) << 48);
            scanned += rows;
        }
        Ok(Folded {
            fold: total,
            scanned,
            passes,
            windowed: false,
        })
    }

    /// Run `clauses` (a FOR over one shard, its FILTER and LET clauses, a
    /// COLLECT) on this node's shard and return its fold's state, as
    /// [`Self::fold_shards`] reads it: what a shard's node answers.
    pub fn shard_fold_state(&self, clauses: &[BodyClause]) -> DbResult<Vec<u8>> {
        let refused = || {
            DbError::BadRequest(
                "a shard computes a FOR … COLLECT it can send the state of".to_string(),
            )
        };
        let found = plan(clauses, 0).ok_or_else(refused)?;
        if found.stages.len() + 2 != clauses.len() || !CollectFold::sent_by_shards(found.collect) {
            return Err(refused());
        }
        let db_name = self
            .database
            .as_ref()
            .ok_or_else(|| DbError::ExecutionError("no database".to_string()))?;
        let database = self.storage.get_database(db_name)?;
        let columnar = crate::storage::ColumnarCollection::load(
            found.for_clause.collection.clone(),
            db_name,
            database.db_arc(),
        )?;
        let known: Vec<String> = columnar
            .metadata()?
            .columns
            .into_iter()
            .map(|c| c.name)
            .collect();
        let outer = Context::default();
        let col_plan = self
            .columnar_plan(&found, &known, &outer)
            .ok_or_else(refused)?;
        let folded = self.fold_columnar(&columnar, &found, &col_plan, &known, &outer, None)?;
        shard_state(&folded)
    }

    /// The fold of `found` over `columnar`'s blocks and tail.
    /// `top_body` is the body the COLLECT ends when it does (the address
    /// of its clauses), for the top-k hint of a `SORT … LIMIT`.
    fn fold_columnar<'q>(
        &self,
        columnar: &crate::storage::ColumnarCollection,
        found: &Plan<'q>,
        col_plan: &ColPlan,
        known: &[String],
        outer: &Context,
        top_body: Option<usize>,
    ) -> DbResult<Folded<'q>> {
        let for_clause = found.for_clause;
        // Too large to be kept decoded: a few blocks at a time.
        if columnar.larger_than_kept() {
            return self.columnar_collect_in_windows(columnar, found, col_plan, known, outer);
        }
        let set = columnar.row_set()?;

        // The blocks, on as many threads as the scan may have: each fills
        // a table of its own, and names the blocks it could not read as
        // arrays.
        // Rows to go through: all of them, or those a first equality on a
        // column of strings lists. Reading arrays is far cheaper than
        // decoding documents: a thread is worth it for eight times the rows
        // a document scan asks.
        let total: usize = set
            .blocks
            .iter()
            .map(|block| match col_plan.preds.first() {
                Some(Pred::Cmp(Cond {
                    column,
                    op: BinaryOperator::Equal,
                    value: Value::String(wanted),
                })) => block.column(column).map_or(block.len(), |c| match &c.data {
                    ColData::Str { dict, .. } if c.nulls.is_none() => dict
                        .binary_search_by(|d| d.as_str().cmp(wanted))
                        .map_or(0, |code| c.rows_of(code as u32).len()),
                    _ => block.len(),
                }),
                _ => block.len(),
            })
            .sum();
        let wanted = self.scan.wanted(total / 8).min(set.blocks.len().max(1));
        let helpers = Helpers::take(wanted.saturating_sub(1));
        let threads = helpers.count() + 1;
        // One numeric key is likely to have many values: the groups are
        // shared out by key, every thread reads every block and keeps the
        // rows of its own keys. Otherwise the blocks are shared out, and
        // the few groups of each thread merged.
        let ev = Eval {
            ex: self,
            outer,
            var: for_clause.variable.as_str(),
        };
        // The computed columns of each block, made by the first thread
        // that asks and read by the others.
        let cells: Vec<Cells> = set
            .blocks
            .iter()
            .map(|_| col_plan.made.iter().map(|_| Default::default()).collect())
            .collect();
        let reading = |at: usize| Read {
            plan: col_plan,
            ev: &ev,
            cells: &cells[at],
            block: &set.blocks[at],
        };
        let numeric = |column: Option<&Column>| {
            column.is_some_and(|c| matches!(c.data, ColData::Int(_) | ColData::Float(_)))
        };
        let by_key = col_plan.keys.len() == 1
            && set
                .blocks
                .iter()
                .position(|b| b.len() > 0)
                .is_some_and(|at| match &col_plan.keys[0] {
                    Key::Column(name) => numeric(set.blocks[at].column(name)),
                    // The start of a bucket of time is a number.
                    Key::Bucket { .. } => true,
                    Key::Made(made) => numeric(reading(at).made(*made)),
                });
        let per = set.blocks.len().div_ceil(threads).max(1);
        // An integer key within a small range: groups by position. Only
        // when the key and the arguments are columns, no block holds a
        // null in one of them and nothing counts distinct values:
        // positions hold none of that.
        let read: Option<Vec<&str>> = col_plan
            .keys
            .iter()
            .map(|key| match key {
                Key::Column(name) => Some(name.as_str()),
                _ => None,
            })
            .chain(col_plan.args.iter().flatten().map(|arg| match arg {
                Source::Column(name) => Some(name.as_str()),
                Source::Made(_) => None,
            }))
            .collect();
        let no_null = !col_plan
            .kinds
            .iter()
            .any(|kind| AggregateAccumulator::takes_its_own(*kind))
            && read.as_ref().is_some_and(|names| {
                names.iter().all(|name| {
                    set.blocks
                        .iter()
                        .all(|b| b.stats(name).is_some_and(|stats| !stats.nulls))
                })
            });
        let ranged = (by_key && no_null)
            .then(|| {
                set.blocks
                    .iter()
                    .filter(|b| b.len() > 0)
                    .map(|b| match &col_plan.keys[0] {
                        Key::Column(name) => b.column(name).and_then(Column::int_range),
                        _ => None,
                    })
                    .try_fold((i64::MAX, i64::MIN), |(lo, hi), range| {
                        range.map(|(a, b)| (lo.min(a), hi.max(b)))
                    })
            })
            .flatten()
            .filter(|(lo, hi)| lo <= hi && hi.wrapping_sub(*lo) >= 0 && hi - lo < (1 << 20));
        let share_of = |mine: u64, of: u64| match ranged {
            Some((lo, hi)) => Share::Ranged {
                mine,
                scale: (of << 32) / ((hi - lo) as u64 + 1),
                base: lo,
                span: (hi - lo) as u64,
                flat: true,
            },
            None => Share::Hashed { mine, of },
        };
        let work = |blocks: &[Arc<ColBlock>], from: usize, share: Share| -> (Table, Vec<usize>) {
            let pass = |share: Share| {
                let mut table = Table::default();
                let mut left = Vec::new();
                for at in from..from + blocks.len() {
                    if fold_block(&reading(at), &mut table, share).is_none() {
                        left.push(at);
                    }
                }
                (table, left)
            };
            let (mut table, left) = pass(share);
            // Plain sums that are exact stay as they are, for the caller to
            // make groups of; one that is not sends the thread through its
            // rows again, with exact sums.
            if let (
                Some(flat),
                Share::Ranged {
                    mine,
                    scale,
                    base,
                    span,
                    ..
                },
            ) = (table.flat.take(), share)
            {
                // (A scale of zero: the blocks are shared out, not the
                // keys, and the caller decides once the tables are added.)
                if scale != 0 && !flat.valid() {
                    return pass(Share::Ranged {
                        mine,
                        scale,
                        base,
                        span,
                        flat: false,
                    });
                }
                table.flat = Some(flat);
            }
            (table, left)
        };
        let (mut table, mut left) = (Table::default(), Vec::new());
        let mut flats: Vec<Flat> = Vec::new();
        let all: &[Arc<ColBlock>] = &set.blocks;
        std::thread::scope(|scope| {
            let work = &work;
            // An integer key of small range: first with the blocks shared
            // out and plain sums in arrays indexed by the key, added
            // together at the end. Each row is read once. If a sum turns
            // out not to be exact, the keys are shared out below, with
            // exact sums where needed.
            if let Some((lo, hi)) = ranged {
                let plain = Share::Ranged {
                    mine: 0,
                    scale: 0,
                    base: lo,
                    span: (hi - lo) as u64,
                    flat: true,
                };
                let mut chunks = all.chunks(per).enumerate();
                let mine = chunks.next();
                let others: Vec<_> = chunks
                    .map(|(n, chunk)| scope.spawn(move || work(chunk, n * per, plain)))
                    .collect();
                let mut added: Option<Flat> = None;
                let mut lefts = Vec::new();
                let mut whole = true;
                let mut take = |(mut theirs, more): (Table, Vec<usize>)| {
                    lefts.extend(more);
                    match (theirs.flat.take(), added.as_mut()) {
                        (Some(flat), None) => added = Some(flat),
                        (Some(flat), Some(sum)) => sum.absorb(&flat),
                        (None, _) => {}
                    }
                };
                if let Some((_, chunk)) = mine {
                    take(work(chunk, 0, plain));
                }
                for other in others {
                    match other.join() {
                        Ok(result) => take(result),
                        Err(_) => whole = false,
                    }
                }
                if whole && added.as_ref().is_none_or(Flat::valid) {
                    flats.extend(added);
                    left = lefts;
                    return;
                }
            }
            // (On one thread too when the key is ranged: the arrays are
            // what makes it fast, not the sharing.)
            if by_key && (threads > 1 || ranged.is_some()) {
                let of = threads as u64;
                let others: Vec<_> = (1..of)
                    .map(|mine| scope.spawn(move || work(all, 0, share_of(mine, of))))
                    .collect();
                // A block that is not read as arrays is so for every
                // thread: this one names them.
                (table, left) = work(all, 0, share_of(0, of));
                flats.extend(table.flat.take());
                for other in others {
                    if let Ok((mut theirs, _)) = other.join() {
                        flats.extend(theirs.flat.take());
                        table.groups.extend(theirs.groups);
                    }
                }
                return;
            }
            let mut chunks = all.chunks(per).enumerate();
            let mine = chunks.next();
            let others: Vec<_> = chunks
                .map(|(n, chunk)| scope.spawn(move || work(chunk, n * per, Share::All)))
                .collect();
            if let Some((_, chunk)) = mine {
                (table, left) = work(chunk, 0, Share::All);
            }
            for other in others {
                if let Ok((theirs, more)) = other.join() {
                    table.merge(theirs);
                    left.extend(more);
                }
            }
        });
        drop(helpers);

        // When the query sorts by an aggregate and keeps a few rows, and
        // every row was read as arrays, only the groups that can be among
        // those few go on: those at or past the value of the last one.
        // Never on a shard: a group's total is made of every shard's.
        let top = top_body
            .and_then(|at| {
                self.caches
                    .collect_top
                    .lock()
                    .clone()
                    .filter(|(body, ..)| *body == at)
            })
            .map(|(_, var, ascending, k)| (var, ascending, k))
            .filter(|_| left.is_empty() && set.tail.is_empty());
        // The plain tables become groups: all of them, or, for such a
        // query and when every thread kept its sums plain, only those that
        // can be returned.
        if let Some((base, _)) = ranged.filter(|_| !flats.is_empty()) {
            let aim =
                top.as_ref()
                    .filter(|_| table.groups.is_empty())
                    .and_then(|(var, ascending, k)| {
                        let agg = found
                            .collect
                            .aggregates
                            .iter()
                            .position(|a| &a.variable == var)?;
                        let kind = col_plan.kinds[agg];
                        let mut scores: Vec<f64> = flats
                            .iter()
                            .flat_map(|flat| {
                                flat.present().map(move |at| flat.score(at, agg, kind))
                            })
                            .collect();
                        let usable =
                            *k > 0 && *k < scores.len() && scores.iter().all(|s| s.is_finite());
                        usable.then(|| {
                            let order = |a: &f64, b: &f64| {
                                let by = a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
                                if *ascending {
                                    by
                                } else {
                                    by.reverse()
                                }
                            };
                            let (_, limit, _) = scores.select_nth_unstable_by(*k - 1, order);
                            (agg, kind, *ascending, *limit)
                        })
                    });
            for flat in &flats {
                table.groups.extend(flat.groups(base, |at| match aim {
                    None => true,
                    Some((agg, kind, ascending, limit)) => {
                        let score = flat.score(at, agg, kind);
                        if ascending {
                            score <= limit
                        } else {
                            score >= limit
                        }
                    }
                }));
            }
        }
        let mut kept: Option<Vec<bool>> = None;
        if let Some((var, ascending, k)) = top {
            let at = found
                .collect
                .aggregates
                .iter()
                .position(|a| a.variable == var);
            if let (Some(at), true) = (at, k > 0 && k < table.groups.len()) {
                let score = |group: &Group| -> f64 {
                    let acc = &group.accs[at];
                    match col_plan.kinds[at] {
                        AggKind::Sum => acc.sum.value(),
                        AggKind::Avg => acc.sum.value() / group.count as f64,
                        AggKind::Min => acc.min.map_or(f64::NAN, |m| m.0),
                        AggKind::Max => acc.max.map_or(f64::NAN, |m| m.0),
                        // Not a number the groups can be cut by.
                        // The number the fold will compute, from the same
                        // sums; a group without enough values is null,
                        // which no number stands for.
                        AggKind::Variance { sample } | AggKind::Stddev { sample } => {
                            let n = group.count.saturating_sub(acc.missing);
                            let variance = Moments::variance_of(n, &acc.sum, &acc.squares, sample)
                                .unwrap_or(f64::NAN);
                            match col_plan.kinds[at] {
                                AggKind::Stddev { .. } => variance.sqrt(),
                                _ => variance,
                            }
                        }
                        kind if AggregateAccumulator::takes_its_own(kind) => f64::NAN,
                        _ => group.count as f64,
                    }
                };
                let mut scores: Vec<f64> = table.groups.iter().map(score).collect();
                if scores.iter().all(|s| s.is_finite()) {
                    let order = |a: &f64, b: &f64| {
                        let by = a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
                        if ascending {
                            by
                        } else {
                            by.reverse()
                        }
                    };
                    let (_, limit, _) = scores.select_nth_unstable_by(k - 1, order);
                    let limit = *limit;
                    kept = Some(
                        table
                            .groups
                            .iter()
                            .map(|g| {
                                if ascending {
                                    score(g) <= limit
                                } else {
                                    score(g) >= limit
                                }
                            })
                            .collect(),
                    );
                }
            }
        }

        // Each group goes into the fold the ordinary path fills, with the
        // state of its sums.
        let mut fold = CollectFold::new(found.collect);
        // ROLLUP / CUBE merges the groups' distinct values into its
        // subtotals: a count alone cannot be merged (it would add two
        // groups' counts of the same value).
        let alone = left.is_empty()
            && set.tail.is_empty()
            && found.collect.grouping == crate::sdbql::ast::Grouping::None;
        self.push_groups(
            &mut fold,
            col_plan,
            &mut table.groups,
            kept.as_deref(),
            alone,
        )?;

        // What the arrays did not answer, as objects: the blocks left, and
        // the tail.
        let var = for_clause.variable.as_str();
        let fields: &[String] = found.fields.as_deref().unwrap_or(known);
        let mut passes = vec![(0usize, 0usize); found.stages.len()];
        let mut scratch = outer.clone();
        let mut one = |row: serde_json::Map<String, Value>, fold: &mut CollectFold<'_>| {
            bind_in_place(&mut scratch, var, Value::Object(row));
            if self.run_stages(found.stages, &mut scratch, &mut passes)? {
                fold.push_ref(self, &scratch)?;
            }
            DbResult::Ok(())
        };
        left.sort_unstable();
        crate::storage::columnar::BLOCKS_FOLDED_AS_ROWS
            .fetch_add(left.len() as u64, std::sync::atomic::Ordering::Relaxed);
        for at in left {
            let block = &set.blocks[at];
            for row in 0..block.len() {
                let object = fields
                    .iter()
                    .map(|f| {
                        let value = block.column(f).map_or(Value::Null, |c| c.value(row));
                        (f.clone(), value)
                    })
                    .collect();
                one(object, &mut fold)?;
            }
        }
        for (_, row) in &set.tail {
            let object = fields
                .iter()
                .map(|f| (f.clone(), row.get(f).cloned().unwrap_or(Value::Null)))
                .collect();
            one(object, &mut fold)?;
        }

        Ok(Folded {
            fold,
            scanned: set.len(),
            passes,
            windowed: false,
        })
    }
}

/// A fold of a shard's rows, before its groups are made rows.
struct Folded<'q> {
    fold: CollectFold<'q>,
    scanned: usize,
    passes: Vec<(usize, usize)>,
    windowed: bool,
}

/// What a shard sends: its fold's state and the rows it read.
fn shard_state(folded: &Folded<'_>) -> DbResult<Vec<u8>> {
    let state = folded.fold.to_state().ok_or_else(|| {
        DbError::InternalError("a shard's fold holds an aggregate it cannot send".to_string())
    })?;
    rmp_serde::to_vec(&(state, folded.scanned))
        .map_err(|e| DbError::InternalError(format!("shard fold: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdbql::executor::helpers::evaluate_binary_op;
    use serde_json::json;

    /// Arithmetic computed in the arrays is what the evaluator computes,
    /// value for value: integers that overflow, divisions that leave a
    /// remainder or are by zero, nulls, floats beside integers.
    #[test]
    fn arithmetic_in_the_arrays_is_the_evaluators() {
        let ints = [
            json!(0),
            json!(1),
            json!(-1),
            json!(7),
            json!(-13),
            json!(1i64 << 53),
            json!(i64::MAX),
            json!(i64::MIN),
            Value::Null,
        ];
        let floats = [
            json!(0.0),
            json!(-0.5),
            json!(2.25),
            json!(1e300),
            json!(-1e300),
            json!(3.0),
            Value::Null,
        ];
        let field = |name: &str| {
            Expression::FieldAccess(
                Box::new(Expression::Variable("r".to_string())),
                name.to_string(),
            )
        };
        let ops = [
            BinaryOperator::Add,
            BinaryOperator::Subtract,
            BinaryOperator::Multiply,
            BinaryOperator::Divide,
            BinaryOperator::Modulus,
        ];
        for (left, right) in [
            (&ints[..], &ints[..]),
            (&ints[..], &floats[..]),
            (&floats[..], &ints[..]),
            (&floats[..], &floats[..]),
        ] {
            // Every pair of values, as the rows of one block.
            let rows: Vec<(String, serde_json::Map<String, Value>)> = left
                .iter()
                .flat_map(|a| right.iter().map(move |b| (a.clone(), b.clone())))
                .enumerate()
                .map(|(n, (a, b))| {
                    let Value::Object(row) = json!({"a": a, "b": b}) else {
                        unreachable!()
                    };
                    (format!("row-{n:05}"), row)
                })
                .collect();
            let names = ["a".to_string(), "b".to_string()];
            let block = ColBlock::seal(&names, &rows);
            // A column of nulls only has no kind: the evaluator's.
            for op in &ops {
                let expr = Expression::BinaryOp {
                    left: Box::new(field("a")),
                    op: op.clone(),
                    right: Box::new(field("b")),
                };
                let Some(sum) = Sum::of(&expr, "r", &block) else {
                    panic!("{op:?}: not computed in the arrays");
                };
                for (n, (_, row)) in rows.iter().enumerate() {
                    let wanted = evaluate_binary_op(&row["a"], op, &row["b"]).unwrap();
                    let got = match sum.at(n) {
                        Some(Num::Int(i)) => json!(i),
                        Some(Num::Float(f)) => json!(f),
                        None => Value::Null,
                    };
                    assert_eq!(got, wanted, "{} {op:?} {}", row["a"], row["b"]);
                    // The same kind of number, not only the same value.
                    assert_eq!(
                        got.is_i64(),
                        wanted.is_i64(),
                        "{} {op:?} {}",
                        row["a"],
                        row["b"]
                    );
                }
                // As a column: of one kind, or of both where that is allowed.
                let column = arithmetic(&expr, "r", &block, true);
                if let Some(column) = column {
                    for (n, (_, row)) in rows.iter().enumerate() {
                        let wanted = evaluate_binary_op(&row["a"], op, &row["b"]).unwrap();
                        assert_eq!(column.value(n), wanted, "{} {op:?} {}", row["a"], row["b"]);
                    }
                }
            }
        }
    }
}
