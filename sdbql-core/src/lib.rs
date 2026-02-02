//! SDBQL Core - Storage-independent SDBQL query language parser and executor.
//!
//! This crate provides the core components for parsing and executing SDBQL queries
//! without any storage engine dependencies. It can be used by both server and client
//! implementations.
//!
//! # Main Components
//!
//! - **Parser**: Parses SDBQL query strings into an AST
//! - **AST**: Abstract syntax tree representation of queries
//! - **Executor**: Executes queries against a `DataSource` trait implementation
//!
//! # Example
//!
//! ```rust
//! use sdbql_core::{LocalExecutor, InMemoryDataSource};
//! use serde_json::json;
//!
//! // Create an in-memory data source for testing
//! let mut ds = InMemoryDataSource::new();
//! ds.insert("users", "1", json!({"name": "Alice", "age": 30}));
//! ds.insert("users", "2", json!({"name": "Bob", "age": 25}));
//!
//! // Create executor
//! let executor = LocalExecutor::new(ds);
//!
//! // Execute query
//! let results = executor.execute("FOR u IN users FILTER u.age > 26 RETURN u.name", None).unwrap();
//! assert_eq!(results, vec![json!("Alice")]);
//! ```

pub mod ast;
pub mod error;
pub mod executor;
pub mod lexer;
pub mod parser;

// Re-export main types for convenience
pub use ast::{
    AggregateExpr, BinaryOperator, BodyClause, CollectClause, Expression, FilterClause, ForClause,
    JoinClause, JoinType, LetClause, LimitClause, Query, ReturnClause, SortClause, UnaryOperator,
    UpdateClause, UpsertClause,
};
pub use error::{SdbqlError, SdbqlResult};
pub use executor::{DataSource, InMemoryDataSource, LocalExecutor};
pub use lexer::{Lexer, Token};
pub use parser::Parser;
