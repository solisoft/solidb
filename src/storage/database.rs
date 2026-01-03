use rocksdb::DB;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::collection::Collection;
use crate::error::{DbError, DbResult};

/// Represents a database that contains multiple collections
#[derive(Clone)]
pub struct Database {
    /// Database name
    pub name: String,
    /// RocksDB instance
    db: Arc<RwLock<DB>>,
    /// Cached collection handles
    collections: Arc<RwLock<HashMap<String, Collection>>>,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("name", &self.name)
            .finish()
    }
}

impl Database {
    /// Create a new database handle
    pub fn new(name: String, db: Arc<RwLock<DB>>) -> Self {
        Self {
            name,
            db,
            collections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new collection in this database
    pub fn create_collection(&self, collection_name: String, collection_type: Option<String>) -> DbResult<()> {
        let cf_name = self.collection_cf_name(&collection_name);
        
        // Default to "document" if not specified
        let type_ = collection_type.unwrap_or_else(|| "document".to_string());
        
        let mut db = self.db.write().unwrap();

        // Check if collection already exists
        if db.cf_handle(&cf_name).is_some() {
            return Err(DbError::CollectionAlreadyExists(collection_name));
        }

        // Create column family
        db.create_cf(&cf_name, &rocksdb::Options::default())
            .map_err(|e| DbError::InternalError(format!("Failed to create collection: {}", e)))?;
            
        // Persist collection type
        if let Some(cf) = db.cf_handle(&cf_name) {
            db.put_cf(cf, "_stats:type".as_bytes(), type_.as_bytes())
                .map_err(|e| DbError::InternalError(format!("Failed to set collection type: {}", e)))?;
        }

        Ok(())
    }

    /// Delete a collection from this database
    pub fn delete_collection(&self, collection_name: &str) -> DbResult<()> {
        let cf_name = self.collection_cf_name(collection_name);
        let mut db = self.db.write().unwrap();

        // Check if collection exists
        if db.cf_handle(&cf_name).is_none() {
            return Err(DbError::CollectionNotFound(collection_name.to_string()));
        }

        // Drop column family
        db.drop_cf(&cf_name)
            .map_err(|e| DbError::InternalError(format!("Failed to delete collection: {}", e)))?;

        // Remove from cache
        {
            let mut cache = self.collections.write().unwrap();
            cache.remove(collection_name);
        }

        Ok(())
    }

    /// List all collections in this database
    pub fn list_collections(&self) -> Vec<String> {
        let db = self.db.read().unwrap();
        let prefix = format!("{}:", self.name);

        // Iterate through all column families
        let mut collections = Vec::new();
        for cf_name in DB::list_cf(&rocksdb::Options::default(), db.path()).unwrap_or_default() {
            if cf_name.starts_with(&prefix) {
                if let Some(name) = cf_name.strip_prefix(&prefix) {
                    collections.push(name.to_string());
                }
            }
        }
        collections
    }

    /// Get a collection handle (cached for consistent atomic counters)
    pub fn get_collection(&self, collection_name: &str) -> DbResult<Collection> {
        // Check cache first
        {
            let cache = self.collections.read().unwrap();
            if let Some(collection) = cache.get(collection_name) {
                return Ok(collection.clone());
            }
        }

        let cf_name = self.collection_cf_name(collection_name);

        // Check if collection exists
        {
            let db = self.db.read().unwrap();
            if db.cf_handle(&cf_name).is_none() {
                return Err(DbError::CollectionNotFound(collection_name.to_string()));
            }
        }

        // Create and cache the collection
        let collection = Collection::new(cf_name, self.db.clone());
        {
            let mut cache = self.collections.write().unwrap();
            cache.insert(collection_name.to_string(), collection.clone());
        }

        Ok(collection)
    }

    /// Generate column family name for a collection
    fn collection_cf_name(&self, collection_name: &str) -> String {
        format!("{}:{}", self.name, collection_name)
    }

    /// Get the underlying RocksDB Arc for advanced operations
    pub fn db_arc(&self) -> Arc<RwLock<DB>> {
        self.db.clone()
    }

    /// Generate column family name for a columnar collection
    fn columnar_cf_name(&self, collection_name: &str) -> String {
        format!("{}:_columnar_{}", self.name, collection_name)
    }

    /// Check if a collection is a columnar collection
    pub fn is_columnar_collection(&self, collection_name: &str) -> bool {
        let cf_name = self.columnar_cf_name(collection_name);
        let db = self.db.read().unwrap();
        db.cf_handle(&cf_name).is_some()
    }

    /// List all columnar collections in this database
    /// Note: This scans metadata keys to find columnar collections
    pub fn list_columnar_collections(&self) -> Vec<String> {
        // Columnar collections store their metadata in a special format
        // We scan for the metadata key pattern: {db}:_columnar_{name}/meta
        // For now, return empty - columnar collections are tracked separately
        // through the columnar handlers which maintain their own list
        vec![]
    }

    /// Get the database name
    pub fn db_name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_db() -> (Arc<RwLock<DB>>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db = DB::open_default(temp_dir.path()).unwrap();
        (Arc::new(RwLock::new(db)), temp_dir)
    }

    #[test]
    fn test_create_collection() {
        let (db, _dir) = create_test_db();
        let database = Database::new("testdb".to_string(), db);

        assert!(database.create_collection("users".to_string(), None).is_ok());
        assert!(database.list_collections().contains(&"users".to_string()));
    }

    #[test]
    fn test_create_duplicate_collection() {
        let (db, _dir) = create_test_db();
        let database = Database::new("testdb".to_string(), db);

        database.create_collection("users".to_string(), None).unwrap();
        assert!(database.create_collection("users".to_string(), None).is_err());
    }

    #[test]
    fn test_delete_collection() {
        let (db, _dir) = create_test_db();
        let database = Database::new("testdb".to_string(), db);

        database.create_collection("users".to_string(), None).unwrap();
        assert!(database.delete_collection("users").is_ok());
        assert!(!database.list_collections().contains(&"users".to_string()));
    }

    #[test]
    fn test_list_collections() {
        let (db, _dir) = create_test_db();
        let database = Database::new("testdb".to_string(), db);

        database.create_collection("users".to_string(), None).unwrap();
        database.create_collection("products".to_string(), None).unwrap();

        let collections = database.list_collections();
        assert_eq!(collections.len(), 2);
        assert!(collections.contains(&"users".to_string()));
        assert!(collections.contains(&"products".to_string()));
    }
}
