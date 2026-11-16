//! A COLLECT over a columnar collection, folded from its blocks.
//!
//! `FOR r IN c [FILTER …] COLLECT … AGGREGATE …` over a columnar collection
//! used to build every row as an object, then filter and group the
//! objects. When the query stays within what the arrays answer —
//!
//! - every FILTER is made of comparisons of a column with a constant
//!   (`==`, `!=`, `<`, `<=`, `>`, `>=` with a string, a number, a
//!   boolean or null, on either side; `IN` a list of such), joined by
//!   `AND` and `OR`;
//! - every group key is a column, or `TIME_BUCKET(column, "<fixed
//!   interval>")` of a column of integers;
//! - every aggregate is one that takes whole columns (`COUNT`, `SUM`,
//!   `AVG`, `MIN`, `MAX`) of a column, or of nothing
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

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::super::aggregation::{AggKind, AggregateAccumulator, Slice};
use super::super::datum::Datum;
use super::super::exact_sum::ExactSum;
use super::super::index_opt::{expression_references_var, push_conjuncts, IndexHint};
use super::super::types::Context;
use super::super::QueryExecutor;
use super::collect::CollectFold;
use super::scan_collect::{bind_in_place, plan, Plan, ScanCollectRun};
use super::scan_threads::Helpers;
use crate::error::DbResult;
use crate::sdbql::ast::{BinaryOperator, BodyClause, Expression};
use crate::sdbql::executor::builtins::timeseries::{parse_interval, Interval};
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
}

/// A group key.
enum Key {
    Column(String),
    /// `TIME_BUCKET(column, "<interval>")` for an interval of `step`
    /// milliseconds: the instant its bucket starts at.
    Bucket {
        column: String,
        step: i64,
    },
}

impl Key {
    /// The column the key is read from.
    fn column(&self) -> &str {
        match self {
            Key::Column(column) | Key::Bucket { column, .. } => column,
        }
    }
}

struct ColPlan {
    preds: Vec<Pred>,
    keys: Vec<Key>,
    /// For each aggregate, its argument and what it computes.
    args: Vec<Option<String>>,
    kinds: Vec<AggKind>,
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

/// What one aggregate holds of one group's numbers.
#[derive(Default)]
struct Acc {
    /// The rows of the group whose argument is null: what the aggregate
    /// passes over.
    missing: u64,
    sum: ExactSum,
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
        self.sum.merge(&other.sum);
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

