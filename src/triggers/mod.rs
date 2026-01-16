use crate::error::{DbError, DbResult};
use crate::queue::{Job, JobStatus};
use crate::storage::{Document, StorageEngine};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Trigger event types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TriggerEvent {
    Insert,
    Update,
    Delete,
}

impl TriggerEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            TriggerEvent::Insert => "insert",
            TriggerEvent::Update => "update",
            TriggerEvent::Delete => "delete",
        }
    }
}

/// A trigger that fires when documents are modified
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    #[serde(rename = "_key")]
    pub id: String,
    #[serde(rename = "_rev", skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Human-readable name
    pub name: String,
    /// Collection to watch
    pub collection: String,
    /// Events that trigger execution (insert, update, delete)
    pub events: Vec<TriggerEvent>,
    /// Path to the Lua script in _scripts collection
    pub script_path: String,
    /// Queue name for job execution
    #[serde(default = "default_queue")]
    pub queue: String,
    /// Job priority (higher = more important)
    #[serde(default)]
    pub priority: i32,
    /// Maximum retry attempts
    #[serde(default = "default_max_retries")]
    pub max_retries: i32,
    /// Whether the trigger is active
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Optional SDBQL filter expression (not yet implemented)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Creation timestamp
    pub created_at: u64,
    /// Last update timestamp
    pub updated_at: u64,
}

fn default_queue() -> String {
    "default".to_string()
}

fn default_max_retries() -> i32 {
    5
}

fn default_enabled() -> bool {
    true
}

impl Trigger {
    /// Create a new trigger
    pub fn new(
        name: String,
        collection: String,
        events: Vec<TriggerEvent>,
        script_path: String,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            revision: None,
            name,
            collection,
            events,
            script_path,
            queue: default_queue(),
            priority: 0,
            max_retries: default_max_retries(),
            enabled: default_enabled(),
            filter: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Check if this trigger should fire for a given event
    pub fn matches_event(&self, event: &TriggerEvent) -> bool {
        self.enabled && self.events.contains(event)
    }
}

/// Parameters passed to the trigger script
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerJobParams {
    /// Name of the trigger that fired
    pub trigger_name: String,
    /// Event type (insert, update, delete)
    pub event: String,
    /// Collection name
    pub collection: String,
    /// Document key
    pub key: String,
    /// New document data (for insert/update)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<JsonValue>,
    /// Previous document data (for update/delete)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_data: Option<JsonValue>,
}

/// Manager for trigger operations
pub struct TriggerManager {
    storage: Arc<StorageEngine>,
    notifier: Option<broadcast::Sender<()>>,
}

impl TriggerManager {
    pub fn new(storage: Arc<StorageEngine>) -> Self {
        Self {
            storage,
            notifier: None,
        }
    }

    /// Set queue notifier for immediate job processing
    pub fn with_notifier(mut self, notifier: broadcast::Sender<()>) -> Self {
        self.notifier = Some(notifier);
        self
    }

    /// Get all triggers for a collection
    pub fn get_triggers_for_collection(
        &self,
        db_name: &str,
        collection_name: &str,
    ) -> DbResult<Vec<Trigger>> {
        let db = self.storage.get_database(db_name)?;

        // Ensure _triggers collection exists
        let triggers_coll = match db.get_collection("_triggers") {
            Ok(coll) => coll,
            Err(_) => return Ok(Vec::new()), // No triggers collection = no triggers
        };

        let mut triggers = Vec::new();
        for doc in triggers_coll.scan(None) {
            if let Ok(trigger) = serde_json::from_value::<Trigger>(doc.to_value()) {
                if trigger.collection == collection_name && trigger.enabled {
                    triggers.push(trigger);
                }
            }
        }

        Ok(triggers)
    }

    /// Fire triggers for a document operation
    pub fn fire_triggers(
        &self,
        db_name: &str,
        collection_name: &str,
        event: TriggerEvent,
        doc: &Document,
        old_doc: Option<&JsonValue>,
    ) -> DbResult<Vec<String>> {
        // Skip triggers for system collections
        if collection_name.starts_with('_') {
            return Ok(Vec::new());
        }

        let triggers = self.get_triggers_for_collection(db_name, collection_name)?;
        let mut job_ids = Vec::new();

        for trigger in triggers {
            if !trigger.matches_event(&event) {
                continue;
            }

            // Create job for this trigger
            match self.create_trigger_job(db_name, &trigger, &event, doc, old_doc) {
                Ok(job_id) => {
                    tracing::info!(
                        "Trigger '{}' fired for {} on {}/{}: created job {}",
                        trigger.name,
                        event.as_str(),
                        collection_name,
                        &doc.key,
                        job_id
                    );
                    job_ids.push(job_id);
                }
                Err(e) => {
                    tracing::error!("Failed to create job for trigger '{}': {}", trigger.name, e);
                }
            }
        }

        // Notify queue workers if we created any jobs
        if !job_ids.is_empty() {
            if let Some(ref notifier) = self.notifier {
                let _ = notifier.send(());
            }
        }

        Ok(job_ids)
    }

