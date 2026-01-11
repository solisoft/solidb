//! TCP transport layer for sync communication
//!
//! Provides persistent TCP connections between nodes with:
//! - Connection pooling
//! - HMAC authentication
//! - Automatic reconnection with exponential backoff
//! - LZ4 compression for large payloads

use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::protocol::SyncMessage;

/// Maximum message size (10 MB)
const MAX_MESSAGE_SIZE: u32 = 10 * 1024 * 1024;

/// Compression threshold (64 KB)
const COMPRESSION_THRESHOLD: usize = 64 * 1024;

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

/// Connection pool for managing peer connections
pub struct ConnectionPool {
    connections: RwLock<HashMap<String, PeerConnection>>,
    _local_node_id: String,
    keyfile_path: String,
}

impl ConnectionPool {
    pub fn new(local_node_id: String, keyfile_path: String) -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
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

        debug!("ConnectionPool: Connecting to peer: {}", peer_addr);

        let stream = match TcpStream::connect(peer_addr).await {
            Ok(s) => {
                debug!("ConnectionPool: TCP connected to {}", peer_addr);
                s
            }
            Err(e) => {
                debug!(
                    "ConnectionPool: TCP connection failed to {}: {}",
                    peer_addr, e
                );
                return Err(TransportError::ConnectionFailed(format!(
                    "{}: {}",
                    peer_addr, e
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
        let stream = match self.authenticate_client(stream).await {
            Ok(s) => s,
            Err(e) => {
                debug!(
                    "ConnectionPool: Authentication failed with {}: {}",
                    peer_addr, e
                );
                return Err(e);
            }
        };

        // Store connection
        self.connections.write().await.insert(
            peer_addr.to_string(),
            PeerConnection {
                stream,
                last_activity: std::time::Instant::now(),
            },
        );

        debug!("ConnectionPool: Connected to peer: {}", peer_addr);
        Ok(())
    }

    /// Authenticate as client (respond to server's challenge)
    async fn authenticate_client(
        &self,
        mut stream: TcpStream,
    ) -> Result<TcpStream, TransportError> {
        // If no keyfile, skip authentication
        if self.keyfile_path.is_empty() || !std::path::Path::new(&self.keyfile_path).exists() {
            debug!("authenticate_client: no keyfile, skipping");
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

        let challenge = match msg {
            SyncMessage::AuthChallenge { challenge } => {
                debug!("authenticate_client: got challenge");
                challenge
            }
            _ => {
                return Err(TransportError::AuthFailed(
                    "Expected AuthChallenge".to_string(),
                ))
            }
        };

        // Compute HMAC response
        let hmac = self.compute_hmac(&challenge)?;

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

    /// Compute HMAC of data using keyfile
    fn compute_hmac(&self, data: &[u8]) -> Result<Vec<u8>, TransportError> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let key = std::fs::read(&self.keyfile_path)
            .map_err(|e| TransportError::AuthFailed(format!("Failed to read keyfile: {}", e)))?;

        let mut mac = Hmac::<Sha256>::new_from_slice(&key)
            .map_err(|e| TransportError::AuthFailed(format!("Invalid key: {}", e)))?;
        mac.update(data);

        Ok(mac.finalize().into_bytes().to_vec())
    }

    /// Send a message to a peer
    pub async fn send(&self, peer_addr: &str, msg: &SyncMessage) -> Result<(), TransportError> {
        let mut conns = self.connections.write().await;

        if let Some(conn) = conns.get_mut(peer_addr) {
            Self::write_message(&mut conn.stream, msg).await?;
            conn.last_activity = std::time::Instant::now();
            Ok(())
        } else {
            Err(TransportError::Disconnected)
        }
    }

    /// Receive a message from a peer
    pub async fn receive(&self, peer_addr: &str) -> Result<SyncMessage, TransportError> {
        let mut conns = self.connections.write().await;

        if let Some(conn) = conns.get_mut(peer_addr) {
            let msg = Self::read_message(&mut conn.stream).await?;
            conn.last_activity = std::time::Instant::now();
            Ok(msg)
        } else {
            Err(TransportError::Disconnected)
        }
    }

    /// Disconnect from a peer
    pub async fn disconnect(&self, peer_addr: &str) {
        self.connections.write().await.remove(peer_addr);
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

    /// Read a message from a stream
    pub async fn read_message<T>(stream: &mut T) -> Result<SyncMessage, TransportError>
    where
        T: tokio::io::AsyncRead + Unpin,
    {
        // Read header
        let mut header = [0u8; 5];
        stream.read_exact(&mut header).await?;

        let compressed = header[0] == 1;
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);

        if len > MAX_MESSAGE_SIZE {
            return Err(TransportError::MessageTooLarge(len));
        }

        // Read payload
        let mut data = vec![0u8; len as usize];
        stream.read_exact(&mut data).await?;

        // Decompress if needed
        let payload = if compressed {
            lz4_flex::decompress_size_prepended(&data)
                .map_err(|e| TransportError::DecodeError(format!("Decompression failed: {}", e)))?
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
    pub async fn accept(&self) -> Result<(SyncStream, String), TransportError> {
        let listener = self.listener.as_ref().ok_or_else(|| {
            TransportError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No listener",
            ))
        })?;
        let (stream, addr) = listener.accept().await?;
        let peer_addr = addr.to_string();

        debug!("Incoming connection from {}", peer_addr);

        // Authenticate the client
        let stream: SyncStream = Box::new(stream);
        let stream = self.authenticate_server(stream).await?;

        info!("Authenticated connection from {}", peer_addr);
        Ok((stream, peer_addr))
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

        // If no keyfile provided or it doesn't exist, skip authentication (for dev/test)
        if keyfile_path.is_empty() || !std::path::Path::new(keyfile_path).exists() {
            debug!("authenticate_standalone: no keyfile found, skipping authentication");

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

        // Generate random challenge
        use rand::Rng;
        let challenge: Vec<u8> = rand::thread_rng().gen::<[u8; 32]>().to_vec();

        // Send challenge
        debug!("authenticate_standalone: sending challenge");
        let challenge_msg = SyncMessage::AuthChallenge {
            challenge: challenge.clone(),
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

        // Verify HMAC
        let expected_hmac = Self::compute_hmac_static(&challenge, keyfile_path)?;

        if client_hmac == expected_hmac {
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

    fn compute_hmac_static(data: &[u8], keyfile_path: &str) -> Result<Vec<u8>, TransportError> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let key = std::fs::read(keyfile_path).map_err(|e| {
            TransportError::AuthFailed(format!("Failed to read keyfile {}: {}", keyfile_path, e))
        })?;

        let mut mac = Hmac::<Sha256>::new_from_slice(&key)
            .map_err(|e| TransportError::AuthFailed(format!("Invalid key: {}", e)))?;
        mac.update(data);

        Ok(mac.finalize().into_bytes().to_vec())
    }
}
