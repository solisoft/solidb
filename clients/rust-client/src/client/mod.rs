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
use tokio::net::TcpStream;

use super::protocol::{
    decode_message, encode_command, Command, DriverError, Response, DRIVER_MAGIC, MAX_MESSAGE_SIZE,
};

pub struct SoliDBClient {
    pub(crate) stream: TcpStream,
    pub(crate) current_tx: Option<String>,
}

impl SoliDBClient {
    pub async fn connect(addr: &str) -> Result<Self, DriverError> {
        let stream = TcpStream::connect(addr).await.map_err(|e| {
            DriverError::ConnectionError(format!("Failed to connect to {}: {}", addr, e))
        })?;

        let mut client = Self {
            stream,
            current_tx: None,
        };

        client.stream.write_all(DRIVER_MAGIC).await.map_err(|e| {
            DriverError::ConnectionError(format!("Failed to send magic header: {}", e))
        })?;
        client
            .stream
            .flush()
            .await
            .map_err(|e| DriverError::ConnectionError(format!("Failed to flush: {}", e)))?;

        Ok(client)
    }

    pub(crate) async fn send_command(&mut self, command: Command) -> Result<Response, DriverError> {
        let data = encode_command(&command)?;
        self.stream
            .write_all(&data)
            .await
            .map_err(|e| DriverError::ConnectionError(format!("Write failed: {}", e)))?;
        self.stream
            .flush()
            .await
            .map_err(|e| DriverError::ConnectionError(format!("Flush failed: {}", e)))?;

        let mut len_buf = [0u8; 4];
        self.stream
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| DriverError::ConnectionError(format!("Read length failed: {}", e)))?;

        let msg_len = u32::from_be_bytes(len_buf) as usize;
        if msg_len > MAX_MESSAGE_SIZE {
            return Err(DriverError::MessageTooLarge);
        }

        let mut payload = vec![0u8; msg_len];
        self.stream
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
