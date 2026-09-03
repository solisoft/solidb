//! Aggregation functions for SDBQL executor.
//!
//! This module contains aggregation logic:
//! - AggregateAccumulator: streaming COUNT/SUM/AVG/MIN/MAX/... for COLLECT groups
//! - try_columnar_aggregation: Optimized columnar aggregation path

use serde_json::Value;

use super::types::Context;
use super::QueryExecutor;
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
            BodyClause::For(fc) if fc.source_expression.is_none() => fc,
            _ => return Ok(None),
        };

        // Check if collection is columnar
        let collection_name = &for_clause.collection;
        if !database.is_columnar_collection(collection_name) {
            return Ok(None);
        }

        // Second clause must be COLLECT with AGGREGATE
        let collect_clause = match &query.body_clauses[1] {
            BodyClause::Collect(cc) if !cc.aggregates.is_empty() => cc,
            _ => return Ok(None),
        };

        // Must have a return clause
        if query.return_clause.is_none() {
            return Ok(None);
        }

        // Load columnar collection
        let columnar =
            match ColumnarCollection::load(collection_name.clone(), db_name, database.db_arc()) {
                Ok(c) => c,
                Err(_) => return Ok(None),
            };

        // Extract group by columns (from COLLECT var1 = x.field1, var2 = x.field2)
        use crate::storage::columnar::GroupByColumn;

        // Helper to extract grouping definition definition
        let parse_group_expr = |expr: &Expression| -> Option<GroupByColumn> {
            match expr {
                Expression::FieldAccess(base, field) => {
                    if let Expression::Variable(var) = base.as_ref() {
                        if var == &for_clause.variable {
                            return Some(GroupByColumn::Simple(field.clone()));
                        }
                    }
                    None
                }
                Expression::FunctionCall { name, args } if name == "TIME_BUCKET" => {
                    if args.len() == 2 {
                        // Arg 0 must be field access
                        let col = if let Expression::FieldAccess(base, field) = &args[0] {
                            if let Expression::Variable(var) = base.as_ref() {
                                if var == &for_clause.variable {
                                    Some(field.clone())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }?;

                        // Arg 1 must be literal string (interval)
                        let interval = if let Expression::Literal(Value::String(s)) = &args[1] {
                            Some(s.clone())
                        } else {
                            None
                        }?;

                        Some(GroupByColumn::TimeBucket(col, interval))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        };

        let group_defs: Vec<GroupByColumn> = collect_clause
            .group_vars
            .iter()
            .filter_map(|(_, expr)| parse_group_expr(expr))
            .collect();

        // If we couldn't parse all group vars, abort optimization
        if group_defs.len() != collect_clause.group_vars.len() {
            return Ok(None);
        }

        // Process aggregations
        let mut result_obj: serde_json::Map<String, Value> = serde_json::Map::new();
        // Grouped results, merged across aggregates by group key.
        let mut grouped: std::collections::BTreeMap<String, serde_json::Map<String, Value>> =
            std::collections::BTreeMap::new();

        for agg in &collect_clause.aggregates {
            let var_name = &agg.variable;
            let func_name = &agg.function;

            // Extract field from argument
            let field = match &agg.argument {
                Some(Expression::FieldAccess(base, field)) => {
                    if let Expression::Variable(var) = base.as_ref() {
                        if var == &for_clause.variable {
                            field.clone()
                        } else {
                            return Ok(None);
                        }
                    } else {
                        return Ok(None);
                    }
                }
                Some(Expression::Variable(_)) | None => {
                    // COUNT(*) style - use special handling
                    "_count".to_string()
                }
                _ => return Ok(None),
            };

            // Map function name to AggregateOp
            let op = match func_name.to_uppercase().as_str() {
                "SUM" => AggregateOp::Sum,
                "AVG" | "AVERAGE" => AggregateOp::Avg,
                "COUNT" | "LENGTH" => AggregateOp::Count,
                "MIN" | "MINIMUM" => AggregateOp::Min,
                "MAX" | "MAXIMUM" => AggregateOp::Max,
                "COUNT_DISTINCT" | "COUNT_UNIQUE" | "UNIQUE" => AggregateOp::CountDistinct,
                _ => return Ok(None), // Unknown aggregate
            };

            // Execute aggregation
            if group_defs.is_empty() {
                // Simple aggregation without grouping
                match columnar.aggregate(&field, op) {
                    Ok(value) => {
                        result_obj.insert(var_name.clone(), value);
                    }
                    Err(_) => return Ok(None),
                }
            } else {
                // Grouped aggregation. Each aggregate is a separate column-native
                // pass; merge them on the group key.
                //
                // This used to `return` inside the loop with the raw storage
                // rows, which meant every aggregate after the first was
                // silently dropped and the column was reported under storage's
                // internal `_agg` name instead of the COLLECT variable.
                let rows = match columnar.group_by(&group_defs, &field, op) {
                    Ok(rows) => rows,
                    Err(_) => return Ok(None),
                };

                for row in rows {
                    let Some(obj) = row.as_object() else {
                        return Ok(None);
                    };
                    let key = group_key_of(obj, &group_defs);
                    let entry = grouped.entry(key).or_default();

                    // Group columns are keyed by column name; re-key them to the
                    // COLLECT variable, which is what the RETURN clause refers to
                    // (`COLLECT h = m.host` binds `h`, not `host`).
                    for ((collect_var, _), col_def) in
                        collect_clause.group_vars.iter().zip(group_defs.iter())
                    {
                        if let Some(v) = obj.get(col_def.name()) {
                            entry.insert(collect_var.clone(), v.clone());
                        }
                    }
                    if let Some(v) = obj.get("_agg") {
                        entry.insert(var_name.clone(), v.clone());
                    }
                }
            }
        }

        // Build the rows this optimization produces, then run them through the
        // query's RETURN clause. Returning the aggregate rows directly ignored
        // RETURN entirely: `RETURN {sum: total}` came back as `{"total": ...}`
        // and `RETURN total` came back as an object instead of a scalar.
        let rows: Vec<serde_json::Map<String, Value>> = if group_defs.is_empty() {
            vec![result_obj]
        } else {
            grouped.into_values().collect()
        };

        let return_expr = &query
            .return_clause
            .as_ref()
            .expect("checked above")
            .expression;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let mut ctx: Context = _initial_bindings.clone();
            for (k, v) in row {
                ctx.insert(k, v);
            }
            out.push(self.evaluate_expr_with_context(return_expr, &ctx)?);
        }
        Ok(Some(out))
    }
}

/// Running state of one `AGGREGATE` function inside one `COLLECT` group.
///
/// `COLLECT` used to keep every row of every group alive until the end of the
/// clause and then compute each aggregate over that list, so `COLLECT
/// AGGREGATE total = SUM(doc.amount)` over 5M rows held 5M cloned contexts —
/// the intermediate row ceiling counted them once, memory paid for them twice.
/// Folding each row in as it arrives keeps the clause at O(groups) for every
/// function except `COLLECT_LIST`, whose output *is* the rows.
///
/// The fold order is the row order, and the result of each variant is exactly
/// what the list-based computation produced.
pub(super) enum AggregateAccumulator {
    /// `COUNT()` — every row.
    CountAll(i64),
    /// `COUNT(expr)` — rows whose value is not null.
    CountNonNull(i64),
    Sum(f64),
    Avg {
        sum: f64,
        count: i64,
    },
    Min(Option<Value>),
    Max(Option<Value>),
    /// `LENGTH(expr)` / `COUNT_DISTINCT(expr)` — serialised values seen.
    Distinct(std::collections::HashSet<String>),
    /// `COLLECT_LIST(expr)` / `COLLECT(expr)`.
    List(Vec<Value>),
}

impl AggregateAccumulator {
    pub(super) fn new(function: &str, has_argument: bool) -> DbResult<Self> {
        Ok(match function {
            "COUNT" if !has_argument => Self::CountAll(0),
            "COUNT" => Self::CountNonNull(0),
            "SUM" => Self::Sum(0.0),
            "AVG" => Self::Avg { sum: 0.0, count: 0 },
            "MIN" => Self::Min(None),
            "MAX" => Self::Max(None),
            "LENGTH" | "COUNT_DISTINCT" => Self::Distinct(Default::default()),
            "COLLECT_LIST" | "COLLECT" => Self::List(Vec::new()),
            _ => {
                return Err(DbError::ExecutionError(format!(
                    "Unknown aggregate function: {}",
                    function
                )))
            }
        })
    }

    /// Fold one row in. `value` is `None` when the aggregate has no argument,
    /// in which case only `COUNT()` has anything to count.
    ///
    /// Returns whether the value was retained (as opposed to reduced), so the
    /// caller can budget the memory this clause is holding on to.
    pub(super) fn push(&mut self, value: Option<Value>) -> bool {
        match self {
            Self::CountAll(n) => {
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
                if let Some(n) = value.as_ref().and_then(as_number) {
                    *sum += n;
                }
                false
            }
            Self::Avg { sum, count } => {
                if let Some(n) = value.as_ref().and_then(as_number) {
                    *sum += n;
                    *count += 1;
                }
                false
            }
            Self::Min(best) => {
                if let Some(val) = value.filter(|v| !v.is_null()) {
                    if replaces(best.as_ref(), &val, std::cmp::Ordering::Less) {
                        *best = Some(val);
                    }
                }
                false
            }
            Self::Max(best) => {
                if let Some(val) = value.filter(|v| !v.is_null()) {
                    if replaces(best.as_ref(), &val, std::cmp::Ordering::Greater) {
                        *best = Some(val);
                    }
                }
                false
            }
            Self::Distinct(seen) => match value {
                Some(v) => seen.insert(serde_json::to_string(&v).unwrap_or_default()),
                None => false,
            },
            Self::List(list) => match value {
                Some(v) => {
                    list.push(v);
                    true
                }
                None => false,
            },
        }
    }

    pub(super) fn finish(self) -> Value {
        match self {
            Self::CountAll(n) | Self::CountNonNull(n) => Value::Number(n.into()),
            Self::Sum(sum) => float_value(sum),
            Self::Avg { count: 0, .. } => Value::Null,
            Self::Avg { sum, count } => float_value(sum / count as f64),
            Self::Min(v) | Self::Max(v) => v.unwrap_or(Value::Null),
            Self::Distinct(seen) => Value::Number((seen.len() as i64).into()),
            Self::List(list) => Value::Array(list),
        }
    }
}

fn as_number(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_i64().map(|n| n as f64))
}

fn float_value(f: f64) -> Value {
    Value::Number(serde_json::Number::from_f64(f).unwrap_or_else(|| (f as i64).into()))
}

/// MIN/MAX ordering: numbers compare with numbers, strings with strings, and
/// a candidate of another kind than the current best never replaces it. The
/// first non-null value always wins.
fn replaces(current: Option<&Value>, candidate: &Value, want: std::cmp::Ordering) -> bool {
    let Some(current) = current else { return true };
    let ord = if let (Some(cur), Some(new)) = (current.as_f64(), candidate.as_f64()) {
        new.partial_cmp(&cur)
    } else if let (Some(cur), Some(new)) = (current.as_str(), candidate.as_str()) {
        Some(new.cmp(cur))
    } else {
        None
    };
    ord == Some(want)
}

/// Stable identity for a group across per-aggregate passes.
///
/// Each aggregate is computed in its own `group_by` call, so their rows have to
/// be merged. The group columns are what identify a group; `_agg` is the value
/// being merged in and is deliberately excluded.
fn group_key_of(
    obj: &serde_json::Map<String, Value>,
    group_defs: &[crate::storage::columnar::GroupByColumn],
) -> String {
    group_defs
        .iter()
        .map(|d| match obj.get(d.name()) {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\u{1}")
}
