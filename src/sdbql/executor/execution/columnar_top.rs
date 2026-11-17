//! The first rows of each partition of a columnar collection, read from
//! its column arrays.
//!
//! ```text
//! FOR r IN c
//!   LET n = ROW_NUMBER() OVER (PARTITION BY r.a ORDER BY r.b DESC)
//!   FILTER n <= 3
//! ```
//!
//! The ordinary path builds every row of the collection, numbers them all,
//! then drops nearly all of them: 4.4 s for two million rows (card #953).
//! Here each block keeps, for each partition it holds, the rows that can
//! be among its first `k` — compared in the arrays, with no row built —
//! and only those few are compared across blocks, numbered and built.
//!
//! It must answer what the ordinary path answers: the rows that pass, in
//! the order of the collection's rows, each with its number. Of rows equal
//! by the ORDER BY the earlier row comes first, as a stable sort leaves
//! them. Anything this does not read exactly as the evaluator does (a
//! column of several kinds, integers and floats together, rows out of id
//! order, a row policy, a function other than `ROW_NUMBER`) is left to the
//! ordinary path.

use std::cmp::Ordering;
use std::collections::HashMap;

use serde_json::Value;

use super::super::index_opt::{expression_references_var, IndexHint};
use super::super::types::Context;
use super::super::QueryExecutor;
use super::scan_collect::ScanCollectRun;
use crate::error::DbResult;
use crate::sdbql::ast::{BinaryOperator, BodyClause, Expression};
use crate::sdbql::executor::helpers::{compare_values, hash_value, values_equal};
use crate::storage::columnar_blocks::{ColBlock, ColData, Column};

/// The most rows of a partition this keeps: past it the ordinary path,
/// which holds every row anyway.
const MOST: usize = 10_000;

/// One ORDER BY column of a block, compared in its array.
struct Order<'a> {
    column: &'a Column,
    ascending: bool,
}

impl Order<'_> {
    /// As `compare_values` orders the two rows' values, null below every
    /// value, then turned round for DESC.
    fn compare(&self, a: usize, b: usize) -> Ordering {
        let null = |row: usize| self.column.nulls.as_ref().is_some_and(|nulls| nulls[row]);
        let by = match (null(a), null(b)) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => match &self.column.data {
                ColData::Int(values) => values[a].cmp(&values[b]),
                ColData::Float(values) => {
                    values[a].partial_cmp(&values[b]).unwrap_or(Ordering::Equal)
                }
                // The dictionary is sorted: codes order as their strings.
                ColData::Str { codes, .. } => codes[a].cmp(&codes[b]),
                ColData::Bool(values) => values[a].cmp(&values[b]),
                ColData::Json(_) | ColData::Mixed { .. } => Ordering::Equal,
            },
        };
        if self.ascending {
            by
        } else {
            by.reverse()
        }
    }
}

/// A number that tells a row's value of `column` from the others of the
/// block, and whether it is null.
fn local(column: &Column, row: usize) -> (u64, bool) {
    if column.nulls.as_ref().is_some_and(|nulls| nulls[row]) {
        return (0, true);
    }
    let bits = match &column.data {
        ColData::Int(values) => values[row] as u64,
        ColData::Float(values) => values[row].to_bits(),
        ColData::Str { codes, .. } => u64::from(codes[row]),
        ColData::Bool(values) => u64::from(values[row]),
        ColData::Json(_) | ColData::Mixed { .. } => 0,
    };
    (bits, false)
}

/// Whether the arrays compare `column` as the evaluator compares values.
fn typed(column: &Column) -> bool {
    !matches!(column.data, ColData::Json(_) | ColData::Mixed { .. })
}

