use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::error::DbError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardParticipantInfo {
    pub shard_id: u16,
    pub node_id: String,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributedTransactionId(pub String);

impl DistributedTransactionId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl Default for DistributedTransactionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DistributedTransactionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "dtx:{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistributedTransactionState {
    Active,
    Preparing,
    Prepared,
    Committing,
    Committed,
    Aborting,
    Aborted,
    CommittedWithErrors,
}

impl std::fmt::Display for DistributedTransactionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DistributedTransactionState::Active => write!(f, "Active"),
            DistributedTransactionState::Preparing => write!(f, "Preparing"),
            DistributedTransactionState::Prepared => write!(f, "Prepared"),
            DistributedTransactionState::Committing => write!(f, "Committing"),
            DistributedTransactionState::Committed => write!(f, "Committed"),
            DistributedTransactionState::Aborting => write!(f, "Aborting"),
            DistributedTransactionState::Aborted => write!(f, "Aborted"),
            DistributedTransactionState::CommittedWithErrors => write!(f, "CommittedWithErrors"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributedTransaction {
    pub id: DistributedTransactionId,
    pub state: DistributedTransactionState,
    pub participants: Vec<ShardParticipantInfo>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub local_operations: Vec<super::Operation>,
}

impl DistributedTransaction {
    pub fn new(participants: Vec<ShardParticipantInfo>) -> Self {
        Self {
            id: DistributedTransactionId::new(),
            state: DistributedTransactionState::Active,
            participants,
            created_at: chrono::Utc::now(),
            local_operations: Vec::new(),
        }
    }

    pub fn add_local_operation(&mut self, op: super::Operation) {
        self.local_operations.push(op);
    }

    pub fn is_active(&self) -> bool {
        self.state == DistributedTransactionState::Active
    }
}

pub struct DistributedTransactionCoordinator {
    active_transactions: Arc<RwLock<HashMap<String, Arc<RwLock<DistributedTransaction>>>>>,
}

impl DistributedTransactionCoordinator {
    pub fn new() -> Self {
        Self {
            active_transactions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn begin_transaction(
        &self,
        participants: Vec<ShardParticipantInfo>,
    ) -> Result<DistributedTransactionId, DbError> {
        let participant_count = participants.len();
        let tx = DistributedTransaction::new(participants);
        let tx_id = tx.id.clone();

        let mut active = self.active_transactions.write().await;
        active.insert(tx_id.0.clone(), Arc::new(RwLock::new(tx)));

        info!(
            "Distributed transaction {} started with {} participants",
            tx_id, participant_count
        );
        Ok(tx_id)
    }

    pub async fn get_transaction(
        &self,
        tx_id: &DistributedTransactionId,
    ) -> Result<Arc<RwLock<DistributedTransaction>>, DbError> {
        let active = self.active_transactions.read().await;
        active
            .get(&tx_id.0)
            .cloned()
            .ok_or_else(|| DbError::InternalError(format!("Transaction {} not found", tx_id)))
    }

    async fn send_prepare_to_participant(
        &self,
        participant: &ShardParticipantInfo,
        tx_id: &str,
    ) -> Result<(), DbError> {
        let url = format!(
            "http://{}/_api/distributed/participant/prepare/{}",
            participant.address, tx_id
        );

        let client = crate::storage::http_client::get_http_client();
        match client
            .post(&url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                info!(
                    "Prepare succeeded on shard {} ({})",
                    participant.shard_id, participant.node_id
                );
                Ok(())
            }
            Ok(resp) => {
                error!(
                    "Prepare failed on shard {} ({}) - status: {}",
                    participant.shard_id,
                    participant.node_id,
                    resp.status()
                );
                Err(DbError::InternalError(format!(
                    "Prepare failed on shard {}: status {}",
                    participant.shard_id,
                    resp.status()
                )))
            }
            Err(e) => {
                error!(
                    "Prepare failed on shard {} ({}): {}",
                    participant.shard_id, participant.node_id, e
                );
                Err(DbError::InternalError(format!(
                    "Prepare failed on shard {}: {}",
                    participant.shard_id, e
                )))
            }
        }
    }

    async fn send_commit_to_participant(
        &self,
        participant: &ShardParticipantInfo,
        tx_id: &str,
    ) -> Result<(), DbError> {
        let url = format!(
            "http://{}/_api/distributed/participant/commit/{}",
            participant.address, tx_id
        );

        let client = crate::storage::http_client::get_http_client();
        match client
            .post(&url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                info!(
                    "Commit succeeded on shard {} ({})",
                    participant.shard_id, participant.node_id
                );
                Ok(())
            }
            Ok(resp) => {
                error!(
                    "Commit failed on shard {} ({}) - status: {}",
                    participant.shard_id,
                    participant.node_id,
                    resp.status()
                );
                Err(DbError::InternalError(format!(
                    "Commit failed on shard {}: status {}",
                    participant.shard_id,
                    resp.status()
                )))
            }
            Err(e) => {
                error!(
                    "Commit failed on shard {} ({}): {}",
                    participant.shard_id, participant.node_id, e
                );
                Err(DbError::InternalError(format!(
                    "Commit failed on shard {}: {}",
                    participant.shard_id, e
                )))
            }
        }
    }

    async fn send_abort_to_participant(
        &self,
        participant: &ShardParticipantInfo,
        tx_id: &str,
    ) -> Result<(), DbError> {
        let url = format!(
            "http://{}/_api/distributed/participant/abort/{}",
            participant.address, tx_id
        );

        let client = crate::storage::http_client::get_http_client();
        match client
            .post(&url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                info!(
                    "Abort succeeded on shard {} ({})",
                    participant.shard_id, participant.node_id
                );
                Ok(())
            }
            Ok(resp) => {
                warn!(
                    "Abort failed on shard {} ({}) - status: {}",
                    participant.shard_id,
                    participant.node_id,
                    resp.status()
                );
                Ok(())
            }
            Err(e) => {
                warn!(
                    "Abort failed on shard {} ({}): {} (continuing)",
                    participant.shard_id, participant.node_id, e
                );
                Ok(())
            }
        }
    }

    pub async fn prepare(&self, tx_id: &DistributedTransactionId) -> Result<bool, DbError> {
        let tx_arc = self.get_transaction(tx_id).await?;
        let mut tx = tx_arc.write().await;

        if !tx.is_active() {
            return Err(DbError::InternalError(format!(
                "Transaction {} is not active (state: {})",
                tx_id, tx.state
            )));
        }

        tx.state = DistributedTransactionState::Preparing;
        info!(
            "Distributed transaction {} preparing on {} participants",
            tx_id,
            tx.participants.len()
        );

        let participants = tx.participants.clone();
        drop(tx);

        let mut all_success = true;
        for participant in &participants {
            if let Err(e) = self
                .send_prepare_to_participant(participant, &tx_id.0)
                .await
            {
                error!(
                    "Prepare failed on participant {}: {}",
                    participant.node_id, e
                );
                all_success = false;
                break;
            }
        }

        let tx_arc = self.get_transaction(tx_id).await?;
        let mut tx = tx_arc.write().await;
        if all_success {
            tx.state = DistributedTransactionState::Prepared;
        } else {
            tx.state = DistributedTransactionState::Aborting;
            drop(tx);
            for participant in &participants {
                let _ = self.send_abort_to_participant(participant, &tx_id.0).await;
            }
            let tx_arc = self.get_transaction(tx_id).await?;
            let mut tx = tx_arc.write().await;
            tx.state = DistributedTransactionState::Aborted;
            return Err(DbError::InternalError(
                "Prepare failed on one or more participants".to_string(),
            ));
        }

        Ok(true)
    }

    pub async fn commit(&self, tx_id: &DistributedTransactionId) -> Result<(), DbError> {
        let tx_arc = self.get_transaction(tx_id).await?;
        let mut tx = tx_arc.write().await;

        if tx.state != DistributedTransactionState::Prepared {
            warn!(
                "Transaction {} in state {}, expected Prepared",
                tx_id, tx.state
            );
            if tx.state == DistributedTransactionState::Aborted {
                return Err(DbError::InternalError(format!(
                    "Transaction {} was aborted",
                    tx_id
                )));
            }
        }

        tx.state = DistributedTransactionState::Committing;
        info!("Distributed transaction {} committing", tx_id);

        let participants = tx.participants.clone();
        drop(tx);

        for participant in &participants {
            if let Err(e) = self.send_commit_to_participant(participant, &tx_id.0).await {
                error!(
                    "Commit failed on participant {}: {}",
                    participant.node_id, e
                );
            }
        }

        let tx_arc = self.get_transaction(tx_id).await?;
        let mut tx = tx_arc.write().await;
        tx.state = DistributedTransactionState::Committed;

        {
            let mut active = self.active_transactions.write().await;
            active.remove(&tx_id.0);
        }

        info!("Distributed transaction {} committed successfully", tx_id);
        Ok(())
    }

    pub async fn abort(&self, tx_id: &DistributedTransactionId) -> Result<(), DbError> {
        let tx_arc = self.get_transaction(tx_id).await?;
        let mut tx = tx_arc.write().await;

        tx.state = DistributedTransactionState::Aborting;
        info!("Distributed transaction {} aborting", tx_id);

        let participants = tx.participants.clone();
        drop(tx);

        for participant in &participants {
            let _ = self.send_abort_to_participant(participant, &tx_id.0).await;
        }

        let tx_arc = self.get_transaction(tx_id).await?;
        let mut tx = tx_arc.write().await;
        tx.state = DistributedTransactionState::Aborted;

        {
            let mut active = self.active_transactions.write().await;
            active.remove(&tx_id.0);
        }

        info!("Distributed transaction {} aborted", tx_id);
        Ok(())
    }

    pub async fn add_participant(
        &self,
        tx_id: &DistributedTransactionId,
        participant: ShardParticipantInfo,
    ) -> Result<(), DbError> {
        let tx_arc = self.get_transaction(tx_id).await?;
        let mut tx = tx_arc.write().await;

        if !tx.is_active() {
            return Err(DbError::InternalError(format!(
                "Cannot add participant to transaction {} in state {}",
                tx_id, tx.state
            )));
        }

        tx.participants.push(participant);
        Ok(())
    }
}

