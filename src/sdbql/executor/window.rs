//! Window function helpers for SDBQL executor.
//!
//! This module contains helper functions for window function processing:
//! - contains_window_functions: Check if expression contains window functions
//! - generate_window_key: Generate unique key for window function
//! - extract_window_functions: Extract window functions from expression

use crate::sdbql::ast::{Expression, TemplateStringPart, WindowSpec};

/// Check if an expression contains window functions
pub fn contains_window_functions(expr: &Expression) -> bool {
    match expr {
        Expression::WindowFunctionCall { .. } => true,
        Expression::Object(fields) => fields.iter().any(|(_, e)| contains_window_functions(e)),
        Expression::Array(elements) => elements.iter().any(contains_window_functions),
        Expression::BinaryOp { left, right, .. } => {
            contains_window_functions(left) || contains_window_functions(right)
        }
        Expression::UnaryOp { operand, .. } => contains_window_functions(operand),
        Expression::Ternary {
            condition,
            true_expr,
            false_expr,
        } => {
            contains_window_functions(condition)
                || contains_window_functions(true_expr)
                || contains_window_functions(false_expr)
        }
        Expression::Case {
            operand,
            when_clauses,
            else_clause,
        } => {
            operand
                .as_ref()
                .map_or(false, |o| contains_window_functions(o))
                || when_clauses
                    .iter()
                    .any(|(c, r)| contains_window_functions(c) || contains_window_functions(r))
                || else_clause
                    .as_ref()
                    .map_or(false, |e| contains_window_functions(e))
        }
        Expression::FunctionCall { args, .. } => args.iter().any(contains_window_functions),
        Expression::FieldAccess(base, _) | Expression::OptionalFieldAccess(base, _) => {
            contains_window_functions(base)
        }
        Expression::ArrayAccess(base, idx) => {
            contains_window_functions(base) || contains_window_functions(idx)
        }
        Expression::Pipeline { left, right } => {
            contains_window_functions(left) || contains_window_functions(right)
        }
        Expression::TemplateString { parts } => parts.iter().any(|p| match p {
            TemplateStringPart::Expression(e) => contains_window_functions(e),
            _ => false,
        }),
        _ => false,
    }
}

/// Generate a unique key for a window function based on its signature
pub fn generate_window_key(
    function: &str,
    arguments: &[Expression],
    over_clause: &WindowSpec,
) -> String {
    // Create a deterministic key from the window function components
    let args_str: String = arguments
        .iter()
        .map(|a| format!("{:?}", a))
        .collect::<Vec<_>>()
        .join(",");
    let partition_str: String = over_clause
        .partition_by
        .iter()
        .map(|p| format!("{:?}", p))
        .collect::<Vec<_>>()
        .join(",");
    let order_str: String = over_clause
        .order_by
        .iter()
        .map(|(e, asc)| format!("{:?}:{}", e, asc))
        .collect::<Vec<_>>()
        .join(",");

    format!(
        "__window_{}_{}_{}_{:x}",
        function.to_uppercase(),
        args_str.len(),
        partition_str.len() + order_str.len(),
        // Simple hash to keep the key shorter
        args_str
            .as_bytes()
            .iter()
            .fold(0u32, |acc, &b| acc.wrapping_add(b as u32))
            + partition_str
                .as_bytes()
                .iter()
                .fold(0u32, |acc, &b| acc.wrapping_add(b as u32))
            + order_str
                .as_bytes()
                .iter()
                .fold(0u32, |acc, &b| acc.wrapping_add(b as u32))
    )
}

/// Extract all window functions from an expression with their assigned variable names
/// Returns: Vec<(var_name, function_name, arguments, WindowSpec)>
pub fn extract_window_functions(
    expr: &Expression,
) -> Vec<(String, String, Vec<Expression>, WindowSpec)> {
    let mut result = Vec::new();
    extract_window_functions_impl(expr, &mut result);
    result
}

