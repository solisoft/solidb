use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

use super::collection::Collection;
use super::database::Database;
use crate::cluster::ClusterConfig;
use crate::error::{DbError, DbResult};
use crate::transaction::manager::TransactionManager;

/// Metadata column family name
const META_CF: &str = "_meta";

/// The main storage engine backed by RocksDB
pub struct StorageEngine {
    /// RocksDB instance wrapped in RwLock for mutability
    db: Arc<RwLock<DB>>,
    /// Database path for reopening
    path: std::path::PathBuf,
    /// Cached collection handles
    collections: Arc<RwLock<HashMap<String, Collection>>>,
    /// Cached database handles
    databases: Arc<RwLock<HashMap<String, Database>>>,
    /// Cluster configuration (if running in cluster mode)
    cluster_config: Option<ClusterConfig>,
    /// Transaction manager (optionally initialized, uses RwLock for interior mutability)
    transaction_manager: RwLock<Option<Arc<TransactionManager>>>,
}

impl Clone for StorageEngine {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            path: self.path.clone(),
            collections: self.collections.clone(),
            databases: self.databases.clone(),
            cluster_config: self.cluster_config.clone(),
            transaction_manager: RwLock::new(self.transaction_manager.read().unwrap().clone()),
        }
    }
}

impl std::fmt::Debug for StorageEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageEngine")
            .field("path", &self.path)
            .finish()
    }
}

impl StorageEngine {
    /// Create a new storage engine
    pub fn new<P: AsRef<Path>>(data_dir: P) -> DbResult<Self> {
        let path = data_dir.as_ref().to_path_buf();

        // Configure RocksDB options
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        
        // Limit WAL file size to prevent unbounded disk growth
        // Max total WAL size across all column families: 50MB
        opts.set_max_total_wal_size(50 * 1024 * 1024);
        
        // Keep fewer LOG files (RocksDB info logs, not WALs)
        opts.set_keep_log_file_num(5);
        
        // Recycle LOG files instead of deleting
        opts.set_recycle_log_file_num(3);

        // Get existing column families or create default
        let cf_names = match DB::list_cf(&opts, &path) {
            Ok(cfs) => cfs,
            Err(_) => vec!["default".to_string()],
        };

        // Ensure META_CF exists
        let mut cf_names: Vec<String> = cf_names.into_iter().collect();
        if !cf_names.contains(&META_CF.to_string()) {
            cf_names.push(META_CF.to_string());
        }

        // Create column family descriptors
        let cf_descriptors: Vec<ColumnFamilyDescriptor> = cf_names
            .iter()
            .map(|name| ColumnFamilyDescriptor::new(name, Options::default()))
            .collect();

        // Open database with column families
        let db = DB::open_cf_descriptors(&opts, &path, cf_descriptors)
            .map_err(|e| DbError::InternalError(format!("Failed to open RocksDB: {}", e)))?;

        Ok(Self {
            db: Arc::new(RwLock::new(db)),
            path,
            collections: Arc::new(RwLock::new(HashMap::new())),
            databases: Arc::new(RwLock::new(HashMap::new())),
            cluster_config: None,
            transaction_manager: RwLock::new(None),
        })
    }

    /// Create a new storage engine with cluster configuration
    pub fn with_cluster_config<P: AsRef<Path>>(
        data_dir: P,
        config: ClusterConfig,
    ) -> DbResult<Self> {
        let mut engine = Self::new(data_dir)?;
        engine.cluster_config = Some(config);
        Ok(engine)
    }

    /// Get the cluster configuration
    pub fn cluster_config(&self) -> Option<&ClusterConfig> {
        self.cluster_config.as_ref()
    }

    /// Check if running in cluster mode
    pub fn is_cluster_mode(&self) -> bool {
        self.cluster_config
            .as_ref()
            .map(|c| c.is_cluster_mode())
            .unwrap_or(false)
    }

    /// Get node ID (returns "standalone" if not in cluster mode)
    pub fn node_id(&self) -> &str {
        self.cluster_config
            .as_ref()
            .map(|c| c.node_id.as_str())
            .unwrap_or("standalone")
    }

    /// Get the data directory path
    pub fn data_dir(&self) -> &str {
        self.path.to_str().unwrap_or("./data")
    }

