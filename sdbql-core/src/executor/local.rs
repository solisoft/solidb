//! Local executor for SDBQL queries.
//!
//! Executes SDBQL queries against a DataSource without requiring a server connection.

use std::collections::HashMap;

use serde_json::Value;

use crate::ast::*;
use crate::error::{SdbqlError, SdbqlResult};
use crate::parser;

use super::builtins::BuiltinFunctions;
use super::helpers::*;
use super::{BindVars, DataSource, QueryLimits};

/// Local executor for SDBQL queries.
///
/// Executes queries against any DataSource implementation.
pub struct LocalExecutor<D: DataSource> {
    data_source: D,
    limits: QueryLimits,
}

impl<D: DataSource> LocalExecutor<D> {
    /// Create a new executor with the given data source.
    pub fn new(data_source: D) -> Self {
        Self {
            data_source,
            limits: QueryLimits::default(),
        }
    }

    /// Create a new executor with custom limits.
    pub fn with_limits(data_source: D, limits: QueryLimits) -> Self {
        Self {
            data_source,
            limits,
        }
    }

    /// Execute a query string.
    ///
    /// # Arguments
    /// * `query` - SDBQL query string
    /// * `bind_vars` - Optional bind variables for parameterized queries
    ///
    /// # Returns
    /// Vector of result values
    pub fn execute(&self, query: &str, bind_vars: Option<BindVars>) -> SdbqlResult<Vec<Value>> {
        let ast = parser::parse(query)?;
        self.execute_query(&ast, bind_vars.unwrap_or_default())
    }

    /// Execute a parsed query AST.
    pub fn execute_query(&self, query: &Query, bind_vars: BindVars) -> SdbqlResult<Vec<Value>> {
        // Check for unsupported operations
        self.check_unsupported(query)?;

        let mut context = ExecutionContext::new(bind_vars);

        // Evaluate initial LET clauses
        for let_clause in &query.let_clauses {
            let value = self.evaluate_expression(&let_clause.expression, &context)?;
            context.set_variable(&let_clause.variable, value);
        }

        // Execute body clauses
        let mut results = self.execute_body(&query.body_clauses, &context)?;

        // Apply SORT
        if let Some(sort) = &query.sort_clause {
            self.apply_sort(&mut results, sort, &context)?;
        }

        // Apply LIMIT
        if let Some(limit) = &query.limit_clause {
            results = self.apply_limit(results, limit, &context)?;
        }

        // Apply RETURN
        if let Some(return_clause) = &query.return_clause {
            results = self.apply_return(results, return_clause, &context)?;
        }

        Ok(results)
    }

    /// Check for unsupported operations in offline mode.
    fn check_unsupported(&self, query: &Query) -> SdbqlResult<()> {
        for clause in &query.body_clauses {
            match clause {
                BodyClause::Insert(_) => {
                    return Err(SdbqlError::OperationNotSupported(
                        "INSERT not supported in local queries - use pending_changes".to_string(),
                    ));
                }
                BodyClause::Update(_) => {
                    return Err(SdbqlError::OperationNotSupported(
                        "UPDATE not supported in local queries - use pending_changes".to_string(),
                    ));
                }
                BodyClause::Remove(_) => {
                    return Err(SdbqlError::OperationNotSupported(
                        "REMOVE not supported in local queries - use pending_changes".to_string(),
                    ));
                }
                BodyClause::Upsert(_) => {
                    return Err(SdbqlError::OperationNotSupported(
                        "UPSERT not supported in local queries - use pending_changes".to_string(),
                    ));
                }
                BodyClause::GraphTraversal(_) => {
                    return Err(SdbqlError::OperationNotSupported(
                        "Graph traversal not supported in local queries".to_string(),
                    ));
                }
                BodyClause::ShortestPath(_) => {
                    return Err(SdbqlError::OperationNotSupported(
                        "Shortest path not supported in local queries".to_string(),
                    ));
                }
                _ => {}
            }
        }

        if query.create_stream_clause.is_some() {
            return Err(SdbqlError::OperationNotSupported(
                "CREATE STREAM not supported in local queries".to_string(),
            ));
        }

        if query.create_materialized_view_clause.is_some() {
            return Err(SdbqlError::OperationNotSupported(
                "CREATE MATERIALIZED VIEW not supported in local queries".to_string(),
            ));
        }

        Ok(())
    }

    /// Execute body clauses (FOR, LET, FILTER, etc.)
    fn execute_body(
        &self,
        clauses: &[BodyClause],
        context: &ExecutionContext,
    ) -> SdbqlResult<Vec<Value>> {
        if clauses.is_empty() {
            // No body clauses, just return a single empty context
            return Ok(vec![Value::Object(serde_json::Map::new())]);
        }

        let mut current_rows: Vec<ExecutionContext> = vec![context.clone()];

        for clause in clauses {
            current_rows = self.execute_clause(clause, current_rows)?;
        }

        // Convert contexts to values
        Ok(current_rows.into_iter().map(|c| c.to_value()).collect())
    }

