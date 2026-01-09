# Transaction Module

## Purpose
ACID transaction support with write-ahead logging (WAL), configurable isolation levels, and validation. Provides durability and atomicity guarantees for multi-operation sequences.

## Key Files

| File | Lines | Description |
|------|-------|-------------|
| `mod.rs` | 431 | Transaction types, states, operations |
| `manager.rs` | 343 | TransactionManager lifecycle handling |
| `wal.rs` | ~200 | Write-ahead log persistence |
| `lock_manager.rs` | ~150 | Lock coordination (for serializable) |

## Architecture

### Transaction Lifecycle
```
begin() → Active → [operations] → prepare() → Preparing → commit() → Committed
                                     ↓
                               rollback() → Aborted
```

### Transaction States
```rust
pub enum TransactionState {
    Active,     // Accepting operations
    Preparing,  // Being committed (two-phase)
    Committed,  // Successfully committed
    Aborted,    // Rolled back
}
```

### Isolation Levels
```rust
pub enum IsolationLevel {
    ReadUncommitted,  // Dirty reads possible
    ReadCommitted,    // Default - only committed data
    RepeatableRead,   // Consistent reads within tx
    Serializable,     // Full isolation
}
```

## Operations

```rust
pub enum Operation {
    Insert { database, collection, key, data },
    Update { database, collection, key, old_data, new_data },
    Delete { database, collection, key, old_data },
    PutBlobChunk { database, collection, key, chunk_index, data },
    DeleteBlob { database, collection, key },
}
```

## TransactionManager API

```rust
// Create manager with WAL path
let manager = TransactionManager::new(wal_path)?;

// Begin transaction
let tx_id = manager.begin(IsolationLevel::ReadCommitted)?;

// Get transaction for operations
let tx_arc = manager.get(tx_id)?;
{
    let mut tx = tx_arc.write().unwrap();
    tx.add_operation(Operation::Insert { ... });
}

// Commit (validates + writes WAL + commits)
manager.commit(tx_id)?;

// Or rollback
manager.rollback(tx_id)?;
```

## Write-Ahead Log (WAL)

Ensures durability:
1. **Begin**: Transaction start marker
2. **Operations**: Each operation logged before execution
3. **Commit**: Commit marker (makes operations permanent)
4. **Abort**: Rollback marker

Recovery replays WAL on startup to restore consistent state.

## Validation

Before commit, `validate()` checks for:
- Duplicate inserts for same key within transaction
- Updates after delete on same key
- Other consistency violations

```rust
manager.validate(tx_id)?;  // Called automatically by commit()
```

## Common Tasks

### Using Transactions in Handlers
```rust
// Begin
let tx_id = tx_manager.begin(IsolationLevel::ReadCommitted)?;

// Operations
let tx_arc = tx_manager.get(tx_id)?;
{
    let mut tx = tx_arc.write().unwrap();
    tx.add_operation(Operation::Insert { ... });
}

// Apply to storage (within transaction context)
storage.insert_with_tx(tx_id, ...)?;

// Commit
tx_manager.commit(tx_id)?;
```

### Cleanup Expired Transactions
```rust
// Called periodically (default timeout: 5 minutes)
let expired_count = manager.cleanup_expired();
```

### Creating a Checkpoint
```rust
manager.checkpoint()?;  // Writes checkpoint marker to WAL
```

## Transaction ID

```rust
pub struct TransactionId(u64);  // Nanosecond timestamp-based

impl TransactionId {
    pub fn new() -> Self;           // Current timestamp
    pub fn from_u64(id: u64) -> Self;
    pub fn as_u64(&self) -> u64;
}
```

Format: `tx:{nanoseconds}` (e.g., `tx:1234567890123456789`)

## Dependencies
- **Uses**: `chrono` for timestamps, file I/O for WAL
- **Used by**: `storage::StorageEngine`, `server::handlers`, `driver::handler`

## Gotchas
- Default timeout: 5 minutes (configurable via `set_timeout()`)
- WAL written synchronously for durability
- Transaction IDs are timestamp-based for ordering
- `prepare()` assigns write timestamp (for MVCC)
- Validation errors added to transaction, checked before commit
- Lock manager used only for Serializable isolation
- Transactions auto-cleaned by `cleanup_expired()`
- Double commit fails with `TransactionNotFound`
