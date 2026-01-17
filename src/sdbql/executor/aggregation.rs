//! Aggregation functions for SDBQL executor.
//!
//! This module contains aggregation logic:
//! - compute_aggregate: Compute aggregate functions (COUNT, SUM, AVG, etc.)
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
                // Group by aggregation
                match columnar.group_by(&group_defs, &field, op) {
                    Ok(grouped_results) => {
                        // For group by, we need to return an array
                        return Ok(Some(grouped_results));
                    }
                    Err(_) => return Ok(None),
                }
            }
        }

        // Return single result object
        Ok(Some(vec![Value::Object(result_obj)]))
    }

    /// Execute query and return results only (backwards compatible)
    pub(super) fn compute_aggregate(
        &self,
        function: &str,
        argument: &Option<Expression>,
        group_docs: &[Context],
    ) -> DbResult<Value> {
        match function {
            "COUNT" => {
                if argument.is_none() {
                    // COUNT() - count all rows
                    Ok(Value::Number((group_docs.len() as i64).into()))
                } else {
                    // COUNT(expr) - count non-null values
                    let mut count = 0i64;
                    for ctx in group_docs {
                        if let Some(expr) = argument {
                            let val = self.evaluate_expr_with_context(expr, ctx)?;
                            if !val.is_null() {
                                count += 1;
                            }
                        }
                    }
                    Ok(Value::Number(count.into()))
                }
            }
            "SUM" => {
                let mut sum = 0.0f64;
                if let Some(expr) = argument {
                    for ctx in group_docs {
                        let val = self.evaluate_expr_with_context(expr, ctx)?;
                        if let Some(n) = val.as_f64() {
                            sum += n;
                        } else if let Some(n) = val.as_i64() {
                            sum += n as f64;
                        }
                    }
                }
                Ok(Value::Number(
                    serde_json::Number::from_f64(sum).unwrap_or_else(|| (sum as i64).into()),
                ))
            }
            "AVG" => {
                let mut sum = 0.0f64;
                let mut count = 0i64;
                if let Some(expr) = argument {
                    for ctx in group_docs {
                        let val = self.evaluate_expr_with_context(expr, ctx)?;
                        if let Some(n) = val.as_f64() {
                            sum += n;
                            count += 1;
                        } else if let Some(n) = val.as_i64() {
                            sum += n as f64;
                            count += 1;
                        }
                    }
                }
                if count == 0 {
                    Ok(Value::Null)
                } else {
                    let avg = sum / (count as f64);
                    Ok(Value::Number(
                        serde_json::Number::from_f64(avg).unwrap_or_else(|| (avg as i64).into()),
                    ))
                }
            }
            "MIN" => {
                let mut min: Option<Value> = None;
                if let Some(expr) = argument {
                    for ctx in group_docs {
                        let val = self.evaluate_expr_with_context(expr, ctx)?;
                        if val.is_null() {
                            continue;
                        }

                        if min.is_none() {
                            min = Some(val);
                        } else if let (Some(cur), Some(new)) =
                            (min.as_ref().and_then(|v| v.as_f64()), val.as_f64())
                        {
                            if new < cur {
                                min = Some(val);
                            }
                        } else if let (Some(cur_str), Some(new_str)) =
                            (min.as_ref().and_then(|v| v.as_str()), val.as_str())
                        {
                            if new_str < cur_str {
                                min = Some(val);
                            }
                        }
                    }
                }
                Ok(min.unwrap_or(Value::Null))
            }
            "MAX" => {
                let mut max: Option<Value> = None;
                if let Some(expr) = argument {
                    for ctx in group_docs {
                        let val = self.evaluate_expr_with_context(expr, ctx)?;
                        if val.is_null() {
                            continue;
                        }

                        if max.is_none() {
                            max = Some(val);
                        } else if let (Some(cur), Some(new)) =
                            (max.as_ref().and_then(|v| v.as_f64()), val.as_f64())
                        {
                            if new > cur {
                                max = Some(val);
                            }
                        } else if let (Some(cur_str), Some(new_str)) =
                            (max.as_ref().and_then(|v| v.as_str()), val.as_str())
                        {
                            if new_str > cur_str {
                                max = Some(val);
                            }
                        }
                    }
                }
                Ok(max.unwrap_or(Value::Null))
            }
            "LENGTH" | "COUNT_DISTINCT" => {
                use std::collections::HashSet;
                let mut seen: HashSet<String> = HashSet::new();
                if let Some(expr) = argument {
                    for ctx in group_docs {
                        let val = self.evaluate_expr_with_context(expr, ctx)?;
                        seen.insert(serde_json::to_string(&val).unwrap_or_default());
                    }
                }
                Ok(Value::Number((seen.len() as i64).into()))
            }
            "COLLECT_LIST" | "COLLECT" => {
                let mut list = Vec::new();
                if let Some(expr) = argument {
                    for ctx in group_docs {
                        let val = self.evaluate_expr_with_context(expr, ctx)?;
                        list.push(val);
                    }
                }
                Ok(Value::Array(list))
            }
            _ => Err(DbError::ExecutionError(format!(
                "Unknown aggregate function: {}",
                function
            ))),
        }
    }
}
