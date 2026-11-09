mod builder;
mod bulk;
mod collection;
mod database;
mod document;
mod http_client;
mod index;
mod query;
mod transaction;

pub use builder::SoliDBClientBuilder;
pub use builder::Transport;
pub use http_client::HttpClient;

use serde_json::Value;
use std::ops::Range;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

use super::protocol::codec::encode_command_into;
use super::protocol::{
    decode_response, Command, DriverError, Response, DRIVER_MAGIC, MAX_MESSAGE_SIZE,
};

const DEFAULT_POOL_SIZE: usize = 4;

/// What a connection's read buffer starts with. A response that fits is read
/// with one `recv`, and a read that leaves room in the buffer is what tells
/// tokio the socket is drained — so the next request waits for its response
/// without first trying a `recv` that can only say "not yet".
const READ_BUFFER: usize = 32 * 1024;

/// A connection keeps buffers that grew for a large frame up to this size;
/// a larger one is given back once the frame has been handled.
const KEEP_BUFFER: usize = 1024 * 1024;

struct PooledConnection {
    read: OwnedReadHalf,
    write: OwnedWriteHalf,
    /// Bytes read from the socket: the frames handed out end at `consumed`,
    /// anything after it belongs to the next one.
    rbuf: Vec<u8>,
    consumed: usize,
    /// The request frame, encoded in place each time.
    wbuf: Vec<u8>,
}

impl PooledConnection {
    fn new(read: OwnedReadHalf, write: OwnedWriteHalf) -> Self {
        Self {
            read,
            write,
            rbuf: Vec::with_capacity(READ_BUFFER),
            consumed: 0,
            wbuf: Vec::new(),
        }
    }

    /// Send `command` as one frame: its length and payload in one `write`.
    async fn send(&mut self, command: &Command) -> Result<(), DriverError> {
        self.wbuf.clear();
        encode_command_into(command, &mut self.wbuf)?;
        let sent = self.write.write_all(&self.wbuf).await;
        if self.wbuf.capacity() > KEEP_BUFFER {
            self.wbuf = Vec::new();
        }
        sent.map_err(|e| DriverError::ConnectionError(format!("Write failed: {}", e)))?;
        self.write
            .flush()
            .await
            .map_err(|e| DriverError::ConnectionError(format!("Flush failed: {}", e)))
    }

    /// Read the next frame and return where its payload lies in `rbuf`.
    ///
    /// The payload is read into the connection's buffer, where it is
    /// decoded in place: no buffer allocated and zeroed for each response,
    /// and the length and the payload usually come in the same `recv`.
    /// Bytes past the frame stay buffered for the next call, as they stayed
    /// in the socket when the frame was read with two `read_exact`s; the
    /// errors are theirs too.
    async fn receive(&mut self) -> Result<Range<usize>, DriverError> {
        self.discard_consumed();
        self.fill(4, "Read length failed").await?;
        let msg_len =
            u32::from_be_bytes([self.rbuf[0], self.rbuf[1], self.rbuf[2], self.rbuf[3]]) as usize;
        if msg_len > MAX_MESSAGE_SIZE {
            self.consumed = 4;
            return Err(DriverError::MessageTooLarge);
        }
        let end = 4 + msg_len;
        self.fill(end, "Read payload failed").await?;
        self.consumed = end;
        Ok(4..end)
    }

    fn discard_consumed(&mut self) {
        if self.consumed == self.rbuf.len() {
            self.rbuf.clear();
        } else {
            self.rbuf.drain(..self.consumed);
        }
        self.consumed = 0;
    }

    /// Read until `rbuf` holds at least `want` bytes, into its spare
    /// capacity: as much as the socket has, never less than what is missing.
    async fn fill(&mut self, want: usize, what: &str) -> Result<(), DriverError> {
        while self.rbuf.len() < want {
            self.rbuf.reserve(want - self.rbuf.len());
            let n = self
                .read
                .read_buf(&mut self.rbuf)
                .await
                .map_err(|e| DriverError::ConnectionError(format!("{}: {}", what, e)))?;
            if n == 0 {
                // What `read_exact` reports when the socket closes first.
                return Err(DriverError::ConnectionError(format!("{}: early eof", what)));
            }
        }
        Ok(())
    }

