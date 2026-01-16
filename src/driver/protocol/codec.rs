use super::command::Command;
use super::error::DriverError;
use super::response::Response;
use serde::{Deserialize, Serialize};

/// Magic header sent at the start of a driver connection
pub const DRIVER_MAGIC: &[u8] = b"solidb-drv-v1\0";

/// Maximum message size (16 MB)
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

/// Helper to encode a command with length prefix (uses compact/fast serialization)
/// Commands are sent from client to server
pub fn encode_command(cmd: &Command) -> Result<Vec<u8>, DriverError> {
    // Use named serialization for commands (required for tagged enums)
    let payload = rmp_serde::to_vec_named(cmd)
        .map_err(|e| DriverError::ProtocolError(format!("Serialization failed: {}", e)))?;

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(DriverError::MessageTooLarge);
    }

    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Helper to encode a response with length prefix (uses named serialization for compatibility)
/// Responses are sent from server to client
pub fn encode_response(resp: &Response) -> Result<Vec<u8>, DriverError> {
    // Use named serialization for responses (required for tagged enums + external clients)
    let payload = rmp_serde::to_vec_named(resp)
        .map_err(|e| DriverError::ProtocolError(format!("Serialization failed: {}", e)))?;

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(DriverError::MessageTooLarge);
    }

    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Helper to encode a generic message with length prefix
pub fn encode_message<T: Serialize>(msg: &T) -> Result<Vec<u8>, DriverError> {
    // Use named serialization to ensure maps are serialized with string keys
    let payload = rmp_serde::to_vec_named(msg)
        .map_err(|e| DriverError::ProtocolError(format!("Serialization failed: {}", e)))?;

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(DriverError::MessageTooLarge);
    }

    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Helper to decode a message from bytes
pub fn decode_message<T: for<'de> Deserialize<'de>>(data: &[u8]) -> Result<T, DriverError> {
    rmp_serde::from_slice(data)
        .map_err(|e| DriverError::ProtocolError(format!("Deserialization failed: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::protocol::{Command, Response};

    #[test]
    fn test_command_serialization() {
        let cmd = Command::Get {
            database: "test".to_string(),
            collection: "users".to_string(),
            key: "user1".to_string(),
        };

        let encoded = encode_message(&cmd).unwrap();
        assert!(encoded.len() > 4);

        // Decode (skip length prefix)
        let decoded: Command = decode_message(&encoded[4..]).unwrap();
        match decoded {
            Command::Get {
                database,
                collection,
                key,
            } => {
                assert_eq!(database, "test");
                assert_eq!(collection, "users");
                assert_eq!(key, "user1");
            }
            _ => panic!("Wrong command type"),
        }
    }

    #[test]
    fn test_response_serialization() {
        let resp = Response::ok(serde_json::json!({"name": "Alice"}));
        let encoded = encode_message(&resp).unwrap();
        let decoded: Response = decode_message(&encoded[4..]).unwrap();

        match decoded {
            Response::Ok { data, .. } => {
                assert_eq!(data.unwrap()["name"], "Alice");
            }
            _ => panic!("Wrong response type"),
        }
    }
}
