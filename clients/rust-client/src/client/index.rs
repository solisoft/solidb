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
