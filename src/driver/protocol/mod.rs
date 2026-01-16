//! Wire protocol definitions for the native driver
//!
//! Uses MessagePack for efficient binary serialization.

pub mod codec;
pub mod command;
pub mod error;
pub mod response;
pub mod types;

pub use codec::{
    decode_message, encode_command, encode_message, encode_response, DRIVER_MAGIC, MAX_MESSAGE_SIZE,
};
pub use command::Command;
pub use error::DriverError;
pub use response::Response;
pub use types::IsolationLevel;
