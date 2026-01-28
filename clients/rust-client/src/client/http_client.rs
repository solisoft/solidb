use super::DriverError;
use reqwest;
use serde_json::Value;
use std::time::Duration;

pub struct HttpClient {
    base_url: String,
    database: Option<String>,
    token: Option<String>,
    client: reqwest::Client,
}

impl HttpClient {
    pub fn new(base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(16)
            .build()
            .unwrap();

        Self {
            base_url: base_url.to_string().trim_end_matches('/').to_string(),
            database: None,
            token: None,
            client,
        }
    }

    pub fn with_database(mut self, database: &str) -> Self {
        self.database = Some(database.to_string());
        self
    }

    pub fn set_database(&mut self, database: &str) {
        self.database = Some(database.to_string());
    }

    pub async fn login(&mut self, database: &str, username: &str, password: &str) -> Result<(), DriverError> {
        let response = self
            .client
            .post(&format!("{}/auth/login", self.base_url))
            .json(&serde_json::json!({
                "database": database,
                "username": username,
                "password": password
            }))
            .send()
            .await
            .map_err(|e| DriverError::ConnectionError(format!("HTTP request failed: {}", e)))?;

        if response.status() == 401 {
            return Err(DriverError::AuthError("Invalid credentials".to_string()));
        }

        let data: Value = response
            .json()
            .await
            .map_err(|e| DriverError::ProtocolError(format!("Failed to parse login response: {}", e)))?;

        if let Some(token) = data.get("token").and_then(|t| t.as_str()) {
            self.token = Some(token.to_string());
            self.database = Some(database.to_string());
            Ok(())
        } else {
            Err(DriverError::AuthError("No token in response".to_string()))
        }
    }

    pub fn set_token(&mut self, token: &str) {
        self.token = Some(token.to_string());
    }

