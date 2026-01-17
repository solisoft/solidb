pub mod auth;
pub mod blobs;
pub mod cluster;
pub mod collections;
pub mod databases;
pub mod documents;
pub mod import_export;
pub mod indexes;
pub mod query;
pub mod schema;
pub mod sharding;
pub mod system;
pub mod websocket;

// Re-export all handlers to maintain compatibility with routes.rs
pub use auth::*;
pub use blobs::*;
pub use cluster::*;
pub use collections::*;
pub use databases::*;
pub use documents::*;
pub use import_export::*;
pub use indexes::*;
pub use query::*;
pub use schema::*;
pub use sharding::*;
pub use system::*;
pub use websocket::*;
