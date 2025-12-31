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
cargo test                     # All tests (592 tests across 54 test files)
cargo test --test <name>       # Specific test file (e.g., cargo test --test http_api_test)
cargo test <pattern>           # Tests matching pattern (e.g., cargo test sdbql)
cargo test -- --nocapture      # Show test output

# Code quality (required before commits)
cargo fmt -- --check           # Check formatting
cargo clippy -- -D warnings    # Lint checks
```

## Architecture

### Core Modules

- **sdbql/** - Custom query language (lexer, parser, AST, executor). The executor (`executor.rs` at 297KB) handles all query execution.
- **storage/** - RocksDB-backed persistence layer. `collection.rs` (125KB) manages document operations, indexing, and TTL.
- **server/** - Axum-based HTTP API and WebSocket handlers. `handlers.rs` (241KB) contains all endpoint logic.
- **cluster/** - Multi-node coordination with Hybrid Logical Clocks for distributed timestamp ordering.
- **sync/** - Replication worker and log management for eventual consistency across nodes.
- **sharding/** - Horizontal partitioning with automatic rebalancing. `coordinator.rs` (151KB) orchestrates shard operations.
- **transaction/** - ACID transactions with configurable isolation levels and WAL support.
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

## Web Application (www/)

The `www/` folder contains a **LuaOnBeans** application with Riot.js components and TailwindCSS.

### Structure

```
www/
├── app/
│   ├── components/      # Riot.js components (.riot files)
│   ├── controllers/     # Lua controllers (name_controller.lua)
│   ├── models/          # Data models
│   └── views/           # Etlua templates
│       ├── dashboard/   # Database management UI
│       ├── docs/        # Documentation website
│       └── talks/       # Slack-like chat application
├── config/
│   ├── database.json    # DB connection config
│   └── routes.lua       # URL routing
├── public/              # Built assets (CSS, JS)
└── beans.lua            # LuaOnBeans initialization
```

### Applications

- **Dashboard** (`/dashboard`) - Database management UI for browsing collections, running queries, managing indexes
- **Documentation** (`/docs`) - SoliDB documentation website
- **Talks** (`/talks`) - Slack-like team chat with channels, DMs, threads, reactions, file uploads, voice/video calls, and real-time updates via LiveQuery WebSocket

### Development Commands

```bash
cd www

# TailwindCSS
npm run build:css         # Build CSS
npm run watch:css         # Watch mode

# Riot components
npm run build:riot        # Compile .riot to JS

# LuaOnBeans scaffolding
lua beans.lua create controller <name>
lua beans.lua create model <name>
lua beans.lua db:migrate
lua beans.lua specs       # Run tests
```

### Key Patterns

- **Routing**: `config/routes.lua` maps URLs to `controller#action`
- **Views**: Etlua templates with `<%= %>` for output, `<% %>` for Lua code
- **Components**: Riot.js with `window.TalksMixin` for shared utilities
- **Real-time**: LiveQuery WebSocket subscriptions for instant updates
- **Auth**: Session cookies with Argon2 password hashing
