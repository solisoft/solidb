use crate::error::DbError;
use crate::server::auth::Claims;
use crate::server::authorization::PermissionAction;
use crate::server::handlers;
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct EnvVarValue {
    pub value: String,
}

/// `_env` holds provider credentials (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`,
/// ...), so every operation on it requires Admin on the target database — the
/// route layer would otherwise settle for Read on GET and Write on PUT/DELETE,
/// which is how a read-only principal used to be able to dump every key
/// (SEC-176). This matches what the docs have always promised: "API keys
/// stored in `_env` are accessible to database administrators."
async fn require_env_admin(
    claims: &Claims,
    state: &handlers::AppState,
    db_name: &str,
) -> Result<(), DbError> {
    crate::server::authz_middleware::enforce(claims, state, PermissionAction::Admin, Some(db_name))
        .await
}

/// GET /_api/database/{db}/env
/// List all environment variables for a database
pub async fn list_env_vars_handler(
    State(state): State<handlers::AppState>,
    Extension(claims): Extension<Claims>,
    Path(db_name): Path<String>,
) -> Result<impl IntoResponse, DbError> {
    require_env_admin(&claims, &state, &db_name).await?;
    let db = state.storage.get_database(&db_name)?;

    // Ensure _env collection exists
    if db.system_collection("_env").is_err() {
        return Ok(Json(std::collections::HashMap::new()));
    }

    let collection = db.system_collection("_env")?;
    let all_docs = collection.scan(None);

    let mut env_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for doc in all_docs {
        if let Some(key) = doc
            .get("_key")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
        {
            if let Some(val) = doc
                .get("value")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
            {
                env_map.insert(key.to_string(), val.to_string());
            }
        }
    }

    Ok(Json(env_map))
}

/// PUT /_api/database/{db}/env/{key}
/// Set an environment variable
pub async fn set_env_var_handler(
    State(state): State<handlers::AppState>,
    Extension(claims): Extension<Claims>,
    Path((db_name, key)): Path<(String, String)>,
    Json(payload): Json<EnvVarValue>,
) -> Result<impl IntoResponse, DbError> {
    require_env_admin(&claims, &state, &db_name).await?;
    let db = state.storage.get_database(&db_name)?;

    // Ensure _env collection exists
    if db.system_collection("_env").is_err() {
        db.create_collection("_env".to_string(), None)?;
    }

    let collection = db.system_collection("_env")?;

    let doc = serde_json::json!({
        "_key": key,
        "value": payload.value,
        "updated_at": chrono::Utc::now().to_rfc3339()
    });

    collection.insert(doc)?;

    Ok(Json(serde_json::json!({ "status": "ok", "key": key })))
}

/// DELETE /_api/database/{db}/env/{key}
/// Delete an environment variable
pub async fn delete_env_var_handler(
    State(state): State<handlers::AppState>,
    Extension(claims): Extension<Claims>,
    Path((db_name, key)): Path<(String, String)>,
) -> Result<impl IntoResponse, DbError> {
    require_env_admin(&claims, &state, &db_name).await?;
    let db = state.storage.get_database(&db_name)?;

    if db.system_collection("_env").is_err() {
        return Err(DbError::DocumentNotFound(format!(
            "Environment variable {} not found",
            key
        )));
    }

    let collection = db.system_collection("_env")?;

    match collection.delete(&key) {
        Ok(_) => Ok(Json(serde_json::json!({ "status": "deleted", "key": key }))),
        Err(_) => Err(DbError::DocumentNotFound(format!(
            "Environment variable {} not found",
            key
        ))),
    }
}
