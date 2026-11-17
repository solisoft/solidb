//! Rows of a columnar collection read for a `FOR`, without building what
//! the query does not need.
//!
//! As a `FOR` source a columnar collection used to be read whole: every
//! row, with every column, was built as an object before a `FILTER` ran or
//! a `LIMIT` cut (card #891). Here:
//!
//! - **Projection.** Only the columns the query reads of the row are
//!   built (the projection recorded for the scan, as for a document
//!   collection).
//! - **Filter.** The `FILTER`s that follow the `FOR` and compare a column
//!   with a constant are answered by the collection first
//!   (`ColumnarCollection::scan_filtered`: the arrays of the blocks,
//!   passing over a block its bounds rule out, or an index for an
//!   equality), and objects are built only for the rows kept. The `FILTER`
//!   clauses still run on those rows afterwards, so what is pushed down
//!   may keep too much but never too little: a comparison is pushed only
//!   when the collection answers it exactly as SDBQL does (an equality
//!   with a constant of the column's own kind, a range of a numeric
//!   column against a number), and whatever else the `FILTER` holds is
//!   left to it.
//! - **Limit.** A scan with a limit and no filter stops at the limit.

use serde_json::Value;

use super::super::index_opt::push_conjuncts;
use super::super::types::Context;
use super::super::QueryExecutor;
use crate::error::DbResult;
use crate::sdbql::ast::{BinaryOperator, Expression, ForClause};
use crate::storage::columnar::{ColumnDef, ColumnFilter, ColumnType};

