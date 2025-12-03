use serde_json::{Value, json};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::time::{Instant, Duration};

use crate::error::{DbError, DbResult};
use crate::storage::{Collection, StorageEngine, GeoPoint, distance_meters};
use super::ast::*;

/// Execution context holding variable bindings
type Context = HashMap<String, Value>;

/// Bind variables for parameterized queries (prevents AQL injection)
pub type BindVars = HashMap<String, Value>;

/// Query execution plan with timing information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryExplain {
    /// Collections accessed by the query
    pub collections: Vec<CollectionAccess>,
    /// LET clause bindings
    pub let_bindings: Vec<LetBinding>,
    /// Filter conditions analyzed
    pub filters: Vec<FilterInfo>,
    /// Sort information
    pub sort: Option<SortInfo>,
    /// Limit information
    pub limit: Option<LimitInfo>,
    /// Execution timing for each step (in microseconds)
    pub timing: ExecutionTiming,
    /// Total documents scanned
    pub documents_scanned: usize,
    /// Total documents returned
    pub documents_returned: usize,
    /// Warnings or suggestions
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionAccess {
    pub name: String,
    pub variable: String,
    pub access_type: String, // "full_scan" or "index_lookup"
    pub index_used: Option<String>,
    pub index_type: Option<String>,
    pub documents_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LetBinding {
    pub variable: String,
    pub is_subquery: bool,
    pub time_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterInfo {
    pub expression: String,
    pub index_candidate: Option<String>,
    pub can_use_index: bool,
    pub documents_before: usize,
    pub documents_after: usize,
    pub time_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortInfo {
    pub field: String,
    pub direction: String,
    pub time_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitInfo {
    pub offset: usize,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTiming {
    pub total_us: u64,
    pub let_clauses_us: u64,
    pub collection_scan_us: u64,
    pub filter_us: u64,
    pub sort_us: u64,
    pub limit_us: u64,
    pub return_projection_us: u64,
}

pub struct QueryExecutor<'a> {
    storage: &'a StorageEngine,
    bind_vars: BindVars,
}

/// Extracted filter condition for index optimization
#[derive(Debug)]
struct IndexableCondition {
    field: String,
    op: BinaryOperator,
    value: Value,
}

impl<'a> QueryExecutor<'a> {
    pub fn new(storage: &'a StorageEngine) -> Self {
        Self {
            storage,
            bind_vars: HashMap::new(),
        }
    }

    /// Create executor with bind variables for parameterized queries
    pub fn with_bind_vars(storage: &'a StorageEngine, bind_vars: BindVars) -> Self {
        Self { storage, bind_vars }
    }

    pub fn execute(&self, query: &Query) -> DbResult<Vec<Value>> {
        // First, evaluate initial LET clauses (before any FOR) to create initial bindings
        let mut initial_bindings: Context = HashMap::new();

        // Merge bind variables into initial context
        for (key, value) in &self.bind_vars {
            initial_bindings.insert(format!("@{}", key), value.clone());
        }

        for let_clause in &query.let_clauses {
            let value = self.evaluate_expr_with_context(&let_clause.expression, &initial_bindings)?;
            initial_bindings.insert(let_clause.variable.clone(), value);
        }

        // Process body_clauses in order (supports correlated subqueries)
        // If body_clauses is empty, fall back to legacy behavior
        let rows = if !query.body_clauses.is_empty() {
            self.execute_body_clauses(&query.body_clauses, &initial_bindings)?
        } else {
            // Legacy path: use for_clauses and filter_clauses separately
            let mut rows = self.build_row_combinations_with_context(&query.for_clauses, &initial_bindings)?;
            for filter in &query.filter_clauses {
                rows.retain(|ctx| {
                    self.evaluate_filter_with_context(&filter.expression, ctx)
                        .unwrap_or(false)
                });
            }
            rows
        };

        let mut rows = rows;

        // Apply SORT
        if let Some(sort) = &query.sort_clause {
            let field_path = &sort.field;
            let ascending = sort.ascending;

            rows.sort_by(|a, b| {
                let a_val = self.evaluate_expr_with_context(
                    &parse_field_expr(field_path), a
                ).unwrap_or(Value::Null);
                let b_val = self.evaluate_expr_with_context(
                    &parse_field_expr(field_path), b
                ).unwrap_or(Value::Null);

                let cmp = compare_values(&a_val, &b_val);
                if ascending { cmp } else { cmp.reverse() }
            });
        }

        // Apply LIMIT
        if let Some(limit) = &query.limit_clause {
            let start = limit.offset.min(rows.len());
            let end = (start + limit.count).min(rows.len());
            rows = rows[start..end].to_vec();
        }

        // Apply RETURN projection
        let results: DbResult<Vec<Value>> = rows
            .iter()
            .map(|ctx| self.evaluate_expr_with_context(&query.return_clause.expression, ctx))
            .collect();

        results
    }

    /// Execute body clauses in order, supporting correlated subqueries
    /// LET clauses inside FOR loops are evaluated per-row with access to outer variables
    fn execute_body_clauses(&self, clauses: &[BodyClause], initial_ctx: &Context) -> DbResult<Vec<Context>> {
        let mut rows: Vec<Context> = vec![initial_ctx.clone()];

        for clause in clauses {
            match clause {
                BodyClause::For(for_clause) => {
                    // Expand each current row with documents from the collection/variable
                    let mut new_rows = Vec::new();
                    for ctx in &rows {
                        let docs = self.get_for_source_docs(for_clause, ctx)?;
                        for doc in docs {
                            let mut new_ctx = ctx.clone();
                            new_ctx.insert(for_clause.variable.clone(), doc);
                            new_rows.push(new_ctx);
                        }
                    }
                    rows = new_rows;
                }
                BodyClause::Let(let_clause) => {
                    // Evaluate LET expression for EACH row (correlated subquery support)
                    for ctx in &mut rows {
                        let value = self.evaluate_expr_with_context(&let_clause.expression, ctx)?;
                        ctx.insert(let_clause.variable.clone(), value);
                    }
                }
                BodyClause::Filter(filter_clause) => {
                    // Filter out rows that don't match
                    rows.retain(|ctx| {
                        self.evaluate_filter_with_context(&filter_clause.expression, ctx)
                            .unwrap_or(false)
                    });
                }
            }
        }

        Ok(rows)
    }

    /// Get documents for a FOR clause source (collection or variable)
    fn get_for_source_docs(&self, for_clause: &ForClause, ctx: &Context) -> DbResult<Vec<Value>> {
        let source_name = for_clause.source_variable.as_ref()
            .unwrap_or(&for_clause.collection);

        // Check if source is a LET variable in current context
        if let Some(value) = ctx.get(source_name) {
            return match value {
                Value::Array(arr) => Ok(arr.clone()),
                other => Ok(vec![other.clone()]),
            };
        }

        // Otherwise it's a collection
        let collection = self.storage.get_collection(&for_clause.collection)?;
        Ok(collection.all().into_iter().map(|d| d.to_value()).collect())
    }

    /// Explain and profile a query execution
    pub fn explain(&self, query: &Query) -> DbResult<QueryExplain> {
        let total_start = Instant::now();
        let mut warnings: Vec<String> = Vec::new();
        let mut collections_info: Vec<CollectionAccess> = Vec::new();
        let mut let_bindings_info: Vec<LetBinding> = Vec::new();
        let mut filters_info: Vec<FilterInfo> = Vec::new();

        // Timing accumulators
        let mut let_clauses_time = Duration::ZERO;
        let mut collection_scan_time = Duration::ZERO;
        let mut filter_time = Duration::ZERO;
        let mut sort_time = Duration::ZERO;
        let mut limit_time = Duration::ZERO;
        let mut return_time = Duration::ZERO;

        // First, evaluate all LET clauses
        let let_start = Instant::now();
        let mut let_bindings: Context = HashMap::new();

        for (key, value) in &self.bind_vars {
            let_bindings.insert(format!("@{}", key), value.clone());
        }

        for let_clause in &query.let_clauses {
            let clause_start = Instant::now();
            let is_subquery = matches!(let_clause.expression, Expression::Subquery(_));
            let value = self.evaluate_expr_with_context(&let_clause.expression, &let_bindings)?;
            let_bindings.insert(let_clause.variable.clone(), value);
            let clause_time = clause_start.elapsed();

            let_bindings_info.push(LetBinding {
                variable: let_clause.variable.clone(),
                is_subquery,
                time_us: clause_time.as_micros() as u64,
            });
        }
        let_clauses_time = let_start.elapsed();

        // Analyze FOR clauses and build row combinations
        let scan_start = Instant::now();
        let mut total_docs_scanned = 0usize;

        for for_clause in &query.for_clauses {
            let source_name = for_clause.source_variable.as_ref()
                .unwrap_or(&for_clause.collection);

            // Check if source is a LET variable or collection
            let (docs_count, access_type, index_used, index_type) = if let_bindings.contains_key(source_name) {
                let arr_len = match let_bindings.get(source_name) {
                    Some(Value::Array(arr)) => arr.len(),
                    Some(_) => 1,
                    None => 0,
                };
                (arr_len, "variable_iteration".to_string(), None, None)
            } else {
                // It's a collection - check for potential index usage
                let collection = self.storage.get_collection(&for_clause.collection)?;
                let docs: Vec<_> = collection.all();
                let doc_count = docs.len();
                total_docs_scanned += doc_count;

                // Check if any filter can use an index
                let mut found_index: Option<(String, String)> = None;
                for filter in &query.filter_clauses {
                    if let Some(condition) = self.extract_indexable_condition(&filter.expression, &for_clause.variable) {
                        let indexes = collection.list_indexes();
                        for idx in &indexes {
                            if idx.field == condition.field {
                                found_index = Some((idx.name.clone(), format!("{:?}", idx.index_type)));
                                break;
                            }
                        }
                    }
                }

                if found_index.is_none() && doc_count > 100 {
                    warnings.push(format!(
                        "Full collection scan on '{}' ({} documents). Consider adding an index.",
                        for_clause.collection, doc_count
                    ));
                }

                let access = if found_index.is_some() { "index_lookup" } else { "full_scan" };
                (doc_count, access.to_string(), found_index.as_ref().map(|(n, _)| n.clone()), found_index.map(|(_, t)| t))
            };

            collections_info.push(CollectionAccess {
                name: for_clause.collection.clone(),
                variable: for_clause.variable.clone(),
                access_type,
                index_used,
                index_type,
                documents_count: docs_count,
            });
        }

        let mut rows = self.build_row_combinations_with_context(&query.for_clauses, &let_bindings)?;
        collection_scan_time = scan_start.elapsed();
        let rows_after_scan = rows.len();

        // Apply and analyze FILTER clauses
        let filter_start = Instant::now();
        for filter in &query.filter_clauses {
            let before_count = rows.len();
            let clause_start = Instant::now();

            rows.retain(|ctx| {
                self.evaluate_filter_with_context(&filter.expression, ctx)
                    .unwrap_or(false)
            });

            let clause_time = clause_start.elapsed();
            let after_count = rows.len();

            // Try to find index candidate for this filter
            let mut index_candidate = None;
            let mut can_use_index = false;

            if !query.for_clauses.is_empty() {
                let var_name = &query.for_clauses[0].variable;
                if let Some(condition) = self.extract_indexable_condition(&filter.expression, var_name) {
                    index_candidate = Some(condition.field.clone());
                    // Check if index exists
                    if let Ok(collection) = self.storage.get_collection(&query.for_clauses[0].collection) {
                        for idx in collection.list_indexes() {
                            if idx.field == condition.field {
                                can_use_index = true;
                                break;
                            }
                        }
                    }
                }
            }

            filters_info.push(FilterInfo {
                expression: format!("{:?}", filter.expression),
                index_candidate,
                can_use_index,
                documents_before: before_count,
                documents_after: after_count,
                time_us: clause_time.as_micros() as u64,
            });
        }
        filter_time = filter_start.elapsed();

        // Apply SORT
        let sort_start = Instant::now();
        let sort_info = if let Some(sort) = &query.sort_clause {
            let field_path = &sort.field;
            let ascending = sort.ascending;

            rows.sort_by(|a, b| {
                let a_val = self.evaluate_expr_with_context(
                    &parse_field_expr(field_path), a
                ).unwrap_or(Value::Null);
                let b_val = self.evaluate_expr_with_context(
                    &parse_field_expr(field_path), b
                ).unwrap_or(Value::Null);

                let cmp = compare_values(&a_val, &b_val);
                if ascending { cmp } else { cmp.reverse() }
            });

            Some(SortInfo {
                field: sort.field.clone(),
                direction: if sort.ascending { "ASC".to_string() } else { "DESC".to_string() },
                time_us: 0, // Will be set below
            })
        } else {
            None
        };
        sort_time = sort_start.elapsed();

        let sort_info = sort_info.map(|mut s| {
            s.time_us = sort_time.as_micros() as u64;
            s
        });

        // Apply LIMIT
        let limit_start = Instant::now();
        let limit_info = if let Some(limit) = &query.limit_clause {
            let start = limit.offset.min(rows.len());
            let end = (start + limit.count).min(rows.len());
            rows = rows[start..end].to_vec();

            Some(LimitInfo {
                offset: limit.offset,
                count: limit.count,
            })
        } else {
            None
        };
        limit_time = limit_start.elapsed();

        // Apply RETURN projection
        let return_start = Instant::now();
        let results: DbResult<Vec<Value>> = rows
            .iter()
            .map(|ctx| self.evaluate_expr_with_context(&query.return_clause.expression, ctx))
            .collect();
        let results = results?;
        return_time = return_start.elapsed();

        let total_time = total_start.elapsed();

        // Add warnings for slow operations
        if filter_time.as_millis() > 100 {
            warnings.push(format!(
                "Filter operations took {}ms. Consider adding indexes on filtered fields.",
                filter_time.as_millis()
            ));
        }

        if sort_time.as_millis() > 100 && rows_after_scan > 1000 {
            warnings.push(format!(
                "Sort operation on {} rows took {}ms. Consider adding a persistent index for sorting.",
                rows_after_scan, sort_time.as_millis()
            ));
        }

        Ok(QueryExplain {
            collections: collections_info,
            let_bindings: let_bindings_info,
            filters: filters_info,
            sort: sort_info,
            limit: limit_info,
            timing: ExecutionTiming {
                total_us: total_time.as_micros() as u64,
                let_clauses_us: let_clauses_time.as_micros() as u64,
                collection_scan_us: collection_scan_time.as_micros() as u64,
                filter_us: filter_time.as_micros() as u64,
                sort_us: sort_time.as_micros() as u64,
                limit_us: limit_time.as_micros() as u64,
                return_projection_us: return_time.as_micros() as u64,
            },
            documents_scanned: total_docs_scanned,
            documents_returned: results.len(),
            warnings,
        })
    }

    /// Build all row combinations from multiple FOR clauses
    /// This creates the Cartesian product for JOINs
    fn build_row_combinations(&self, for_clauses: &[ForClause]) -> DbResult<Vec<Context>> {
        self.build_row_combinations_with_context(for_clauses, &HashMap::new())
    }

    /// Build all row combinations from multiple FOR clauses with initial context (LET bindings)
    /// This creates the Cartesian product for JOINs
    fn build_row_combinations_with_context(
        &self,
        for_clauses: &[ForClause],
        let_bindings: &Context
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
            let source_name = for_clause.source_variable.as_ref()
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
                let collection = self.storage.get_collection(&for_clause.collection)?;
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
    fn evaluate_filter_with_context(&self, expr: &Expression, ctx: &Context) -> DbResult<bool> {
        match self.evaluate_expr_with_context(expr, ctx)? {
            Value::Bool(b) => Ok(b),
            _ => Ok(false),
        }
    }

    /// Evaluate an expression with a context containing multiple variables
    fn evaluate_expr_with_context(&self, expr: &Expression, ctx: &Context) -> DbResult<Value> {
        match expr {
            Expression::Variable(name) => {
                ctx.get(name)
                    .cloned()
                    .ok_or_else(|| DbError::ExecutionError(format!("Variable '{}' not found", name)))
            }

            Expression::BindVariable(name) => {
                // First check context (bind vars are stored with @ prefix)
                if let Some(value) = ctx.get(&format!("@{}", name)) {
                    return Ok(value.clone());
                }
                // Then check bind_vars directly
                self.bind_vars.get(name)
                    .cloned()
                    .ok_or_else(|| DbError::ExecutionError(format!("Bind variable '@{}' not found. Did you forget to pass it in bindVars?", name)))
            }

            Expression::FieldAccess(base, field) => {
                let base_value = self.evaluate_expr_with_context(base, ctx)?;
                Ok(get_field_value(&base_value, field))
            }

            Expression::DynamicFieldAccess(base, field_expr) => {
                let base_value = self.evaluate_expr_with_context(base, ctx)?;
                let field_value = self.evaluate_expr_with_context(field_expr, ctx)?;

                // The field expression should evaluate to a string (field name)
                let field_name = match field_value {
                    Value::String(s) => s,
                    Value::Number(n) => n.to_string(),
                    _ => return Err(DbError::ExecutionError(
                        format!("Dynamic field access requires a string or number, got: {:?}", field_value)
                    )),
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
                                return Err(DbError::ExecutionError(
                                    format!("Array index must be non-negative, got: {}", f)
                                ));
                            }
                            f as usize
                        } else {
                            return Err(DbError::ExecutionError(
                                format!("Invalid array index: {}", n)
                            ));
                        }
                    }
                    _ => return Err(DbError::ExecutionError(
                        format!("Array index must be a number, got: {:?}", index_value)
                    )),
                };

                // Access the array element
                match base_value {
                    Value::Array(ref arr) => {
                        Ok(arr.get(index).cloned().unwrap_or(Value::Null))
                    }
                    _ => Ok(Value::Null), // Non-arrays return null
                }
            }


            Expression::Literal(value) => Ok(value.clone()),

            Expression::BinaryOp { left, op, right } => {
                let left_val = self.evaluate_expr_with_context(left, ctx)?;
                let right_val = self.evaluate_expr_with_context(right, ctx)?;
                evaluate_binary_op(&left_val, op, &right_val)
            }

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

            Expression::FunctionCall { name, args } => {
                self.evaluate_function(name, args, ctx)
            }

            Expression::Subquery(subquery) => {
                // Execute the subquery with parent context (enables correlated subqueries)
                let results = self.execute_with_parent_context(subquery, ctx)?;
                Ok(Value::Array(results))
            }
        }
    }

    /// Execute a subquery with access to parent context (for correlated subqueries)
    fn execute_with_parent_context(&self, query: &Query, parent_ctx: &Context) -> DbResult<Vec<Value>> {
        // Start with parent context (enables correlation with outer query)
        let mut initial_bindings = parent_ctx.clone();

        // Add bind variables
        for (key, value) in &self.bind_vars {
            initial_bindings.insert(format!("@{}", key), value.clone());
        }

        // Evaluate initial LET clauses (before FOR)
        for let_clause in &query.let_clauses {
            let value = self.evaluate_expr_with_context(&let_clause.expression, &initial_bindings)?;
            initial_bindings.insert(let_clause.variable.clone(), value);
        }

        // Process body_clauses in order
        let rows = if !query.body_clauses.is_empty() {
            self.execute_body_clauses(&query.body_clauses, &initial_bindings)?
        } else {
            let mut rows = self.build_row_combinations_with_context(&query.for_clauses, &initial_bindings)?;
            for filter in &query.filter_clauses {
                rows.retain(|ctx| {
                    self.evaluate_filter_with_context(&filter.expression, ctx)
                        .unwrap_or(false)
                });
            }
            rows
        };

        let mut rows = rows;

        // Apply SORT
        if let Some(sort) = &query.sort_clause {
            let field_path = &sort.field;
            let ascending = sort.ascending;

            rows.sort_by(|a, b| {
                let a_val = self.evaluate_expr_with_context(
                    &parse_field_expr(field_path), a
                ).unwrap_or(Value::Null);
                let b_val = self.evaluate_expr_with_context(
                    &parse_field_expr(field_path), b
                ).unwrap_or(Value::Null);

                let cmp = compare_values(&a_val, &b_val);
                if ascending { cmp } else { cmp.reverse() }
            });
        }

        // Apply LIMIT
        if let Some(limit) = &query.limit_clause {
            let start = limit.offset.min(rows.len());
            let end = (start + limit.count).min(rows.len());
            rows = rows[start..end].to_vec();
        }

        // Apply RETURN projection
        let results: DbResult<Vec<Value>> = rows
            .iter()
            .map(|ctx| self.evaluate_expr_with_context(&query.return_clause.expression, ctx))
            .collect();

        results
    }

    /// Evaluate a function call
    fn evaluate_function(&self, name: &str, args: &[Expression], ctx: &Context) -> DbResult<Value> {
        // Evaluate all arguments
        let evaluated_args: Vec<Value> = args
            .iter()
            .map(|arg| self.evaluate_expr_with_context(arg, ctx))
            .collect::<DbResult<Vec<_>>>()?;

        match name.to_uppercase().as_str() {
            // DISTANCE(lat1, lon1, lat2, lon2) - distance between two points in meters
            "DISTANCE" => {
                if evaluated_args.len() != 4 {
                    return Err(DbError::ExecutionError(
                        "DISTANCE requires 4 arguments: lat1, lon1, lat2, lon2".to_string()
                    ));
                }
                let lat1 = evaluated_args[0].as_f64()
                    .ok_or_else(|| DbError::ExecutionError("DISTANCE: lat1 must be a number".to_string()))?;
                let lon1 = evaluated_args[1].as_f64()
                    .ok_or_else(|| DbError::ExecutionError("DISTANCE: lon1 must be a number".to_string()))?;
                let lat2 = evaluated_args[2].as_f64()
                    .ok_or_else(|| DbError::ExecutionError("DISTANCE: lat2 must be a number".to_string()))?;
                let lon2 = evaluated_args[3].as_f64()
                    .ok_or_else(|| DbError::ExecutionError("DISTANCE: lon2 must be a number".to_string()))?;

                let dist = distance_meters(lat1, lon1, lat2, lon2);
                Ok(Value::Number(serde_json::Number::from_f64(dist).unwrap()))
            }

            // GEO_DISTANCE(geopoint1, geopoint2) - distance between two geo points
            "GEO_DISTANCE" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "GEO_DISTANCE requires 2 arguments: point1, point2".to_string()
                    ));
                }
                let p1 = GeoPoint::from_value(&evaluated_args[0])
                    .ok_or_else(|| DbError::ExecutionError("GEO_DISTANCE: first argument must be a geo point".to_string()))?;
                let p2 = GeoPoint::from_value(&evaluated_args[1])
                    .ok_or_else(|| DbError::ExecutionError("GEO_DISTANCE: second argument must be a geo point".to_string()))?;

                let dist = distance_meters(p1.lat, p1.lon, p2.lat, p2.lon);
                Ok(Value::Number(serde_json::Number::from_f64(dist).unwrap()))
            }

            // LENGTH(array_or_string) - get length
            "LENGTH" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("LENGTH requires 1 argument".to_string()));
                }
                let len = match &evaluated_args[0] {
                    Value::Array(arr) => arr.len(),
                    Value::String(s) => s.len(),
                    Value::Object(obj) => obj.len(),
                    _ => return Err(DbError::ExecutionError("LENGTH: argument must be array, string, or object".to_string())),
                };
                Ok(Value::Number(serde_json::Number::from(len)))
            }

            // SUM(array) - sum of numeric array elements
            "SUM" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("SUM requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("SUM: argument must be an array".to_string()))?;

                let sum: f64 = arr.iter()
                    .filter_map(|v| v.as_f64())
                    .sum();

                Ok(Value::Number(serde_json::Number::from_f64(sum).unwrap_or(serde_json::Number::from(0))))
            }

            // AVG(array) - average of numeric array elements
            "AVG" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("AVG requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("AVG: argument must be an array".to_string()))?;

                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                let avg = nums.iter().sum::<f64>() / nums.len() as f64;
                Ok(Value::Number(serde_json::Number::from_f64(avg).unwrap_or(serde_json::Number::from(0))))
            }

            // MIN(array) - minimum value in array
            "MIN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("MIN requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("MIN: argument must be an array".to_string()))?;

                let min = arr.iter()
                    .filter_map(|v| v.as_f64())
                    .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                match min {
                    Some(n) => Ok(Value::Number(serde_json::Number::from_f64(n).unwrap())),
                    None => Ok(Value::Null),
                }
            }

            // MAX(array) - maximum value in array
            "MAX" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("MAX requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("MAX: argument must be an array".to_string()))?;

                let max = arr.iter()
                    .filter_map(|v| v.as_f64())
                    .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                match max {
                    Some(n) => Ok(Value::Number(serde_json::Number::from_f64(n).unwrap())),
                    None => Ok(Value::Null),
                }
            }

            // ROUND(number, precision?) - round a number
            "ROUND" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                    return Err(DbError::ExecutionError("ROUND requires 1-2 arguments".to_string()));
                }
                let num = evaluated_args[0].as_f64()
                    .ok_or_else(|| DbError::ExecutionError("ROUND: first argument must be a number".to_string()))?;
                let precision = if evaluated_args.len() > 1 {
                    evaluated_args[1].as_i64().unwrap_or(0) as i32
                } else {
                    0
                };
                let factor = 10_f64.powi(precision);
                let rounded = (num * factor).round() / factor;
                Ok(Value::Number(serde_json::Number::from_f64(rounded).unwrap()))
            }

            // ABS(number) - absolute value
            "ABS" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("ABS requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64()
                    .ok_or_else(|| DbError::ExecutionError("ABS: argument must be a number".to_string()))?;
                Ok(Value::Number(serde_json::Number::from_f64(num.abs()).unwrap()))
            }

            // FLOOR(number) - floor
            "FLOOR" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("FLOOR requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64()
                    .ok_or_else(|| DbError::ExecutionError("FLOOR: argument must be a number".to_string()))?;
                Ok(Value::Number(serde_json::Number::from_f64(num.floor()).unwrap()))
            }

            // CEIL(number) - ceiling
            "CEIL" => {
                
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("CEIL requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64()
                    .ok_or_else(|| DbError::ExecutionError("CEIL: argument must be a number".to_string()))?;
                Ok(Value::Number(serde_json::Number::from_f64(num.ceil()).unwrap()))
            }

            // UPPER(string) - uppercase
            "UPPER" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("UPPER requires 1 argument".to_string()));
                }
                let s = evaluated_args[0].as_str()
                    .ok_or_else(|| DbError::ExecutionError("UPPER: argument must be a string".to_string()))?;
                Ok(Value::String(s.to_uppercase()))
            }

            // LOWER(string) - lowercase
            "LOWER" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("LOWER requires 1 argument".to_string()));
                }
                let s = evaluated_args[0].as_str()
                    .ok_or_else(|| DbError::ExecutionError("LOWER: argument must be a string".to_string()))?;
                Ok(Value::String(s.to_lowercase()))
            }

            // CONCAT(str1, str2, ...) - concatenate strings
            "CONCAT" => {
                let mut result = String::new();
                for arg in &evaluated_args {
                    match arg {
                        Value::String(s) => result.push_str(s),
                        Value::Number(n) => result.push_str(&n.to_string()),
                        Value::Bool(b) => result.push_str(&b.to_string()),
                        Value::Null => result.push_str("null"),
                        _ => return Err(DbError::ExecutionError("CONCAT: arguments must be strings or primitives".to_string())),
                    }
                }
                Ok(Value::String(result))
            }

            // CONCAT_SEPARATOR(separator, array) - join array elements with separator
            "CONCAT_SEPARATOR" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError("CONCAT_SEPARATOR requires 2 arguments: separator and array".to_string()));
                }
                let separator = evaluated_args[0].as_str()
                    .ok_or_else(|| DbError::ExecutionError("CONCAT_SEPARATOR: first argument (separator) must be a string".to_string()))?;

                let array = match &evaluated_args[1] {
                    Value::Array(arr) => arr,
                    _ => return Err(DbError::ExecutionError("CONCAT_SEPARATOR: second argument must be an array".to_string())),
                };

                let strings: Vec<String> = array.iter().map(|v| {
                    match v {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Null => "null".to_string(),
                        _ => format!("{}", v),
                    }
                }).collect();

                Ok(Value::String(strings.join(separator)))
            }

            // SUBSTRING(string, start, length?) - substring
            "SUBSTRING" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError("SUBSTRING requires 2-3 arguments".to_string()));
                }
                let s = evaluated_args[0].as_str()
                    .ok_or_else(|| DbError::ExecutionError("SUBSTRING: first argument must be a string".to_string()))?;
                let start = evaluated_args[1].as_i64()
                    .ok_or_else(|| DbError::ExecutionError("SUBSTRING: start must be a number".to_string()))? as usize;
                let len = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_i64().unwrap_or(s.len() as i64) as usize
                } else {
                    s.len() - start
                };

                let result: String = s.chars().skip(start).take(len).collect();
                Ok(Value::String(result))
            }

            // FULLTEXT(collection, field, query, maxDistance?) - fulltext search with fuzzy matching
            "FULLTEXT" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "FULLTEXT requires 3-4 arguments: collection, field, query, [maxDistance]".to_string()
                    ));
                }
                let collection_name = evaluated_args[0].as_str()
                    .ok_or_else(|| DbError::ExecutionError("FULLTEXT: collection must be a string".to_string()))?;
                let field = evaluated_args[1].as_str()
                    .ok_or_else(|| DbError::ExecutionError("FULLTEXT: field must be a string".to_string()))?;
                let query = evaluated_args[2].as_str()
                    .ok_or_else(|| DbError::ExecutionError("FULLTEXT: query must be a string".to_string()))?;
                let max_distance = if evaluated_args.len() == 4 {
                    evaluated_args[3].as_u64().unwrap_or(2) as usize
                } else {
                    2 // Default Levenshtein distance
                };

                let collection = self.storage.get_collection(collection_name)?;

                match collection.fulltext_search(field, query, max_distance) {
                    Some(matches) => {
                        let results: Vec<Value> = matches.iter().filter_map(|m| {
                            collection.get(&m.doc_key).ok().map(|doc| {
                                let mut obj = serde_json::Map::new();
                                obj.insert("doc".to_string(), doc.to_value());
                                obj.insert("score".to_string(), json!(m.score));
                                obj.insert("matched".to_string(), json!(m.matched_terms));
                                Value::Object(obj)
                            })
                        }).collect();
                        Ok(Value::Array(results))
                    }
                    None => Err(DbError::ExecutionError(format!(
                        "No fulltext index found on field '{}' in collection '{}'", field, collection_name
                    ))),
                }
            }

            // LEVENSHTEIN(string1, string2) - Levenshtein distance between two strings
            "LEVENSHTEIN" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "LEVENSHTEIN requires 2 arguments: string1, string2".to_string()
                    ));
                }
                let s1 = evaluated_args[0].as_str()
                    .ok_or_else(|| DbError::ExecutionError("LEVENSHTEIN: first argument must be a string".to_string()))?;
                let s2 = evaluated_args[1].as_str()
                    .ok_or_else(|| DbError::ExecutionError("LEVENSHTEIN: second argument must be a string".to_string()))?;

                let distance = crate::storage::levenshtein_distance(s1, s2);
                Ok(Value::Number(serde_json::Number::from(distance)))
            }

            // MERGE(obj1, obj2, ...) - merge multiple objects (later objects override earlier ones)
            "MERGE" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "MERGE requires at least 1 argument".to_string()
                    ));
                }

                let mut result = serde_json::Map::new();

                for arg in &evaluated_args {
                    match arg {
                        Value::Object(obj) => {
                            // Merge this object into the result
                            for (key, value) in obj {
                                result.insert(key.clone(), value.clone());
                            }
                        }
                        Value::Null => {
                            // Skip null values
                            continue;
                        }
                        _ => {
                            return Err(DbError::ExecutionError(
                                format!("MERGE: all arguments must be objects, got: {:?}", arg)
                            ));
                        }
                    }
                }

                Ok(Value::Object(result))
            }

            _ => Err(DbError::ExecutionError(format!("Unknown function: {}", name))),

        }
    }

    // ==================== Index Optimization (for single FOR queries) ====================

    /// Try to use index for single-FOR queries
    #[allow(dead_code)]
    fn get_indexed_documents(
        &self,
        collection: &Collection,
        filter_clauses: &[FilterClause],
        var_name: &str,
    ) -> Option<Vec<Value>> {
        for filter in filter_clauses {
            if let Some(condition) = self.extract_indexable_condition(&filter.expression, var_name) {
                if let Some(docs) = self.use_index_for_condition(collection, &condition) {
                    return Some(docs.iter().map(|d| d.to_value()).collect());
                }
            }
        }
        None
    }

    /// Extract a simple indexable condition from a filter expression
    fn extract_indexable_condition(&self, expr: &Expression, var_name: &str) -> Option<IndexableCondition> {
        match expr {
            Expression::BinaryOp { left, op, right } => {
                match op {
                    BinaryOperator::Equal |
                    BinaryOperator::LessThan |
                    BinaryOperator::LessThanOrEqual |
                    BinaryOperator::GreaterThan |
                    BinaryOperator::GreaterThanOrEqual => {
                        // Try left = field access, right = literal
                        if let Some(field) = self.extract_field_path(left, var_name) {
                            if let Expression::Literal(value) = right.as_ref() {
                                return Some(IndexableCondition {
                                    field,
                                    op: op.clone(),
                                    value: value.clone(),
                                });
                            }
                        }
                        // Try right = field access, left = literal
                        if let Some(field) = self.extract_field_path(right, var_name) {
                            if let Expression::Literal(value) = left.as_ref() {
                                let reversed_op = match op {
                                    BinaryOperator::LessThan => BinaryOperator::GreaterThan,
                                    BinaryOperator::LessThanOrEqual => BinaryOperator::GreaterThanOrEqual,
                                    BinaryOperator::GreaterThan => BinaryOperator::LessThan,
                                    BinaryOperator::GreaterThanOrEqual => BinaryOperator::LessThanOrEqual,
                                    other => other.clone(),
                                };
                                return Some(IndexableCondition {
                                    field,
                                    op: reversed_op,
                                    value: value.clone(),
                                });
                            }
                        }
                    }
                    BinaryOperator::And => {
                        if let Some(cond) = self.extract_indexable_condition(left, var_name) {
                            return Some(cond);
                        }
                        return self.extract_indexable_condition(right, var_name);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        None
    }

    /// Extract field path from an expression
    fn extract_field_path(&self, expr: &Expression, var_name: &str) -> Option<String> {
        match expr {
            Expression::FieldAccess(base, field) => {
                if let Expression::Variable(name) = base.as_ref() {
                    if name == var_name {
                        return Some(field.clone());
                    }
                }
                if let Some(base_path) = self.extract_field_path(base, var_name) {
                    return Some(format!("{}.{}", base_path, field));
                }
                None
            }
            _ => None,
        }
    }

    /// Use index for a condition lookup
    fn use_index_for_condition(
        &self,
        collection: &Collection,
        condition: &IndexableCondition,
    ) -> Option<Vec<crate::storage::Document>> {
        match condition.op {
            BinaryOperator::Equal => collection.index_lookup_eq(&condition.field, &condition.value),
            BinaryOperator::GreaterThan => collection.index_lookup_gt(&condition.field, &condition.value),
            BinaryOperator::GreaterThanOrEqual => collection.index_lookup_gte(&condition.field, &condition.value),
            BinaryOperator::LessThan => collection.index_lookup_lt(&condition.field, &condition.value),
            BinaryOperator::LessThanOrEqual => collection.index_lookup_lte(&condition.field, &condition.value),
            _ => None,
        }
    }
}

/// Parse a field path string into an Expression (e.g., "u.name" -> FieldAccess)
fn parse_field_expr(field_path: &str) -> Expression {
    let parts: Vec<&str> = field_path.split('.').collect();

    if parts.is_empty() {
        return Expression::Literal(Value::Null);
    }

    let mut expr = Expression::Variable(parts[0].to_string());

    for part in &parts[1..] {
        expr = Expression::FieldAccess(Box::new(expr), part.to_string());
    }

    expr
}

#[inline]
fn get_field_value(value: &Value, field_path: &str) -> Value {
    let mut current = value;

    for part in field_path.split('.') {
        match current.get(part) {
            Some(val) => current = val,
            None => return Value::Null,
        }
    }

    current.clone()
}

#[inline]
fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        _ => left == right,
    }
}

#[inline]
fn evaluate_binary_op(left: &Value, op: &BinaryOperator, right: &Value) -> DbResult<Value> {
    match op {
        BinaryOperator::Equal => Ok(Value::Bool(values_equal(left, right))),
        BinaryOperator::NotEqual => Ok(Value::Bool(!values_equal(left, right))),

        BinaryOperator::LessThan => {
            Ok(Value::Bool(compare_values(left, right) == std::cmp::Ordering::Less))
        }
        BinaryOperator::LessThanOrEqual => {
            Ok(Value::Bool(compare_values(left, right) != std::cmp::Ordering::Greater))
        }
        BinaryOperator::GreaterThan => {
            Ok(Value::Bool(compare_values(left, right) == std::cmp::Ordering::Greater))
        }
        BinaryOperator::GreaterThanOrEqual => {
            Ok(Value::Bool(compare_values(left, right) != std::cmp::Ordering::Less))
        }

        BinaryOperator::And => Ok(Value::Bool(to_bool(left) && to_bool(right))),
        BinaryOperator::Or => Ok(Value::Bool(to_bool(left) || to_bool(right))),

        BinaryOperator::Add => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from_f64(a + b).unwrap()))
            } else if let (Some(a), Some(b)) = (left.as_str(), right.as_str()) {
                Ok(Value::String(format!("{}{}", a, b)))
            } else {
                Err(DbError::ExecutionError("Cannot add these types".to_string()))
            }
        }

        BinaryOperator::Subtract => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from_f64(a - b).unwrap()))
            } else {
                Err(DbError::ExecutionError("Cannot subtract non-numbers".to_string()))
            }
        }

        BinaryOperator::Multiply => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from_f64(a * b).unwrap()))
            } else {
                Err(DbError::ExecutionError("Cannot multiply non-numbers".to_string()))
            }
        }

        BinaryOperator::Divide => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                if b == 0.0 {
                    Err(DbError::ExecutionError("Division by zero".to_string()))
                } else {
                    Ok(Value::Number(serde_json::Number::from_f64(a / b).unwrap()))
                }
            } else {
                Err(DbError::ExecutionError("Cannot divide non-numbers".to_string()))
            }
        }
    }
}

#[inline]
fn evaluate_unary_op(op: &UnaryOperator, operand: &Value) -> DbResult<Value> {
    match op {
        UnaryOperator::Not => Ok(Value::Bool(!to_bool(operand))),
        UnaryOperator::Negate => {
            if let Some(n) = operand.as_f64() {
                Ok(Value::Number(serde_json::Number::from_f64(-n).unwrap()))
            } else {
                Err(DbError::ExecutionError("Cannot negate non-number".to_string()))
            }
        }
    }
}

#[inline]
fn to_bool(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

#[inline]
fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Number(a), Value::Number(b)) => {
            let a_f64 = a.as_f64().unwrap_or(0.0);
            let b_f64 = b.as_f64().unwrap_or(0.0);
            a_f64.partial_cmp(&b_f64).unwrap_or(Ordering::Equal)
        }
        (Value::String(a), Value::String(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}
