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
- 💾 **RocksDB Storage** — Production-grade persistence with automatic crash recovery; all collections share one keyspace, so creating or dropping a collection costs the same at ten collections or ten thousand

### Query Language
- 🔍 **SDBQL Query Language** — Familiar query syntax inspired by ArangoDB, plus as-of joins, lambdas, sketches, and time-travel scans
- 📊 **Indexing** — Hash, persistent, geo, fulltext, vector, bloom and cuckoo indexes
- 🌍 **Geo Queries** — Spatial indexes and distance functions
- 📝 **Graph Traversals** — Native graph queries and shortest path algorithms
- 📋 **Common Table Expressions** — WITH clauses for composable, reusable subqueries
- 📦 **COLLECT AGGREGATE** — GROUP BY with inline aggregations (COUNT, SUM, AVG, MIN, MAX)
- ⚡ **Prepared Statement and Result Caches** — Parse once, execute many; read results cached per caller and dropped on every write or schema change

### Time Series & Streaming
- ⏱️ **Time Series** — TIME_BUCKET, DELTA, RATE, FILL, RESAMPLE, and ASOF JOIN
- 🌊 **Stream Processing** — CREATE STREAM with tumbling/sliding WINDOW clauses for continuous aggregations

### Vector Search
- 🧠 **Vector Indexes** — Create and query vector indexes for ML/AI workloads
- 🔗 **Hybrid Search** — Combined vector similarity + fulltext search in a single query
- 📐 **Vector Functions** — VECTOR_SEARCH, VECTOR_DISTANCE, VECTOR_SIMILARITY, VECTOR_NORMALIZE; quantized indexes

### Real-time & Scripting
- ⚡ **Live Queries** — Real-time subscriptions via WebSocket
- 🖥️ **Lua Scripting** — Server-side scripts for custom API endpoints (`--no-lua` / `SOLIDB_NO_LUA=1` skips the VM pool)

### Distributed Architecture
- 🔄 **Multi-Node Replication** — Peer-to-peer replication with automatic sync
- 🧩 **Sharding** — Horizontal data partitioning with configurable shard count
- ⚖️ **Auto-Rebalancing** — Automatic data redistribution when nodes change
- ⚡ **Hybrid Logical Clocks** — Consistent ordering across distributed nodes
- 📈 **Replication Lag Metrics** — Per-peer lag monitoring via Prometheus

### Observability & Performance
- 📊 **Prometheus Metrics** — `/metrics` reports query latency, slow queries, result-cache hits, replication lag and memory
- 🔭 **OpenTelemetry Tracing** — W3C traceparent propagation for distributed tracing
- 🔌 **Binary Driver Protocol** — MessagePack over TCP for the official clients, alongside the HTTP API

### Security & Administration
- 🔐 **JWT Authentication** — Secure API access with Bearer tokens
- 🔑 **API Keys** — Non-expiring keys for server-to-server communication
- 💳 **Transactions** — ACID transactions on one node via the X-Transaction-ID header (no transaction spans shards or nodes)
- 🖥️ **Admin UI** — A separate web app in [`admin/`](admin/) to browse collections, run SDBQL, and manage indexes, users and the cluster

## 📦 Official Clients

SoliDB provides native client libraries for multiple languages. See the [**client documentation**](https://solidb.solisoft.net/docs/clients) for installation and usage details.

- **Rust** (`solidb-client`)
- **Node.js and browser** (`solidb-client`)
- **Python** (`solidb`)
- **Go** (`github.com/solisoft/solidb-go-client`)
- **PHP** (`solidb/php-client`), plus **Laravel Eloquent** (`solidb/laravel-eloquent`)
- **Ruby** (`solidb`)
- **Elixir** (`solidb`)
- **Mobile** — iOS (Swift), Android (Kotlin), React Native and Flutter, with offline-first sync ([`clients/mobile-sdk`](clients/mobile-sdk))

## 🚀 Quick Start

```bash
# Clone and build
git clone https://github.com/solisoft/solidb
cd solidb
cargo install --path .

# Start the server
solidb
```

The server listens on `http://localhost:6745`.

> **Note**: On first start an `admin` user is created. Its password is `SOLIDB_ADMIN_PASSWORD` when set; otherwise a random one is written to `<data-dir>/.admin_password` (owner-only on Unix), and the log gives that path.

## 💻 Platforms

Prebuilt binaries are published for each release:

| Platform | Artifact |
|---|---|
| Linux x86_64 | `solidb-linux-amd64.tar.gz` |
| Linux x86_64, static (musl) | `solidb-linux-amd64-musl.tar.gz` |
| Linux arm64 | `solidb-linux-arm64.tar.gz` |
| macOS arm64 | `solidb-darwin-arm64.tar.gz` |
| Windows x86_64 | `solidb-windows-amd64.zip` |

On Linux and macOS, `install.sh` fetches and installs the right one, and falls back to the static musl build when the system glibc is too old. On Windows, extract the
`.zip` and add the folder to your `PATH`, then run `solidb.exe --port 6745 --data-dir .\data`.

Two Windows caveats: `--daemon` is Unix-only and exits with an error — run SoliDB in a
console, or wrap it with a service manager such as [NSSM](https://nssm.cc) or `sc.exe`. And
the generated `.admin_password` file under the data directory is written with default ACLs
rather than the owner-only permissions used on Unix, so restrict the data directory yourself
on a shared machine.

## 📋 Build Requirements

### Ubuntu/Debian
```bash
sudo apt-get install -y build-essential clang libclang-dev pkg-config libzstd-dev
```

### Arch Linux
```bash
sudo pacman -S base-devel clang gcc pkg-config zstd
```

### Windows
Requires the MSVC build tools, CMake, and LLVM (RocksDB's bindgen needs `libclang`):
```powershell
winget install Microsoft.VisualStudio.2022.BuildTools Kitware.CMake LLVM.LLVM
```
Expect a long first build — RocksDB is a large C++ tree.

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
