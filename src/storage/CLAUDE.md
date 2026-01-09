# Storage Module

## Purpose
Persistence layer built on RocksDB. Manages databases, collections, documents, indexes, and columnar storage.

## Key Files

| File | Lines | Description |
|------|-------|-------------|
| `collection.rs` | 3,846 | Collection operations, CRUD, indexing, TTL, fulltext |
| `columnar.rs` | 2,103 | Column-oriented storage for analytics workloads |
| `engine.rs` | 593 | StorageEngine - main entry point, RocksDB management |
| `index.rs` | 492 | Index types: hash, persistent, fulltext, TTL |
| `document.rs` | 294 | Document wrapper with metadata (_key, _rev) |
| `geo.rs` | 270 | Geo-spatial index using R-tree |
| `codec.rs` | 262 | Binary encoding/decoding for storage |
| `database.rs` | 224 | Database container for collections |
| `schema.rs` | 176 | JSON Schema validation for documents |

## Architecture

### Hierarchy
```
StorageEngine
  └── Database (e.g., "_system", "mydb")
        └── Collection (e.g., "users", "orders")
              ├── Documents (JSON with _key, _rev)
              ├── Indexes (hash, persistent, fulltext, geo, ttl)
              └── Schema (optional JSON Schema validation)
```

### RocksDB Column Families
Each collection uses a RocksDB column family:
- `{db}_{collection}` - Main document storage
- `{db}_{collection}_idx_{name}` - Index data
- `_meta` - System metadata

### StorageEngine (engine.rs)
```rust
pub struct StorageEngine {
    db: Arc<RwLock<DB>>,           // RocksDB instance
    collections: Arc<RwLock<HashMap<String, Collection>>>,
    databases: Arc<RwLock<HashMap<String, Database>>>,
    transaction_manager: RwLock<Option<Arc<TransactionManager>>>,
}
```

### Collection (collection.rs)
Main workhorse - handles all document operations:
- CRUD: `insert()`, `get()`, `update()`, `delete()`
- Batch: `insert_batch()`, `delete_batch()`
- Scan: `scan()`, `scan_prefix()`
- Indexes: `create_index()`, `create_geo_index()`, `create_ttl_index()`
- Fulltext: `fulltext_search()` with BM25 scoring
- TTL: Automatic document expiration

## Index Types

### Hash Index
Fast exact-match lookups on single or multiple fields.
```rust
collection.create_index("email_idx", vec!["email"], IndexType::Hash)?;
```

### Persistent Index
Sorted index for range queries.
```rust
collection.create_index("age_idx", vec!["age"], IndexType::Persistent)?;
```

### Fulltext Index
Text search with BM25 ranking, tokenization, stop words.
```rust
collection.create_index("content_idx", vec!["content"], IndexType::Fulltext)?;
collection.fulltext_search("content", "search terms")?;
```

### Geo Index
Spatial queries using R-tree.
```rust
collection.create_geo_index("location_idx", "location")?;
collection.geo_near("location", lat, lng, radius)?;
```

### TTL Index
Automatic document expiration.
```rust
collection.create_ttl_index("expires_idx", "expires_at", 0)?;
```

## Columnar Storage (columnar.rs)

Column-oriented storage for analytics:
```rust
let columnar = ColumnarCollection::create(&db, "metrics", vec![
    ColumnDef { name: "timestamp", column_type: ColumnType::DateTime, ... },
    ColumnDef { name: "value", column_type: ColumnType::Float64, ... },
])?;
```

Features:
- Column-wise compression (Snappy, LZ4, Zstd)
- Efficient aggregations (SUM, AVG, MIN, MAX, COUNT)
- Time-series optimized
- Bloom filter indexes

## Common Tasks

### Adding a New Index Type
1. Add variant to `IndexType` enum in `index.rs`
2. Implement index logic in `collection.rs`
3. Add create/drop handlers in `server/handlers.rs`

### Debugging Storage Issues
1. Check RocksDB logs in data directory
2. Use `compact_collection` to reclaim space
3. Use `repair_collection` for corruption

### Understanding collection.rs
- Lines 1-500: Struct definitions, CRUD operations
- Lines 500-1500: Index operations
- Lines 1500-2500: Fulltext search, BM25
- Lines 2500-3000: Geo operations
- Lines 3000+: TTL, schema validation, helpers

## Dependencies
- **Uses**: `rocksdb` crate, `serde_json` for documents
- **Used by**: `sdbql::Executor`, `server::handlers`, `scripting`

## Gotchas
- `collection.rs` is 3,846 lines - use search for specific operations
- Documents must have `_key` field (auto-generated if missing)
- `_rev` field is auto-managed for optimistic concurrency
- Column families are created lazily on first collection access
- Fulltext index uses English stop words by default
- Geo index expects `[longitude, latitude]` or `{lat, lng}` format
- TTL worker runs every 60 seconds by default (configurable)
