//! Natural Language to SDBQL Query Handler
//!
//! Translates natural language queries to SDBQL using LLM providers.
//! Maintains conversation history for few-shot learning and user corrections.

use crate::error::DbError;
use crate::sdbql::{parse, QueryExecutor};
use crate::server::handlers::AppState;
use crate::server::llm_client::{LLMClient, Message};
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Collection name for NL query history
const NL_HISTORY_COLLECTION: &str = "_nl_history";
/// Maximum number of examples to include in few-shot context
const MAX_FEW_SHOT_EXAMPLES: usize = 5;

/// Request for natural language query
#[derive(Debug, Deserialize)]
pub struct NLQueryRequest {
    /// Natural language query
    pub query: String,
    /// Execute the translated query (default: true)
    #[serde(default = "default_true")]
    pub execute: bool,
    /// LLM provider: "openai", "anthropic", "ollama"
    pub provider: Option<String>,
    /// Model override (uses env default if not specified)
    pub model: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Request for providing feedback/correction
#[derive(Debug, Deserialize)]
pub struct NLFeedbackRequest {
    /// The original natural language query
    pub query: String,
    /// The SDBQL that was generated (to correct)
    pub original_sdbql: String,
    /// The corrected SDBQL
    pub corrected_sdbql: String,
}

/// Response for natural language query
#[derive(Debug, Serialize)]
pub struct NLQueryResponse {
    /// Generated SDBQL query
    pub sdbql: String,
    /// Query results (if execute=true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Vec<Value>>,
    /// Number of LLM attempts needed
    pub attempts: u32,
    /// History entry ID (for feedback)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_id: Option<String>,
}

/// Response for feedback submission
#[derive(Debug, Serialize)]
pub struct NLFeedbackResponse {
    pub status: String,
    pub message: String,
}

/// History entry stored in _nl_history collection
#[derive(Debug, Serialize, Deserialize)]
struct NLHistoryEntry {
    /// Natural language query
    query: String,
    /// Generated SDBQL
    sdbql: String,
    /// User-corrected SDBQL (if provided)
    #[serde(skip_serializing_if = "Option::is_none")]
    corrected_sdbql: Option<String>,
    /// Whether the query executed successfully
    success: bool,
    /// Timestamp
    created_at: String,
}

/// Error response when translation fails
#[derive(Debug, Serialize)]
pub struct NLQueryError {
    pub error: String,
    pub last_attempt: Option<String>,
    pub parse_error: Option<String>,
}

/// Collection metadata for schema context
#[derive(Debug)]
struct CollectionMeta {
    name: String,
    doc_count: usize,
    fields: HashMap<String, String>, // field -> type
    indexes: Vec<String>,
}

/// Schema context for the database
struct SchemaContext {
    collections: Vec<CollectionMeta>,
}

impl SchemaContext {
    /// Build schema context from database
    fn build(storage: &crate::storage::StorageEngine, db_name: &str) -> Result<Self, DbError> {
        let db = storage.get_database(db_name)?;
        let collection_names = db.list_collections();

        let mut collections = Vec::new();

        for name in collection_names {
            // Skip system collections
            if name.starts_with('_') {
                continue;
            }

            if let Ok(coll) = db.get_collection(&name) {
                let doc_count = coll.count();

                // Sample documents to infer fields
                let sample_docs = coll.scan(Some(5));
                let mut fields: HashMap<String, String> = HashMap::new();

                for doc in sample_docs {
                    let value = doc.to_value();
                    if let Value::Object(obj) = value {
                        for (key, val) in obj {
                            fields.entry(key).or_insert_with(|| match val {
                                Value::Null => "null".to_string(),
                                Value::Bool(_) => "boolean".to_string(),
                                Value::Number(_) => "number".to_string(),
                                Value::String(_) => "string".to_string(),
                                Value::Array(_) => "array".to_string(),
                                Value::Object(_) => "object".to_string(),
                            });
                        }
                    }
                }

                // Get indexes
                let index_stats = coll.list_indexes();
                let indexes: Vec<String> = index_stats
                    .iter()
                    .map(|idx| format!("{}({})", idx.name, idx.fields.join(", ")))
                    .collect();

                collections.push(CollectionMeta {
                    name,
                    doc_count,
                    fields,
                    indexes,
                });
            }
        }

        Ok(SchemaContext { collections })
    }

