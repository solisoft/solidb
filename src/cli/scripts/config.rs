//! Configuration handling for the scripts CLI
//!
//! Manages the solidb-scripts.toml configuration file.
//!
//! ## Environment Variables
//!
//! The following environment variables can override config file settings:
//!
//! - `SOLIDB_API_KEY` - API key for authentication (takes precedence over login token)
//! - `SOLIDB_HOST` - Server host
//! - `SOLIDB_PORT` - Server port
//! - `SOLIDB_DATABASE` - Target database
//!
//! These can be set in a `.env` file in the scripts directory.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Configuration file name
pub const CONFIG_FILE_NAME: &str = "solidb-scripts.toml";

/// Environment variable names
pub const ENV_API_KEY: &str = "SOLIDB_API_KEY";
pub const ENV_HOST: &str = "SOLIDB_HOST";
pub const ENV_PORT: &str = "SOLIDB_PORT";
pub const ENV_DATABASE: &str = "SOLIDB_DATABASE";

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Server host
    pub host: String,
    /// Server port
    pub port: u16,
    /// Target database
    pub database: String,
    /// Optional authentication token
    #[serde(default)]
    pub auth_token: String,
    /// Scripts configuration
    #[serde(default)]
    pub scripts: ScriptsConfig,
}

/// Scripts-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptsConfig {
    /// Directory containing scripts (relative to config file)
    #[serde(default = "default_directory")]
    pub directory: PathBuf,
    /// Patterns to ignore
    #[serde(default = "default_ignore")]
    pub ignore: Vec<String>,
}

fn default_directory() -> PathBuf {
    PathBuf::from(".")
}

fn default_ignore() -> Vec<String> {
    vec![
        "*.bak".to_string(),
        ".git".to_string(),
        "node_modules".to_string(),
    ]
}

impl Default for ScriptsConfig {
    fn default() -> Self {
        Self {
            directory: default_directory(),
            ignore: default_ignore(),
        }
    }
}

impl Config {
    /// Create a new configuration with the given settings
    pub fn new(host: String, port: u16, database: String) -> Self {
        Self {
            host,
            port,
            database,
            auth_token: String::new(),
            scripts: ScriptsConfig::default(),
        }
    }

    /// Load configuration from a directory
    ///
    /// This also loads any `.env` file in the directory and applies
    /// environment variable overrides.
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        // Load .env file if present (ignore errors)
        let env_path = dir.join(".env");
        if env_path.exists() {
            let _ = dotenvy::from_path(&env_path);
        }

        let config_path = dir.join(CONFIG_FILE_NAME);
        if !config_path.exists() {
            anyhow::bail!(
                "Configuration file not found: {}\nRun 'solidb scripts init' to create one.",
                config_path.display()
            );
        }

        let content = std::fs::read_to_string(&config_path)?;
        let mut config: Config = toml::from_str(&content)?;

        // Apply environment variable overrides
        config.apply_env_overrides();

        Ok(config)
    }

    /// Apply environment variable overrides to the configuration
    fn apply_env_overrides(&mut self) {
        // SOLIDB_API_KEY takes precedence over saved auth_token
        if let Ok(api_key) = std::env::var(ENV_API_KEY) {
            if !api_key.is_empty() {
                self.auth_token = api_key;
            }
        }

        // Override host if set
        if let Ok(host) = std::env::var(ENV_HOST) {
            if !host.is_empty() {
                self.host = host;
            }
        }

        // Override port if set
        if let Ok(port_str) = std::env::var(ENV_PORT) {
            if let Ok(port) = port_str.parse::<u16>() {
                self.port = port;
            }
        }

        // Override database if set
        if let Ok(database) = std::env::var(ENV_DATABASE) {
            if !database.is_empty() {
                self.database = database;
            }
        }
    }

    /// Check if authentication is configured (either via token or env var)
    pub fn has_auth(&self) -> bool {
        !self.auth_token.is_empty()
    }

    /// Save configuration to a directory
    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        let config_path = dir.join(CONFIG_FILE_NAME);
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;
        Ok(())
    }

    /// Get the base URL for API requests
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    /// Get the absolute scripts directory path
    pub fn scripts_dir(&self, config_dir: &Path) -> PathBuf {
        if self.scripts.directory.is_absolute() {
            self.scripts.directory.clone()
        } else {
            config_dir.join(&self.scripts.directory)
        }
    }

    /// Check if a path should be ignored
    pub fn should_ignore(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        for pattern in &self.scripts.ignore {
            // Simple glob matching for common patterns
            if pattern.starts_with("*.") {
                // Extension pattern like "*.bak"
                if let Some(ext) = pattern.strip_prefix("*.") {
                    if path_str.ends_with(&format!(".{}", ext)) {
                        return true;
                    }
                }
            } else if path_str.contains(pattern) {
                // Directory name pattern like ".git" or "node_modules"
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_serialization() {
        let config = Config::new("localhost".to_string(), 6745, "mydb".to_string());
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("host = \"localhost\""));
        assert!(toml_str.contains("port = 6745"));
        assert!(toml_str.contains("database = \"mydb\""));
    }

    #[test]
    fn test_should_ignore() {
        let config = Config::new("localhost".to_string(), 6745, "test".to_string());

        assert!(config.should_ignore(Path::new("test.bak")));
        assert!(config.should_ignore(Path::new(".git/config")));
        assert!(config.should_ignore(Path::new("node_modules/package.json")));
        assert!(!config.should_ignore(Path::new("users.lua")));
    }
}
