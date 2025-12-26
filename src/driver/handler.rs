//! Connection handler for native driver protocol
//!
//! Processes incoming commands and executes them against the storage engine.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::storage::StorageEngine;
use crate::transaction::{TransactionId, IsolationLevel as TxIsolationLevel};
use crate::sdbql::QueryExecutor;

use super::protocol::{Command, Response, DriverError, MAX_MESSAGE_SIZE, encode_response, decode_message};

/// Handler for a single driver connection
pub struct DriverHandler {
    storage: Arc<StorageEngine>,
    /// Active transactions for this connection
    transactions: HashMap<String, TransactionId>,
    /// Authenticated database (None = not authenticated)
    authenticated_db: Option<String>,
}

impl DriverHandler {
    /// Create a new handler
    pub fn new(storage: Arc<StorageEngine>) -> Self {
        Self {
            storage,
            transactions: HashMap::new(),
            authenticated_db: None,
        }
    }

    /// Handle a driver connection
    pub async fn handle_connection(&mut self, mut stream: TcpStream, addr: String) {
        tracing::info!("Driver connection from {}", addr);

        // The magic header has already been consumed by the multiplexer
        // Start processing commands immediately

        loop {
            // Read message length (4 bytes, big-endian)
            let mut len_buf = [0u8; 4];
            match stream.read_exact(&mut len_buf).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    tracing::debug!("Driver connection closed: {}", addr);
                    break;
                }
                Err(e) => {
                    tracing::warn!("Driver read error from {}: {}", addr, e);
                    break;
                }
            }

            let msg_len = u32::from_be_bytes(len_buf) as usize;

            // Validate message size
            if msg_len > MAX_MESSAGE_SIZE {
                let resp = Response::error(DriverError::MessageTooLarge);
                if let Err(e) = self.send_response(&mut stream, &resp).await {
                    tracing::warn!("Failed to send error response: {}", e);
                }
                break;
            }

            // Read message payload
            let mut payload = vec![0u8; msg_len];
            if let Err(e) = stream.read_exact(&mut payload).await {
                tracing::warn!("Driver read payload error from {}: {}", addr, e);
                break;
            }

            // Decode command
            let command: Command = match decode_message(&payload) {
                Ok(cmd) => cmd,
                Err(e) => {
                    let resp = Response::error(e);
                    if let Err(e) = self.send_response(&mut stream, &resp).await {
                        tracing::warn!("Failed to send error response: {}", e);
                    }
                    continue;
                }
            };

            // Execute command
            let response = self.execute_command(command).await;

