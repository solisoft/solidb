# Driver Module

## Purpose
High-performance binary protocol for native database access. Uses MessagePack serialization for lower latency than HTTP REST API, with persistent TCP connections.

## Key Files

| File | Lines | Description |
|------|-------|-------------|
| `mod.rs` | 21 | Module exports, protocol overview |
| `protocol.rs` | 399 | Wire protocol, commands, responses |
| `handler.rs` | ~500 | Server-side command handling |
| `client.rs` | ~300 | Rust client implementation |

## Protocol Overview

### Connection Handshake
```
Client → Server: "solidb-drv-v1\0" (14 bytes magic header)
```

### Message Format
```
[length: 4 bytes BE][msgpack payload]
```

- Maximum message size: 16 MB
- Uses MessagePack for efficient binary serialization

## Commands

### Authentication
```rust
Command::Auth { database, username, password }
```

### Database Operations
```rust
Command::ListDatabases
Command::CreateDatabase { name }
Command::DeleteDatabase { name }
```

### Collection Operations
```rust
Command::ListCollections { database }
Command::CreateCollection { database, name, collection_type }
Command::DeleteCollection { database, name }
Command::CollectionStats { database, name }
```

### Document Operations
```rust
Command::Get { database, collection, key }
Command::Insert { database, collection, key, document }
Command::Update { database, collection, key, document, merge }
Command::Delete { database, collection, key }
Command::List { database, collection, limit, offset }
```

### Query Operations
```rust
Command::Query { database, sdbql, bind_vars }
Command::Explain { database, sdbql, bind_vars }
```

### Index Operations
```rust
Command::CreateIndex { database, collection, name, fields, unique, sparse }
Command::DeleteIndex { database, collection, name }
Command::ListIndexes { database, collection }
```

### Transaction Operations
```rust
Command::BeginTransaction { database, isolation_level }
Command::CommitTransaction { tx_id }
Command::RollbackTransaction { tx_id }
Command::TransactionCommand { tx_id, command }
```

### Bulk Operations
```rust
Command::Batch { commands }       // Multiple commands
Command::BulkInsert { database, collection, documents }
```

## Responses

```rust
Response::Ok { data, count, tx_id }  // Success
Response::Error { error }            // Failure
Response::Pong { timestamp }         // Keep-alive
Response::Batch { responses }        // Batch results
```

## Error Types

```rust
DriverError::ConnectionError(String)
DriverError::ProtocolError(String)
DriverError::DatabaseError(String)
DriverError::AuthError(String)
DriverError::TransactionError(String)
DriverError::MessageTooLarge
DriverError::InvalidCommand(String)
```

## Usage (Rust Client)

```rust
use solidb::driver::SoliDBClient;

let mut client = SoliDBClient::connect("127.0.0.1:6746").await?;
client.auth("_system", "admin", "password").await?;

// Query
let result = client.query("_system", "FOR doc IN users RETURN doc", HashMap::new()).await?;

// Insert
let doc = json!({"name": "Alice", "age": 30});
client.insert("_system", "users", None, doc).await?;
```

## Server Port

Default driver port: **6746** (separate from HTTP port 6745)

## Common Tasks

### Adding a New Command
1. Add variant to `Command` enum in `protocol.rs`
2. Add handling in `handler.rs` `handle_command()`
3. Add client method in `client.rs`
4. Update client SDKs in `/clients/`

### Debugging Protocol Issues
1. Check magic header sent correctly
2. Verify message length prefix (4 bytes BE)
3. Use `decode_message` to inspect payloads
4. Check for `MessageTooLarge` errors

## Dependencies
- **Uses**: `rmp_serde` for MessagePack, `storage::StorageEngine`, `transaction::TransactionManager`
- **Used by**: Client SDKs (Rust, Go, Python, etc.)

## Gotchas
- Magic header sent once at connection start, not per message
- Commands use tagged enums (`#[serde(tag = "cmd")]`)
- Responses use named MessagePack serialization for client compatibility
- Transaction commands wrap other commands in `TransactionCommand`
- Batch commands return `Response::Batch` with ordered results
- `Ping` command returns `Pong` with server timestamp (keep-alive)
