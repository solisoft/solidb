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

/// Serialize a cluster message for the wire, signed.
///
/// **Fails closed.** Without a secret this used to fall through to
/// `Ok(payload.into_bytes())` — an unsigned message on the wire, with nothing
/// in the logs to say so. The matching reader accepted unsigned messages under
/// the same condition, so a cluster started without a keyfile had *no*
/// authentication on its replication bus and looked identical to one that did.
///
/// The asymmetry was the dangerous part: a node **with** a secret rejects
/// unsigned messages, while a node **without** one accepts both. One
/// misconfigured member is therefore an open door into the replicated state of
/// the whole cluster, and the misconfiguration is invisible from every other
/// node.
///
/// So an absent secret is now an error. A cluster that cannot authenticate its
/// own traffic must refuse to exchange it rather than exchange it in the clear
/// — the same rule the workload credential probe follows: less isolation still
/// works, no secret does not.
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
        _ => anyhow::bail!(
            "refusing to send an unauthenticated cluster message: no cluster secret is \
             configured. Set `cluster.keyfile` to a file containing a shared secret of at \
             least 32 bytes, identical on every node."
        ),
    }
}

/// Parse and verify an incoming cluster message.
///
/// **Fails closed**, for the reason given on [`seal_cluster_message`]: the
/// no-secret branch used to `serde_json::from_slice` whatever arrived and hand
/// it to `handle_message`, which applies membership changes and rebalances. On
/// a host with a public address — every OVH VPS — that is remote control of the
/// replicated state by anyone who can reach the port.
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
        _ => anyhow::bail!(
            "refusing to accept an unauthenticated cluster message: no cluster secret is \
             configured. Until one is, this node cannot take part in a cluster."
        ),
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

#[cfg(test)]
mod cluster_auth_tests {
    use super::*;

    /// A `Leave` on purpose: forging one evicts a node from the cluster, which
    /// is precisely what an unauthenticated bus lets a stranger do.
    fn message() -> ClusterMessage {
        ClusterMessage::Leave {
            from: "node-a".to_string(),
        }
    }

    const SECRET: &str = "a-shared-cluster-secret-at-least-32-bytes";

    #[test]
    fn a_signed_message_round_trips() {
        let sealed = seal_cluster_message(&message(), Some(SECRET)).unwrap();
        assert!(open_cluster_message(&sealed, Some(SECRET)).is_ok());
    }

    #[test]
    fn sending_without_a_secret_is_refused_rather_than_sent_in_the_clear() {
        // The regression this test exists for: both arms used to fall through
        // to the raw payload, so a cluster with no keyfile ran unauthenticated
        // and looked exactly like one that did not.
        for absent in [None, Some(""), Some("   ")] {
            let result = seal_cluster_message(&message(), absent.map(str::trim));
            assert!(result.is_err(), "sent unauthenticated for {absent:?}");
        }
    }

    #[test]
    fn receiving_without_a_secret_is_refused_rather_than_trusted() {
        // The dangerous half. `handle_message` applies membership changes and
        // rebalances, so accepting an unsigned message is remote control of the
        // replicated state by anyone who can reach the port.
        let raw = serde_json::to_vec(&message()).unwrap();
        assert!(open_cluster_message(&raw, None).is_err());
        assert!(open_cluster_message(&raw, Some("")).is_err());
    }

    #[test]
    fn a_node_with_a_secret_still_rejects_an_unsigned_message() {
        // This half already worked. Asserted so the fix above cannot be
        // "simplified" by making both arms permissive again.
        let raw = serde_json::to_vec(&message()).unwrap();
        assert!(open_cluster_message(&raw, Some(SECRET)).is_err());
    }

    #[test]
    fn a_message_signed_with_another_secret_is_rejected() {
        let sealed =
            seal_cluster_message(&message(), Some("some-other-cluster-secret-32b")).unwrap();
        assert!(open_cluster_message(&sealed, Some(SECRET)).is_err());
    }

    #[test]
    fn a_tampered_payload_is_rejected() {
        // The signature covers the payload, so altering it after signing must
        // fail rather than replicate the attacker's version.
        let sealed = seal_cluster_message(&message(), Some(SECRET)).unwrap();
        let mut envelope: serde_json::Value = serde_json::from_slice(&sealed).unwrap();
        envelope["payload"] = serde_json::Value::String(
            serde_json::to_string(&ClusterMessage::Leave {
                from: "a-node-the-attacker-wants-evicted".into(),
            })
            .unwrap(),
        );
        let altered = serde_json::to_vec(&envelope).unwrap();
        assert!(open_cluster_message(&altered, Some(SECRET)).is_err());
    }

    #[test]
    fn the_refusal_says_what_to_configure() {
        // An operator meets this at cluster start-up. "Refused" alone sends
        // them to the network; naming the setting sends them to the fix.
        let err = seal_cluster_message(&message(), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("cluster.keyfile"), "{err}");
        assert!(err.contains("every node"), "{err}");
    }
}
