//! Sync Manager
//!
//! Coordinates offline-first synchronization between local storage and server.
//! Handles conflict resolution, network status, and background sync.

use crate::client::HttpClient;
use crate::sync::store::LocalStore;
use chrono::Utc;
use serde_json::Value;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// Configuration for sync manager
#[derive(Clone, Debug)]
pub struct SyncConfig {
    /// Sync interval when online
    pub sync_interval_secs: u64,
    /// Maximum number of changes per batch
    pub batch_size: usize,
    /// Maximum retry attempts for failed changes
    pub max_retries: u32,
    /// Enable automatic background sync
    pub auto_sync: bool,
    /// Collections to sync (empty = all)
    pub collections: Vec<String>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            sync_interval_secs: 30,
            batch_size: 100,
            max_retries: 5,
            auto_sync: true,
            collections: vec![],
        }
    }
}

/// Sync manager for offline-first synchronization
pub struct SyncManager {
    /// Local SQLite storage
    local_store: Arc<Mutex<LocalStore>>,
    /// HTTP client for server communication
    client: Arc<HttpClient>,
    /// Sync configuration
    config: SyncConfig,
    /// Current sync state
    state: Arc<RwLock<SyncState>>,
    /// Channel for sync commands
    command_tx: Option<mpsc::Sender<SyncCommand>>,
    /// Is currently syncing?
    is_syncing: Arc<RwLock<bool>>,
    /// Is online?
    is_online: Arc<RwLock<bool>>,
}

/// Current sync state
#[derive(Clone, Debug)]
struct SyncState {
    /// Last successful sync timestamp
    last_sync_at: Option<i64>,
    /// Number of pending changes
    pending_count: usize,
    /// Current sync session ID
    #[allow(dead_code)]
    session_id: Option<String>,
    /// Collections being synced
    active_collections: Vec<String>,
}

/// Commands that can be sent to the sync manager
#[derive(Debug)]
pub enum SyncCommand {
    /// Trigger a sync now
    SyncNow,
    /// Subscribe to a collection
    Subscribe {
        collection: String,
        filter: Option<String>,
    },
    /// Unsubscribe from a collection
    Unsubscribe { collection: String },
    /// Set online/offline status
    SetOnline(bool),
    /// Stop the sync manager
    Stop,
}

/// Result of a sync operation
#[derive(Debug, Clone)]
pub struct SyncResult {
    pub success: bool,
    pub pulled: usize,
    pub pushed: usize,
    pub conflicts: usize,
    pub errors: Vec<String>,
}

impl SyncManager {
    /// Create a new sync manager
    pub fn new(local_store: LocalStore, client: HttpClient, config: SyncConfig) -> Self {
        let state = SyncState {
            last_sync_at: None,
            pending_count: 0,
            session_id: None,
            active_collections: config.collections.clone(),
        };

        Self {
            local_store: Arc::new(Mutex::new(local_store)),
            client: Arc::new(client),
            config,
            state: Arc::new(RwLock::new(state)),
            command_tx: None,
            is_syncing: Arc::new(RwLock::new(false)),
            is_online: Arc::new(RwLock::new(true)),
        }
    }

