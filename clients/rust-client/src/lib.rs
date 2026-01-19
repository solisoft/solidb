//! SoliDB Rust Client
//!
//! High-performance native driver client for SoliDB using MessagePack binary protocol.
//!
//! # Example
//!
//! ```rust
//! use solidb_client::SoliDBClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), solidb_client::DriverError> {
//!     let mut client = SoliDBClient::connect("localhost:6745").await?;
//!     let version = client.ping().await?;
//!     println!("Server version: {}", version);
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod protocol;

pub use client::{SoliDBClient, SoliDBClientBuilder};
pub use protocol::{Command, DriverError, Response};
