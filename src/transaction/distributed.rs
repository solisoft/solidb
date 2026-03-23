use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

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

        for participant in &tx.participants {
            info!(
                "Would send prepare to shard {} on node {}",
                participant.shard_id, participant.node_id
            );
        }

        tx.state = DistributedTransactionState::Prepared;
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

        for participant in &tx.participants {
            info!(
                "Would send commit to shard {} on node {}",
                participant.shard_id, participant.node_id
            );
        }

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

        for participant in &tx.participants {
            info!(
                "Would send abort to shard {} on node {}",
                participant.shard_id, participant.node_id
            );
        }

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
