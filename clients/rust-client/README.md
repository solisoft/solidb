# SoliDB Rust Client

Official Rust client library for [SoliDB](https://github.com/solisoft/solidb), a lightweight, high-performance multi-document database.

## Features

- Native binary protocol using MessagePack serialization
- Persistent TCP connections for low latency
- Full async support with Tokio
- Complete API coverage: documents, collections, queries, indexes, transactions
- Builder pattern for connection configuration

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
solidb-client = "0.5.0"
tokio = { version = "1", features = ["full"] }
```

## Quick Start

```rust
use solidb_client::{SoliDBClient, SoliDBClientBuilder};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), solidb_client::DriverError> {
    // Connect to server
    let mut client = SoliDBClient::connect("localhost:6745").await?;
    
    // Authenticate (optional - depends on server config)
    client.auth("mydb", "admin", "password").await?;
    
    // Ping server
    let version = client.ping().await?;
    println!("Server version: {}", version);
    
    // Create a database
    client.create_database("mydb").await?;
    
    // Create a collection
    client.create_collection("mydb", "users", None).await?;
    
    // Insert a document
    let doc = client.insert("mydb", "users", None, json!({
        "name": "Alice",
        "age": 30,
        "email": "alice@example.com"
    })).await?;
    println!("Inserted: {:?}", doc);
    
    // Query documents with SDBQL
    let users = client.query(
        "mydb",
        "FOR u IN users FILTER u.age > @min_age RETURN u",
        Some([("min_age".to_string(), json!(25))].into())
    ).await?;
    println!("Users older than 25: {:?}", users);
    
    Ok(())
}
```

### Using Bind Variables

Bind variables allow you to safely parameterize your queries and reuse query plans:

```rust
use std::collections::HashMap;

// Simple bind variables
let results = client.query(
    "mydb",
    "FOR u IN users FILTER u.age > @min_age RETURN u",
    Some([("min_age".to_string(), json!(25))].into())
).await?;

// Multiple bind variables
let mut bind_vars = HashMap::new();
bind_vars.insert("min_age".to_string(), json!(25));
bind_vars.insert("max_age".to_string(), json!(65));
let results = client.query(
    "mydb",
    "FOR u IN users FILTER u.age > @min_age AND u.age < @max_age RETURN u",
    Some(bind_vars)
).await?;
```

## Using the Builder

For more control over connection setup:

```rust
let client = SoliDBClientBuilder::new("localhost:6745")
    .auth("mydb", "admin", "password")
    .timeout_ms(5000)
    .build()
    .await?;
```

## API Reference

### Connection

- `SoliDBClient::connect(addr)` - Connect to a server
- `client.ping()` - Check server connectivity
- `client.auth(database, username, password)` - Authenticate

### Databases

- `client.list_databases()` - List all databases
- `client.create_database(name)` - Create a database
- `client.delete_database(name)` - Delete a database

### Collections

- `client.list_collections(database)` - List collections
- `client.create_collection(database, name, collection_type)` - Create a collection
- `client.delete_collection(database, name)` - Delete a collection
- `client.collection_stats(database, collection)` - Get collection statistics

### Documents

- `client.get(database, collection, key)` - Get a document by key
- `client.insert(database, collection, key, document)` - Insert a document
- `client.update(database, collection, key, document, merge)` - Update a document
- `client.delete(database, collection, key)` - Delete a document
- `client.list(database, collection, limit, offset)` - List documents with pagination

### Queries (SDBQL)

```rust
// Simple query
let results = client.query("mydb", "FOR u IN users RETURN u", None).await?;

// With bind variables
let mut bind_vars = std::collections::HashMap::new();
bind_vars.insert("min_age".to_string(), json!(25));
let results = client.query(
    "mydb",
    "FOR u IN users FILTER u.age > @min_age RETURN u",
    Some(bind_vars)
).await?;

// Explain query plan
let plan = client.explain("mydb", "FOR u IN users RETURN u", None).await?;
```

### Indexes

- `client.create_index(database, collection, name, fields, unique, sparse)` - Create an index
- `client.delete_index(database, collection, name)` - Delete an index
- `client.list_indexes(database, collection)` - List indexes

### Transactions

```rust
// Begin a transaction
let tx_id = client.begin_transaction("mydb", None).await?;

// All operations within the transaction use the same connection
let _ = client.insert("mydb", "users", None, json!({"name": "Bob"})).await?;

// Commit
client.commit().await?;

// Or rollback
// client.rollback().await?;
```

### Bulk Operations

```rust
// Batch multiple commands
let commands = vec![
    solidb_client::Command::Insert { database: "mydb".to_string(), collection: "users".to_string(), key: None, document: json!({"name": "Carla"}) },
    solidb_client::Command::Insert { database: "mydb".to_string(), collection: "users".to_string(), key: None, document: json!({"name": "David"}) },
];
let responses = client.batch(commands).await?;

// Bulk insert documents
let count = client.bulk_insert(
    "mydb",
    "users",
    vec![
        json!({"name": "Eve"}),
        json!({"name": "Frank"}),
        json!({"name": "Grace"}),
    ]
).await?;
println!("Inserted {} documents", count);
```

## Error Handling

```rust
use solidb_client::DriverError;

match client.get("mydb", "users", "nonexistent").await {
    Ok(doc) => println!("Found: {:?}", doc),
    Err(DriverError::DatabaseError(msg)) => println!("Database error: {}", msg),
    Err(DriverError::ConnectionError(msg)) => println!("Connection error: {}", msg),
    Err(e) => println!("Other error: {}", e),
}
```

## Feature Flags

Currently no feature flags. The client is designed to be lightweight with minimal dependencies.

## Version Compatibility

| Client Version | SoliDB Server Version |
|----------------|----------------------|
| 0.5.0          | 0.5.0+               |

## License

MIT License - see [LICENSE](LICENSE) file.

## Contributing

Contributions are welcome! Please see the main [SoliDB repository](https://github.com/solisoft/solidb) for guidelines.