    /// Initialize the storage engine with default _system database
    pub fn initialize(&self) -> DbResult<()> {
        // Check if _system database exists
        let databases = self.list_databases();
        if !databases.contains(&"_system".to_string()) {
            // Create _system database
            self.create_database("_system".to_string())?;
        }

        // Recalculate document counts for all collections
        // This ensures counts are accurate after crashes or unclean shutdowns
        self.recalculate_all_counts();

        Ok(())
    }

    /// Recalculate document counts for all collections
    /// Called on startup to ensure counts are accurate after potential crashes
    pub fn recalculate_all_counts(&self) {
        let databases = self.list_databases();
        let mut total_collections = 0;

        for db_name in databases {
            if let Ok(database) = self.get_database(&db_name) {
                let collections = database.list_collections();
                for coll_name in collections {
                    if let Ok(collection) = database.get_collection(&coll_name) {
                        collection.recalculate_count();
                        total_collections += 1;
                    }
                }
            }
        }

        if total_collections > 0 {
            tracing::info!(
                "Recalculated document counts for {} collections",
                total_collections
            );
        }
    }

    /// Flush all collection stats to disk
    /// Called on shutdown to ensure counts are persisted
    pub fn flush_all_stats(&self) {
        let databases = self.list_databases();

        for db_name in databases {
            if let Ok(database) = self.get_database(&db_name) {
                let collections = database.list_collections();
                for coll_name in collections {
                    if let Ok(collection) = database.get_collection(&coll_name) {
                        collection.flush_stats();
                    }
                }
            }
        }

        // Also flush RocksDB
        let _ = self.flush();
        tracing::info!("Flushed all collection stats to disk");
    }

    // ==================== Database Operations ====================

    /// Create a new database
    pub fn create_database(&self, name: String) -> DbResult<()> {
        // Validate database name
        if name.is_empty() || name.contains(':') {
            return Err(DbError::InvalidDocument(
                "Invalid database name".to_string(),
            ));
        }

        // Check if database already exists by looking for any collection with this prefix
        let existing_dbs = self.list_databases();
        if existing_dbs.contains(&name) {
            return Err(DbError::CollectionAlreadyExists(format!(
                "Database '{}' already exists",
                name
            )));
        }

        // Store database metadata
        let mut db = self.db.write().unwrap();
        let meta_cf = db.cf_handle(META_CF).expect("META_CF should exist");
        let db_key = format!("db:{}", name);
        db.put_cf(meta_cf, db_key.as_bytes(), b"1")
            .map_err(|e| DbError::InternalError(format!("Failed to create database: {}", e)))?;

        Ok(())
    }

    /// Delete a database and all its collections
    pub fn delete_database(&self, name: &str) -> DbResult<()> {
        // Prevent deletion of _system database
        if name == "_system" {
            return Err(DbError::InvalidDocument(
                "Cannot delete _system database".to_string(),
            ));
        }

        // Get database to ensure it exists
        let database = self.get_database(name)?;

        // Delete all collections in the database
        let collections = database.list_collections();
        for collection_name in collections {
            database.delete_collection(&collection_name)?;
        }

        // Remove database metadata
        let mut db = self.db.write().unwrap();
        let meta_cf = db.cf_handle(META_CF).expect("META_CF should exist");
        let db_key = format!("db:{}", name);
        db.delete_cf(meta_cf, db_key.as_bytes())
            .map_err(|e| DbError::InternalError(format!("Failed to delete database: {}", e)))?;

        // Remove from cache
        {
            let mut cache = self.databases.write().unwrap();
            cache.remove(name);
        }

        Ok(())
    }

    /// List all databases
    pub fn list_databases(&self) -> Vec<String> {
        let db = self.db.read().unwrap();
        let meta_cf = match db.cf_handle(META_CF) {
            Some(cf) => cf,
            None => return vec![],
        };

        let prefix = b"db:";
        let iter = db.prefix_iterator_cf(meta_cf, prefix);

        iter.filter_map(|result| {
            result.ok().and_then(|(key, _)| {
                let key_str = String::from_utf8(key.to_vec()).ok()?;
                key_str.strip_prefix("db:").map(|s| s.to_string())
            })
        })
        .collect()
    }