    /// Create a job for a trigger
    fn create_trigger_job(
        &self,
        db_name: &str,
        trigger: &Trigger,
        event: &TriggerEvent,
        doc: &Document,
        old_doc: Option<&JsonValue>,
    ) -> DbResult<String> {
        let db = self.storage.get_database(db_name)?;

        // Ensure _jobs collection exists
        if db.get_collection("_jobs").is_err() {
            db.create_collection("_jobs".to_string(), None)?;
        }

        let jobs_coll = db.get_collection("_jobs")?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Build job params
        let params = TriggerJobParams {
            trigger_name: trigger.name.clone(),
            event: event.as_str().to_string(),
            collection: trigger.collection.clone(),
            key: doc.key.clone(),
            data: match event {
                TriggerEvent::Delete => None,
                _ => Some(doc.to_value()),
            },
            old_data: old_doc.cloned(),
        };

        let job = Job {
            id: uuid::Uuid::new_v4().to_string(),
            revision: None,
            queue: trigger.queue.clone(),
            priority: trigger.priority,
            script_path: trigger.script_path.clone(),
            params: serde_json::to_value(&params).unwrap_or(JsonValue::Null),
            status: JobStatus::Pending,
            retry_count: 0,
            max_retries: trigger.max_retries,
            last_error: None,
            cron_job_id: None,
            run_at: now,
            created_at: now,
            started_at: None,
            completed_at: None,
        };

        let job_id = job.id.clone();
        let job_val = serde_json::to_value(&job)
            .map_err(|e| DbError::InternalError(format!("Failed to serialize job: {}", e)))?;

        jobs_coll.insert(job_val)?;

        Ok(job_id)
    }
}

/// Standalone function to fire triggers (for use from collection operations)
pub fn fire_collection_triggers(
    storage: &Arc<StorageEngine>,
    notifier: Option<&broadcast::Sender<()>>,
    db_name: &str,
    collection_name: &str,
    event: TriggerEvent,
    doc: &Document,
    old_doc: Option<&JsonValue>,
) -> DbResult<Vec<String>> {
    let mut manager = TriggerManager::new(storage.clone());
    if let Some(n) = notifier {
        manager = manager.with_notifier(n.clone());
    }
    manager.fire_triggers(db_name, collection_name, event, doc, old_doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trigger_event_serialization() {
        let event = TriggerEvent::Insert;
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, "\"insert\"");

        let parsed: TriggerEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, TriggerEvent::Insert);
    }

    #[test]
    fn test_trigger_creation() {
        let trigger = Trigger::new(
            "test_trigger".to_string(),
            "users".to_string(),
            vec![TriggerEvent::Insert, TriggerEvent::Update],
            "triggers/welcome.lua".to_string(),
        );

        assert_eq!(trigger.name, "test_trigger");
        assert_eq!(trigger.collection, "users");
        assert_eq!(trigger.events.len(), 2);
        assert!(trigger.enabled);
        assert_eq!(trigger.queue, "default");
        assert_eq!(trigger.max_retries, 5);
    }

    #[test]
    fn test_trigger_matches_event() {
        let trigger = Trigger::new(
            "test".to_string(),
            "users".to_string(),
            vec![TriggerEvent::Insert],
            "test.lua".to_string(),
        );

        assert!(trigger.matches_event(&TriggerEvent::Insert));
        assert!(!trigger.matches_event(&TriggerEvent::Update));
        assert!(!trigger.matches_event(&TriggerEvent::Delete));
    }

    #[test]
    fn test_trigger_disabled() {
        let mut trigger = Trigger::new(
            "test".to_string(),
            "users".to_string(),
            vec![TriggerEvent::Insert],
            "test.lua".to_string(),
        );
        trigger.enabled = false;

        assert!(!trigger.matches_event(&TriggerEvent::Insert));
    }
}
