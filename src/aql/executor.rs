use serde_json::{Value, json};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::time::{Instant, Duration};
use chrono::Utc;

use crate::error::{DbError, DbResult};
use crate::storage::{Collection, StorageEngine, GeoPoint, distance_meters};
use crate::cluster::{ReplicationService, Operation};
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
    database: Option<String>,
    replication: Option<&'a ReplicationService>,
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
            database: None,
            replication: None,
        }
    }

    /// Create executor with bind variables for parameterized queries
    pub fn with_bind_vars(storage: &'a StorageEngine, bind_vars: BindVars) -> Self {
        Self { storage, bind_vars, database: None, replication: None }
    }

    /// Create executor with database context
    pub fn with_database(storage: &'a StorageEngine, database: String) -> Self {
        Self {
            storage,
            bind_vars: HashMap::new(),
            database: Some(database),
            replication: None,
        }
    }

    /// Create executor with both database context and bind variables
    pub fn with_database_and_bind_vars(storage: &'a StorageEngine, database: String, bind_vars: BindVars) -> Self {
        Self {
            storage,
            bind_vars,
            database: Some(database),
            replication: None,
        }
    }

    /// Set replication service for logging mutations
    pub fn with_replication(mut self, replication: &'a ReplicationService) -> Self {
        self.replication = Some(replication);
        self
    }

    /// Log a mutation to the replication service
    fn log_mutation(&self, collection: &str, operation: Operation, key: &str, data: Option<&Value>) {
        if let (Some(repl), Some(ref db)) = (&self.replication, &self.database) {
            let doc_bytes = data.and_then(|v| serde_json::to_vec(v).ok());
            repl.record_write(
                db,
                collection,
                operation,
                key,
                doc_bytes.as_deref(),
                None,
            );
        }
    }

    /// Log multiple mutations asynchronously in a background thread
    /// Used for bulk INSERT operations to avoid blocking the response
    fn log_mutations_async(&self, collection: &str, operation: Operation, docs: &[crate::storage::Document]) {
        // Clone the replication service if available (needs to be done before pattern matching)
        let repl_clone = self.replication.map(|r| r.clone());
        let db_clone = self.database.clone();

        if let (Some(repl), Some(db)) = (repl_clone, db_clone) {
            let collection = collection.to_string();

            // Serialize documents upfront to minimize work in the thread
            let mutations: Vec<(String, Vec<u8>)> = docs.iter()
                .filter_map(|doc| {
                    serde_json::to_vec(&doc.to_value())
                        .ok()
                        .map(|bytes| (doc.key.clone(), bytes))
                })
                .collect();

            let count = mutations.len();
            tracing::debug!("INSERT: Starting async replication logging for {} docs", count);

            std::thread::spawn(move || {
                let start = std::time::Instant::now();
                for (key, doc_bytes) in mutations {
                    repl.record_write(
                        &db,
                        &collection,
                        operation.clone(),
                        &key,
                        Some(&doc_bytes),
                        None,
                    );
                }
                let elapsed = start.elapsed();
                tracing::debug!("INSERT: Async replication logging of {} docs completed in {:?}", count, elapsed);
            });
        }
    }

    /// Get collection with database prefix if set
    /// Uses database.get_collection() to share the same cached Collection instances
    fn get_collection(&self, name: &str) -> DbResult<crate::storage::Collection> {
        // If we have a database context, get collection through the database
        // This ensures we use the same cached Collection instances as the handlers
        if let Some(ref db_name) = self.database {
            let database = self.storage.get_database(db_name)?;
            database.get_collection(name)
        } else {
            // No database context - fall back to legacy storage method
            self.storage.get_collection(name)
        }
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

        // Optimization: Use index for SORT + LIMIT if available
        // Check if query is: FOR var IN collection SORT var.field LIMIT n RETURN ...
        if let (Some(sort), Some(limit)) = (&query.sort_clause, &query.limit_clause) {
            // Check if we have a simple FOR loop on a collection
            if query.body_clauses.len() == 1 {
                if let Some(BodyClause::For(for_clause)) = query.body_clauses.first() {
                    // Check if the sort field is on the loop variable
                    let sort_field = &sort.field;
                    if sort_field.starts_with(&format!("{}.", for_clause.variable)) {
                        let field = &sort_field[for_clause.variable.len() + 1..];

                        // Try to get collection and check for index
                        if let Ok(collection) = self.get_collection(&for_clause.collection) {
                            if let Some(docs) = collection.index_sorted(field, sort.ascending, Some(limit.offset + limit.count)) {
                                // Got sorted documents from index! Apply offset and build result
                                let start = limit.offset.min(docs.len());
                                let end = (start + limit.count).min(docs.len());
                                let docs = &docs[start..end];

                                if let Some(ref return_clause) = query.return_clause {
                                    let results: DbResult<Vec<Value>> = docs.iter().map(|doc| {
                                        let mut ctx = initial_bindings.clone();
                                        ctx.insert(for_clause.variable.clone(), doc.to_value());
                                        self.evaluate_expr_with_context(&return_clause.expression, &ctx)
                                    }).collect();
                                    return results;
                                } else {
                                    // No RETURN clause - return empty array
                                    return Ok(vec![]);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Optimization: Check if we can push LIMIT down to storage scan
        let scan_limit = if query.sort_clause.is_none() {
            let for_count = query.body_clauses.iter().filter(|c| matches!(c, BodyClause::For(_))).count();
            let filter_count = query.body_clauses.iter().filter(|c| matches!(c, BodyClause::Filter(_))).count();

            if for_count == 1 && filter_count == 0 {
                query.limit_clause.as_ref().map(|l| l.offset + l.count)
            } else {
                None
            }
        } else {
            None
        };

        // Process body_clauses in order (supports correlated subqueries)
        // If body_clauses is empty, fall back to legacy behavior
        let rows = if !query.body_clauses.is_empty() {
            self.execute_body_clauses(&query.body_clauses, &initial_bindings, scan_limit)?
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

        // Apply RETURN projection (if present)
        if let Some(ref return_clause) = query.return_clause {
            let results: DbResult<Vec<Value>> = rows
                .iter()
                .map(|ctx| self.evaluate_expr_with_context(&return_clause.expression, ctx))
                .collect();
            results
        } else {
            // No RETURN clause - return empty array (mutations don't need to return anything)
            Ok(vec![])
        }
    }

    /// Execute body clauses in order, supporting correlated subqueries
    /// LET clauses inside FOR loops are evaluated per-row with access to outer variables
    fn execute_body_clauses(&self, clauses: &[BodyClause], initial_ctx: &Context, scan_limit: Option<usize>) -> DbResult<Vec<Context>> {
        let mut rows: Vec<Context> = vec![initial_ctx.clone()];

        // Optimization: Check if we can use index for FOR + FILTER pattern
        // Pattern: FOR var IN collection, followed by FILTER on var.field == value
        let mut i = 0;
        while i < clauses.len() {
            match &clauses[i] {
                BodyClause::For(for_clause) => {
                    // Check if next clause is a FILTER that can use an index
                    let use_index = if i + 1 < clauses.len() {
                        if let BodyClause::Filter(filter_clause) = &clauses[i + 1] {
                            // Check if this is a collection (not a LET variable)
                            // source_variable might be None or Some(collection_name)
                            let is_collection = if let Some(src) = &for_clause.source_variable {
                                // If source_variable == collection, it's a collection
                                src == &for_clause.collection
                            } else {
                                // If source_variable is None, it's definitely a collection
                                true
                            };

                            if is_collection {
                                // Try to extract indexable condition
                                self.extract_indexable_condition(&filter_clause.expression, &for_clause.variable).is_some()
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if use_index {
                        // Try to use index lookup
                        if let BodyClause::Filter(filter_clause) = &clauses[i + 1] {
                            let mut used_index = false;
                            let mut new_rows = Vec::new();

                            for ctx in &rows {
                                if let Ok(collection) = self.get_collection(&for_clause.collection) {
                                    if let Some(condition) = self.extract_indexable_condition(&filter_clause.expression, &for_clause.variable) {
                                        if let Some(docs) = self.use_index_for_condition(&collection, &condition) {
                                            if !docs.is_empty() {
                                                used_index = true;
                                                // Apply scan_limit to index results
                                                let docs: Vec<_> = if let Some(n) = scan_limit {
                                                    docs.into_iter().take(n).collect()
                                                } else {
                                                    docs
                                                };

                                                for doc in docs {
                                                    let mut new_ctx = ctx.clone();
                                                    new_ctx.insert(for_clause.variable.clone(), doc.to_value());
                                                    new_rows.push(new_ctx);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Only use index results if we actually found documents
                            if used_index {
                                rows = new_rows;
                                i += 2; // Skip both FOR and FILTER
                                continue;
                            }
                            // Otherwise fall through to normal FOR processing
                        }
                    }

                    // Normal FOR processing (no index)
                    let mut new_rows = Vec::new();
                    for ctx in &rows {
                        let docs = self.get_for_source_docs(for_clause, ctx, scan_limit)?;
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
                BodyClause::Insert(insert_clause) => {
                    // Get collection once, outside the loop
                    let collection = self.get_collection(&insert_clause.collection)?;

                    // For bulk inserts (>100 docs), use batch mode for maximum performance
                    let bulk_mode = rows.len() > 100;
                    let has_indexes = !collection.list_indexes().is_empty();

                    tracing::debug!(
                        "INSERT: {} documents, bulk_mode={}, has_indexes={}",
                        rows.len(), bulk_mode, has_indexes
                    );

                    if bulk_mode {
                        // Evaluate all documents first
                        let eval_start = std::time::Instant::now();
                        let mut documents = Vec::with_capacity(rows.len());
                        for ctx in &rows {
                            let doc_value = self.evaluate_expr_with_context(&insert_clause.document, ctx)?;
                            documents.push(doc_value);
                        }
                        let eval_time = eval_start.elapsed();
                        tracing::debug!("INSERT: Document evaluation took {:?}", eval_time);

                        // Batch insert all documents at once (uses RocksDB WriteBatch)
                        let insert_start = std::time::Instant::now();
                        let inserted_docs = collection.insert_batch(documents)?;
                        let insert_time = insert_start.elapsed();
                        tracing::debug!("INSERT: Batch insert of {} docs took {:?}", inserted_docs.len(), insert_time);

                        // Log to replication asynchronously for bulk inserts
                        self.log_mutations_async(&insert_clause.collection, Operation::Insert, &inserted_docs);

                        // Index ONLY the newly inserted documents asynchronously
                        if has_indexes {
                            tracing::debug!("INSERT: Starting async indexing of {} new docs", inserted_docs.len());
                            let coll = collection.clone();
                            std::thread::spawn(move || {
                                let index_start = std::time::Instant::now();
                                let result = coll.index_documents(&inserted_docs);
                                let index_time = index_start.elapsed();
                                match result {
                                    Ok(count) => tracing::debug!("INSERT: Indexed {} docs in {:?}", count, index_time),
                                    Err(e) => tracing::error!("INSERT: Indexing failed: {}", e),
                                }
                            });
                        }
                    } else {
                        // Small inserts - use normal path with indexes
                        let insert_start = std::time::Instant::now();
                        for ctx in &rows {
                            let doc_value = self.evaluate_expr_with_context(&insert_clause.document, ctx)?;
                            let doc = collection.insert(doc_value)?;
                            // Log to replication
                            self.log_mutation(&insert_clause.collection, Operation::Insert, &doc.key, Some(&doc.to_value()));
                        }
                        let insert_time = insert_start.elapsed();
                        tracing::debug!("INSERT: {} docs with indexes took {:?}", rows.len(), insert_time);
                    }
                }
                BodyClause::Update(update_clause) => {
                    // Get collection once, outside the loop
                    let collection = self.get_collection(&update_clause.collection)?;

                    // Update documents for each row context
                    for ctx in &rows {
                        // Evaluate selector expression to get the document key
                        let selector_value = self.evaluate_expr_with_context(&update_clause.selector, ctx)?;

                        // Extract _key from selector (can be a string key or a document with _key field)
                        let key = match &selector_value {
                            Value::String(s) => s.clone(),
                            Value::Object(obj) => {
                                obj.get("_key")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .ok_or_else(|| DbError::ExecutionError(
                                        "UPDATE: selector object must have a _key field".to_string()
                                    ))?
                            }
                            _ => return Err(DbError::ExecutionError(
                                "UPDATE: selector must be a string key or an object with _key field".to_string()
                            )),
                        };

                        // Evaluate changes expression
                        let changes_value = self.evaluate_expr_with_context(&update_clause.changes, ctx)?;

                        // Ensure changes is an object
                        if !changes_value.is_object() {
                            return Err(DbError::ExecutionError(
                                "UPDATE: changes must be an object".to_string()
                            ));
                        }

                        // Update the document (collection.update handles merging internally)
                        let doc = collection.update(&key, changes_value)?;
                        // Log to replication
                        self.log_mutation(&update_clause.collection, Operation::Update, &key, Some(&doc.to_value()));
                    }
                }
                BodyClause::Remove(remove_clause) => {
                    // Get collection once, outside the loop
                    let collection = self.get_collection(&remove_clause.collection)?;

                    // Remove documents for each row context
                    for ctx in &rows {
                        // Evaluate selector expression to get the document key
                        let selector_value = self.evaluate_expr_with_context(&remove_clause.selector, ctx)?;

                        // Extract _key from selector (can be a string key or a document with _key field)
                        let key = match &selector_value {
                            Value::String(s) => s.clone(),
                            Value::Object(obj) => {
                                obj.get("_key")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .ok_or_else(|| DbError::ExecutionError(
                                        "REMOVE: selector object must have a _key field".to_string()
                                    ))?
                            }
                            _ => return Err(DbError::ExecutionError(
                                "REMOVE: selector must be a string key or an object with _key field".to_string()
                            )),
                        };

                        // Delete the document
                        collection.delete(&key)?;
                        // Log to replication
                        self.log_mutation(&remove_clause.collection, Operation::Delete, &key, None);
                    }
                }
            }
            i += 1;
        }

        Ok(rows)
    }

    /// Get documents for a FOR clause source (collection or variable)
    fn get_for_source_docs(&self, for_clause: &ForClause, ctx: &Context, limit: Option<usize>) -> DbResult<Vec<Value>> {
        // Check if source is an expression (e.g., range 1..5)
        if let Some(expr) = &for_clause.source_expression {
            let value = self.evaluate_expr_with_context(expr, ctx)?;
            return match value {
                Value::Array(arr) => {
                    if let Some(n) = limit {
                        Ok(arr.into_iter().take(n).collect())
                    } else {
                        Ok(arr)
                    }
                },
                other => Ok(vec![other]),
            };
        }

        let source_name = for_clause.source_variable.as_ref()
            .unwrap_or(&for_clause.collection);

        // Check if source is a LET variable in current context
        if let Some(value) = ctx.get(source_name) {
            return match value {
                Value::Array(arr) => {
                    if let Some(n) = limit {
                        Ok(arr.iter().take(n).cloned().collect())
                    } else {
                        Ok(arr.clone())
                    }
                },
                other => Ok(vec![other.clone()]),
            };
        }

        // Otherwise it's a collection - use scan with limit for optimization
        let collection = self.get_collection(&for_clause.collection)?;
        Ok(collection.scan(limit).into_iter().map(|d| d.to_value()).collect())
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
                let collection = self.get_collection(&for_clause.collection)?;
                let doc_count = collection.count();
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

        // Execute query using optimized path (with index support)
        let scan_start = Instant::now();
        let mut rows = if !query.body_clauses.is_empty() {
            // Use optimized path with index support
            // Don't pass scan_limit to explain - we want to see full execution
            self.execute_body_clauses(&query.body_clauses, &let_bindings, None)?
        } else {
            // Legacy path for old queries
            self.build_row_combinations_with_context(&query.for_clauses, &let_bindings)?
        };
        collection_scan_time = scan_start.elapsed();
        let rows_after_scan = rows.len();

        // Note: Filters are already applied in execute_body_clauses, but we need to analyze them
        // So we'll extract filter info from body_clauses
        let filter_start = Instant::now();

        if !query.body_clauses.is_empty() {
            // Filters were already applied in execute_body_clauses
            // Extract filter info from body_clauses for reporting
            for clause in &query.body_clauses {
                if let BodyClause::Filter(filter) = clause {
                    // Try to find index candidate for this filter
                    let mut index_candidate = None;
                    let mut can_use_index = false;

                    if !query.for_clauses.is_empty() {
                        let var_name = &query.for_clauses[0].variable;
                        if let Some(condition) = self.extract_indexable_condition(&filter.expression, var_name) {
                            index_candidate = Some(condition.field.clone());
                            // Check if index exists
                            if let Ok(collection) = self.get_collection(&query.for_clauses[0].collection) {
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
                        documents_before: total_docs_scanned,
                        documents_after: rows.len(),
                        time_us: 0, // Timing included in collection_scan_time
                    });
                }
            }
        } else {
            // Legacy path: Apply and analyze FILTER clauses
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
                        if let Ok(collection) = self.get_collection(&query.for_clauses[0].collection) {
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

        // Apply RETURN projection (if present)
        let return_start = Instant::now();
        let results = if let Some(ref return_clause) = query.return_clause {
            let results: DbResult<Vec<Value>> = rows
                .iter()
                .map(|ctx| self.evaluate_expr_with_context(&return_clause.expression, ctx))
                .collect();
            results?
        } else {
            vec![]
        };
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

            Expression::Range(start_expr, end_expr) => {
                let start_val = self.evaluate_expr_with_context(start_expr, ctx)?;
                let end_val = self.evaluate_expr_with_context(end_expr, ctx)?;

                let start = match &start_val {
                    Value::Number(n) => {
                        // Try integer first, then fall back to truncating float
                        n.as_i64().or_else(|| n.as_f64().map(|f| f as i64))
                            .ok_or_else(|| DbError::ExecutionError("Range start must be a number".to_string()))?
                    }
                    _ => return Err(DbError::ExecutionError(
                        format!("Range start must be a number, got: {:?}", start_val)
                    )),
                };

                let end = match &end_val {
                    Value::Number(n) => {
                        // Try integer first, then fall back to truncating float
                        n.as_i64().or_else(|| n.as_f64().map(|f| f as i64))
                            .ok_or_else(|| DbError::ExecutionError("Range end must be a number".to_string()))?
                    }
                    _ => return Err(DbError::ExecutionError(
                        format!("Range end must be a number, got: {:?}", end_val)
                    )),
                };

                // Generate array from start to end (inclusive)
                let arr: Vec<Value> = (start..=end)
                    .map(|i| Value::Number(serde_json::Number::from(i)))
                    .collect();

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
            self.execute_body_clauses(&query.body_clauses, &initial_bindings, None)?
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

        // Apply RETURN projection (if present)
        if let Some(ref return_clause) = query.return_clause {
            let results: DbResult<Vec<Value>> = rows
                .iter()
                .map(|ctx| self.evaluate_expr_with_context(&return_clause.expression, ctx))
                .collect();
            results
        } else {
            Ok(vec![])
        }
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

            // COUNT(array) - count elements in array
            "COUNT" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("COUNT requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("COUNT: argument must be an array".to_string()))?;
                Ok(Value::Number(serde_json::Number::from(arr.len())))
            }

            // COUNT_DISTINCT(array) - count distinct values in array
            "COUNT_DISTINCT" | "COUNT_UNIQUE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("COUNT_DISTINCT requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("COUNT_DISTINCT: argument must be an array".to_string()))?;
                let unique: std::collections::HashSet<String> = arr.iter()
                    .map(|v| v.to_string())
                    .collect();
                Ok(Value::Number(serde_json::Number::from(unique.len())))
            }

            // VARIANCE_POPULATION(array) - population variance
            "VARIANCE_POPULATION" | "VARIANCE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("VARIANCE_POPULATION requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("VARIANCE_POPULATION: argument must be an array".to_string()))?;
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let variance = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64;
                Ok(Value::Number(serde_json::Number::from_f64(variance).unwrap_or(serde_json::Number::from(0))))
            }

            // VARIANCE_SAMPLE(array) - sample variance (n-1 denominator)
            "VARIANCE_SAMPLE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("VARIANCE_SAMPLE requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("VARIANCE_SAMPLE: argument must be an array".to_string()))?;
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.len() < 2 {
                    return Ok(Value::Null);
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let variance = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nums.len() - 1) as f64;
                Ok(Value::Number(serde_json::Number::from_f64(variance).unwrap_or(serde_json::Number::from(0))))
            }

            // STDDEV_POPULATION(array) - population standard deviation
            "STDDEV_POPULATION" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("STDDEV_POPULATION requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("STDDEV_POPULATION: argument must be an array".to_string()))?;
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let variance = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64;
                let stddev = variance.sqrt();
                Ok(Value::Number(serde_json::Number::from_f64(stddev).unwrap_or(serde_json::Number::from(0))))
            }

            // STDDEV_SAMPLE(array) / STDDEV(array) - sample standard deviation (n-1 denominator)
            "STDDEV_SAMPLE" | "STDDEV" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("STDDEV_SAMPLE requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("STDDEV_SAMPLE: argument must be an array".to_string()))?;
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.len() < 2 {
                    return Ok(Value::Null);
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let variance = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nums.len() - 1) as f64;
                let stddev = variance.sqrt();
                Ok(Value::Number(serde_json::Number::from_f64(stddev).unwrap_or(serde_json::Number::from(0))))
            }

            // MEDIAN(array) - median value
            "MEDIAN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("MEDIAN requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("MEDIAN: argument must be an array".to_string()))?;
                let mut nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let len = nums.len();
                let median = if len % 2 == 0 {
                    (nums[len / 2 - 1] + nums[len / 2]) / 2.0
                } else {
                    nums[len / 2]
                };
                Ok(Value::Number(serde_json::Number::from_f64(median).unwrap_or(serde_json::Number::from(0))))
            }

            // PERCENTILE(array, p) - percentile value (p between 0 and 100)
            "PERCENTILE" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError("PERCENTILE requires 2 arguments: array, percentile (0-100)".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("PERCENTILE: first argument must be an array".to_string()))?;
                let p = evaluated_args[1].as_f64()
                    .ok_or_else(|| DbError::ExecutionError("PERCENTILE: second argument must be a number".to_string()))?;
                if !(0.0..=100.0).contains(&p) {
                    return Err(DbError::ExecutionError("PERCENTILE: percentile must be between 0 and 100".to_string()));
                }
                let mut nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let index = (p / 100.0) * (nums.len() - 1) as f64;
                let lower = index.floor() as usize;
                let upper = index.ceil() as usize;
                let result = if lower == upper {
                    nums[lower]
                } else {
                    let fraction = index - lower as f64;
                    nums[lower] * (1.0 - fraction) + nums[upper] * fraction
                };
                Ok(Value::Number(serde_json::Number::from_f64(result).unwrap_or(serde_json::Number::from(0))))
            }

            // UNIQUE(array) - return unique values
            "UNIQUE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("UNIQUE requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("UNIQUE: argument must be an array".to_string()))?;
                let mut seen = std::collections::HashSet::new();
                let unique: Vec<Value> = arr.iter()
                    .filter(|v| seen.insert(v.to_string()))
                    .cloned()
                    .collect();
                Ok(Value::Array(unique))
            }

            // SORTED(array) - sort array (ascending)
            "SORTED" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("SORTED requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("SORTED: argument must be an array".to_string()))?;
                let mut sorted = arr.clone();
                sorted.sort_by(|a, b| {
                    match (a, b) {
                        (Value::Number(n1), Value::Number(n2)) => {
                            n1.as_f64().unwrap_or(0.0).partial_cmp(&n2.as_f64().unwrap_or(0.0))
                                .unwrap_or(std::cmp::Ordering::Equal)
                        }
                        (Value::String(s1), Value::String(s2)) => s1.cmp(s2),
                        _ => a.to_string().cmp(&b.to_string())
                    }
                });
                Ok(Value::Array(sorted))
            }

            // SORTED_UNIQUE(array) - sort and return unique values
            "SORTED_UNIQUE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("SORTED_UNIQUE requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("SORTED_UNIQUE: argument must be an array".to_string()))?;
                let mut seen = std::collections::HashSet::new();
                let mut unique: Vec<Value> = arr.iter()
                    .filter(|v| seen.insert(v.to_string()))
                    .cloned()
                    .collect();
                unique.sort_by(|a, b| {
                    match (a, b) {
                        (Value::Number(n1), Value::Number(n2)) => {
                            n1.as_f64().unwrap_or(0.0).partial_cmp(&n2.as_f64().unwrap_or(0.0))
                                .unwrap_or(std::cmp::Ordering::Equal)
                        }
                        (Value::String(s1), Value::String(s2)) => s1.cmp(s2),
                        _ => a.to_string().cmp(&b.to_string())
                    }
                });
                Ok(Value::Array(unique))
            }

            // REVERSE(array) - reverse array
            "REVERSE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("REVERSE requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("REVERSE: argument must be an array".to_string()))?;
                let mut reversed = arr.clone();
                reversed.reverse();
                Ok(Value::Array(reversed))
            }

            // FIRST(array) - first element
            "FIRST" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("FIRST requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("FIRST: argument must be an array".to_string()))?;
                Ok(arr.first().cloned().unwrap_or(Value::Null))
            }

            // LAST(array) - last element
            "LAST" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("LAST requires 1 argument".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("LAST: argument must be an array".to_string()))?;
                Ok(arr.last().cloned().unwrap_or(Value::Null))
            }

            // NTH(array, index) - nth element (0-based)
            "NTH" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError("NTH requires 2 arguments: array, index".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("NTH: first argument must be an array".to_string()))?;
                let index = evaluated_args[1].as_i64()
                    .ok_or_else(|| DbError::ExecutionError("NTH: second argument must be an integer".to_string()))? as usize;
                Ok(arr.get(index).cloned().unwrap_or(Value::Null))
            }

            // SLICE(array, start, length?) - slice array
            "SLICE" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError("SLICE requires 2-3 arguments: array, start, [length]".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("SLICE: first argument must be an array".to_string()))?;
                let start = evaluated_args[1].as_i64()
                    .ok_or_else(|| DbError::ExecutionError("SLICE: start must be an integer".to_string()))?;
                let start = if start < 0 {
                    (arr.len() as i64 + start).max(0) as usize
                } else {
                    start as usize
                };
                let length = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_u64().unwrap_or(arr.len() as u64) as usize
                } else {
                    arr.len().saturating_sub(start)
                };
                let end = (start + length).min(arr.len());
                Ok(Value::Array(arr[start..end].to_vec()))
            }

            // FLATTEN(array, depth?) - flatten nested arrays
            "FLATTEN" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                    return Err(DbError::ExecutionError("FLATTEN requires 1-2 arguments: array, [depth]".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("FLATTEN: first argument must be an array".to_string()))?;
                let depth = if evaluated_args.len() > 1 {
                    evaluated_args[1].as_u64().unwrap_or(1) as usize
                } else {
                    1
                };
                fn flatten_recursive(arr: &[Value], depth: usize) -> Vec<Value> {
                    let mut result = Vec::new();
                    for item in arr {
                        if let Value::Array(inner) = item {
                            if depth > 0 {
                                result.extend(flatten_recursive(inner, depth - 1));
                            } else {
                                result.push(item.clone());
                            }
                        } else {
                            result.push(item.clone());
                        }
                    }
                    result
                }
                Ok(Value::Array(flatten_recursive(arr, depth)))
            }

            // PUSH(array, element, unique?) - add element to array
            "PUSH" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError("PUSH requires 2-3 arguments: array, element, [unique]".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("PUSH: first argument must be an array".to_string()))?;
                let element = &evaluated_args[1];
                let unique = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_bool().unwrap_or(false)
                } else {
                    false
                };
                let mut result = arr.clone();
                if unique {
                    if !result.iter().any(|v| v.to_string() == element.to_string()) {
                        result.push(element.clone());
                    }
                } else {
                    result.push(element.clone());
                }
                Ok(Value::Array(result))
            }

            // APPEND(array1, array2, unique?) - append arrays
            "APPEND" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError("APPEND requires 2-3 arguments: array1, array2, [unique]".to_string()));
                }
                let arr1 = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("APPEND: first argument must be an array".to_string()))?;
                let arr2 = evaluated_args[1].as_array()
                    .ok_or_else(|| DbError::ExecutionError("APPEND: second argument must be an array".to_string()))?;
                let unique = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_bool().unwrap_or(false)
                } else {
                    false
                };
                let mut result = arr1.clone();
                if unique {
                    let existing: std::collections::HashSet<String> = result.iter().map(|v| v.to_string()).collect();
                    for item in arr2 {
                        if !existing.contains(&item.to_string()) {
                            result.push(item.clone());
                        }
                    }
                } else {
                    result.extend(arr2.iter().cloned());
                }
                Ok(Value::Array(result))
            }

            // UNION(array1, array2, ...) - union of arrays (unique values)
            "UNION" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError("UNION requires at least 1 argument".to_string()));
                }
                let mut seen = std::collections::HashSet::new();
                let mut result = Vec::new();
                for arg in &evaluated_args {
                    let arr = arg.as_array()
                        .ok_or_else(|| DbError::ExecutionError("UNION: all arguments must be arrays".to_string()))?;
                    for item in arr {
                        if seen.insert(item.to_string()) {
                            result.push(item.clone());
                        }
                    }
                }
                Ok(Value::Array(result))
            }

            // UNION_DISTINCT(array1, array2, ...) - same as UNION (for compatibility)
            "UNION_DISTINCT" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError("UNION_DISTINCT requires at least 1 argument".to_string()));
                }
                let mut seen = std::collections::HashSet::new();
                let mut result = Vec::new();
                for arg in &evaluated_args {
                    let arr = arg.as_array()
                        .ok_or_else(|| DbError::ExecutionError("UNION_DISTINCT: all arguments must be arrays".to_string()))?;
                    for item in arr {
                        if seen.insert(item.to_string()) {
                            result.push(item.clone());
                        }
                    }
                }
                Ok(Value::Array(result))
            }

            // MINUS(array1, array2) - elements in array1 not in array2
            "MINUS" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError("MINUS requires 2 arguments: array1, array2".to_string()));
                }
                let arr1 = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("MINUS: first argument must be an array".to_string()))?;
                let arr2 = evaluated_args[1].as_array()
                    .ok_or_else(|| DbError::ExecutionError("MINUS: second argument must be an array".to_string()))?;
                let set2: std::collections::HashSet<String> = arr2.iter().map(|v| v.to_string()).collect();
                let mut seen = std::collections::HashSet::new();
                let result: Vec<Value> = arr1.iter()
                    .filter(|v| {
                        let key = v.to_string();
                        !set2.contains(&key) && seen.insert(key)
                    })
                    .cloned()
                    .collect();
                Ok(Value::Array(result))
            }

            // INTERSECTION(array1, array2, ...) - common elements in all arrays
            "INTERSECTION" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError("INTERSECTION requires at least 1 argument".to_string()));
                }
                let first = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("INTERSECTION: all arguments must be arrays".to_string()))?;

                if evaluated_args.len() == 1 {
                    return Ok(Value::Array(first.clone()));
                }

                // Build sets for all other arrays
                let mut sets: Vec<std::collections::HashSet<String>> = Vec::new();
                for arg in &evaluated_args[1..] {
                    let arr = arg.as_array()
                        .ok_or_else(|| DbError::ExecutionError("INTERSECTION: all arguments must be arrays".to_string()))?;
                    sets.push(arr.iter().map(|v| v.to_string()).collect());
                }

                let mut seen = std::collections::HashSet::new();
                let result: Vec<Value> = first.iter()
                    .filter(|v| {
                        let key = v.to_string();
                        sets.iter().all(|s| s.contains(&key)) && seen.insert(key)
                    })
                    .cloned()
                    .collect();
                Ok(Value::Array(result))
            }

            // POSITION(array, search, start?) - find position of element in array (0-based, -1 if not found)
            "POSITION" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError("POSITION requires 2-3 arguments: array, search, [start]".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("POSITION: first argument must be an array".to_string()))?;
                let search = &evaluated_args[1];
                let start = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_i64().unwrap_or(0) as usize
                } else {
                    0
                };
                let position = arr.iter()
                    .skip(start)
                    .position(|v| v.to_string() == search.to_string())
                    .map(|p| p + start);
                Ok(match position {
                    Some(p) => Value::Number(serde_json::Number::from(p)),
                    None => Value::Number(serde_json::Number::from(-1)),
                })
            }

            // CONTAINS_ARRAY(array, search) - check if array contains element
            "CONTAINS_ARRAY" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError("CONTAINS_ARRAY requires 2 arguments: array, search".to_string()));
                }
                let arr = evaluated_args[0].as_array()
                    .ok_or_else(|| DbError::ExecutionError("CONTAINS_ARRAY: first argument must be an array".to_string()))?;
                let search = &evaluated_args[1];
                let contains = arr.iter().any(|v| v.to_string() == search.to_string());
                Ok(Value::Bool(contains))
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

                let collection = self.get_collection(collection_name)?;

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


            // DATE_NOW() - current timestamp in milliseconds since Unix epoch
            "DATE_NOW" => {
                if !evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError("DATE_NOW requires 0 arguments".to_string()));
                }
                let timestamp = Utc::now().timestamp_millis();
                Ok(Value::Number(serde_json::Number::from(timestamp)))
            }

            // COLLECTION_COUNT(collection) - get the count of documents in a collection
            "COLLECTION_COUNT" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "COLLECTION_COUNT requires 1 argument: collection name".to_string()
                    ));
                }
                let collection_name = evaluated_args[0].as_str()
                    .ok_or_else(|| DbError::ExecutionError(
                        "COLLECTION_COUNT: argument must be a string (collection name)".to_string()
                    ))?;

                let collection = self.get_collection(collection_name)?;
                let count = collection.count();
                Ok(Value::Number(serde_json::Number::from(count)))
            }

            // DATE_ISO8601(date) - convert timestamp to ISO 8601 string
            "DATE_ISO8601" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("DATE_ISO8601 requires 1 argument: timestamp in milliseconds".to_string()));
                }

                // Handle both integer and float timestamps
                let timestamp_ms = match &evaluated_args[0] {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError("DATE_ISO8601: argument must be a number (timestamp in milliseconds)".to_string()));
                        }
                    }
                    _ => return Err(DbError::ExecutionError("DATE_ISO8601: argument must be a number (timestamp in milliseconds)".to_string())),
                };

                // Convert milliseconds to seconds for chrono
                let timestamp_secs = timestamp_ms / 1000;
                let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;

                // Create DateTime from timestamp
                use chrono::TimeZone;
                let datetime = match Utc.timestamp_opt(timestamp_secs, nanos) {
                    chrono::LocalResult::Single(dt) => dt,
                    _ => return Err(DbError::ExecutionError(
                        format!("DATE_ISO8601: invalid timestamp: {}", timestamp_ms)
                    )),
                };

                // Format as ISO 8601 string (e.g., "2023-12-03T13:44:00.000Z")
                let iso_string = datetime.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                Ok(Value::String(iso_string))
            }

            // DATE_TIMESTAMP(date) - convert ISO 8601 string to timestamp in milliseconds
            "DATE_TIMESTAMP" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "DATE_TIMESTAMP requires 1 argument: ISO 8601 date string".to_string()
                    ));
                }

                let date_str = evaluated_args[0].as_str()
                    .ok_or_else(|| DbError::ExecutionError(
                        "DATE_TIMESTAMP: argument must be a string (ISO 8601 date)".to_string()
                    ))?;

                // Parse ISO 8601 string to DateTime
                use chrono::DateTime;
                let datetime = DateTime::parse_from_rfc3339(date_str)
                    .map_err(|e| DbError::ExecutionError(
                        format!("DATE_TIMESTAMP: invalid ISO 8601 date '{}': {}", date_str, e)
                    ))?;

                // Convert to milliseconds since Unix epoch
                let timestamp_ms = datetime.timestamp_millis();
                Ok(Value::Number(serde_json::Number::from(timestamp_ms)))
            }

            // DATE_TRUNC(date, unit, timezone?) - truncate date to specified unit
            "DATE_TRUNC" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "DATE_TRUNC requires 2-3 arguments: date, unit, [timezone]".to_string()
                    ));
                }

                use chrono::{DateTime, Datelike, Timelike, TimeZone, NaiveDateTime};
                use chrono_tz::Tz;

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> = match &evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_TRUNC: invalid timestamp".to_string()
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => return Err(DbError::ExecutionError(
                                format!("DATE_TRUNC: invalid timestamp: {}", timestamp_ms)
                            )),
                        }
                    }
                    Value::String(s) => {
                        DateTime::parse_from_rfc3339(s)
                            .map_err(|e| DbError::ExecutionError(
                                format!("DATE_TRUNC: invalid ISO 8601 date '{}': {}", s, e)
                            ))?
                            .with_timezone(&Utc)
                    }
                    _ => return Err(DbError::ExecutionError(
                        "DATE_TRUNC: first argument must be a timestamp or ISO 8601 string".to_string()
                    )),
                };

                // Parse the unit
                let unit = evaluated_args[1].as_str()
                    .ok_or_else(|| DbError::ExecutionError(
                        "DATE_TRUNC: unit must be a string".to_string()
                    ))?
                    .to_lowercase();

                // Parse optional timezone
                let tz: Tz = if evaluated_args.len() == 3 {
                    let tz_str = evaluated_args[2].as_str()
                        .ok_or_else(|| DbError::ExecutionError(
                            "DATE_TRUNC: timezone must be a string".to_string()
                        ))?;
                    tz_str.parse::<Tz>()
                        .map_err(|_| DbError::ExecutionError(
                            format!("DATE_TRUNC: unknown timezone '{}'", tz_str)
                        ))?
                } else {
                    chrono_tz::UTC
                };

                // Convert to the target timezone for truncation
                let datetime_tz = datetime_utc.with_timezone(&tz);

                // Truncate based on unit
                let truncated: DateTime<Tz> = match unit.as_str() {
                    "y" | "year" | "years" => {
                        let naive = NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(datetime_tz.year(), 1, 1).unwrap(),
                            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_TRUNC: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "m" | "month" | "months" => {
                        let naive = NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), 1).unwrap(),
                            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_TRUNC: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "d" | "day" | "days" => {
                        let naive = NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), datetime_tz.day()).unwrap(),
                            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_TRUNC: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "h" | "hour" | "hours" => {
                        let naive = NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), datetime_tz.day()).unwrap(),
                            chrono::NaiveTime::from_hms_opt(datetime_tz.hour(), 0, 0).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_TRUNC: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "i" | "minute" | "minutes" => {
                        let naive = NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), datetime_tz.day()).unwrap(),
                            chrono::NaiveTime::from_hms_opt(datetime_tz.hour(), datetime_tz.minute(), 0).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_TRUNC: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "s" | "second" | "seconds" => {
                        let naive = NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), datetime_tz.day()).unwrap(),
                            chrono::NaiveTime::from_hms_opt(datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second()).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_TRUNC: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "f" | "millisecond" | "milliseconds" => {
                        // Keep the original datetime (milliseconds are the finest granularity we support)
                        datetime_tz
                    }
                    _ => return Err(DbError::ExecutionError(
                        format!("DATE_TRUNC: unknown unit '{}'. Valid units: y/year/years, m/month/months, d/day/days, h/hour/hours, i/minute/minutes, s/second/seconds, f/millisecond/milliseconds", unit)
                    )),
                };

                // Convert back to UTC and format as ISO 8601
                let truncated_utc = truncated.with_timezone(&Utc);
                let iso_string = truncated_utc.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                Ok(Value::String(iso_string))
            }

            // DATE_DAYS_IN_MONTH(date, timezone?) - return number of days in the month
            "DATE_DAYS_IN_MONTH" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                    return Err(DbError::ExecutionError(
                        "DATE_DAYS_IN_MONTH requires 1-2 arguments: date, [timezone]".to_string()
                    ));
                }

                use chrono::{DateTime, Datelike, TimeZone, NaiveDate};
                use chrono_tz::Tz;

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> = match &evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_DAYS_IN_MONTH: invalid timestamp".to_string()
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => return Err(DbError::ExecutionError(
                                format!("DATE_DAYS_IN_MONTH: invalid timestamp: {}", timestamp_ms)
                            )),
                        }
                    }
                    Value::String(s) => {
                        DateTime::parse_from_rfc3339(s)
                            .map_err(|e| DbError::ExecutionError(
                                format!("DATE_DAYS_IN_MONTH: invalid ISO 8601 date '{}': {}", s, e)
                            ))?
                            .with_timezone(&Utc)
                    }
                    _ => return Err(DbError::ExecutionError(
                        "DATE_DAYS_IN_MONTH: first argument must be a timestamp or ISO 8601 string".to_string()
                    )),
                };

                // Get year and month, optionally in a specific timezone
                let (year, month) = if evaluated_args.len() == 2 {
                    let tz_str = evaluated_args[1].as_str()
                        .ok_or_else(|| DbError::ExecutionError(
                            "DATE_DAYS_IN_MONTH: timezone must be a string".to_string()
                        ))?;
                    let tz: Tz = tz_str.parse()
                        .map_err(|_| DbError::ExecutionError(
                            format!("DATE_DAYS_IN_MONTH: unknown timezone '{}'", tz_str)
                        ))?;
                    let dt_tz = datetime_utc.with_timezone(&tz);
                    (dt_tz.year(), dt_tz.month())
                } else {
                    (datetime_utc.year(), datetime_utc.month())
                };

                // Calculate days in month by finding the first day of next month
                // and subtracting from it
                let days_in_month = if month == 12 {
                    NaiveDate::from_ymd_opt(year + 1, 1, 1)
                } else {
                    NaiveDate::from_ymd_opt(year, month + 1, 1)
                }
                .and_then(|next_month| NaiveDate::from_ymd_opt(year, month, 1).map(|this_month| {
                    (next_month - this_month).num_days()
                }))
                .unwrap_or(30) as u32; // Fallback, though this shouldn't happen

                Ok(Value::Number(serde_json::Number::from(days_in_month)))
            }

            // DATE_DAYOFYEAR(date, timezone?) - return day of year (1-366)
            "DATE_DAYOFYEAR" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                    return Err(DbError::ExecutionError(
                        "DATE_DAYOFYEAR requires 1-2 arguments: date, [timezone]".to_string()
                    ));
                }

                use chrono::{DateTime, Datelike, TimeZone};
                use chrono_tz::Tz;

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> = match &evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_DAYOFYEAR: invalid timestamp".to_string()
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => return Err(DbError::ExecutionError(
                                format!("DATE_DAYOFYEAR: invalid timestamp: {}", timestamp_ms)
                            )),
                        }
                    }
                    Value::String(s) => {
                        DateTime::parse_from_rfc3339(s)
                            .map_err(|e| DbError::ExecutionError(
                                format!("DATE_DAYOFYEAR: invalid ISO 8601 date '{}': {}", s, e)
                            ))?
                            .with_timezone(&Utc)
                    }
                    _ => return Err(DbError::ExecutionError(
                        "DATE_DAYOFYEAR: first argument must be a timestamp or ISO 8601 string".to_string()
                    )),
                };

                // Parse optional timezone
                let day_of_year = if evaluated_args.len() == 2 {
                    let tz_str = evaluated_args[1].as_str()
                        .ok_or_else(|| DbError::ExecutionError(
                            "DATE_DAYOFYEAR: timezone must be a string".to_string()
                        ))?;
                    let tz: Tz = tz_str.parse()
                        .map_err(|_| DbError::ExecutionError(
                            format!("DATE_DAYOFYEAR: unknown timezone '{}'", tz_str)
                        ))?;
                    datetime_utc.with_timezone(&tz).ordinal()
                } else {
                    datetime_utc.ordinal()
                };

                Ok(Value::Number(serde_json::Number::from(day_of_year)))
            }

            // DATE_ISOWEEK(date) - return ISO 8601 week number
            "DATE_ISOWEEK" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "DATE_ISOWEEK requires 1 argument: date".to_string()
                    ));
                }

                use chrono::{DateTime, Datelike, TimeZone};

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> = match &evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_ISOWEEK: invalid timestamp".to_string()
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => return Err(DbError::ExecutionError(
                                format!("DATE_ISOWEEK: invalid timestamp: {}", timestamp_ms)
                            )),
                        }
                    }
                    Value::String(s) => {
                        DateTime::parse_from_rfc3339(s)
                            .map_err(|e| DbError::ExecutionError(
                                format!("DATE_ISOWEEK: invalid ISO 8601 date '{}': {}", s, e)
                            ))?
                            .with_timezone(&Utc)
                    }
                    _ => return Err(DbError::ExecutionError(
                        "DATE_ISOWEEK: argument must be a timestamp or ISO 8601 string".to_string()
                    )),
                };

                // Get ISO week number
                let iso_week = datetime_utc.iso_week().week();
                Ok(Value::Number(serde_json::Number::from(iso_week)))
            }

            // DATE_FORMAT(date, format, timezone?) - format date according to format string
            "DATE_FORMAT" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "DATE_FORMAT requires 2-3 arguments: date, format, [timezone]".to_string()
                    ));
                }

                use chrono::{DateTime, TimeZone};
                use chrono_tz::Tz;

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> = match &evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_FORMAT: invalid timestamp".to_string()
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => return Err(DbError::ExecutionError(
                                format!("DATE_FORMAT: invalid timestamp: {}", timestamp_ms)
                            )),
                        }
                    }
                    Value::String(s) => {
                        DateTime::parse_from_rfc3339(s)
                            .map_err(|e| DbError::ExecutionError(
                                format!("DATE_FORMAT: invalid ISO 8601 date '{}': {}", s, e)
                            ))?
                            .with_timezone(&Utc)
                    }
                    _ => return Err(DbError::ExecutionError(
                        "DATE_FORMAT: first argument must be a timestamp or ISO 8601 string".to_string()
                    )),
                };

                // Parse the format string
                let format_str = evaluated_args[1].as_str()
                    .ok_or_else(|| DbError::ExecutionError(
                        "DATE_FORMAT: format must be a string".to_string()
                    ))?;

                // Parse optional timezone
                let tz: Tz = if evaluated_args.len() == 3 {
                    let tz_str = evaluated_args[2].as_str()
                        .ok_or_else(|| DbError::ExecutionError(
                            "DATE_FORMAT: timezone must be a string".to_string()
                        ))?;
                    tz_str.parse::<Tz>()
                        .map_err(|_| DbError::ExecutionError(
                            format!("DATE_FORMAT: unknown timezone '{}'", tz_str)
                        ))?
                } else {
                    chrono_tz::UTC
                };

                // Convert to the target timezone
                let datetime_tz = datetime_utc.with_timezone(&tz);

                // Format using strftime-style format string
                // Chrono supports: %Y, %m, %d, %H, %M, %S, %f, %a, %A, %b, %B, %j, %U, %W, %w, %Z, etc.
                let formatted = datetime_tz.format(format_str).to_string();
                Ok(Value::String(formatted))
            }

            // DATE_ADD(date, amount, unit, timezone?) - add amount of time to date
            "DATE_ADD" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "DATE_ADD requires 3-4 arguments: date, amount, unit, [timezone]".to_string()
                    ));
                }

                use chrono::{DateTime, Datelike, Timelike, TimeZone, Duration};
                use chrono_tz::Tz;

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> = match &evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_ADD: invalid timestamp".to_string()
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => return Err(DbError::ExecutionError(
                                format!("DATE_ADD: invalid timestamp: {}", timestamp_ms)
                            )),
                        }
                    }
                    Value::String(s) => {
                        DateTime::parse_from_rfc3339(s)
                            .map_err(|e| DbError::ExecutionError(
                                format!("DATE_ADD: invalid ISO 8601 date '{}': {}", s, e)
                            ))?
                            .with_timezone(&Utc)
                    }
                    _ => return Err(DbError::ExecutionError(
                        "DATE_ADD: first argument must be a timestamp or ISO 8601 string".to_string()
                    )),
                };

                // Parse the amount
                let amount = match &evaluated_args[1] {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_ADD: amount must be a number".to_string()
                            ));
                        }
                    }
                    _ => return Err(DbError::ExecutionError(
                        "DATE_ADD: amount must be a number".to_string()
                    )),
                };

                // Parse the unit
                let unit = evaluated_args[2].as_str()
                    .ok_or_else(|| DbError::ExecutionError(
                        "DATE_ADD: unit must be a string".to_string()
                    ))?
                    .to_lowercase();

                // Parse optional timezone
                let tz: Tz = if evaluated_args.len() == 4 {
                    let tz_str = evaluated_args[3].as_str()
                        .ok_or_else(|| DbError::ExecutionError(
                            "DATE_ADD: timezone must be a string".to_string()
                        ))?;
                    tz_str.parse::<Tz>()
                        .map_err(|_| DbError::ExecutionError(
                            format!("DATE_ADD: unknown timezone '{}'", tz_str)
                        ))?
                } else {
                    chrono_tz::UTC
                };

                // Convert to the target timezone for calculation
                let datetime_tz = datetime_utc.with_timezone(&tz);

                // Perform the addition based on unit
                let result_tz: DateTime<Tz> = match unit.as_str() {
                    "y" | "year" | "years" => {
                        // Add years
                        let new_year = datetime_tz.year() + amount as i32;
                        let naive = chrono::NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(new_year, datetime_tz.month(), datetime_tz.day())
                                .unwrap_or_else(|| {
                                    // Handle invalid dates (e.g., Feb 29 in non-leap year)
                                    chrono::NaiveDate::from_ymd_opt(new_year, datetime_tz.month(), 28).unwrap()
                                }),
                            chrono::NaiveTime::from_hms_milli_opt(
                                datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second(),
                                datetime_tz.timestamp_subsec_millis()
                            ).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "m" | "month" | "months" => {
                        // Add months
                        let total_months = datetime_tz.year() * 12 + datetime_tz.month() as i32 - 1 + amount as i32;
                        let new_year = total_months / 12;
                        let new_month = (total_months % 12 + 1) as u32;

                        // Handle day overflow (e.g., Jan 31 + 1 month = Feb 28/29)
                        let max_day = chrono::NaiveDate::from_ymd_opt(new_year, new_month + 1, 1)
                            .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(new_year + 1, 1, 1).unwrap())
                            .pred_opt()
                            .unwrap()
                            .day();
                        let new_day = datetime_tz.day().min(max_day);

                        let naive = chrono::NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(new_year, new_month, new_day).unwrap(),
                            chrono::NaiveTime::from_hms_milli_opt(
                                datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second(),
                                datetime_tz.timestamp_subsec_millis()
                            ).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "w" | "week" | "weeks" => {
                        // Add weeks (7 days)
                        datetime_tz.checked_add_signed(Duration::weeks(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: date arithmetic overflow".to_string()
                            ))?
                    }
                    "d" | "day" | "days" => {
                        // Add days
                        datetime_tz.checked_add_signed(Duration::days(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: date arithmetic overflow".to_string()
                            ))?
                    }
                    "h" | "hour" | "hours" => {
                        // Add hours
                        datetime_tz.checked_add_signed(Duration::hours(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: date arithmetic overflow".to_string()
                            ))?
                    }
                    "i" | "minute" | "minutes" => {
                        // Add minutes
                        datetime_tz.checked_add_signed(Duration::minutes(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: date arithmetic overflow".to_string()
                            ))?
                    }
                    "s" | "second" | "seconds" => {
                        // Add seconds
                        datetime_tz.checked_add_signed(Duration::seconds(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: date arithmetic overflow".to_string()
                            ))?
                    }
                    "f" | "millisecond" | "milliseconds" => {
                        // Add milliseconds
                        datetime_tz.checked_add_signed(Duration::milliseconds(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: date arithmetic overflow".to_string()
                            ))?
                    }
                    _ => return Err(DbError::ExecutionError(
                        format!("DATE_ADD: unknown unit '{}'. Valid units: y/year/years, m/month/months, w/week/weeks, d/day/days, h/hour/hours, i/minute/minutes, s/second/seconds, f/millisecond/milliseconds", unit)
                    )),
                };

                // Convert back to UTC and format as ISO 8601
                let result_utc = result_tz.with_timezone(&Utc);
                let iso_string = result_utc.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                Ok(Value::String(iso_string))
            }

            // DATE_SUBTRACT(date, amount, unit, timezone?) - subtract amount of time from date
            // This is a convenience wrapper around DATE_ADD with negated amount
            "DATE_SUBTRACT" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "DATE_SUBTRACT requires 3-4 arguments: date, amount, unit, [timezone]".to_string()
                    ));
                }

                // Negate the amount and reuse DATE_ADD logic
                let negated_amount = match &evaluated_args[1] {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Value::Number(serde_json::Number::from(-i))
                        } else if let Some(f) = n.as_f64() {
                            Value::Number(serde_json::Number::from_f64(-f).unwrap())
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_SUBTRACT: amount must be a number".to_string()
                            ));
                        }
                    }
                    _ => return Err(DbError::ExecutionError(
                        "DATE_SUBTRACT: amount must be a number".to_string()
                    )),
                };

                // Build new evaluated_args with negated amount
                let mut new_evaluated_args = evaluated_args.clone();
                new_evaluated_args[1] = negated_amount;

                // Now execute the DATE_ADD logic inline with the negated amount
                // (We can't easily call evaluate_function recursively with modified args,
                // so we just negate and fall through to DATE_ADD logic by swapping the name)

                // Actually, let's just duplicate the key parts of DATE_ADD logic here
                // but with the negated amount
                use chrono::{DateTime, Datelike, Timelike, TimeZone, Duration};
                use chrono_tz::Tz;

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> = match &new_evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_SUBTRACT: invalid timestamp".to_string()
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => return Err(DbError::ExecutionError(
                                format!("DATE_SUBTRACT: invalid timestamp: {}", timestamp_ms)
                            )),
                        }
                    }
                    Value::String(s) => {
                        DateTime::parse_from_rfc3339(s)
                            .map_err(|e| DbError::ExecutionError(
                                format!("DATE_SUBTRACT: invalid ISO 8601 date '{}': {}", s, e)
                            ))?
                            .with_timezone(&Utc)
                    }
                    _ => return Err(DbError::ExecutionError(
                        "DATE_SUBTRACT: first argument must be a timestamp or ISO 8601 string".to_string()
                    )),
                };

                // Parse the negated amount
                let amount = match &new_evaluated_args[1] {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_SUBTRACT: amount must be a number".to_string()
                            ));
                        }
                    }
                    _ => return Err(DbError::ExecutionError(
                        "DATE_SUBTRACT: amount must be a number".to_string()
                    )),
                };

                // Parse the unit
                let unit = new_evaluated_args[2].as_str()
                    .ok_or_else(|| DbError::ExecutionError(
                        "DATE_SUBTRACT: unit must be a string".to_string()
                    ))?
                    .to_lowercase();

                // Parse optional timezone
                let tz: Tz = if new_evaluated_args.len() == 4 {
                    let tz_str = new_evaluated_args[3].as_str()
                        .ok_or_else(|| DbError::ExecutionError(
                            "DATE_SUBTRACT: timezone must be a string".to_string()
                        ))?;
                    tz_str.parse::<Tz>()
                        .map_err(|_| DbError::ExecutionError(
                            format!("DATE_SUBTRACT: unknown timezone '{}'", tz_str)
                        ))?
                } else {
                    chrono_tz::UTC
                };

                // Convert to the target timezone for calculation
                let datetime_tz = datetime_utc.with_timezone(&tz);

                // Perform the addition based on unit (amount is already negated)
                let result_tz: DateTime<Tz> = match unit.as_str() {
                    "y" | "year" | "years" => {
                        let new_year = datetime_tz.year() + amount as i32;
                        let naive = chrono::NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(new_year, datetime_tz.month(), datetime_tz.day())
                                .unwrap_or_else(|| {
                                    chrono::NaiveDate::from_ymd_opt(new_year, datetime_tz.month(), 28).unwrap()
                                }),
                            chrono::NaiveTime::from_hms_milli_opt(
                                datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second(),
                                datetime_tz.timestamp_subsec_millis()
                            ).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "m" | "month" | "months" => {
                        let total_months = datetime_tz.year() * 12 + datetime_tz.month() as i32 - 1 + amount as i32;
                        let new_year = total_months / 12;
                        let new_month = (total_months % 12 + 1) as u32;

                        let max_day = chrono::NaiveDate::from_ymd_opt(new_year, new_month + 1, 1)
                            .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(new_year + 1, 1, 1).unwrap())
                            .pred_opt()
                            .unwrap()
                            .day();
                        let new_day = datetime_tz.day().min(max_day);

                        let naive = chrono::NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(new_year, new_month, new_day).unwrap(),
                            chrono::NaiveTime::from_hms_milli_opt(
                                datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second(),
                                datetime_tz.timestamp_subsec_millis()
                            ).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "w" | "week" | "weeks" => {
                        datetime_tz.checked_add_signed(Duration::weeks(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: date arithmetic overflow".to_string()
                            ))?
                    }
                    "d" | "day" | "days" => {
                        datetime_tz.checked_add_signed(Duration::days(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: date arithmetic overflow".to_string()
                            ))?
                    }
                    "h" | "hour" | "hours" => {
                        datetime_tz.checked_add_signed(Duration::hours(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: date arithmetic overflow".to_string()
                            ))?
                    }
                    "i" | "minute" | "minutes" => {
                        datetime_tz.checked_add_signed(Duration::minutes(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: date arithmetic overflow".to_string()
                            ))?
                    }
                    "s" | "second" | "seconds" => {
                        datetime_tz.checked_add_signed(Duration::seconds(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: date arithmetic overflow".to_string()
                            ))?
                    }
                    "f" | "millisecond" | "milliseconds" => {
                        datetime_tz.checked_add_signed(Duration::milliseconds(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: date arithmetic overflow".to_string()
                            ))?
                    }
                    _ => return Err(DbError::ExecutionError(
                        format!("DATE_SUBTRACT: unknown unit '{}'. Valid units: y/year/years, m/month/months, w/week/weeks, d/day/days, h/hour/hours, i/minute/minutes, s/second/seconds, f/millisecond/milliseconds", unit)
                    )),
                };

                // Convert back to UTC and format as ISO 8601
                let result_utc = result_tz.with_timezone(&Utc);
                let iso_string = result_utc.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                Ok(Value::String(iso_string))
            }

            // DATE_DIFF(date1, date2, unit, asFloat?, timezone1?, timezone2?) - calculate difference between dates
            "DATE_DIFF" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 6 {
                    return Err(DbError::ExecutionError(
                        "DATE_DIFF requires 3-6 arguments: date1, date2, unit, [asFloat], [timezone1], [timezone2]".to_string()
                    ));
                }

                use chrono::{DateTime, Datelike, TimeZone};
                use chrono_tz::Tz;

                // Parse date1 (can be timestamp or ISO string)
                let datetime1_utc: DateTime<Utc> = match &evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_DIFF: invalid timestamp for date1".to_string()
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => return Err(DbError::ExecutionError(
                                format!("DATE_DIFF: invalid timestamp for date1: {}", timestamp_ms)
                            )),
                        }
                    }
                    Value::String(s) => {
                        DateTime::parse_from_rfc3339(s)
                            .map_err(|e| DbError::ExecutionError(
                                format!("DATE_DIFF: invalid ISO 8601 date for date1 '{}': {}", s, e)
                            ))?
                            .with_timezone(&Utc)
                    }
                    _ => return Err(DbError::ExecutionError(
                        "DATE_DIFF: date1 must be a timestamp or ISO 8601 string".to_string()
                    )),
                };

                // Parse date2 (can be timestamp or ISO string)
                let datetime2_utc: DateTime<Utc> = match &evaluated_args[1] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_DIFF: invalid timestamp for date2".to_string()
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => return Err(DbError::ExecutionError(
                                format!("DATE_DIFF: invalid timestamp for date2: {}", timestamp_ms)
                            )),
                        }
                    }
                    Value::String(s) => {
                        DateTime::parse_from_rfc3339(s)
                            .map_err(|e| DbError::ExecutionError(
                                format!("DATE_DIFF: invalid ISO 8601 date for date2 '{}': {}", s, e)
                            ))?
                            .with_timezone(&Utc)
                    }
                    _ => return Err(DbError::ExecutionError(
                        "DATE_DIFF: date2 must be a timestamp or ISO 8601 string".to_string()
                    )),
                };

                // Parse the unit
                let unit = evaluated_args[2].as_str()
                    .ok_or_else(|| DbError::ExecutionError(
                        "DATE_DIFF: unit must be a string".to_string()
                    ))?
                    .to_lowercase();

                // Parse optional asFloat (default: false)
                let as_float = if evaluated_args.len() >= 4 {
                    evaluated_args[3].as_bool().unwrap_or(false)
                } else {
                    false
                };

                // Parse optional timezones
                let (tz1, tz2) = if evaluated_args.len() >= 5 {
                    let tz1_str = evaluated_args[4].as_str()
                        .ok_or_else(|| DbError::ExecutionError(
                            "DATE_DIFF: timezone1 must be a string".to_string()
                        ))?;
                    let tz1: Tz = tz1_str.parse()
                        .map_err(|_| DbError::ExecutionError(
                            format!("DATE_DIFF: unknown timezone1 '{}'", tz1_str)
                        ))?;

                    let tz2 = if evaluated_args.len() >= 6 {
                        let tz2_str = evaluated_args[5].as_str()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_DIFF: timezone2 must be a string".to_string()
                            ))?;
                        tz2_str.parse::<Tz>()
                            .map_err(|_| DbError::ExecutionError(
                                format!("DATE_DIFF: unknown timezone2 '{}'", tz2_str)
                            ))?
                    } else {
                        tz1  // If timezone2 not specified, use timezone1 for both
                    };

                    (tz1, tz2)
                } else {
                    (chrono_tz::UTC, chrono_tz::UTC)
                };

                // Convert dates to their respective timezones
                let datetime1_tz = datetime1_utc.with_timezone(&tz1);
                let datetime2_tz = datetime2_utc.with_timezone(&tz2);

                // Calculate the difference based on unit
                let diff: f64 = match unit.as_str() {
                    "y" | "year" | "years" => {
                        // Calculate year difference
                        let year_diff = datetime2_tz.year() - datetime1_tz.year();
                        if as_float {
                            // More precise calculation considering months and days
                            let month_diff = datetime2_tz.month() as i32 - datetime1_tz.month() as i32;
                            let day_diff = datetime2_tz.day() as i32 - datetime1_tz.day() as i32;
                            year_diff as f64 + (month_diff as f64 / 12.0) + (day_diff as f64 / 365.25)
                        } else {
                            year_diff as f64
                        }
                    }
                    "m" | "month" | "months" => {
                        // Calculate month difference
                        let total_months1 = datetime1_tz.year() * 12 + datetime1_tz.month() as i32;
                        let total_months2 = datetime2_tz.year() * 12 + datetime2_tz.month() as i32;
                        let month_diff = total_months2 - total_months1;
                        if as_float {
                            // Add fractional part based on days
                            let day_diff = datetime2_tz.day() as i32 - datetime1_tz.day() as i32;
                            month_diff as f64 + (day_diff as f64 / 30.0)
                        } else {
                            month_diff as f64
                        }
                    }
                    "w" | "week" | "weeks" => {
                        // Calculate week difference using milliseconds
                        let diff_ms = datetime2_utc.timestamp_millis() - datetime1_utc.timestamp_millis();
                        let weeks = diff_ms as f64 / (7.0 * 24.0 * 60.0 * 60.0 * 1000.0);
                        if as_float {
                            weeks
                        } else {
                            weeks.trunc()
                        }
                    }
                    "d" | "day" | "days" => {
                        // Calculate day difference using milliseconds
                        let diff_ms = datetime2_utc.timestamp_millis() - datetime1_utc.timestamp_millis();
                        let days = diff_ms as f64 / (24.0 * 60.0 * 60.0 * 1000.0);
                        if as_float {
                            days
                        } else {
                            days.trunc()
                        }
                    }
                    "h" | "hour" | "hours" => {
                        // Calculate hour difference using milliseconds
                        let diff_ms = datetime2_utc.timestamp_millis() - datetime1_utc.timestamp_millis();
                        let hours = diff_ms as f64 / (60.0 * 60.0 * 1000.0);
                        if as_float {
                            hours
                        } else {
                            hours.trunc()
                        }
                    }
                    "i" | "minute" | "minutes" => {
                        // Calculate minute difference using milliseconds
                        let diff_ms = datetime2_utc.timestamp_millis() - datetime1_utc.timestamp_millis();
                        let minutes = diff_ms as f64 / (60.0 * 1000.0);
                        if as_float {
                            minutes
                        } else {
                            minutes.trunc()
                        }
                    }
                    "s" | "second" | "seconds" => {
                        // Calculate second difference using milliseconds
                        let diff_ms = datetime2_utc.timestamp_millis() - datetime1_utc.timestamp_millis();
                        let seconds = diff_ms as f64 / 1000.0;
                        if as_float {
                            seconds
                        } else {
                            seconds.trunc()
                        }
                    }
                    "f" | "millisecond" | "milliseconds" => {
                        // Calculate millisecond difference
                        let diff_ms = datetime2_utc.timestamp_millis() - datetime1_utc.timestamp_millis();
                        diff_ms as f64
                    }
                    _ => return Err(DbError::ExecutionError(
                        format!("DATE_DIFF: unknown unit '{}'. Valid units: y/year/years, m/month/months, w/week/weeks, d/day/days, h/hour/hours, i/minute/minutes, s/second/seconds, f/millisecond/milliseconds", unit)
                    )),
                };

                Ok(Value::Number(serde_json::Number::from_f64(diff).unwrap()))
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
        // Normalize the value for index lookup
        // If it's a float that's actually an integer (e.g., 30.0), convert to integer
        // This handles the case where AQL parses "30" as 30.0 but data has integer 30
        let normalized_value = if let Value::Number(n) = &condition.value {
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f.is_finite() {
                    // It's a whole number, try as integer first
                    Value::Number(serde_json::Number::from(f as i64))
                } else {
                    condition.value.clone()
                }
            } else {
                condition.value.clone()
            }
        } else {
            condition.value.clone()
        };

        match condition.op {
            BinaryOperator::Equal => {
                // Try with normalized value first
                if let Some(docs) = collection.index_lookup_eq(&condition.field, &normalized_value) {
                    if !docs.is_empty() {
                        return Some(docs);
                    }
                }
                // Fall back to original value
                collection.index_lookup_eq(&condition.field, &condition.value)
            },
            BinaryOperator::GreaterThan => collection.index_lookup_gt(&condition.field, &normalized_value),
            BinaryOperator::GreaterThanOrEqual => collection.index_lookup_gte(&condition.field, &normalized_value),
            BinaryOperator::LessThan => collection.index_lookup_lt(&condition.field, &normalized_value),
            BinaryOperator::LessThanOrEqual => collection.index_lookup_lte(&condition.field, &normalized_value),
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
