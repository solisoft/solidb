//! Physical backup endpoint.
//!
//! `solidb-dump` is a *logical* export: it re-serialises every document as
//! JSONL, which is roughly 3x the on-disk size, takes minutes to restore, and
//! has no point-in-time consistency across collections. This endpoint exposes
//! RocksDB checkpoints instead — a hard-linked, instantly-taken physical copy
//! of the whole instance.
//!
//! The checkpoint is necessarily written by the server process, since it is
//! the process holding the database open; the caller names a server-side
//! destination path.

use super::system::AppState;
use crate::error::DbError;
use axum::{extract::State, response::Json};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct CreateBackupRequest {
    /// Destination directory, on the server. Must not already exist.
    pub path: String,
}

/// POST /_api/backup
///
/// Creates a physical checkpoint of the entire instance — every database and
/// collection, since they share a single RocksDB instance.
///
/// Requires Admin: the caller chooses a filesystem path the server writes to,
/// and the result contains every database's data regardless of the caller's
/// per-database grants.
pub async fn create_backup(
    State(state): State<AppState>,
    axum::Extension(claims): axum::Extension<crate::server::auth::Claims>,
    Json(req): Json<CreateBackupRequest>,
) -> Result<Json<Value>, DbError> {
    crate::server::authz_middleware::enforce(
        &claims,
        &state,
        crate::server::authorization::PermissionAction::Admin,
        None,
    )
    .await?;

    if req.path.trim().is_empty() {
        return Err(DbError::BadRequest("path is required".to_string()));
    }

    let target = std::path::PathBuf::from(&req.path);

    // Checkpointing blocks on a flush and then on hard-linking every SST file,
    // which scales with the column-family count — keep it off the async
    // runtime's worker threads.
    let storage = state.storage.clone();
    let target_for_task = target.clone();
    tokio::task::spawn_blocking(move || storage.create_checkpoint(&target_for_task))
        .await
        .map_err(|e| DbError::InternalError(format!("Backup task failed: {}", e)))??;

    tracing::info!(
        target: "audit",
        "backup: user '{}' created a checkpoint at '{}'",
        claims.sub,
        target.display()
    );

    Ok(Json(serde_json::json!({
        "status": "created",
        "path": target.to_string_lossy(),
        // Scope is not obvious from the request, so state it in the response.
        "scope": "instance",
        "note": "Whole-instance checkpoint. RocksDB hard-links SST files, so a \
                 checkpoint on the same filesystem is not protection against \
                 losing that filesystem — copy it elsewhere.",
    })))
}
