//! Validation Pipeline handlers
//!
//! Provides endpoints for running validation on AI contributions.

use axum::extract::State;
use axum::response::Json;
use axum::Extension;
use serde::Deserialize;

use crate::ai::{ValidationConfig, ValidationPipeline, ValidationResult};
use crate::error::DbError;
use crate::server::auth::Claims;
use crate::server::authorization::PermissionAction;
use crate::server::handlers::AppState;

/// These endpoints shell out to `cargo` on the server host. That is a
/// development convenience, not a database operation, so it is disabled
/// unless an operator explicitly turns it on *and* the caller is a global
/// admin.
///
/// Without both checks the route was reachable by any authenticated
/// principal (it has no `{db}` segment, so the per-database authorization
/// layer never ran and the handler did none of its own): a `viewer` key could
/// pin a CPU for 300 s per request running `cargo test` on any Cargo project
/// on disk, probe the filesystem through the "does not exist" error, and
/// reach arbitrary code execution wherever it could also drop a
/// `Cargo.toml` + `build.rs`.
fn require_validation_enabled(claims: &Claims, state: &AppState) -> Result<(), DbError> {
    let enabled = std::env::var("SOLIDB_ENABLE_AI_VALIDATION")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !enabled {
        return Err(DbError::Forbidden(
            "The AI validation endpoints are disabled; set \
             SOLIDB_ENABLE_AI_VALIDATION=1 to enable them on a development host"
                .to_string(),
        ));
    }
    // Global admin, checked synchronously against the claims' roles: this
    // handler has no database to scope against.
    let roles = claims.roles.clone().unwrap_or_default();
    if !roles.iter().any(|r| r.eq_ignore_ascii_case("admin")) {
        return Err(DbError::Forbidden(format!(
            "{:?} requires global admin",
            PermissionAction::Admin
        )));
    }
    let _ = state;
    Ok(())
}

/// Request body for running validation
#[derive(Debug, Deserialize)]
pub struct RunValidationRequest {
    /// Ignored. Kept so existing clients still deserialize; the pipeline
    /// always runs in the server's own working directory (see
    /// `run_validation_handler`).
    #[serde(default)]
    pub project_root: Option<String>,
    /// Run tests (defaults to true)
    #[serde(default = "default_true")]
    pub run_tests: bool,
    /// Run clippy (defaults to true)
    #[serde(default = "default_true")]
    pub run_clippy: bool,
    /// Run rustfmt check (defaults to true)
    #[serde(default = "default_true")]
    pub run_rustfmt: bool,
    /// Quick mode - skip tests (defaults to false)
    #[serde(default)]
    pub quick: bool,
}

fn default_true() -> bool {
    true
}

/// POST /_api/ai/validate - Run validation pipeline
///
/// Runs cargo check, clippy, and tests on the project
pub async fn run_validation_handler(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(request): Json<RunValidationRequest>,
) -> Result<Json<ValidationResult>, DbError> {
    require_validation_enabled(&claims, &state)?;

    // The project root is fixed to the server's own working directory. It
    // used to come from the request body, which chose the directory `cargo`
    // ran in and doubled as a filesystem-existence oracle.
    let project_root = ".".to_string();

    let config = ValidationConfig {
        project_root,
        run_tests: request.run_tests && !request.quick,
        run_clippy: request.run_clippy,
        run_rustfmt: request.run_rustfmt,
        test_timeout_secs: 300,
        test_filter: None,
    };

    let pipeline = ValidationPipeline::new(config);

    let result = if request.quick {
        pipeline.run_quick()
    } else {
        pipeline.run()
    };

    Ok(Json(result))
}

/// GET /_api/ai/validate/quick - Run quick validation (no tests)
///
/// Runs only cargo check and rustfmt
pub async fn run_quick_validation_handler(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<ValidationResult>, DbError> {
    require_validation_enabled(&claims, &state)?;
    let pipeline = ValidationPipeline::for_project(".");
    let result = pipeline.run_quick();
    Ok(Json(result))
}
