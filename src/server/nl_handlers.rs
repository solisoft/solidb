//! Natural Language to SDBQL Query Handler
//!
//! Translates natural language queries to SDBQL using LLM providers.

use crate::error::DbError;
use crate::sdbql::{parse, QueryExecutor};
use crate::server::handlers::AppState;
use crate::server::llm_client::{LLMClient, Message};
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

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
    fn build(
        storage: &crate::storage::StorageEngine,
        db_name: &str,
    ) -> Result<Self, DbError> {
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
                            if !fields.contains_key(&key) {
                                let type_name = match val {
                                    Value::Null => "null",
                                    Value::Bool(_) => "boolean",
                                    Value::Number(_) => "number",
                                    Value::String(_) => "string",
                                    Value::Array(_) => "array",
                                    Value::Object(_) => "object",
                                };
                                fields.insert(key, type_name.to_string());
                            }
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

/// Build system prompt for SDBQL translation
fn build_system_prompt(schema: &SchemaContext) -> String {
    format!(
        r#"You are a SDBQL query translator. Convert natural language to valid SDBQL queries.

## Database Schema
{}
## SDBQL Syntax Reference

### Basic Query Structure
```
FOR doc IN collection
  FILTER condition
  SORT field ASC|DESC
  LIMIT n
  RETURN expression
```

### Operators
- Comparison: ==, !=, >, <, >=, <=
- Logical: &&, ||, !
- String: LIKE (with % wildcard)
- Null check: == null, != null

### Aggregation
```
FOR doc IN collection
  COLLECT groupVar = doc.field
  AGGREGATE count = COUNT(1), total = SUM(doc.amount)
  RETURN {{ group: groupVar, count, total }}
```

### Common Functions
- String: UPPER(), LOWER(), CONCAT(), LENGTH(), SUBSTRING(), CONTAINS()
- Array: LENGTH(), FIRST(), LAST(), PUSH(), FLATTEN()
- Math: ABS(), ROUND(), FLOOR(), CEIL(), SUM(), AVG(), MIN(), MAX(), COUNT()
- Date: DATE_NOW(), DATE_YEAR(), DATE_MONTH(), DATE_DAY()
- Type: IS_STRING(), IS_NUMBER(), IS_ARRAY(), IS_NULL()

### Graph Traversal
```
FOR v, e IN 1..3 OUTBOUND "collection/key" edgeCollection
  RETURN v
```

## Rules
1. Return ONLY the SDBQL query - no explanations, no markdown code blocks
2. Use the exact collection and field names from the schema above
3. Use appropriate variable names (e.g., `doc`, `u` for users, `o` for orders)
4. For counting, use: `RETURN {{ count: LENGTH(FOR x IN collection RETURN 1) }}`
5. For grouping/aggregation, use COLLECT with AGGREGATE

User Query: "#,
        schema.to_prompt()
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

    // 2. Create LLM client from _system/_env collection
    let client = LLMClient::from_storage(&state.storage, req.provider.as_deref())?;

    // 3. Build initial messages
    let system_prompt = build_system_prompt(&schema);
    let mut messages = vec![
        Message::system(&system_prompt),
        Message::user(&req.query),
    ];

    let mut last_sdbql = String::new();
    let mut last_error = String::new();

    // 4. Try up to 3 times
    for attempt in 1..=3u32 {
        let sdbql = client.chat(messages.clone()).await?;

        // Clean up response (remove markdown code blocks if present)
        let sdbql = sdbql
            .trim()
            .trim_start_matches("```sql")
            .trim_start_matches("```sdbql")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
            .to_string();

        last_sdbql = sdbql.clone();

        // 5. Validate via parser
        match parse(&sdbql) {
            Ok(query) => {
                // Valid! Execute if requested
                if req.execute {
                    let executor =
                        QueryExecutor::with_database(&state.storage, db_name.clone());
                    let results = executor.execute(&query)?;
                    return Ok(Json(NLQueryResponse {
                        sdbql,
                        result: Some(results),
                        attempts: attempt,
                    }));
                }
                return Ok(Json(NLQueryResponse {
                    sdbql,
                    result: None,
                    attempts: attempt,
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

    // Failed after 3 attempts
    Err(DbError::ExecutionError(format!(
        "Failed to generate valid SDBQL after 3 attempts. Last attempt: '{}'. Error: {}",
        last_sdbql, last_error
    )))
}
