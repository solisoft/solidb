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
                .is_some_and(|o| contains_window_functions(o))
                || when_clauses
                    .iter()
                    .any(|(c, r)| contains_window_functions(c) || contains_window_functions(r))
                || else_clause
                    .as_ref()
                    .is_some_and(|e| contains_window_functions(e))
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

/// Generate a unique key for a window function based on its signature.
///
/// The key must differ whenever the call differs: two window calls that share
/// a key share one set of computed values. It used to be the function name,
/// two lengths and a byte sum of the Debug text, so `SUM(x.ab)` and
/// `SUM(x.ba)` — or swapped PARTITION BY and ORDER BY expressions — collided
/// and the second call silently returned the first one's results. Now it is
/// the full Debug text of the arguments and the window spec, each prefixed
/// with its length so the concatenation cannot be read two ways.
pub fn generate_window_key(
    function: &str,
    arguments: &[Expression],
    over_clause: &WindowSpec,
) -> String {
    let args = format!("{:?}", arguments);
    let spec = format!("{:?}", over_clause);
    format!(
        "__window_{}|{}:{}|{}:{}",
        function.to_uppercase(),
        args.len(),
        args,
        spec.len(),
        spec
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
use std::hash::{Hash, Hasher};

use serde_json::Value;

use super::builtins::array::as_int;
use super::types::Context;
use super::{compare_values, hash_value, values_equal, QueryExecutor};
use crate::error::{DbError, DbResult};

/// How often the per-row window loops consult the execution budget (same
/// cadence as the row-building loops in `execution::clauses`).
const WINDOW_BUDGET_INTERVAL: usize = 4096;

/// A partition in window order: each row's index into the input and its
/// ORDER BY key.
type SortedPartition = Vec<(usize, Vec<Value>)>;

fn keys_equal(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| values_equal(x, y))
}

/// Peers share an ORDER BY key under `compare_values` — the same test the
/// sort used, so ranks agree with the order rows came out in. (RANK used to
/// compare keys with `!=`, which split `1` from `1.0`.)
fn peers(a: &[Value], b: &[Value]) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| compare_values(x, y) == std::cmp::Ordering::Equal)
}

