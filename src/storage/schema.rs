//! JSON Schema validation module
//!
//! This module provides optional JSON Schema validation for documents.
//! Schemas can be attached to collections and validation modes control
//! how strictly they are enforced.

use jsonschema::validator_for;
use serde_json::Value;

/// Validation mode for schema enforcement
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SchemaValidationMode {
    /// No validation (default)
    #[default]
    Off,
    /// Strict validation - reject any document that doesn't match schema
    Strict,
    /// Lenient validation - accept document but log warnings for violations
    Lenient,
}

/// Schema metadata stored in RocksDB
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CollectionSchema {
    /// Schema name (for future versioning support)
    pub name: String,
    /// JSON Schema document
    pub schema: Value,
    /// Validation mode
    pub validation_mode: SchemaValidationMode,
}

impl CollectionSchema {
    /// Create a new JSON schema
    pub fn new(name: String, schema: Value, validation_mode: SchemaValidationMode) -> Self {
        Self {
            name,
            schema,
            validation_mode,
        }
    }

    /// Get validation mode
    pub fn validation_mode(&self) -> SchemaValidationMode {
        self.validation_mode
    }

    /// Check if validation is enabled
    pub fn is_enabled(&self) -> bool {
        self.validation_mode != SchemaValidationMode::Off
    }
}

/// Compiled schema validator
pub struct SchemaValidator {
    schema: CollectionSchema,
    validator: Option<jsonschema::Validator>,
}

impl SchemaValidator {
    /// Create a new schema validator
    pub fn new(schema: CollectionSchema) -> Result<Self, SchemaCompilationError> {
        let validator = if schema.is_enabled() {
            Some(
                validator_for(&schema.schema)
                    .map_err(|e| SchemaCompilationError::InvalidSchema(e.to_string()))?,
            )
        } else {
            None
        };

        Ok(Self { schema, validator })
    }

    /// Validate a document against the schema
    pub fn validate(&self, document: &Value) -> Result<(), SchemaValidationError> {
        if let Some(ref validator) = self.validator {
            match self.schema.validation_mode {
                SchemaValidationMode::Off => Ok(()),
                SchemaValidationMode::Strict => {
                    let mut violations = Vec::new();
                    for error in validator.iter_errors(document) {
                        violations.push(ValidationViolation {
                            instance_path: error.instance_path().to_string(),
                            schema_path: error.schema_path().to_string(),
                            error: error.to_string(),
                        });
                    }
                    if violations.is_empty() {
                        Ok(())
                    } else {
                        Err(SchemaValidationError::SchemaViolations(violations))
                    }
                }
                SchemaValidationMode::Lenient => {
                    for error in validator.iter_errors(document) {
                        tracing::warn!(
                            schema = %self.schema.name,
                            path = %error.instance_path(),
                            violation = %error,
                            "Schema validation warning (lenient mode)"
                        );
                    }
                    Ok(())
                }
            }
        } else {
            Ok(())
        }
    }

    /// Get the underlying schema
    pub fn schema(&self) -> &CollectionSchema {
        &self.schema
    }
}

/// Schema compilation error (when setting an invalid schema)
#[derive(Debug, thiserror::Error)]
pub enum SchemaCompilationError {
    #[error("Invalid JSON Schema: {0}")]
    InvalidSchema(String),
}

/// Schema validation error (when a document doesn't match schema)
#[derive(Debug, thiserror::Error, serde::Serialize)]
pub enum SchemaValidationError {
    #[error("Schema validation failed with violations")]
    SchemaViolations(Vec<ValidationViolation>),
}

/// Individual schema violation detail
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationViolation {
    /// JSON Path to the violating instance
    pub instance_path: String,
    /// JSON Path to the schema keyword that failed
    pub schema_path: String,
    /// Human-readable error message
    pub error: String,
}

/// Schema validation result
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether validation passed
    pub is_valid: bool,
    /// List of violations (empty if valid)
    pub violations: Vec<ValidationViolation>,
}

impl ValidationResult {
    /// Create a successful result
    pub fn valid() -> Self {
        Self {
            is_valid: true,
            violations: Vec::new(),
        }
    }

    /// Create a failed result
    pub fn invalid(violations: Vec<ValidationViolation>) -> Self {
        Self {
            is_valid: false,
            violations,
        }
    }
}
