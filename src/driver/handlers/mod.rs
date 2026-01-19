//! Connection handler for native driver protocol
//!
//! Processes incoming commands and executes them against the storage engine.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::storage::StorageEngine;
use crate::transaction::TransactionId;

use solidb_client::protocol::{
    decode_message, encode_response, Command, DriverError, Response, MAX_MESSAGE_SIZE,
};

pub mod admin;
pub mod auth;
pub mod database;
pub mod document;
pub mod index;
pub mod query;
pub mod scheduler;
pub mod transaction;

/// Handler for a single driver connection
pub struct DriverHandler {
    pub(crate) storage: Arc<StorageEngine>,
    /// Active transactions for this connection
    pub(crate) transactions: HashMap<String, TransactionId>,
    /// Authenticated database (None = not authenticated)
    pub(crate) authenticated_db: Option<String>,
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
                api_key,
            } => auth::handle_auth(self, database, username, password, api_key).await,

            // ==================== Database Operations ====================
            Command::ListDatabases => database::handle_list_databases(self),

            Command::CreateDatabase { name } => database::handle_create_database(self, name),

            Command::DeleteDatabase { name } => database::handle_delete_database(self, name),

            // ==================== Collection Operations ====================
            Command::ListCollections { database } => {
                database::handle_list_collections(self, database)
            }

            Command::CreateCollection {
                database,
                name,
                collection_type,
            } => database::handle_create_collection(self, database, name, collection_type),

            Command::DeleteCollection { database, name } => {
                database::handle_delete_collection(self, database, name)
            }

            Command::CollectionStats { database, name } => {
                database::handle_collection_stats(self, database, name)
            }

            // ==================== Document Operations ====================
            Command::Get {
                database,
                collection,
                key,
            } => document::handle_get(self, database, collection, key),

            Command::Insert {
                database,
                collection,
                key,
                document,
            } => document::handle_insert(self, database, collection, key, document),

            Command::Update {
                database,
                collection,
                key,
                document,
                merge,
            } => document::handle_update(self, database, collection, key, document, merge),

            Command::Delete {
                database,
                collection,
                key,
            } => document::handle_delete(self, database, collection, key),

            Command::List {
                database,
                collection,
                limit,
                offset,
            } => document::handle_list(self, database, collection, limit, offset),

            // ==================== Query Operations ====================
            Command::Query {
                database,
                sdbql,
                bind_vars,
            } => query::handle_query(self, database, sdbql, bind_vars),

            Command::Explain {
                database,
                sdbql,
                bind_vars,
            } => query::handle_explain(self, database, sdbql, bind_vars),

            // ==================== Index Operations ====================
            Command::CreateIndex {
                database,
                collection,
                name,
                fields,
                unique,
                sparse: _,
            } => index::handle_create_index(self, database, collection, name, fields, unique),

            Command::DeleteIndex {
                database,
                collection,
                name,
            } => index::handle_delete_index(self, database, collection, name),

            Command::ListIndexes {
                database,
                collection,
            } => index::handle_list_indexes(self, database, collection),

            // ==================== Transaction Operations ====================
            Command::BeginTransaction {
                database,
                isolation_level,
            } => transaction::handle_begin_transaction(self, database, isolation_level),

            Command::CommitTransaction { tx_id } => {
                transaction::handle_commit_transaction(self, tx_id)
            }

            Command::RollbackTransaction { tx_id } => {
                transaction::handle_rollback_transaction(self, tx_id)
            }

            Command::TransactionCommand { tx_id, command } => {
                transaction::handle_transaction_command(self, tx_id, command).await
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
            } => document::handle_bulk_insert(self, database, collection, documents),

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
                scheduler::handle_script_create(
                    self,
                    database,
                    scheduler::ScriptCreateConfig {
                        name,
                        path,
                        methods,
                        code,
                        description,
                        collection,
                    },
                )
                .await
            }

            Command::ListScripts { database } => {
                scheduler::handle_script_list(self, database).await
            }

            Command::GetScript {
                database,
                script_id,
            } => scheduler::handle_script_get(self, database, script_id).await,

            Command::UpdateScript {
                database,
                script_id,
                name,
                path,
                methods,
                code,
                description,
            } => {
                scheduler::handle_script_update(
                    self,
                    database,
                    script_id,
                    scheduler::ScriptUpdateConfig {
                        name,
                        path,
                        methods,
                        code,
                        description,
                    },
                )
                .await
            }

            Command::DeleteScript {
                database,
                script_id,
            } => scheduler::handle_script_delete(self, database, script_id).await,

            Command::GetScriptStats => {
                Response::ok(serde_json::json!({"message": "Script stats available via HTTP API"}))
            }

            // ==================== Job/Queue Management ====================
            Command::ListQueues { database } => scheduler::handle_list_queues(self, database).await,

            Command::ListJobs {
                database,
                queue_name,
                status,
                limit,
                offset,
            } => {
                scheduler::handle_list_jobs(
                    self,
                    database,
                    scheduler::ListJobsConfig {
                        queue_name,
                        status,
                        limit,
                        offset,
                    },
                )
                .await
            }

            Command::EnqueueJob {
                database,
                queue_name,
                script_path,
                params,
                priority,
                run_at,
                max_retries,
            } => {
                scheduler::handle_enqueue_job(
                    self,
                    database,
                    scheduler::EnqueueJobConfig {
                        queue_name,
                        script_path,
                        params,
                        priority,
                        run_at,
                        max_retries,
                    },
                )
                .await
            }

            Command::CancelJob { database, job_id } => {
                scheduler::handle_cancel_job(self, database, job_id).await
            }

            // ==================== Cron Job Management ====================
            Command::ListCronJobs { database } => {
                scheduler::handle_list_cron_jobs(self, database).await
            }

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
                scheduler::handle_create_cron_job(
                    self,
                    database,
                    scheduler::CronJobCreateConfig {
                        name,
                        cron_expression,
                        script_path,
                        params,
                        queue,
                        priority,
                        max_retries,
                    },
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
                scheduler::handle_update_cron_job(
                    self,
                    database,
                    cron_id,
                    scheduler::CronJobUpdateConfig {
                        name,
                        cron_expression,
                        script_path,
                        params,
                        queue,
                        priority,
                        max_retries,
                    },
                )
                .await
            }

            Command::DeleteCronJob { database, cron_id } => {
                scheduler::handle_delete_cron_job(self, database, cron_id).await
            }

            // ==================== Trigger Management ====================
            Command::ListTriggers { database } => {
                scheduler::handle_list_triggers(self, database).await
            }

            Command::ListCollectionTriggers {
                database,
                collection,
            } => scheduler::handle_list_collection_triggers(self, database, collection).await,

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
                scheduler::handle_create_trigger(
                    self,
                    database,
                    scheduler::TriggerCreateConfig {
                        name,
                        collection,
                        events,
                        script_path,
                        filter,
                        queue,
                        priority,
                        max_retries,
                        enabled,
                    },
                )
                .await
            }

            Command::GetTrigger {
                database,
                trigger_id,
            } => scheduler::handle_get_trigger(self, database, trigger_id).await,

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
                scheduler::handle_update_trigger(
                    self,
                    database,
                    trigger_id,
                    scheduler::TriggerUpdateConfig {
                        name,
                        events,
                        script_path,
                        filter,
                        queue,
                        priority,
                        max_retries,
                        enabled,
                    },
                )
                .await
            }

            Command::DeleteTrigger {
                database,
                trigger_id,
            } => scheduler::handle_delete_trigger(self, database, trigger_id).await,

            Command::ToggleTrigger {
                database,
                trigger_id,
            } => scheduler::handle_toggle_trigger(self, database, trigger_id).await,

            // ==================== Environment Variables ====================
            Command::ListEnvVars { database } => admin::handle_list_env_vars(self, database).await,

            Command::SetEnvVar {
                database,
                key,
                value,
            } => admin::handle_set_env_var(self, database, key, value).await,

            Command::DeleteEnvVar { database, key } => {
                admin::handle_delete_env_var(self, database, key).await
            }

            // ==================== Role Management ====================
            Command::ListRoles => admin::handle_list_roles(self).await,

            Command::CreateRole { name, permissions } => {
                admin::handle_create_role(self, name, permissions).await
            }

            Command::GetRole { name } => admin::handle_get_role(self, name).await,

            Command::UpdateRole { name, permissions } => {
                admin::handle_update_role(self, name, permissions).await
            }

            Command::DeleteRole { name } => admin::handle_delete_role(self, name).await,

            // ==================== User Management ====================
            Command::ListUsers => admin::handle_list_users(self).await,

            Command::CreateUser {
                username,
                password,
                roles,
            } => admin::handle_create_user(self, username, password, roles).await,

            Command::DeleteUser { username } => admin::handle_delete_user(self, username).await,

            Command::GetUserRoles { username } => {
                admin::handle_get_user_roles(self, username).await
            }

            Command::AssignRole {
                username,
                role,
                database,
            } => admin::handle_assign_role(self, username, role, database).await,

            Command::RevokeRole { username, role } => {
                admin::handle_revoke_role(self, username, role).await
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
            Command::ListApiKeys => admin::handle_list_api_keys(self).await,

            Command::CreateApiKey {
                name,
                permissions,
                expires_at,
            } => admin::handle_create_api_key(self, name, permissions, expires_at).await,

            Command::DeleteApiKey { key_id } => admin::handle_delete_api_key(self, key_id).await,

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
            } => database::handle_truncate_collection(self, database, collection),

            Command::CompactCollection {
                database,
                collection,
            } => database::handle_compact_collection(self, database, collection),

            Command::PruneCollection { .. } => Response::error(DriverError::InvalidCommand(
                "Prune not supported".to_string(),
            )),

            Command::RecountCollection {
                database,
                collection,
            } => database::handle_recount_collection(self, database, collection),

            Command::RepairCollection { .. } => Response::error(DriverError::InvalidCommand(
                "Repair not supported".to_string(),
            )),

            Command::GetCollectionSharding { .. } => Response::error(DriverError::InvalidCommand(
                "Sharding not supported".to_string(),
            )),

            Command::ExportCollection {
                database,
                collection,
            } => database::handle_export_collection(self, database, collection),

            Command::ImportCollection {
                database,
                collection,
                documents,
            } => database::handle_import_collection(self, database, collection, documents),

            Command::SetCollectionSchema {
                database,
                collection,
                schema,
            } => database::handle_set_collection_schema(self, database, collection, schema),

            Command::GetCollectionSchema {
                database,
                collection,
            } => database::handle_get_collection_schema(self, database, collection),

            Command::DeleteCollectionSchema {
                database,
                collection,
            } => database::handle_delete_collection_schema(self, database, collection),

            // ==================== Advanced Index Operations ====================
            Command::RebuildIndexes {
                database,
                collection,
            } => index::handle_rebuild_indexes(self, database, collection),

            Command::HybridSearch { .. } => Response::error(DriverError::InvalidCommand(
                "Hybrid search not yet supported".to_string(),
            )),

            // ==================== Geo Index Operations ====================
            Command::CreateGeoIndex {
                database,
                collection,
                name,
                field,
            } => index::handle_create_geo_index(self, database, collection, name, field),

            Command::ListGeoIndexes {
                database,
                collection,
            } => index::handle_list_geo_indexes(self, database, collection),

            Command::DeleteGeoIndex {
                database,
                collection,
                name,
            } => index::handle_delete_geo_index(self, database, collection, name),

            Command::GeoNear {
                database,
                collection,
                field,
                latitude,
                longitude,
                radius,
                limit,
            } => index::handle_geo_near(
                self,
                database,
                index::GeoNearConfig {
                    collection,
                    field,
                    latitude,
                    longitude,
                    radius,
                    limit,
                },
            ),

            Command::GeoWithin { .. } => Response::error(DriverError::InvalidCommand(
                "Geo polygon search not supported".to_string(),
            )),

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
            } => index::handle_create_vector_index(
                self,
                database,
                index::VectorIndexCreateConfig {
                    collection,
                    name,
                    field,
                    dimensions,
                    metric,
                    ef_construction,
                    m,
                },
            ),

            Command::ListVectorIndexes {
                database,
                collection,
            } => index::handle_list_vector_indexes(self, database, collection),

            Command::DeleteVectorIndex {
                database,
                collection,
                name,
            } => index::handle_delete_vector_index(self, database, collection, name),

            Command::VectorSearch {
                database,
                collection,
                index_name,
                vector,
                limit,
                ef_search,
                filter: _,
            } => index::handle_vector_search(
                self, database, collection, index_name, vector, limit, ef_search,
            ),

            Command::QuantizeVectorIndex {
                database,
                collection,
                index_name,
            } => index::handle_quantize_vector_index(self, database, collection, index_name),

            Command::DequantizeVectorIndex {
                database,
                collection,
                index_name,
            } => index::handle_dequantize_vector_index(self, database, collection, index_name),

            // ==================== TTL Index Operations ====================
            Command::CreateTtlIndex {
                database,
                collection,
                name,
                field,
                expire_after_seconds,
            } => index::handle_create_ttl_index(
                self,
                database,
                collection,
                name,
                field,
                expire_after_seconds,
            ),

            Command::ListTtlIndexes {
                database,
                collection,
            } => index::handle_list_ttl_indexes(self, database, collection),

            Command::DeleteTtlIndex {
                database,
                collection,
                name,
            } => index::handle_delete_ttl_index(self, database, collection, name),

            // ==================== Columnar Storage ====================
            Command::CreateColumnar {
                database,
                name,
                columns,
            } => database::handle_create_columnar(self, database, name, columns),

            Command::ListColumnar { database } => database::handle_list_columnar(self, database),

            Command::GetColumnar {
                database,
                collection,
            } => database::handle_get_columnar(self, database, collection),

            Command::DeleteColumnar {
                database,
                collection,
            } => database::handle_delete_columnar(self, database, collection),

            Command::InsertColumnar {
                database,
                collection,
                rows,
            } => database::handle_insert_columnar(self, database, collection, rows),

            Command::AggregateColumnar {
                database,
                collection,
                aggregations,
                group_by,
                filter,
            } => database::handle_aggregate_columnar(
                self,
                database,
                collection,
                aggregations,
                group_by,
                filter,
            ),

            Command::QueryColumnar {
                database,
                collection,
                columns,
                filter,
                order_by,
                limit,
            } => database::handle_query_columnar(
                self, database, collection, columns, filter, order_by, limit,
            ),

            Command::CreateColumnarIndex {
                database,
                collection,
                column,
            } => database::handle_create_columnar_index(self, database, collection, column),

            Command::ListColumnarIndexes {
                database,
                collection,
            } => database::handle_list_columnar_indexes(self, database, collection),

            Command::DeleteColumnarIndex {
                database,
                collection,
                column,
            } => database::handle_delete_columnar_index(self, database, collection, column),
        }
    }

    /// Helper to get a collection
    pub(crate) fn get_collection(
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