    /// Execute a single body clause.
    fn execute_clause(
        &self,
        clause: &BodyClause,
        contexts: Vec<ExecutionContext>,
    ) -> SdbqlResult<Vec<ExecutionContext>> {
        match clause {
            BodyClause::For(for_clause) => self.execute_for(for_clause, contexts),
            BodyClause::Let(let_clause) => self.execute_let(let_clause, contexts),
            BodyClause::Filter(filter_clause) => self.execute_filter(filter_clause, contexts),
            BodyClause::Join(join_clause) => self.execute_join(join_clause, contexts),
            BodyClause::Collect(collect_clause) => self.execute_collect(collect_clause, contexts),
            BodyClause::Window(_) => Ok(contexts), // Window clause handled differently
            _ => Err(SdbqlError::OperationNotSupported(
                "Clause not supported in local queries".to_string(),
            )),
        }
    }

    /// Execute a FOR clause.
    fn execute_for(
        &self,
        for_clause: &ForClause,
        contexts: Vec<ExecutionContext>,
    ) -> SdbqlResult<Vec<ExecutionContext>> {
        let mut results = Vec::new();

        for ctx in contexts {
            let items = self.get_for_source(for_clause, &ctx)?;

            // Check scan limit
            if items.len() > self.limits.max_scan_docs {
                return Err(SdbqlError::ExecutionError(format!(
                    "Scan limit exceeded: {} > {}",
                    items.len(),
                    self.limits.max_scan_docs
                )));
            }

            for item in items {
                let mut new_ctx = ctx.clone();
                new_ctx.set_variable(&for_clause.variable, item);
                results.push(new_ctx);
            }
        }

        Ok(results)
    }

    /// Get the source items for a FOR clause.
    fn get_for_source(
        &self,
        for_clause: &ForClause,
        context: &ExecutionContext,
    ) -> SdbqlResult<Vec<Value>> {
        // Check if iterating over an expression
        if let Some(expr) = &for_clause.source_expression {
            let value = self.evaluate_expression(expr, context)?;
            match value {
                Value::Array(arr) => return Ok(arr),
                _ => {
                    return Err(SdbqlError::ExecutionError(
                        "FOR source must be an array".to_string(),
                    ))
                }
            }
        }

        // Check if iterating over a variable
        if let Some(var_name) = &for_clause.source_variable {
            // First check if it's an array variable in context
            if let Some(Value::Array(arr)) = context.get_variable(var_name) {
                return Ok(arr.clone());
            }

            // Otherwise treat as collection name
            if self.data_source.collection_exists(var_name) {
                return Ok(self
                    .data_source
                    .scan(var_name, Some(self.limits.max_scan_docs)));
            }

            return Err(SdbqlError::CollectionNotFound(var_name.clone()));
        }

        // Use collection name
        if !for_clause.collection.is_empty() {
            if self.data_source.collection_exists(&for_clause.collection) {
                return Ok(self
                    .data_source
                    .scan(&for_clause.collection, Some(self.limits.max_scan_docs)));
            }
            return Err(SdbqlError::CollectionNotFound(
                for_clause.collection.clone(),
            ));
        }

        Err(SdbqlError::ExecutionError(
            "FOR clause has no source".to_string(),
        ))
    }

    /// Execute a LET clause.
    fn execute_let(
        &self,
        let_clause: &LetClause,
        contexts: Vec<ExecutionContext>,
    ) -> SdbqlResult<Vec<ExecutionContext>> {
        let mut results = Vec::new();

        for mut ctx in contexts {
            let value = self.evaluate_expression(&let_clause.expression, &ctx)?;
            ctx.set_variable(&let_clause.variable, value);
            results.push(ctx);
        }

        Ok(results)
    }

    /// Execute a FILTER clause.
    fn execute_filter(
        &self,
        filter_clause: &FilterClause,
        contexts: Vec<ExecutionContext>,
    ) -> SdbqlResult<Vec<ExecutionContext>> {
        let mut results = Vec::new();

        for ctx in contexts {
            let value = self.evaluate_expression(&filter_clause.expression, &ctx)?;
            if to_bool(&value) {
                results.push(ctx);
            }
        }

        Ok(results)
    }