fn extract_window_functions_impl(
    expr: &Expression,
    result: &mut Vec<(String, String, Vec<Expression>, WindowSpec)>,
) {
    match expr {
        Expression::WindowFunctionCall {
            function,
            arguments,
            over_clause,
        } => {
            let var_name = generate_window_key(function, arguments, over_clause);
            result.push((
                var_name,
                function.clone(),
                arguments.clone(),
                over_clause.clone(),
            ));
        }
        Expression::Object(fields) => {
            for (_, e) in fields {
                extract_window_functions_impl(e, result);
            }
        }
        Expression::Array(elements) => {
            for e in elements {
                extract_window_functions_impl(e, result);
            }
        }
        Expression::BinaryOp { left, right, .. } => {
            extract_window_functions_impl(left, result);
            extract_window_functions_impl(right, result);
        }
        Expression::UnaryOp { operand, .. } => {
            extract_window_functions_impl(operand, result);
        }
        Expression::Ternary {
            condition,
            true_expr,
            false_expr,
        } => {
            extract_window_functions_impl(condition, result);
            extract_window_functions_impl(true_expr, result);
            extract_window_functions_impl(false_expr, result);
        }
        Expression::Case {
            operand,
            when_clauses,
            else_clause,
        } => {
            if let Some(op) = operand {
                extract_window_functions_impl(op, result);
            }
            for (cond, res) in when_clauses {
                extract_window_functions_impl(cond, result);
                extract_window_functions_impl(res, result);
            }
            if let Some(else_expr) = else_clause {
                extract_window_functions_impl(else_expr, result);
            }
        }
        Expression::FunctionCall { args, .. } => {
            for arg in args {
                extract_window_functions_impl(arg, result);
            }
        }
        Expression::FieldAccess(base, _) | Expression::OptionalFieldAccess(base, _) => {
            extract_window_functions_impl(base, result);
        }
        Expression::ArrayAccess(base, idx) => {
            extract_window_functions_impl(base, result);
            extract_window_functions_impl(idx, result);
        }
        Expression::Pipeline { left, right } => {
            extract_window_functions_impl(left, result);
            extract_window_functions_impl(right, result);
        }
        Expression::TemplateString { parts } => {
            for part in parts {
                if let TemplateStringPart::Expression(e) = part {
                    extract_window_functions_impl(e, result);
                }
            }
        }
        _ => {}
    }
}

// Window function computation
use std::collections::HashMap;

use serde_json::Value;

use super::types::Context;
use super::{compare_values, QueryExecutor};
use crate::error::{DbError, DbResult};