    /// Start the sync manager with background task
    pub async fn start(&mut self) -> mpsc::Sender<SyncCommand> {
        let (tx, mut rx) = mpsc::channel(100);
        self.command_tx = Some(tx.clone());

        let local_store = self.local_store.clone();
        let client = self.client.clone();
        let config = self.config.clone();
        let state = self.state.clone();
        let is_syncing = self.is_syncing.clone();
        let is_online = self.is_online.clone();

        // Spawn background sync task
        tokio::spawn(async move {
            let mut sync_interval = interval(Duration::from_secs(config.sync_interval_secs));

            loop {
                tokio::select! {
                    // Periodic sync
                    _ = sync_interval.tick(), if config.auto_sync => {
                        let online = *is_online.read().await;
                        if online {
                            let mut syncing = is_syncing.write().await;
                            if !*syncing {
                                *syncing = true;
                                drop(syncing);

                                    debug!("Starting periodic sync");
                                let mut store = local_store.lock().await;
                                let result = Self::do_sync(&mut store, &client, &config).await;
                                drop(store);

                                if let Ok(result) = result {
                                    let mut s = state.write().await;
                                    if result.success {
                                        s.last_sync_at = Some(Utc::now().timestamp_millis());
                                    }
                                    s.pending_count = 0;
                                }

                                *is_syncing.write().await = false;
                            }
                        }
                    }

                    // Handle commands
                    Some(cmd) = rx.recv() => {
                        match cmd {
                            SyncCommand::SyncNow => {
                                let mut syncing = is_syncing.write().await;
                                if !*syncing {
                                    *syncing = true;
                                    drop(syncing);

                                    info!("Manual sync triggered");
                                    let mut store = local_store.lock().await;
                                    let result = Self::do_sync(&mut store, &client, &config).await;
                                drop(store);

                                    if let Ok(result) = result {
                                        let mut s = state.write().await;
                                        if result.success {
                                            s.last_sync_at = Some(Utc::now().timestamp_millis());
                                        }
                                    }

                                    *is_syncing.write().await = false;
                                }
                            }
                            SyncCommand::Subscribe { collection, filter } => {
                                let mut store = local_store.lock().await;
                                if let Err(e) = store.subscribe_collection(&collection, filter.as_deref()) {
                                    error!("Failed to subscribe: {}", e);
                                } else {
                                    let mut s = state.write().await;
                                    if !s.active_collections.contains(&collection) {
                                        s.active_collections.push(collection.clone());
                                    }
                                    info!("Subscribed to collection: {}", collection);
                                }
                            }
                            SyncCommand::Unsubscribe { collection } => {
                                let mut store = local_store.lock().await;
                                if let Err(e) = store.unsubscribe_collection(&collection) {
                                    error!("Failed to unsubscribe: {}", e);
                                } else {
                                    let mut s = state.write().await;
                                    s.active_collections.retain(|c| c != &collection);
                                    info!("Unsubscribed from collection: {}", collection);
                                }
                            }
                            SyncCommand::SetOnline(online) => {
                                let mut is_online_guard = is_online.write().await;
                                *is_online_guard = online;
                                if online {
                                    info!("Going online - will sync");
                                } else {
                                    warn!("Going offline - queueing changes");
                                }
                            }
                            SyncCommand::Stop => {
                                info!("Sync manager stopping");
                                break;
                            }
                        }
                    }
                }
            }
        });

        tx
    }

    /// Perform a sync operation
    async fn do_sync(
        store: &mut LocalStore,
        _client: &HttpClient,
        _config: &SyncConfig,
    ) -> Result<SyncResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = SyncResult {
            success: false,
            pulled: 0,
            pushed: 0,
            conflicts: 0,
            errors: vec![],
        };

        // Step 1: Push pending changes to server
        let pending = store.get_pending_changes()?;
        if !pending.is_empty() {
            debug!("Pushing {} pending changes", pending.len());

            for change in pending {
                // TODO: Actually push to server via HTTP API
                // For now, simulate success
                debug!(
                    "Would push: {} {} in {}",
                    change.operation, change.document_key, change.collection
                );

                // Remove from pending after successful push
                store.remove_pending_change(change.id)?;
                result.pushed += 1;
            }
        }

        // Step 2: Pull changes from server
        // TODO: Implement actual server pull
        // For now, simulate success
        debug!("Would pull changes from server");
        result.pulled = 0;

        // Step 3: Handle conflicts
        // TODO: Check for conflicts and resolve
        result.conflicts = 0;

