use std::path::Path;
use std::sync::{Arc, RwLock};
use rocksdb::{DB, Options, ColumnFamilyDescriptor};

use crate::error::{DbError, DbResult};
use super::collection::Collection;

/// Metadata column family name
const META_CF: &str = "_meta";

/// The main storage engine backed by RocksDB
#[derive(Clone)]
pub struct StorageEngine {
    /// RocksDB instance wrapped in RwLock for mutability
    db: Arc<RwLock<DB>>,
    /// Database path for reopening
    path: std::path::PathBuf,
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
        })
    }

    /// Create a new collection (column family)
    pub fn create_collection(&self, name: String) -> DbResult<()> {
        let mut db = self.db.write().unwrap();

        // Check if collection already exists
        if db.cf_handle(&name).is_some() {
            return Err(DbError::CollectionAlreadyExists(name));
        }

        // Create the column family
        let opts = Options::default();
        db.create_cf(&name, &opts)
            .map_err(|e| DbError::InternalError(format!("Failed to create collection: {}", e)))?;

        Ok(())
    }

    /// Get a collection
    pub fn get_collection(&self, name: &str) -> DbResult<Collection> {
        let db = self.db.read().unwrap();

        // Check if column family exists
        if db.cf_handle(name).is_none() {
            return Err(DbError::CollectionNotFound(name.to_string()));
        }
        drop(db);

        Ok(Collection::new(name.to_string(), self.db.clone()))
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
}
