//! Validation Pipeline for AI-generated code
//!
//! This module provides a multi-stage validation pipeline that runs:
//! 1. Syntax check (rustfmt)
//! 2. Linting (clippy)
//! 3. Type checking (cargo check)
//! 4. Unit tests (cargo test)
//! 5. Schema validation
//! 6. Security analysis

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use super::agent::{ValidationMessage, ValidationResult, ValidationStage, ValidationStageResult};

/// Configuration for the validation pipeline
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    /// Project root directory
    pub project_root: String,
    /// Whether to run tests
    pub run_tests: bool,
    /// Whether to run clippy
    pub run_clippy: bool,
    /// Whether to run rustfmt check
    pub run_rustfmt: bool,
    /// Test timeout in seconds
    pub test_timeout_secs: u64,
    /// Specific test filter (optional)
    pub test_filter: Option<String>,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            project_root: ".".to_string(),
            run_tests: true,
            run_clippy: true,
            run_rustfmt: true,
            test_timeout_secs: 300,
            test_filter: None,
        }
    }
}

/// The validation pipeline runner
pub struct ValidationPipeline {
    config: ValidationConfig,
}

impl ValidationPipeline {
    /// Create a new validation pipeline with the given config
    pub fn new(config: ValidationConfig) -> Self {
        Self { config }
    }

    /// Create a pipeline with default config for the given project root
    pub fn for_project(project_root: &str) -> Self {
        Self::new(ValidationConfig {
            project_root: project_root.to_string(),
            ..Default::default()
        })
    }

    /// Run the full validation pipeline
    pub fn run(&self) -> ValidationResult {
        let mut result = ValidationResult::new();

        // Stage 1: Syntax (rustfmt)
        if self.config.run_rustfmt {
            result.add_stage(self.run_rustfmt());
        }

        // Stage 2: Linting (clippy)
        if self.config.run_clippy {
            result.add_stage(self.run_clippy());
        }

        // Stage 3: Type checking (cargo check)
        result.add_stage(self.run_cargo_check());

        // Stage 4: Unit tests
        if self.config.run_tests {
            result.add_stage(self.run_tests());
        }

        result
    }

    /// Run only quick checks (no tests)
    pub fn run_quick(&self) -> ValidationResult {
        let mut result = ValidationResult::new();

        if self.config.run_rustfmt {
            result.add_stage(self.run_rustfmt());
        }

        result.add_stage(self.run_cargo_check());

        result
    }

    /// Run rustfmt check
    fn run_rustfmt(&self) -> ValidationStageResult {
        let start = Instant::now();

        let output = Command::new("cargo")
            .args(["fmt", "--", "--check"])
            .current_dir(&self.config.project_root)
            .output();

        let duration_ms = start.elapsed().as_millis() as u64;

        match output {
            Ok(output) => {
                let passed = output.status.success();
                let mut errors = Vec::new();

                if !passed {
                    let stderr = String::from_utf8_lossy(&output.stdout);
                    for line in stderr.lines() {
                        if line.starts_with("Diff in") {
                            errors.push(ValidationMessage {
                                file: extract_file_from_diff(line),
                                line: None,
                                column: None,
                                message: "File needs formatting".to_string(),
                                code: Some("rustfmt".to_string()),
                            });
                        }
                    }

                    if errors.is_empty() {
                        errors.push(ValidationMessage {
                            file: None,
                            line: None,
                            column: None,
                            message: "Code formatting check failed".to_string(),
                            code: Some("rustfmt".to_string()),
                        });
                    }
                }

                ValidationStageResult {
                    stage: ValidationStage::Syntax,
                    passed,
                    errors,
                    warnings: Vec::new(),
                    duration_ms,
                }
            }
            Err(e) => ValidationStageResult {
                stage: ValidationStage::Syntax,
                passed: false,
                errors: vec![ValidationMessage {
                    file: None,
                    line: None,
                    column: None,
                    message: format!("Failed to run rustfmt: {}", e),
                    code: None,
                }],
                warnings: Vec::new(),
                duration_ms,
            },
        }
    }