        result.success = result.errors.is_empty();
        Ok(result)
    }

    /// Trigger a manual sync
    pub async fn sync_now(&self) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(SyncCommand::SyncNow).await;
        }
    }

    /// Subscribe to a collection
    pub async fn subscribe(&self, collection: &str, filter: Option<&str>) {
        if let Some(tx) = &self.command_tx {
            let _ = tx
                .send(SyncCommand::Subscribe {
                    collection: collection.to_string(),
                    filter: filter.map(|s| s.to_string()),
                })
                .await;
        }
    }

    /// Unsubscribe from a collection
    pub async fn unsubscribe(&self, collection: &str) {
        if let Some(tx) = &self.command_tx {
            let _ = tx
                .send(SyncCommand::Unsubscribe {
                    collection: collection.to_string(),
                })
                .await;
        }
    }

    /// Set online/offline status
    pub async fn set_online(&self, online: bool) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(SyncCommand::SetOnline(online)).await;
        }
    }

    /// Get current sync state
    pub async fn get_state(&self) -> (Option<i64>, usize) {
        let state = self.state.read().await;
        (state.last_sync_at, state.pending_count)
    }

    /// Stop the sync manager
    pub async fn stop(&self) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(SyncCommand::Stop).await;
        }
    }

    /// Save a document locally (will be synced when online)
    pub async fn save_document(
        &self,
        collection: &str,
        key: &str,
        data: &Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut store = self.local_store.lock().await;

        // Generate version vector
        let device_id = store.device_id().to_string();
        let vector = format!("{{\"{}\": {}}}", device_id, Utc::now().timestamp_millis());

        // Save locally
        store.put_document(collection, key, data, &vector)?;

        // Add to pending changes
        store.add_pending_change(
            collection,
            key,
            if data.get("_key").is_some() {
                "UPDATE"
            } else {
                "INSERT"
            },
            Some(data),
            &vector,
        )?;

        // Update pending count in state
        let pending = store.get_pending_changes()?;
        drop(store);

        let mut state = self.state.write().await;
        state.pending_count = pending.len();

        debug!("Document saved locally: {} in {}", key, collection);
        Ok(())
    }

    /// Get a document from local storage
    pub async fn get_document(
        &self,
        collection: &str,
        key: &str,
    ) -> Result<Option<Value>, Box<dyn std::error::Error + Send + Sync>> {
        let store = self.local_store.lock().await;
        match store.get_document(collection, key)? {
            Some((data, _)) => Ok(Some(data)),
            None => Ok(None),
        }
    }

    /// Delete a document locally
    pub async fn delete_document(
        &self,
        collection: &str,
        key: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut store = self.local_store.lock().await;

        let device_id = store.device_id().to_string();
        let vector = format!("{{\"{}\": {}}}", device_id, Utc::now().timestamp_millis());

        // Mark as deleted locally
        store.delete_document(collection, key, &vector)?;

        // Add to pending changes
        store.add_pending_change(collection, key, "DELETE", None, &vector)?;

        let pending = store.get_pending_changes()?;
        drop(store);

        let mut state = self.state.write().await;
        state.pending_count = pending.len();

        debug!("Document marked for deletion: {} in {}", key, collection);
        Ok(())
    }

    /// Query local documents (simple implementation)
    pub async fn query_documents(
        &self,
        collection: &str,
    ) -> Result<Vec<(String, Value)>, Box<dyn std::error::Error + Send + Sync>> {
        let store = self.local_store.lock().await;
        let docs = store.list_documents(collection)?;

        let result = docs.into_iter().map(|(key, data, _)| (key, data)).collect();

        Ok(result)
    }

    /// Execute an SDBQL query against the local store.
    ///
    /// This allows full query language support for offline queries,
    /// including filtering, sorting, joining, and aggregations.
    ///
    /// # Arguments
    /// * `query` - SDBQL query string
    /// * `bind_vars` - Optional bind variables for parameterized queries
    ///
    /// # Example
    /// ```rust,ignore
    /// let results = sync_manager.query(
    ///     "FOR u IN users FILTER u.age > @min_age SORT u.name RETURN u",
    ///     Some(HashMap::from([("min_age".to_string(), json!(18))])),
    /// ).await?;
    /// ```
    pub async fn query(
        &self,
        query: &str,
        bind_vars: Option<std::collections::HashMap<String, Value>>,
    ) -> Result<Vec<Value>, Box<dyn std::error::Error + Send + Sync>> {
        use super::local_data_source::SqliteDataSource;
        use sdbql_core::LocalExecutor;

        let store = self.local_store.lock().await;
        let data_source = SqliteDataSource::new(&store);
        let executor = LocalExecutor::new(data_source);

        let results = executor.execute(query, bind_vars)?;
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use crate::sync::store::LocalStore;
    use std::path::PathBuf;

    #[allow(dead_code)]
    fn create_test_store() -> LocalStore {
        let temp_path = PathBuf::from(format!("/tmp/test_sync_{}.db", uuid::Uuid::new_v4()));
        LocalStore::open(&temp_path, "test-device".to_string()).unwrap()
    }

    // Note: These tests would require mocking the HttpClient
    // For now, we just test the structure compiles
}
