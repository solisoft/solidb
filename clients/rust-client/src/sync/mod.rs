//! Offline Sync Module
//!
//! Provides offline-first synchronization capabilities for the SoliDB Rust client.
//!
//! # Features
//! - Local SQLite storage for offline data
//! - Automatic sync when online
//! - Conflict detection and resolution
//! - Version vector tracking
//!
//! # Example
//!
//! ```rust
//! use solidb_client::{SoliDBClientBuilder, SyncManager, SyncConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create HTTP client
//!     let client = SoliDBClientBuilder::new("http://localhost:6745")
//!         .auth("mydb", "admin", "password")
//!         .build_http()
//!         .await?;
//!
//!     // Create local store
//!     let local_store = LocalStore::open_default("myapp", "device-123".to_string())?;
//!
//!     // Create sync manager
//!     let config = SyncConfig::default();
//!     let mut sync_manager = SyncManager::new(local_store, client, config);
//!     let _command_tx = sync_manager.start().await;
//!
//!     // Save a document (works offline)
//!     let doc = serde_json::json!({"name": "Alice", "score": 100});
//!     sync_manager.save_document("users", "user-1", &doc).await?;
//!
//!     // Query documents locally
//!     let users = sync_manager.query_documents("users").await?;
//!     println!("Local users: {:?}", users);
//!
//!     // Trigger sync manually
//!     sync_manager.sync_now().await;
//!
//!     Ok(())
//! }
//! ```

pub mod manager;
pub mod store;

pub use manager::{SyncCommand, SyncConfig, SyncManager, SyncResult};
pub use store::{LocalStore, PendingChange};
