//! Validation Pipeline handlers
//!
//! Provides endpoints for running validation on AI contributions.

use axum::response::Json;
use serde::Deserialize;

use crate::ai::{ValidationConfig, ValidationPipeline, ValidationResult};
use crate::error::DbError;

/// Request body for running validation
#[derive(Debug, Deserialize)]
pub struct RunValidationRequest {
    /// Project root path (defaults to current directory)
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
    Json(request): Json<RunValidationRequest>,
) -> Result<Json<ValidationResult>, DbError> {
    let project_root = request.project_root.unwrap_or_else(|| ".".to_string());

    // Verify the path exists
    if !crate::ai::validation::path_exists(&project_root) {
        return Err(DbError::BadRequest(format!(
            "Project root does not exist: {}",
            project_root
        )));
    }

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
pub async fn run_quick_validation_handler() -> Result<Json<ValidationResult>, DbError> {
    let pipeline = ValidationPipeline::for_project(".");
    let result = pipeline.run_quick();
    Ok(Json(result))
}