/// The rows of `block` that can be among the first `most` of their
/// partition: for each partition met in the block, its first `most` rows
/// by `order` then by position. `None` when a column is not one the
/// arrays compare.
fn candidates(
    block: &ColBlock,
    partition: &[String],
    order: &[(String, bool)],
    most: usize,
) -> Option<Vec<u32>> {
    let keys: Vec<&Column> = partition
        .iter()
        .map(|name| block.column(name).filter(|c| typed(c)))
        .collect::<Option<_>>()?;
    let order: Vec<Order<'_>> = order
        .iter()
        .map(|(name, ascending)| {
            block.column(name).filter(|c| typed(c)).map(|column| Order {
                column,
                ascending: *ascending,
            })
        })
        .collect::<Option<_>>()?;
    let mut groups: Vec<Vec<u32>> = Vec::new();
    let mut seen: HashMap<Vec<(u64, bool)>, usize, foldhash::fast::RandomState> =
        HashMap::default();
    let mut scratch: Vec<(u64, bool)> = Vec::with_capacity(keys.len());
    for row in 0..block.len() {
        scratch.clear();
        scratch.extend(keys.iter().map(|key| local(key, row)));
        let group = match seen.get(&scratch) {
            Some(group) => *group,
            None => {
                seen.insert(scratch.clone(), groups.len());
                groups.push(Vec::new());
                groups.len() - 1
            }
        };
        groups[group].push(row as u32);
    }
    let before = |a: &u32, b: &u32| {
        order
            .iter()
            .map(|by| by.compare(*a as usize, *b as usize))
            .find(|by| by.is_ne())
            .unwrap_or_else(|| a.cmp(b))
    };
    let mut kept = Vec::new();
    for mut rows in groups {
        if rows.len() > most {
            rows.select_nth_unstable_by(most - 1, before);
            rows.truncate(most);
        }
        kept.extend(rows);
    }
    Some(kept)
}

/// `expr` as a column of `var`.
fn column_of(expr: &Expression, var: &str, known: &[String]) -> Option<String> {
    let Expression::FieldAccess(base, field) = expr else {
        return None;
    };
    (matches!(base.as_ref(), Expression::Variable(v) if v == var)
        && known.iter().any(|k| k == field))
    .then(|| field.clone())
}

