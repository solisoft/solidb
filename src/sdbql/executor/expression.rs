//! Expression evaluation for SDBQL executor.
//!
//! This module contains expression evaluation logic:
//! - evaluate_expr_with_context: Main expression evaluator
//! - evaluate_filter_with_context: Filter expression evaluation
//! - evaluate_hof_with_lambda: Higher-order function evaluation

use super::window::generate_window_key;
use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use super::types::Context;
use super::{
    compare_key_rows, compare_values, evaluate_binary_op, evaluate_unary_op, get_field_ref,
    get_field_value, hash_value, to_bool, values_equal, QueryExecutor, ValueSet,
};
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
                let docs = self.scan_bounded(&collection)?;
                self.apply_row_policy(&for_clause.collection, docs, let_bindings)
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
    /// Resolve simple variable / field-path expressions to a borrow of the
    /// value already in the context, so FILTER predicates, SORT keys and
    /// projections don't deep-clone the whole document just to read one
    /// field. `None` means "not resolvable by reference" and the caller falls
    /// back to the owned evaluation path (which also reproduces its error
    /// semantics, e.g. the "Variable not found" error).
    fn resolve_ref<'v>(&'v self, expr: &'v Expression, ctx: &'v Context) -> Option<&'v Value> {
        static NULL: Value = Value::Null;
        match expr {
            Expression::Variable(name) => ctx.get(name),
            // Literals are borrowed from the AST: `x IN ["a", ..., "z"]` used
            // to copy the array for every row (audit P5).
            Expression::Literal(v) => Some(v),
            // The executor's bind vars are what entry points copy into the
            // context under "@name"; reading them first avoids building that
            // key on every access.
            Expression::BindVariable(name) => self
                .bind_vars
                .get(name)
                .or_else(|| ctx.get(&format!("@{}", name))),
            Expression::FieldAccess(base, field) => {
                let base_value = self.resolve_ref(base, ctx)?;
                // A missing segment reads as Null, matching get_field_value
                Some(get_field_ref(base_value, field).unwrap_or(&NULL))
            }
            Expression::OptionalFieldAccess(base, field) => {
                let base_value = self.resolve_ref(base, ctx)?;
                match base_value {
                    Value::Object(_) => Some(get_field_ref(base_value, field).unwrap_or(&NULL)),
                    // Null and non-object bases read as Null
                    _ => Some(&NULL),
                }
            }
            // `doc.tags[0]`, `doc[@f]`: borrow instead of cloning the base
            // (audit P7). An invalid key falls back to the owned path, which
            // reports the error.
            Expression::ArrayAccess(base, key) | Expression::DynamicFieldAccess(base, key) => {
                let base_value = self.resolve_ref(base, ctx)?;
                let key_value = self.resolve_ref(key, ctx)?;
                element_of(base_value, key_value)
                    .ok()
                    .map(|v| v.unwrap_or(&NULL))
            }
            _ => None,
        }
    }

    /// `ValueSet` for `x IN @var` when `@var` is a large array: built once per
    /// executor instead of scanning the array for every row (audit P9).
    fn bind_var_set(&self, name: &str) -> Option<Arc<ValueSet>> {
        /// Below this, a linear scan is as fast as hashing.
        const MIN_SET_LEN: usize = 16;
        if let Some(set) = self.caches.in_sets.lock().get(name) {
            return Some(set.clone());
        }
        let Value::Array(items) = self.bind_vars.get(name)? else {
            return None;
        };
        if items.len() < MIN_SET_LEN {
            return None;
        }
        let set = Arc::new(ValueSet::from_values(items));
        self.caches
            .in_sets
            .lock()
            .insert(name.to_string(), set.clone());
        Some(set)
    }

    /// The context key a window function's precomputed value is stored under.
    /// Computed once per call site rather than per row: the key serialises
    /// the whole call. The cached node is compared on each hit, so another
    /// AST reusing the same address can never read a stale key.
    fn window_key(
        &self,
        expr: &Expression,
        function: &str,
        arguments: &[Expression],
        over_clause: &WindowSpec,
    ) -> Arc<str> {
        let addr = expr as *const Expression as usize;
        if let Some((cached, key)) = self.caches.window_keys.lock().get(&addr) {
            if cached == expr {
                return key.clone();
            }
        }
        let key: Arc<str> = generate_window_key(function, arguments, over_clause).into();
        self.caches
            .window_keys
            .lock()
            .insert(addr, (expr.clone(), key.clone()));
        key
    }

    /// Sort rows by precomputing each row's sort keys once
    /// (decorate-sort-undecorate) instead of re-evaluating the sort
    /// expressions for both sides of every comparison. Evaluation errors read
    /// as Null, matching the previous comparator's `unwrap_or`; `sort_by` is
    /// stable, so tie order is unchanged.
    pub(crate) fn sort_rows(
        &self,
        rows: Vec<Context>,
        fields: &[(Expression, bool)],
    ) -> Vec<Context> {
        let ascending: Vec<bool> = fields.iter().map(|(_, asc)| *asc).collect();
        let mut decorated: Vec<(Vec<Value>, Context)> = rows
            .into_iter()
            .map(|ctx| {
                let keys = fields
                    .iter()
                    .map(|(expr, _)| {
                        self.evaluate_expr_with_context(expr, &ctx)
                            .unwrap_or(Value::Null)
                    })
                    .collect();
                (keys, ctx)
            })
            .collect();
        decorated.sort_by(|a, b| compare_key_rows(&a.0, &b.0, &ascending));
        decorated.into_iter().map(|(_, ctx)| ctx).collect()
    }

    /// Keep only the first `k` rows of the sorted order, for SORT immediately
    /// followed by LIMIT. The original row index is the final tiebreaker,
    /// making the order total — the result is exactly the first `k` rows of
    /// the stable full sort, at O(N + k log k) comparisons instead of
    /// O(N log N).
    pub(crate) fn sort_rows_top_k(
        &self,
        rows: Vec<Context>,
        fields: &[(Expression, bool)],
        k: usize,
    ) -> Vec<Context> {
        if k == 0 {
            return Vec::new();
        }
        let ascending: Vec<bool> = fields.iter().map(|(_, asc)| *asc).collect();
        let mut decorated: Vec<(Vec<Value>, usize, Context)> = rows
            .into_iter()
            .enumerate()
            .map(|(index, ctx)| {
                let keys = fields
                    .iter()
                    .map(|(expr, _)| {
                        self.evaluate_expr_with_context(expr, &ctx)
                            .unwrap_or(Value::Null)
                    })
                    .collect();
                (keys, index, ctx)
            })
            .collect();
        let cmp = |a: &(Vec<Value>, usize, Context), b: &(Vec<Value>, usize, Context)| {
            compare_key_rows(&a.0, &b.0, &ascending).then(a.1.cmp(&b.1))
        };
        if k < decorated.len() {
            decorated.select_nth_unstable_by(k - 1, cmp);
            decorated.truncate(k);
        }
        decorated.sort_by(cmp);
        decorated.into_iter().map(|(_, _, ctx)| ctx).collect()
    }

    /// Evaluate a filter expression with full context. The result is cast
    /// with AQL truthiness ([`to_bool`]), as the ternary, `!` and row
    /// policies already were: `FILTER 1` keeps the row, `FILTER null` drops it.
    pub fn evaluate_filter_with_context(&self, expr: &Expression, ctx: &Context) -> DbResult<bool> {
        if let Some(v) = self.resolve_ref(expr, ctx) {
            return Ok(to_bool(v));
        }
        Ok(to_bool(&self.evaluate_expr_with_context(expr, ctx)?))
    }

    /// Evaluate an expression with a context containing multiple variables
    pub fn evaluate_expr_with_context(&self, expr: &Expression, ctx: &Context) -> DbResult<Value> {
        match expr {
            Expression::Variable(name) => ctx
                .get(name)
                .cloned()
                .ok_or_else(|| DbError::ExecutionError(format!("Variable '{}' not found", name))),

            Expression::BindVariable(name) => {
                if let Some(value) = self.bind_vars.get(name) {
                    return Ok(value.clone());
                }
                // Contexts built by the entry points carry them as "@name".
                ctx.get(&format!("@{}", name)).cloned().ok_or_else(|| {
                    DbError::ExecutionError(format!(
                        "Bind variable '@{}' not found. Did you forget to pass it in bindVars?",
                        name
                    ))
                })
            }

            Expression::FieldAccess(base, field) => {
                // Fast path: borrow the base from the context and clone only
                // the leaf instead of deep-cloning the whole document.
                if let Some(base_value) = self.resolve_ref(base, ctx) {
                    return Ok(get_field_value(base_value, field));
                }
                let base_value = self.evaluate_expr_with_context(base, ctx)?;
                Ok(get_field_value(&base_value, field))
            }

            Expression::OptionalFieldAccess(base, field) => {
                if let Some(base_value) = self.resolve_ref(base, ctx) {
                    return Ok(match base_value {
                        Value::Object(_) => get_field_value(base_value, field),
                        _ => Value::Null,
                    });
                }
                let base_value = self.evaluate_expr_with_context(base, ctx)?;
                // Return null if base is null or not an object
                match base_value {
                    Value::Null => Ok(Value::Null),
                    Value::Object(_) => Ok(get_field_value(&base_value, field)),
                    _ => Ok(Value::Null), // Non-object types return null for optional access
                }
            }

            // `base[key]`: a number indexes an array (negative counts from the
            // end), a string is a literal object key (no dot splitting).
            Expression::DynamicFieldAccess(base, key) | Expression::ArrayAccess(base, key) => {
                if let Some(v) = self.resolve_ref(expr, ctx) {
                    return Ok(v.clone());
                }
                let base_owned;
                let base_value = match self.resolve_ref(base, ctx) {
                    Some(v) => v,
                    None => {
                        base_owned = self.evaluate_expr_with_context(base, ctx)?;
                        &base_owned
                    }
                };
                let key_owned;
                let key_value = match self.resolve_ref(key, ctx) {
                    Some(v) => v,
                    None => {
                        key_owned = self.evaluate_expr_with_context(key, ctx)?;
                        &key_owned
                    }
                };
                Ok(element_of(base_value, key_value)?
                    .cloned()
                    .unwrap_or(Value::Null))
            }

            Expression::ArraySpreadAccess(base, field_path) => {
                let base_owned;
                let base_value = match self.resolve_ref(base, ctx) {
                    Some(v) => v,
                    None => {
                        base_owned = self.evaluate_expr_with_context(base, ctx)?;
                        &base_owned
                    }
                };
                let Value::Array(arr) = base_value else {
                    return Ok(Value::Array(vec![])); // Non-array returns empty array
                };
                let mut results = Vec::with_capacity(arr.len());
                for elem in arr {
                    match field_path {
                        Some(path) => results.push(get_field_value(elem, path)),
                        // Flatten nested arrays when no field path
                        None => match elem {
                            Value::Array(inner) => results.extend(inner.iter().cloned()),
                            other => results.push(other.clone()),
                        },
                    }
                }
                Ok(Value::Array(results))
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
                    if matches!(op, BinaryOperator::In | BinaryOperator::NotIn) {
                        if let Expression::BindVariable(name) = right.as_ref() {
                            if let Some(set) = self.bind_var_set(name) {
                                let found = match self.resolve_ref(left, ctx) {
                                    Some(v) => set.contains(v),
                                    None => {
                                        set.contains(&self.evaluate_expr_with_context(left, ctx)?)
                                    }
                                };
                                return Ok(Value::Bool(
                                    found != matches!(op, BinaryOperator::NotIn),
                                ));
                            }
                        }
                    }
                    // Borrow operands that are simple variable/field paths so
                    // `FILTER doc.f == @v` evaluates without cloning anything.
                    let left_owned;
                    let left_val = match self.resolve_ref(left, ctx) {
                        Some(v) => v,
                        None => {
                            left_owned = self.evaluate_expr_with_context(left, ctx)?;
                            &left_owned
                        }
                    };
                    let right_owned;
                    let right_val = match self.resolve_ref(right, ctx) {
                        Some(v) => v,
                        None => {
                            right_owned = self.evaluate_expr_with_context(right, ctx)?;
                            &right_owned
                        }
                    };
                    evaluate_binary_op(left_val, op, right_val)
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
                        if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            if !f.is_finite() {
                                return Err(DbError::ExecutionError(format!(
                                    "Range start must be finite, got: {}",
                                    f
                                )));
                            }
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "Range start must be a number".to_string(),
                            ));
                        }
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
                        if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            if !f.is_finite() {
                                return Err(DbError::ExecutionError(format!(
                                    "Range end must be finite, got: {}",
                                    f
                                )));
                            }
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "Range end must be a number".to_string(),
                            ));
                        }
                    }
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "Range end must be a number, got: {:?}",
                            end_val
                        )))
                    }
                };

                const MAX_RANGE_SIZE: i64 = 10_000_000;
                // Use checked_sub so `start = i64::MIN` does not panic / wrap.
                // Any subtraction overflow is itself proof the range exceeds MAX.
                let range_size = end
                    .checked_sub(start)
                    .and_then(i64::checked_abs)
                    .ok_or_else(|| {
                        DbError::ExecutionError(format!(
                            "Range size overflow (start={}, end={})",
                            start, end
                        ))
                    })?;
                if range_size > MAX_RANGE_SIZE {
                    return Err(DbError::ExecutionError(format!(
                        "Range size {} exceeds maximum allowed size of {}",
                        range_size, MAX_RANGE_SIZE
                    )));
                }

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
                        let name_upper = super::builtins::upper_name(name);
                        let mut evaluated_args = Vec::with_capacity(args.len() + 1);
                        evaluated_args.push(left_val);
                        let mut has_lambda = false;
                        for arg in args {
                            if matches!(arg, Expression::Lambda { .. }) {
                                has_lambda = true;
                            } else {
                                evaluated_args.push(self.evaluate_expr_with_context(arg, ctx)?);
                            }
                        }
                        if has_lambda {
                            return self.evaluate_hof_with_lambda(
                                &name_upper,
                                evaluated_args,
                                args,
                                ctx,
                            );
                        }
                        // Every function, including the executor's own
                        // (MERGE, DOCUMENT, ...), not just the value builtins.
                        self.call_function(&name_upper, evaluated_args, ctx)
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
                // Window functions are pre-computed and stored in the context
                // under a key derived from the call.
                let key = self.window_key(expr, function, arguments, over_clause);
                if let Some(val) = ctx.get(&*key) {
                    return Ok(val.clone());
                }
                Err(DbError::ExecutionError(format!(
                    "Window function {} must be used in RETURN clause. \
                     Window functions are computed after all rows are collected.",
                    function
                )))
            }

            Expression::ArrayComparison {
                quantifier,
                left,
                op,
                right,
            } => self.evaluate_array_comparison(quantifier, left, op, right, ctx),

            Expression::ArrayInline {
                base,
                depth,
                filter,
                limit,
                projection,
                field_path,
            } => self.evaluate_array_inline(
                base,
                *depth,
                filter.as_deref(),
                limit.as_ref().map(|(off, n)| (&**off, &**n)),
                projection.as_deref(),
                field_path.as_deref(),
                ctx,
            ),

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

    /// Evaluate a higher-order function with lambda argument.
    ///
    /// `evaluated_args` are the non-lambda arguments in order: the array
    /// first (the piped value in the pipeline form), then any further value
    /// such as REDUCE's initial accumulator — so `REDUCE(arr, f, 0)` and
    /// `arr |> REDUCE(f, 0)` both start from 0.
    ///
    /// Lambda errors propagate (they used to read as `false` in FILTER, FIND
    /// and the quantifiers, deadline errors included). The body runs in one
    /// scratch context per call holding the parameters and only the outer
    /// variables it reads; items are moved in and out of it rather than the
    /// whole row being cloned per element (audit P6).
    pub(super) fn evaluate_hof_with_lambda(
        &self,
        name: &str,
        evaluated_args: Vec<Value>,
        original_args: &[Expression],
        ctx: &Context,
    ) -> DbResult<Value> {
        let mut values = evaluated_args.into_iter();
        let arr = match values.next() {
            Some(Value::Array(a)) => a,
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
        let extra: Vec<Value> = values.collect();

        let lambdas: Vec<(&[String], &Expression)> = original_args
            .iter()
            .filter_map(|arg| match arg {
                Expression::Lambda { params, body } => Some((params.as_slice(), body.as_ref())),
                _ => None,
            })
            .collect();
        let Some(&(params, body)) = lambdas.first() else {
            return Err(DbError::ExecutionError(format!(
                "{} requires a lambda argument",
                name
            )));
        };

        match name {
            "FILTER" => {
                let mut scope = LambdaScope::new(params, body, ctx);
                let mut out = Vec::with_capacity(arr.len());
                for item in arr {
                    let (keep, item) = scope.eval_keep(self, body, item)?;
                    if to_bool(&keep) {
                        out.push(item);
                    }
                }
                Ok(Value::Array(out))
            }
            "MAP" => {
                let mut scope = LambdaScope::new(params, body, ctx);
                let mut out = Vec::with_capacity(arr.len());
                for item in arr {
                    out.push(scope.eval(self, body, item)?);
                }
                Ok(Value::Array(out))
            }
            "FLAT_MAP" => {
                let mut scope = LambdaScope::new(params, body, ctx);
                let mut out = Vec::with_capacity(arr.len());
                for item in arr {
                    match scope.eval(self, body, item)? {
                        Value::Array(inner) => out.extend(inner),
                        other => out.push(other),
                    }
                }
                Ok(Value::Array(out))
            }
            "GROUP_BY" => {
                // Keys hashed once each: the old version compared every item
                // against every group so far (O(n·groups)).
                let mut scope = LambdaScope::new(params, body, ctx);
                let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
                let mut groups: Vec<(Value, Vec<Value>)> = Vec::new();
                for item in arr {
                    let (key, item) = scope.eval_keep(self, body, item)?;
                    let bucket = index.entry(hash_value(&key)).or_default();
                    match bucket.iter().find(|&&g| values_equal(&groups[g].0, &key)) {
                        Some(&g) => groups[g].1.push(item),
                        None => {
                            bucket.push(groups.len());
                            groups.push((key, vec![item]));
                        }
                    }
                }
                let out: Vec<Value> = groups
                    .into_iter()
                    .map(|(key, items)| json!({ "key": key, "items": items }))
                    .collect();
                Ok(Value::Array(out))
            }
            "SORT_BY" => {
                let mut scope = LambdaScope::new(params, body, ctx);
                let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(arr.len());
                for item in arr {
                    keyed.push(scope.eval_keep(self, body, item)?);
                }
                keyed.sort_by(|a, b| compare_values(&a.0, &b.0));
                Ok(Value::Array(keyed.into_iter().map(|(_, v)| v).collect()))
            }
            "MIN_BY" | "MAX_BY" => {
                let want = if name == "MIN_BY" {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
                let mut scope = LambdaScope::new(params, body, ctx);
                let mut best: Option<(Value, Value)> = None;
                for item in arr {
                    let (key, item) = scope.eval_keep(self, body, item)?;
                    super::builtins::array::keep_extreme(&mut best, key, item, want);
                }
                Ok(best.map(|(_, item)| item).unwrap_or(Value::Null))
            }
            "WINDOW_BY" => {
                // WINDOW_BY(arr, order_lambda) or WINDOW_BY(arr, part_lambda, order_lambda)
                let (part_l, order_l) = if lambdas.len() >= 2 {
                    (Some(lambdas[0]), lambdas[1])
                } else {
                    (None, lambdas[0])
                };
                let mut order_scope = LambdaScope::new(order_l.0, order_l.1, ctx);
                let mut part_scope = part_l.map(|(p, b)| LambdaScope::new(p, b, ctx));
                let mut rows: Vec<(Value, Value, Value)> = Vec::with_capacity(arr.len());
                for item in arr {
                    let (order_key, item) = order_scope.eval_keep(self, order_l.1, item)?;
                    let (part_key, item) = match (part_l, part_scope.as_mut()) {
                        (Some((_, part_body)), Some(scope)) => {
                            scope.eval_keep(self, part_body, item)?
                        }
                        _ => (Value::Null, item),
                    };
                    rows.push((part_key, order_key, item));
                }
                rows.sort_by(|a, b| {
                    compare_values(&a.0, &b.0).then_with(|| compare_values(&a.1, &b.1))
                });
                let mut out = Vec::with_capacity(rows.len());
                let mut last_part: Option<Value> = None;
                let mut rn = 0u64;
                for (part, _ord, item) in rows {
                    if last_part.as_ref().is_none_or(|p| !values_equal(p, &part)) {
                        rn = 0;
                        last_part = Some(part);
                    }
                    rn += 1;
                    let mut obj = match item {
                        Value::Object(m) => m,
                        other => {
                            let mut m = serde_json::Map::new();
                            m.insert("value".into(), other);
                            m
                        }
                    };
                    obj.insert("row_number".into(), json!(rn));
                    out.push(Value::Object(obj));
                }
                Ok(Value::Array(out))
            }
            "FIND" | "FIND_FIRST" => {
                let mut scope = LambdaScope::new(params, body, ctx);
                for item in arr {
                    let (hit, item) = scope.eval_keep(self, body, item)?;
                    if to_bool(&hit) {
                        return Ok(item);
                    }
                }
                Ok(Value::Null)
            }
            "ALL" | "EVERY" => {
                let mut scope = LambdaScope::new(params, body, ctx);
                for item in arr {
                    if !to_bool(&scope.eval(self, body, item)?) {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }
            "ANY" | "SOME" => {
                let mut scope = LambdaScope::new(params, body, ctx);
                for item in arr {
                    if to_bool(&scope.eval(self, body, item)?) {
                        return Ok(Value::Bool(true));
                    }
                }
                Ok(Value::Bool(false))
            }
            "NONE" => {
                let mut scope = LambdaScope::new(params, body, ctx);
                for item in arr {
                    if to_bool(&scope.eval(self, body, item)?) {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }
            "REDUCE" => {
                // The initial value is the argument after the lambda (the
                // first non-lambda argument was the array itself).
                let mut acc = extra.into_iter().next().unwrap_or(Value::Null);
                let mut scope = LambdaScope::new(params, body, ctx);
                // Lambda should have 2 params: (acc, item)
                for item in arr {
                    if params.len() >= 2 {
                        scope.bind(0, acc);
                        scope.bind(1, item);
                    } else {
                        scope.bind(0, item);
                    }
                    acc = self.evaluate_expr_with_context(body, &scope.ctx)?;
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

/// The scratch context a lambda body runs in: its parameters, rebound per
/// element in place, plus the outer variables the body reads.
struct LambdaScope<'p> {
    ctx: Context,
    params: &'p [String],
}

impl<'p> LambdaScope<'p> {
    fn new(params: &'p [String], body: &Expression, outer: &Context) -> Self {
        let ctx = match lambda_free_names(body) {
            None => outer.clone(),
            Some(names) => {
                let mut scope = Context::with_capacity(names.len() + params.len());
                for n in names {
                    if params.contains(&n) || scope.contains_key(&n) {
                        continue;
                    }
                    if let Some(v) = outer.get(&n) {
                        scope.insert(n, v.clone());
                    }
                }
                scope
            }
        };
        Self { ctx, params }
    }

    /// Bind parameter `i` (a no-op when the lambda has fewer parameters).
    fn bind(&mut self, i: usize, v: Value) {
        if let Some(p) = self.params.get(i) {
            match self.ctx.get_mut(p) {
                Some(slot) => *slot = v,
                None => {
                    self.ctx.insert(p.clone(), v);
                }
            }
        }
    }

    /// Evaluate `body` with the first parameter bound to `item`.
    fn eval(
        &mut self,
        exec: &QueryExecutor<'_>,
        body: &Expression,
        item: Value,
    ) -> DbResult<Value> {
        self.bind(0, item);
        exec.evaluate_expr_with_context(body, &self.ctx)
    }

    /// As [`Self::eval`], also handing `item` back (moved out of the scope,
    /// not cloned) for FILTER / FIND / SORT_BY / GROUP_BY.
    fn eval_keep(
        &mut self,
        exec: &QueryExecutor<'_>,
        body: &Expression,
        item: Value,
    ) -> DbResult<(Value, Value)> {
        let params = self.params;
        let Some(p) = params.first() else {
            return Ok((exec.evaluate_expr_with_context(body, &self.ctx)?, item));
        };
        self.bind(0, item);
        let result = exec.evaluate_expr_with_context(body, &self.ctx);
        let item = self
            .ctx
            .get_mut(p)
            .map(std::mem::take)
            .unwrap_or(Value::Null);
        Ok((result?, item))
    }
}

/// The outer names a lambda body reads (variables, and bind variables as
/// `@name`), or `None` when it may read the context in ways a name scan
/// cannot see — subqueries, window-function keys, `SEARCH_SCORE()`, dynamic
/// `APPLY` / `CALL` — and must get all of it.
fn lambda_free_names(body: &Expression) -> Option<Vec<String>> {
    fn walk(e: &Expression, out: &mut Vec<String>) -> bool {
        match e {
            Expression::Variable(n) => out.push(n.clone()),
            Expression::BindVariable(n) => out.push(format!("@{}", n)),
            Expression::Subquery(_) | Expression::WindowFunctionCall { .. } => return false,
            Expression::FunctionCall { name, .. }
                if ["SEARCH_SCORE", "APPLY", "CALL"]
                    .iter()
                    .any(|f| name.eq_ignore_ascii_case(f)) =>
            {
                return false
            }
            _ => {}
        }
        let mut ok = true;
        e.for_each_child(&mut |child| {
            if ok && !walk(child, out) {
                ok = false;
            }
        });
        ok
    }
    let mut out = Vec::new();
    walk(body, &mut out).then_some(out)
}

/// `base[key]`. A number indexes an array, counting from the end when
/// negative (`arr[-1]` is the last element), or reads the object key of the
/// same spelling; a string is a literal object key (`doc["a.b"]` does not
/// split on the dot). `Ok(None)` reads as null; a key of any other type is an
/// error (`null` reads as null).
fn element_of<'v>(base: &'v Value, key: &Value) -> DbResult<Option<&'v Value>> {
    match key {
        Value::String(k) => Ok(match base {
            Value::Object(o) => o.get(k.as_str()),
            _ => None,
        }),
        Value::Number(n) => match base {
            Value::Array(arr) => Ok(array_position(n, arr.len())?.and_then(|i| arr.get(i))),
            Value::Object(o) => Ok(match n.as_i64() {
                Some(i) => o.get(&i.to_string()),
                None => o.get(&n.to_string()),
            }),
            _ => Ok(None),
        },
        Value::Null => Ok(None),
        other => Err(DbError::ExecutionError(format!(
            "Dynamic field access requires a string or number, got: {:?}",
            other
        ))),
    }
}

/// Position of array index `n` in an array of `len`, or `None` when out of
/// range. Fractional indexes truncate.
fn array_position(n: &serde_json::Number, len: usize) -> DbResult<Option<usize>> {
    let i: i64 = match n.as_i64() {
        Some(i) => i,
        None if n.as_u64().is_some() => return Ok(None), // beyond any array
        None => {
            let f = n.as_f64().unwrap_or(0.0);
            if !f.is_finite() {
                return Err(DbError::ExecutionError(format!(
                    "Array index must be finite, got: {}",
                    f
                )));
            }
            f.trunc() as i64
        }
    };
    if i < 0 {
        Ok((len as i64)
            .checked_add(i)
            .filter(|j| *j >= 0)
            .map(|j| j as usize))
    } else {
        Ok(usize::try_from(i).ok())
    }
}
