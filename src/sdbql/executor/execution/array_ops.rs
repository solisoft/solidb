//! Array comparison operators and inline array expressions (AQL syntax).
//!
//! - `arr ANY == x`, `arr ALL IN list`, `arr NONE > 3`, `arr AT LEAST (2) == 1`
//!   ([`Expression::ArrayComparison`])
//! - `arr[* FILTER cond LIMIT off, n RETURN proj].path` and `arr[**]`
//!   ([`Expression::ArrayInline`])
//!
//! The expression evaluator dispatches both variants here.

use serde_json::Value;

use super::super::types::Context;
use super::super::{evaluate_binary_op, get_field_value, to_bool, QueryExecutor};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{ArrayQuantifier, BinaryOperator, Expression};

/// Name the element is bound to inside an inline array expression.
const CURRENT: &str = "CURRENT";

impl<'a> QueryExecutor<'a> {
    /// Evaluate `left QUANTIFIER op right` with AQL semantics: a non-array
    /// `left` is `false`, and so is a non-array `right` for `IN` / `NOT IN`.
    pub(in crate::sdbql::executor) fn evaluate_array_comparison(
        &self,
        quantifier: &ArrayQuantifier,
        left: &Expression,
        op: &BinaryOperator,
        right: &Expression,
        ctx: &Context,
    ) -> DbResult<Value> {
        let left_val = self.evaluate_expr_with_context(left, ctx)?;
        let right_val = self.evaluate_expr_with_context(right, ctx)?;

        let Value::Array(items) = &left_val else {
            return Ok(Value::Bool(false));
        };
        if matches!(op, BinaryOperator::In | BinaryOperator::NotIn) && !right_val.is_array() {
            return Ok(Value::Bool(false));
        }

        let matches = |item: &Value| -> DbResult<bool> {
            Ok(to_bool(&evaluate_binary_op(item, op, &right_val)?))
        };

        let result = match quantifier {
            ArrayQuantifier::Any => {
                let mut any = false;
                for item in items {
                    if matches(item)? {
                        any = true;
                        break;
                    }
                }
                any
            }
            ArrayQuantifier::All => {
                let mut all = true;
                for item in items {
                    if !matches(item)? {
                        all = false;
                        break;
                    }
                }
                all
            }
            ArrayQuantifier::None => {
                let mut none = true;
                for item in items {
                    if matches(item)? {
                        none = false;
                        break;
                    }
                }
                none
            }
            ArrayQuantifier::AtLeast(count_expr) => {
                let wanted = match self.evaluate_expr_with_context(count_expr, ctx)? {
                    Value::Number(n) => n.as_f64().unwrap_or(0.0),
                    other => {
                        return Err(DbError::ExecutionError(format!(
                            "AT LEAST expects a number, got {}",
                            other
                        )))
                    }
                };
                if wanted <= 0.0 {
                    true
                } else {
                    let mut hits = 0usize;
                    let mut reached = false;
                    for item in items {
                        if matches(item)? {
                            hits += 1;
                            if hits as f64 >= wanted {
                                reached = true;
                                break;
                            }
                        }
                    }
                    reached
                }
            }
        };
        Ok(Value::Bool(result))
    }

    /// Evaluate an inline array expression:
    /// `base[*…* FILTER f LIMIT off, n RETURN proj].field_path`.
    ///
    /// A non-array `base` yields `[]`. With `depth > 1` the operand is
    /// flattened `depth - 1` levels first. FILTER, then LIMIT (applied to the
    /// elements that passed the filter), then RETURN, then the trailing
    /// attribute path; `CURRENT` is the element throughout.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::sdbql::executor) fn evaluate_array_inline(
        &self,
        base: &Expression,
        depth: usize,
        filter: Option<&Expression>,
        limit: Option<(&Expression, &Expression)>,
        projection: Option<&Expression>,
        field_path: Option<&str>,
        ctx: &Context,
    ) -> DbResult<Value> {
        let Value::Array(items) = self.evaluate_expr_with_context(base, ctx)? else {
            return Ok(Value::Array(Vec::new()));
        };

        let items = if depth > 1 {
            let mut flat = Vec::with_capacity(items.len());
            flatten_into(items, depth - 1, &mut flat);
            flat
        } else {
            items
        };

        // LIMIT bounds are evaluated once, in the enclosing scope.
        let (offset, count) = match limit {
            Some((offset_expr, count_expr)) => (
                limit_bound(self.evaluate_expr_with_context(offset_expr, ctx)?, "offset")?,
                Some(limit_bound(
                    self.evaluate_expr_with_context(count_expr, ctx)?,
                    "count",
                )?),
            ),
            None => (0, None),
        };

        // One copy of the scope for the whole loop; CURRENT is overwritten
        // per element rather than cloning the context for every item.
        let needs_scope = filter.is_some() || projection.is_some();
        let mut scope = if needs_scope { Some(ctx.clone()) } else { None };

        let mut out = Vec::new();
        let mut passed = 0usize;
        for item in items {
            if count.is_some_and(|c| out.len() >= c) {
                break;
            }

            let value = if let Some(scope) = scope.as_mut() {
                scope.insert(CURRENT.to_string(), item);
                if let Some(f) = filter {
                    if !to_bool(&self.evaluate_expr_with_context(f, scope)?) {
                        continue;
                    }
                }
                passed += 1;
                if passed <= offset {
                    continue;
                }
                match projection {
                    Some(p) => self.evaluate_expr_with_context(p, scope)?,
                    None => scope.remove(CURRENT).unwrap_or(Value::Null),
                }
            } else {
                passed += 1;
                if passed <= offset {
                    continue;
                }
                item
            };

            out.push(match field_path {
                Some(path) => get_field_value(&value, path),
                None => value,
            });
        }

        Ok(Value::Array(out))
    }
}

/// Splice nested arrays into `out`, `levels` levels deep.
fn flatten_into(items: Vec<Value>, levels: usize, out: &mut Vec<Value>) {
    for item in items {
        match item {
            Value::Array(inner) if levels > 0 => flatten_into(inner, levels - 1, out),
            other => out.push(other),
        }
    }
}

/// A LIMIT bound inside an inline expression: a non-negative integer.
fn limit_bound(value: Value, what: &str) -> DbResult<usize> {
    match value.as_f64() {
        Some(n) if n >= 0.0 && n.is_finite() => Ok(n as usize),
        _ => Err(DbError::ExecutionError(format!(
            "Inline LIMIT {} must be a non-negative number, got {}",
            what, value
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flatten_levels() {
        let mut out = Vec::new();
        flatten_into(vec![json!([1, [2]]), json!(3), json!([[4]])], 1, &mut out);
        assert_eq!(out, vec![json!(1), json!([2]), json!(3), json!([4])]);
    }

    #[test]
    fn limit_bound_rejects_negative() {
        assert!(limit_bound(json!(-1), "count").is_err());
        assert_eq!(limit_bound(json!(2), "count").unwrap(), 2);
    }
}
