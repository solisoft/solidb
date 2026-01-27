mod builder;
mod bulk;
mod collection;
mod database;
mod document;
mod index;
mod query;
mod transaction;

pub use builder::SoliDBClientBuilder;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

use super::protocol::{
    decode_message, encode_command, Command, DriverError, Response, DRIVER_MAGIC, MAX_MESSAGE_SIZE,
};

const DEFAULT_POOL_SIZE: usize = 4;

struct PooledConnection {
    read: OwnedReadHalf,
    write: OwnedWriteHalf,
}

pub struct SoliDBClient {
    pool: Vec<PooledConnection>,
    next_index: usize,
    current_tx: Option<String>,
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

            pool_connections.push(PooledConnection { read, write });
        }

        Ok(Self {
            pool: pool_connections,
            next_index: 0,
            current_tx: None,
        })
    }

    fn get_next_connection(&mut self) -> &mut PooledConnection {
        let idx = self.next_index;
        self.next_index = (self.next_index + 1) % self.pool.len();
        &mut self.pool[idx]
    }

    pub(crate) async fn send_command(&mut self, command: Command) -> Result<Response, DriverError> {
        let conn = self.get_next_connection();

        let data = encode_command(&command)?;
        conn.write
            .write_all(&data)
            .await
            .map_err(|e| DriverError::ConnectionError(format!("Write failed: {}", e)))?;
        conn.write
            .flush()
            .await
            .map_err(|e| DriverError::ConnectionError(format!("Flush failed: {}", e)))?;

        let mut len_buf = [0u8; 4];
        conn.read
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| DriverError::ConnectionError(format!("Read length failed: {}", e)))?;

        let msg_len = u32::from_be_bytes(len_buf) as usize;
        if msg_len > MAX_MESSAGE_SIZE {
            return Err(DriverError::MessageTooLarge);
        }

        let mut payload = vec![0u8; msg_len];
        conn.read
            .read_exact(&mut payload)
            .await
            .map_err(|e| DriverError::ConnectionError(format!("Read payload failed: {}", e)))?;

        decode_message(&payload)
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
        let response = self
            .send_command(Command::Auth {
                database: database.to_string(),
                username: username.to_string(),
                password: password.to_string(),
                api_key: None,
            })
            .await?;

        match response {
            Response::Ok { .. } => Ok(()),
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError(
                "Unexpected response".to_string(),
            )),
        }
    }

    pub async fn auth_with_api_key(
        &mut self,
        database: &str,
        api_key: &str,
    ) -> Result<(), DriverError> {
        let response = self
            .send_command(Command::Auth {
                database: database.to_string(),
                username: String::new(),
                password: String::new(),
                api_key: Some(api_key.to_string()),
            })
            .await?;

        match response {
            Response::Ok { .. } => Ok(()),
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError(
                "Unexpected response".to_string(),
            )),
        }
    }
}
