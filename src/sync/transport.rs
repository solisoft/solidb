//! TCP transport layer for sync communication
//!
//! Provides persistent TCP connections between nodes with:
//! - Connection pooling
//! - HMAC authentication
//! - Automatic reconnection with exponential backoff
//! - LZ4 compression for large payloads

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use super::protocol::SyncMessage;

/// Maximum message size (10 MB)
pub const MAX_MESSAGE_SIZE: u32 = 10 * 1024 * 1024;

/// Budget for the encoded payload of one batch, well under
/// [`MAX_MESSAGE_SIZE`] so framing and incompressible data still fit
/// (audit A6).
pub const BATCH_BYTE_BUDGET: usize = 8 * 1024 * 1024;

/// TCP connect timeout to a peer. Without it a black-holed address held the
/// sync worker for the OS default (~2 minutes) — longer than the dead-node
/// timeout, so peers evicted this healthy node (audit A7).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Bound on the whole client handshake (magic + challenge/response).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// Bound on writing one frame to a peer whose receive window is full.
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// Bound on an inbound connection's server-side handshake, magic included
/// (audit A8).
pub const SERVER_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// Compression threshold (64 KB)
const COMPRESSION_THRESHOLD: usize = 64 * 1024;

/// Unauthenticated replication is off unless an operator explicitly opts in.
/// `SOLIDB_REQUIRE_KEYFILE=true` still wins (always required).
fn allow_unauthenticated_sync() -> bool {
    std::env::var("SOLIDB_ALLOW_UNAUTHENTICATED_SYNC")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Trait alias for sync streams
pub trait SyncStreamTrait: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> SyncStreamTrait for T {}

pub type SyncStream = Box<dyn SyncStreamTrait>;

/// Error type for transport operations
#[derive(Debug)]
pub enum TransportError {
    ConnectionFailed(String),
    AuthFailed(String),
    IoError(std::io::Error),
    EncodeError(String),
    DecodeError(String),
    MessageTooLarge(u32),
    Disconnected,
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::ConnectionFailed(msg) => write!(f, "Connection failed: {}", msg),
            TransportError::AuthFailed(msg) => write!(f, "Authentication failed: {}", msg),
            TransportError::IoError(e) => write!(f, "IO error: {}", e),
            TransportError::EncodeError(msg) => write!(f, "Encode error: {}", msg),
            TransportError::DecodeError(msg) => write!(f, "Decode error: {}", msg),
            TransportError::MessageTooLarge(size) => write!(f, "Message too large: {} bytes", size),
            TransportError::Disconnected => write!(f, "Disconnected"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<std::io::Error> for TransportError {
    fn from(e: std::io::Error) -> Self {
        TransportError::IoError(e)
    }
}

/// Active connection to a peer
struct PeerConnection {
    stream: TcpStream,
    last_activity: std::time::Instant,
}

type SharedConnection = Arc<Mutex<PeerConnection>>;

/// Connection pool for managing peer connections
///
/// Each connection has its own lock (audit A7): the pool map used to be
/// write-locked across a whole `receive`, so one slow peer stalled every
/// other peer and the heartbeats. Heartbeats use a separate connection per
/// peer so they are never queued behind a long sync round trip.
pub struct ConnectionPool {
    connections: RwLock<HashMap<String, SharedConnection>>,
    heartbeat_connections: RwLock<HashMap<String, SharedConnection>>,
    _local_node_id: String,
    keyfile_path: String,
}

impl ConnectionPool {
    pub fn new(local_node_id: String, keyfile_path: String) -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
            heartbeat_connections: RwLock::new(HashMap::new()),
            _local_node_id: local_node_id,
            keyfile_path,
        }
    }

    /// Connect to a peer with authentication
    pub async fn connect(&self, peer_addr: &str) -> Result<(), TransportError> {
        // Check if already connected
        {
            let conns = self.connections.read().await;
            if conns.contains_key(peer_addr) {
                return Ok(());
            }
        }

        let stream = self.establish(peer_addr).await?;

        // Store connection (keep an existing one if a concurrent connect won)
        self.connections
            .write()
            .await
            .entry(peer_addr.to_string())
            .or_insert_with(|| {
                Arc::new(Mutex::new(PeerConnection {
                    stream,
                    last_activity: std::time::Instant::now(),
                }))
            });

        debug!("ConnectionPool: Connected to peer: {}", peer_addr);
        Ok(())
    }

    /// Open, announce and authenticate a new TCP connection, all bounded.
    async fn establish(&self, peer_addr: &str) -> Result<TcpStream, TransportError> {
        tokio::time::timeout(
            HANDSHAKE_TIMEOUT + CONNECT_TIMEOUT,
            self.establish_inner(peer_addr),
        )
        .await
        .map_err(|_| {
            TransportError::ConnectionFailed(format!("{}: handshake timed out", peer_addr))
        })?
    }

    async fn establish_inner(&self, peer_addr: &str) -> Result<TcpStream, TransportError> {
        debug!("ConnectionPool: Connecting to peer: {}", peer_addr);

        let stream =
            match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(peer_addr)).await {
                Ok(Ok(s)) => {
                    debug!("ConnectionPool: TCP connected to {}", peer_addr);
                    s
                }
                Ok(Err(e)) => {
                    debug!(
                        "ConnectionPool: TCP connection failed to {}: {}",
                        peer_addr, e
                    );
                    return Err(TransportError::ConnectionFailed(format!(
                        "{}: {}",
                        peer_addr, e
                    )));
                }
                Err(_) => {
                    return Err(TransportError::ConnectionFailed(format!(
                        "{}: connect timed out after {:?}",
                        peer_addr, CONNECT_TIMEOUT
                    )));
                }
            };

        // Send magic header for protocol detection
        use tokio::io::AsyncWriteExt;
        let mut stream = stream;
        if let Err(e) = stream.write_all(b"solidb-sync-v1").await {
            debug!(
                "ConnectionPool: Failed to send magic header to {}: {}",
                peer_addr, e
            );
            return Err(TransportError::IoError(e));
        }
        debug!("ConnectionPool: Magic header sent to {}", peer_addr);

        // Flush to ensure magic header is sent before authentication
        if let Err(e) = stream.flush().await {
            debug!(
                "ConnectionPool: Failed to flush magic header to {}: {}",
                peer_addr, e
            );
            return Err(TransportError::IoError(e));
        }

        // Perform authentication
        match self.authenticate_client(stream).await {
            Ok(s) => Ok(s),
            Err(e) => {
                debug!(
                    "ConnectionPool: Authentication failed with {}: {}",
                    peer_addr, e
                );
                Err(e)
            }
        }
    }

    /// Send a heartbeat on the peer's dedicated heartbeat connection,
    /// connecting it on demand. Bounded end to end; on failure the
    /// connection is dropped so the next tick reconnects.
    pub async fn send_heartbeat(
        &self,
        peer_addr: &str,
        msg: &SyncMessage,
    ) -> Result<(), TransportError> {
        let existing = self
            .heartbeat_connections
            .read()
            .await
            .get(peer_addr)
            .cloned();
        let conn = match existing {
            Some(c) => c,
            None => {
                let stream = self.establish(peer_addr).await?;
                let c = Arc::new(Mutex::new(PeerConnection {
                    stream,
                    last_activity: std::time::Instant::now(),
                }));
                self.heartbeat_connections
                    .write()
                    .await
                    .insert(peer_addr.to_string(), c.clone());
                c
            }
        };

        let result = {
            let mut guard = conn.lock().await;
            let r = Self::write_message_timed(&mut guard.stream, msg).await;
            if r.is_ok() {
                guard.last_activity = std::time::Instant::now();
            }
            r
        };
        if result.is_err() {
            self.heartbeat_connections.write().await.remove(peer_addr);
        }
        result
    }

    /// Forget the heartbeat connection to a peer (next heartbeat reconnects).
    pub async fn drop_heartbeat_connection(&self, peer_addr: &str) {
        self.heartbeat_connections.write().await.remove(peer_addr);
    }

    async fn write_message_timed<T>(stream: &mut T, msg: &SyncMessage) -> Result<(), TransportError>
    where
        T: tokio::io::AsyncWrite + Unpin,
    {
        tokio::time::timeout(WRITE_TIMEOUT, Self::write_message(stream, msg))
            .await
            .map_err(|_| {
                TransportError::IoError(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "sync write timed out",
                ))
            })?
    }

    async fn get_connection(&self, peer_addr: &str) -> Option<SharedConnection> {
        self.connections.read().await.get(peer_addr).cloned()
    }

    /// Authenticate as client (respond to server's challenge)
    async fn authenticate_client(
        &self,
        mut stream: TcpStream,
    ) -> Result<TcpStream, TransportError> {
        if self.keyfile_path.is_empty() || !std::path::Path::new(&self.keyfile_path).exists() {
            if !allow_unauthenticated_sync() {
                return Err(TransportError::AuthFailed(
                    "Cluster keyfile required for replication (set SOLIDB_ALLOW_UNAUTHENTICATED_SYNC=true only for local tests)".to_string(),
                ));
            }
            warn!("authenticate_client: no keyfile, skipping authentication — inter-node replication is unauthenticated");
            return Ok(stream);
        }

        debug!("authenticate_client: waiting for challenge");
        // Small delay to let server process magic header and send challenge
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Read challenge from server
        let msg = match Self::read_message(&mut stream).await {
            Ok(m) => {
                debug!("authenticate_client: received message");
                m
            }
            Err(e) => {
                debug!("authenticate_client: failed to read challenge: {}", e);
                return Err(e);
            }
        };

        let (challenge, timestamp, nonce) = match msg {
            SyncMessage::AuthChallenge {
                challenge,
                timestamp,
                nonce,
            } => {
                debug!("authenticate_client: got challenge");
                (challenge, timestamp, nonce)
            }
            _ => {
                return Err(TransportError::AuthFailed(
                    "Expected AuthChallenge".to_string(),
                ))
            }
        };

        // Compute HMAC response including timestamp and nonce
        let hmac = self.compute_hmac_with_timestamp(&challenge, timestamp, &nonce)?;

        // Send response
        debug!("authenticate_client: sending response");
        let response = SyncMessage::AuthResponse { hmac };
        Self::write_message(&mut stream, &response).await?;
        debug!("authenticate_client: waiting for result");

        // Read result
        let result = Self::read_message(&mut stream).await?;
        match result {
            SyncMessage::AuthResult { success: true, .. } => {
                debug!("authenticate_client: success");
                Ok(stream)
            }
            SyncMessage::AuthResult {
                success: false,
                message,
            } => {
                debug!("authenticate_client: failed: {}", message);
                Err(TransportError::AuthFailed(message))
            }
            _ => Err(TransportError::AuthFailed(
                "Unexpected response".to_string(),
            )),
        }
    }

    /// Compute HMAC of data with timestamp and nonce using keyfile
    fn compute_hmac_with_timestamp(
        &self,
        data: &[u8],
        timestamp: u64,
        nonce: &[u8],
    ) -> Result<Vec<u8>, TransportError> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let key = std::fs::read(&self.keyfile_path)
            .map_err(|e| TransportError::AuthFailed(format!("Failed to read keyfile: {}", e)))?;

        let mut mac = Hmac::<Sha256>::new_from_slice(&key)
            .map_err(|e| TransportError::AuthFailed(format!("Invalid key: {}", e)))?;
        mac.update(data);
        mac.update(&timestamp.to_be_bytes());
        mac.update(nonce);

        Ok(mac.finalize().into_bytes().to_vec())
    }

    /// Send a message to a peer
    pub async fn send(&self, peer_addr: &str, msg: &SyncMessage) -> Result<(), TransportError> {
        let conn = self
            .get_connection(peer_addr)
            .await
            .ok_or(TransportError::Disconnected)?;
        let mut conn = conn.lock().await;
        Self::write_message_timed(&mut conn.stream, msg).await?;
        conn.last_activity = std::time::Instant::now();
        Ok(())
    }

    /// Receive a message from a peer
    pub async fn receive(&self, peer_addr: &str) -> Result<SyncMessage, TransportError> {
        let conn = self
            .get_connection(peer_addr)
            .await
            .ok_or(TransportError::Disconnected)?;
        let mut conn = conn.lock().await;
        let msg = Self::read_message(&mut conn.stream).await?;
        conn.last_activity = std::time::Instant::now();
        Ok(msg)
    }

    /// Disconnect from a peer
    pub async fn disconnect(&self, peer_addr: &str) {
        self.connections.write().await.remove(peer_addr);
        self.heartbeat_connections.write().await.remove(peer_addr);
        debug!("Disconnected from peer: {}", peer_addr);
    }

    /// Check if connected to a peer
    pub async fn is_connected(&self, peer_addr: &str) -> bool {
        self.connections.read().await.contains_key(peer_addr)
    }

    /// Reconnect to a peer with exponential backoff
    pub async fn reconnect_with_backoff(
        &self,
        peer_addr: &str,
        max_attempts: u32,
    ) -> Result<(), TransportError> {
        use rand::Rng;
        let mut delay = Duration::from_millis(100);

        for attempt in 1..=max_attempts {
            match self.connect(peer_addr).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    warn!(
                        "Connection attempt {} to {} failed: {}",
                        attempt, peer_addr, e
                    );
                    if attempt < max_attempts {
                        tokio::time::sleep(delay).await;
                        delay = std::cmp::min(delay * 2, Duration::from_secs(30));
                        // Add up to ±25% jitter so reconnect storms don't synchronize
                        // (full jitter scales with current delay, not a fixed 25 ms).
                        let quarter = (delay.as_millis() as u64 / 4).max(1);
                        let jitter: u64 = rand::rngs::OsRng.gen_range(0..=quarter);
                        delay += Duration::from_millis(jitter);
                    }
                }
            }
        }

        Err(TransportError::ConnectionFailed(format!(
            "Failed after {} attempts",
            max_attempts
        )))
    }

    /// Write a message to a stream
    pub async fn write_message<T>(stream: &mut T, msg: &SyncMessage) -> Result<(), TransportError>
    where
        T: tokio::io::AsyncWrite + Unpin,
    {
        let payload =
            bincode::serialize(msg).map_err(|e| TransportError::EncodeError(e.to_string()))?;

        // Compress if large
        let (data, compressed) = if payload.len() > COMPRESSION_THRESHOLD {
            let compressed = lz4_flex::compress_prepend_size(&payload);
            (compressed, true)
        } else {
            (payload, false)
        };

        // Write: [compressed_flag: 1 byte] [length: 4 bytes BE] [data]
        let len = data.len() as u32;
        if len > MAX_MESSAGE_SIZE {
            return Err(TransportError::MessageTooLarge(len));
        }

        let mut header = [0u8; 5];
        header[0] = if compressed { 1 } else { 0 };
        header[1..5].copy_from_slice(&len.to_be_bytes());

        stream.write_all(&header).await?;
        stream.write_all(&data).await?;
        stream.flush().await?;

        Ok(())
    }

    /// Read a message from a stream.
    ///
    /// Used for handshake challenges and request/response reads, where a
    /// reply is expected promptly — both reads are bounded so a peer that
    /// sends a partial header (or a length prefix and then stalls) can't
    /// park the task and hold the connection open forever.
    pub async fn read_message<T>(stream: &mut T) -> Result<SyncMessage, TransportError>
    where
        T: tokio::io::AsyncRead + Unpin,
    {
        let timeout_err = || {
            TransportError::IoError(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "sync read timed out",
            ))
        };

        // Read header
        let mut header = [0u8; 5];
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            stream.read_exact(&mut header),
        )
        .await
        .map_err(|_| timeout_err())??;

        let compressed = header[0] == 1;
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);

        if len > MAX_MESSAGE_SIZE {
            return Err(TransportError::MessageTooLarge(len));
        }

        // Read payload
        let mut data = vec![0u8; len as usize];
        tokio::time::timeout(
            std::time::Duration::from_secs(60),
            stream.read_exact(&mut data),
        )
        .await
        .map_err(|_| timeout_err())??;

        // Decompress if needed
        let payload = if compressed {
            super::protocol::decompress_checked(&data).map_err(TransportError::DecodeError)?
        } else {
            data
        };

        // Decode
        bincode::deserialize(&payload).map_err(|e| TransportError::DecodeError(e.to_string()))
    }

    /// Get list of connected peers
    pub async fn connected_peers(&self) -> Vec<String> {
        self.connections.read().await.keys().cloned().collect()
    }
}

