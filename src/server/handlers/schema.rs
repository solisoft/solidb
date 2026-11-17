use super::system::AppState;
use crate::error::DbError;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};

fn default_validation_mode() -> String {
    "off".to_string()
}

/// Schema management request
#[derive(Debug, Deserialize)]
pub struct SetSchemaRequest {
    /// JSON Schema document
    pub schema: serde_json::Value,
    /// Validation mode: "off", "strict", or "lenient"
    #[serde(rename = "validationMode", default = "default_validation_mode")]
    pub validation_mode: String,
}

/// Schema response
#[derive(Debug, Serialize)]
pub struct SchemaResponse {
    pub schema: Option<serde_json::Value>,
    #[serde(rename = "validationMode")]
    pub validation_mode: String,
    pub collection: String,
}

/// Tell the peers what the collection now asks of its documents: its field
/// rules or its schema, whole, in place of what they hold.
fn log_validation(
    state: &AppState,
    database: &str,
    collection: &str,
    operation: crate::sync::protocol::Operation,
    data: Option<Vec<u8>>,
) {
    if let Some(ref log) = state.replication_log {
        let _ = log.append(crate::sync::log::LogEntry::new_op(
            database, collection, operation, "", data,
        ));
    }
}

// ==================== Schema Handlers ====================

/// Set or update JSON schema for a collection
pub async fn set_collection_schema(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<SetSchemaRequest>,
) -> Result<Json<SchemaResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Parse validation mode
    let validation_mode = match req.validation_mode.to_lowercase().as_str() {
        "strict" => crate::storage::schema::SchemaValidationMode::Strict,
        "lenient" => crate::storage::schema::SchemaValidationMode::Lenient,
        _ => crate::storage::schema::SchemaValidationMode::Off,
    };

    // Set schema
    let schema_clone = req.schema.clone();
    let schema = crate::storage::schema::CollectionSchema::new(
        "default".to_string(),
        req.schema,
        validation_mode,
    );
    let logged = serde_json::to_vec(&schema).ok();
    collection.set_json_schema(schema)?;
    log_validation(
        &state,
        &db_name,
        &coll_name,
        crate::sync::protocol::Operation::SetSchema,
        logged,
    );

    Ok(Json(SchemaResponse {
        schema: Some(schema_clone),
        validation_mode: req.validation_mode.clone(),
        collection: coll_name,
    }))
}

/// Get JSON schema for a collection
pub async fn get_collection_schema(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<SchemaResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    let schema = collection.get_json_schema();
    let (schema_value, validation_mode) = if let Some(s) = schema {
        let mode_str = match s.validation_mode {
            crate::storage::schema::SchemaValidationMode::Strict => "strict".to_string(),
            crate::storage::schema::SchemaValidationMode::Lenient => "lenient".to_string(),
            crate::storage::schema::SchemaValidationMode::Off => "off".to_string(),
        };
        (Some(s.schema.clone()), mode_str)
    } else {
        (None, "off".to_string())
    };

    Ok(Json(SchemaResponse {
        schema: schema_value,
        validation_mode,
        collection: coll_name,
    }))
}

// ==================== Field rules ====================

/// The field rules of a collection, or those to declare.
#[derive(Debug, Serialize, Deserialize)]
pub struct FieldRulesBody {
    pub rules: Vec<crate::storage::field_rules::FieldRule>,
}

/// The field rules of a collection.
pub async fn get_field_rules(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<FieldRulesBody>, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    Ok(Json(FieldRulesBody {
        rules: collection.field_rules().as_ref().clone(),
    }))
}

/// Declare the field rules of a collection, in place of those it had.
pub async fn set_field_rules(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(body): Json<FieldRulesBody>,
) -> Result<Json<FieldRulesBody>, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    collection.set_field_rules(body.rules)?;
    log_validation(
        &state,
        &db_name,
        &coll_name,
        crate::sync::protocol::Operation::SetFieldRules,
        serde_json::to_vec(collection.field_rules().as_ref()).ok(),
    );
    Ok(Json(FieldRulesBody {
        rules: collection.field_rules().as_ref().clone(),
    }))
}

/// Remove the field rules of a collection.
pub async fn delete_field_rules(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<StatusCode, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    collection.set_field_rules(Vec::new())?;
    log_validation(
        &state,
        &db_name,
        &coll_name,
        crate::sync::protocol::Operation::SetFieldRules,
        Some(b"[]".to_vec()),
    );
    Ok(StatusCode::NO_CONTENT)
}

/// Remove JSON schema from a collection
pub async fn delete_collection_schema(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<StatusCode, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    collection.remove_json_schema()?;
    log_validation(
        &state,
        &db_name,
        &coll_name,
        crate::sync::protocol::Operation::SetSchema,
        None,
    );

    Ok(StatusCode::NO_CONTENT)
}