    /// One command, its response handed to `decode` where it was read.
    async fn round_trip<T>(
        &mut self,
        command: &Command,
        decode: impl FnOnce(&[u8]) -> Result<T, DriverError>,
    ) -> Result<T, DriverError> {
        self.send(command).await?;
        let payload = self.receive().await?;
        let decoded = decode(&self.rbuf[payload]);
        if self.rbuf.capacity() > KEEP_BUFFER {
            self.discard_consumed();
            self.rbuf.shrink_to(READ_BUFFER);
        }
        decoded
    }
}

pub struct SoliDBClient {
    pool: Vec<PooledConnection>,
    next_index: usize,
    current_tx: Option<OpenTransaction>,
}

/// The transaction `begin_transaction` opened, and the pooled connection it
/// lives on: the server keeps a driver transaction per connection, so every
/// command of the transaction, its commit and its rollback must go there.
pub(crate) struct OpenTransaction {
    pub(crate) id: String,
    pub(crate) connection: usize,
}

impl SoliDBClient {
    pub async fn connect(addr: &str) -> Result<Self, DriverError> {
        Self::connect_with_pool(addr, DEFAULT_POOL_SIZE).await
    }

    pub async fn connect_with_pool(addr: &str, pool_size: usize) -> Result<Self, DriverError> {
        let mut pool_connections: Vec<PooledConnection> = Vec::with_capacity(pool_size);

        for _ in 0..pool_size {
            let stream = TcpStream::connect(addr).await.map_err(|e| {
                DriverError::ConnectionError(format!("Failed to connect to {}: {}", addr, e))
            })?;

            stream.set_nodelay(true).map_err(|e| {
                DriverError::ConnectionError(format!("Failed to set TCP_NODELAY: {}", e))
            })?;

            let (read, mut write) = stream.into_split();

            write.write_all(DRIVER_MAGIC).await.map_err(|e| {
                DriverError::ConnectionError(format!("Failed to send magic header: {}", e))
            })?;

            pool_connections.push(PooledConnection::new(read, write));
        }

        Ok(Self {
            pool: pool_connections,
            next_index: 0,
            current_tx: None,
        })
    }

    fn next_connection_index(&mut self) -> usize {
        let idx = self.next_index;
        self.next_index = (self.next_index + 1) % self.pool.len();
        idx
    }

    pub(crate) async fn send_command(&mut self, command: Command) -> Result<Response, DriverError> {
        let (idx, command) = self.route(command)?;
        self.send_command_on(idx, command).await
    }

    /// Where `command` goes, and in what form.
    ///
    /// Outside a transaction: the next pooled connection, as it is. Inside
    /// one, the server stages `Insert`, `Update` and `Delete` and runs a
    /// read-only `Query` only when they arrive as a `TransactionCommand` on
    /// the connection that began the transaction, so they are wrapped and
    /// sent there; a write sent any other way would apply at once and survive
    /// a rollback. `BulkInsert` and `Batch` cannot be staged and are refused
    /// here rather than applied outside the transaction. Everything else
    /// (reads by key, listings, schema changes) is not part of the
    /// transaction and goes out as it is.
    fn route(&mut self, command: Command) -> Result<(usize, Command), DriverError> {
        let Some(tx) = &self.current_tx else {
            return Ok((self.next_connection_index(), command));
        };
        let (tx_id, connection) = (tx.id.clone(), tx.connection);
        match command {
            Command::Insert { .. }
            | Command::Update { .. }
            | Command::Delete { .. }
            | Command::Query { .. } => Ok((
                connection,
                Command::TransactionCommand {
                    tx_id,
                    command: Box::new(command),
                },
            )),
            Command::BulkInsert { .. } | Command::Batch { .. } => {
                Err(DriverError::TransactionError(
                    "bulk_insert and batch cannot run inside a transaction: use insert, \
                     update and delete, or commit first"
                        .to_string(),
                ))
            }
            Command::BeginTransaction { .. } => Err(DriverError::TransactionError(
                "a transaction is already open on this client: commit or roll it back first"
                    .to_string(),
            )),
            other => Ok((self.next_connection_index(), other)),
        }
    }