            // Send response
            if let Err(e) = self.send_response(&mut stream, &response).await {
                tracing::warn!("Failed to send response to {}: {}", addr, e);
                break;
            }
        }

        // Cleanup: rollback any uncommitted transactions
        for (tx_id_str, tx_id) in self.transactions.drain() {
            tracing::debug!("Rolling back uncommitted transaction: {}", tx_id_str);
            let _ = self.storage.rollback_transaction(tx_id);
        }
    }

    /// Send a response to the client
    async fn send_response(&self, stream: &mut TcpStream, response: &Response) -> Result<(), DriverError> {
        let data = encode_response(response)?;
        stream.write_all(&data).await
            .map_err(|e| DriverError::ConnectionError(e.to_string()))?;
        stream.flush().await
            .map_err(|e| DriverError::ConnectionError(e.to_string()))?;
        Ok(())
    }

    /// Execute a command and return a response
    async fn execute_command(&mut self, command: Command) -> Response {
        match command {
            // ==================== Auth & Utility ====================
            Command::Ping => Response::pong(),

            Command::Auth { database, username, password } => {
                self.handle_auth(database, username, password).await
            }

            // ==================== Database Operations ====================
            Command::ListDatabases => {
                let dbs = self.storage.list_databases();
                Response::ok(serde_json::json!(dbs))
            }

            Command::CreateDatabase { name } => {
                match self.storage.create_database(name) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }

            Command::DeleteDatabase { name } => {
                match self.storage.delete_database(&name) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }

            // ==================== Collection Operations ====================
            Command::ListCollections { database } => {
                match self.storage.get_database(&database) {
                    Ok(db) => {
                        let collections = db.list_collections();
                        Response::ok(serde_json::json!(collections))
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }

            Command::CreateCollection { database, name, collection_type } => {
                match self.storage.get_database(&database) {
                    Ok(db) => {
                        match db.create_collection(name, collection_type) {
                            Ok(_) => Response::ok_empty(),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }

            Command::DeleteCollection { database, name } => {
                match self.storage.get_database(&database) {
                    Ok(db) => {
                        match db.delete_collection(&name) {
                            Ok(_) => Response::ok_empty(),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }

            Command::CollectionStats { database, name } => {
                match self.storage.get_database(&database) {
                    Ok(db) => {
                        match db.get_collection(&name) {
                            Ok(coll) => {
                                let stats = coll.stats();
                                Response::ok(serde_json::to_value(stats).unwrap_or_default())
                            }
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }

            // ==================== Document Operations ====================
            Command::Get { database, collection, key } => {
                match self.get_collection(&database, &collection) {
                    Ok(coll) => {
                        match coll.get(&key) {
                            Ok(doc) => Response::ok(doc.to_value()),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(e),
                }
            }

            Command::Insert { database, collection, key, document } => {
                match self.get_collection(&database, &collection) {
                    Ok(coll) => {
                        // If key provided, add it to document; otherwise insert() will auto-generate
                        let mut doc_data = document;
                        if let Some(k) = key {
                            if let Some(obj) = doc_data.as_object_mut() {
                                obj.insert("_key".to_string(), serde_json::json!(k));
                            }
                        }
                        match coll.insert(doc_data) {
                            Ok(doc) => Response::ok(doc.to_value()),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(e),
                }
            }

            Command::Update { database, collection, key, document, merge } => {
                match self.get_collection(&database, &collection) {
                    Ok(coll) => {
                        let result = if merge {
                            // Merge update: get existing doc and merge
                            match coll.get(&key) {
                                Ok(existing) => {
                                    let mut merged = existing.data.clone();
                                    if let (Some(base), Some(updates)) = (merged.as_object_mut(), document.as_object()) {
                                        for (k, v) in updates {
                                            base.insert(k.clone(), v.clone());
                                        }
                                    }
                                    coll.update(&key, merged)
                                }
                                Err(e) => Err(e),
                            }
                        } else {
                            coll.update(&key, document)
                        };

                        match result {
                            Ok(doc) => Response::ok(doc.to_value()),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(e),
                }
            }

            Command::Delete { database, collection, key } => {
                match self.get_collection(&database, &collection) {
                    Ok(coll) => {
                        match coll.delete(&key) {
                            Ok(_) => Response::ok_empty(),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(e),
                }
            }

            Command::List { database, collection, limit, offset } => {
                match self.get_collection(&database, &collection) {
                    Ok(coll) => {
                        // Use scan() which is the correct method for listing documents
                        let all_docs = coll.scan(None);
                        let total = all_docs.len();
                        
                        // Apply pagination
                        let offset = offset.unwrap_or(0);
                        let limit = limit.unwrap_or(100);
                        let docs: Vec<_> = all_docs.into_iter()
                            .skip(offset)
                            .take(limit)
                            .map(|d| d.to_value())
                            .collect();

                        Response::Ok {
                            data: Some(serde_json::json!(docs)),
                            count: Some(total),
                            tx_id: None,
                        }
                    }
                    Err(e) => Response::error(e),
                }
            }

            // ==================== Query Operations ====================
            Command::Query { database, sdbql, bind_vars } => {
                // Parse the SDBQL query first
                match crate::sdbql::parse(&sdbql) {
                    Ok(query) => {
                        // Create executor with database and bind vars
                        let executor = if bind_vars.is_empty() {
                            QueryExecutor::with_database(&self.storage, database)
                        } else {
                            QueryExecutor::with_database_and_bind_vars(&self.storage, database, bind_vars)
                        };
                        
                        match executor.execute(&query) {
                            Ok(results) => Response::ok(serde_json::json!(results)),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(format!("Parse error: {}", e))),
                }
            }

            Command::Explain { database, sdbql, bind_vars } => {
                // Parse the SDBQL query first
                match crate::sdbql::parse(&sdbql) {
                    Ok(query) => {
                        let executor = if bind_vars.is_empty() {
                            QueryExecutor::with_database(&self.storage, database)
                        } else {
                            QueryExecutor::with_database_and_bind_vars(&self.storage, database, bind_vars)
                        };
                        
                        match executor.explain(&query) {
                            Ok(explanation) => Response::ok(serde_json::to_value(explanation).unwrap_or_default()),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(format!("Parse error: {}", e))),
                }
            }

            // ==================== Index Operations ====================
            Command::CreateIndex { database, collection, name, fields, unique, sparse: _ } => {
                match self.get_collection(&database, &collection) {
                    Ok(coll) => {
                        // Default to Persistent index type
                        let index_type = crate::storage::IndexType::Persistent;
                        match coll.create_index(name, fields, index_type, unique) {
                            Ok(stats) => Response::ok(serde_json::to_value(stats).unwrap_or_default()),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(e),
                }
            }

            Command::DeleteIndex { database, collection, name } => {
                match self.get_collection(&database, &collection) {
                    Ok(coll) => {
                        // Try dropping as standard index first
                        if coll.drop_index(&name).is_ok() {
                            return Response::ok_empty();
                        }
                        // Try dropping as fulltext index
                        if coll.drop_fulltext_index(&name).is_ok() {
                            return Response::ok_empty();
                        }
                        // Try dropping as geo index
                        if coll.drop_geo_index(&name).is_ok() {
                            return Response::ok_empty();
                        }
                        // Try dropping as TTL index
                        if coll.drop_ttl_index(&name).is_ok() {
                            return Response::ok_empty();
                        }
                        Response::error(DriverError::DatabaseError(format!("Index '{}' not found", name)))
                    }
                    Err(e) => Response::error(e),
                }
            }

            Command::ListIndexes { database, collection } => {
                match self.get_collection(&database, &collection) {
                    Ok(coll) => {
                        let indexes = coll.list_indexes();
                        Response::ok(serde_json::to_value(indexes).unwrap_or_default())
                    }
                    Err(e) => Response::error(e),
                }
            }

            // ==================== Transaction Operations ====================
            Command::BeginTransaction { database, isolation_level } => {
                match self.storage.get_database(&database) {
                    Ok(_) => {
                        let tx_isolation: TxIsolationLevel = isolation_level.into();
                        match self.storage.transaction_manager() {
                            Ok(tx_manager) => {
                                match tx_manager.begin(tx_isolation) {
                                    Ok(tx_id) => {
                                        let tx_id_str = tx_id.to_string();
                                        self.transactions.insert(tx_id_str.clone(), tx_id);
                                        Response::ok_tx(tx_id_str)
                                    }
                                    Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
                                }
                            }
                            Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }

            Command::CommitTransaction { tx_id } => {
                match self.transactions.remove(&tx_id) {
                    Some(tx) => {
                        match self.storage.commit_transaction(tx) {
                            Ok(_) => Response::ok_empty(),
                            Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
                        }
                    }
                    None => Response::error(DriverError::TransactionError("Transaction not found".to_string())),
                }
            }

            Command::RollbackTransaction { tx_id } => {
                match self.transactions.remove(&tx_id) {
                    Some(tx) => {
                        match self.storage.rollback_transaction(tx) {
                            Ok(_) => Response::ok_empty(),
                            Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
                        }
                    }
                    None => Response::error(DriverError::TransactionError("Transaction not found".to_string())),
                }
            }

            Command::TransactionCommand { tx_id, command } => {
                // Verify transaction exists
                if !self.transactions.contains_key(&tx_id) {
                    return Response::error(DriverError::TransactionError("Transaction not found".to_string()));
                }
                // Execute the inner command (transaction context not yet implemented for all commands)
                // For now, just execute the command normally
                // TODO: Implement proper transaction context
                Box::pin(self.execute_command(*command)).await
            }

            // ==================== Bulk Operations ====================
            Command::Batch { commands } => {
                let mut responses = Vec::with_capacity(commands.len());
                for cmd in commands {
                    let resp = Box::pin(self.execute_command(cmd)).await;
                    responses.push(resp);
                }
                Response::Batch { responses }
            }

            Command::BulkInsert { database, collection, documents } => {
                match self.get_collection(&database, &collection) {
                    Ok(coll) => {
                        // Use batch insert for efficiency
                        match coll.insert_batch(documents) {
                            Ok(docs) => Response::ok_count(docs.len()),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(e),
                }
            }
        }
    }

    /// Helper to get a collection
    fn get_collection(&self, database: &str, collection: &str) -> Result<crate::storage::Collection, DriverError> {
        let db = self.storage.get_database(database)
            .map_err(|e| DriverError::DatabaseError(e.to_string()))?;
        db.get_collection(collection)
            .map_err(|e| DriverError::DatabaseError(e.to_string()))
    }

    /// Handle authentication
    async fn handle_auth(&mut self, database: String, username: String, password: String) -> Response {
        // Get the _system database for auth lookup
        let system_db = match self.storage.get_database("_system") {
            Ok(db) => db,
            Err(e) => return Response::error(DriverError::AuthError(format!("System database error: {}", e))),
        };

        // Get admins collection (username is the _key)
        let admins = match system_db.get_collection("_admins") {
            Ok(coll) => coll,
            Err(_) => return Response::error(DriverError::AuthError("Admins collection not found".to_string())),
        };

        // Find user by username (username IS the _key in _admins collection)
        let user_doc = match admins.get(&username) {
            Ok(doc) => doc,
            Err(_) => return Response::error(DriverError::AuthError("Invalid credentials".to_string())),
        };

        // Parse user
        let user: crate::server::auth::User = match serde_json::from_value(user_doc.to_value()) {
            Ok(u) => u,
            Err(_) => return Response::error(DriverError::AuthError("Invalid credentials".to_string())),
        };

        // Verify password using AuthService
        if !crate::server::auth::AuthService::verify_password(&password, &user.password_hash) {
            return Response::error(DriverError::AuthError("Invalid credentials".to_string()));
        }

        // Verify requested database exists
        if let Err(e) = self.storage.get_database(&database) {
            return Response::error(DriverError::DatabaseError(format!("Database not found: {}", e)));
        }

        // Set authenticated state (admin users have access to all databases)
        self.authenticated_db = Some(database);
        Response::ok_empty()
    }
}

/// Spawn a handler for incoming driver connections
pub fn spawn_driver_handler(
    storage: Arc<StorageEngine>,
) -> tokio::sync::mpsc::Sender<(TcpStream, String)> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(TcpStream, String)>(100);

    tokio::spawn(async move {
        while let Some((stream, addr)) = rx.recv().await {
            let storage = storage.clone();
            tokio::spawn(async move {
                let mut handler = DriverHandler::new(storage);
                handler.handle_connection(stream, addr).await;
            });
        }
    });

    tx
}