impl QueryExecutor<'_> {
    /// Run `clauses[i]`, a FOR over a columnar collection, with the LET
    /// that numbers its rows and the FILTER that keeps the first of each
    /// partition, from the column arrays; `Ok(None)` when the ordinary
    /// path has to run them (see the module's notes).
    pub(in crate::sdbql::executor) fn try_columnar_top(
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
        let (
            Some(BodyClause::For(for_clause)),
            Some(BodyClause::Let(let_clause)),
            Some(BodyClause::Filter(filter)),
        ) = (clauses.get(i), clauses.get(i + 1), clauses.get(i + 2))
        else {
            return Ok(None);
        };
        let Expression::WindowFunctionCall {
            function,
            arguments,
            over_clause,
        } = &let_clause.expression
        else {
            return Ok(None);
        };
        if !function.eq_ignore_ascii_case("ROW_NUMBER")
            || !arguments.is_empty()
            || over_clause.order_by.is_empty()
        {
            return Ok(None);
        }
        let (var, number) = (for_clause.variable.as_str(), let_clause.variable.as_str());
        let name = for_clause.collection.as_str();
        if var == number
            || for_clause.source_expression.is_some()
            || for_clause.system_time.is_some()
            || for_clause.valid_time.is_some()
            || IndexHint::of(for_clause).is_some()
            || outer.contains_key(name)
        {
            return Ok(None);
        }

        // `n <= k`, `n < k` or `n == 1`, with `k` a constant.
        let Expression::BinaryOp { left, op, right } = &filter.expression else {
            return Ok(None);
        };
        if !matches!(left.as_ref(), Expression::Variable(v) if v == number)
            || expression_references_var(right, var)
            || expression_references_var(right, number)
        {
            return Ok(None);
        }
        let Ok(Value::Number(bound)) = self.evaluate_expr_with_context(right, outer) else {
            return Ok(None);
        };
        let Some(bound) = bound.as_i64() else {
            return Ok(None);
        };
        let most = match op {
            BinaryOperator::LessThanOrEqual => bound,
            BinaryOperator::LessThan => bound - 1,
            BinaryOperator::Equal if bound == 1 => 1,
            _ => return Ok(None),
        };
        if most < 1 || most as usize > MOST {
            return Ok(None);
        }
        let most = most as usize;

        let Some(db_name) = &self.database else {
            return Ok(None);
        };
        let Ok(database) = self.storage.get_database(db_name) else {
            return Ok(None);
        };
        if !self.is_columnar(&database, name) || self.row_policy_applies(name) {
            return Ok(None);
        }
        self.note_columnar_read(&database, name)?;
        let columnar =
            crate::storage::ColumnarCollection::load(name.to_string(), db_name, database.db_arc())?;
        if columnar.larger_than_kept() {
            return Ok(None);
        }
        let known: Vec<String> = columnar
            .metadata()?
            .columns
            .into_iter()
            .map(|c| c.name)
            .collect();
        let Some(partition) = over_clause
            .partition_by
            .iter()
            .map(|expr| column_of(expr, var, &known))
            .collect::<Option<Vec<String>>>()
        else {
            return Ok(None);
        };
        let Some(order) = over_clause
            .order_by
            .iter()
            .map(|(expr, ascending)| column_of(expr, var, &known).map(|c| (c, *ascending)))
            .collect::<Option<Vec<(String, bool)>>>()
        else {
            return Ok(None);
        };

        let set = columnar.row_set()?;
        if !set.in_order() {
            return Ok(None);
        }
        // What each block keeps, then every row of the tail: by position
        // among the rows.
        let mut kept: Vec<(usize, usize)> = Vec::new();
        for (at, block) in set.blocks.iter().enumerate() {
            let Some(rows) = candidates(block, &partition, &order, most) else {
                return Ok(None);
            };
            kept.extend(rows.into_iter().map(|row| (at, row as usize)));
        }
        kept.extend((0..set.tail.len()).map(|row| (set.blocks.len(), row)));
        self.check_budget(kept.len())?;

        // Those rows by partition, as the evaluator tells partitions
        // apart, each ordered as it orders them: by the ORDER BY, then by
        // position.
        struct Candidate {
            at: (usize, usize),
            position: usize,
            order: Vec<Value>,
        }
        let mut keys: Vec<Vec<Value>> = Vec::new();
        let mut members: Vec<Vec<Candidate>> = Vec::new();
        let mut index: HashMap<u64, Vec<usize>, foldhash::fast::RandomState> = HashMap::default();
        for at in kept {
            let key: Vec<Value> = partition.iter().map(|c| set.value(at, c)).collect();
            let hash = key
                .iter()
                .fold(0u64, |hash, value| hash.rotate_left(17) ^ hash_value(value));
            let bucket = index.entry(hash).or_default();
            let same = |held: &Vec<Value>| held.iter().zip(&key).all(|(a, b)| values_equal(a, b));
            let group = match bucket.iter().copied().find(|group| same(&keys[*group])) {
                Some(group) => group,
                None => {
                    bucket.push(keys.len());
                    keys.push(key);
                    members.push(Vec::new());
                    keys.len() - 1
                }
            };
            members[group].push(Candidate {
                at,
                position: set.start(at.0) + at.1,
                order: order.iter().map(|(c, _)| set.value(at, c)).collect(),
            });
        }
        let mut numbered: Vec<((usize, usize), usize, usize)> = Vec::new();
        for mut group in members {
            group.sort_by(|a, b| {
                a.order
                    .iter()
                    .zip(&b.order)
                    .zip(&order)
                    .map(|((x, y), (_, ascending))| {
                        let by = compare_values(x, y);
                        if *ascending {
                            by
                        } else {
                            by.reverse()
                        }
                    })
                    .find(|by| by.is_ne())
                    .unwrap_or_else(|| a.position.cmp(&b.position))
            });
            numbered.extend(
                group
                    .iter()
                    .take(most)
                    .enumerate()
                    .map(|(n, candidate)| (candidate.at, candidate.position, n + 1)),
            );
        }
        // In the order of the collection's rows, as the FILTER leaves them.
        numbered.sort_unstable_by_key(|(_, position, _)| *position);

        // The columns the query reads of the row; all of them when it
        // reads the row whole.
        let projected = self.scan_projection_for(for_clause);
        let columns: Vec<&String> = known
            .iter()
            .filter(|column| {
                projected
                    .as_ref()
                    .is_none_or(|fields| fields.iter().any(|f| &f == column))
            })
            .collect();
        let rows: Vec<Context> = numbered
            .into_iter()
            .map(|(at, _, n)| {
                let row: serde_json::Map<String, Value> = columns
                    .iter()
                    .map(|column| ((*column).clone(), set.value(at, column)))
                    .collect();
                let mut ctx = outer.clone();
                ctx.insert(var.to_string(), Value::Object(row));
                ctx.insert(number.to_string(), Value::Number(n.into()));
                ctx
            })
            .collect();
        crate::storage::columnar::FIRST_ROWS_FROM_ARRAYS
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(Some(ScanCollectRun {
            rows,
            consumed: 3,
            scanned: set.len(),
            filters: Vec::new(),
            note: format!(
                "optimizer: the first {} rows of each partition found in the column arrays of '{}'",
                most, name
            ),
        }))
    }
}