    /// Send on one specific pooled connection.
    ///
    /// Needed because **authentication is per-socket state** in this protocol:
    /// the server records it against the connection, not against a client
    /// identity or token. `auth()` therefore has to reach every connection in
    /// the pool, which it cannot do through the round-robin `send_command`.
    pub(crate) async fn send_command_on(
        &mut self,
        idx: usize,
        command: Command,
    ) -> Result<Response, DriverError> {
        self.pool[idx].round_trip(&command, decode_response).await
    }

    /// Send a command and decode its response payload with `decode`, so a
    /// caller can deserialize it straight into its own types (see
    /// [`query_as`](Self::query_as)).
    pub(crate) async fn send_command_with<T>(
        &mut self,
        command: Command,
        decode: impl FnOnce(&[u8]) -> Result<T, DriverError>,
    ) -> Result<T, DriverError> {
        let (idx, command) = self.route(command)?;
        self.pool[idx].round_trip(&command, decode).await
    }

    pub(crate) fn extract_data(response: Response) -> Result<Option<Value>, DriverError> {
        match response {
            Response::Ok { data, .. } => Ok(data),
            Response::Error { error } => Err(error),
            Response::Pong { .. } => Ok(None),
            Response::Batch { .. } => Ok(None),
        }
    }

    pub(crate) fn extract_tx_id(response: Response) -> Result<String, DriverError> {
        match response {
            Response::Ok {
                tx_id: Some(id), ..
            } => Ok(id),
            Response::Ok { .. } => Err(DriverError::ProtocolError(
                "Expected transaction ID".to_string(),
            )),
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError(
                "Unexpected response type".to_string(),
            )),
        }
    }

    pub async fn ping(&mut self) -> Result<i64, DriverError> {
        let response = self.send_command(Command::Ping).await?;
        match response {
            Response::Pong { timestamp } => Ok(timestamp),
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError(
                "Expected pong response".to_string(),
            )),
        }
    }

    pub async fn auth(
        &mut self,
        database: &str,
        username: &str,
        password: &str,
    ) -> Result<(), DriverError> {
        self.authenticate_pool(Command::Auth {
            database: database.to_string(),
            username: username.to_string(),
            password: password.to_string(),
            api_key: None,
        })
        .await
    }

    pub async fn auth_with_api_key(
        &mut self,
        database: &str,
        api_key: &str,
    ) -> Result<(), DriverError> {
        self.authenticate_pool(Command::Auth {
            database: database.to_string(),
            username: String::new(),
            password: String::new(),
            api_key: Some(api_key.to_string()),
        })
        .await
    }

    /// Authenticate **every** connection in the pool.
    ///
    /// Authentication is per-socket state here, so sending `Auth` through the
    /// round-robin `send_command` authenticates exactly one connection and
    /// leaves the rest bare. The next command lands on a bare one and fails with
    /// `Authentication required` — which made every `pool_size > 1` client
    /// unusable, including the `benchmark` binary in this crate, and is why the
    /// TCP transport looked broken rather than merely unmeasured.
    async fn authenticate_pool(&mut self, command: Command) -> Result<(), DriverError> {
        for idx in 0..self.pool.len() {
            let response = self.send_command_on(idx, command.clone()).await?;
            match response {
                Response::Ok { .. } => {}
                Response::Error { error } => return Err(error),
                _ => {
                    return Err(DriverError::ProtocolError(
                        "Unexpected response".to_string(),
                    ))
                }
            }
        }
        Ok(())
    }
}
