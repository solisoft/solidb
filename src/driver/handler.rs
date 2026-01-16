//! Connection handler for native driver protocol
//!
//! Processes incoming commands and executes them against the storage engine.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::sdbql::QueryExecutor;
use crate::storage::{CollectionSchema, StorageEngine, VectorIndexConfig, VectorMetric};
use crate::transaction::{IsolationLevel as TxIsolationLevel, TransactionId};

use super::protocol::{
    decode_message, encode_response, Command, DriverError, Response, MAX_MESSAGE_SIZE,
};

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
    async fn send_response(
        &self,
        stream: &mut TcpStream,
        response: &Response,
    ) -> Result<(), DriverError> {
        let data = encode_response(response)?;
        stream
            .write_all(&data)
            .await
            .map_err(|e| DriverError::ConnectionError(e.to_string()))?;
        stream
            .flush()
            .await
            .map_err(|e| DriverError::ConnectionError(e.to_string()))?;
        Ok(())
    }

    /// Execute a command and return a response
    async fn execute_command(&mut self, command: Command) -> Response {
        match command {
            // ==================== Auth & Utility ====================
            Command::Ping => Response::pong(),

            Command::Auth {
                database,
                username,
                password,
            } => self.handle_auth(database, username, password).await,

            // ==================== Database Operations ====================
            Command::ListDatabases => {
                let dbs = self.storage.list_databases();
                Response::ok(serde_json::json!(dbs))
            }

            Command::CreateDatabase { name } => match self.storage.create_database(name) {
                Ok(_) => Response::ok_empty(),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::DeleteDatabase { name } => match self.storage.delete_database(&name) {
                Ok(_) => Response::ok_empty(),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            // ==================== Collection Operations ====================
            Command::ListCollections { database } => match self.storage.get_database(&database) {
                Ok(db) => {
                    let collections = db.list_collections();
                    Response::ok(serde_json::json!(collections))
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::CreateCollection {
                database,
                name,
                collection_type,
            } => match self.storage.get_database(&database) {
                Ok(db) => match db.create_collection(name, collection_type) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::DeleteCollection { database, name } => {
                match self.storage.get_database(&database) {
                    Ok(db) => match db.delete_collection(&name) {
                        Ok(_) => Response::ok_empty(),
                        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                    },
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }

            Command::CollectionStats { database, name } => {
                match self.storage.get_database(&database) {
                    Ok(db) => match db.get_collection(&name) {
                        Ok(coll) => {
                            let stats = coll.stats();
                            Response::ok(serde_json::to_value(stats).unwrap_or_default())
                        }
                        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                    },
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }

            // ==================== Document Operations ====================
            Command::Get {
                database,
                collection,
                key,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.get(&key) {
                    Ok(doc) => Response::ok(doc.to_value()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            Command::Insert {
                database,
                collection,
                key,
                document,
            } => {
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

            Command::Update {
                database,
                collection,
                key,
                document,
                merge,
            } => {
                match self.get_collection(&database, &collection) {
                    Ok(coll) => {
                        let result = if merge {
                            // Merge update: get existing doc and merge
                            match coll.get(&key) {
                                Ok(existing) => {
                                    let mut merged = existing.data.clone();
                                    if let (Some(base), Some(updates)) =
                                        (merged.as_object_mut(), document.as_object())
                                    {
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

            Command::Delete {
                database,
                collection,
                key,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.delete(&key) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            Command::List {
                database,
                collection,
                limit,
                offset,
            } => {
                match self.get_collection(&database, &collection) {
                    Ok(coll) => {
                        // Use scan() which is the correct method for listing documents
                        let all_docs = coll.scan(None);
                        let total = all_docs.len();

                        // Apply pagination
                        let offset = offset.unwrap_or(0);
                        let limit = limit.unwrap_or(100);
                        let docs: Vec<_> = all_docs
                            .into_iter()
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
            Command::Query {
                database,
                sdbql,
                bind_vars,
            } => {
                // Parse the SDBQL query first
                match crate::sdbql::parse(&sdbql) {
                    Ok(query) => {
                        // Create executor with database and bind vars
                        let executor = if bind_vars.is_empty() {
                            QueryExecutor::with_database(&self.storage, database)
                        } else {
                            QueryExecutor::with_database_and_bind_vars(
                                &self.storage,
                                database,
                                bind_vars,
                            )
                        };

                        match executor.execute(&query) {
                            Ok(results) => Response::ok(serde_json::json!(results)),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => {
                        Response::error(DriverError::DatabaseError(format!("Parse error: {}", e)))
                    }
                }
            }

            Command::Explain {
                database,
                sdbql,
                bind_vars,
            } => {
                // Parse the SDBQL query first
                match crate::sdbql::parse(&sdbql) {
                    Ok(query) => {
                        let executor = if bind_vars.is_empty() {
                            QueryExecutor::with_database(&self.storage, database)
                        } else {
                            QueryExecutor::with_database_and_bind_vars(
                                &self.storage,
                                database,
                                bind_vars,
                            )
                        };

                        match executor.explain(&query) {
                            Ok(explanation) => {
                                Response::ok(serde_json::to_value(explanation).unwrap_or_default())
                            }
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => {
                        Response::error(DriverError::DatabaseError(format!("Parse error: {}", e)))
                    }
                }
            }

            // ==================== Index Operations ====================
            Command::CreateIndex {
                database,
                collection,
                name,
                fields,
                unique,
                sparse: _,
            } => {
                match self.get_collection(&database, &collection) {
                    Ok(coll) => {
                        // Default to Persistent index type
                        let index_type = crate::storage::IndexType::Persistent;
                        match coll.create_index(name, fields, index_type, unique) {
                            Ok(stats) => {
                                Response::ok(serde_json::to_value(stats).unwrap_or_default())
                            }
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(e),
                }
            }

            Command::DeleteIndex {
                database,
                collection,
                name,
            } => {
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
                        Response::error(DriverError::DatabaseError(format!(
                            "Index '{}' not found",
                            name
                        )))
                    }
                    Err(e) => Response::error(e),
                }
            }

            Command::ListIndexes {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => {
                    let indexes = coll.list_indexes();
                    Response::ok(serde_json::to_value(indexes).unwrap_or_default())
                }
                Err(e) => Response::error(e),
            },

            // ==================== Transaction Operations ====================
            Command::BeginTransaction {
                database,
                isolation_level,
            } => match self.storage.get_database(&database) {
                Ok(_) => {
                    let tx_isolation: TxIsolationLevel = isolation_level.into();
                    match self.storage.transaction_manager() {
                        Ok(tx_manager) => match tx_manager.begin(tx_isolation) {
                            Ok(tx_id) => {
                                let tx_id_str = tx_id.to_string();
                                self.transactions.insert(tx_id_str.clone(), tx_id);
                                Response::ok_tx(tx_id_str)
                            }
                            Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
                        },
                        Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
                    }
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::CommitTransaction { tx_id } => match self.transactions.remove(&tx_id) {
                Some(tx) => match self.storage.commit_transaction(tx) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
                },
                None => Response::error(DriverError::TransactionError(
                    "Transaction not found".to_string(),
                )),
            },

            Command::RollbackTransaction { tx_id } => match self.transactions.remove(&tx_id) {
                Some(tx) => match self.storage.rollback_transaction(tx) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
                },
                None => Response::error(DriverError::TransactionError(
                    "Transaction not found".to_string(),
                )),
            },

            Command::TransactionCommand { tx_id, command } => {
                // Verify transaction exists
                if !self.transactions.contains_key(&tx_id) {
                    return Response::error(DriverError::TransactionError(
                        "Transaction not found".to_string(),
                    ));
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

            Command::BulkInsert {
                database,
                collection,
                documents,
            } => {
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

            // ==================== Script Management ====================
            Command::CreateScript {
                database,
                name,
                path,
                methods,
                code,
                description,
                collection,
            } => {
                self.handle_script_create(database, name, path, methods, code, description, collection)
                    .await
            }

            Command::ListScripts { database } => self.handle_script_list(database).await,

            Command::GetScript {
                database,
                script_id,
            } => self.handle_script_get(database, script_id).await,

            Command::UpdateScript {
                database,
                script_id,
                name,
                path,
                methods,
                code,
                description,
            } => {
                self.handle_script_update(database, script_id, name, path, methods, code, description)
                    .await
            }

            Command::DeleteScript {
                database,
                script_id,
            } => self.handle_script_delete(database, script_id).await,

            Command::GetScriptStats => {
                Response::ok(serde_json::json!({"message": "Script stats available via HTTP API"}))
            }

            // ==================== Job/Queue Management ====================
            Command::ListQueues { database } => self.handle_list_queues(database).await,

            Command::ListJobs {
                database,
                queue_name,
                status,
                limit,
                offset,
            } => self.handle_list_jobs(database, queue_name, status, limit, offset).await,

            Command::EnqueueJob {
                database,
                queue_name,
                script_path,
                params,
                priority,
                run_at,
                max_retries,
            } => {
                self.handle_enqueue_job(
                    database,
                    queue_name,
                    script_path,
                    params,
                    priority,
                    run_at,
                    max_retries,
                )
                .await
            }

            Command::CancelJob { database, job_id } => {
                self.handle_cancel_job(database, job_id).await
            }

            // ==================== Cron Job Management ====================
            Command::ListCronJobs { database } => self.handle_list_cron_jobs(database).await,

            Command::CreateCronJob {
                database,
                name,
                cron_expression,
                script_path,
                params,
                queue,
                priority,
                max_retries,
            } => {
                self.handle_create_cron_job(
                    database,
                    name,
                    cron_expression,
                    script_path,
                    params,
                    queue,
                    priority,
                    max_retries,
                )
                .await
            }

            Command::UpdateCronJob {
                database,
                cron_id,
                name,
                cron_expression,
                script_path,
                params,
                queue,
                priority,
                max_retries,
            } => {
                self.handle_update_cron_job(
                    database,
                    cron_id,
                    name,
                    cron_expression,
                    script_path,
                    params,
                    queue,
                    priority,
                    max_retries,
                )
                .await
            }

            Command::DeleteCronJob { database, cron_id } => {
                self.handle_delete_cron_job(database, cron_id).await
            }

            // ==================== Trigger Management ====================
            Command::ListTriggers { database } => self.handle_list_triggers(database).await,

            Command::ListCollectionTriggers {
                database,
                collection,
            } => self.handle_list_collection_triggers(database, collection).await,

            Command::CreateTrigger {
                database,
                name,
                collection,
                events,
                script_path,
                filter,
                queue,
                priority,
                max_retries,
                enabled,
            } => {
                self.handle_create_trigger(
                    database,
                    name,
                    collection,
                    events,
                    script_path,
                    filter,
                    queue,
                    priority,
                    max_retries,
                    enabled,
                )
                .await
            }

            Command::GetTrigger {
                database,
                trigger_id,
            } => self.handle_get_trigger(database, trigger_id).await,

            Command::UpdateTrigger {
                database,
                trigger_id,
                name,
                events,
                script_path,
                filter,
                queue,
                priority,
                max_retries,
                enabled,
            } => {
                self.handle_update_trigger(
                    database,
                    trigger_id,
                    name,
                    events,
                    script_path,
                    filter,
                    queue,
                    priority,
                    max_retries,
                    enabled,
                )
                .await
            }

            Command::DeleteTrigger {
                database,
                trigger_id,
            } => self.handle_delete_trigger(database, trigger_id).await,

            Command::ToggleTrigger {
                database,
                trigger_id,
            } => self.handle_toggle_trigger(database, trigger_id).await,

            // ==================== Environment Variables ====================
            Command::ListEnvVars { database } => self.handle_list_env_vars(database).await,

            Command::SetEnvVar {
                database,
                key,
                value,
            } => self.handle_set_env_var(database, key, value).await,

            Command::DeleteEnvVar { database, key } => {
                self.handle_delete_env_var(database, key).await
            }

            // ==================== Role Management ====================
            Command::ListRoles => self.handle_list_roles().await,

            Command::CreateRole { name, permissions } => {
                self.handle_create_role(name, permissions).await
            }

            Command::GetRole { name } => self.handle_get_role(name).await,

            Command::UpdateRole { name, permissions } => {
                self.handle_update_role(name, permissions).await
            }

            Command::DeleteRole { name } => self.handle_delete_role(name).await,

            // ==================== User Management ====================
            Command::ListUsers => self.handle_list_users().await,

            Command::CreateUser {
                username,
                password,
                roles,
            } => self.handle_create_user(username, password, roles).await,

            Command::DeleteUser { username } => self.handle_delete_user(username).await,

            Command::GetUserRoles { username } => self.handle_get_user_roles(username).await,

            Command::AssignRole {
                username,
                role,
                database,
            } => self.handle_assign_role(username, role, database).await,

            Command::RevokeRole { username, role } => {
                self.handle_revoke_role(username, role).await
            }

            Command::GetCurrentUser => {
                // This requires knowing the current authenticated user context
                Response::ok(serde_json::json!({"database": self.authenticated_db}))
            }

            Command::GetCurrentUserPermissions => {
                // Would need user context
                Response::ok(serde_json::json!({"message": "Permissions available after auth"}))
            }

            // ==================== API Key Management ====================
            Command::ListApiKeys => self.handle_list_api_keys().await,

            Command::CreateApiKey {
                name,
                permissions,
                expires_at,
            } => self.handle_create_api_key(name, permissions, expires_at).await,

            Command::DeleteApiKey { key_id } => self.handle_delete_api_key(key_id).await,

            // ==================== Cluster Management ====================
            Command::ClusterStatus | Command::ClusterInfo => {
                // These need ClusterManager which driver doesn't have access to
                Response::error(DriverError::DatabaseError(
                    "Cluster operations require HTTP API".to_string(),
                ))
            }

            Command::ClusterRemoveNode { .. }
            | Command::ClusterRebalance
            | Command::ClusterCleanup
            | Command::ClusterReshard { .. } => Response::error(DriverError::DatabaseError(
                "Cluster operations require HTTP API".to_string(),
            )),

            // ==================== Advanced Collection Operations ====================
            Command::TruncateCollection {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.truncate() {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            Command::CompactCollection {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => {
                    coll.compact();
                    Response::ok_empty()
                }
                Err(e) => Response::error(e),
            },

            Command::PruneCollection {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.prune() {
                    Ok(count) => Response::ok_count(count),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            Command::RecountCollection {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.recount() {
                    Ok(count) => Response::ok_count(count),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            Command::RepairCollection {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(_) => Response::error(DriverError::InvalidCommand(
                    "Repair not supported".to_string(),
                )),
                Err(e) => Response::error(e),
            },

            Command::GetCollectionSharding {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(_) => Response::error(DriverError::InvalidCommand(
                    "Sharding not supported".to_string(),
                )),
                Err(e) => Response::error(e),
            },

            Command::ExportCollection {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => {
                    let docs: Vec<_> = coll.scan(None).into_iter().map(|d| d.to_value()).collect();
                    Response::ok(serde_json::json!(docs))
                }
                Err(e) => Response::error(e),
            },

            Command::ImportCollection {
                database,
                collection,
                documents,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.insert_batch(documents) {
                    Ok(docs) => Response::ok_count(docs.len()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            Command::SetCollectionSchema {
                database,
                collection,
                schema,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => {
                    match serde_json::from_value::<CollectionSchema>(schema) {
                        Ok(s) => match coll.set_json_schema(s) {
                            Ok(_) => Response::ok_empty(),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        },
                        Err(e) => Response::error(DriverError::InvalidCommand(format!("Invalid schema: {}", e))),
                    }
                }
                Err(e) => Response::error(e),
            },

            Command::GetCollectionSchema {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.get_json_schema() {
                    Some(schema) => Response::ok(serde_json::to_value(schema).unwrap_or_default()),
                    None => Response::ok(serde_json::json!(null)),
                },
                Err(e) => Response::error(e),
            },

            Command::DeleteCollectionSchema {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.delete_collection_schema() {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            // ==================== Advanced Index Operations ====================
            Command::RebuildIndexes {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.rebuild_all_indexes() {
                    Ok(stats) => Response::ok(serde_json::to_value(stats).unwrap_or_default()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            Command::HybridSearch {
                database,
                collection,
                query: _,
                vector: _,
                vector_field: _,
                limit: _,
                alpha: _,
            } => match self.get_collection(&database, &collection) {
                Ok(_) => Response::error(DriverError::InvalidCommand(
                    "Hybrid search not yet supported".to_string(),
                )),
                Err(e) => Response::error(e),
            },

            // ==================== Geo Index Operations ====================
            Command::CreateGeoIndex {
                database,
                collection,
                name,
                field,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.create_geo_index(name, field) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            Command::ListGeoIndexes {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => {
                    let indexes = coll.list_geo_indexes();
                    Response::ok(serde_json::to_value(indexes).unwrap_or_default())
                }
                Err(e) => Response::error(e),
            },

            Command::DeleteGeoIndex {
                database,
                collection,
                name,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.drop_geo_index(&name) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            Command::GeoNear {
                database,
                collection,
                field,
                latitude,
                longitude,
                radius,
                limit,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => {
                    let results_opt = if let Some(r) = radius {
                        coll.geo_within(&field, latitude, longitude, r).map(|mut res| {
                            if let Some(l) = limit {
                                if (l as usize) < res.len() {
                                    res.truncate(l as usize);
                                }
                            }
                            res
                        })
                    } else {
                        coll.geo_near(
                            &field,
                            latitude,
                            longitude,
                            limit.unwrap_or(10) as usize,
                        )
                    };

                    match results_opt {
                        Some(results) => Response::ok(serde_json::json!(results)),
                        None => Response::error(DriverError::DatabaseError(
                            "Geo index not found".to_string(),
                        )),
                    }
                }
                Err(e) => Response::error(e),
            },

            Command::GeoWithin {
                database,
                collection,
                field,
                polygon: _,
            } => match self.get_collection(&database, &collection) {
                Ok(_) => Response::error(DriverError::InvalidCommand(
                    "Geo polygon search not supported".to_string(),
                )),
                Err(e) => Response::error(e),
            },

            // ==================== Vector Index Operations ====================
            Command::CreateVectorIndex {
                database,
                collection,
                name,
                field,
                dimensions,
                metric,
                ef_construction,
                m,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => {
                    let mut config = VectorIndexConfig::new(name, field, dimensions as usize);
                    
                    if let Some(m_str) = metric {
                        if let Ok(val) = serde_json::from_value::<VectorMetric>(serde_json::Value::String(m_str)) {
                            config = config.with_metric(val);
                        }
                    }
                    
                    if let Some(ef) = ef_construction {
                        config = config.with_ef_construction(ef as usize);
                    }
                    
                    if let Some(m_val) = m {
                        config = config.with_m(m_val as usize);
                    }

                    match coll.create_vector_index(config) {
                        Ok(_) => Response::ok_empty(),
                        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                    }
                }
                Err(e) => Response::error(e),
            },

            Command::ListVectorIndexes {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => {
                    let indexes = coll.list_vector_indexes();
                    Response::ok(serde_json::to_value(indexes).unwrap_or_default())
                }
                Err(e) => Response::error(e),
            },

            Command::DeleteVectorIndex {
                database,
                collection,
                name,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.drop_vector_index(&name) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            Command::VectorSearch {
                database,
                collection,
                index_name,
                vector,
                limit,
                ef_search,
                filter,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => {
                    // TODO: Implement filter support when Collection::vector_search supports it
                    match coll.vector_search(
                        &index_name,
                        &vector,
                        limit.unwrap_or(10) as usize,
                        ef_search.map(|v| v as usize),
                    ) {
                        Ok(results) => Response::ok(serde_json::json!(results)),
                        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                    }
                }
                Err(e) => Response::error(e),
            },

            Command::QuantizeVectorIndex {
                database,
                collection,
                index_name,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.quantize_vector_index(&index_name) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            Command::DequantizeVectorIndex {
                database,
                collection,
                index_name,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.dequantize_vector_index(&index_name) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            // ==================== TTL Index Operations ====================
            Command::CreateTtlIndex {
                database,
                collection,
                name,
                field,
                expire_after_seconds,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => {
                    match coll.create_ttl_index(name, field, expire_after_seconds as u64) {
                        Ok(_) => Response::ok_empty(),
                        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                    }
                }
                Err(e) => Response::error(e),
            },

            Command::ListTtlIndexes {
                database,
                collection,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => {
                    let indexes = coll.list_ttl_indexes();
                    Response::ok(serde_json::to_value(indexes).unwrap_or_default())
                }
                Err(e) => Response::error(e),
            },

            Command::DeleteTtlIndex {
                database,
                collection,
                name,
            } => match self.get_collection(&database, &collection) {
                Ok(coll) => match coll.drop_ttl_index(&name) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(e),
            },

            // ==================== Columnar Storage ====================
            Command::CreateColumnar {
                database,
                name,
                columns,
            } => match self.storage.get_database(&database) {
                Ok(db) => match db.create_columnar(name, columns) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::ListColumnar { database } => match self.storage.get_database(&database) {
                Ok(db) => {
                    let collections = db.list_columnar();
                    Response::ok(serde_json::json!(collections))
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::GetColumnar {
                database,
                collection,
            } => match self.storage.get_database(&database) {
                Ok(db) => match db.get_columnar(&collection) {
                    Ok(info) => Response::ok(serde_json::to_value(info).unwrap_or_default()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::DeleteColumnar {
                database,
                collection,
            } => match self.storage.get_database(&database) {
                Ok(db) => match db.delete_columnar(&collection) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::InsertColumnar {
                database,
                collection,
                rows,
            } => match self.storage.get_database(&database) {
                Ok(db) => match db.insert_columnar(&collection, rows) {
                    Ok(count) => Response::ok_count(count),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::AggregateColumnar {
                database,
                collection,
                aggregations,
                group_by,
                filter,
            } => match self.storage.get_database(&database) {
                Ok(db) => {
                    match db.aggregate_columnar(&collection, aggregations, group_by, filter) {
                        Ok(results) => Response::ok(serde_json::json!(results)),
                        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                    }
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::QueryColumnar {
                database,
                collection,
                columns,
                filter,
                order_by,
                limit,
            } => match self.storage.get_database(&database) {
                Ok(db) => {
                    match db.query_columnar(
                        &collection,
                        columns,
                        filter,
                        order_by,
                        limit.map(|l| l as usize),
                    ) {
                        Ok(results) => Response::ok(serde_json::json!(results)),
                        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                    }
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::CreateColumnarIndex {
                database,
                collection,
                column,
            } => match self.storage.get_database(&database) {
                Ok(db) => match db.create_columnar_index(&collection, &column) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::ListColumnarIndexes {
                database,
                collection,
            } => match self.storage.get_database(&database) {
                Ok(db) => match db.list_columnar_indexes(&collection) {
                    Ok(indexes) => Response::ok(serde_json::json!(indexes)),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },

            Command::DeleteColumnarIndex {
                database,
                collection,
                column,
            } => match self.storage.get_database(&database) {
                Ok(db) => match db.delete_columnar_index(&collection, &column) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
        }
    }

    /// Helper to get a collection
    fn get_collection(
        &self,
        database: &str,
        collection: &str,
    ) -> Result<crate::storage::Collection, DriverError> {
        let db = self
            .storage
            .get_database(database)
            .map_err(|e| DriverError::DatabaseError(e.to_string()))?;
        db.get_collection(collection)
            .map_err(|e| DriverError::DatabaseError(e.to_string()))
    }

    /// Handle authentication
    async fn handle_auth(
        &mut self,
        database: String,
        username: String,
        password: String,
    ) -> Response {
        // Get the _system database for auth lookup
        let system_db = match self.storage.get_database("_system") {
            Ok(db) => db,
            Err(e) => {
                return Response::error(DriverError::AuthError(format!(
                    "System database error: {}",
                    e
                )))
            }
        };

        // Get admins collection (username is the _key)
        let admins = match system_db.get_collection("_admins") {
            Ok(coll) => coll,
            Err(_) => {
                return Response::error(DriverError::AuthError(
                    "Admins collection not found".to_string(),
                ))
            }
        };

        // Find user by username (username IS the _key in _admins collection)
        let user_doc = match admins.get(&username) {
            Ok(doc) => doc,
            Err(_) => {
                return Response::error(DriverError::AuthError("Invalid credentials".to_string()))
            }
        };

        // Parse user
        let user: crate::server::auth::User = match serde_json::from_value(user_doc.to_value()) {
            Ok(u) => u,
            Err(_) => {
                return Response::error(DriverError::AuthError("Invalid credentials".to_string()))
            }
        };

        // Verify password using AuthService
        if !crate::server::auth::AuthService::verify_password(&password, &user.password_hash) {
            return Response::error(DriverError::AuthError("Invalid credentials".to_string()));
        }

        // Verify requested database exists
        if let Err(e) = self.storage.get_database(&database) {
            return Response::error(DriverError::DatabaseError(format!(
                "Database not found: {}",
                e
            )));
        }

        // Set authenticated state (admin users have access to all databases)
        self.authenticated_db = Some(database);
        Response::ok_empty()
    }

    // ==================== Script Management Handlers ====================

    async fn handle_script_create(
        &self,
        database: String,
        name: String,
        path: String,
        methods: Vec<String>,
        code: String,
        description: Option<String>,
        collection: Option<String>,
    ) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => {
                let scripts_coll = match db.get_or_create_collection("_scripts") {
                    Ok(c) => c,
                    Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
                };

                let script_doc = serde_json::json!({
                    "name": name,
                    "path": path,
                    "methods": methods,
                    "code": code,
                    "description": description,
                    "collection": collection,
                    "created_at": chrono::Utc::now().to_rfc3339(),
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                });

                match scripts_coll.insert(script_doc) {
                    Ok(doc) => Response::ok(doc.to_value()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_script_list(&self, database: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_scripts") {
                Ok(coll) => {
                    let scripts: Vec<_> = coll.scan(None).into_iter().map(|d| d.to_value()).collect();
                    Response::ok(serde_json::json!({"scripts": scripts}))
                }
                Err(_) => Response::ok(serde_json::json!({"scripts": []})),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_script_get(&self, database: String, script_id: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_scripts") {
                Ok(coll) => match coll.get(&script_id) {
                    Ok(doc) => Response::ok(doc.to_value()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_script_update(
        &self,
        database: String,
        script_id: String,
        name: Option<String>,
        path: Option<String>,
        methods: Option<Vec<String>>,
        code: Option<String>,
        description: Option<String>,
    ) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_scripts") {
                Ok(coll) => {
                    let mut update = serde_json::Map::new();
                    if let Some(v) = name {
                        update.insert("name".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = path {
                        update.insert("path".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = methods {
                        update.insert("methods".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = code {
                        update.insert("code".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = description {
                        update.insert("description".to_string(), serde_json::json!(v));
                    }
                    update.insert(
                        "updated_at".to_string(),
                        serde_json::json!(chrono::Utc::now().to_rfc3339()),
                    );

                    match coll.get(&script_id) {
                        Ok(existing) => {
                            let mut merged = existing.data.clone();
                            if let Some(obj) = merged.as_object_mut() {
                                for (k, v) in update {
                                    obj.insert(k, v);
                                }
                            }
                            match coll.update(&script_id, merged) {
                                Ok(doc) => Response::ok(doc.to_value()),
                                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                            }
                        }
                        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                    }
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_script_delete(&self, database: String, script_id: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_scripts") {
                Ok(coll) => match coll.delete(&script_id) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    // ==================== Job/Queue Handlers ====================

    async fn handle_list_queues(&self, database: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_jobs") {
                Ok(coll) => {
                    let jobs: Vec<_> = coll.scan(None);
                    let mut queues: std::collections::HashSet<String> = std::collections::HashSet::new();
                    for job in jobs {
                        if let Some(queue) = job.data.get("queue").and_then(|v| v.as_str()) {
                            queues.insert(queue.to_string());
                        }
                    }
                    let queue_list: Vec<_> = queues.into_iter().collect();
                    Response::ok(serde_json::json!({"queues": queue_list}))
                }
                Err(_) => Response::ok(serde_json::json!({"queues": []})),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_list_jobs(
        &self,
        database: String,
        queue_name: String,
        status: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_jobs") {
                Ok(coll) => {
                    let jobs: Vec<_> = coll
                        .scan(None)
                        .into_iter()
                        .filter(|job| {
                            let queue_match = job
                                .data
                                .get("queue")
                                .and_then(|v| v.as_str())
                                .map(|q| q == queue_name)
                                .unwrap_or(false);
                            let status_match = status.as_ref().map_or(true, |s| {
                                job.data
                                    .get("status")
                                    .and_then(|v| v.as_str())
                                    .map(|js| js == s)
                                    .unwrap_or(false)
                            });
                            queue_match && status_match
                        })
                        .skip(offset.unwrap_or(0))
                        .take(limit.unwrap_or(50))
                        .map(|d| d.to_value())
                        .collect();
                    Response::ok(serde_json::json!({"jobs": jobs}))
                }
                Err(_) => Response::ok(serde_json::json!({"jobs": []})),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_enqueue_job(
        &self,
        database: String,
        queue_name: String,
        script_path: String,
        params: HashMap<String, serde_json::Value>,
        priority: Option<i32>,
        run_at: Option<String>,
        max_retries: Option<i32>,
    ) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => {
                let jobs_coll = match db.get_or_create_collection("_jobs") {
                    Ok(c) => c,
                    Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
                };

                let job_doc = serde_json::json!({
                    "queue": queue_name,
                    "script_path": script_path,
                    "params": params,
                    "priority": priority.unwrap_or(0),
                    "run_at": run_at,
                    "max_retries": max_retries.unwrap_or(3),
                    "retry_count": 0,
                    "status": "pending",
                    "created_at": chrono::Utc::now().to_rfc3339(),
                });

                match jobs_coll.insert(job_doc) {
                    Ok(doc) => Response::ok(doc.to_value()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_cancel_job(&self, database: String, job_id: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_jobs") {
                Ok(coll) => match coll.delete(&job_id) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    // ==================== Cron Job Handlers ====================

    async fn handle_list_cron_jobs(&self, database: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_cron_jobs") {
                Ok(coll) => {
                    let cron_jobs: Vec<_> = coll.scan(None).into_iter().map(|d| d.to_value()).collect();
                    Response::ok(serde_json::json!({"cron_jobs": cron_jobs}))
                }
                Err(_) => Response::ok(serde_json::json!({"cron_jobs": []})),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_create_cron_job(
        &self,
        database: String,
        name: String,
        cron_expression: String,
        script_path: String,
        params: HashMap<String, serde_json::Value>,
        queue: Option<String>,
        priority: Option<i32>,
        max_retries: Option<i32>,
    ) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => {
                let cron_coll = match db.get_or_create_collection("_cron_jobs") {
                    Ok(c) => c,
                    Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
                };

                let cron_doc = serde_json::json!({
                    "name": name,
                    "cron_expression": cron_expression,
                    "script_path": script_path,
                    "params": params,
                    "queue": queue.unwrap_or_else(|| "default".to_string()),
                    "priority": priority.unwrap_or(0),
                    "max_retries": max_retries.unwrap_or(3),
                    "created_at": chrono::Utc::now().to_rfc3339(),
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                });

                match cron_coll.insert(cron_doc) {
                    Ok(doc) => Response::ok(doc.to_value()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_update_cron_job(
        &self,
        database: String,
        cron_id: String,
        name: Option<String>,
        cron_expression: Option<String>,
        script_path: Option<String>,
        params: Option<HashMap<String, serde_json::Value>>,
        queue: Option<String>,
        priority: Option<i32>,
        max_retries: Option<i32>,
    ) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_cron_jobs") {
                Ok(coll) => {
                    let mut update = serde_json::Map::new();
                    if let Some(v) = name {
                        update.insert("name".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = cron_expression {
                        update.insert("cron_expression".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = script_path {
                        update.insert("script_path".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = params {
                        update.insert("params".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = queue {
                        update.insert("queue".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = priority {
                        update.insert("priority".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = max_retries {
                        update.insert("max_retries".to_string(), serde_json::json!(v));
                    }
                    update.insert(
                        "updated_at".to_string(),
                        serde_json::json!(chrono::Utc::now().to_rfc3339()),
                    );

                    match coll.get(&cron_id) {
                        Ok(existing) => {
                            let mut merged = existing.data.clone();
                            if let Some(obj) = merged.as_object_mut() {
                                for (k, v) in update {
                                    obj.insert(k, v);
                                }
                            }
                            match coll.update(&cron_id, merged) {
                                Ok(doc) => Response::ok(doc.to_value()),
                                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                            }
                        }
                        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                    }
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_delete_cron_job(&self, database: String, cron_id: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_cron_jobs") {
                Ok(coll) => match coll.delete(&cron_id) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    // ==================== Trigger Handlers ====================

    async fn handle_list_triggers(&self, database: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_triggers") {
                Ok(coll) => {
                    let triggers: Vec<_> = coll.scan(None).into_iter().map(|d| d.to_value()).collect();
                    Response::ok(serde_json::json!({"triggers": triggers}))
                }
                Err(_) => Response::ok(serde_json::json!({"triggers": []})),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_list_collection_triggers(
        &self,
        database: String,
        collection: String,
    ) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_triggers") {
                Ok(coll) => {
                    let triggers: Vec<_> = coll
                        .scan(None)
                        .into_iter()
                        .filter(|t| {
                            t.data
                                .get("collection")
                                .and_then(|v| v.as_str())
                                .map(|c| c == collection)
                                .unwrap_or(false)
                        })
                        .map(|d| d.to_value())
                        .collect();
                    Response::ok(serde_json::json!({"triggers": triggers}))
                }
                Err(_) => Response::ok(serde_json::json!({"triggers": []})),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_create_trigger(
        &self,
        database: String,
        name: String,
        collection: String,
        events: Vec<String>,
        script_path: String,
        filter: Option<String>,
        queue: Option<String>,
        priority: Option<i32>,
        max_retries: Option<i32>,
        enabled: Option<bool>,
    ) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => {
                let triggers_coll = match db.get_or_create_collection("_triggers") {
                    Ok(c) => c,
                    Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
                };

                let trigger_doc = serde_json::json!({
                    "name": name,
                    "collection": collection,
                    "events": events,
                    "script_path": script_path,
                    "filter": filter,
                    "queue": queue.unwrap_or_else(|| "default".to_string()),
                    "priority": priority.unwrap_or(0),
                    "max_retries": max_retries.unwrap_or(3),
                    "enabled": enabled.unwrap_or(true),
                    "created_at": chrono::Utc::now().to_rfc3339(),
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                });

                match triggers_coll.insert(trigger_doc) {
                    Ok(doc) => Response::ok(doc.to_value()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_get_trigger(&self, database: String, trigger_id: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_triggers") {
                Ok(coll) => match coll.get(&trigger_id) {
                    Ok(doc) => Response::ok(doc.to_value()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_update_trigger(
        &self,
        database: String,
        trigger_id: String,
        name: Option<String>,
        events: Option<Vec<String>>,
        script_path: Option<String>,
        filter: Option<String>,
        queue: Option<String>,
        priority: Option<i32>,
        max_retries: Option<i32>,
        enabled: Option<bool>,
    ) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_triggers") {
                Ok(coll) => {
                    let mut update = serde_json::Map::new();
                    if let Some(v) = name {
                        update.insert("name".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = events {
                        update.insert("events".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = script_path {
                        update.insert("script_path".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = filter {
                        update.insert("filter".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = queue {
                        update.insert("queue".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = priority {
                        update.insert("priority".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = max_retries {
                        update.insert("max_retries".to_string(), serde_json::json!(v));
                    }
                    if let Some(v) = enabled {
                        update.insert("enabled".to_string(), serde_json::json!(v));
                    }
                    update.insert(
                        "updated_at".to_string(),
                        serde_json::json!(chrono::Utc::now().to_rfc3339()),
                    );

                    match coll.get(&trigger_id) {
                        Ok(existing) => {
                            let mut merged = existing.data.clone();
                            if let Some(obj) = merged.as_object_mut() {
                                for (k, v) in update {
                                    obj.insert(k, v);
                                }
                            }
                            match coll.update(&trigger_id, merged) {
                                Ok(doc) => Response::ok(doc.to_value()),
                                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                            }
                        }
                        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                    }
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_delete_trigger(&self, database: String, trigger_id: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_triggers") {
                Ok(coll) => match coll.delete(&trigger_id) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_toggle_trigger(&self, database: String, trigger_id: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_triggers") {
                Ok(coll) => match coll.get(&trigger_id) {
                    Ok(existing) => {
                        let current_enabled = existing
                            .data
                            .get("enabled")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        let mut merged = existing.data.clone();
                        if let Some(obj) = merged.as_object_mut() {
                            obj.insert("enabled".to_string(), serde_json::json!(!current_enabled));
                            obj.insert(
                                "updated_at".to_string(),
                                serde_json::json!(chrono::Utc::now().to_rfc3339()),
                            );
                        }
                        match coll.update(&trigger_id, merged) {
                            Ok(doc) => Response::ok(doc.to_value()),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    // ==================== Environment Variable Handlers ====================

    async fn handle_list_env_vars(&self, database: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_env") {
                Ok(coll) => {
                    let mut vars: HashMap<String, String> = HashMap::new();
                    for doc in coll.scan(None) {
                        if let (Some(key), Some(value)) = (
                            doc.data.get("key").and_then(|v| v.as_str()),
                            doc.data.get("value").and_then(|v| v.as_str()),
                        ) {
                            vars.insert(key.to_string(), value.to_string());
                        }
                    }
                    Response::ok(serde_json::json!({"variables": vars}))
                }
                Err(_) => Response::ok(serde_json::json!({"variables": {}})),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_set_env_var(&self, database: String, key: String, value: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => {
                let env_coll = match db.get_or_create_collection("_env") {
                    Ok(c) => c,
                    Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
                };

                // Use key as _key for easy lookup
                let env_doc = serde_json::json!({
                    "_key": key,
                    "key": key,
                    "value": value,
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                });

                // Try update first, then insert
                match env_coll.update(&key, env_doc.clone()) {
                    Ok(_) => Response::ok_empty(),
                    Err(_) => match env_coll.insert(env_doc) {
                        Ok(_) => Response::ok_empty(),
                        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                    },
                }
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_delete_env_var(&self, database: String, key: String) -> Response {
        match self.storage.get_database(&database) {
            Ok(db) => match db.get_collection("_env") {
                Ok(coll) => match coll.delete(&key) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    // ==================== Role Management Handlers ====================

    async fn handle_list_roles(&self) -> Response {
        match self.storage.get_database("_system") {
            Ok(db) => match db.get_collection("_roles") {
                Ok(coll) => {
                    let roles: Vec<_> = coll.scan(None).into_iter().map(|d| d.to_value()).collect();
                    Response::ok(serde_json::json!({"roles": roles}))
                }
                Err(_) => Response::ok(serde_json::json!({"roles": []})),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_create_role(&self, name: String, permissions: Vec<serde_json::Value>) -> Response {
        match self.storage.get_database("_system") {
            Ok(db) => {
                let roles_coll = match db.get_or_create_collection("_roles") {
                    Ok(c) => c,
                    Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
                };

                let role_doc = serde_json::json!({
                    "_key": name,
                    "name": name,
                    "permissions": permissions,
                    "created_at": chrono::Utc::now().to_rfc3339(),
                });

                match roles_coll.insert(role_doc) {
                    Ok(doc) => Response::ok(doc.to_value()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_get_role(&self, name: String) -> Response {
        match self.storage.get_database("_system") {
            Ok(db) => match db.get_collection("_roles") {
                Ok(coll) => match coll.get(&name) {
                    Ok(doc) => Response::ok(doc.to_value()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_update_role(&self, name: String, permissions: Vec<serde_json::Value>) -> Response {
        match self.storage.get_database("_system") {
            Ok(db) => match db.get_collection("_roles") {
                Ok(coll) => match coll.get(&name) {
                    Ok(existing) => {
                        let mut merged = existing.data.clone();
                        if let Some(obj) = merged.as_object_mut() {
                            obj.insert("permissions".to_string(), serde_json::json!(permissions));
                            obj.insert(
                                "updated_at".to_string(),
                                serde_json::json!(chrono::Utc::now().to_rfc3339()),
                            );
                        }
                        match coll.update(&name, merged) {
                            Ok(doc) => Response::ok(doc.to_value()),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_delete_role(&self, name: String) -> Response {
        // Prevent deleting built-in roles
        if name == "admin" || name == "developer" || name == "viewer" {
            return Response::error(DriverError::DatabaseError(
                "Cannot delete built-in role".to_string(),
            ));
        }

        match self.storage.get_database("_system") {
            Ok(db) => match db.get_collection("_roles") {
                Ok(coll) => match coll.delete(&name) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    // ==================== User Management Handlers ====================

    async fn handle_list_users(&self) -> Response {
        match self.storage.get_database("_system") {
            Ok(db) => match db.get_collection("_admins") {
                Ok(coll) => {
                    let users: Vec<_> = coll
                        .scan(None)
                        .into_iter()
                        .map(|d| {
                            // Strip password_hash from response
                            let mut val = d.to_value();
                            if let Some(obj) = val.as_object_mut() {
                                obj.remove("password_hash");
                            }
                            val
                        })
                        .collect();
                    Response::ok(serde_json::json!({"users": users}))
                }
                Err(_) => Response::ok(serde_json::json!({"users": []})),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_create_user(
        &self,
        username: String,
        password: String,
        roles: Option<Vec<String>>,
    ) -> Response {
        match self.storage.get_database("_system") {
            Ok(db) => {
                let admins_coll = match db.get_or_create_collection("_admins") {
                    Ok(c) => c,
                    Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
                };

                // Hash password
                let password_hash = crate::server::auth::AuthService::hash_password(&password);

                let user_doc = serde_json::json!({
                    "_key": username,
                    "username": username,
                    "password_hash": password_hash,
                    "roles": roles.unwrap_or_default(),
                    "created_at": chrono::Utc::now().to_rfc3339(),
                });

                match admins_coll.insert(user_doc) {
                    Ok(doc) => {
                        let mut val = doc.to_value();
                        if let Some(obj) = val.as_object_mut() {
                            obj.remove("password_hash");
                        }
                        Response::ok(val)
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_delete_user(&self, username: String) -> Response {
        // Prevent deleting admin user
        if username == "admin" {
            return Response::error(DriverError::DatabaseError(
                "Cannot delete admin user".to_string(),
            ));
        }

        match self.storage.get_database("_system") {
            Ok(db) => match db.get_collection("_admins") {
                Ok(coll) => match coll.delete(&username) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_get_user_roles(&self, username: String) -> Response {
        match self.storage.get_database("_system") {
            Ok(db) => match db.get_collection("_user_roles") {
                Ok(coll) => {
                    let roles: Vec<_> = coll
                        .scan(None)
                        .into_iter()
                        .filter(|d| {
                            d.data
                                .get("username")
                                .and_then(|v| v.as_str())
                                .map(|u| u == username)
                                .unwrap_or(false)
                        })
                        .map(|d| d.to_value())
                        .collect();
                    Response::ok(serde_json::json!({"roles": roles}))
                }
                Err(_) => Response::ok(serde_json::json!({"roles": []})),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_assign_role(
        &self,
        username: String,
        role: String,
        database: Option<String>,
    ) -> Response {
        match self.storage.get_database("_system") {
            Ok(db) => {
                let user_roles_coll = match db.get_or_create_collection("_user_roles") {
                    Ok(c) => c,
                    Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
                };

                let role_doc = serde_json::json!({
                    "username": username,
                    "role": role,
                    "database": database,
                    "assigned_at": chrono::Utc::now().to_rfc3339(),
                });

                match user_roles_coll.insert(role_doc) {
                    Ok(doc) => Response::ok(doc.to_value()),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_revoke_role(&self, username: String, role: String) -> Response {
        match self.storage.get_database("_system") {
            Ok(db) => match db.get_collection("_user_roles") {
                Ok(coll) => {
                    // Find and delete the role assignment
                    for doc in coll.scan(None) {
                        let matches = doc
                            .data
                            .get("username")
                            .and_then(|v| v.as_str())
                            .map(|u| u == username)
                            .unwrap_or(false)
                            && doc
                                .data
                                .get("role")
                                .and_then(|v| v.as_str())
                                .map(|r| r == role)
                                .unwrap_or(false);
                        if matches {
                            if let Some(key) = doc.data.get("_key").and_then(|v| v.as_str()) {
                                let _ = coll.delete(key);
                            }
                        }
                    }
                    Response::ok_empty()
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    // ==================== API Key Management Handlers ====================

    async fn handle_list_api_keys(&self) -> Response {
        match self.storage.get_database("_system") {
            Ok(db) => match db.get_collection("_api_keys") {
                Ok(coll) => {
                    let keys: Vec<_> = coll
                        .scan(None)
                        .into_iter()
                        .map(|d| {
                            // Strip the actual key value from response
                            let mut val = d.to_value();
                            if let Some(obj) = val.as_object_mut() {
                                obj.remove("key");
                            }
                            val
                        })
                        .collect();
                    Response::ok(serde_json::json!({"api_keys": keys}))
                }
                Err(_) => Response::ok(serde_json::json!({"api_keys": []})),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_create_api_key(
        &self,
        name: String,
        permissions: Option<Vec<serde_json::Value>>,
        expires_at: Option<String>,
    ) -> Response {
        match self.storage.get_database("_system") {
            Ok(db) => {
                let api_keys_coll = match db.get_or_create_collection("_api_keys") {
                    Ok(c) => c,
                    Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
                };

                // Generate a random API key
                let key = format!("sdb_{}", uuid::Uuid::new_v4().to_string().replace("-", ""));

                let api_key_doc = serde_json::json!({
                    "name": name,
                    "key": key,
                    "permissions": permissions.unwrap_or_default(),
                    "expires_at": expires_at,
                    "created_at": chrono::Utc::now().to_rfc3339(),
                });

                match api_keys_coll.insert(api_key_doc) {
                    Ok(doc) => {
                        // Return the key only on creation
                        let mut val = doc.to_value();
                        if let Some(obj) = val.as_object_mut() {
                            obj.insert("key".to_string(), serde_json::json!(key));
                        }
                        Response::ok(val)
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
    }

    async fn handle_delete_api_key(&self, key_id: String) -> Response {
        match self.storage.get_database("_system") {
            Ok(db) => match db.get_collection("_api_keys") {
                Ok(coll) => match coll.delete(&key_id) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        }
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