    /// Run clippy
    fn run_clippy(&self) -> ValidationStageResult {
        let start = Instant::now();

        let output = Command::new("cargo")
            .args(["clippy", "--message-format=short", "--", "-D", "warnings"])
            .current_dir(&self.config.project_root)
            .output();

        let duration_ms = start.elapsed().as_millis() as u64;

        match output {
            Ok(output) => {
                let passed = output.status.success();
                let stderr = String::from_utf8_lossy(&output.stderr);
                let (errors, warnings) = parse_compiler_output(&stderr);

                ValidationStageResult {
                    stage: ValidationStage::Linting,
                    passed,
                    errors,
                    warnings,
                    duration_ms,
                }
            }
            Err(e) => ValidationStageResult {
                stage: ValidationStage::Linting,
                passed: false,
                errors: vec![ValidationMessage {
                    file: None,
                    line: None,
                    column: None,
                    message: format!("Failed to run clippy: {}", e),
                    code: None,
                }],
                warnings: Vec::new(),
                duration_ms,
            },
        }
    }

    /// Run cargo check
    fn run_cargo_check(&self) -> ValidationStageResult {
        let start = Instant::now();

        let output = Command::new("cargo")
            .args(["check", "--message-format=short"])
            .current_dir(&self.config.project_root)
            .output();

        let duration_ms = start.elapsed().as_millis() as u64;

        match output {
            Ok(output) => {
                let passed = output.status.success();
                let stderr = String::from_utf8_lossy(&output.stderr);
                let (errors, warnings) = parse_compiler_output(&stderr);

                ValidationStageResult {
                    stage: ValidationStage::TypeCheck,
                    passed,
                    errors,
                    warnings,
                    duration_ms,
                }
            }
            Err(e) => ValidationStageResult {
                stage: ValidationStage::TypeCheck,
                passed: false,
                errors: vec![ValidationMessage {
                    file: None,
                    line: None,
                    column: None,
                    message: format!("Failed to run cargo check: {}", e),
                    code: None,
                }],
                warnings: Vec::new(),
                duration_ms,
            },
        }
    }

    /// Run cargo test
    fn run_tests(&self) -> ValidationStageResult {
        let start = Instant::now();

        let mut args = vec!["test", "--lib"];

        if let Some(ref filter) = self.config.test_filter {
            args.push(filter);
        }

        let output = Command::new("cargo")
            .args(&args)
            .current_dir(&self.config.project_root)
            .output();

        let duration_ms = start.elapsed().as_millis() as u64;

        match output {
            Ok(output) => {
                let passed = output.status.success();
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                let mut errors = Vec::new();
                let mut warnings = Vec::new();

                // Parse test failures from stdout
                if !passed {
                    for line in stdout.lines() {
                        if line.contains("FAILED") && line.contains("::") {
                            // Extract test name from lines like "test module::test_name ... FAILED"
                            if let Some(test_name) = extract_test_name(line) {
                                errors.push(ValidationMessage {
                                    file: None,
                                    line: None,
                                    column: None,
                                    message: format!("Test failed: {}", test_name),
                                    code: Some("test_failure".to_string()),
                                });
                            }
                        }
                    }

                    // If no specific failures found, add generic error
                    if errors.is_empty() {
                        errors.push(ValidationMessage {
                            file: None,
                            line: None,
                            column: None,
                            message: "Test suite failed".to_string(),
                            code: Some("test_failure".to_string()),
                        });
                    }
                }

                // Check for compilation warnings in test build
                let (compile_errors, compile_warnings) = parse_compiler_output(&stderr);
                errors.extend(compile_errors);
                warnings.extend(compile_warnings);

                ValidationStageResult {
                    stage: ValidationStage::UnitTests,
                    passed,
                    errors,
                    warnings,
                    duration_ms,
                }
            }
            Err(e) => ValidationStageResult {
                stage: ValidationStage::UnitTests,
                passed: false,
                errors: vec![ValidationMessage {
                    file: None,
                    line: None,
                    column: None,
                    message: format!("Failed to run tests: {}", e),
                    code: None,
                }],
                warnings: Vec::new(),
                duration_ms,
            },
        }
    }
}

/// Parse compiler output (cargo check, clippy) into errors and warnings
fn parse_compiler_output(output: &str) -> (Vec<ValidationMessage>, Vec<ValidationMessage>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    for line in output.lines() {
        // Skip non-diagnostic lines
        if !line.contains("error") && !line.contains("warning") {
            continue;
        }

        // Parse lines like "src/main.rs:10:5: error[E0001]: message"
        // Or simpler: "error: message"
        if let Some(msg) = parse_diagnostic_line(line) {
            if line.contains("error") {
                errors.push(msg);
            } else if line.contains("warning") {
                warnings.push(msg);
            }
        }
    }

    (errors, warnings)
}

