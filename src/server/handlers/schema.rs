use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use crate::error::DbError;
use super::system::AppState;

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
    collection.set_json_schema(crate::storage::schema::CollectionSchema::new(
        "default".to_string(),
        req.schema,
        validation_mode,
    ))?;

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

/// Remove JSON schema from a collection
pub async fn delete_collection_schema(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<StatusCode, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    collection.remove_json_schema()?;

    Ok(StatusCode::NO_CONTENT)
}
