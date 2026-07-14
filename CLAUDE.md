# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

SoliDB is a lightweight, high-performance multi-document database written in Rust. It features a custom query language (SDBQL), multi-node replication, sharding, ACID transactions, Lua scripting, and WebSocket-based real-time subscriptions.

## Build & Development Commands

```bash
# Build
cargo build                    # Debug build
cargo build --release          # Release build

# Run server
./target/release/solidb --port 6745 --data-dir ./data

# Testing
cargo test --release                          # All tests (592 tests across 54 test files)
cargo test --release --test <name>            # Specific test file (e.g., cargo test --release --test http_api_test)
cargo test --release <pattern>                # Tests matching pattern (e.g., cargo test --release sdbql)
cargo test --release -- --nocapture           # Show test output

# Code quality (required before commits)
cargo fmt -- --check           # Check formatting
cargo clippy -- -D warnings    # Lint checks
```

## Releasing

When bumping the version, keep the docs site in sync — **always** update the
version pill in `doc/app/views/home/index.html.slv` (`<span class="ver-pill">vX.Y.Z</span>`)
to match `version` in `Cargo.toml`. Both must be updated in the same release.

## Architecture

### Core Modules

- **sdbql/** - Custom query language (lexer, parser, AST, executor). The executor (`executor.rs` at 297KB) handles all query execution.
- **storage/** - RocksDB-backed persistence layer. `collection.rs` (125KB) manages document operations, indexing, and TTL.
- **server/** - Axum-based HTTP API and WebSocket handlers. `handlers.rs` (241KB) contains all endpoint logic.
- **cluster/** - Multi-node coordination with Hybrid Logical Clocks for distributed timestamp ordering.
- **sync/** - Replication worker and log management for eventual consistency across nodes.
- **sharding/** - Horizontal partitioning with automatic rebalancing. `coordinator.rs` (151KB) orchestrates shard operations.
- **transaction/** - ACID transactions with configurable isolation levels, WAL support, and row-level locking.
- **scripting/** - Embedded Lua 5.4 runtime for custom endpoints and database operations.
- **queue/** - Background job processing with priorities and cron scheduling.
- **driver/** - MessagePack-based binary protocol for high-performance clients.

### Entry Points

- `src/main.rs` - Server startup, CLI argument parsing, daemon mode
- `src/bin/solidb-dump.rs` - Database export utility
- `src/bin/solidb-restore.rs` - Database restore utility
- `src/bin/solidb-fuse.rs` - FUSE filesystem mount (optional feature)

### Key Patterns

- **Error Handling**: Unified `DbError` enum with `DbResult<T>` type alias throughout
- **Async**: Tokio runtime with async handlers; `spawn_blocking` for CPU-intensive operations
- **Serialization**: Serde for JSON/MessagePack, Bincode for internal storage

## Query Language (SDBQL)

ArangoDB-inspired syntax supporting:
- FOR/FILTER/SORT/LIMIT/RETURN clauses
- JOIN/LEFT JOIN operations
- UPSERT/INSERT/UPDATE/REMOVE operations
- 60+ built-in functions
- Graph traversal and aggregations
- Indexes: hash, persistent, geo, fulltext

Example:
```sdbql
FOR doc IN users
  FILTER doc.age > 25
  SORT doc.age DESC
  LIMIT 10
  RETURN {name: doc.name, age: doc.age}
```

## Distributed Features

- **Replication**: Master-master with eventual consistency; writes queue for offline nodes
- **Sharding**: `ShardID = hash(key) % NumShards`; configurable replication factor
- **Cluster Scripts**: `/scripts/` contains cluster testing utilities (`start_cluster.sh`, `test_cluster_full.sh`)

## System Dependencies

Ubuntu/Debian:
```bash
apt-get install build-essential clang libclang-dev pkg-config libssl-dev libzstd-dev
```

macOS: Xcode Command Line Tools (macFUSE for FUSE support)

## Client SDKs

8 client libraries in `/clients/`: Rust, Go, Python, NodeJS, JavaScript, PHP, Ruby, Elixir

Benchmark all clients: `./bench_all.sh`

## Web Applications

Two **Soli** framework apps live alongside the database engine, each with its own `CLAUDE.md`:

- **`admin/`** — the database management / admin UI: browse collections and documents, run SDBQL, manage indexes, cluster, users, queues, and more. Soli app with `.sl` controllers and `.html.slv` views.
- **`doc/`** — the SoliDB documentation website and landing page. Also a Soli app.

### Development Commands

```bash
cd admin        # or cd doc

soli serve . --dev     # dev server, hot reload
soli test              # run specs
soli lint              # static analysis
```

See `admin/CLAUDE.md` and `doc/CLAUDE.md` for the Soli language and framework conventions.

> The former LuaOnBeans `www/` app (old dashboard + docs website) has been removed; `admin/` and `doc/` supersede it.
