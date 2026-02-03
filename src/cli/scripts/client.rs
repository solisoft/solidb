//! HTTP client for interacting with the SoliDB script API
//!
//! Provides methods to list, create, update, and delete scripts.

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

/// Script summary returned by the list endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptSummary {
    pub id: String,
    pub name: String,
    pub path: String,
    pub methods: Vec<String>,
    pub description: Option<String>,
    pub database: String,
    pub collection: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Full script data including code
#[derive(Debug, Clone, Deserialize)]
pub struct Script {
    #[serde(rename = "_key")]
    pub key: String,
    pub name: String,
    pub path: String,
    pub methods: Vec<String>,
    pub code: String,
    pub description: Option<String>,
    pub database: String,
    pub collection: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Request body for creating a script
#[derive(Debug, Serialize)]
pub struct CreateScriptRequest {
    pub name: String,
    pub path: String,
    pub methods: Vec<String>,
    pub code: String,
    pub description: Option<String>,
    pub collection: Option<String>,
}

/// Response from list scripts endpoint
#[derive(Debug, Deserialize)]
pub struct ListScriptsResponse {
    pub scripts: Vec<ScriptSummary>,
}

/// Response from create script endpoint
#[derive(Debug, Deserialize)]
pub struct CreateScriptResponse {
    pub id: String,
    pub name: String,
    pub path: String,
    pub methods: Vec<String>,
    pub created_at: String,
}

/// HTTP client for the SoliDB script API
pub struct ScriptClient {
    client: Client,
    base_url: String,
    auth_token: Option<String>,
}

impl ScriptClient {
    /// Create a new script client
    pub fn new(base_url: &str, auth_token: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            auth_token,
        }
    }

    /// Add auth header if token is set
    fn auth_header(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        if let Some(token) = &self.auth_token {
            if !token.is_empty() {
                return request.bearer_auth(token);
            }
        }
        request
    }

    /// List all scripts for a database
    pub fn list_scripts(&self, database: &str) -> anyhow::Result<Vec<ScriptSummary>> {
        let url = format!("{}/_api/database/{}/scripts", self.base_url, database);
        let request = self.client.get(&url);
        let response = self.auth_header(request).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            anyhow::bail!("Failed to list scripts: {} - {}", status, body);
        }

        let result: ListScriptsResponse = response.json()?;
        Ok(result.scripts)
    }

    /// Get a single script by ID
    pub fn get_script(&self, database: &str, script_id: &str) -> anyhow::Result<Script> {
        let url = format!(
            "{}/_api/database/{}/scripts/{}",
            self.base_url, database, script_id
        );
        let request = self.client.get(&url);
        let response = self.auth_header(request).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            anyhow::bail!("Failed to get script: {} - {}", status, body);
        }

        let script: Script = response.json()?;
        Ok(script)
    }

    /// Find a script by its API path
    pub fn find_script_by_path(
        &self,
        database: &str,
        api_path: &str,
    ) -> anyhow::Result<Option<Script>> {
        let scripts = self.list_scripts(database)?;

        // Normalize path for comparison (remove leading slash if present)
        let normalized_path = api_path.trim_start_matches('/');

        for summary in scripts {
            let script_path = summary.path.trim_start_matches('/');
            if script_path == normalized_path {
                // Found it, now get the full script with code
                return Ok(Some(self.get_script(database, &summary.id)?));
            }
        }

        Ok(None)
    }

    /// Create a new script
    pub fn create_script(
        &self,
        database: &str,
        path: &str,
        methods: &[String],
        code: &str,
        description: Option<&str>,
        collection: Option<&str>,
    ) -> anyhow::Result<CreateScriptResponse> {
        let url = format!("{}/_api/database/{}/scripts", self.base_url, database);

        // Generate name from path
        let name = super::mapper::ScriptMeta::name_from_path(path);

        let request_body = CreateScriptRequest {
            name,
            path: path.trim_start_matches('/').to_string(),
            methods: methods.to_vec(),
            code: code.to_string(),
            description: description.map(|s| s.to_string()),
            collection: collection.map(|s| s.to_string()),
        };

        let request = self.client.post(&url).json(&request_body);
        let response = self.auth_header(request).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            anyhow::bail!("Failed to create script: {} - {}", status, body);
        }

        let result: CreateScriptResponse = response.json()?;
        Ok(result)
    }

    /// Update an existing script
    #[allow(clippy::too_many_arguments)]
    pub fn update_script(
        &self,
        database: &str,
        script_id: &str,
        path: &str,
        methods: &[String],
        code: &str,
        description: Option<&str>,
        collection: Option<&str>,
    ) -> anyhow::Result<Script> {
        let url = format!(
            "{}/_api/database/{}/scripts/{}",
            self.base_url, database, script_id
        );

        // Generate name from path
        let name = super::mapper::ScriptMeta::name_from_path(path);

        let request_body = CreateScriptRequest {
            name,
            path: path.trim_start_matches('/').to_string(),
            methods: methods.to_vec(),
            code: code.to_string(),
            description: description.map(|s| s.to_string()),
            collection: collection.map(|s| s.to_string()),
        };

        let request = self.client.put(&url).json(&request_body);
        let response = self.auth_header(request).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            anyhow::bail!("Failed to update script: {} - {}", status, body);
        }

        let result: Script = response.json()?;
        Ok(result)
    }

    /// Delete a script
    pub fn delete_script(&self, database: &str, script_id: &str) -> anyhow::Result<()> {
        let url = format!(
            "{}/_api/database/{}/scripts/{}",
            self.base_url, database, script_id
        );

        let request = self.client.delete(&url);
        let response = self.auth_header(request).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            anyhow::bail!("Failed to delete script: {} - {}", status, body);
        }

        Ok(())
    }

    /// Delete a script by its API path
    pub fn delete_script_by_path(&self, database: &str, api_path: &str) -> anyhow::Result<bool> {
        if let Some(script) = self.find_script_by_path(database, api_path)? {
            self.delete_script(database, &script.key)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Test connection to the server
    pub fn test_connection(&self) -> anyhow::Result<()> {
        let url = format!("{}/_api/health", self.base_url);
        let response = self.client.get(&url).send()?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to connect to server: {} {}",
                response.status(),
                response.text().unwrap_or_default()
            );
        }

        Ok(())
    }

    /// Login to the server and get an authentication token
    pub fn login(&self, username: &str, password: &str) -> anyhow::Result<String> {
        let url = format!("{}/auth/login", self.base_url);

        #[derive(Serialize)]
        struct LoginRequest {
            username: String,
            password: String,
        }

        #[derive(Deserialize)]
        struct LoginResponse {
            token: String,
        }

        let request_body = LoginRequest {
            username: username.to_string(),
            password: password.to_string(),
        };

        let response = self.client.post(&url).json(&request_body).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            if status.as_u16() == 400 {
                anyhow::bail!("Invalid credentials");
            }
            anyhow::bail!("Login failed: {} - {}", status, body);
        }

        let result: LoginResponse = response.json()?;
        Ok(result.token)
    }
}