impl QueryExecutor<'_> {
    /// The rows `for_clause` reads when its source is a columnar
    /// collection, `None` otherwise. `filters` are the `FILTER`s right
    /// after it, which the caller still applies; `limit` is the scan's.
    pub(in crate::sdbql::executor) fn columnar_for_rows(
        &self,
        for_clause: &ForClause,
        filters: &[&Expression],
        ctx: &Context,
        limit: Option<usize>,
    ) -> DbResult<Option<Vec<Value>>> {
        let name = for_clause.collection.as_str();
        let Some(columnar) = self.columnar_source(name)? else {
            return Ok(None);
        };
        let meta = columnar.metadata()?;

        // A row policy reads the row, whatever the query reads of it:
        // whole rows, those the policy keeps, and the limit after it.
        if let Some(gate) = self.row_policy_gate(name) {
            let columns: Vec<&str> = meta.columns.iter().map(|c| c.name.as_str()).collect();
            let pushed = self.pushed_filter(&for_clause.variable, filters, &meta.columns, ctx);
            let rows = match pushed {
                Some(filter) => columnar.scan_filtered(&filter, &columns)?,
                None => columnar.read_columns_limited(&columns, None)?,
            };
            let mut rows: Vec<Value> = rows
                .into_iter()
                .filter(|row| self.row_policy_allows(&gate, row, ctx))
                .collect();
            if let Some(limit) = limit {
                rows.truncate(limit);
            }
            return Ok(Some(rows));
        }

        // The columns the query reads of the row; all of them when it
        // reads the row whole.
        let projected = self.scan_projection_for(for_clause);
        let columns: Vec<&str> = meta
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .filter(|column| {
                projected
                    .as_ref()
                    .is_none_or(|fields| fields.iter().any(|f| f == column))
            })
            .collect();

        // A limit cuts the scan before the FILTERs run: nothing is pushed
        // down then, or the limit would count rows the scan never gave.
        let pushed = match limit {
            Some(_) => None,
            None => self.pushed_filter(&for_clause.variable, filters, &meta.columns, ctx),
        };
        let rows = match pushed {
            Some(filter) => columnar.scan_filtered(&filter, &columns)?,
            None => columnar.read_columns_limited(&columns, limit)?,
        };
        Ok(Some(rows))
    }

    /// Whether `name` is a columnar collection of the query's database.
    pub(in crate::sdbql::executor) fn is_columnar_name(&self, name: &str) -> bool {
        self.database
            .as_ref()
            .and_then(|db| self.storage.get_database(db).ok())
            .is_some_and(|database| self.is_columnar(&database, name))
    }

    /// What EXPLAIN says of a `FOR` over a columnar collection, and the
    /// indexes that read uses: `None` when the source is not columnar.
    /// Follows `columnar_for_rows`.
    pub(in crate::sdbql::executor) fn columnar_note(
        &self,
        for_clause: &ForClause,
        filters: &[&Expression],
        ctx: &Context,
        limit: Option<usize>,
        projected: Option<&[String]>,
    ) -> DbResult<Option<(String, Vec<String>)>> {
        let name = for_clause.collection.as_str();
        let Some(columnar) = self.columnar_source(name)? else {
            return Ok(None);
        };
        let meta = columnar.metadata()?;
        // `projected`: the fields the run reads of the row (EXPLAIN does
        // not record a projection, and builds whole rows).
        let read = match projected {
            Some(fields) => meta
                .columns
                .iter()
                .filter(|c| fields.contains(&c.name))
                .count(),
            None => meta.columns.len(),
        };
        let mut note = format!(
            "`{name}` is columnar: {read} of its {} columns are built",
            meta.columns.len()
        );
        let mut indexes = Vec::new();
        let pushed = match limit {
            Some(_) => None,
            None => self.pushed_filter(&for_clause.variable, filters, &meta.columns, ctx),
        };
        match (pushed, limit) {
            (Some(filter), _) => {
                let mut columns = Vec::new();
                let mut leaves = vec![&filter];
                while let Some(leaf) = leaves.pop() {
                    let column = match leaf {
                        ColumnFilter::And(sides) | ColumnFilter::Or(sides) => {
                            leaves.extend(sides);
                            continue;
                        }
                        ColumnFilter::Eq(column, _)
                        | ColumnFilter::Ne(column, _)
                        | ColumnFilter::Gt(column, _)
                        | ColumnFilter::Gte(column, _)
                        | ColumnFilter::Lt(column, _)
                        | ColumnFilter::Lte(column, _)
                        | ColumnFilter::In(column, _) => column,
                    };
                    if !columns.contains(column) {
                        columns.push(column.clone());
                    }
                    if let Some(kind) = columnar.index_answering(leaf) {
                        let index = format!("{column} ({kind:?})").to_lowercase();
                        if !indexes.contains(&index) {
                            indexes.push(index);
                        }
                    }
                }
                columns.sort();
                indexes.sort();
                note.push_str(&format!(
                    ", for the rows its FILTER on {} keeps, found in the column arrays",
                    columns
                        .iter()
                        .map(|c| format!("`{c}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                if !indexes.is_empty() {
                    note.push_str(&format!(" and the index on {}", indexes.join(", ")));
                }
            }
            (None, Some(limit)) => note.push_str(&format!(
                ", for the first {limit} rows: the scan stops there"
            )),
            (None, None) => {
                note.push_str(", for every row: no FILTER is the collection's to answer")
            }
        }
        Ok(Some((note, indexes)))
    }

    /// What of `filters` the collection can answer: the conjuncts that
    /// compare a column of `variable` with a constant of its kind.
    fn pushed_filter(
        &self,
        variable: &str,
        filters: &[&Expression],
        schema: &[ColumnDef],
        ctx: &Context,
    ) -> Option<ColumnFilter> {
        let mut conjuncts = Vec::new();
        for filter in filters {
            push_conjuncts(filter, &mut conjuncts);
        }
        joined(
            conjuncts
                .into_iter()
                .filter_map(|conjunct| self.pushed_term(variable, conjunct, schema, ctx))
                .collect(),
        )
    }

    /// How many rows `FOR v IN c FILTER filter` gives when `c` is a
    /// columnar collection and the collection answers the whole FILTER:
    /// counted in the arrays, no row built. `None` when it is not
    /// columnar, or when one conjunct is not the collection's to answer
    /// (the count would then be of too many rows).
    pub(in crate::sdbql::executor) fn columnar_count(
        &self,
        for_clause: &ForClause,
        filter: &Expression,
        ctx: &Context,
    ) -> DbResult<Option<u64>> {
        let name = for_clause.collection.as_str();
        // A LET-bound array of that name is iterated, not the collection;
        // under a row policy the rows are read, for the policy to see.
        if ctx.contains_key(name) || self.row_policy_applies(name) {
            return Ok(None);
        }
        let Some(columnar) = self.columnar_source(name)? else {
            return Ok(None);
        };
        let meta = columnar.metadata()?;
        let mut conjuncts = Vec::new();
        push_conjuncts(filter, &mut conjuncts);
        let mut terms = Vec::with_capacity(conjuncts.len());
        for conjunct in conjuncts {
            let Some(term) = self.pushed_term(&for_clause.variable, conjunct, &meta.columns, ctx)
            else {
                return Ok(None);
            };
            terms.push(term);
        }
        let Some(filter) = joined(terms) else {
            return Ok(None);
        };
        Ok(Some(columnar.count_filtered(&filter)?))
    }

    /// One conjunct as a filter of the collection, when it is a comparison
    /// the collection answers as SDBQL does, or an `OR` of such.
    fn pushed_term(
        &self,
        variable: &str,
        term: &Expression,
        schema: &[ColumnDef],
        ctx: &Context,
    ) -> Option<ColumnFilter> {
        let Expression::BinaryOp { left, op, right } = term else {
            return None;
        };
        if *op == BinaryOperator::Or {
            // Every branch, or none: a branch left out would lose rows.
            let mut branches = Vec::new();
            for side in [left, right] {
                branches.push(or_null(self.pushed_term(variable, side, schema, ctx)?));
            }
            return Some(ColumnFilter::Or(branches));
        }
        // `column <op> constant`, or the same the other way round.
        let (column, constant, op) = match (column_of(variable, left), column_of(variable, right)) {
            (Some(column), None) => (column, right, op.clone()),
            (None, Some(column)) => (column, left, mirrored(op)?),
            _ => return None,
        };
        if !matches!(
            **constant,
            Expression::Literal(_) | Expression::BindVariable(_)
        ) {
            return None;
        }
        let value = self.evaluate_expr_with_context(constant, ctx).ok()?;
        let kind = &schema.iter().find(|c| c.name == column)?.data_type;
        // Only where the collection answers as SDBQL does. An equality is
        // compared as JSON values, to which `3` and `3.0` differ: both
        // sides of one kind. A range compares two integers as integers,
        // and a float with a number as floats, which is exact while the
        // bound is one a float holds (up to 2^53).
        let exact = match (kind, &value) {
            (ColumnType::String, Value::String(_)) => op == BinaryOperator::Equal,
            (ColumnType::Bool, Value::Bool(_)) => op == BinaryOperator::Equal,
            (ColumnType::Int64, Value::Number(n)) => n.as_i64().is_some(),
            (ColumnType::Float64, Value::Number(n)) => {
                op != BinaryOperator::Equal
                    && n.as_f64()
                        .is_some_and(|bound| bound.abs() <= 9_007_199_254_740_992.0)
            }
            _ => false,
        };
        if !exact {
            return None;
        }
        let column = column.to_string();
        Some(match op {
            BinaryOperator::Equal => ColumnFilter::Eq(column, value),
            BinaryOperator::GreaterThan => ColumnFilter::Gt(column, value),
            BinaryOperator::GreaterThanOrEqual => ColumnFilter::Gte(column, value),
            BinaryOperator::LessThan => ColumnFilter::Lt(column, value),
            BinaryOperator::LessThanOrEqual => ColumnFilter::Lte(column, value),
            _ => return None,
        })
    }
}

/// The conjuncts the collection answers, as one filter.
fn joined(pushed: Vec<ColumnFilter>) -> Option<ColumnFilter> {
    // A column another conjunct bounds from below, or compares with a
    // value, holds no null among the rows kept: its `<` stays a plain
    // range, which lets a range bounded on both sides pass over the
    // blocks outside it.
    let bounded: Vec<String> = pushed
        .iter()
        .filter_map(|term| match term {
            ColumnFilter::Gt(column, _)
            | ColumnFilter::Gte(column, _)
            | ColumnFilter::Eq(column, _) => Some(column.clone()),
            _ => None,
        })
        .collect();
    let mut pushed: Vec<ColumnFilter> = pushed
        .into_iter()
        .map(|term| match term.column() {
            Some(column) if bounded.iter().any(|b| b == column) => term,
            _ => or_null(term),
        })
        .collect();
    match pushed.len() {
        0 => None,
        1 => pushed.pop(),
        _ => Some(ColumnFilter::And(pushed)),
    }
}

/// A `<` or `<=` with the rows holding null beside it: to SDBQL null is
/// below every number, so those comparisons keep them, and the collection's
/// ranges keep numbers only. Any other filter as it is.
fn or_null(filter: ColumnFilter) -> ColumnFilter {
    match &filter {
        ColumnFilter::Lt(column, _) | ColumnFilter::Lte(column, _) => {
            let null = ColumnFilter::Eq(column.clone(), Value::Null);
            ColumnFilter::Or(vec![filter, null])
        }
        _ => filter,
    }
}

/// `variable.column`, as its column.
fn column_of<'e>(variable: &str, expr: &'e Expression) -> Option<&'e str> {
    match expr {
        Expression::FieldAccess(base, column) => match base.as_ref() {
            Expression::Variable(name) if name == variable => Some(column.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// The comparison with its sides swapped.
fn mirrored(op: &BinaryOperator) -> Option<BinaryOperator> {
    Some(match op {
        BinaryOperator::Equal => BinaryOperator::Equal,
        BinaryOperator::GreaterThan => BinaryOperator::LessThan,
        BinaryOperator::GreaterThanOrEqual => BinaryOperator::LessThanOrEqual,
        BinaryOperator::LessThan => BinaryOperator::GreaterThan,
        BinaryOperator::LessThanOrEqual => BinaryOperator::GreaterThanOrEqual,
        _ => return None,
    })
}
