use super::SoliDBClient;
use crate::protocol::{Command, DriverError};
use serde_json::Value;

impl SoliDBClient {
    pub async fn create_index(
        &mut self,
        database: &str,
        collection: &str,
        name: &str,
        fields: Vec<String>,
        unique: bool,
        sparse: bool,
    ) -> Result<(), DriverError> {
        let response = self
            .send_command(Command::CreateIndex {
                database: database.to_string(),
                collection: collection.to_string(),
                name: name.to_string(),
                fields,
                unique,
                sparse,
            })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    pub async fn delete_index(
        &mut self,
        database: &str,
        collection: &str,
        name: &str,
    ) -> Result<(), DriverError> {
        let response = self
            .send_command(Command::DeleteIndex {
                database: database.to_string(),
                collection: collection.to_string(),
                name: name.to_string(),
            })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    pub async fn list_indexes(
        &mut self,
        database: &str,
        collection: &str,
    ) -> Result<Vec<Value>, DriverError> {
        let response = self
            .send_command(Command::ListIndexes {
                database: database.to_string(),
                collection: collection.to_string(),
            })
            .await?;
        let data = Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))?;

        serde_json::from_value(data)
            .map_err(|e| DriverError::ProtocolError(format!("Invalid response: {}", e)))
    }

    /// Compare the indexes of `collection` with its documents: per index,
    /// the entries that are missing, those no document accounts for, and
    /// the unique values held by more than one document. Reads the whole
    /// collection; changes nothing.
    pub async fn check_indexes(
        &mut self,
        database: &str,
        collection: &str,
    ) -> Result<Value, DriverError> {
        let response = self
            .send_command(Command::CheckIndexes {
                database: database.to_string(),
                collection: collection.to_string(),
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// The foreign keys declared on `collection` (`foreign_keys`) and those
    /// of other collections that refer to it (`referenced_by`).
    pub async fn list_foreign_keys(
        &mut self,
        database: &str,
        collection: &str,
    ) -> Result<Value, DriverError> {
        let response = self
            .send_command(Command::ListForeignKeys {
                database: database.to_string(),
                collection: collection.to_string(),
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// Declare that `field` of `collection` holds the `_key` of a document
    /// of `references`, a collection of the same database, or null.
    /// `on_delete` is "restrict" (the default), "cascade" or "set_null".
    /// Refused when stored documents already refer to a missing parent:
    /// [`Self::check_foreign_key`] lists them. Returns the key as declared.
    pub async fn create_foreign_key(
        &mut self,
        database: &str,
        collection: &str,
        field: &str,
        references: &str,
        on_delete: Option<&str>,
    ) -> Result<Value, DriverError> {
        let response = self
            .send_command(Command::CreateForeignKey {
                database: database.to_string(),
                collection: collection.to_string(),
                field: field.to_string(),
                references: references.to_string(),
                on_delete: on_delete.map(str::to_string),
                by: None,
                name: None,
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    pub async fn delete_foreign_key(
        &mut self,
        database: &str,
        collection: &str,
        name: &str,
    ) -> Result<(), DriverError> {
        let response = self
            .send_command(Command::DeleteForeignKey {
                database: database.to_string(),
                collection: collection.to_string(),
                name: name.to_string(),
            })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// The documents of `collection` whose `field` names a document that is
    /// not in `references`: `{"count": n, "sample": [...]}`.
    pub async fn check_foreign_key(
        &mut self,
        database: &str,
        collection: &str,
        field: &str,
        references: &str,
    ) -> Result<Value, DriverError> {
        let response = self
            .send_command(Command::CheckForeignKey {
                database: database.to_string(),
                collection: collection.to_string(),
                field: field.to_string(),
                references: references.to_string(),
                by: None,
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// The counter fields of `collection`: `{"counters": [...]}`, each with
    /// its `state` (`next` is the next number).
    pub async fn list_counters(
        &mut self,
        database: &str,
        collection: &str,
    ) -> Result<Value, DriverError> {
        let response = self
            .send_command(Command::ListCounters {
                database: database.to_string(),
                collection: collection.to_string(),
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// Declare that the database numbers `field` of `collection` on insert:
    /// 1, 2, 3…, never the same number twice and with no gap. `options` is
    /// an object with any of `start`, `reset` ("day", "week", "month",
    /// "year"), `timezone`, `prefix`, `suffix`, `padding`, `durable` and
    /// `node`, or `Value::Null` for a plain count from 1. Returns the
    /// counter as declared, with its state. An insert then leaves the field
    /// out and reads it from the answer; in a transaction the number is
    /// given at commit.
    pub async fn create_counter(
        &mut self,
        database: &str,
        collection: &str,
        field: &str,
        options: Value,
    ) -> Result<Value, DriverError> {
        let mut counter = match options {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        counter.insert("field".to_string(), Value::String(field.to_string()));
        let response = self
            .send_command(Command::CreateCounter {
                database: database.to_string(),
                collection: collection.to_string(),
                counter: Value::Object(counter),
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// Stop numbering `field`. The stored numbers and their unique index
    /// stay.
    pub async fn delete_counter(
        &mut self,
        database: &str,
        collection: &str,
        field: &str,
    ) -> Result<(), DriverError> {
        let response = self
            .send_command(Command::DeleteCounter {
                database: database.to_string(),
                collection: collection.to_string(),
                field: field.to_string(),
            })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// Make `next` the number the next insert gets. Refused when a document
    /// already holds it. Returns the counter with its state.
    pub async fn set_counter_next(
        &mut self,
        database: &str,
        collection: &str,
        field: &str,
        next: u64,
    ) -> Result<Value, DriverError> {
        let response = self
            .send_command(Command::SetCounterNext {
                database: database.to_string(),
                collection: collection.to_string(),
                field: field.to_string(),
                next,
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// Hand the numbering of `field` to `node` (in a cluster, one node
    /// numbers). Asked of the node that numbers now.
    pub async fn move_counter(
        &mut self,
        database: &str,
        collection: &str,
        field: &str,
        node: Option<&str>,
    ) -> Result<Value, DriverError> {
        let response = self
            .send_command(Command::MoveCounter {
                database: database.to_string(),
                collection: collection.to_string(),
                field: field.to_string(),
                node: node.map(str::to_string),
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// Combined vector + fulltext search with score fusion.
    ///
    /// Optional weights default to 0.5/0.5, `limit` to 10 and `fusion` to
    /// "weighted" ("rrf" selects Reciprocal Rank Fusion). Each hit carries
    /// `doc_key`, `score`, `vector_score`, `text_score`, `sources` and the
    /// full `document`.
    #[allow(clippy::too_many_arguments)]
    pub async fn hybrid_search(
        &mut self,
        database: &str,
        collection: &str,
        vector: Vec<f32>,
        text_query: &str,
        vector_index: &str,
        fulltext_field: &str,
        vector_weight: Option<f32>,
        text_weight: Option<f32>,
        limit: Option<u32>,
        fusion: Option<String>,
    ) -> Result<Vec<Value>, DriverError> {
        let response = self
            .send_command(Command::HybridSearch {
                database: database.to_string(),
                collection: collection.to_string(),
                vector,
                text_query: text_query.to_string(),
                vector_index: vector_index.to_string(),
                fulltext_field: fulltext_field.to_string(),
                vector_weight,
                text_weight,
                limit,
                fusion,
            })
            .await?;
        let data = Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))?;

        serde_json::from_value(data)
            .map_err(|e| DriverError::ProtocolError(format!("Invalid response: {}", e)))
    }
}
