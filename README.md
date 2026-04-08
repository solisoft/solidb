# SoliDB

A lightweight, high-performance multi-document database with live query and blob support, written in Rust.

https://github.com/user-attachments/assets/aa64e937-39b8-42ca-8ee5-beb7dac90c23

[![CI](https://github.com/solisoft/solidb/actions/workflows/ci.yml/badge.svg)](https://github.com/solisoft/solidb/actions/workflows/ci.yml)

## 📖 Documentation

**Full documentation available at: [https://solidb.solisoft.net/docs/](https://solidb.solisoft.net/docs/)**

## ✨ Main Features

### Core Database
- 🚀 **Fast & Efficient** — Built with Rust for maximum performance
- 📄 **JSON Document Storage** — Store and query JSON documents with ease
- 📊 **Columnar Storage** — Column-oriented storage for analytics workloads with LZ4 compression and specialized indexes (Sorted, Hash, Bitmap, MinMax, Bloom)
- 🗃️ **Blob Storage** — Native support for storing and retrieving binary files
- 💾 **RocksDB Storage** — Production-grade persistence with automatic crash recovery

### Query Language
- 🔍 **SDBQL Query Language** — Familiar query syntax inspired by ArangoDB
- 📊 **Indexing** — Hash, persistent, geo, and fulltext indexes
- 🌍 **Geo Queries** — Spatial indexes and distance functions
- 📝 **Graph Traversals** — Native graph queries and shortest path algorithms
- 📋 **Common Table Expressions** — WITH clauses for composable, reusable subqueries
- 📦 **COLLECT AGGREGATE** — GROUP BY with inline aggregations (COUNT, SUM, AVG, MIN, MAX)
- ⚡ **Prepared Statement Cache** — Parse once, execute many with automatic LRU eviction

### Time Series & Streaming
- ⏱️ **Time Series** — TIME_BUCKET function for grouping temporal data into intervals (seconds, minutes, hours, days)
- 🌊 **Stream Processing** — CREATE STREAM with tumbling/sliding WINDOW clauses for continuous aggregations

### Vector Search
- 🧠 **Vector Indexes** — Create and query vector indexes for ML/AI workloads
- 🔗 **Hybrid Search** — Combined vector similarity + fulltext search in a single query
- 📐 **Vector Functions** — VECTOR_DISTANCE, VECTOR_NORMALIZE, quantization/dequantization

### Real-time & Scripting
- ⚡ **Live Queries** — Real-time subscriptions via WebSocket
- 🖥️ **Lua Scripting** — Server-side scripts for custom API endpoints
- ⏰ **Background Jobs** — Cron jobs and job queues with priorities and retries

### Distributed Architecture
- 🔄 **Multi-Node Replication** — Peer-to-peer replication with automatic sync
- 🧩 **Sharding** — Horizontal data partitioning with configurable shard count
- ⚖️ **Auto-Rebalancing** — Automatic data redistribution when nodes change
- ⚡ **Hybrid Logical Clocks** — Consistent ordering across distributed nodes
- 🔀 **Distributed Transactions** — Two-phase commit (2PC) for ACID guarantees across shards
- 📈 **Replication Lag Metrics** — Per-peer lag monitoring via Prometheus

### Observability & Performance
- 🔭 **OpenTelemetry Tracing** — W3C traceparent propagation for distributed tracing
- ⚡ **Regex Caching** — Compiled regex patterns cached globally (~8000x speedup on repeated patterns)

### Security & Administration
- 🔐 **JWT Authentication** — Secure API access with Bearer tokens
- 🔑 **API Keys** — Non-expiring keys for server-to-server communication
- 💳 **Transactions** — ACID transactions via X-Transaction-ID header
- 🖥️ **Web Dashboard** — Built-in admin UI for managing the database

## 📦 Official Clients

SoliDB provides native client libraries for multiple languages. See [**Client Documentation**](docs/CLIENTS.md) for installation and usage details.

- **Node.js** (`solidb-client`)
- **Python** (`solidb`)
- **Go** (`solidb-go-client`)
- **PHP** (`solidb/php-client`)
- **Ruby** (`solidb`)
- **Elixir** (`solidb`)

## 🚀 Quick Start

```bash
# Clone and build
git clone https://github.com/solisoft/solidb
cd solidb
cargo install --path .

# Start the server
solidb
```

The server starts on `http://localhost:6745` with a web dashboard.

> **Note**: A default admin user is created on startup with a randomly generated password displayed in the logs.

## 📋 Build Requirements

### Ubuntu/Debian
```bash
sudo apt-get install -y build-essential clang libclang-dev pkg-config libssl-dev libzstd-dev
```

### Arch Linux
```bash
sudo pacman -S base-devel clang gcc pkg-config openssl zstd
```

## 📚 Learn More

Visit the **[full documentation](https://solidb.solisoft.net/docs/)** for:
- Getting started guide
- API reference
- SDBQL query syntax
- Cluster setup
- Lua scripting
- And much more!

## 📄 License

[O'Saasy License](LICENCE.md)
