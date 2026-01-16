use crate::error::{DbError, DbResult};
use crate::sdbql::ast::Query;
use crate::storage::StorageEngine;
use crate::stream::task::StreamTask;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
pub struct StreamDefinition {
    pub name: String,
    pub query: Query,
    pub created_at: i64,
}

pub struct StreamManager {
    storage: Arc<StorageEngine>,
    tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    definitions: Arc<Mutex<HashMap<String, StreamDefinition>>>,
}

impl StreamManager {
    pub fn new(storage: Arc<StorageEngine>) -> Self {
        Self {
            storage,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            definitions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn create_stream(&self, db_name: &str, query: Query) -> DbResult<String> {
        let create_clause = query
            .create_stream_clause
            .as_ref()
            .ok_or(DbError::ExecutionError(
                "Not a CREATE STREAM query".to_string(),
            ))?;

        let name = create_clause.name.clone();

        // 1. Store definition
        let def = StreamDefinition {
            name: name.clone(),
            query: query.clone(),
            created_at: chrono::Utc::now().timestamp(),
        };

        // Check if exists
        {
            let mut defs = self.definitions.lock().unwrap();
            if defs.contains_key(&name) {
                if !create_clause.if_not_exists {
                    return Err(DbError::ExecutionError(format!(
                        "Stream '{}' already exists",
                        name
                    )));
                }
                return Ok(name);
            }
            defs.insert(name.clone(), def);
        }

        // 2. Start task
        if let Err(e) = self.start_stream_task(db_name, &name, query) {
            // Rollback definition on failure
            self.definitions.lock().unwrap().remove(&name);
            return Err(e);
        }

        Ok(name)
    }

    fn start_stream_task(&self, db_name: &str, name: &str, query: Query) -> DbResult<()> {
        let storage = self.storage.clone();

        // Subscribe to collection changes
        // We need to identify the source collection from query
        if query.for_clauses.is_empty() {
            return Err(DbError::ExecutionError(
                "Stream query must have a FOR clause".to_string(),
            ));
        }

        let for_clause = &query.for_clauses[0];
        let collection_name = &for_clause.collection;
        let full_coll_name = format!("{}:{}", db_name, collection_name);

        let collection = storage.get_collection(&full_coll_name)?;
        let rx = collection.change_sender.subscribe();

        let task = StreamTask::new(name.to_string(), query, db_name.to_string(), storage, rx)?;

        let handle = tokio::spawn(async move {
            task.run().await;
        });

        self.tasks.lock().unwrap().insert(name.to_string(), handle);
        tracing::info!(
            "Stream Manager: Started stream '{}' on '{}'",
            name,
            full_coll_name
        );
        Ok(())
    }

    pub fn stop_stream(&self, name: &str) -> DbResult<()> {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(handle) = tasks.remove(name) {
            handle.abort();
            tracing::info!("Stream Manager: Stopped stream '{}'", name);
        }

        self.definitions.lock().unwrap().remove(name);
        Ok(())
    }

    pub fn list_streams(&self) -> Vec<StreamDefinition> {
        self.definitions.lock().unwrap().values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdbql::parser::parse;
    use tempfile::TempDir;

    #[test]
    fn test_create_stream_manager() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(StorageEngine::new(temp_dir.path()).unwrap());
        storage.create_database("test_db".to_string()).unwrap();
        // Assuming create_collection via storage engine (legacy) or database.
        // StorageEngine::create_collection also exists but requires correct naming for some methods.
        // Let's use database handle strictly.
        let db = storage.get_database("test_db").unwrap();
        db.create_collection("events".to_string(), None).unwrap();

        let manager = StreamManager::new(storage);
        let definitions = manager.list_streams();
        assert!(definitions.is_empty());
    }

    #[tokio::test]
    async fn test_create_and_register_stream() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(StorageEngine::new(temp_dir.path()).unwrap());
        storage.create_database("test_db".to_string()).unwrap();
        let db = storage.get_database("test_db").unwrap();
        db.create_collection("events".to_string(), None).unwrap();

        let manager = StreamManager::new(storage);

        let query_str = r#"
            CREATE STREAM high_value_events AS
            FOR e IN events
            FILTER e.amount > 100
            WINDOW TUMBLING (SIZE "1m")
            RETURN e
        "#;

        let query = parse(query_str).expect("Failed to parse query");

        let result = manager.create_stream("test_db", query);
        assert!(
            result.is_ok(),
            "Failed to create stream: {:?}",
            result.err()
        );

        let streams = manager.list_streams();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].name, "high_value_events");

        // Clean up
        manager.stop_stream("high_value_events").unwrap();
        assert!(manager.list_streams().is_empty());
    }
}