    /// Execute a JOIN clause.
    fn execute_join(
        &self,
        join_clause: &JoinClause,
        contexts: Vec<ExecutionContext>,
    ) -> SdbqlResult<Vec<ExecutionContext>> {
        let mut results = Vec::new();

        // Get join collection documents
        if !self.data_source.collection_exists(&join_clause.collection) {
            return Err(SdbqlError::CollectionNotFound(
                join_clause.collection.clone(),
            ));
        }

        let join_docs = self
            .data_source
            .scan(&join_clause.collection, Some(self.limits.max_scan_docs));

        for ctx in &contexts {
            let mut found_match = false;

            for join_doc in &join_docs {
                let mut test_ctx = ctx.clone();
                test_ctx.set_variable(&join_clause.variable, join_doc.clone());

                let condition = self.evaluate_expression(&join_clause.condition, &test_ctx)?;
                if to_bool(&condition) {
                    results.push(test_ctx);
                    found_match = true;
                }
            }

            // Handle LEFT/FULL joins when no match found
            if !found_match && matches!(join_clause.join_type, JoinType::Left | JoinType::FullOuter)
            {
                let mut null_ctx = ctx.clone();
                null_ctx.set_variable(&join_clause.variable, Value::Null);
                results.push(null_ctx);
            }
        }

        Ok(results)
    }

    /// Execute a COLLECT clause.
    fn execute_collect(
        &self,
        collect_clause: &CollectClause,
        contexts: Vec<ExecutionContext>,
    ) -> SdbqlResult<Vec<ExecutionContext>> {
        // Group by group_vars
        let mut groups: HashMap<String, (Value, Vec<ExecutionContext>)> = HashMap::new();

        for ctx in contexts {
            let mut group_key_parts = Vec::new();
            let mut group_values = serde_json::Map::new();

            for (var_name, expr) in &collect_clause.group_vars {
                let value = self.evaluate_expression(expr, &ctx)?;
                group_key_parts.push(serde_json::to_string(&value).unwrap_or_default());
                group_values.insert(var_name.clone(), value);
            }

            let group_key = group_key_parts.join("|");

            groups
                .entry(group_key)
                .or_insert_with(|| (Value::Object(group_values.clone()), Vec::new()))
                .1
                .push(ctx);
        }

        let mut results = Vec::new();

        for (_, (group_values, group_contexts)) in groups {
            let mut result_ctx = ExecutionContext::new(HashMap::new());

            // Set group variables
            if let Value::Object(obj) = &group_values {
                for (k, v) in obj {
                    result_ctx.set_variable(k, v.clone());
                }
            }

            // Set INTO variable (array of grouped items)
            if let Some(into_var) = &collect_clause.into_var {
                let items: Vec<Value> = group_contexts.iter().map(|c| c.to_value()).collect();
                result_ctx.set_variable(into_var, Value::Array(items));
            }

            // Set COUNT variable
            if let Some(count_var) = &collect_clause.count_var {
                result_ctx.set_variable(
                    count_var,
                    Value::Number(serde_json::Number::from(group_contexts.len())),
                );
            }

            // Compute aggregates
            for agg in &collect_clause.aggregates {
                let value = self.compute_aggregate(agg, &group_contexts)?;
                result_ctx.set_variable(&agg.variable, value);
            }

            results.push(result_ctx);
        }

        Ok(results)
    }

    /// Compute an aggregate value.
    fn compute_aggregate(
        &self,
        agg: &AggregateExpr,
        contexts: &[ExecutionContext],
    ) -> SdbqlResult<Value> {
        match agg.function.as_str() {
            "COUNT" => Ok(Value::Number(serde_json::Number::from(contexts.len()))),
            "SUM" => {
                let mut sum = 0.0;
                if let Some(arg) = &agg.argument {
                    for ctx in contexts {
                        let val = self.evaluate_expression(arg, ctx)?;
                        if let Some(n) = val.as_f64() {
                            sum += n;
                        }
                    }
                }
                Ok(Value::Number(number_from_f64(sum)))
            }
            "AVG" => {
                let mut sum = 0.0;
                let mut count = 0;
                if let Some(arg) = &agg.argument {
                    for ctx in contexts {
                        let val = self.evaluate_expression(arg, ctx)?;
                        if let Some(n) = val.as_f64() {
                            sum += n;
                            count += 1;
                        }
                    }
                }
                if count > 0 {
                    Ok(Value::Number(number_from_f64(sum / count as f64)))
                } else {
                    Ok(Value::Null)
                }
            }
            "MIN" => {
                let mut min: Option<Value> = None;
                if let Some(arg) = &agg.argument {
                    for ctx in contexts {
                        let val = self.evaluate_expression(arg, ctx)?;
                        if !val.is_null() {
                            min = Some(match min {
                                None => val,
                                Some(m) => {
                                    if compare_values(&val, &m) == std::cmp::Ordering::Less {
                                        val
                                    } else {
                                        m
                                    }
                                }
                            });
                        }
                    }
                }
                Ok(min.unwrap_or(Value::Null))
            }
            "MAX" => {
                let mut max: Option<Value> = None;
                if let Some(arg) = &agg.argument {
                    for ctx in contexts {
                        let val = self.evaluate_expression(arg, ctx)?;
                        if !val.is_null() {
                            max = Some(match max {
                                None => val,
                                Some(m) => {
                                    if compare_values(&val, &m) == std::cmp::Ordering::Greater {
                                        val
                                    } else {
                                        m
                                    }
                                }
                            });
                        }
                    }
                }
                Ok(max.unwrap_or(Value::Null))
            }
            _ => Err(SdbqlError::ExecutionError(format!(
                "Unknown aggregate function: {}",
                agg.function
            ))),
        }
    }

