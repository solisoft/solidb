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

# Testing — use --profile ci, not --release (see below)
cargo test --profile ci --test <name>         # Specific test file (e.g., cargo test --profile ci --test http_api_test)
cargo test --profile ci <pattern>             # Tests matching pattern (e.g., cargo test --profile ci sdbql)
cargo test --profile ci -- --nocapture        # Show test output
cargo test --profile ci                       # FULL suite — CI's job, not yours. See below.

# The five benchmarks that live in tests/ are behind a feature and #[ignore]d
cargo test --profile ci --features bench-tests -- --ignored

# Code quality (required before commits)
cargo fmt -- --check           # Check formatting
cargo clippy -- -D warnings    # Lint checks
```

### Test with `--profile ci`, not `--release`

`[profile.ci]` (in `Cargo.toml`) is what CI runs and what you should run. It
inherits `release`, so dependencies — RocksDB above all — stay at `opt-level 3`
and the suite still executes in seconds. What it drops is thin-LTO and
release-level codegen for the workspace crates, which is what the ~96 separate
test binaries were each paying for.

That mattered more than it sounds: CI measured **95m 07s of compilation against
18.4s of test execution**. Do not "fix" this by relaxing `[profile.release]` —
the `build-binaries` and `docker` jobs ship what it produces.

Consequence to know about: `--profile ci` writes to `target/ci/`, *alongside*
`target/release/`. Building both doubles that part of your disk.

### Do not run the full suite locally

`cargo test --profile ci` with no filter builds and runs ~96 test binaries.
Compiling them takes tens of minutes even on a warm cache, and each integration
test opens its own RocksDB instance, so a dev box that is also running the
server and a fleet of app dev servers goes to swap. Runs get killed part-way and
tell you nothing.

Run the targeted forms above — the specific `--test <name>`, or a pattern — and
let the `test` job in CI run the whole thing. The other six CI jobs (`fmt`,
`clippy`, `docs-sync`, `audit`, `msrv`, `windows-check`) are all cheap enough to
reproduce locally and are the ones worth checking before a push.

### One failure that is not your change

It reproduces on a clean checkout; do not go hunting for it in a diff.

- **`queue::jobs::tests::validate_target_accepts_webhook_only`** fails only on a
  machine whose resolver wildcards `.test` to `127.0.0.1` (a common local dev
  setup). The SSRF guard resolves `example.test`, sees loopback, and rejects it.
  Passes in CI, where the name does not resolve.

The second entry that used to live here — **`rbac_admin_endpoints_tests`
aborting at process *exit*** with `SIGSEGV` or `std::bad_alloc`/`SIGABRT` after
all its tests reported ok — is **fixed**. `PendingCfDrops::spawn_dropper`
(`src/storage/pending_drops.rs`) detached its thread and nothing joined it, so a
column-family drop could still be inside RocksDB's `PersistRocksDBOptions` while
the main thread's static destructors freed the global option-type registry. The
handles are now kept and joined from `StorageEngine`'s `Drop`, by whichever
clone is the last one alive. Measured before and after on the same binaries,
60 runs each: `db_authorization_tests` 5/60 → 0/60, `rbac_admin_endpoints_tests`
→ 0/60. The server's shutdown path went through the same `Drop`, so it is fixed
there too.

## Releasing

When bumping the version, update all three of these in the same commit:

1. `version` in `Cargo.toml` (and `Cargo.lock` — `cargo update -p solidb --offline`).
2. The version pill in `doc/app/views/home/index.html.slv`
   (`<span class="ver-pill">vX.Y.Z</span>`).
3. **A section for the new version in `doc/app/views/docs/changelog.html.slv`**,
   and move anything under *Unreleased* into it.

Steps 2 and 3 are enforced by `scripts/check_docs_sync.sh`, which fails when
the docs site does not describe the version in `Cargo.toml`. It runs as the
`docs-sync` CI job (which gates `release`) and at the top of
`scripts/release.sh`, so a stale docs site stops the release before a tag or a
crates.io publish exists.

Step 3 is the one that used to get forgotten: the docs changelog is
hand-written and duplicates `CHANGELOG.md` (which release-please generates from
conventional commits). Between v0.31.0 and v0.32.2 the page was never touched,
so the docs site advertised a release three versions behind the shipped binary.
Copy the `CHANGELOG.md` entry across, or write the section directly if the
release was tagged by hand.

Tag releases as annotated tags on the release commit (`git tag -a vX.Y.Z`),
matching the existing `v0.32.1` style — CI's `release` job triggers on
`refs/tags/v*`.

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
- **queue/** - Internal scheduled work: trigger dispatch (script + signed webhook), embedding generation, and materialized-view refresh. SolidB exposes no client-facing job or cron queue; application background jobs live in the Soli framework.
- **driver/** - MessagePack-based binary protocol for high-performance clients.

### Entry Points

- `src/main.rs` - Server startup, CLI argument parsing, daemon mode
- `src/bin/solidb-dump.rs` - Database export utility (logical, per-database)
- `src/bin/solidb-restore.rs` - Database restore utility
- `src/bin/solidb-fuse.rs` - FUSE filesystem mount (optional feature)

### Backups

Two mechanisms, and they are not interchangeable:

- **Physical** — `POST /_api/backup` (admin) takes a RocksDB checkpoint of the
  *whole instance* via `StorageEngine::create_checkpoint`. Near-instant,
  hard-linked, point-in-time consistent across collections. Restore by pointing
  a server at the directory. All databases share one RocksDB instance (a column
  family per collection), so there is no per-database checkpoint.
- **Logical** — `solidb-dump` / `solidb-restore`. Per-database or per-collection,
  portable JSONL, but several times the on-disk size and much slower to restore.
  Use it to move data between versions or to edit a dump.

Because a checkpoint hard-links SSTs on the same filesystem, it is not
protection against losing that filesystem — copy it elsewhere.

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

- **`admin/`** — the database management / admin UI: browse collections and documents, run SDBQL, manage indexes, cluster, users, and more. Soli app with `.sl` controllers and `.html.slv` views.
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