    /// Get a database handle (cached for consistent collection counters)
    pub fn get_database(&self, name: &str) -> DbResult<Database> {
        // Check cache first
        {
            let cache = self.databases.read().unwrap();
            if let Some(database) = cache.get(name) {
                return Ok(database.clone());
            }
        }

        // Verify database exists
        let databases = self.list_databases();
        if !databases.contains(&name.to_string()) {
            return Err(DbError::CollectionNotFound(format!(
                "Database '{}' not found",
                name
            )));
        }

        // Create and cache the database
        let database = Database::new(name.to_string(), self.db.clone());
        {
            let mut cache = self.databases.write().unwrap();
            cache.insert(name.to_string(), database.clone());
        }

        Ok(database)
    }

    // ==================== Legacy Collection Operations (for backward compatibility) ====================

    /// Create a new collection (column family)
    pub fn create_collection(&self, name: String, collection_type: Option<String>) -> DbResult<()> {
        let mut db = self.db.write().unwrap();

        // Check if collection already exists
        if db.cf_handle(&name).is_some() {
            return Err(DbError::CollectionAlreadyExists(name));
        }
        
        // Default to "document" if not specified
        let type_ = collection_type.unwrap_or_else(|| "document".to_string());

        // Create the column family
        let opts = Options::default();
        db.create_cf(&name, &opts)
            .map_err(|e| DbError::InternalError(format!("Failed to create collection: {}", e)))?;
            
        // Persist collection type
        if let Some(cf) = db.cf_handle(&name) {
            db.put_cf(cf, "_stats:type".as_bytes(), type_.as_bytes())
                .map_err(|e| DbError::InternalError(format!("Failed to set collection type: {}", e)))?;
        }

        Ok(())
    }

    /// Get a collection (legacy method - checks both database-prefixed and plain names)
    /// Uses cached collection handles for performance
    pub fn get_collection(&self, name: &str) -> DbResult<Collection> {
        // Check cache first
        {
            let cache = self.collections.read().unwrap();
            if let Some(collection) = cache.get(name) {
                return Ok(collection.clone());
            }
        }

        let db = self.db.read().unwrap();

        // First, try the exact name (for backward compatibility or direct access)
        let actual_name = if db.cf_handle(name).is_some() {
            name.to_string()
        } else {
            // If not found, try prefixing with _system database
            let system_name = format!("_system:{}", name);
            if db.cf_handle(&system_name).is_some() {
                system_name
            } else {
                // Not found in either format
                return Err(DbError::CollectionNotFound(name.to_string()));
            }
        };
        drop(db);

        // Create and cache the collection
        let collection = Collection::new(actual_name.clone(), self.db.clone());
        {
            let mut cache = self.collections.write().unwrap();
            cache.insert(name.to_string(), collection.clone());
            if actual_name != name {
                cache.insert(actual_name, collection.clone());
            }
        }

        Ok(collection)
    }

    /// Delete a collection
    pub fn delete_collection(&self, name: &str) -> DbResult<()> {
        let mut db = self.db.write().unwrap();

        if db.cf_handle(name).is_none() {
            return Err(DbError::CollectionNotFound(name.to_string()));
        }

        db.drop_cf(name)
            .map_err(|e| DbError::InternalError(format!("Failed to delete collection: {}", e)))?;

        Ok(())
    }

    /// List all collection names
    pub fn list_collections(&self) -> Vec<String> {
        // Get all column family names, excluding internal ones
        DB::list_cf(&Options::default(), &self.path)
            .unwrap_or_default()
            .into_iter()
            .filter(|name| name != "default" && name != META_CF)
            .collect()
    }

    /// Save a collection - no-op with RocksDB (auto-persisted)
    pub fn save_collection(&self, _name: &str) -> DbResult<()> {
        // RocksDB automatically persists data, nothing to do
        Ok(())
    }

    /// Flush all pending writes to disk
    pub fn flush(&self) -> DbResult<()> {
        let db = self.db.read().unwrap();
        db.flush()
            .map_err(|e| DbError::InternalError(format!("Failed to flush: {}", e)))?;
        Ok(())
    }

    // ==================== Transaction Operations ====================

