//! Native driver module for direct database access
//!
//! This module provides a high-performance binary protocol for database operations,
//! offering lower latency than the HTTP REST API by using MessagePack serialization
//! and persistent connections.
//!
//! # Protocol Overview
//!
//! The driver protocol uses a simple framed message format:
//! - **Magic Header**: `solidb-drv-v1` (14 bytes, sent once on connection)
//! - **Request Frame**: `[length: 4 bytes BE][msgpack payload]`
//! - **Response Frame**: `[length: 4 bytes BE][msgpack payload]`

pub use solidb_client::protocol::{
    decode_message, encode_command, encode_response, Command, DriverError, Response,
    MAX_MESSAGE_SIZE,
};
pub use solidb_client::SoliDBClient;

pub mod handlers;

pub use handlers::spawn_driver_handler;
pub use handlers::DriverHandler;
