//! JSON Schema validation module
//!
//! This module provides optional JSON Schema validation for documents.
//! Schemas can be attached to collections and validation modes control
//! how strictly they are enforced.

use jsonschema::{validator_for, ValidationError as JsonSchemaError};
use serde_json::Value;

/// Validation mode for schema enforcement
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SchemaValidationMode {
    /// No validation (default)
    Off,
    /// Strict validation - reject any document that doesn't match schema
    Strict,
    /// Lenient validation - accept document but log warnings for violations
    Lenient,
}

impl Default for SchemaValidationMode {
    fn default() -> Self {
        Self::Off
    }
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
            Some(validator_for(&schema.schema).map_err(|e| {
                SchemaCompilationError::InvalidSchema(e.to_string())
            })?)
        } else {
            None
        };

        Ok(Self {
            schema,
            validator,
        })
    }

    /// Validate a document against the schema
    pub fn validate(&self, document: &Value) -> Result<(), SchemaValidationError> {
        if let Some(ref validator) = self.validator {
            match self.schema.validation_mode {
                SchemaValidationMode::Off => Ok(()),
                SchemaValidationMode::Strict => {
                    let validation_result = validator.validate(document);
                    if let Err(errors) = validation_result {
                        let violations: Vec<ValidationViolation> = errors
                            .iter()
                            .map(|e| ValidationViolation {
                                instance_path: e.instance_path().to_string(),
                                schema_path: e.schema_path().to_string(),
                                error: e.to_string(),
                            })
                            .collect();

                        Err(SchemaValidationError::SchemaViolations(violations))
                    } else {
                        Ok(())
                    }
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
    #[error("Schema validation failed: {0} violation(s): {1:?}")]
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

impl Default for SchemaValidationMode {
    fn default() -> Self {
        Self::Off
    }
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

        Ok(Self {
            schema,
            validator,
        })
    }

    /// Validate a document against the schema
    pub fn validate(&self, document: &Value) -> Result<(), SchemaValidationError> {
        if let Some(ref validator) = self.validator {
            match self.schema.validation_mode {
                SchemaValidationMode::Off => Ok(()),
                SchemaValidationMode::Strict => {
                    let validation_result = validator.validate(document);
                    if let Err(errors) = validation_result {
                        let violations: Vec<ValidationViolation> = errors
                            .iter()
                            .map(|e| ValidationViolation {
                                instance_path: e.instance_path.to_string(),
                                schema_path: e.schema_path.to_string(),
                                error: e.to_string(),
                            })
                            .collect();

                        Err(SchemaValidationError::SchemaViolations(violations))
                    } else {
                        Ok(())
                    }
                }
                SchemaValidationMode::Lenient => {
                    for error in validator.iter_errors(document) {
                        tracing::warn!(
                            schema = %self.schema.name,
                            path = %error.instance_path,
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
    #[error("Schema validation failed with {0} violation(s): {1:?}")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validation_mode_default() {
        let mode: SchemaValidationMode = Default::default();
        assert_eq!(mode, SchemaValidationMode::Off);
    }

    #[test]
    fn test_schema_creation() {
        let schema = CollectionSchema::new(
            "test".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "age": { "type": "number" }
                },
                "required": ["name"]
            }),
            SchemaValidationMode::Strict,
        );

        assert_eq!(schema.name, "test");
        assert!(schema.is_enabled());
        assert_eq!(schema.validation_mode(), SchemaValidationMode::Strict);
    }

    #[test]
    fn test_schema_off_mode() {
        let schema = CollectionSchema::new(
            "test".to_string(),
            json!({"type": "object"}),
            SchemaValidationMode::Off,
        );

        assert!(!schema.is_enabled());
    }

    #[test]
    fn test_validator_valid_document() {
        let schema = CollectionSchema::new(
            "test".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "age": { "type": "number" }
                },
                "required": ["name"]
            }),
            SchemaValidationMode::Strict,
        );

        let validator = SchemaValidator::new(schema).unwrap();
        let doc = json!({
            "name": "Alice",
            "age": 30
        });

        assert!(validator.validate(&doc).is_ok());
    }

    #[test]
    fn test_validator_invalid_document() {
        let schema = CollectionSchema::new(
            "test".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "age": { "type": "number" }
                },
                "required": ["name"]
            }),
            SchemaValidationMode::Strict,
        );

        let validator = SchemaValidator::new(schema).unwrap();
        let doc = json!({
            "age": "thirty" // Missing 'name', wrong type for 'age'
        });

        let result = validator.validate(&doc);
        assert!(result.is_err());

        if let Err(SchemaValidationError::SchemaViolations(violations)) = result {
            assert!(!violations.is_empty());
            assert!(violations.len() > 0);
        } else {
            panic!("Expected SchemaViolations error");
        }
    }

    #[test]
    fn test_validator_off_mode() {
        let schema = CollectionSchema::new(
            "test".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                }
            }),
            SchemaValidationMode::Off,
        );

        let validator = SchemaValidator::new(schema).unwrap();
        let doc = json!({ "name": 123 }); // Wrong type but validation is off

        assert!(validator.validate(&doc).is_ok());
    }

    #[test]
    fn test_validation_result() {
        let valid = ValidationResult::valid();
        assert!(valid.is_valid);
        assert!(valid.violations.is_empty());

        let violations = vec![ValidationViolation {
            instance_path: "/name".to_string(),
            schema_path: "/properties/name/type".to_string(),
            error: "expected string".to_string(),
        }];
        let invalid = ValidationResult::invalid(violations);
        assert!(!invalid.is_valid);
        assert_eq!(invalid.violations.len(), 1);
    }
}
