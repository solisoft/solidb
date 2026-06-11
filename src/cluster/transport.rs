use super::node::Node;
use super::stats::NodeBasicStats;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

/// Maximum accepted size for a cluster control message.
pub const MAX_CLUSTER_MESSAGE_SIZE: usize = 1024 * 1024;

/// Maximum clock skew tolerated when validating a signed message (replay window).
const MAX_TIMESTAMP_SKEW_MS: u64 = 5 * 60 * 1000;

/// Message types for cluster management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClusterMessage {
    JoinRequest(Node),
    JoinResponse {
        success: bool,
        peers: Vec<Node>,
    },
    Heartbeat {
        from: String,
        sequence: u64,
        stats: Option<NodeBasicStats>,
    },
    Leave {
        from: String,
    },
    Replication(crate::sync::SyncMessage),
}

/// Abstract transport layer
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, to: &str, msg: ClusterMessage) -> Result<()>;
    async fn broadcast(&self, msg: ClusterMessage) -> Result<()>;
    // Receiver handling is usually done by binding a listener
}

/// HMAC-signed envelope for cluster control messages.
///
/// Cluster messages drive membership and shard rebalancing; an
/// unauthenticated `NodeLeave` or `JoinRequest` injected by anything that can
/// reach the port would cause split-brain or shard reshuffling. When a
/// cluster keyfile is configured, every message is wrapped in this envelope
/// and receivers reject anything unsigned, stale, or with a bad signature.
#[derive(Debug, Serialize, Deserialize)]
pub struct SignedClusterMessage {
    /// Envelope version
    pub v: u8,
    /// Sender wall clock, ms since epoch (replay window check)
    pub ts: u64,
    /// Random nonce included in the signature
    pub nonce: String,
    /// hex(HMAC-SHA256(secret, "{ts}:{nonce}:{payload}"))
    pub sig: String,
    /// JSON-serialized `ClusterMessage` (signed as the exact string)
    pub payload: String,
}

fn hmac_hex(secret: &str, ts: u64, nonce: &str, payload: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");
    mac.update(format!("{}:{}:{}", ts, nonce, payload).as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Serialize a cluster message for the wire, signing it when a secret is set.
pub fn seal_cluster_message(msg: &ClusterMessage, secret: Option<&str>) -> Result<Vec<u8>> {
    let payload = serde_json::to_string(msg)?;
    match secret {
        Some(secret) if !secret.is_empty() => {
            let ts = chrono::Utc::now().timestamp_millis() as u64;
            let nonce = uuid::Uuid::new_v4().to_string();
            let sig = hmac_hex(secret, ts, &nonce, &payload);
            Ok(serde_json::to_vec(&SignedClusterMessage {
                v: 1,
                ts,
                nonce,
                sig,
                payload,
            })?)
        }
        _ => Ok(payload.into_bytes()),
    }
}

/// Parse (and verify, when a secret is configured) an incoming cluster
/// message. With a secret set, unsigned or invalid messages are rejected.
pub fn open_cluster_message(data: &[u8], secret: Option<&str>) -> Result<ClusterMessage> {
    match secret {
        Some(secret) if !secret.is_empty() => {
            let envelope: SignedClusterMessage = serde_json::from_slice(data)
                .map_err(|_| anyhow::anyhow!("unsigned or malformed cluster message rejected"))?;
            let now = chrono::Utc::now().timestamp_millis() as u64;
            if envelope.ts.abs_diff(now) > MAX_TIMESTAMP_SKEW_MS {
                anyhow::bail!("cluster message timestamp outside replay window");
            }
            let expected = hmac_hex(secret, envelope.ts, &envelope.nonce, &envelope.payload);
            if !crate::server::auth::constant_time_eq(expected.as_bytes(), envelope.sig.as_bytes())
            {
                anyhow::bail!("cluster message signature mismatch");
            }
            Ok(serde_json::from_str(&envelope.payload)?)
        }
        _ => Ok(serde_json::from_slice(data)?),
    }
}

pub struct TcpTransport {
    local_address: String,
    /// Cluster secret (keyfile content); messages are signed when set.
    secret: Option<String>,
    // Simplified: in real app, we might keep connection pools
}

impl TcpTransport {
    pub fn new(local_address: String, secret: Option<String>) -> Self {
        Self {
            local_address,
            secret,
        }
    }

    pub async fn listen(&self) -> Result<TcpListener> {
        let listener = TcpListener::bind(&self.local_address).await?;
        Ok(listener)
    }

    pub async fn connect_and_send_signed(
        addr: &str,
        msg: ClusterMessage,
        secret: Option<&str>,
    ) -> Result<()> {
        let mut stream = TcpStream::connect(addr).await?;
        let data = seal_cluster_message(&msg, secret)?;
        stream.write_all(&data).await?;
        Ok(())
    }

    pub async fn connect_and_send(addr: &str, msg: ClusterMessage) -> Result<()> {
        Self::connect_and_send_signed(addr, msg, None).await
    }
}

#[async_trait::async_trait]
impl Transport for TcpTransport {
    async fn send(&self, to: &str, msg: ClusterMessage) -> Result<()> {
        Self::connect_and_send_signed(to, msg, self.secret.as_deref()).await
    }

    async fn broadcast(&self, _msg: ClusterMessage) -> Result<()> {
        // Broadcast implementation requires knowing peers, usually passed or managed higher up
        Ok(())
    }
}