/// Parse a single diagnostic line
fn parse_diagnostic_line(line: &str) -> Option<ValidationMessage> {
    // Try to parse "file:line:col: level[code]: message" format
    let parts: Vec<&str> = line.splitn(4, ':').collect();

    if parts.len() >= 4 {
        let file = parts[0].trim().to_string();
        let line_num = parts[1].trim().parse::<u32>().ok();
        let col_num = parts[2].trim().parse::<u32>().ok();

        // Check if this looks like a file path
        if file.ends_with(".rs") || file.contains('/') {
            let rest = parts[3..].join(":");
            let (code, message) = extract_error_code(&rest);

            return Some(ValidationMessage {
                file: Some(file),
                line: line_num,
                column: col_num,
                message: message.trim().to_string(),
                code,
            });
        }
    }

    // Fallback: just extract the message
    if line.contains("error") || line.contains("warning") {
        let message = line
            .replace("error:", "")
            .replace("warning:", "")
            .replace("error[", "[")
            .trim()
            .to_string();

        if !message.is_empty() && message.len() > 3 {
            return Some(ValidationMessage {
                file: None,
                line: None,
                column: None,
                message,
                code: None,
            });
        }
    }

    None
}

/// Extract error code from message like "error[E0001]: message"
fn extract_error_code(text: &str) -> (Option<String>, String) {
    if let Some(start) = text.find('[') {
        if let Some(end) = text.find(']') {
            let code = text[start + 1..end].to_string();
            let message = text[end + 1..].trim_start_matches(':').trim().to_string();
            return (Some(code), message);
        }
    }
    (None, text.to_string())
}

/// Extract file path from rustfmt diff output
fn extract_file_from_diff(line: &str) -> Option<String> {
    // Lines like "Diff in /path/to/file.rs at line 10:"
    if line.starts_with("Diff in ") {
        let rest = line.trim_start_matches("Diff in ");
        if let Some(at_pos) = rest.find(" at ") {
            return Some(rest[..at_pos].to_string());
        }
    }
    None
}

/// Extract test name from test output line
fn extract_test_name(line: &str) -> Option<String> {
    // Lines like "test module::test_name ... FAILED"
    if line.starts_with("test ") && line.contains("FAILED") {
        let rest = line.trim_start_matches("test ");
        if let Some(dots_pos) = rest.find(" ...") {
            return Some(rest[..dots_pos].to_string());
        }
    }
    None
}

/// Check if a path exists
pub fn path_exists(path: &str) -> bool {
    Path::new(path).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_config_default() {
        let config = ValidationConfig::default();
        assert!(config.run_tests);
        assert!(config.run_clippy);
        assert!(config.run_rustfmt);
        assert_eq!(config.test_timeout_secs, 300);
    }

    #[test]
    fn test_parse_diagnostic_line() {
        let line = "src/main.rs:10:5: error[E0425]: cannot find value `x`";
        let msg = parse_diagnostic_line(line).unwrap();

        assert_eq!(msg.file, Some("src/main.rs".to_string()));
        assert_eq!(msg.line, Some(10));
        assert_eq!(msg.column, Some(5));
        assert_eq!(msg.code, Some("E0425".to_string()));
        assert!(msg.message.contains("cannot find value"));
    }

    #[test]
    fn test_parse_simple_error() {
        let line = "error: could not compile `myproject`";
        let msg = parse_diagnostic_line(line).unwrap();

        assert!(msg.file.is_none());
        assert!(msg.message.contains("could not compile"));
    }

    #[test]
    fn test_extract_error_code() {
        let (code, msg) = extract_error_code(" error[E0001]: some message");
        assert_eq!(code, Some("E0001".to_string()));
        assert_eq!(msg, "some message");
    }

    #[test]
    fn test_extract_test_name() {
        let line = "test ai::agent::tests::test_creation ... FAILED";
        let name = extract_test_name(line).unwrap();
        assert_eq!(name, "ai::agent::tests::test_creation");
    }

    #[test]
    fn test_parse_compiler_output() {
        let output = r#"
   Compiling myproject v0.1.0
warning: unused variable: `x`
 --> src/main.rs:5:9
  |
5 |     let x = 5;
  |         ^ help: if this is intentional, prefix it with an underscore: `_x`

error[E0425]: cannot find value `y` in this scope
 --> src/main.rs:10:5
  |
10 |     y
   |     ^ not found in this scope

error: could not compile `myproject`
"#;

        let (errors, warnings) = parse_compiler_output(output);

        // Should have found at least one error and one warning
        assert!(!errors.is_empty());
        assert!(!warnings.is_empty());
    }
}
