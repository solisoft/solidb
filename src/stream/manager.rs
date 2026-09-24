use crate::error::{DbError, DbResult};
use crate::sdbql::ast::Query;
use crate::sdbql::QueryPrincipal;
use crate::storage::StorageEngine;
use crate::stream::task::StreamTask;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
pub struct StreamDefinition {
    pub name: String,
    /// Database the stream belongs to. Audit H6: streams used to be keyed by
    /// name alone, so any database's script could list or stop them all.
    pub database: String,
    pub query: Query,
    pub created_at: i64,
    /// The principal the stream was created by, and runs as. `None` runs as
    /// an anonymous non-admin client, never as the server.
    pub principal: Option<QueryPrincipal>,
}

/// Streams are identified by (database, name).
type StreamKey = (String, String);

fn key(db_name: &str, name: &str) -> StreamKey {
    (db_name.to_string(), name.to_string())
}

pub struct StreamManager {
    storage: Arc<StorageEngine>,
    tasks: Arc<Mutex<HashMap<StreamKey, JoinHandle<()>>>>,
    definitions: Arc<Mutex<HashMap<StreamKey, StreamDefinition>>>,
}

impl StreamManager {
    pub fn new(storage: Arc<StorageEngine>) -> Self {
        Self {
            storage,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            definitions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a stream with no recorded creator. It runs as an anonymous
    /// non-admin principal (fail closed); prefer [`Self::create_stream_as`].
    pub fn create_stream(&self, db_name: &str, query: Query) -> DbResult<String> {
        self.create_stream_inner(db_name, query, None)
    }

    /// Create a stream that evaluates its query as `principal`.
    pub fn create_stream_as(
        &self,
        db_name: &str,
        query: Query,
        principal: QueryPrincipal,
    ) -> DbResult<String> {
        self.create_stream_inner(db_name, query, Some(principal))
    }

    fn create_stream_inner(
        &self,
        db_name: &str,
        query: Query,
        principal: Option<QueryPrincipal>,
    ) -> DbResult<String> {
        let create_clause = query
            .create_stream_clause
            .as_ref()
            .ok_or(DbError::ExecutionError(
                "Not a CREATE STREAM query".to_string(),
            ))?;

        let name = create_clause.name.clone();
        let k = key(db_name, &name);

        // 1. Store definition
        let def = StreamDefinition {
            name: name.clone(),
            database: db_name.to_string(),
            query: query.clone(),
            created_at: chrono::Utc::now().timestamp(),
            principal: principal.clone(),
        };

        // Check if exists
        {
            let mut defs = self.definitions.lock().unwrap();
            if defs.contains_key(&k) {
                if !create_clause.if_not_exists {
                    return Err(DbError::ExecutionError(format!(
                        "Stream '{}' already exists",
                        name
                    )));
                }
                return Ok(name);
            }
            defs.insert(k.clone(), def);
        }

        // 2. Start task
        let run_as = principal.unwrap_or_else(QueryPrincipal::anonymous);
        if let Err(e) = self.start_stream_task(db_name, &name, query, run_as) {
            // Rollback definition on failure
            self.definitions.lock().unwrap().remove(&k);
            return Err(e);
        }

        Ok(name)
    }

    fn start_stream_task(
        &self,
        db_name: &str,
        name: &str,
        query: Query,
        principal: QueryPrincipal,
    ) -> DbResult<()> {
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

        let task = StreamTask::new(
            name.to_string(),
            query,
            db_name.to_string(),
            storage,
            rx,
            principal,
        )?;

        let handle = tokio::spawn(async move {
            task.run().await;
        });

        self.tasks
            .lock()
            .unwrap()
            .insert(key(db_name, name), handle);
        tracing::info!(
            "Stream Manager: Started stream '{}' on '{}'",
            name,
            full_coll_name
        );
        Ok(())
    }

    /// Stop a stream of `db_name`. A stream of another database with the same
    /// name is untouched.
    pub fn stop_stream(&self, db_name: &str, name: &str) -> DbResult<()> {
        let k = key(db_name, name);
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(handle) = tasks.remove(&k) {
            handle.abort();
            tracing::info!("Stream Manager: Stopped stream '{}' in '{}'", name, db_name);
        }

        self.definitions.lock().unwrap().remove(&k);
        Ok(())
    }

    /// Streams defined in `db_name`.
    pub fn list_streams(&self, db_name: &str) -> Vec<StreamDefinition> {
        self.definitions
            .lock()
            .unwrap()
            .values()
            .filter(|d| d.database == db_name)
            .cloned()
            .collect()
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
        let definitions = manager.list_streams("test_db");
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

        let result = manager.create_stream_as(
            "test_db",
            query,
            QueryPrincipal::from_roles("alice", vec!["editor".to_string()]),
        );
        assert!(
            result.is_ok(),
            "Failed to create stream: {:?}",
            result.err()
        );

        let streams = manager.list_streams("test_db");
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].name, "high_value_events");
        assert_eq!(
            streams[0].principal.as_ref().map(|p| p.user.as_str()),
            Some("alice")
        );

        // Clean up
        manager.stop_stream("test_db", "high_value_events").unwrap();
        assert!(manager.list_streams("test_db").is_empty());
    }

    /// Audit H6: a stream is visible to, and stoppable from, its own
    /// database only; same-named streams in two databases coexist.
    #[tokio::test]
    async fn streams_are_scoped_per_database() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(StorageEngine::new(temp_dir.path()).unwrap());
        for db_name in ["db_a", "db_b"] {
            storage.create_database(db_name.to_string()).unwrap();
            let db = storage.get_database(db_name).unwrap();
            db.create_collection("events".to_string(), None).unwrap();
        }
        let manager = StreamManager::new(storage);
        let q = r#"
            CREATE STREAM s AS
            FOR e IN events
            WINDOW TUMBLING (SIZE "1m")
            RETURN e
        "#;
        manager.create_stream("db_a", parse(q).unwrap()).unwrap();
        manager.create_stream("db_b", parse(q).unwrap()).unwrap();

        assert_eq!(manager.list_streams("db_a").len(), 1);
        assert_eq!(manager.list_streams("db_b").len(), 1);
        assert!(manager.list_streams("db_c").is_empty());
        // Legacy/no-creator definitions record no principal.
        assert!(manager.list_streams("db_a")[0].principal.is_none());

        // Stopping from db_c or db_b leaves db_a's stream alone.
        manager.stop_stream("db_c", "s").unwrap();
        manager.stop_stream("db_b", "s").unwrap();
        assert_eq!(manager.list_streams("db_a").len(), 1);
        assert!(manager.list_streams("db_b").is_empty());
        manager.stop_stream("db_a", "s").unwrap();
    }

    #[tokio::test]
    async fn zero_window_is_rejected_at_create() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(StorageEngine::new(temp_dir.path()).unwrap());
        storage.create_database("test_db".to_string()).unwrap();
        let db = storage.get_database("test_db").unwrap();
        db.create_collection("events".to_string(), None).unwrap();
        let manager = StreamManager::new(storage);
        let q = r#"
            CREATE STREAM z AS
            FOR e IN events
            WINDOW TUMBLING (SIZE "0s")
            RETURN e
        "#;
        assert!(manager.create_stream("test_db", parse(q).unwrap()).is_err());
        // The failed create must not leave a definition behind.
        assert!(manager.list_streams("test_db").is_empty());
    }
}