    /// Apply SORT clause.
    fn apply_sort(
        &self,
        results: &mut [Value],
        sort: &SortClause,
        context: &ExecutionContext,
    ) -> SdbqlResult<()> {
        let sort_fields = &sort.fields;

        results.sort_by(|a, b| {
            for (expr, ascending) in sort_fields {
                let ctx_a = ExecutionContext::from_value(a.clone(), context.bind_vars.clone());
                let ctx_b = ExecutionContext::from_value(b.clone(), context.bind_vars.clone());

                let val_a = self
                    .evaluate_expression(expr, &ctx_a)
                    .unwrap_or(Value::Null);
                let val_b = self
                    .evaluate_expression(expr, &ctx_b)
                    .unwrap_or(Value::Null);

                let ordering = compare_values(&val_a, &val_b);
                if ordering != std::cmp::Ordering::Equal {
                    return if *ascending {
                        ordering
                    } else {
                        ordering.reverse()
                    };
                }
            }
            std::cmp::Ordering::Equal
        });

        Ok(())
    }

    /// Apply LIMIT clause.
    fn apply_limit(
        &self,
        results: Vec<Value>,
        limit: &LimitClause,
        context: &ExecutionContext,
    ) -> SdbqlResult<Vec<Value>> {
        let offset = self
            .evaluate_expression(&limit.offset, context)?
            .as_u64()
            .unwrap_or(0) as usize;
        let count = self
            .evaluate_expression(&limit.count, context)?
            .as_u64()
            .unwrap_or(usize::MAX as u64) as usize;

        Ok(results.into_iter().skip(offset).take(count).collect())
    }

    /// Apply RETURN clause.
    fn apply_return(
        &self,
        results: Vec<Value>,
        return_clause: &ReturnClause,
        parent_context: &ExecutionContext,
    ) -> SdbqlResult<Vec<Value>> {
        let mut output = Vec::new();

        for result in results {
            // Create context from result and merge with parent context variables
            let mut ctx = ExecutionContext::from_value(result, parent_context.bind_vars.clone());
            // Inherit variables from parent context (LET bindings)
            for (k, v) in &parent_context.variables {
                if !ctx.variables.contains_key(k) {
                    ctx.variables.insert(k.clone(), v.clone());
                }
            }
            let value = self.evaluate_expression(&return_clause.expression, &ctx)?;
            output.push(value);
        }

        Ok(output)
    }