    /// Initialize transaction manager (call once on startup if transactions are needed)
    pub fn initialize_transactions(&self) -> DbResult<()> {
        // Check if already initialized (read lock first)
        {
            let tx_mgr = self.transaction_manager.read().unwrap();
            if tx_mgr.is_some() {
                return Ok(()); // Already initialized
            }
        }

        let wal_path = self.path.join("transaction.wal");
        
        // Recover any committed transactions from WAL BEFORE creating manager
        // This ensures we don't double-apply on restart
        self.recover_transactions()?;
        
        let manager = TransactionManager::new(wal_path)?;

        // Now acquire write lock to store manager
        {
            let mut tx_mgr = self.transaction_manager.write().unwrap();
            *tx_mgr = Some(Arc::new(manager));
        }

        tracing::info!("Transaction manager initialized");
        Ok(())
    }

    /// Get transaction manager (initializes if needed)
    pub fn transaction_manager(&self) -> DbResult<Arc<TransactionManager>> {
        // Try to read first
        {
            let tx_mgr = self.transaction_manager.read().unwrap();
            if let Some(ref manager) = *tx_mgr {
                return Ok(manager.clone());
            }
        }
        
        // Not initialized, so initialize it
        self.initialize_transactions()?;
        
        // Read again after initialization
        let tx_mgr = self.transaction_manager.read().unwrap();
        Ok(tx_mgr.as_ref().unwrap().clone())
    }

    /// Recover committed transactions from WAL (called on startup)
    fn recover_transactions(&self) -> DbResult<()> {
        use crate::transaction::wal::WalReader;

        let wal_path = self.path.join("transaction.wal");
        if !wal_path.exists() {
            return Ok(()); // No WAL to recover
        }

        let reader = WalReader::new(&wal_path);
        let committed_txs = reader.replay()?;

        if committed_txs.is_empty() {
            return Ok(());
        }

        tracing::info!("Recovering {} committed transactions from WAL", committed_txs.len());

        // Apply each committed transaction
        for tx in committed_txs {
            // Group operations by collection
            let mut ops_by_collection: HashMap<String, Vec<crate::transaction::Operation>> =
                HashMap::new();

            for op in tx.operations {
                let coll_name = format!("{}:{}", op.database(), op.collection());
                ops_by_collection
                    .entry(coll_name)
                    .or_insert_with(Vec::new)
                    .push(op);
            }

            // Apply operations for each collection
            for (coll_name, ops) in ops_by_collection {
                if let Ok(collection) = self.get_collection(&coll_name) {
                    collection.apply_transaction_operations(&ops)?;
                } else {
                    tracing::warn!(
                        "Collection {} not found during WAL recovery, skipping",
                        coll_name
                    );
                }
            }
        }

        tracing::info!("Transaction recovery complete");
        Ok(())
    }

    /// Commit a transaction by applying all operations atomically
    pub fn commit_transaction(
        &self,
        tx_id: crate::transaction::TransactionId,
    ) -> DbResult<()> {
        let manager = {
            let tx_mgr = self.transaction_manager.read().unwrap();
            tx_mgr
                .as_ref()
                .ok_or_else(|| {
                    DbError::InternalError("Transaction manager not initialized".to_string())
                })?
                .clone()
        };

        // Get transaction
        let tx_arc = manager.get(tx_id)?;
        let operations = {
            let tx = tx_arc.read().unwrap();
            tx.operations.clone()
        };

        // Group operations by collection
        let mut ops_by_collection: HashMap<String, Vec<crate::transaction::Operation>> =
            HashMap::new();

        for op in operations {
            let coll_name = format!("{}:{}", op.database(), op.collection());
            ops_by_collection
                .entry(coll_name)
                .or_insert_with(Vec::new)
                .push(op);
        }

        // Apply operations for each collection
        for (coll_name, ops) in ops_by_collection {
            let collection = self.get_collection(&coll_name)?;
            collection.apply_transaction_operations(&ops)?;
        }

        // Mark transaction as committed in manager
        manager.commit(tx_id)?;

        Ok(())
    }

    /// Rollback a transaction (operations already in WAL as aborted)
    pub fn rollback_transaction(
        &self,
        tx_id: crate::transaction::TransactionId,
    ) -> DbResult<()> {
        let manager = {
            let tx_mgr = self.transaction_manager.read().unwrap();
            tx_mgr
                .as_ref()
                .ok_or_else(|| {
                    DbError::InternalError("Transaction manager not initialized".to_string())
                })?
                .clone()
        };

        // Just mark as aborted - operations were never applied
        manager.rollback(tx_id)?;

        Ok(())
    }
}