    /// Convert to prompt context string
    fn to_prompt(&self) -> String {
        let mut result = String::new();

        for coll in &self.collections {
            result.push_str(&format!(
                "### Collection: `{}` ({} documents)\n",
                coll.name, coll.doc_count
            ));

            result.push_str("Fields:\n");
            let mut sorted_fields: Vec<_> = coll.fields.iter().collect();
            sorted_fields.sort_by_key(|(k, _)| *k);
            for (field, type_name) in sorted_fields {
                result.push_str(&format!("  - `{}`: {}\n", field, type_name));
            }

            if !coll.indexes.is_empty() {
                result.push_str("Indexes:\n");
                for idx in &coll.indexes {
                    result.push_str(&format!("  - {}\n", idx));
                }
            }
            result.push('\n');
        }

        result
    }
}

/// Ensure _nl_history collection exists and return it
fn ensure_history_collection(
    storage: &crate::storage::StorageEngine,
    db_name: &str,
) -> Result<crate::storage::Collection, DbError> {
    let db = storage.get_database(db_name)?;
    if db.get_collection(NL_HISTORY_COLLECTION).is_err() {
        db.create_collection(NL_HISTORY_COLLECTION.to_string(), None)?;
    }
    db.get_collection(NL_HISTORY_COLLECTION)
}

/// Load recent successful examples for few-shot learning
fn load_few_shot_examples(
    storage: &crate::storage::StorageEngine,
    db_name: &str,
) -> Vec<(String, String)> {
    let mut examples = Vec::new();

    if let Ok(db) = storage.get_database(db_name) {
        if let Ok(coll) = db.get_collection(NL_HISTORY_COLLECTION) {
            // Get recent successful entries, preferring corrected ones
            let docs = coll.scan(Some(50)); // Get more, then filter
            let mut entries: Vec<(String, String, bool, String)> = Vec::new();

            for doc in docs {
                let value = doc.to_value();
                if let (Some(query), Some(sdbql), Some(success)) = (
                    value.get("query").and_then(|v| v.as_str()),
                    value.get("sdbql").and_then(|v| v.as_str()),
                    value.get("success").and_then(|v| v.as_bool()),
                ) {
                    if success {
                        // Prefer corrected_sdbql if available
                        let final_sdbql = value
                            .get("corrected_sdbql")
                            .and_then(|v| v.as_str())
                            .unwrap_or(sdbql);
                        let has_correction = value.get("corrected_sdbql").is_some();
                        let created_at = value
                            .get("created_at")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        entries.push((
                            query.to_string(),
                            final_sdbql.to_string(),
                            has_correction,
                            created_at,
                        ));
                    }
                }
            }

            // Sort: corrected ones first, then by recency
            entries.sort_by(|a, b| {
                // Corrected entries get priority
                match (a.2, b.2) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => b.3.cmp(&a.3), // More recent first
                }
            });

            // Take top examples
            for (query, sdbql, _, _) in entries.into_iter().take(MAX_FEW_SHOT_EXAMPLES) {
                examples.push((query, sdbql));
            }
        }
    }

    examples
}

/// Save a query to history
fn save_to_history(
    storage: &crate::storage::StorageEngine,
    db_name: &str,
    query: &str,
    sdbql: &str,
    success: bool,
) -> Option<String> {
    if let Ok(coll) = ensure_history_collection(storage, db_name) {
        let entry = NLHistoryEntry {
            query: query.to_string(),
            sdbql: sdbql.to_string(),
            corrected_sdbql: None,
            success,
            created_at: Utc::now().to_rfc3339(),
        };
        if let Ok(value) = serde_json::to_value(&entry) {
            if let Ok(doc) = coll.insert(value) {
                return Some(doc.key.clone());
            }
        }
    }
    None
}

/// Build system prompt for SDBQL translation with few-shot examples
fn build_system_prompt(
    schema: &SchemaContext,
    provider: Option<&str>,
    examples: &[(String, String)],
) -> String {
    let reference = if provider.is_some_and(|p| p.eq_ignore_ascii_case("ollama")) {
        // Use condensed reference for local models to improve speed
        r#"SDBQL Basic Syntax:
- FOR doc IN collection FILTER doc.field == value RETURN doc
- Operators: ==, !=, <, <=, >, >=, AND, OR, NOT, LIKE, IN
- Functions: LENGTH(), COUNT(), SUM(), AVG(), MIN(), MAX()
- Aggregation: COLLECT var = doc.field INTO group
- Sorting: SORT doc.field ASC/DESC LIMIT 10"#
    } else {
        // Use full reference for cloud models
        include_str!("../../docs/SDBQL_REFERENCE.md")
    };

    // Build examples section
    let examples_section = if examples.is_empty() {
        String::new()
    } else {
        let mut section = String::from("\n## Successful Examples from This Database\n");
        for (i, (query, sdbql)) in examples.iter().enumerate() {
            section.push_str(&format!(
                "Example {}:\n  Query: \"{}\"\n  SDBQL: {}\n\n",
                i + 1,
                query,
                sdbql
            ));
        }
        section
    };

    format!(
        r#"You are a SDBQL query translator. Convert natural language to valid SDBQL queries.

## Database Schema
{}

## SDBQL Syntax Reference
{}
{}
## Rules / Best Practices
1. Return ONLY the SDBQL query - no explanations, no markdown code blocks.
2. Use the exact collection and field names from the schema.
3. For aggregations, prefer `COLLECT ... WITH COUNT INTO ...` syntax.
4. For searching text, prefer `LIKE` for simple patterns.
5. For recent items, sort by timestamp field DESC and LIMIT.
6. Use `LET` variables to simplify complex logic or subqueries.
7. Use `Not In` operator `x NOT IN [...]` instead of `!(x IN [...])`.

User Query: "#,
        schema.to_prompt(),
        reference,
        examples_section
    )
}