    /// Evaluate an expression.
    fn evaluate_expression(
        &self,
        expr: &Expression,
        context: &ExecutionContext,
    ) -> SdbqlResult<Value> {
        match expr {
            Expression::Literal(v) => Ok(v.clone()),

            Expression::Variable(name) => {
                Ok(context.get_variable(name).cloned().unwrap_or(Value::Null))
            }

            Expression::BindVariable(name) => {
                Ok(context.bind_vars.get(name).cloned().unwrap_or(Value::Null))
            }

            Expression::FieldAccess(base, field) => {
                let base_val = self.evaluate_expression(base, context)?;
                Ok(get_field_value(&base_val, field))
            }

            Expression::OptionalFieldAccess(base, field) => {
                let base_val = self.evaluate_expression(base, context)?;
                if base_val.is_null() {
                    Ok(Value::Null)
                } else {
                    Ok(get_field_value(&base_val, field))
                }
            }

            Expression::DynamicFieldAccess(base, index) => {
                let base_val = self.evaluate_expression(base, context)?;
                let index_val = self.evaluate_expression(index, context)?;
                if let Some(key) = index_val.as_str() {
                    Ok(get_field_value(&base_val, key))
                } else {
                    Ok(Value::Null)
                }
            }

            Expression::ArrayAccess(base, index) => {
                let base_val = self.evaluate_expression(base, context)?;
                let index_val = self.evaluate_expression(index, context)?;
                if let (Value::Array(arr), Some(idx)) = (&base_val, index_val.as_i64()) {
                    let idx = if idx < 0 {
                        (arr.len() as i64 + idx) as usize
                    } else {
                        idx as usize
                    };
                    Ok(arr.get(idx).cloned().unwrap_or(Value::Null))
                } else {
                    Ok(Value::Null)
                }
            }

            Expression::ArraySpreadAccess(base, field_path) => {
                let base_val = self.evaluate_expression(base, context)?;
                if let Value::Array(arr) = base_val {
                    let results: Vec<Value> = arr
                        .iter()
                        .map(|item| {
                            if let Some(path) = field_path {
                                get_field_value(item, path)
                            } else {
                                item.clone()
                            }
                        })
                        .collect();
                    Ok(Value::Array(results))
                } else {
                    Ok(Value::Null)
                }
            }

            Expression::BinaryOp { left, op, right } => {
                // Handle short-circuit evaluation
                match op {
                    BinaryOperator::And => {
                        let left_val = self.evaluate_expression(left, context)?;
                        if !to_bool(&left_val) {
                            return Ok(Value::Bool(false));
                        }
                        let right_val = self.evaluate_expression(right, context)?;
                        Ok(Value::Bool(to_bool(&right_val)))
                    }
                    BinaryOperator::Or => {
                        let left_val = self.evaluate_expression(left, context)?;
                        if to_bool(&left_val) {
                            return Ok(Value::Bool(true));
                        }
                        let right_val = self.evaluate_expression(right, context)?;
                        Ok(Value::Bool(to_bool(&right_val)))
                    }
                    BinaryOperator::NullCoalesce => {
                        let left_val = self.evaluate_expression(left, context)?;
                        if !left_val.is_null() {
                            return Ok(left_val);
                        }
                        self.evaluate_expression(right, context)
                    }
                    BinaryOperator::LogicalOr => {
                        let left_val = self.evaluate_expression(left, context)?;
                        if to_bool(&left_val) {
                            return Ok(left_val);
                        }
                        self.evaluate_expression(right, context)
                    }
                    _ => {
                        let left_val = self.evaluate_expression(left, context)?;
                        let right_val = self.evaluate_expression(right, context)?;
                        evaluate_binary_op(&left_val, op, &right_val)
                    }
                }
            }

            Expression::UnaryOp { op, operand } => {
                let operand_val = self.evaluate_expression(operand, context)?;
                evaluate_unary_op(op, &operand_val)
            }

            Expression::Object(fields) => {
                let mut obj = serde_json::Map::new();
                for (key, value_expr) in fields {
                    let value = self.evaluate_expression(value_expr, context)?;
                    obj.insert(key.clone(), value);
                }
                Ok(Value::Object(obj))
            }

            Expression::Array(elements) => {
                let mut arr = Vec::new();
                for elem in elements {
                    let value = self.evaluate_expression(elem, context)?;
                    arr.push(value);
                }
                Ok(Value::Array(arr))
            }

            Expression::Range(start, end) => {
                let start_val = self.evaluate_expression(start, context)?;
                let end_val = self.evaluate_expression(end, context)?;
                if let (Some(s), Some(e)) = (start_val.as_i64(), end_val.as_i64()) {
                    let arr: Vec<Value> = (s..=e)
                        .map(|i| Value::Number(serde_json::Number::from(i)))
                        .collect();
                    Ok(Value::Array(arr))
                } else {
                    Err(SdbqlError::ExecutionError(
                        "Range bounds must be integers".to_string(),
                    ))
                }
            }

            Expression::FunctionCall { name, args } => {
                self.evaluate_function_call(name, args, context)
            }

            Expression::Ternary {
                condition,
                true_expr,
                false_expr,
            } => {
                let cond = self.evaluate_expression(condition, context)?;
                if to_bool(&cond) {
                    self.evaluate_expression(true_expr, context)
                } else {
                    self.evaluate_expression(false_expr, context)
                }
            }

            Expression::Case {
                operand,
                when_clauses,
                else_clause,
            } => {
                let operand_val = if let Some(op) = operand {
                    Some(self.evaluate_expression(op, context)?)
                } else {
                    None
                };

                for (condition, result) in when_clauses {
                    let cond_val = self.evaluate_expression(condition, context)?;
                    let matches = if let Some(ref op_val) = operand_val {
                        values_equal(op_val, &cond_val)
                    } else {
                        to_bool(&cond_val)
                    };

                    if matches {
                        return self.evaluate_expression(result, context);
                    }
                }

                if let Some(else_expr) = else_clause {
                    self.evaluate_expression(else_expr, context)
                } else {
                    Ok(Value::Null)
                }
            }

            Expression::Pipeline { left, right } => {
                let left_val = self.evaluate_expression(left, context)?;
                // Insert left value as first argument to the function call
                if let Expression::FunctionCall { name, args } = right.as_ref() {
                    let mut new_args = vec![left_val];
                    for arg in args {
                        new_args.push(self.evaluate_expression(arg, context)?);
                    }
                    self.call_function(name, &new_args, context)
                } else {
                    Err(SdbqlError::ExecutionError(
                        "Pipeline right side must be a function call".to_string(),
                    ))
                }
            }

            Expression::Lambda { .. } => {
                // Lambdas are evaluated when called, return a placeholder
                Ok(Value::Null)
            }

            Expression::Subquery(query) => {
                let results = self.execute_query(query, context.bind_vars.clone())?;
                Ok(Value::Array(results))
            }

            Expression::TemplateString { parts } => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        TemplateStringPart::Literal(s) => result.push_str(s),
                        TemplateStringPart::Expression(expr) => {
                            let val = self.evaluate_expression(expr, context)?;
                            match val {
                                Value::String(s) => result.push_str(&s),
                                Value::Null => result.push_str("null"),
                                _ => result.push_str(&val.to_string()),
                            }
                        }
                    }
                }
                Ok(Value::String(result))
            }

            Expression::WindowFunctionCall { .. } => Err(SdbqlError::OperationNotSupported(
                "Window functions not supported in local queries".to_string(),
            )),
        }
    }

    /// Evaluate a function call.
    fn evaluate_function_call(
        &self,
        name: &str,
        args: &[Expression],
        context: &ExecutionContext,
    ) -> SdbqlResult<Value> {
        // Evaluate arguments (except for lambdas which are handled specially)
        let mut evaluated_args = Vec::new();
        let mut lambda_args = Vec::new();

        for (i, arg) in args.iter().enumerate() {
            if let Expression::Lambda { .. } = arg {
                evaluated_args.push(Value::Null); // Placeholder
                lambda_args.push((i, arg.clone()));
            } else {
                evaluated_args.push(self.evaluate_expression(arg, context)?);
            }
        }

        // Handle higher-order functions with lambdas
        if !lambda_args.is_empty() {
            return self.evaluate_higher_order_function(
                name,
                &evaluated_args,
                &lambda_args,
                context,
            );
        }

        self.call_function(name, &evaluated_args, context)
    }

    /// Call a builtin function.
    fn call_function(
        &self,
        name: &str,
        args: &[Value],
        _context: &ExecutionContext,
    ) -> SdbqlResult<Value> {
        BuiltinFunctions::call(name, args)
    }

    /// Evaluate higher-order functions (FILTER, MAP, etc.)
    fn evaluate_higher_order_function(
        &self,
        name: &str,
        args: &[Value],
        lambda_args: &[(usize, Expression)],
        context: &ExecutionContext,
    ) -> SdbqlResult<Value> {
        let upper_name = name.to_uppercase();

        match upper_name.as_str() {
            "FILTER" => {
                if args.is_empty() || lambda_args.is_empty() {
                    return Err(SdbqlError::ExecutionError(
                        "FILTER requires array and lambda".to_string(),
                    ));
                }
                let arr = args[0].as_array().ok_or_else(|| {
                    SdbqlError::ExecutionError("FILTER requires array".to_string())
                })?;
                let (_, lambda) = &lambda_args[0];

                if let Expression::Lambda { params, body } = lambda {
                    let mut results = Vec::new();
                    for item in arr {
                        let mut lambda_ctx = context.clone();
                        if let Some(param) = params.first() {
                            lambda_ctx.set_variable(param, item.clone());
                        }
                        let result = self.evaluate_expression(body, &lambda_ctx)?;
                        if to_bool(&result) {
                            results.push(item.clone());
                        }
                    }
                    Ok(Value::Array(results))
                } else {
                    Err(SdbqlError::ExecutionError("Invalid lambda".to_string()))
                }
            }

            "MAP" => {
                if args.is_empty() || lambda_args.is_empty() {
                    return Err(SdbqlError::ExecutionError(
                        "MAP requires array and lambda".to_string(),
                    ));
                }
                let arr = args[0]
                    .as_array()
                    .ok_or_else(|| SdbqlError::ExecutionError("MAP requires array".to_string()))?;
                let (_, lambda) = &lambda_args[0];

                if let Expression::Lambda { params, body } = lambda {
                    let mut results = Vec::new();
                    for item in arr {
                        let mut lambda_ctx = context.clone();
                        if let Some(param) = params.first() {
                            lambda_ctx.set_variable(param, item.clone());
                        }
                        let result = self.evaluate_expression(body, &lambda_ctx)?;
                        results.push(result);
                    }
                    Ok(Value::Array(results))
                } else {
                    Err(SdbqlError::ExecutionError("Invalid lambda".to_string()))
                }
            }

            "ANY" => {
                if args.is_empty() || lambda_args.is_empty() {
                    return Err(SdbqlError::ExecutionError(
                        "ANY requires array and lambda".to_string(),
                    ));
                }
                let arr = args[0]
                    .as_array()
                    .ok_or_else(|| SdbqlError::ExecutionError("ANY requires array".to_string()))?;
                let (_, lambda) = &lambda_args[0];

                if let Expression::Lambda { params, body } = lambda {
                    for item in arr {
                        let mut lambda_ctx = context.clone();
                        if let Some(param) = params.first() {
                            lambda_ctx.set_variable(param, item.clone());
                        }
                        let result = self.evaluate_expression(body, &lambda_ctx)?;
                        if to_bool(&result) {
                            return Ok(Value::Bool(true));
                        }
                    }
                    Ok(Value::Bool(false))
                } else {
                    Err(SdbqlError::ExecutionError("Invalid lambda".to_string()))
                }
            }

            "ALL" => {
                if args.is_empty() || lambda_args.is_empty() {
                    return Err(SdbqlError::ExecutionError(
                        "ALL requires array and lambda".to_string(),
                    ));
                }
                let arr = args[0]
                    .as_array()
                    .ok_or_else(|| SdbqlError::ExecutionError("ALL requires array".to_string()))?;
                let (_, lambda) = &lambda_args[0];

                if let Expression::Lambda { params, body } = lambda {
                    for item in arr {
                        let mut lambda_ctx = context.clone();
                        if let Some(param) = params.first() {
                            lambda_ctx.set_variable(param, item.clone());
                        }
                        let result = self.evaluate_expression(body, &lambda_ctx)?;
                        if !to_bool(&result) {
                            return Ok(Value::Bool(false));
                        }
                    }
                    Ok(Value::Bool(true))
                } else {
                    Err(SdbqlError::ExecutionError("Invalid lambda".to_string()))
                }
            }

            "REDUCE" => {
                if args.len() < 2 || lambda_args.is_empty() {
                    return Err(SdbqlError::ExecutionError(
                        "REDUCE requires array, initial value, and lambda".to_string(),
                    ));
                }
                let arr = args[0].as_array().ok_or_else(|| {
                    SdbqlError::ExecutionError("REDUCE requires array".to_string())
                })?;
                let initial = args[1].clone();
                let (_, lambda) = &lambda_args[0];

                if let Expression::Lambda { params, body } = lambda {
                    let mut acc = initial;
                    for item in arr {
                        let mut lambda_ctx = context.clone();
                        if params.len() >= 2 {
                            lambda_ctx.set_variable(&params[0], acc);
                            lambda_ctx.set_variable(&params[1], item.clone());
                        }
                        acc = self.evaluate_expression(body, &lambda_ctx)?;
                    }
                    Ok(acc)
                } else {
                    Err(SdbqlError::ExecutionError("Invalid lambda".to_string()))
                }
            }

            _ => Err(SdbqlError::ExecutionError(format!(
                "Unknown higher-order function: {}",
                name
            ))),
        }
    }
}

