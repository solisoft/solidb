//! SoliDB Rust Client
//!
//! High-performance native driver client for SoliDB with HTTP and TCP transport support.
//!
//! # HTTP Example (Default)
//!
//! ```rust
//! use solidb_client::{SoliDBClientBuilder, HttpClient};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), solidb_client::DriverError> {
//!     let client = SoliDBClientBuilder::new("http://localhost:6745")
//!         .auth("mydb", "admin", "password")
//!         .build_http()
//!         .await?;
//!
//!     let databases = client.list_databases().await?;
//!     println!("Databases: {:?}", databases);
//!     Ok(())
//! }
//! ```
//!
//! # TCP Example
//!
//! ```rust
//! use solidb_client::SoliDBClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), solidb_client::DriverError> {
//!     let mut client = SoliDBClientBuilder::new("localhost:6745")
//!         .use_tcp()
//!         .auth("mydb", "admin", "password")
//!         .build()
//!         .await?;
//!
//!     let version = client.ping().await?;
//!     println!("Server version: {}", version);
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod protocol;

pub use client::{SoliDBClient, SoliDBClientBuilder, HttpClient, Transport};
pub use protocol::{Command, DriverError, Response};
