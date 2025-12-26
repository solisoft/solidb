//! SoliDB native driver client library
//!
//! Provides a high-performance client for connecting to SoliDB using the
//! native binary protocol instead of HTTP REST.
//!
//! # Example
//!
//! ```rust,no_run
//! use solidb::driver::SoliDBClient;
//! use serde_json::json;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Connect to the server
//!     let mut client = SoliDBClient::connect("localhost:6745").await?;
//!
//!     // Authenticate (optional, depending on server config)
//!     client.auth("mydb", "admin", "password").await?;
//!
//!     // Insert a document
//!     let doc = client.insert("mydb", "users", None, json!({
//!         "name": "Alice",
//!         "email": "alice@example.com"
//!     })).await?;
//!
//!     println!("Inserted: {:?}", doc);
//!
//!     // Query using SDBQL
//!     let results = client.query("mydb", "FOR u IN users RETURN u", None).await?;
//!     println!("Query results: {:?}", results);
//!
//!     Ok(())
//! }
//! ```

use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use serde_json::Value;

use super::protocol::{Command, Response, DriverError, IsolationLevel, DRIVER_MAGIC, encode_command, decode_message, MAX_MESSAGE_SIZE};

/// SoliDB native driver client
pub struct SoliDBClient {
    stream: TcpStream,
    /// Current transaction ID (if any)
    current_tx: Option<String>,
}

impl SoliDBClient {
    /// Connect to a SoliDB server
    ///
    /// # Arguments
    /// * `addr` - Server address (e.g., "localhost:6745" or "192.168.1.100:6745")
    ///
    /// # Example
    /// ```rust,no_run
    /// use solidb::driver::SoliDBClient;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let client = SoliDBClient::connect("localhost:6745").await.unwrap();
    /// }
    /// ```
    pub async fn connect(addr: &str) -> Result<Self, DriverError> {
        let stream = TcpStream::connect(addr).await
            .map_err(|e| DriverError::ConnectionError(format!("Failed to connect to {}: {}", addr, e)))?;

        let mut client = Self {
            stream,
            current_tx: None,
        };

        // Send magic header
        client.stream.write_all(DRIVER_MAGIC).await
            .map_err(|e| DriverError::ConnectionError(format!("Failed to send magic header: {}", e)))?;
        client.stream.flush().await
            .map_err(|e| DriverError::ConnectionError(format!("Failed to flush: {}", e)))?;

        Ok(client)
    }

    /// Send a command and receive the response
    async fn send_command(&mut self, command: Command) -> Result<Response, DriverError> {
        // Encode and send
        let data = encode_command(&command)?;
        self.stream.write_all(&data).await
            .map_err(|e| DriverError::ConnectionError(format!("Write failed: {}", e)))?;
        self.stream.flush().await
            .map_err(|e| DriverError::ConnectionError(format!("Flush failed: {}", e)))?;

        // Read response length
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await
            .map_err(|e| DriverError::ConnectionError(format!("Read length failed: {}", e)))?;

        let msg_len = u32::from_be_bytes(len_buf) as usize;
        if msg_len > MAX_MESSAGE_SIZE {
            return Err(DriverError::MessageTooLarge);
        }

        // Read response payload
        let mut payload = vec![0u8; msg_len];
        self.stream.read_exact(&mut payload).await
            .map_err(|e| DriverError::ConnectionError(format!("Read payload failed: {}", e)))?;

        decode_message(&payload)
    }

    /// Extract data from a response, returning an error if the response is an error
    fn extract_data(response: Response) -> Result<Option<Value>, DriverError> {
        match response {
            Response::Ok { data, .. } => Ok(data),
            Response::Error { error } => Err(error),
            Response::Pong { .. } => Ok(None),
            Response::Batch { .. } => Ok(None),
        }
    }

