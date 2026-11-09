//! Wire protocol definitions for the native driver
//!
//! Uses MessagePack for efficient binary serialization.

pub mod codec;
pub mod command;
pub mod error;
pub mod response;
pub mod types;

pub use codec::{
    decode_command, decode_message, encode_command, encode_message, encode_response,
    parse_raw_write, RawWrite, DRIVER_MAGIC, MAX_MESSAGE_SIZE,
};
pub use command::{Command, TransactionStep};
pub use error::DriverError;
pub use response::Response;
pub use types::IsolationLevel;
