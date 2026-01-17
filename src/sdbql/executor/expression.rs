//! Expression evaluation for SDBQL executor.
//!
//! This module contains expression evaluation logic:
//! - evaluate_expr_with_context: Main expression evaluator
//! - evaluate_filter_with_context: Filter expression evaluation
//! - evaluate_hof_with_lambda: Higher-order function evaluation

use std::collections::HashMap;
use super::window::generate_window_key;

use serde_json::Value;

use super::types::Context;
use super::{evaluate_binary_op, evaluate_unary_op, get_field_value, to_bool, values_equal, QueryExecutor};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::*;

impl<'a> QueryExecutor<'a> {
    pub(super) fn build_row_combinations_with_context(
        &self,
        for_clauses: &[ForClause],
        let_bindings: &Context,
    ) -> DbResult<Vec<Context>> {
        if for_clauses.is_empty() {
            // If no FOR clauses but we have LET bindings, return single row with bindings
            if !let_bindings.is_empty() {
                return Ok(vec![let_bindings.clone()]);
            }
            return Ok(vec![HashMap::new()]);
        }

        // Start with LET bindings as initial context
        let mut result: Vec<Context> = vec![let_bindings.clone()];

        for for_clause in for_clauses {
            let source_name = for_clause
                .source_variable
                .as_ref()
                .unwrap_or(&for_clause.collection);

            // First check if source is a LET variable (array)
            let docs: Vec<Value> = if let Some(let_value) = let_bindings.get(source_name) {
                // Source is a LET variable - should be an array
                match let_value {
                    Value::Array(arr) => arr.clone(),
                    // If it's a single value, wrap it in an array
                    other => vec![other.clone()],
                }
            } else {
                // Source is a collection name
                let collection = self.get_collection(&for_clause.collection)?;
                collection.all().iter().map(|d| d.to_value()).collect()
            };

            let var_name = &for_clause.variable;

            // Cross product: for each existing row, create new rows with each doc
            let mut new_result = Vec::with_capacity(result.len() * docs.len());

            for existing_ctx in &result {
                for doc in &docs {
                    let mut new_ctx = existing_ctx.clone();
                    new_ctx.insert(var_name.clone(), doc.clone());
                    new_result.push(new_ctx);
                }
            }

            result = new_result;
        }

        Ok(result)
    }
    /// Evaluate a filter expression with full context
    pub fn evaluate_filter_with_context(&self, expr: &Expression, ctx: &Context) -> DbResult<bool> {
        match self.evaluate_expr_with_context(expr, ctx)? {
            Value::Bool(b) => Ok(b),
            _ => Ok(false),
        }
    }