    fn get_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::CONTENT_TYPE, "application/json".parse().unwrap());
        if let Some(token) = &self.token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", token).parse().unwrap(),
            );
        }
        headers
    }

    async fn request<T>(&self, method: &str, path: &str, body: Option<&Value>) -> Result<T, DriverError>
    where
        T: serde::de::DeserializeOwned,
    {
        let url = format!("{}{}", self.base_url, path);
        let mut request = self.client.request(method.parse().unwrap(), &url);
        request = request.headers(self.get_headers());

        if let Some(b) = body {
            request = request.json(b);
        }

        let response = request
            .send()
            .await
            .map_err(|e| DriverError::ConnectionError(format!("HTTP request failed: {}", e)))?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(DriverError::ServerError(format!("HTTP {} {}: {}", status, path, error_text)));
        }

        let text = response
            .text()
            .await
            .map_err(|e| DriverError::ProtocolError(format!("Failed to read response: {}", e)))?;

        if text.is_empty() {
            return Err(DriverError::ServerError(format!("Empty response for HTTP {} {}", method, path)));
        }

        serde_json::from_str(&text)
            .map_err(|e| DriverError::ProtocolError(format!("Failed to parse response: {} - Text: {}", e, text)))
    }

    pub async fn list_databases(&self) -> Result<Vec<String>, DriverError> {
        let response: Value = self.request("GET", "/_api/databases", None).await?;
        Ok(response.get("databases")
            .and_then(|d| d.as_array())
            .map(|arr| arr.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default())
    }

    pub async fn create_database(&self, name: &str) -> Result<(), DriverError> {
        self.request::<Value>("POST", "/_api/databases", Some(&serde_json::json!({"name": name}))).await?;
        Ok(())
    }

    pub async fn list_collections(&self, database: Option<&str>) -> Result<Vec<Value>, DriverError> {
        let db = database.or(self.database.as_deref()).ok_or_else(|| DriverError::ProtocolError("No database specified".to_string()))?;
        let response: Value = self.request("GET", &format!("/_api/database/{}/collection", db), None).await?;
        Ok(response.get("collections")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default())
    }

    pub async fn create_collection(&self, name: &str) -> Result<(), DriverError> {
        let db = self.database.as_deref().ok_or_else(|| DriverError::ProtocolError("No database specified".to_string()))?;
        self.request::<Value>("POST", &format!("/_api/database/{}/collection", db), Some(&serde_json::json!({"name": name}))).await?;
        Ok(())
    }

    pub async fn insert(&self, collection: &str, document: Value, key: Option<&str>) -> Result<Value, DriverError> {
        let db = self.database.as_deref().ok_or_else(|| DriverError::ProtocolError("No database specified".to_string()))?;
        let mut doc = document;
        if let Some(k) = key {
            if let Some(obj) = doc.as_object_mut() {
                obj.insert("_key".to_string(), serde_json::json!(k));
            }
        }
        let path = format!("/_api/database/{}/document/{}", db, collection);
        self.client
            .post(&format!("{}{}", self.base_url, path))
            .headers(self.get_headers())
            .json(&doc)
            .send()
            .await
            .map_err(|e| DriverError::ConnectionError(format!("HTTP request failed: {}", e)))?
            .json()
            .await
            .map_err(|e| DriverError::ProtocolError(format!("Failed to parse response: {}", e)))
    }

    pub async fn get(&self, collection: &str, key: &str) -> Result<Option<Value>, DriverError> {
        let db = self.database.as_deref().ok_or_else(|| DriverError::ProtocolError("No database specified".to_string()))?;
        let path = format!("/_api/database/{}/document/{}/{}", db, collection, key);
        let response: Value = self
            .client
            .get(&format!("{}{}", self.base_url, path))
            .headers(self.get_headers())
            .send()
            .await
            .map_err(|e| DriverError::ConnectionError(format!("HTTP request failed: {}", e)))?
            .json()
            .await
            .map_err(|e| DriverError::ProtocolError(format!("Failed to parse response: {}", e)))?;
        Ok(Some(response))
    }

    pub async fn update(&self, collection: &str, key: &str, document: Value, merge: bool) -> Result<(), DriverError> {
        let db = self.database.as_deref().ok_or_else(|| DriverError::ProtocolError("No database specified".to_string()))?;
        let payload = serde_json::json!({
            "document": document,
            "merge": merge
        });
        let path = format!("/_api/database/{}/document/{}/{}", db, collection, key);
        self.request::<Value>("PUT", &path, Some(&payload)).await?;
        Ok(())
    }

    pub async fn delete(&self, collection: &str, key: &str) -> Result<(), DriverError> {
        let db = self.database.as_deref().ok_or_else(|| DriverError::ProtocolError("No database specified".to_string()))?;
        let path = format!("/_api/database/{}/collection/{}/document/{}", db, collection, key);
        self.request::<Value>("DELETE", &path, None).await?;
        Ok(())
    }

    pub async fn list(&self, collection: &str, limit: i32, offset: i32) -> Result<Vec<Value>, DriverError> {
        let db = self.database.as_deref().ok_or_else(|| DriverError::ProtocolError("No database specified".to_string()))?;
        let path = format!("/_api/database/{}/collection/{}/documents?limit={}&offset={}", db, collection, limit, offset);
        let response: Value = self.request("GET", &path, None).await?;
        Ok(response.get("documents")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default())
    }

    pub async fn query(&self, sdbql: &str, bind_vars: Option<Value>) -> Result<Vec<Value>, DriverError> {
        let db = self.database.as_deref().ok_or_else(|| DriverError::ProtocolError("No database specified".to_string()))?;
        let mut payload = serde_json::json!({
            "database": db,
            "sdbql": sdbql
        });
        if let Some(bv) = bind_vars {
            payload["bind_vars"] = bv;
        }
        let response: Value = self.request("POST", "/_api/query", Some(&payload)).await?;
        Ok(response.get("result")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default())
    }

    pub async fn begin_transaction(&self, isolation_level: Option<&str>) -> Result<String, DriverError> {
        let db = self.database.as_deref().ok_or_else(|| DriverError::ProtocolError("No database specified".to_string()))?;
        let mut payload = serde_json::json!({
            "database": db
        });
        if let Some(il) = isolation_level {
            payload["isolation_level"] = serde_json::json!(il);
        }
        let response: Value = self.request("POST", "/_api/transaction/begin", Some(&payload)).await?;
        response.get("tx_id")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| DriverError::ProtocolError("No tx_id in response".to_string()))
    }

    pub async fn commit_transaction(&self, tx_id: &str) -> Result<(), DriverError> {
        self.request::<Value>("POST", "/_api/transaction/commit", Some(&serde_json::json!({"tx_id": tx_id}))).await?;
        Ok(())
    }

    pub async fn rollback_transaction(&self, tx_id: &str) -> Result<(), DriverError> {
        self.request::<Value>("POST", "/_api/transaction/rollback", Some(&serde_json::json!({"tx_id": tx_id}))).await?;
        Ok(())
    }

    pub async fn cluster_status(&self) -> Result<Value, DriverError> {
        self.request("GET", "/_api/cluster/status", None).await
    }

    pub async fn cluster_info(&self) -> Result<Value, DriverError> {
        self.request("GET", "/_api/cluster/info", None).await
    }

    pub async fn ping(&self) -> Result<bool, DriverError> {
        let response = self.request::<Value>("GET", "/health", None).await;
        Ok(response.is_ok())
    }
}
