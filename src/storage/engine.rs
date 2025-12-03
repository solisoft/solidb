use std::path::Path;
use std::sync::{Arc, RwLock};
use rocksdb::{DB, Options, ColumnFamilyDescriptor};

use crate::error::{DbError, DbResult};
use super::collection::Collection;
use super::database::Database;

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

    /// Initialize the storage engine with default _system database
    pub fn initialize(&self) -> DbResult<()> {
        // Check if _system database exists
        let databases = self.list_databases();
        if !databases.contains(&"_system".to_string()) {
            // Create _system database
            self.create_database("_system".to_string())?;
        }
        Ok(())
    }

    // ==================== Database Operations ====================

    /// Create a new database
    pub fn create_database(&self, name: String) -> DbResult<()> {
        // Validate database name
        if name.is_empty() || name.contains(':') {
            return Err(DbError::InvalidDocument("Invalid database name".to_string()));
        }

        // Check if database already exists by looking for any collection with this prefix
        let existing_dbs = self.list_databases();
        if existing_dbs.contains(&name) {
            return Err(DbError::CollectionAlreadyExists(format!("Database '{}' already exists", name)));
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
            return Err(DbError::InvalidDocument("Cannot delete _system database".to_string()));
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

    /// Get a database handle
    pub fn get_database(&self, name: &str) -> DbResult<Database> {
        let databases = self.list_databases();
        if !databases.contains(&name.to_string()) {
            return Err(DbError::CollectionNotFound(format!("Database '{}' not found", name)));
        }

        Ok(Database::new(name.to_string(), self.db.clone()))
    }

    // ==================== Legacy Collection Operations (for backward compatibility) ====================

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

    /// Get a collection (legacy method - checks both database-prefixed and plain names)
    pub fn get_collection(&self, name: &str) -> DbResult<Collection> {
        let db = self.db.read().unwrap();

        // First, try the exact name (for backward compatibility or direct access)
        if db.cf_handle(name).is_some() {
            drop(db);
            return Ok(Collection::new(name.to_string(), self.db.clone()));
        }

        // If not found, try prefixing with _system database
        let system_name = format!("_system:{}", name);
        if db.cf_handle(&system_name).is_some() {
            drop(db);
            return Ok(Collection::new(system_name, self.db.clone()));
        }

        // Not found in either format
        Err(DbError::CollectionNotFound(name.to_string()))
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