    /// Evaluate an expression with a context containing multiple variables
    pub fn evaluate_expr_with_context(&self, expr: &Expression, ctx: &Context) -> DbResult<Value> {
        match expr {
            Expression::Variable(name) => ctx
                .get(name)
                .cloned()
                .ok_or_else(|| DbError::ExecutionError(format!("Variable '{}' not found", name))),

            Expression::BindVariable(name) => {
                // First check context (bind vars are stored with @ prefix)
                if let Some(value) = ctx.get(&format!("@{}", name)) {
                    return Ok(value.clone());
                }
                // Then check bind_vars directly
                self.bind_vars.get(name).cloned().ok_or_else(|| {
                    DbError::ExecutionError(format!(
                        "Bind variable '@{}' not found. Did you forget to pass it in bindVars?",
                        name
                    ))
                })
            }

            Expression::FieldAccess(base, field) => {
                let base_value = self.evaluate_expr_with_context(base, ctx)?;
                Ok(get_field_value(&base_value, field))
            }

            Expression::OptionalFieldAccess(base, field) => {
                let base_value = self.evaluate_expr_with_context(base, ctx)?;
                // Return null if base is null or not an object
                match base_value {
                    Value::Null => Ok(Value::Null),
                    Value::Object(_) => Ok(get_field_value(&base_value, field)),
                    _ => Ok(Value::Null), // Non-object types return null for optional access
                }
            }

            Expression::DynamicFieldAccess(base, field_expr) => {
                let base_value = self.evaluate_expr_with_context(base, ctx)?;
                let field_value = self.evaluate_expr_with_context(field_expr, ctx)?;

                // The field expression should evaluate to a string (field name)
                let field_name = match field_value {
                    Value::String(s) => s,
                    Value::Number(n) => n.to_string(),
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "Dynamic field access requires a string or number, got: {:?}",
                            field_value
                        )))
                    }
                };

                Ok(get_field_value(&base_value, &field_name))
            }

            Expression::ArrayAccess(base, index_expr) => {
                let base_value = self.evaluate_expr_with_context(base, ctx)?;
                let index_value = self.evaluate_expr_with_context(index_expr, ctx)?;

                // The index should be a number
                let index = match index_value {
                    Value::Number(n) => {
                        // Handle both integer and float numbers
                        if let Some(i) = n.as_u64() {
                            i as usize
                        } else if let Some(f) = n.as_f64() {
                            // Convert float to integer (truncate)
                            if f < 0.0 {
                                return Err(DbError::ExecutionError(format!(
                                    "Array index must be non-negative, got: {}",
                                    f
                                )));
                            }
                            f as usize
                        } else {
                            return Err(DbError::ExecutionError(format!(
                                "Invalid array index: {}",
                                n
                            )));
                        }
                    }
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "Array index must be a number, got: {:?}",
                            index_value
                        )))
                    }
                };

                // Access the array element
                match base_value {
                    Value::Array(ref arr) => Ok(arr.get(index).cloned().unwrap_or(Value::Null)),
                    _ => Ok(Value::Null), // Non-arrays return null
                }
            }

            Expression::ArraySpreadAccess(base, field_path) => {
                let base_value = self.evaluate_expr_with_context(base, ctx)?;

                match base_value {
                    Value::Array(arr) => {
                        let results: Vec<Value> = arr
                            .iter()
                            .flat_map(|elem| match field_path {
                                Some(ref path) => vec![get_field_value(elem, path)],
                                None => {
                                    // Flatten nested arrays when no field path
                                    match elem {
                                        Value::Array(inner) => inner.clone(),
                                        other => vec![other.clone()],
                                    }
                                }
                            })
                            .collect();
                        Ok(Value::Array(results))
                    }
                    _ => Ok(Value::Array(vec![])), // Non-array returns empty array
                }
            }

            Expression::Literal(value) => Ok(value.clone()),

            Expression::BinaryOp { left, op, right } => match op {
                BinaryOperator::And => {
                    let left_val = self.evaluate_expr_with_context(left, ctx)?;
                    if !to_bool(&left_val) {
                        return Ok(Value::Bool(false));
                    }
                    let right_val = self.evaluate_expr_with_context(right, ctx)?;
                    Ok(Value::Bool(to_bool(&right_val)))
                }
                BinaryOperator::Or => {
                    let left_val = self.evaluate_expr_with_context(left, ctx)?;
                    if to_bool(&left_val) {
                        return Ok(Value::Bool(true));
                    }
                    let right_val = self.evaluate_expr_with_context(right, ctx)?;
                    Ok(Value::Bool(to_bool(&right_val)))
                }
                BinaryOperator::NullCoalesce => {
                    let left_val = self.evaluate_expr_with_context(left, ctx)?;
                    if !left_val.is_null() {
                        return Ok(left_val);
                    }
                    self.evaluate_expr_with_context(right, ctx)
                }
                BinaryOperator::LogicalOr => {
                    // || returns left if truthy, otherwise right (short-circuit)
                    let left_val = self.evaluate_expr_with_context(left, ctx)?;
                    if to_bool(&left_val) {
                        return Ok(left_val);
                    }
                    self.evaluate_expr_with_context(right, ctx)
                }
                _ => {
                    let left_val = self.evaluate_expr_with_context(left, ctx)?;
                    let right_val = self.evaluate_expr_with_context(right, ctx)?;
                    evaluate_binary_op(&left_val, op, &right_val)
                }
            },

            Expression::UnaryOp { op, operand } => {
                let val = self.evaluate_expr_with_context(operand, ctx)?;
                evaluate_unary_op(op, &val)
            }

            Expression::Object(fields) => {
                let mut obj = serde_json::Map::with_capacity(fields.len());
                for (key, value_expr) in fields {
                    let value = self.evaluate_expr_with_context(value_expr, ctx)?;
                    obj.insert(key.clone(), value);
                }
                Ok(Value::Object(obj))
            }

            Expression::Array(elements) => {
                let mut arr = Vec::with_capacity(elements.len());
                for elem in elements {
                    arr.push(self.evaluate_expr_with_context(elem, ctx)?);
                }
                Ok(Value::Array(arr))
            }

            Expression::Range(start_expr, end_expr) => {
                let start_val = self.evaluate_expr_with_context(start_expr, ctx)?;
                let end_val = self.evaluate_expr_with_context(end_expr, ctx)?;

                let start = match &start_val {
                    Value::Number(n) => {
                        // Try integer first, then fall back to truncating float
                        n.as_i64()
                            .or_else(|| n.as_f64().map(|f| f as i64))
                            .ok_or_else(|| {
                                DbError::ExecutionError("Range start must be a number".to_string())
                            })?
                    }
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "Range start must be a number, got: {:?}",
                            start_val
                        )))
                    }
                };

                let end = match &end_val {
                    Value::Number(n) => {
                        // Try integer first, then fall back to truncating float
                        n.as_i64()
                            .or_else(|| n.as_f64().map(|f| f as i64))
                            .ok_or_else(|| {
                                DbError::ExecutionError("Range end must be a number".to_string())
                            })?
                    }
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "Range end must be a number, got: {:?}",
                            end_val
                        )))
                    }
                };

                // Generate array from start to end (inclusive)
                let arr: Vec<Value> = (start..=end)
                    .map(|i| Value::Number(serde_json::Number::from(i)))
                    .collect();

                Ok(Value::Array(arr))
            }

            Expression::FunctionCall { name, args } => self.evaluate_function(name, args, ctx),

            Expression::Subquery(subquery) => {
                // Execute the subquery with parent context (enables correlated subqueries)
                let results = self.execute_with_parent_context(subquery, ctx)?;
                Ok(Value::Array(results))
            }

            Expression::Ternary {
                condition,
                true_expr,
                false_expr,
            } => {
                let cond_val = self.evaluate_expr_with_context(condition, ctx)?;
                if to_bool(&cond_val) {
                    self.evaluate_expr_with_context(true_expr, ctx)
                } else {
                    self.evaluate_expr_with_context(false_expr, ctx)
                }
            }

            Expression::Case {
                operand,
                when_clauses,
                else_clause,
            } => {
                // Evaluate operand once if present (simple CASE)
                let operand_val = match operand {
                    Some(op) => Some(self.evaluate_expr_with_context(op, ctx)?),
                    None => None,
                };

                // Check each WHEN clause
                for (condition, result) in when_clauses {
                    let matches = if let Some(ref op_val) = operand_val {
                        // Simple CASE: compare operand to WHEN value
                        let when_val = self.evaluate_expr_with_context(condition, ctx)?;
                        values_equal(op_val, &when_val)
                    } else {
                        // Searched CASE: evaluate WHEN condition as boolean
                        let cond_val = self.evaluate_expr_with_context(condition, ctx)?;
                        to_bool(&cond_val)
                    };

                    if matches {
                        return self.evaluate_expr_with_context(result, ctx);
                    }
                }

                // No WHEN matched - return ELSE or null
                match else_clause {
                    Some(else_expr) => self.evaluate_expr_with_context(else_expr, ctx),
                    None => Ok(Value::Null),
                }
            }

            Expression::Pipeline { left, right } => {
                // Evaluate left side first
                let left_val = self.evaluate_expr_with_context(left, ctx)?;

                // Right side must be a FunctionCall - prepend left_val to args
                match right.as_ref() {
                    Expression::FunctionCall { name, args } => {
                        // Check if any arg is a lambda - if so, use HOF evaluation
                        let has_lambda =
                            args.iter().any(|a| matches!(a, Expression::Lambda { .. }));

                        if has_lambda {
                            // Pass left_val as first evaluated arg, keep original args for lambda
                            return self.evaluate_hof_with_lambda(
                                &name.to_uppercase(),
                                &[left_val],
                                args,
                                ctx,
                            );
                        }

                        // No lambda - evaluate all args normally
                        let mut evaluated_args = vec![left_val];
                        for arg in args {
                            evaluated_args.push(self.evaluate_expr_with_context(arg, ctx)?);
                        }
                        self.evaluate_function_with_values(&name.to_uppercase(), &evaluated_args)
                    }
                    _ => Err(DbError::ExecutionError(
                        "Pipeline operator |> requires a function call on the right side"
                            .to_string(),
                    )),
                }
            }

            Expression::Lambda { params, body: _ } => {
                // Lambdas cannot be evaluated directly - they must be used with HOFs
                // Return an error if someone tries to evaluate a lambda standalone
                Err(DbError::ExecutionError(format!(
                    "Lambda expression with params {:?} cannot be evaluated directly. \
                     Use it with higher-order functions like FILTER, MAP, etc.",
                    params
                )))
            }

            Expression::WindowFunctionCall {
                function,
                arguments,
                over_clause,
            } => {
                // Window functions are pre-computed and stored in context with __window_N keys
                // Generate a unique key from the window function signature
                let key = generate_window_key(function, arguments, over_clause);
                if let Some(val) = ctx.get(&key) {
                    return Ok(val.clone());
                }
                // Fallback: try looking up by sequential index (for backwards compatibility)
                for i in 0..100 {
                    let fallback_key = format!("__window_{}", i);
                    if let Some(val) = ctx.get(&fallback_key) {
                        return Ok(val.clone());
                    }
                }
                Err(DbError::ExecutionError(format!(
                    "Window function {} must be used in RETURN clause. \
                     Window functions are computed after all rows are collected.",
                    function
                )))
            }

            Expression::TemplateString { parts } => {
                let mut result = String::new();

                for part in parts {
                    match part {
                        TemplateStringPart::Literal(s) => {
                            result.push_str(s);
                        }
                        TemplateStringPart::Expression(expr) => {
                            let value = self.evaluate_expr_with_context(expr, ctx)?;
                            // Type coercion to string
                            match value {
                                Value::String(s) => result.push_str(&s),
                                Value::Number(n) => {
                                    // Format integers without decimal point
                                    if let Some(i) = n.as_i64() {
                                        result.push_str(&i.to_string());
                                    } else if let Some(f) = n.as_f64() {
                                        // Check if it's a whole number
                                        if f.fract() == 0.0 && f.abs() < (i64::MAX as f64) {
                                            result.push_str(&(f as i64).to_string());
                                        } else {
                                            result.push_str(&f.to_string());
                                        }
                                    } else {
                                        result.push_str(&n.to_string());
                                    }
                                }
                                Value::Bool(b) => result.push_str(&b.to_string()),
                                Value::Null => result.push_str("null"),
                                Value::Array(_) | Value::Object(_) => {
                                    result.push_str(
                                        &serde_json::to_string(&value).unwrap_or_default(),
                                    );
                                }
                            }
                        }
                    }
                }

                Ok(Value::String(result))
            }
        }
    }

    /// Evaluate a higher-order function with lambda argument
    pub(super) fn evaluate_hof_with_lambda(
        &self,
        name: &str,
        evaluated_args: &[Value],
        original_args: &[Expression],
        ctx: &Context,
    ) -> DbResult<Value> {
        // First arg should be array (already evaluated)
        let arr = match evaluated_args.first() {
            Some(Value::Array(a)) => a.clone(),
            Some(other) => {
                return Err(DbError::ExecutionError(format!(
                    "{} expects an array as first argument, got {:?}",
                    name, other
                )))
            }
            None => {
                return Err(DbError::ExecutionError(format!(
                    "{} requires arguments",
                    name
                )))
            }
        };

        // Find the lambda in original args (skip first which is the piped value)
        let lambda = original_args.iter().find_map(|arg| match arg {
            Expression::Lambda { params, body } => Some((params.clone(), body.clone())),
            _ => None,
        });

        let (params, body) = match lambda {
            Some(l) => l,
            None => {
                return Err(DbError::ExecutionError(format!(
                    "{} requires a lambda argument",
                    name
                )))
            }
        };

        match name {
            "FILTER" => {
                let filtered: Vec<Value> = arr
                    .into_iter()
                    .filter(|item| {
                        let mut lambda_ctx = ctx.clone();
                        if let Some(param) = params.first() {
                            lambda_ctx.insert(param.clone(), item.clone());
                        }
                        self.evaluate_expr_with_context(&body, &lambda_ctx)
                            .map(|v| to_bool(&v))
                            .unwrap_or(false)
                    })
                    .collect();
                Ok(Value::Array(filtered))
            }
            "MAP" => {
                let mapped: DbResult<Vec<Value>> = arr
                    .into_iter()
                    .map(|item| {
                        let mut lambda_ctx = ctx.clone();
                        if let Some(param) = params.first() {
                            lambda_ctx.insert(param.clone(), item.clone());
                        }
                        self.evaluate_expr_with_context(&body, &lambda_ctx)
                    })
                    .collect();
                Ok(Value::Array(mapped?))
            }
            "FIND" | "FIND_FIRST" => {
                for item in arr {
                    let mut lambda_ctx = ctx.clone();
                    if let Some(param) = params.first() {
                        lambda_ctx.insert(param.clone(), item.clone());
                    }
                    if self
                        .evaluate_expr_with_context(&body, &lambda_ctx)
                        .map(|v| to_bool(&v))
                        .unwrap_or(false)
                    {
                        return Ok(item);
                    }
                }
                Ok(Value::Null)
            }
            "ALL" | "EVERY" => {
                for item in arr {
                    let mut lambda_ctx = ctx.clone();
                    if let Some(param) = params.first() {
                        lambda_ctx.insert(param.clone(), item.clone());
                    }
                    if !self
                        .evaluate_expr_with_context(&body, &lambda_ctx)
                        .map(|v| to_bool(&v))
                        .unwrap_or(false)
                    {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }
            "ANY" | "SOME" => {
                for item in arr {
                    let mut lambda_ctx = ctx.clone();
                    if let Some(param) = params.first() {
                        lambda_ctx.insert(param.clone(), item.clone());
                    }
                    if self
                        .evaluate_expr_with_context(&body, &lambda_ctx)
                        .map(|v| to_bool(&v))
                        .unwrap_or(false)
                    {
                        return Ok(Value::Bool(true));
                    }
                }
                Ok(Value::Bool(false))
            }
            "REDUCE" => {
                // REDUCE needs initial value - find non-lambda arg in original_args
                let initial = original_args
                    .iter()
                    .find(|arg| !matches!(arg, Expression::Lambda { .. }))
                    .map(|arg| self.evaluate_expr_with_context(arg, ctx))
                    .transpose()?
                    .unwrap_or(Value::Null);
                let mut acc = initial;

                // Lambda should have 2 params: (acc, item)
                for item in arr {
                    let mut lambda_ctx = ctx.clone();
                    if params.len() >= 2 {
                        lambda_ctx.insert(params[0].clone(), acc.clone());
                        lambda_ctx.insert(params[1].clone(), item.clone());
                    } else if let Some(param) = params.first() {
                        lambda_ctx.insert(param.clone(), item.clone());
                    }
                    acc = self.evaluate_expr_with_context(&body, &lambda_ctx)?;
                }
                Ok(acc)
            }
            _ => Err(DbError::ExecutionError(format!(
                "Function {} does not support lambda arguments",
                name
            ))),
        }
    }

}
