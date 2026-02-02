//! SoliDB Rust Client
//!
//! High-performance native driver client for SoliDB with HTTP and TCP transport support.
//! Includes offline-first synchronization capabilities.
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
//! # Offline Sync Example
//!
//! ```rust
//! use solidb_client::{SoliDBClientBuilder, SyncManager, SyncConfig, LocalStore};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create client and sync manager
//!     let client = SoliDBClientBuilder::new("http://localhost:6745")
//!         .build_http()
//!         .await?;
//!     
//!     let local_store = LocalStore::open_default("myapp", "device-123".to_string())?;
//!     let mut sync = SyncManager::new(local_store, client, SyncConfig::default());
//!     sync.start().await;
//!     
//!     // Works offline!
//!     sync.save_document("docs", "doc-1", &serde_json::json!({"title": "Hello"})).await?;
//!     
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod protocol;
pub mod sync;

// Mobile FFI module - for iOS and Android bindings (requires 'mobile' feature)
#[cfg(feature = "mobile")]
pub mod mobile_ffi;

pub use client::{HttpClient, SoliDBClient, SoliDBClientBuilder, Transport};
pub use protocol::{Command, DriverError, Response};
pub use sync::{LocalStore, PendingChange, SyncCommand, SyncConfig, SyncManager, SyncResult};

// Re-export mobile types when building for mobile
#[cfg(feature = "mobile")]
pub use mobile_ffi::{SyncConfig as MobileSyncConfig, SyncManager as MobileSyncManager};
