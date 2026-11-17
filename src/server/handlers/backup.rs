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
    /// Destination directory. Relative paths are resolved under
    /// `SOLIDB_BACKUP_ROOT` (or `{data_dir}/backups`). Absolute paths must
    /// stay inside that root.
    pub path: String,
}

fn resolve_backup_path(data_dir: &str, requested: &str) -> Result<std::path::PathBuf, DbError> {
    let root = std::env::var("SOLIDB_BACKUP_ROOT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(data_dir).join("backups"));
    std::fs::create_dir_all(&root).map_err(|e| {
        DbError::InternalError(format!(
            "Failed to create backup root '{}': {}",
            root.display(),
            e
        ))
    })?;
    let root = root
        .canonicalize()
        .map_err(|e| DbError::InternalError(format!("Failed to resolve backup root: {}", e)))?;

    let requested = std::path::Path::new(requested);
    if requested
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(DbError::BadRequest(
            "backup path must not contain '..'".to_string(),
        ));
    }
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let parent = candidate.parent().unwrap_or(&root);
    if parent.exists() {
        let parent_canon = parent
            .canonicalize()
            .map_err(|e| DbError::BadRequest(format!("invalid backup path: {}", e)))?;
        if !parent_canon.starts_with(&root) {
            return Err(DbError::BadRequest(
                "backup path must be inside the configured backup root".to_string(),
            ));
        }
    } else if !candidate.starts_with(&root) {
        return Err(DbError::BadRequest(
            "backup path must be inside the configured backup root".to_string(),
        ));
    }
    Ok(candidate)
}

/// The file a checkpoint holds that says where the replication log stood
/// when it was taken.
pub const LOG_POSITION_FILE: &str = "SYNC_LOG_POSITION";

#[derive(Debug, Deserialize)]
pub struct ArchiveLogRequest {
    /// Destination file, under the backup root as a checkpoint's path is.
    pub path: String,
    /// Entries after this sequence are written (a checkpoint's
    /// `log_position.sequence`); 0, or absent: every entry the log holds.
    #[serde(default)]
    pub after: u64,
}

/// POST /_api/backup/log
///
/// Writes the entries of the replication log after a sequence to a file:
/// the history a point-in-time restore replays over a checkpoint. The log
/// keeps that history only when `SOLIDB_SYNC_LOG_KEEP_SECS` says so (or
/// while a peer has not received it): the answer names the oldest entry
/// written, for the caller to see a gap.
///
/// Requires Admin, as a checkpoint does.
pub async fn archive_log(
    State(state): State<AppState>,
    axum::Extension(claims): axum::Extension<crate::server::auth::Claims>,
    Json(req): Json<ArchiveLogRequest>,
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
    let Some(log) = state.replication_log.clone() else {
        return Err(DbError::BadRequest(
            "this server has no replication log".to_string(),
        ));
    };
    if !log.is_active() {
        return Err(DbError::BadRequest(
            "the replication log is not being written: start the server with \
             SOLIDB_SYNC_LOG_KEEP_SECS to keep the history a restore replays"
                .to_string(),
        ));
    }
    let target = resolve_backup_path(state.storage.data_dir(), &req.path)?;
    if target.exists() {
        return Err(DbError::BadRequest(format!(
            "log archive target '{}' already exists",
            target.display()
        )));
    }
    let after = req.after;
    let oldest = log.oldest_sequence();
    let file = target.clone();
    let summary = tokio::task::spawn_blocking(move || log.archive_after(after, &file))
        .await
        .map_err(|e| DbError::InternalError(format!("Log archive task failed: {}", e)))?
        .map_err(|e| DbError::InternalError(format!("Failed to write the log archive: {}", e)))?;
    tracing::info!(
        target: "audit",
        "backup: user '{}' archived {} log entries at '{}'",
        claims.sub,
        summary.entries,
        target.display()
    );
    // An entry between `after` and the oldest one kept was pruned: the
    // archive does not follow the checkpoint it was asked for.
    let complete = oldest.is_none_or(|oldest| oldest <= after + 1);
    Ok(Json(serde_json::json!({
        "status": "created",
        "path": target.to_string_lossy(),
        "after": after,
        "entries": summary.entries,
        "first_sequence": summary.first_sequence,
        "last_sequence": summary.last_sequence,
        "first_timestamp": summary.first_timestamp,
        "last_timestamp": summary.last_timestamp,
        "complete": complete,
    })))
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

    let target = resolve_backup_path(state.storage.data_dir(), &req.path)?;

    // Where the replication log stands, read before the checkpoint: every
    // entry up to there is in the checkpoint, and replaying from there
    // applies again, at worst, a few writes it already holds.
    let position = state.replication_log.as_ref().map(|log| {
        serde_json::json!({
            "sequence": log.current_sequence(),
            "node_id": log.node_id(),
            "log_active": log.is_active(),
            "taken_at_ms": chrono::Utc::now().timestamp_millis(),
        })
    });

    // Checkpointing blocks on a flush and then on hard-linking every SST file,
    // which scales with the column-family count — keep it off the async
    // runtime's worker threads.
    let storage = state.storage.clone();
    let target_for_task = target.clone();
    tokio::task::spawn_blocking(move || storage.create_checkpoint(&target_for_task))
        .await
        .map_err(|e| DbError::InternalError(format!("Backup task failed: {}", e)))??;

    // Written in the checkpoint, beside its files: what a point-in-time
    // restore starts the replay from.
    if let Some(position) = &position {
        let file = target.join(LOG_POSITION_FILE);
        std::fs::write(&file, position.to_string()).map_err(|e| {
            DbError::InternalError(format!("Failed to write '{}': {}", file.display(), e))
        })?;
    }

    tracing::info!(
        target: "audit",
        "backup: user '{}' created a checkpoint at '{}'",
        claims.sub,
        target.display()
    );

    Ok(Json(serde_json::json!({
        "status": "created",
        "path": target.to_string_lossy(),
        "log_position": position,
        // Scope is not obvious from the request, so state it in the response.
        "scope": "instance",
        "note": "Whole-instance checkpoint. RocksDB hard-links SST files, so a \
                 checkpoint on the same filesystem is not protection against \
                 losing that filesystem — copy it elsewhere.",
    })))
}