/// Execution context holding variables and bind vars.
#[derive(Clone)]
struct ExecutionContext {
    variables: HashMap<String, Value>,
    bind_vars: HashMap<String, Value>,
}

impl ExecutionContext {
    fn new(bind_vars: HashMap<String, Value>) -> Self {
        Self {
            variables: HashMap::new(),
            bind_vars,
        }
    }

    fn from_value(value: Value, bind_vars: HashMap<String, Value>) -> Self {
        let mut ctx = Self::new(bind_vars);
        if let Value::Object(obj) = value {
            for (k, v) in obj {
                ctx.variables.insert(k, v);
            }
        }
        ctx
    }

    fn get_variable(&self, name: &str) -> Option<&Value> {
        self.variables.get(name)
    }

    fn set_variable(&mut self, name: &str, value: Value) {
        self.variables.insert(name.to_string(), value);
    }

    fn to_value(&self) -> Value {
        Value::Object(
            self.variables
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::InMemoryDataSource;
    use serde_json::json;

    fn create_test_executor() -> LocalExecutor<InMemoryDataSource> {
        let mut ds = InMemoryDataSource::new();
        ds.add_collection(
            "users",
            vec![
                json!({"_key": "1", "name": "Alice", "age": 30, "city": "NYC"}),
                json!({"_key": "2", "name": "Bob", "age": 25, "city": "LA"}),
                json!({"_key": "3", "name": "Charlie", "age": 35, "city": "NYC"}),
            ],
        );
        ds.add_collection(
            "orders",
            vec![
                json!({"_key": "o1", "user_key": "1", "amount": 100}),
                json!({"_key": "o2", "user_key": "2", "amount": 200}),
                json!({"_key": "o3", "user_key": "1", "amount": 150}),
            ],
        );
        LocalExecutor::new(ds)
    }

    #[test]
    fn test_simple_query() {
        let executor = create_test_executor();
        let results = executor
            .execute("FOR doc IN users RETURN doc.name", None)
            .unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_filter_query() {
        let executor = create_test_executor();
        let results = executor
            .execute("FOR doc IN users FILTER doc.age > 28 RETURN doc.name", None)
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.contains(&json!("Alice")));
        assert!(results.contains(&json!("Charlie")));
    }

    #[test]
    fn test_sort_query() {
        let executor = create_test_executor();
        let results = executor
            .execute("FOR doc IN users SORT doc.age DESC RETURN doc.name", None)
            .unwrap();
        assert_eq!(results[0], json!("Charlie"));
        assert_eq!(results[1], json!("Alice"));
        assert_eq!(results[2], json!("Bob"));
    }

    #[test]
    fn test_limit_query() {
        let executor = create_test_executor();
        let results = executor
            .execute("FOR doc IN users LIMIT 2 RETURN doc.name", None)
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_let_clause() {
        let executor = create_test_executor();
        let results = executor.execute("LET x = 10 RETURN x * 2", None).unwrap();
        assert_eq!(results[0], json!(20.0));
    }

    #[test]
    fn test_bind_variables() {
        let executor = create_test_executor();
        let mut bind_vars = HashMap::new();
        bind_vars.insert("min_age".to_string(), json!(30));

        let results = executor
            .execute(
                "FOR doc IN users FILTER doc.age >= @min_age RETURN doc.name",
                Some(bind_vars),
            )
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_range_iteration() {
        let executor = create_test_executor();
        let results = executor
            .execute("FOR i IN 1..5 RETURN i * 2", None)
            .unwrap();
        assert_eq!(
            results,
            vec![json!(2.0), json!(4.0), json!(6.0), json!(8.0), json!(10.0)]
        );
    }

    #[test]
    fn test_collect_with_count() {
        let executor = create_test_executor();
        let results = executor
            .execute(
                "FOR doc IN users COLLECT city = doc.city WITH COUNT INTO count RETURN {city, count}",
                None,
            )
            .unwrap();
        assert_eq!(results.len(), 2); // NYC and LA
    }

    #[test]
    fn test_function_call() {
        let executor = create_test_executor();
        let results = executor.execute("RETURN LENGTH([1, 2, 3])", None).unwrap();
        assert_eq!(results[0], json!(3));
    }

    #[test]
    fn test_array_filter() {
        let executor = create_test_executor();
        let results = executor
            .execute("RETURN FILTER([1, 2, 3, 4, 5], x -> x > 3)", None)
            .unwrap();
        assert_eq!(results[0], json!([4, 5]));
    }

    #[test]
    fn test_array_map() {
        let executor = create_test_executor();
        let results = executor
            .execute("RETURN MAP([1, 2, 3], x -> x * 2)", None)
            .unwrap();
        assert_eq!(results[0], json!([2.0, 4.0, 6.0]));
    }

    #[test]
    fn test_ternary_expression() {
        let executor = create_test_executor();
        let results = executor
            .execute("RETURN true ? 'yes' : 'no'", None)
            .unwrap();
        assert_eq!(results[0], json!("yes"));
    }

    #[test]
    fn test_null_coalesce() {
        let executor = create_test_executor();
        let results = executor
            .execute("LET x = null RETURN x ?? 'default'", None)
            .unwrap();
        assert_eq!(results[0], json!("default"));
    }

    #[test]
    fn test_subquery() {
        let executor = create_test_executor();
        let results = executor
            .execute(
                "LET names = (FOR doc IN users RETURN doc.name) RETURN names",
                None,
            )
            .unwrap();
        assert!(results[0].is_array());
        assert_eq!(results[0].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_insert_rejected() {
        let executor = create_test_executor();
        let result = executor.execute("INSERT {name: 'test'} INTO users", None);
        assert!(result.is_err());
        if let Err(SdbqlError::OperationNotSupported(_)) = result {
            // Expected
        } else {
            panic!("Expected OperationNotSupported error");
        }
    }

    #[test]
    fn test_object_construction() {
        let executor = create_test_executor();
        let results = executor
            .execute("RETURN {a: 1, b: 'hello', c: [1, 2, 3]}", None)
            .unwrap();
        assert_eq!(results[0], json!({"a": 1, "b": "hello", "c": [1, 2, 3]}));
    }
}