/// POST /_api/database/{db}/nl
/// Translate natural language to SDBQL and optionally execute
pub async fn nl_query(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<NLQueryRequest>,
) -> Result<impl IntoResponse, DbError> {
    // 1. Build schema context
    let schema = SchemaContext::build(&state.storage, &db_name)?;

    if schema.collections.is_empty() {
        return Err(DbError::ExecutionError(
            "No collections found in database. Create collections first.".to_string(),
        ));
    }

    // 2. Create LLM client from _env collection (checks current db, then _system, then OS env)
    let client = LLMClient::from_storage(&state.storage, &db_name, req.provider.as_deref())?;

    // 3. Load few-shot examples from history
    let examples = load_few_shot_examples(&state.storage, &db_name);

    // 4. Build initial messages with few-shot examples
    let system_prompt = build_system_prompt(&schema, req.provider.as_deref(), &examples);
    let mut messages = vec![Message::system(&system_prompt), Message::user(&req.query)];

    let mut last_sdbql = String::new();
    let mut last_error = String::new();

    // 5. Try up to 3 times
    for attempt in 1..=3u32 {
        let sdbql = client.chat(messages.clone()).await?;

        // Clean up response (extract code block if present)
        let sdbql = if let Some(start) = sdbql.find("```") {
            let rest = &sdbql[start + 3..];
            // Skip language identifier (e.g., "sdbql", "sql")
            let code_start = if let Some(newline_pos) = rest.find('\n') {
                newline_pos + 1
            } else {
                0
            };

            // Find end of block
            let code_end = if let Some(end) = rest[code_start..].find("```") {
                end
            } else {
                rest.len() - code_start
            };

            rest[code_start..code_start + code_end].trim().to_string()
        } else {
            // No code block, assume raw query
            sdbql.trim().to_string()
        };

        last_sdbql = sdbql.clone();

        // 6. Validate via parser
        match parse(&sdbql) {
            Ok(query) => {
                // Valid! Save to history for future few-shot learning
                let history_id =
                    save_to_history(&state.storage, &db_name, &req.query, &sdbql, true);

                // Execute if requested
                if req.execute {
                    let executor = QueryExecutor::with_database(&state.storage, db_name.clone());
                    let results = executor.execute(&query)?;
                    return Ok(Json(NLQueryResponse {
                        sdbql,
                        result: Some(results),
                        attempts: attempt,
                        history_id,
                    }));
                }
                return Ok(Json(NLQueryResponse {
                    sdbql,
                    result: None,
                    attempts: attempt,
                    history_id,
                }));
            }
            Err(e) => {
                last_error = e.to_string();
                // Add error context for retry
                messages.push(Message::assistant(&sdbql));
                messages.push(Message::user(&format!(
                    "Parse error: {}. Please fix the SDBQL query. Return ONLY the corrected query.",
                    e
                )));
            }
        }
    }

    // Failed after 3 attempts - save failure to history (but don't use for few-shot)
    let _ = save_to_history(&state.storage, &db_name, &req.query, &last_sdbql, false);

    Err(DbError::ExecutionError(format!(
        "Failed to generate valid SDBQL after 3 attempts. Last attempt: '{}'. Error: {}",
        last_sdbql, last_error
    )))
}

/// POST /_api/database/{db}/nl/feedback
/// Submit feedback to correct a generated SDBQL query
pub async fn nl_feedback(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<NLFeedbackRequest>,
) -> Result<impl IntoResponse, DbError> {
    // Validate the corrected SDBQL parses
    if let Err(e) = parse(&req.corrected_sdbql) {
        return Err(DbError::ExecutionError(format!(
            "Corrected SDBQL is invalid: {}",
            e
        )));
    }

    // Find and update the history entry, or create a new one
    let coll = ensure_history_collection(&state.storage, &db_name)?;

    // Search for matching entry
    let docs = coll.scan(Some(100));
    let mut found_key: Option<String> = None;

    for doc in docs {
        let value = doc.to_value();
        if let (Some(query), Some(sdbql)) = (
            value.get("query").and_then(|v| v.as_str()),
            value.get("sdbql").and_then(|v| v.as_str()),
        ) {
            if query == req.query && sdbql == req.original_sdbql {
                found_key = Some(doc.key.clone());
                break;
            }
        }
    }

    if let Some(key) = found_key {
        // Update existing entry with correction
        let update = serde_json::json!({
            "corrected_sdbql": req.corrected_sdbql,
            "success": true
        });
        coll.update(&key, update)?;
        Ok(Json(NLFeedbackResponse {
            status: "updated".to_string(),
            message: "Correction saved. Future queries will learn from this.".to_string(),
        }))
    } else {
        // Create new entry with correction
        let entry = NLHistoryEntry {
            query: req.query,
            sdbql: req.original_sdbql,
            corrected_sdbql: Some(req.corrected_sdbql),
            success: true,
            created_at: Utc::now().to_rfc3339(),
        };
        let value = serde_json::to_value(&entry)?;
        coll.insert(value)?;
        Ok(Json(NLFeedbackResponse {
            status: "created".to_string(),
            message: "Correction saved as new example. Future queries will learn from this."
                .to_string(),
        }))
    }
}
