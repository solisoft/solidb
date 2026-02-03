//! HTTP client wrapper for TUI API calls

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// TUI HTTP client for SoliDB API
pub struct TuiClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

/// Database info from API
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseInfo {
    pub name: String,
}

/// Collection info from API
#[derive(Debug, Clone, Deserialize)]
pub struct CollectionInfo {
    pub name: String,
    #[serde(default)]
    pub count: u64,
    #[serde(rename = "type", default)]
    pub collection_type: String,
}

/// Document from API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    #[serde(rename = "_key")]
    pub key: String,
    #[serde(flatten)]
    pub data: serde_json::Value,
}

/// Index info from API
#[derive(Debug, Clone, Deserialize)]
pub struct IndexInfo {
    pub name: String,
    pub index_type: String,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub sparse: bool,
}

/// Queue stats from API
#[derive(Debug, Clone, Deserialize)]
pub struct QueueStats {
    pub name: String,
    pub pending: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub total: usize,
}

/// Job info from API
#[derive(Debug, Clone, Deserialize)]
pub struct JobInfo {
    #[serde(rename = "_key")]
    pub id: String,
    pub queue: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub script_path: String,
    pub status: serde_json::Value,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub max_retries: i32,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub started_at: Option<u64>,
    #[serde(default)]
    pub completed_at: Option<u64>,
}

/// Cluster node info
#[derive(Debug, Clone, Deserialize)]
pub struct ClusterNode {
    pub id: String,
    pub address: String,
    #[serde(default)]
    pub api_address: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub last_heartbeat: Option<u64>,
}

/// Cluster status response
#[derive(Debug, Clone, Deserialize)]
pub struct ClusterStatus {
    pub node_id: String,
    #[serde(default)]
    pub nodes: Vec<ClusterNode>,
    #[serde(default)]
    pub is_cluster_mode: bool,
}

/// Query result
#[derive(Debug, Clone, Deserialize)]
pub struct QueryResult {
    #[serde(default)]
    pub result: Vec<serde_json::Value>,
    #[serde(default)]
    pub count: Option<u64>,
    #[serde(default)]
    pub error: bool,
    #[serde(default, alias = "errorMessage")]
    pub error_message: Option<String>,
}

/// Document list response
#[derive(Debug, Clone)]
pub struct DocumentListResponse {
    pub documents: Vec<serde_json::Value>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

impl TuiClient {
    /// Create a new TUI client
    pub fn new(base_url: &str, api_key: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        }
    }

    /// Build request with auth headers
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::blocking::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.request(method, &url);

        if let Some(ref key) = self.api_key {
            // API keys start with "sk_", JWT tokens don't
            if key.starts_with("sk_") {
                req = req.header("X-API-Key", key);
            } else {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
        }

        req
    }

    /// Test connection to server
    pub fn test_connection(&self) -> Result<bool, String> {
        match self.request(reqwest::Method::GET, "/_api/databases").send() {
            Ok(resp) => {
                if resp.status().is_success() {
                    Ok(true)
                } else if resp.status().as_u16() == 401 {
                    // Unauthorized but server is reachable
                    Ok(true)
                } else {
                    Err(format!("Server returned status: {}", resp.status()))
                }
            }
            Err(e) => Err(format!("Connection failed: {}", e)),
        }
    }

    /// List all databases
    pub fn list_databases(&self) -> Result<Vec<DatabaseInfo>, String> {
        let resp = self
            .request(reqwest::Method::GET, "/_api/databases")
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Server returned status: {}", resp.status()));
        }

        #[derive(Deserialize)]
        struct Response {
            databases: Vec<String>,
        }

        let text = resp.text().map_err(|e| format!("Read failed: {}", e))?;
        let data: Response = serde_json::from_str(&text)
            .map_err(|e| format!("JSON parse failed: {} (response: {})", e, &text[..text.len().min(200)]))?;