    /// Extract the transaction ID from a response
    fn extract_tx_id(response: Response) -> Result<String, DriverError> {
        match response {
            Response::Ok { tx_id: Some(id), .. } => Ok(id),
            Response::Ok { .. } => Err(DriverError::ProtocolError("Expected transaction ID".to_string())),
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError("Unexpected response type".to_string())),
        }
    }

    // ==================== Utility Methods ====================

    /// Ping the server
    pub async fn ping(&mut self) -> Result<i64, DriverError> {
        let response = self.send_command(Command::Ping).await?;
        match response {
            Response::Pong { timestamp } => Ok(timestamp),
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError("Expected pong response".to_string())),
        }
    }

    /// Authenticate with the server
    pub async fn auth(&mut self, database: &str, username: &str, password: &str) -> Result<(), DriverError> {
        let response = self.send_command(Command::Auth {
            database: database.to_string(),
            username: username.to_string(),
            password: password.to_string(),
        }).await?;

        match response {
            Response::Ok { .. } => Ok(()),
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError("Unexpected response".to_string())),
        }
    }

    // ==================== Database Operations ====================

    /// List all databases
    pub async fn list_databases(&mut self) -> Result<Vec<String>, DriverError> {
        let response = self.send_command(Command::ListDatabases).await?;
        let data = Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))?;
        
        serde_json::from_value(data)
            .map_err(|e| DriverError::ProtocolError(format!("Invalid response: {}", e)))
    }

    /// Create a new database
    pub async fn create_database(&mut self, name: &str) -> Result<(), DriverError> {
        let response = self.send_command(Command::CreateDatabase {
            name: name.to_string(),
        }).await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// Delete a database
    pub async fn delete_database(&mut self, name: &str) -> Result<(), DriverError> {
        let response = self.send_command(Command::DeleteDatabase {
            name: name.to_string(),
        }).await?;
        Self::extract_data(response)?;
        Ok(())
    }

    // ==================== Collection Operations ====================

    /// List collections in a database
    pub async fn list_collections(&mut self, database: &str) -> Result<Vec<String>, DriverError> {
        let response = self.send_command(Command::ListCollections {
            database: database.to_string(),
        }).await?;
        let data = Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))?;
        
        serde_json::from_value(data)
            .map_err(|e| DriverError::ProtocolError(format!("Invalid response: {}", e)))
    }

    /// Create a new collection
    pub async fn create_collection(&mut self, database: &str, name: &str, collection_type: Option<&str>) -> Result<(), DriverError> {
        let response = self.send_command(Command::CreateCollection {
            database: database.to_string(),
            name: name.to_string(),
            collection_type: collection_type.map(|s| s.to_string()),
        }).await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// Delete a collection
    pub async fn delete_collection(&mut self, database: &str, name: &str) -> Result<(), DriverError> {
        let response = self.send_command(Command::DeleteCollection {
            database: database.to_string(),
            name: name.to_string(),
        }).await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// Get collection statistics
    pub async fn collection_stats(&mut self, database: &str, collection: &str) -> Result<Value, DriverError> {
        let response = self.send_command(Command::CollectionStats {
            database: database.to_string(),
            name: collection.to_string(),
        }).await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    // ==================== Document Operations ====================

    /// Get a document by key
    pub async fn get(&mut self, database: &str, collection: &str, key: &str) -> Result<Value, DriverError> {
        let response = self.send_command(Command::Get {
            database: database.to_string(),
            collection: collection.to_string(),
            key: key.to_string(),
        }).await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// Insert a new document
    ///
    /// Returns the inserted document with `_key` field set.
    pub async fn insert(&mut self, database: &str, collection: &str, key: Option<&str>, document: Value) -> Result<Value, DriverError> {
        let response = self.send_command(Command::Insert {
            database: database.to_string(),
            collection: collection.to_string(),
            key: key.map(|s| s.to_string()),
            document,
        }).await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// Update an existing document
    ///
    /// If `merge` is true, the document will be merged with the existing one.
    /// Otherwise, the document will be replaced entirely.
    pub async fn update(&mut self, database: &str, collection: &str, key: &str, document: Value, merge: bool) -> Result<Value, DriverError> {
        let response = self.send_command(Command::Update {
            database: database.to_string(),
            collection: collection.to_string(),
            key: key.to_string(),
            document,
            merge,
        }).await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// Delete a document
    pub async fn delete(&mut self, database: &str, collection: &str, key: &str) -> Result<(), DriverError> {
        let response = self.send_command(Command::Delete {
            database: database.to_string(),
            collection: collection.to_string(),
            key: key.to_string(),
        }).await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// List documents in a collection with pagination
    pub async fn list(&mut self, database: &str, collection: &str, limit: Option<usize>, offset: Option<usize>) -> Result<(Vec<Value>, usize), DriverError> {
        let response = self.send_command(Command::List {
            database: database.to_string(),
            collection: collection.to_string(),
            limit,
            offset,
        }).await?;

        match response {
            Response::Ok { data, count, .. } => {
                let docs: Vec<Value> = data
                    .and_then(|d| serde_json::from_value(d).ok())
                    .unwrap_or_default();
                let len = docs.len();
                Ok((docs, count.unwrap_or(len)))
            }
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError("Unexpected response".to_string())),
        }
    }

    // ==================== Query Operations ====================

    /// Execute an SDBQL query
    pub async fn query(&mut self, database: &str, sdbql: &str, bind_vars: Option<HashMap<String, Value>>) -> Result<Vec<Value>, DriverError> {
        let response = self.send_command(Command::Query {
            database: database.to_string(),
            sdbql: sdbql.to_string(),
            bind_vars: bind_vars.unwrap_or_default(),
        }).await?;
        
        let data = Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))?;
        
        serde_json::from_value(data)
            .map_err(|e| DriverError::ProtocolError(format!("Invalid response: {}", e)))
    }

    /// Explain an SDBQL query without executing it
    pub async fn explain(&mut self, database: &str, sdbql: &str, bind_vars: Option<HashMap<String, Value>>) -> Result<Value, DriverError> {
        let response = self.send_command(Command::Explain {
            database: database.to_string(),
            sdbql: sdbql.to_string(),
            bind_vars: bind_vars.unwrap_or_default(),
        }).await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    // ==================== Index Operations ====================

    /// Create an index on a collection
    pub async fn create_index(&mut self, database: &str, collection: &str, name: &str, fields: Vec<String>, unique: bool, sparse: bool) -> Result<(), DriverError> {
        let response = self.send_command(Command::CreateIndex {
            database: database.to_string(),
            collection: collection.to_string(),
            name: name.to_string(),
            fields,
            unique,
            sparse,
        }).await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// Delete an index
    pub async fn delete_index(&mut self, database: &str, collection: &str, name: &str) -> Result<(), DriverError> {
        let response = self.send_command(Command::DeleteIndex {
            database: database.to_string(),
            collection: collection.to_string(),
            name: name.to_string(),
        }).await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// List indexes on a collection
    pub async fn list_indexes(&mut self, database: &str, collection: &str) -> Result<Vec<Value>, DriverError> {
        let response = self.send_command(Command::ListIndexes {
            database: database.to_string(),
            collection: collection.to_string(),
        }).await?;
        let data = Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))?;
        
        serde_json::from_value(data)
            .map_err(|e| DriverError::ProtocolError(format!("Invalid response: {}", e)))
    }

    // ==================== Transaction Operations ====================

    /// Begin a new transaction
    pub async fn begin_transaction(&mut self, database: &str, isolation_level: Option<IsolationLevel>) -> Result<String, DriverError> {
        let response = self.send_command(Command::BeginTransaction {
            database: database.to_string(),
            isolation_level: isolation_level.unwrap_or_default(),
        }).await?;
        
        let tx_id = Self::extract_tx_id(response)?;
        self.current_tx = Some(tx_id.clone());
        Ok(tx_id)
    }

    /// Commit the current transaction
    pub async fn commit(&mut self) -> Result<(), DriverError> {
        let tx_id = self.current_tx.take()
            .ok_or_else(|| DriverError::TransactionError("No active transaction".to_string()))?;
        
        let response = self.send_command(Command::CommitTransaction { tx_id }).await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// Rollback the current transaction
    pub async fn rollback(&mut self) -> Result<(), DriverError> {
        let tx_id = self.current_tx.take()
            .ok_or_else(|| DriverError::TransactionError("No active transaction".to_string()))?;
        
        let response = self.send_command(Command::RollbackTransaction { tx_id }).await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// Check if there's an active transaction
    pub fn in_transaction(&self) -> bool {
        self.current_tx.is_some()
    }

    /// Get the current transaction ID
    pub fn transaction_id(&self) -> Option<&str> {
        self.current_tx.as_deref()
    }

    // ==================== Bulk Operations ====================

    /// Execute multiple commands in a batch
    pub async fn batch(&mut self, commands: Vec<Command>) -> Result<Vec<Response>, DriverError> {
        let response = self.send_command(Command::Batch { commands }).await?;
        match response {
            Response::Batch { responses } => Ok(responses),
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError("Expected batch response".to_string())),
        }
    }

    /// Bulk insert documents
    pub async fn bulk_insert(&mut self, database: &str, collection: &str, documents: Vec<Value>) -> Result<usize, DriverError> {
        let response = self.send_command(Command::BulkInsert {
            database: database.to_string(),
            collection: collection.to_string(),
            documents,
        }).await?;

        match response {
            Response::Ok { count, .. } => Ok(count.unwrap_or(0)),
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError("Unexpected response".to_string())),
        }
    }
}

/// Builder for creating a SoliDBClient with additional options
pub struct SoliDBClientBuilder {
    addr: String,
    auth: Option<(String, String, String)>, // (database, username, password)
    timeout_ms: Option<u64>,
}

impl SoliDBClientBuilder {
    /// Create a new builder
    pub fn new(addr: &str) -> Self {
        Self {
            addr: addr.to_string(),
            auth: None,
            timeout_ms: None,
        }
    }

    /// Set authentication credentials
    pub fn auth(mut self, database: &str, username: &str, password: &str) -> Self {
        self.auth = Some((database.to_string(), username.to_string(), password.to_string()));
        self
    }

    /// Set connection timeout in milliseconds
    pub fn timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// Build the client
    pub async fn build(self) -> Result<SoliDBClient, DriverError> {
        let mut client = SoliDBClient::connect(&self.addr).await?;

        if let Some((database, username, password)) = self.auth {
            client.auth(&database, &username, &password).await?;
        }

        Ok(client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a running SoliDB server
    // Run with: cargo test --features integration-tests

    #[tokio::test]
    #[ignore]
    async fn test_connect_and_ping() {
        let mut client = SoliDBClient::connect("localhost:6745").await.unwrap();
        let timestamp = client.ping().await.unwrap();
        assert!(timestamp > 0);
    }

    #[tokio::test]
    #[ignore]
    async fn test_crud_operations() {
        let mut client = SoliDBClient::connect("localhost:6745").await.unwrap();

        // Create database and collection
        let _ = client.create_database("test_driver").await;
        let _ = client.create_collection("test_driver", "users", None).await;

        // Insert
        let doc = client.insert("test_driver", "users", Some("user1"), serde_json::json!({
            "name": "Alice",
            "age": 30
        })).await.unwrap();
        assert_eq!(doc["name"], "Alice");

        // Get
        let doc = client.get("test_driver", "users", "user1").await.unwrap();
        assert_eq!(doc["name"], "Alice");

        // Update
        let doc = client.update("test_driver", "users", "user1", serde_json::json!({
            "age": 31
        }), true).await.unwrap();
        assert_eq!(doc["age"], 31);

        // Delete
        client.delete("test_driver", "users", "user1").await.unwrap();

        // Cleanup
        let _ = client.delete_database("test_driver").await;
    }
}
