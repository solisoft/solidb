use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::AsyncWriteExt;
use anyhow::Result;
use super::node::Node;

/// Message types for cluster management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClusterMessage {
    JoinRequest(Node),
    JoinResponse { success: bool, peers: Vec<Node> },
    Heartbeat { from: String, sequence: u64 },
    Leave { from: String },
    Replication(crate::replication::protocol::ReplicationMessage),
}

/// Abstract transport layer
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, to: &str, msg: ClusterMessage) -> Result<()>;
    async fn broadcast(&self, msg: ClusterMessage) -> Result<()>;
    // Receiver handling is usually done by binding a listener
}

pub struct TcpTransport {
    local_address: String,
    // Simplified: in real app, we might keep connection pools
}

impl TcpTransport {
    pub fn new(local_address: String) -> Self {
        Self { local_address }
    }

    pub async fn listen(&self) -> Result<TcpListener> {
        let listener = TcpListener::bind(&self.local_address).await?;
        Ok(listener)
    }

    pub async fn connect_and_send(addr: &str, msg: ClusterMessage) -> Result<()> {
        let mut stream = TcpStream::connect(addr).await?;
        let data = serde_json::to_vec(&msg)?;
        stream.write_all(&data).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Transport for TcpTransport {
    async fn send(&self, to: &str, msg: ClusterMessage) -> Result<()> {
        Self::connect_and_send(to, msg).await
    }

    async fn broadcast(&self, _msg: ClusterMessage) -> Result<()> {
        // Broadcast implementation requires knowing peers, usually passed or managed higher up
        Ok(())
    }
}
