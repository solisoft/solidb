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
//!   (`ColumnarCollection::scan_filtered`: one column read, or an index
//!   on it), and objects are built only for the rows kept. The `FILTER`
//!   clauses still run on those rows afterwards, so what is pushed down
//!   may keep too much but never too little: a comparison is pushed only
//!   when the collection answers it exactly as SDBQL does (a constant of
//!   the column's own kind), and whatever else the `FILTER` holds is left
//!   to it.
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
    pub(super) fn columnar_for_rows(
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
        let mut pushed: Vec<ColumnFilter> = conjuncts
            .into_iter()
            .filter_map(|conjunct| self.pushed_term(variable, conjunct, schema, ctx))
            .collect();
        match pushed.len() {
            0 => None,
            1 => pushed.pop(),
            _ => Some(ColumnFilter::And(pushed)),
        }
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
                branches.push(self.pushed_term(variable, side, schema, ctx)?);
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
        // Only where both sides are of one kind: the collection compares an
        // equality as JSON values, to which `3` and `3.0` differ.
        let exact = match (kind, &value) {
            (ColumnType::String, Value::String(_)) => op == BinaryOperator::Equal,
            (ColumnType::Bool, Value::Bool(_)) => op == BinaryOperator::Equal,
            (ColumnType::Int64, Value::Number(n)) => n.is_i64() || n.is_u64(),
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
