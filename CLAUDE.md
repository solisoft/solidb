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

### On a workstation with `rbuild`, compile remotely

If `rbuild` is on `PATH`, prefer it for anything that compiles. It rsyncs the
working tree — uncommitted changes included — to the dedicated build server,
runs cargo there on twelve cores, and pulls only the executables back.
Everywhere else (CI runners, a fresh clone, another machine) `rbuild` is absent
and the plain cargo commands above are the right ones.

```bash
rbuild db                                   # replaces cargo build --release
rbuild db --features fuse                   # solidb-fuse; fuse3-devel is installed there
rbuild check db                             # replaces the fmt + clippy gate
rbuild test db --profile ci --test <name>   # replaces cargo test
```

This is the direct answer to the two disk problems described below. The ~96 test
binaries and the second `target/ci/` tree are built on a machine with 486 GB
free rather than on the box that is also running the server and a fleet of app
dev servers — so the swap death spiral stops being a reason not to run tests.
The local `target/` was deleted when compilation moved off this machine; only
`target/remote/` survives, so a local `cargo build` now starts from nothing,
RocksDB's vendored C++ included.

Binaries land in `target/remote/release/` (`solidb`, `solidb-dump`,
`solidb-restore`, `solidb-repl`) and are linked into `Work/soli/bin/`, first on
`PATH`. Two caveats:

- **The remote release binary is not the manifest's.** The server overrides
  `lto` to `thin` and `codegen-units` to 16, and a `[profile.*]` table in a
  cargo config outranks `Cargo.toml` key by key. Benchmark with
  `rbuild --faithful db`.
- **`cargo clean` here also deletes `target/remote/`.** Prefer
  `rbuild clean db -p solidb`, which is package-scoped and so spares the
  vendored C/C++ in `target/release/build/` — measured after a run:
  `tikv-jemalloc-sys` 286 MB, `rust-librocksdb-sys` 95 MB across two units,
  plus `aws-lc-sys` and `zstd-sys`. A bare `cargo clean` throws all of that
  away and the next build pays for it in g++. `rbuild pull db` re-fetches the
  binaries without recompiling.

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

### Known local-only failures

There are none right now.

- **`queue::jobs::tests::validate_target_accepts_webhook_only`** used to fail on
  any machine whose resolver wildcards `.test` to `127.0.0.1` (a common local
  dev setup): the SSRF guard resolved `example.test`, saw loopback, and
  rejected it. **Fixed** — the fixture is a literal public IP, so the test does
  no DNS at all. This became load-bearing when the guard stopped treating an
  unresolvable host as allowed: `example.test` would now fail in CI too, where
  the name does not resolve.

The entry that used to live here — **`rbac_admin_endpoints_tests`
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

