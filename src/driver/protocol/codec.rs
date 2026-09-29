use super::command::Command;
use super::error::DriverError;
use super::response::Response;
use serde::{Deserialize, Serialize};

pub const DRIVER_MAGIC: &[u8] = b"solidb-drv-v1\0";
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

pub fn encode_command(cmd: &Command) -> Result<Vec<u8>, DriverError> {
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

pub fn encode_response(resp: &Response) -> Result<Vec<u8>, DriverError> {
    // Serialized in place after a length placeholder, rather than into its
    // own buffer and then copied behind the length: one allocation and no
    // copy of what can be a large result set.
    let mut buf = Vec::with_capacity(256);
    buf.extend_from_slice(&[0u8; 4]);
    rmp_serde::encode::write_named(&mut buf, resp)
        .map_err(|e| DriverError::ProtocolError(format!("Serialization failed: {}", e)))?;

    let payload_len = buf.len() - 4;
    if payload_len > MAX_MESSAGE_SIZE {
        return Err(DriverError::MessageTooLarge);
    }
    buf[..4].copy_from_slice(&(payload_len as u32).to_be_bytes());
    Ok(buf)
}

pub fn encode_message<T: Serialize>(msg: &T) -> Result<Vec<u8>, DriverError> {
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

pub fn decode_message<T: for<'de> Deserialize<'de>>(data: &[u8]) -> Result<T, DriverError> {
    rmp_serde::from_slice(data)
        .map_err(|e| DriverError::ProtocolError(format!("Deserialization failed: {}", e)))
}