fn float(f: f64) -> Value {
    serde_json::Number::from_f64(f)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

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

        let mut done: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (var_name, func_name, args, spec) in window_funcs {
            // The same call written twice has one key; compute it once.
            if !done.insert(var_name.clone()) {
                continue;
            }
            let values = self.compute_window_function(&rows, &func_name, &args, &spec)?;

            // Inject computed values into row contexts
            for (row, value) in rows.iter_mut().zip(values) {
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

        let partitions = self.partition_rows(rows, &spec.partition_by)?;
        let mut results = vec![Value::Null; rows.len()];

        for partition_indices in partitions {
            let sorted = self.sort_partition(rows, &partition_indices, &spec.order_by)?;
            let partition_values = self.compute_window_in_partition(
                rows,
                &sorted,
                function,
                arguments,
                !spec.order_by.is_empty(),
            )?;
            for ((original_idx, _), value) in sorted.iter().zip(partition_values) {
                results[*original_idx] = value;
            }
        }

        Ok(results)
    }

    /// Group row indices by PARTITION BY key (hash of the key values,
    /// confirmed with `values_equal`), in first-seen order.
    pub(super) fn partition_rows(
        &self,
        rows: &[Context],
        partition_by: &[Expression],
    ) -> DbResult<Vec<Vec<usize>>> {
        if partition_by.is_empty() {
            return Ok(vec![(0..rows.len()).collect()]);
        }
        let mut keys: Vec<Vec<Value>> = Vec::new();
        let mut members: Vec<Vec<usize>> = Vec::new();
        let mut index: HashMap<u64, Vec<usize>> = HashMap::new();

        for (idx, row) in rows.iter().enumerate() {
            if idx % WINDOW_BUDGET_INTERVAL == 0 {
                self.check_budget(rows.len())?;
            }
            let key = partition_by
                .iter()
                .map(|expr| self.evaluate_expr_with_context(expr, row))
                .collect::<DbResult<Vec<_>>>()?;
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for v in &key {
                hash_value(v).hash(&mut h);
            }
            let bucket = index.entry(h.finish()).or_default();
            let found = bucket.iter().copied().find(|&p| keys_equal(&keys[p], &key));
            match found {
                Some(p) => members[p].push(idx),
                None => {
                    bucket.push(keys.len());
                    keys.push(key);
                    members.push(vec![idx]);
                }
            }
        }
        Ok(members)
    }

    /// Order a partition by ORDER BY, keeping each row's key for the peer
    /// tests the ranking functions need.
    pub(super) fn sort_partition(
        &self,
        rows: &[Context],
        indices: &[usize],
        order_by: &[(Expression, bool)],
    ) -> DbResult<SortedPartition> {
        // Audit P4: evaluate each row's ORDER BY key once, not twice per
        // comparison. `sort_by` is stable either way, so ties keep their order.
        let mut keyed: SortedPartition = Vec::with_capacity(indices.len());
        for (n, &idx) in indices.iter().enumerate() {
            if n % WINDOW_BUDGET_INTERVAL == 0 {
                self.check_budget(rows.len())?;
            }
            let key = order_by
                .iter()
                .map(|(expr, _)| self.evaluate_expr_with_context(expr, &rows[idx]))
                .collect::<DbResult<Vec<_>>>()?;
            keyed.push((idx, key));
        }

        if !order_by.is_empty() {
            keyed.sort_by(|(_, a), (_, b)| {
                for ((a_val, b_val), (_, ascending)) in a.iter().zip(b).zip(order_by) {
                    let cmp = compare_values(a_val, b_val);
                    if cmp != std::cmp::Ordering::Equal {
                        return if *ascending { cmp } else { cmp.reverse() };
                    }
                }
                std::cmp::Ordering::Equal
            });
        }
        Ok(keyed)
    }

    /// Evaluate argument `i` of a window call against one row; a missing
    /// argument is null.
    fn window_arg(&self, arguments: &[Expression], i: usize, row: &Context) -> DbResult<Value> {
        match arguments.get(i) {
            Some(arg) => self.evaluate_expr_with_context(arg, row),
            None => Ok(Value::Null),
        }
    }

    /// Read a constant integer argument (an offset, a bucket count, an
    /// index), evaluated against the partition's first row.
    fn window_int_arg(
        &self,
        function: &str,
        arguments: &[Expression],
        i: usize,
        row: &Context,
        default: i64,
    ) -> DbResult<i64> {
        match arguments.get(i) {
            None => Ok(default),
            Some(arg) => {
                let v = self.evaluate_expr_with_context(arg, row)?;
                if v.is_null() {
                    return Ok(default);
                }
                as_int(&v).ok_or_else(|| {
                    DbError::ExecutionError(format!(
                        "{}: argument {} must be an integer",
                        function,
                        i + 1
                    ))
                })
            }
        }
    }

    /// Compute one window function over one partition, already in window
    /// order.
    ///
    /// Frames: with an ORDER BY, the aggregates (SUM, AVG, COUNT, MIN, MAX)
    /// run from the partition start to the current row; without one they
    /// cover the whole partition, as in SQL — they used to be running totals
    /// in whatever order rows arrived. FIRST_VALUE, LAST_VALUE and NTH_VALUE
    /// always look at the whole partition (as documented for LAST_VALUE).
    pub(super) fn compute_window_in_partition(
        &self,
        rows: &[Context],
        sorted: &[(usize, Vec<Value>)],
        function: &str,
        arguments: &[Expression],
        has_order: bool,
    ) -> DbResult<Vec<Value>> {
        let n = sorted.len();
        let mut results = Vec::with_capacity(n);
        if n == 0 {
            return Ok(results);
        }
        let first_row = &rows[sorted[0].0];

        match function.to_uppercase().as_str() {
            "ROW_NUMBER" => {
                for i in 0..n {
                    results.push(Value::Number((i + 1).into()));
                }
            }

            f @ ("RANK" | "DENSE_RANK" | "PERCENT_RANK" | "CUME_DIST") => {
                // Peer groups: [start, end) runs of equal ORDER BY keys.
                let mut groups: Vec<(usize, usize)> = Vec::new();
                let mut start = 0;
                for i in 1..=n {
                    if i == n || !peers(&sorted[i].1, &sorted[start].1) {
                        groups.push((start, i));
                        start = i;
                    }
                }
                for (dense, &(start, end)) in groups.iter().enumerate() {
                    let rank = start + 1;
                    let value = match f {
                        "RANK" => Value::Number(rank.into()),
                        "DENSE_RANK" => Value::Number((dense + 1).into()),
                        "PERCENT_RANK" => {
                            if n == 1 {
                                float(0.0)
                            } else {
                                float((rank - 1) as f64 / (n - 1) as f64)
                            }
                        }
                        // CUME_DIST: rows up to and including the last peer.
                        _ => float(end as f64 / n as f64),
                    };
                    results.extend(std::iter::repeat_n(value, end - start));
                }
            }

            "NTILE" => {
                let buckets = self.window_int_arg("NTILE", arguments, 0, first_row, 0)?;
                if buckets <= 0 {
                    return Err(DbError::ExecutionError(
                        "NTILE: bucket count must be a positive integer".to_string(),
                    ));
                }
                // SQL: sizes differ by at most one, larger buckets first.
                let buckets = usize::try_from(buckets).unwrap_or(usize::MAX).min(n);
                let base = n / buckets;
                let extra = n % buckets;
                for b in 0..buckets {
                    let size = base + usize::from(b < extra);
                    results.extend(std::iter::repeat_n(Value::Number((b + 1).into()), size));
                }
            }

            f @ ("LAG" | "LEAD") => {
                let offset = self.window_int_arg(f, arguments, 1, first_row, 1)?;
                if offset < 0 {
                    return Err(DbError::ExecutionError(format!(
                        "{}: offset must not be negative",
                        f
                    )));
                }
                let offset = usize::try_from(offset).unwrap_or(usize::MAX);
                let default_val = self.window_arg(arguments, 2, first_row)?;

                for i in 0..n {
                    let target = if f == "LAG" {
                        i.checked_sub(offset)
                    } else {
                        i.checked_add(offset).filter(|&t| t < n)
                    };
                    match target {
                        Some(t) => {
                            results.push(self.window_arg(arguments, 0, &rows[sorted[t].0])?)
                        }
                        None => results.push(default_val.clone()),
                    }
                }
            }

            f @ ("FIRST_VALUE" | "LAST_VALUE" | "NTH_VALUE") => {
                let pos = match f {
                    "FIRST_VALUE" => Some(0),
                    "LAST_VALUE" => Some(n - 1),
                    _ => {
                        let nth = self.window_int_arg("NTH_VALUE", arguments, 1, first_row, 0)?;
                        if nth < 1 {
                            return Err(DbError::ExecutionError(
                                "NTH_VALUE: position must be a positive integer (1-based)"
                                    .to_string(),
                            ));
                        }
                        usize::try_from(nth - 1).ok().filter(|&p| p < n)
                    }
                };
                let value = match pos {
                    Some(p) => self.window_arg(arguments, 0, &rows[sorted[p].0])?,
                    None => Value::Null,
                };
                results.resize(n, value);
            }

            // Running aggregates (SUM, AVG, COUNT, MIN, MAX).
            //
            // Audit P4: one pass with running accumulators. Re-reducing the
            // whole frame for every row was O(n²) evaluations — ~5×10¹¹ for a
            // cumulative SUM over 1M rows — and never looked at the deadline.
            // MIN keeps the first minimum, MAX the last maximum.
            agg @ ("SUM" | "AVG" | "COUNT" | "MIN" | "MAX") => {
                let mut sum = 0.0f64;
                let mut numeric = 0usize;
                let mut non_null = 0usize;
                let mut extreme: Option<Value> = None;

                let current = |sum: f64,
                               numeric: usize,
                               non_null: usize,
                               extreme: &Option<Value>| {
                    match agg {
                        "SUM" => float(sum),
                        "AVG" => {
                            if numeric == 0 {
                                Value::Null
                            } else {
                                float(sum / numeric as f64)
                            }
                        }
                        "COUNT" => Value::Number(non_null.into()),
                        _ => extreme.clone().unwrap_or(Value::Null),
                    }
                };

                for (pos, (idx, _)) in sorted.iter().enumerate() {
                    if pos % WINDOW_BUDGET_INTERVAL == 0 {
                        self.check_budget(rows.len())?;
                    }
                    let value = self.window_arg(arguments, 0, &rows[*idx])?;

                    if let Some(x) = value.as_f64() {
                        sum += x;
                        numeric += 1;
                    }
                    if !value.is_null() {
                        non_null += 1;
                        if agg == "MIN" || agg == "MAX" {
                            let replace = match &extreme {
                                None => true,
                                Some(cur) => {
                                    let cmp = compare_values(&value, cur);
                                    if agg == "MIN" {
                                        cmp == std::cmp::Ordering::Less
                                    } else {
                                        cmp != std::cmp::Ordering::Less
                                    }
                                }
                            };
                            if replace {
                                extreme = Some(value);
                            }
                        }
                    }

                    if has_order {
                        results.push(current(sum, numeric, non_null, &extreme));
                    }
                }
                if !has_order {
                    results.resize(n, current(sum, numeric, non_null, &extreme));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdbql::ast::Expression;

    fn field(var: &str, f: &str) -> Expression {
        Expression::FieldAccess(
            Box::new(Expression::Variable(var.to_string())),
            f.to_string(),
        )
    }

    #[test]
    fn window_keys_do_not_collide() {
        let spec = WindowSpec {
            partition_by: vec![],
            order_by: vec![(field("x", "d"), true)],
        };
        let ab = generate_window_key("SUM", &[field("x", "ab")], &spec);
        let ba = generate_window_key("SUM", &[field("x", "ba")], &spec);
        assert_ne!(ab, ba);

        let p_then_o = WindowSpec {
            partition_by: vec![field("x", "a")],
            order_by: vec![(field("x", "b"), true)],
        };
        let o_then_p = WindowSpec {
            partition_by: vec![field("x", "b")],
            order_by: vec![(field("x", "a"), true)],
        };
        assert_ne!(
            generate_window_key("RANK", &[], &p_then_o),
            generate_window_key("RANK", &[], &o_then_p)
        );
        // Same call, same key.
        assert_eq!(ab, generate_window_key("sum", &[field("x", "ab")], &spec));
    }
}