- **sdbql/** - Custom query language (lexer, parser, AST, executor). Query
  execution lives in the `executor/` directory, not a single file — start at
  `executor/execution/clauses.rs` for the FOR/FILTER/SORT/LIMIT pipeline.
- **storage/** - RocksDB-backed persistence layer. `collection/` holds document
  operations, indexing and TTL, split across `crud.rs`, `indexes.rs`,
  `fulltext.rs`, `geo.rs`, `ttl.rs`, `blobs.rs`, `vector.rs` and `versioning.rs`.
  One collection is one RocksDB column family; `collection/mod.rs` defines the
  key prefixes (`doc:`, `idx:`, `ft:`, `blo:` …) that namespace data within it.
- **server/** - Axum-based HTTP API and WebSocket handlers. Endpoint logic is in
  the `handlers/` directory plus ~28 sibling files in `src/server/`; routes are
  declared in one place, `routes.rs`.
- **cluster/** - Multi-node coordination with Hybrid Logical Clocks for distributed timestamp ordering.
- **sync/** - Replication worker and log management for eventual consistency across nodes.
- **sharding/** - Horizontal partitioning with automatic rebalancing.
  `coordinator.rs` orchestrates shard operations — at 3848 lines it is the
  largest single source file in the repository.
- **transaction/** - ACID transactions with configurable isolation levels, WAL support, and row-level locking.
- **scripting/** - Embedded Lua 5.4 runtime for custom endpoints and database operations.
- **queue/** - Internal scheduled work: trigger dispatch (script + signed webhook), embedding generation, and materialized-view refresh. SolidB exposes no client-facing job or cron queue; application background jobs live in the Soli framework.
- **driver/** - MessagePack-based binary protocol for high-performance clients.

### Entry Points

- `src/main.rs` - Server startup, CLI argument parsing, daemon mode
- `src/bin/solidb-dump.rs` - Database export utility (logical, per-database)
- `src/bin/solidb-restore.rs` - Database restore utility
- `src/bin/solidb-fuse.rs` - FUSE filesystem mount (optional feature)
- `src/bin/solidb-repl.rs` - Interactive SDBQL shell

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

## Security-relevant configuration

These all fail closed. Each exists because the permissive behaviour was a
finding in the September 2026 audit; flipping one back is an explicit
decision, not a default.

| Variable | Default | Effect when set |
|---|---|---|
| `ADMIN_UI_PASSWORD` | unset | Requires a login on the `admin/` console. Without it (and without the override below) the console refuses to serve. |
| `ADMIN_UI_ALLOW_NO_AUTH` | off | Declares that something in front of `admin/` authenticates requests. The console holds a SoliDB admin credential and acts under it for every visitor, so one of these two must be set. |
| `SOLIDB_ENABLE_AI_VALIDATION` | off | Enables `/_api/ai/validate`, which shells out to `cargo` on the server host. Dev hosts only; also requires a global admin. |
| `SOLIDB_ALLOW_GLOBAL_WEBHOOK_SECRET` | off | Lets a job with no `webhook_secret` be signed with `SOLI_WEBHOOK_SECRET`. Off by default because the target URL is tenant-chosen, which makes it a signing oracle for the instance secret. Prefer a per-trigger secret. |
| `SOLIDB_ALLOW_PRIVATE_LLM_URL` | off | Lets a tenant-supplied `OLLAMA_URL` point at a loopback, link-local or RFC1918 address. Off by default: the URL lives in the database's own `_env`, so any principal with Write on that database chooses where the server dials — that is read-SSRF from the server's network position. Turn it on only when the instance is genuinely meant to reach a private LLM endpoint. |
| `SOLIDB_MAX_INTERMEDIATE_ROWS` | 5,000,000 | Ceiling on rows a single query may materialise, checked cooperatively inside the executor. Bounds nested-`FOR` cartesian products, which no `LIMIT` applies to. |

Two existing switches worth knowing in the same breath: `SOLIDB_ALLOW_WEBHOOK_LOOPBACK`
and `SOLIDB_ALLOW_INSECURE_WEBHOOK_TLS` are dev-only escape hatches on the
webhook path, and `SOLIDB_DB_AUTHZ_MODE=warn` (plus `SOLIDB_DB_AUTHZ_ALLOW_WARN=1`)
turns per-database authorization into a dry run.

### Resource bounds added by the September 2026 audit fixes

Each bounds something a single client could grow without limit. Invalid or `0`
falls back to the default.

| Variable | Default | Bounds |
|---|---|---|
| `SOLIDB_QUEUE_MAX_CONCURRENCY` | 4 × cores | Jobs executing at once |
| `SOLIDB_JOBS_RETENTION_SECS` | 604800 | Age at which `completed`/`failed` `_jobs` rows are deleted (Soli rows, which use `state`, are never touched) |
| `SOLIDB_JOBS_LEASE_SECS` | max(600, 2 × Lua timeout) | How long a `running` job may go unfinished before it is requeued |
| `SOLIDB_STREAM_MAX_BUFFER_EVENTS` / `_BYTES` | 100000 / 64 MiB | Per-stream window buffer |
| `SOLIDB_LUA_FETCH_MAX_BYTES` | 10 MB | Lua `fetch` response body |
| `SOLIDB_LUA_RATE_LIMIT_MAX_KEYS` | 100000 | Keys held by `solidb.rate_limit` |
| `SOLIDB_REPL_MAX_SESSIONS_PER_USER` / `SOLIDB_REPL_MAX_SESSIONS` | 8 / 1000 | REPL sessions |
| `SOLIDB_CLUSTER_HTTP_TIMEOUT_SECS` / `_READ_TIMEOUT_SECS` | 60 / 60 | Inter-node HTTP requests |
| `SOLIDB_CLUSTER_STREAM_TIMEOUT_SECS` | 21600 | Shard copy / export streams |

`SOLIDB_CLUSTER_SCHEME` now applies to every inter-node URL (`src/cluster/http.rs`
`peer_url`); anything but `http` means https. Build new inter-node URLs with
that helper, never `format!("http://…")`.

### Column-family lifecycle knobs

One collection is one RocksDB column family, and every `create_cf`/`drop_cf`
rewrites *and fsyncs* the whole OPTIONS file — one section per CF, so the cost
is proportional to the instance's **total** collection count, not to the
collection being touched. There is no RocksDB setting that avoids this; the
write is unconditional and ends with a re-parse of the file it just wrote.

| Variable | Default | Effect |
|---|---|---|
| `SOLIDB_CF_REUSE_GRACE_SECS` | 300 | How long a deleted collection's column family is kept so a same-name recreate can wipe and reuse it, saving two OPTIONS rewrites. Its data is erased at deletion time, so the shell holds nothing while it waits. A single reaper thread drops the ones nobody reclaims. |
| `SOLIDB_AUTO_CREATE_COLLECTIONS` | on | Writing a document to an unknown collection creates it. Set to `0` to get `CollectionNotFound` instead — worth it on an instance whose schema is managed elsewhere, since every auto-creation grows the OPTIONS file. |
| `SOLIDB_MAX_TOTAL_WAL_SIZE` | 2GB prod / 256MB `--dev` | Total WAL budget. This is a **flush trigger**, not a disk cap: crossing it flushes every CF holding data in the oldest WAL. Lowering it to save disk is a false economy — it produces thousands of sub-4KB SSTs. Bound memory with `--memtable-budget`, which flushes one CF at a time. |

Watch `solidb_cf_ops_total`, `solidb_cf_op_seconds_total`,
`solidb_cf_reuses_total` and `solidb_collections_autocreated_total` on
`/metrics`; CF churn shows up as latency that no single query accounts for.

### Three tiers of protected collections

`src/storage/protected.rs` holds the lists, and the boundary is *a
caller-supplied collection name* — server-side code uses
`Database::system_collection` and is unaffected.

- **Not readable or writable by name**: `_env`, `_admins`, `_api_keys`,
  `_roles`, `_user_roles`. Credentials, plus the authorization state the
  server reads back to decide permissions.
- **Readable, never written by name**: `_scripts`, `_services`, `_triggers`,
  `_views`, `_graphs`, `_config`, `_rag_pipelines`. Everything the server
  later executes or schedules. Writes go through the dedicated Admin-gated
  endpoints, which validate their input.
- **Readable, written by name with Admin only**: `_jobs`. The trigger
  dispatcher executes its `pending` rows as `_system`, so plain Write must
  not reach it — but it is also the Soli framework's job store, written by
  name from `perform_later` and the worker with an admin credential.

Every by-name write goes through `check_write_access(name, WriteActor)`; the
storage getters `get_collection_for_write` take the actor so a handler cannot
forget to say who is writing. `WriteActor::Server` is for the server's own
work, never for a name that came in over the wire. The check covers the
document API, SDBQL, the driver, import, truncate, blob uploads, and the Lua
bindings: a script writes as its **caller** (`ScriptContext::write_actor`,
kept in mlua app data as `LuaCaller`), never as the server, so a public
"insert into `request.body.collection`" script cannot be steered at
`_scripts`. The queue worker runs job and trigger scripts as `_system` with
the admin role, which is what lets them write `_jobs`.

Adding a collection that the server interprets after storing it means adding
it to one of these lists.

## System Dependencies

Ubuntu/Debian:
```bash
apt-get install build-essential clang libclang-dev pkg-config libssl-dev libzstd-dev
```

macOS: Xcode Command Line Tools (macFUSE for FUSE support)

## Client SDKs

`clients/` holds nine libraries and four example applications; the count of
"8 SDKs" that used to sit here counted `js-client` twice (as "NodeJS" and as
"JavaScript") and missed two.

- **Libraries**: `rust-client`, `go-client`, `PYTHON-client`, `js-client`
  (Node and browser both), `PHP-client`, `Ruby-client`, `elixir_client`,
  `solidb-laravel-eloquent`, `mobile-sdk`.
- **Example apps, not SDKs**: `android-example`, `ios-example`,
  `flutter-example`, `react-native-example`. The docs site has a page for each,
  so they are shipped surfaces even though nothing depends on them.

Benchmark all clients: `./bench_all.sh`

`clients/Ruby-client/.bundle/config` sets `BUNDLE_PATH: vendor/bundle`, so
`bundle install` writes into the working tree. That directory is ignored — do
not commit it.

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

### Keep the Claude skills in step

`~/.claude/skills/solidb/` (SKILL.md + `references/sdbql.md`, `api.md`,
`from-soli.md`) teaches agents to write SDBQL and call this server, and
`~/.claude/skills/soli-lang/` covers the Soli side. Every claim in them was
observed on a running server, with the version noted. Any change to SDBQL
syntax or semantics, a builtin function, index or planner behavior, the query
cache, cursors, transactions, an HTTP endpoint, auth, or a CLI tool MUST be
checked against the solidb skill in the same change: grep the skill for the
feature, then fix, add or delete the entry. **A fix that removes a trap the
skill warns about (e.g. UPSERT on non-key fields, FILTER swallowing errors,
index SORT dropping rows) means deleting that warning** and bumping the version
the skill says it was observed on. Verify edited examples with the skill's curl
helper against a throwaway database. The skills live outside the repo, so say in
the commit message which entries were updated, or that none needed to be.

### Which documentation is authoritative

`doc/` (the Soli app) is the live documentation site and is the one to update.
`docs/` is a handful of older engine markdown files kept only where they have no
page on the site — `SDBQL_REFERENCE.md` and `BACKUP.md` are current; treat
anything else there as historical. When the two disagree, `doc/` wins.