impl Default for DistributedTransactionCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dtx_lifecycle() {
        let coordinator = DistributedTransactionCoordinator::new();
        let participants = vec![
            ShardParticipantInfo {
                shard_id: 0,
                node_id: "node1".to_string(),
                address: "localhost:8001".to_string(),
            },
            ShardParticipantInfo {
                shard_id: 1,
                node_id: "node2".to_string(),
                address: "localhost:8002".to_string(),
            },
        ];

        let tx_id = coordinator.begin_transaction(participants).await.unwrap();
        assert_eq!(tx_id.0.len(), 36);

        let tx_arc = coordinator.get_transaction(&tx_id).await.unwrap();
        let tx = tx_arc.read().await;
        assert_eq!(tx.state, DistributedTransactionState::Active);
        assert_eq!(tx.participants.len(), 2);

        coordinator.abort(&tx_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_dtx_prepare_commit() {
        let coordinator = DistributedTransactionCoordinator::new();
        let participants = vec![ShardParticipantInfo {
            shard_id: 0,
            node_id: "node1".to_string(),
            address: "localhost:8001".to_string(),
        }];

        let tx_id = coordinator.begin_transaction(participants).await.unwrap();
        coordinator.prepare(&tx_id).await.unwrap();
        coordinator.commit(&tx_id).await.unwrap();

        let active = coordinator.active_transactions.read().await;
        assert!(active.is_empty());
    }
}