        Ok(data
            .databases
            .into_iter()
            .map(|name| DatabaseInfo { name })
            .collect())
    }

    /// List collections in a database
    pub fn list_collections(&self, database: &str) -> Result<Vec<CollectionInfo>, String> {
        let path = format!("/_api/database/{}/collection", database);
        let resp = self
            .request(reqwest::Method::GET, &path)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Server returned status: {}", resp.status()));
        }

        #[derive(Deserialize)]
        struct Response {
            collections: Vec<CollectionInfo>,
        }

        let data: Response = resp
            .json()
            .map_err(|e| format!("JSON parse failed: {}", e))?;
        Ok(data.collections)
    }

    /// Get collection count
    pub fn get_collection_count(&self, database: &str, collection: &str) -> Result<u64, String> {
        let path = format!(
            "/_api/database/{}/collection/{}/count",
            database, collection
        );
        let resp = self
            .request(reqwest::Method::GET, &path)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Server returned status: {}", resp.status()));
        }

        #[derive(Deserialize)]
        struct Response {
            count: u64,
        }

        let data: Response = resp
            .json()
            .map_err(|e| format!("JSON parse failed: {}", e))?;
        Ok(data.count)
    }

    /// List documents in a collection with pagination (uses query)
    pub fn list_documents(
        &self,
        database: &str,
        collection: &str,
        offset: u64,
        limit: u64,
    ) -> Result<DocumentListResponse, String> {
        // First get total count
        let total = self.get_collection_count(database, collection).unwrap_or(0);

        // Use SDBQL query to get documents with pagination
        let query = format!(
            "FOR doc IN {} LIMIT {}, {} RETURN doc",
            collection, offset, limit
        );

        let result = self.execute_query(database, &query)?;

        Ok(DocumentListResponse {
            documents: result.result,
            total,
            offset,
            limit,
        })
    }

    /// Get a single document
    pub fn get_document(
        &self,
        database: &str,
        collection: &str,
        key: &str,
    ) -> Result<serde_json::Value, String> {
        let path = format!(
            "/_api/database/{}/document/{}/{}",
            database, collection, key
        );
        let resp = self
            .request(reqwest::Method::GET, &path)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Server returned status: {}", resp.status()));
        }

        let data: serde_json::Value = resp
            .json()
            .map_err(|e| format!("JSON parse failed: {}", e))?;
        Ok(data)
    }

    /// Execute an SDBQL query
    pub fn execute_query(&self, database: &str, query: &str) -> Result<QueryResult, String> {
        let path = format!("/_api/database/{}/cursor", database);

        let mut body = HashMap::new();
        body.insert("query", query);

        let resp = self
            .request(reqwest::Method::POST, &path)
            .json(&body)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("Query failed ({}): {}", status, text));
        }

        let data: QueryResult = resp
            .json()
            .map_err(|e| format!("JSON parse failed: {}", e))?;

        if data.error {
            return Err(data
                .error_message
                .unwrap_or_else(|| "Unknown error".to_string()));
        }

        Ok(data)
    }

    /// List indexes for a collection
    pub fn list_indexes(&self, database: &str, collection: &str) -> Result<Vec<IndexInfo>, String> {
        let path = format!("/_api/database/{}/index/{}", database, collection);
        let resp = self
            .request(reqwest::Method::GET, &path)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Server returned status: {}", resp.status()));
        }

        #[derive(Deserialize)]
        struct Response {
            indexes: Vec<IndexInfo>,
        }

        let text = resp.text().map_err(|e| format!("Read failed: {}", e))?;
        let data: Response = serde_json::from_str(&text)
            .map_err(|e| format!("JSON parse failed: {} (response: {})", e, &text[..text.len().min(300)]))?;
        Ok(data.indexes)
    }

    /// Get cluster status
    pub fn get_cluster_status(&self) -> Result<ClusterStatus, String> {
        let resp = self
            .request(reqwest::Method::GET, "/_api/cluster/status")
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Server returned status: {}", resp.status()));
        }

        let data: ClusterStatus = resp
            .json()
            .map_err(|e| format!("JSON parse failed: {}", e))?;
        Ok(data)
    }

    /// Get cluster info
    pub fn get_cluster_info(&self) -> Result<serde_json::Value, String> {
        let resp = self
            .request(reqwest::Method::GET, "/_api/cluster/info")
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Server returned status: {}", resp.status()));
        }

        let data: serde_json::Value = resp
            .json()
            .map_err(|e| format!("JSON parse failed: {}", e))?;
        Ok(data)
    }

    /// Create a new document
    pub fn create_document(
        &self,
        database: &str,
        collection: &str,
        document: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let path = format!("/_api/database/{}/document/{}", database, collection);
        let resp = self
            .request(reqwest::Method::POST, &path)
            .json(document)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("Create failed ({}): {}", status, text));
        }

        let data: serde_json::Value = resp
            .json()
            .map_err(|e| format!("JSON parse failed: {}", e))?;
        Ok(data)
    }

    /// Update a document
    pub fn update_document(
        &self,
        database: &str,
        collection: &str,
        key: &str,
        document: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let path = format!(
            "/_api/database/{}/document/{}/{}",
            database, collection, key
        );
        let resp = self
            .request(reqwest::Method::PATCH, &path)
            .json(document)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("Update failed ({}): {}", status, text));
        }

        let data: serde_json::Value = resp
            .json()
            .map_err(|e| format!("JSON parse failed: {}", e))?;
        Ok(data)
    }

    /// Delete a document
    pub fn delete_document(
        &self,
        database: &str,
        collection: &str,
        key: &str,
    ) -> Result<(), String> {
        let path = format!(
            "/_api/database/{}/document/{}/{}",
            database, collection, key
        );
        let resp = self
            .request(reqwest::Method::DELETE, &path)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("Delete failed ({}): {}", status, text));
        }

        Ok(())
    }

    /// Create a new collection
    pub fn create_collection(&self, database: &str, name: &str) -> Result<(), String> {
        let path = format!("/_api/database/{}/collection", database);

        let mut body = HashMap::new();
        body.insert("name", name);

        let resp = self
            .request(reqwest::Method::POST, &path)
            .json(&body)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("Create collection failed ({}): {}", status, text));
        }

        Ok(())
    }

    /// Delete a collection
    pub fn delete_collection(&self, database: &str, name: &str) -> Result<(), String> {
        let path = format!("/_api/database/{}/collection/{}", database, name);
        let resp = self
            .request(reqwest::Method::DELETE, &path)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("Delete collection failed ({}): {}", status, text));
        }

        Ok(())
    }

    /// Truncate a collection
    pub fn truncate_collection(&self, database: &str, name: &str) -> Result<(), String> {
        let path = format!("/_api/database/{}/collection/{}/truncate", database, name);
        let resp = self
            .request(reqwest::Method::PUT, &path)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("Truncate collection failed ({}): {}", status, text));
        }

        Ok(())
    }

    /// Create a new database
    pub fn create_database(&self, name: &str) -> Result<(), String> {
        let path = "/_api/database";

        let mut body = HashMap::new();
        body.insert("name", name);

        let resp = self
            .request(reqwest::Method::POST, path)
            .json(&body)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("Create database failed ({}): {}", status, text));
        }

        Ok(())
    }

    /// Delete a database
    pub fn delete_database(&self, name: &str) -> Result<(), String> {
        let path = format!("/_api/database/{}", name);
        let resp = self
            .request(reqwest::Method::DELETE, &path)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("Delete database failed ({}): {}", status, text));
        }

        Ok(())
    }

    /// List queues with stats
    pub fn list_queues(&self, database: &str) -> Result<Vec<QueueStats>, String> {
        let path = format!("/_api/database/{}/queues", database);
        let resp = self
            .request(reqwest::Method::GET, &path)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Server returned status: {}", resp.status()));
        }

        let text = resp.text().map_err(|e| format!("Read failed: {}", e))?;
        let data: Vec<QueueStats> = serde_json::from_str(&text)
            .map_err(|e| format!("JSON parse failed: {} (response: {})", e, &text[..text.len().min(200)]))?;
        Ok(data)
    }

    /// List jobs in a queue
    pub fn list_jobs(&self, database: &str, queue: &str, limit: usize) -> Result<Vec<JobInfo>, String> {
        let path = format!("/_api/database/{}/queues/{}/jobs?limit={}", database, queue, limit);
        let resp = self
            .request(reqwest::Method::GET, &path)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Server returned status: {}", resp.status()));
        }

        #[derive(Deserialize)]
        struct Response {
            jobs: Vec<JobInfo>,
        }

        let text = resp.text().map_err(|e| format!("Read failed: {}", e))?;
        let data: Response = serde_json::from_str(&text)
            .map_err(|e| format!("JSON parse failed: {} (response: {})", e, &text[..text.len().min(300)]))?;
        Ok(data.jobs)
    }
}