impl<'a> QueryExecutor<'a> {
    pub(super) fn apply_window_functions(
        &self,
        mut rows: Vec<Context>,
        return_expr: &Expression,
    ) -> DbResult<Vec<Context>> {
        // Extract all window functions with unique IDs
        let window_funcs = extract_window_functions(return_expr);

        if window_funcs.is_empty() {
            return Ok(rows);
        }

        for (var_name, func_name, args, spec) in window_funcs {
            // Compute window function for all rows
            let values = self.compute_window_function(&rows, &func_name, &args, &spec)?;

            // Inject computed values into row contexts
            for (row, value) in rows.iter_mut().zip(values.into_iter()) {
                row.insert(var_name.clone(), value);
            }
        }

        Ok(rows)
    }
    pub(super) fn compute_window_function(
        &self,
        rows: &[Context],
        function: &str,
        arguments: &[Expression],
        spec: &WindowSpec,
    ) -> DbResult<Vec<Value>> {
        if rows.is_empty() {
            return Ok(vec![]);
        }

        // Group rows by partition key
        let partitions = self.partition_rows(rows, &spec.partition_by)?;

        let mut results = vec![Value::Null; rows.len()];

        for (_, partition_indices) in partitions {
            // Sort partition by ORDER BY
            let sorted_indices =
                self.sort_partition_indices(rows, &partition_indices, &spec.order_by)?;

            // Compute function for each row in partition
            let partition_values = self.compute_window_in_partition(
                rows,
                &sorted_indices,
                function,
                arguments,
                &spec.order_by,
            )?;

            // Map values back to original row positions
            for (i, original_idx) in sorted_indices.iter().enumerate() {
                results[*original_idx] = partition_values[i].clone();
            }
        }

        Ok(results)
    }
    pub(super) fn partition_rows(
        &self,
        rows: &[Context],
        partition_by: &[Expression],
    ) -> DbResult<HashMap<String, Vec<usize>>> {
        let mut partitions: HashMap<String, Vec<usize>> = HashMap::new();

        for (idx, row) in rows.iter().enumerate() {
            let key = if partition_by.is_empty() {
                String::new() // All rows in single partition
            } else {
                let key_values: Vec<String> = partition_by
                    .iter()
                    .map(|expr| {
                        self.evaluate_expr_with_context(expr, row)
                            .map(|v| serde_json::to_string(&v).unwrap_or_default())
                            .unwrap_or_default()
                    })
                    .collect();
                key_values.join("|")
            };

            partitions.entry(key).or_default().push(idx);
        }

        Ok(partitions)
    }
    pub(super) fn sort_partition_indices(
        &self,
        rows: &[Context],
        indices: &[usize],
        order_by: &[(Expression, bool)],
    ) -> DbResult<Vec<usize>> {
        let mut sorted = indices.to_vec();

        if !order_by.is_empty() {
            sorted.sort_by(|&a, &b| {
                for (expr, ascending) in order_by {
                    let a_val = self
                        .evaluate_expr_with_context(expr, &rows[a])
                        .unwrap_or(Value::Null);
                    let b_val = self
                        .evaluate_expr_with_context(expr, &rows[b])
                        .unwrap_or(Value::Null);

                    let cmp = compare_values(&a_val, &b_val);
                    if cmp != std::cmp::Ordering::Equal {
                        return if *ascending { cmp } else { cmp.reverse() };
                    }
                }
                std::cmp::Ordering::Equal
            });
        }

        Ok(sorted)
    }
    pub(super) fn compute_window_in_partition(
        &self,
        rows: &[Context],
        sorted_indices: &[usize],
        function: &str,
        arguments: &[Expression],
        order_by: &[(Expression, bool)],
    ) -> DbResult<Vec<Value>> {
        let partition_size = sorted_indices.len();
        let mut results = Vec::with_capacity(partition_size);

        match function.to_uppercase().as_str() {
            "ROW_NUMBER" => {
                for i in 0..partition_size {
                    results.push(Value::Number((i + 1).into()));
                }
            }

            "RANK" => {
                // Same ORDER BY value gets same rank, next ranks are skipped
                let mut rank = 1;
                let mut prev_values: Option<Vec<Value>> = None;

                for (i, &idx) in sorted_indices.iter().enumerate() {
                    // Compare using ORDER BY expressions
                    let current_values: Vec<Value> = order_by
                        .iter()
                        .map(|(expr, _)| {
                            self.evaluate_expr_with_context(expr, &rows[idx])
                                .unwrap_or(Value::Null)
                        })
                        .collect();

                    if let Some(ref prev) = prev_values {
                        if current_values != *prev {
                            rank = i + 1;
                        }
                    }

                    results.push(Value::Number(rank.into()));
                    prev_values = Some(current_values);
                }
            }

            "DENSE_RANK" => {
                // Same as RANK but no gaps
                let mut dense_rank = 1;
                let mut prev_values: Option<Vec<Value>> = None;

                for &idx in sorted_indices.iter() {
                    // Compare using ORDER BY expressions
                    let current_values: Vec<Value> = order_by
                        .iter()
                        .map(|(expr, _)| {
                            self.evaluate_expr_with_context(expr, &rows[idx])
                                .unwrap_or(Value::Null)
                        })
                        .collect();

                    if let Some(ref prev) = prev_values {
                        if current_values != *prev {
                            dense_rank += 1;
                        }
                    }

                    results.push(Value::Number(dense_rank.into()));
                    prev_values = Some(current_values);
                }
            }

            "LAG" => {
                let offset = arguments
                    .get(1)
                    .and_then(|arg| {
                        self.evaluate_expr_with_context(arg, &rows[0])
                            .ok()
                            .and_then(|v| v.as_u64())
                    })
                    .unwrap_or(1) as usize;

                let default_val = arguments
                    .get(2)
                    .and_then(|arg| self.evaluate_expr_with_context(arg, &rows[0]).ok())
                    .unwrap_or(Value::Null);

                for (i, &_idx) in sorted_indices.iter().enumerate() {
                    if i >= offset {
                        let prev_idx = sorted_indices[i - offset];
                        let val = arguments
                            .first()
                            .map(|arg| self.evaluate_expr_with_context(arg, &rows[prev_idx]).ok())
                            .flatten()
                            .unwrap_or(Value::Null);
                        results.push(val);
                    } else {
                        results.push(default_val.clone());
                    }
                }
            }

            "LEAD" => {
                let offset = arguments
                    .get(1)
                    .and_then(|arg| {
                        self.evaluate_expr_with_context(arg, &rows[0])
                            .ok()
                            .and_then(|v| v.as_u64())
                    })
                    .unwrap_or(1) as usize;

                let default_val = arguments
                    .get(2)
                    .and_then(|arg| self.evaluate_expr_with_context(arg, &rows[0]).ok())
                    .unwrap_or(Value::Null);

                for (i, &_idx) in sorted_indices.iter().enumerate() {
                    if i + offset < partition_size {
                        let next_idx = sorted_indices[i + offset];
                        let val = arguments
                            .first()
                            .map(|arg| self.evaluate_expr_with_context(arg, &rows[next_idx]).ok())
                            .flatten()
                            .unwrap_or(Value::Null);
                        results.push(val);
                    } else {
                        results.push(default_val.clone());
                    }
                }
            }

            "FIRST_VALUE" => {
                let first_idx = sorted_indices[0];
                let first_val = arguments
                    .first()
                    .map(|arg| self.evaluate_expr_with_context(arg, &rows[first_idx]).ok())
                    .flatten()
                    .unwrap_or(Value::Null);

                for _ in 0..partition_size {
                    results.push(first_val.clone());
                }
            }

            "LAST_VALUE" => {
                // With default frame, LAST_VALUE uses unbounded frame
                let last_idx = sorted_indices[partition_size - 1];
                let last_val = arguments
                    .first()
                    .map(|arg| self.evaluate_expr_with_context(arg, &rows[last_idx]).ok())
                    .flatten()
                    .unwrap_or(Value::Null);

                for _ in 0..partition_size {
                    results.push(last_val.clone());
                }
            }

            // Running aggregates (SUM, AVG, COUNT, MIN, MAX)
            "SUM" | "AVG" | "COUNT" | "MIN" | "MAX" => {
                // Default frame: UNBOUNDED PRECEDING to CURRENT ROW (running totals)
                for (current_pos, _) in sorted_indices.iter().enumerate() {
                    // Collect values from start of partition to current row
                    let frame_values: Vec<Value> = (0..=current_pos)
                        .map(|i| {
                            let idx = sorted_indices[i];
                            arguments
                                .first()
                                .map(|arg| self.evaluate_expr_with_context(arg, &rows[idx]).ok())
                                .flatten()
                                .unwrap_or(Value::Null)
                        })
                        .collect();

                    let result = match function.to_uppercase().as_str() {
                        "SUM" => {
                            let sum: f64 = frame_values.iter().filter_map(|v| v.as_f64()).sum();
                            serde_json::Number::from_f64(sum)
                                .map(Value::Number)
                                .unwrap_or(Value::Null)
                        }
                        "AVG" => {
                            let nums: Vec<f64> =
                                frame_values.iter().filter_map(|v| v.as_f64()).collect();
                            if nums.is_empty() {
                                Value::Null
                            } else {
                                let avg = nums.iter().sum::<f64>() / nums.len() as f64;
                                serde_json::Number::from_f64(avg)
                                    .map(Value::Number)
                                    .unwrap_or(Value::Null)
                            }
                        }
                        "COUNT" => {
                            let count = frame_values.iter().filter(|v| !v.is_null()).count();
                            Value::Number(count.into())
                        }
                        "MIN" => frame_values
                            .into_iter()
                            .filter(|v| !v.is_null())
                            .min_by(|a, b| compare_values(a, b))
                            .unwrap_or(Value::Null),
                        "MAX" => frame_values
                            .into_iter()
                            .filter(|v| !v.is_null())
                            .max_by(|a, b| compare_values(a, b))
                            .unwrap_or(Value::Null),
                        _ => Value::Null,
                    };

                    results.push(result);
                }
            }

            _ => {
                return Err(DbError::ExecutionError(format!(
                    "Unknown window function: {}",
                    function
                )));
            }
        }

        Ok(results)
    }
}