/// TCP server for accepting incoming sync connections
pub struct SyncServer {
    listener: Option<TcpListener>,
    keyfile_path: String,
    _local_node_id: String,
}

impl SyncServer {
    /// Bind to address and create server
    pub async fn bind(
        addr: &str,
        keyfile_path: String,
        local_node_id: String,
    ) -> Result<Self, TransportError> {
        let listener = TcpListener::bind(addr).await?;
        info!("Sync server listening on {}", addr);

        Ok(Self {
            listener: Some(listener),
            keyfile_path,
            _local_node_id: local_node_id,
        })
    }

    /// Accept incoming connection and authenticate
    ///
    /// Runs the handshake inline, so a caller looping on this is blocked by
    /// one silent client; accept loops should use [`Self::accept_raw`] and
    /// spawn [`Self::handshake`] per connection (audit A8).
    pub async fn accept(&self) -> Result<(SyncStream, String), TransportError> {
        let (stream, peer_addr) = self.accept_raw().await?;
        let stream = Self::handshake(stream, &self.keyfile_path).await?;
        info!("Authenticated connection from {}", peer_addr);
        Ok((stream, peer_addr))
    }

    /// Accept a TCP connection without authenticating it.
    pub async fn accept_raw(&self) -> Result<(SyncStream, String), TransportError> {
        let listener = self.listener.as_ref().ok_or_else(|| {
            TransportError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No listener",
            ))
        })?;
        let (stream, addr) = listener.accept().await?;
        let peer_addr = addr.to_string();
        debug!("Incoming connection from {}", peer_addr);
        Ok((Box::new(stream), peer_addr))
    }

    /// Keyfile used to authenticate inbound peers.
    pub fn keyfile_path(&self) -> &str {
        &self.keyfile_path
    }

    /// Server-side handshake (magic header + challenge), bounded by
    /// [`SERVER_HANDSHAKE_TIMEOUT`] so a client that connects and sends
    /// nothing cannot hold the task.
    pub async fn handshake(
        stream: SyncStream,
        keyfile_path: &str,
    ) -> Result<SyncStream, TransportError> {
        tokio::time::timeout(
            SERVER_HANDSHAKE_TIMEOUT,
            Self::authenticate_standalone(stream, keyfile_path),
        )
        .await
        .map_err(|_| TransportError::AuthFailed("Handshake timed out".to_string()))?
    }

    /// Authenticate as server (send challenge, verify response)
    pub async fn authenticate_server(
        &self,
        stream: SyncStream,
    ) -> Result<SyncStream, TransportError> {
        Self::authenticate_standalone(stream, &self.keyfile_path).await
    }

    /// Standalone authentication flow (e.g. for multiplexed connections)
    /// If magic_already_verified is true, skip reading the magic header (multiplexer already did it)
    pub async fn authenticate_standalone(
        stream: SyncStream,
        keyfile_path: &str,
    ) -> Result<SyncStream, TransportError> {
        Self::authenticate_standalone_impl(stream, keyfile_path, false).await
    }

    /// Version that skips magic header reading (for multiplexed mode where header was already peeked)
    pub async fn authenticate_standalone_skip_magic(
        stream: SyncStream,
        keyfile_path: &str,
    ) -> Result<SyncStream, TransportError> {
        Self::authenticate_standalone_impl(stream, keyfile_path, true).await
    }

    async fn authenticate_standalone_impl(
        mut stream: SyncStream,
        keyfile_path: &str,
        skip_magic: bool,
    ) -> Result<SyncStream, TransportError> {
        debug!(
            "authenticate_standalone: starting, skip_magic={}, keyfile={}",
            skip_magic, keyfile_path
        );

        if keyfile_path.is_empty() || !std::path::Path::new(keyfile_path).exists() {
            let require_keyfile = std::env::var("SOLIDB_REQUIRE_KEYFILE")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false);
            if require_keyfile || !allow_unauthenticated_sync() {
                warn!("authenticate_standalone: cluster keyfile missing; refusing unauthenticated sync");
                return Err(TransportError::AuthFailed(
                    "Cluster keyfile required (set SOLIDB_ALLOW_UNAUTHENTICATED_SYNC=true only for local tests)".to_string(),
                ));
            }

            warn!("authenticate_standalone: no keyfile found, skipping authentication");
            warn!("WARNING: Inter-node communication is unauthenticated. Set SOLIDB_REQUIRE_KEYFILE=true to enforce authentication.");

            // Still need to handle magic header if not skipped
            if !skip_magic {
                let mut magic = [0u8; 14];
                if let Err(e) = stream.read_exact(&mut magic).await {
                    return Err(TransportError::IoError(e));
                }
                if &magic != b"solidb-sync-v1" {
                    return Err(TransportError::AuthFailed(
                        "Invalid protocol header".to_string(),
                    ));
                }
            }

            return Ok(stream);
        }

        if !skip_magic {
            // Read magic header
            let mut magic = [0u8; 14];
            match stream.read_exact(&mut magic).await {
                Ok(_) => debug!("authenticate_standalone: read magic header"),
                Err(e) => {
                    debug!(
                        "authenticate_standalone: failed to read magic header: {}",
                        e
                    );
                    return Err(TransportError::IoError(e));
                }
            }

            if &magic != b"solidb-sync-v1" {
                return Err(TransportError::AuthFailed(
                    "Invalid protocol header".to_string(),
                ));
            }
        } else {
            debug!("authenticate_standalone: skip magic (multiplexed)");
        }

        // Generate random challenge with timestamp and nonce to prevent replay attacks
        use rand::Rng;
        let challenge: Vec<u8> = rand::rngs::OsRng.gen::<[u8; 32]>().to_vec();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let nonce: Vec<u8> = rand::rngs::OsRng.gen::<[u8; 16]>().to_vec();

        // Send challenge with timestamp and nonce
        debug!("authenticate_standalone: sending challenge");
        let challenge_msg = SyncMessage::AuthChallenge {
            challenge: challenge.clone(),
            timestamp,
            nonce: nonce.clone(),
        };
        ConnectionPool::write_message(&mut stream, &challenge_msg).await?;
        debug!("authenticate_standalone: waiting for response");

        // Read response
        let response = ConnectionPool::read_message(&mut stream).await?;
        debug!("authenticate_standalone: got response");

        let client_hmac = match response {
            SyncMessage::AuthResponse { hmac } => hmac,
            _ => {
                let _ = ConnectionPool::write_message(
                    &mut stream,
                    &SyncMessage::AuthResult {
                        success: false,
                        message: "Expected AuthResponse".to_string(),
                    },
                )
                .await;
                return Err(TransportError::AuthFailed(
                    "Expected AuthResponse".to_string(),
                ));
            }
        };

        // The 32-byte random challenge already prevents replay; timestamp here
        // bounds how long the handshake may take (clients that delay past this
        // window are rejected, limiting slow-loris on the auth path).
        const HANDSHAKE_MAX_AGE_MS: u64 = 30_000;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if now_ms.saturating_sub(timestamp) > HANDSHAKE_MAX_AGE_MS {
            return Err(TransportError::AuthFailed(
                "Auth handshake timed out".to_string(),
            ));
        }

        let expected_hmac =
            Self::compute_hmac_with_timestamp(&challenge, timestamp, &nonce, keyfile_path)?;

        if crate::server::auth::constant_time_eq(&client_hmac, &expected_hmac) {
            let _ = ConnectionPool::write_message(
                &mut stream,
                &SyncMessage::AuthResult {
                    success: true,
                    message: "OK".to_string(),
                },
            )
            .await;
            Ok(stream)
        } else {
            Err(TransportError::AuthFailed("Invalid HMAC".to_string()))
        }
    }

    fn compute_hmac_with_timestamp(
        data: &[u8],
        timestamp: u64,
        nonce: &[u8],
        keyfile_path: &str,
    ) -> Result<Vec<u8>, TransportError> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let key = std::fs::read(keyfile_path).map_err(|e| {
            TransportError::AuthFailed(format!("Failed to read keyfile {}: {}", keyfile_path, e))
        })?;

        let mut mac = Hmac::<Sha256>::new_from_slice(&key)
            .map_err(|e| TransportError::AuthFailed(format!("Invalid key: {}", e)))?;
        mac.update(data);
        mac.update(&timestamp.to_be_bytes());
        mac.update(nonce);

        Ok(mac.finalize().into_bytes().to_vec())
    }
}