    fn merge(&mut self, other: Table) {
        for mut group in other.groups {
            let id = self.group(group.key, group.accs.len()) as usize;
            let mine = &mut self.groups[id];
            mine.count += group.count;
            for (acc, theirs) in mine.accs.iter_mut().zip(&mut group.accs) {
                // Settled, a sum goes in at the cost of an addition.
                theirs.sum.settle();
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
    // What the comparison keeps of how a value stands to the constant.
    let wanted: fn(Ordering) -> bool = match cond.op {
        Equal => |o| o == Ordering::Equal,
        NotEqual => |o| o != Ordering::Equal,
        LessThan => |o| o == Ordering::Less,
        LessThanOrEqual => |o| o != Ordering::Greater,
        GreaterThan => |o| o == Ordering::Greater,
        GreaterThanOrEqual => |o| o != Ordering::Less,
        _ => return None,
    };
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
        _ => None,
    }
}

/// The rows `pred` keeps among `rows` (`None`: every row of the block),
/// ascending; the outer `None` when the arrays cannot answer for it.
fn kept(
    block: &ColBlock,
    pred: &Pred,
    rows: Option<Vec<u32>>,
    len: usize,
) -> Option<Option<Vec<u32>>> {
    if rows.as_ref().is_some_and(Vec::is_empty) {
        return Some(rows);
    }
    match pred {
        Pred::Cmp(cond) => {
            let column = block
                .column(&cond.column)
                .filter(|c| !matches!(c.data, ColData::Json(_)))?;
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
        Pred::All(all) => {
            let mut rows = rows;
            for pred in all {
                rows = kept(block, pred, rows, len)?;
            }
            Some(rows)
        }
        Pred::Any(any) => {
            let mut found: Vec<u32> = Vec::new();
            for pred in any {
                match kept(block, pred, rows.clone(), len)? {
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
    }
}

/// Fold rows whose keys or arguments hold nulls: a row at a time, a null
/// key being a group of its own and a null argument passed over
/// (`Acc::missing`). The groups of a single numeric key are found where
/// the blocks without nulls put them (`Table::numbers`), so that a key is
/// one group in the table.
fn fold_with_nulls(
    table: &mut Table,
    share: Share,
    keys: &[&Column],
    args: &[Option<&Column>],
    rows: Option<&[u32]>,
    len: usize,
) -> Option<()> {
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
    let mut seen: HashMap<Vec<u64>, u32, foldhash::fast::RandomState> = HashMap::default();
    let mut scratch: Vec<u64> = Vec::with_capacity(keys.len() + 1);
    let mut each = |row: usize| {
        let group = if by_number {
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
        for (acc, column) in group.accs.iter_mut().zip(args) {
            match column {
                Some(column) if is_null(column, row) => acc.missing += 1,
                Some(column) => match &column.data {
                    ColData::Int(values) => acc.int(values[row]),
                    ColData::Float(values) => acc.float(values[row]),
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
fn fold_block(plan: &ColPlan, block: &ColBlock, table: &mut Table, share: Share) -> Option<()> {
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
        rows = kept(block, pred, rows, len)?;
    }
    // A key that is a bucket of time: the instants its buckets start at,
    // as a column of its own. One that overflows is the builtin's to
    // report, from the rows.
    let mut made: Vec<Option<Column>> = Vec::with_capacity(plan.keys.len());
    for key in &plan.keys {
        made.push(match key {
            Key::Column(_) => None,
            Key::Bucket { column, step } => {
                let source = usable(column)?;
                let ColData::Int(times) = &source.data else {
                    return None;
                };
                let starts = times
                    .iter()
                    .map(|time| time.div_euclid(*step).checked_mul(*step))
                    .collect::<Option<Vec<i64>>>()?;
                Some(Column::ints(starts, source.nulls.clone()))
            }
        });
    }
    let keys: Vec<&Column> = plan
        .keys
        .iter()
        .zip(&made)
        .map(|(key, made)| match made {
            Some(column) => Some(column),
            None => usable(key.column()),
        })
        .collect::<Option<_>>()?;
    // Numbers for the aggregates that add or compare; a string or a
    // boolean can only be counted.
    let mut args: Vec<Option<&Column>> = Vec::with_capacity(plan.args.len());
    for (arg, kind) in plan.args.iter().zip(&plan.kinds) {
        let column = match arg {
            None => None,
            Some(name) => Some(usable(name)?),
        };
        let numeric = column.is_none_or(|c| matches!(c.data, ColData::Int(_) | ColData::Float(_)));
        if !numeric && !matches!(kind, AggKind::CountNonNull | AggKind::CountRows) {
            return None;
        }
        args.push(column);
    }
    let accs = args.len();
    // A null among the keys or the arguments: a row at a time, with the
    // masks. What follows reads whole arrays.
    let nullable = keys
        .iter()
        .chain(args.iter().flatten())
        .any(|column| column.nulls.is_some());
    if nullable {
        return fold_with_nulls(table, share, &keys, &args, rows.as_deref(), len);
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
    fn columnar_plan(
        &self,
        found: &Plan<'_>,
        known: &[String],
        outer: &Context,
    ) -> Option<ColPlan> {
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
            preds: Vec::new(),
            keys: Vec::new(),
            args: Vec::new(),
            kinds: Vec::new(),
        };
        let mut conjuncts = Vec::new();
        for stage in found.stages {
            let BodyClause::Filter(filter) = stage else {
                return None;
            };
            push_conjuncts(&filter.expression, &mut conjuncts);
        }
        for conjunct in conjuncts {
            plan.preds
                .push(self.columnar_pred(conjunct, var, &column, outer)?);
        }
        for (_, expr) in &found.collect.group_vars {
            plan.keys.push(match expr {
                Expression::FunctionCall { name, args }
                    if name.eq_ignore_ascii_case("TIME_BUCKET") && args.len() == 2 =>
                {
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
                        Interval::Fixed(step) if step > 0 => Key::Bucket {
                            column: column(&args[0])?,
                            step,
                        },
                        _ => return None,
                    }
                }
                other => Key::Column(column(other)?),
            });
        }
        for aggregate in &found.collect.aggregates {
            plan.args.push(match &aggregate.argument {
                None => None,
                Some(expr) => Some(column(expr)?),
            });
            let kind = AggKind::resolve(&aggregate.function, aggregate.argument.is_some())?;
            if !AggregateAccumulator::takes_slices(kind) {
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
                // `column <op> constant`, or the same the other way round.
                let (name, value, op) = match (column(left), column(right)) {
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
    /// its sums; `kept`, when given, says which groups go.
    fn push_groups(
        &self,
        fold: &mut CollectFold<'_>,
        col_plan: &ColPlan,
        groups: &[Group],
        kept: Option<&[bool]>,
    ) -> DbResult<()> {
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
            let args: Vec<Option<Slice<'_>>> = col_plan
                .args
                .iter()
                .zip(&group.accs)
                .map(|(arg, acc)| {
                    // The rows of the group whose argument is not null.
                    let rows = group.count.saturating_sub(acc.missing) as usize;
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
    fn columnar_collect_in_windows(
        &self,
        columnar: &crate::storage::ColumnarCollection,
        found: &Plan<'_>,
        col_plan: &ColPlan,
        known: &[String],
        outer: &Context,
        name: &str,
    ) -> DbResult<ScanCollectRun> {
        let wanted = self.scan.wanted(columnar.count() / 8);
        let helpers = Helpers::take(wanted.saturating_sub(1));
        let threads = helpers.count() + 1;
        let mut table = Table::default();
        // Blocks the arrays cannot answer for, by key: read again below.
        let mut left: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut scanned = 0usize;
        columnar.for_each_blocks(threads, None, |blocks| {
            scanned += blocks.iter().map(|b| b.len()).sum::<usize>();
            let work = |block: &Arc<ColBlock>| {
                let mut table = Table::default();
                fold_block(col_plan, block, &mut table, Share::All).map(|()| table)
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

        let mut fold = CollectFold::new(found.collect);
        self.push_groups(&mut fold, col_plan, &table.groups, None)?;
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
        let tail = columnar.tail_rows()?;
        scanned += tail.len();
        for (_, row) in &tail {
            let object = fields
                .iter()
                .map(|f| (f.clone(), row.get(f).cloned().unwrap_or(Value::Null)))
                .collect();
            one(object, &mut fold)?;
        }
        let groups = fold.finish()?;
        self.check_budget(groups.len())?;
        Ok(ScanCollectRun {
            rows: groups,
            consumed: found.stages.len() + 2,
            scanned,
            filters: passes,
            note: format!(
                "optimizer: COLLECT folded from the blocks of the columnar collection '{}', \
                 read a few at a time: they are larger than what is kept in memory",
                name
            ),
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
        if !self.is_columnar(&database, name) {
            return Ok(None);
        }
        let columnar =
            crate::storage::ColumnarCollection::load(name.to_string(), db_name, database.db_arc())?;
        let known: Vec<String> = columnar
            .metadata()?
            .columns
            .into_iter()
            .map(|c| c.name)
            .collect();
        let Some(col_plan) = self.columnar_plan(&found, &known, outer) else {
            return Ok(None);
        };
        self.refuse_untracked_read("a columnar collection")?;
        // Too large to be kept decoded: a few blocks at a time.
        if columnar.larger_than_kept() {
            return self
                .columnar_collect_in_windows(&columnar, &found, &col_plan, &known, outer, name)
                .map(Some);
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
        let by_key = col_plan.keys.len() == 1
            && set.blocks.iter().find(|b| b.len() > 0).is_some_and(|b| {
                b.column(col_plan.keys[0].column())
                    .is_some_and(|c| matches!(c.data, ColData::Int(_) | ColData::Float(_)))
            });
        let per = set.blocks.len().div_ceil(threads).max(1);
        // An integer key within a small range: groups by position. Only
        // for a key that is a column, and when no block holds a null in
        // the key or in an argument: positions hold neither.
        let plain_key = matches!(col_plan.keys.first(), Some(Key::Column(_)));
        let read = col_plan
            .keys
            .iter()
            .map(Key::column)
            .chain(col_plan.args.iter().flatten().map(String::as_str));
        let no_null = plain_key
            && read.into_iter().all(|name| {
                set.blocks
                    .iter()
                    .all(|b| b.stats(name).is_some_and(|stats| !stats.nulls))
            });
        let ranged = (by_key && no_null)
            .then(|| {
                set.blocks
                    .iter()
                    .filter(|b| b.len() > 0)
                    .map(|b| {
                        b.column(col_plan.keys[0].column())
                            .and_then(Column::int_range)
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
                for (at, block) in blocks.iter().enumerate() {
                    if fold_block(&col_plan, block, &mut table, share).is_none() {
                        left.push(from + at);
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
        let last = i + found.stages.len() + 2 == clauses.len();
        let top = self
            .caches
            .collect_top
            .lock()
            .clone()
            .filter(|(body, ..)| *body == clauses.as_ptr() as usize)
            .map(|(_, var, ascending, k)| (var, ascending, k))
            .filter(|_| last && left.is_empty() && set.tail.is_empty());
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
        self.push_groups(&mut fold, &col_plan, &table.groups, kept.as_deref())?;

        // What the arrays did not answer, as objects: the blocks left, and
        // the tail.
        let var = for_clause.variable.as_str();
        let fields: &[String] = found.fields.as_deref().unwrap_or(&known);
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

        let groups = fold.finish()?;
        self.check_budget(groups.len())?;
        Ok(Some(ScanCollectRun {
            rows: groups,
            consumed: found.stages.len() + 2,
            scanned: set.len(),
            filters: passes,
            note: format!(
                "optimizer: COLLECT folded from the blocks of the columnar collection '{}'",
                name
            ),
        }))
    }
}
