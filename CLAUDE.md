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

# The six benchmarks that live in tests/ are behind a feature and #[ignore]d
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
let the `test` job in CI run the whole thing (`test-solistore` runs it again
on the solistore engine). The other six CI jobs (`fmt`,
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
2. The version pill, written once in
   `doc/app/views/shared/_version_pill.html.slv` (`<span class="ver-pill">vX.Y.Z</span>`);
   the landing page and the docs layout render it. `check_docs_sync.sh` fails if it
   is stale or if a literal pill is added to another view.
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
  Since 2.0 every collection lives in one shared column family, `__keyspaces__`,
  under an 8-byte key prefix (its *keyspace*, `storage/keyspace.rs`);
  `collection/mod.rs` defines the logical key prefixes (`doc:`, `idx:`, `ft:`,
  `blo:` …) that namespace data within it. See `docs/storage-format.md`.
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
- `tray/` - `solidb-tray.exe` (the Windows notification-area icon) and
  `solidb-service.exe` (the Windows service): a crate of its own, outside
  the workspace, that depends on `windows-sys` alone and never links the
  server. Both start `solidb.exe` without a window (`server::spawn`) and
  stop it by sending Ctrl-C to its console (`server::ask_to_stop`), which
  is what `shutdown_signal` in `main.rs` waits for on Windows: a change to
  how the server takes a shutdown has to keep that. The icon drives the
  service when one is installed (`scm.rs`), else a server of its own.
  `config.rs`, `state.rs` and `probe.rs` are tested on any system
  (`cargo test --manifest-path tray/Cargo.toml`); the rest is only
  compiled for Windows (`--target x86_64-pc-windows-gnu` from Linux, with
  mingw-w64).

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

### The sync log is written only when something reads it (2.0.3)

Every document write appends an entry to the replication log
(`<data-dir>/sync_log`, a second RocksDB): a second WAL write under a lock all
writers share, ~40% of a primary-key update. A node started without `--peer`,
and with no persisted replication history (`SyncState::has_replicated_with_peers`),
starts it *deferred* (`SyncLog::defer_until_needed`) and activates it on the
first authenticated peer sync connection or offline-sync session/pull —
before serving anything from it. Activation is persisted (`_meta:active` in
the log DB), so a node that has ever had a reader keeps logging across
restarts. Any new reader of the log must call `SyncLog::activate` first.
`--no-sync-log` still disables it outright.

`SOLIDB_CLUSTER_SCHEME` now applies to every inter-node URL (`src/cluster/http.rs`
`peer_url`); anything but `http` means https. Build new inter-node URLs with
that helper, never `format!("http://…")`.

### Keyspaces: how collections are stored (2.0)

Up to 1.3, one collection was one RocksDB column family, and every
`create_cf`/`drop_cf` rewrote *and fsynced* the whole OPTIONS file — a cost
proportional to the instance's **total** collection count (≈0.2 s per create at
1,232 collections). Since 2.0 all collections share the `__keyspaces__` column
family, each under a fixed 8-byte prefix: database id then collection id, both
big-endian `u32`, allocated in `_meta` and never reused. Creating a collection
is one `_meta` write; dropping one is a registry delete plus one `DeleteRange`;
dropping a database is one `DeleteRange` over its id range. None of them touch
the column-family map. Full layout: `docs/storage-format.md`.

Rules for code that touches storage:

- Collection code reaches RocksDB only through its `Keyspace`
  (`coll.ks.handle(&db)` → `KsCf`) and the `*_ks` methods of `KsDbExt` /
  `KsBatchExt`. Key builders stay *logical*; the prefix is added and stripped
  there, and every iterator is bounded to the keyspace. A raw `*_cf` call does
  not accept a `KsCf`, so a missed site fails to compile — keep it that way.
- Existence is the `_meta` registry (`collection_registry::keyspace_of`), not
  `cf_handle(name)`. A dropped keyspace's handles all report
  `CollectionNotFound` (shared dead flag), never the next incarnation's data.
- 1.x collections are migrated automatically at the first 2.x start
  (`storage/keyspace_migration.rs`, resumable; `--migrate-only` runs it and
  exits). One that fails stays *legacy* — its own column family, empty prefix —
  and is served as before until the next start retries it.

| Variable | Default | Effect |
|---|---|---|
| `SOLIDB_AUTO_CREATE_COLLECTIONS` | on | Writing a document to an unknown collection creates it. Set to `0` to get `CollectionNotFound` instead, for an instance whose schema is managed elsewhere. |
| `SOLIDB_KS_GC_DELAY_SECS` | 10 | Delay before a dropped keyspace's range is compacted (its space reclaimed). |
| `SOLIDB_LEGACY_COLLECTION_CFS` | off | Create new collections as 1.x column families and skip the migration. For tests of the legacy path only. |
| `SOLIDB_MIGRATION_SKIP_SPACE_CHECK` | off | Start the migration even when free disk is below 1.2 × the largest collection + 1 GiB. |
| `SOLIDB_CF_REUSE_GRACE_SECS` | 300 | Legacy collections only: grace before an emptied 1.x column family is dropped. |
| `SOLIDB_BLOCK_CACHE` | 40% of (memory − 1 GiB), ≥ 64 MiB, prod / 128MB `--dev` | Block cache of either engine (`--block-cache`). "Memory" is the tightest cgroup `memory.max` on the process's cgroup path, else the host's total (`storage/memory_limit.rs`); 512MB when neither can be read. The startup log names the basis; `/metrics` has `solidb_block_cache_capacity_bytes`. |
| `SOLIDB_TX_LOCK_WAIT_MS` | 500 | How long a transaction waits for a document lock another transaction holds before it is refused with a conflict (`transaction/lock_manager.rs`). `0`: refused at once, the behaviour up to 2.9.3. A deadlock (two transactions waiting for each other) is refused at once whatever the value. Once a waiter has waited a quarter of it, transactions that arrive queue behind it instead of taking a free lock (`OVERTAKING_ENDS`): before that they may overtake, so that a release is not always handed to a sleeping thread. The wait runs under `block_in_place`, so only on the multi-threaded runtime: a `#[tokio::test]` without `flavor = "multi_thread"` never waits. Transactions at `repeatable_read` / `serializable` do not wait for a write lock. |
| `SOLIDB_TX_GROUP_COMMIT` | on | Under load (when a driver commit would leave its async worker) and on an engine that merges batches (solistore), a driver commit is prepared on its connection and written by the commit thread with every other commit waiting, as one batch and one fsync (`storage/group_commit.rs`). `0`: each commit writes its own batch from its own thread, as up to 2.9.3. HTTP commits and RocksDB are not concerned. |
| `SOLIDB_IO_POLL_INTERVAL` | 4 | Tasks a busy async worker runs between two looks at the sockets (tokio's `event_interval`, whose own default is 61). Set in `main.rs`; applies to every transport. Lower reads arriving requests sooner under load (grouped commits need it: 56,000 transfers/s at 61, 75,600 at 4) and lengthens the p99 of reads at 32 clients. |
| `SOLIDB_WORKER_THREADS` | unset: one per CPU (tokio's default) | Async worker threads (`main.rs`). With as many workers as CPUs and fewer busy connections, workers park between requests and each request pays a wake-up. Measured on 8 hardware threads, 8 against 6 workers: at 8 clients, single inserts 219,000 → 275,000/s; at 32 clients, point reads 822,000 → 717,000/s and index reads 17% down. A setting for few, busy connections, not a default. |
| `SOLIDB_SCAN_THREADS` | the CPUs the process may use (cgroup `cpu.max`, else `available_parallelism`) | Threads a full-scan `COLLECT` may split into, and a process-wide cap on scan threads (`execution/scan_threads.rs`). `1` turns parallel scans off. Only engines with snapshots run them, which both have (solistore since 0.3); a collection under 65,536 documents stays on one thread. |
| `SOLIDB_MAX_TOTAL_WAL_SIZE` | 2GB prod / 256MB `--dev` | Total WAL budget. This is a **flush trigger**, not a disk cap. Lowering it to save disk is a false economy — it produces thousands of tiny SSTs. Bound memory with `--memtable-budget`. |

A driver connection answered within 40 µs waits that long for its next
request without letting its worker park (`driver/handlers/mod.rs`,
`stay_awake_for_request`), while the connections active lately are no more
than the workers (`transaction::connections_are_few`, the same count as
`off_the_worker`). It yields, which looks at the sockets; it must never
spin without yielding, and never when connections outnumber workers:
there a worker has requests waiting and the yield only delays them.

Per-collection `disk_usage` of a shared collection is RocksDB's approximate
size of its key range (flushed data only; memtable and SST count read 0).
`/metrics` carries `solidb_keyspace_creates_total`,
`solidb_keyspace_drops_total` and `solidb_keyspace_gc_compactions_total`;
`solidb_cf_ops_total` now counts only legacy column-family drops.

### Storage engines: `storage::kv` (RocksDB, and solistore behind a feature)

All storage goes through `storage::kv::Kv` (aliased `RocksDb` for
history): RocksDB's method names, SoliDB's own types (`KvCf`, `KvBatch`,
`KvIter`, `KvError`). `rust_rocksdb` is named only under `storage/kv/` and
in tests that build a 1.x directory by hand — keep it that way. A second
engine, solistore (github.com/solisoft/solistore, pure Rust), is compiled in
with `--features engine-solistore`; the plan is
`solistore/docs/solidb-integration.md` in that repository.

A data directory is opened with the engine recorded in its `ENGINE` file. An
unmarked directory with RocksDB `OPTIONS-*` files is RocksDB's; anything
else ambiguous is refused. A `KvBatch` takes its engine from the first
column family written to it.

| Variable | Default | Effect |
|---|---|---|
| `SOLIDB_ENGINE` | `rocksdb` | Engine for a *new* data directory (`rocksdb` or `solistore`). Ignored for an existing one. |
| `SOLIDB_DURABILITY` | eventual | solistore only. `full` fsyncs every commit (concurrent commits share it); eventual fsyncs in the background. `sync` writes (transaction commits) always wait. |
| `SOLIDB_DURABILITY_INTERVAL_MS` | 100 | solistore only: the eventual mode's fsync interval, i.e. what a power loss can lose. |

To run tests on solistore, build with the feature and set
`SOLIDB_ENGINE=solistore` (rbuild does not forward the environment: build
with `rbuild cargo db-kv test --features engine-solistore --no-run`, then run
the binaries over ssh).

### Raw rows: driver reads that never decode (2.0.5)

`FOR v IN c [FILTER v.f == x] [LIMIT …] RETURN v | v.f | {k: v.f, …}` over a
plain local collection (the FILTER only when a single-field index on `f`
serves it), sent over the driver with the query cache off, is answered by
copying the stored MessagePack bytes into the response
(`sdbql/executor/execution/raw_scan.rs` plans it, `storage/raw_rows.rs` writes
the rows, `Response::RawRows` carries them). The output must stay
byte-identical to decoding and re-encoding, which tests check — so a change to
the stored document format (`serializer.rs`: `StoredDocRef` is read by hand)
or to what `Document::to_value` returns (system fields, timestamp form) has to
be made in `raw_rows` too. Anything the planner does not recognise, and any
guard it cannot clear (row policy, search view, columnar, sharded, row
ceiling), falls back to the ordinary executor.

Since 2.8.0 two more shapes take that path:
- **An index-ordered read** (`FILTER … SORT v.f LIMIT n RETURN v | v.f | {…}`, the `index_sort` walk). Each candidate is checked against the FILTER on its stored bytes when the FILTER compiles to the typed predicates of `typed_scan.rs` (`TypedFilters`: fields compared with constants or bind variables, AND / OR / NOT, IN), else on a decode of only the fields it reads (`raw_rows::project`), then copied.
- **An index equality naming at most 64 documents.** It is answered on the connection's async worker instead of the blocking pool (`driver/handlers/query.rs`, `execute_raw_msgpack_small`); the index is read first to decide.

A whole document (`RETURN v`) is copied as runs of its stored map entries,
the system fields written where they sort (`raw_rows::write_whole_in_runs`),
when its keys are UTF-8, strictly increasing and behind their shortest string
header — what every writer stores; any other document is written entry by
entry. The per-row LETs of an index-ordered read (`index_sort.rs`,
`sorted_rows_out`) fetch a `LET v = DOCUMENT(id)` that the rest of the row
reads only as `v.field` with just those fields decoded
(`Collection::get_value_projected`), except where a row policy needs the whole
document.

### The parse cache

`/cursor` and the driver look a query's text up in `sdbql/prepared.rs` before
parsing it: one `Arc<Query>` per exact text (not a hash of it), shared by
every request whatever its bind variables, at most 1,000 texts and 4 MiB of
text, the least recently used quarter evicted past either, texts over 16 KiB
never stored. That is only right while parsing stays a pure function of the
text and nothing mutates a parsed tree: bind variables stay `@name` nodes
resolved by the executor, and anything that rewrites a tree (constant
folding, for EXPLAIN) works on a clone. Hand the shared `Arc` to a blocking
task rather than cloning the tree. `/metrics` reports it as
`solidb_query_parse_cache_{hits,misses}_total` and
`solidb_query_parse_cache_entries`.

Counts are read from an index the same way, on every transport:
`FOR v IN c FILTER v.f == x COLLECT WITH COUNT INTO n RETURN …` and
`LENGTH(FOR v IN c FILTER v.f == x RETURN 1)` (the Soli ORM's `count` with a
`where`) count the entries of the single-field index on `f` and read no
document (`sdbql/executor/execution/index_count.rs`). Both the raw path and
the count rely on an index holding exactly one entry per document under the
value's `encode_key` form; anything that changes that (array expansion,
sparse entries) must change them too.

### Rows and subqueries that skip the pipeline, or leave the query's thread

Four paths measured by `benchmarks/document_workload.py` and held to the
paths they replace by `tests/document_workload_tests.rs`:

- **A subquery over an array in memory** (`execution/array_subquery.rs`):
  `(FOR x IN <variable | expression> [FILTER …]* RETURN …)` binds each
  element in one copy of the parent's bindings. The elements of an array of
  documents are put behind `Arc`s once per array and thread
  (`ExecCaches::array_elements`). It must stay what the pipeline answers,
  errors included: several FILTERs are run clause by clause (which FILTER
  records a mistake of the query first depends on it), and a name that is
  also a collection is left to the pipeline, whose index and columnar reads
  look at the name before the row (`names_a_collection`). A clause added to
  the shape (LET, SORT…) has to keep the pipeline's order of evaluation.
- **A LET or a RETURN on the scan threads** (`execution/par_rows.rs`): for
  256 rows or more, when the expression holds a subquery or `DOCUMENT()`,
  writes nothing and calls only what a memoised subquery may call
  (`subquery_memo::is_nondeterministic`), outside a transaction. Sound only
  while everything the evaluator shares is behind a lock or atomic and keyed
  by what it describes; a new `ExecCaches` field that keeps "the last one"
  of something per query must hold under several threads. A FILTER mistake
  recorded during such a run is forgotten and the clause run again on one
  thread, which decides which mistake the query fails with.
- **A COLLECT behind a range on a single-field index**
  (`scan_collect.rs`, `try_range_collect`): `>` and `>=` only, folded from
  the stored bytes in the index's order, the range cut into slices of key
  space (`Collection::key_range_slices`) that threads take one by one, rows
  numbered by slice (`CollectFold::number_from`). It is never handed to a
  scan of the collection, however wide the range: a scan reads in key order
  and the first row of a group shows (`UNIQUE`, a key that is `1` here and
  `1.0` there). `<` / `<=` are read downwards by the ordinary path and are
  left to it for the same reason.
- **A filtered REMOVE** (`execution/scan_remove.rs`):
  `FOR x IN c [FILTER …]* REMOVE x | x._key IN c` with nothing after it
  checks the FILTERs on the stored bytes (`TypedFilters::compile_unsorted`)
  and keeps keys only, then calls the same `delete_batch_strict`. It steps
  aside at the guards of `scan_collect_source`, for a row policy, a
  transaction, OPTIONS, `OLD`, and a sharded target.

`Collection::delete_batch_checked` reads its documents with `multi_get`, and
from 64 documents decodes only `IndexMetaSnapshot::read_fields` of each when
the collection keeps no versions and has no change subscriber: a new thing
a delete must read of a document (as blobs, vectors and versions are handled
after the batch) has to be in that list or turn the slim read off. A delete
of several documents announces them with `publish_changes` (one
invalidation, the coarse generation), one document with `emit_change`.

### Raw writes: driver writes that never decode (2.9.0)

The write-side twin. A driver `insert`, `bulk_insert` or `update` with
`merge: true` is stored straight from the client's MessagePack:
`storage/raw_write.rs` transcodes each document into its stored form
(`canonical_document`) and decodes only the fields an index reads
(`indexed_view`), `storage/collection/raw_insert.rs` writes it, and the reply
is written from the stored bytes (`raw_rows`). The stored bytes, index entries,
replies, events and errors must stay identical to the decoded path's, which
tests check (`raw_write`, the fuzzed `codec` tests, `driver/handlers/document.rs`)
— so the transcoder must take exactly what the driver's direct decoder
(`codec.rs`, `frame::value`) takes and write what `write_msgpack` writes for
the decoded value. A change to either, to the stored format, to
`Document::with_key`/`update` (keys, revisions, timestamps, system fields) or
to what `insert`/`merge_update` check has to be made there too. Collections
whose writes need a decoded document — neither a plain `document` nor a
`timeseries` collection, a schema, versioning, a vector index — an active
replication log, and any document it refuses go the decoded way
(`Collection::raw_write_ready`, `document::prepare_raw`). A `timeseries`
collection takes the raw path for inserts only: its update is sent the
decoded way, which refuses it (`Collection::refuses_updates`). A document
that arrives without a key gets a UUIDv7 made in `raw_insert::document_key`,
and the write does not look for a stored document under it; a new way to
give a document a key that a client could have chosen must not report it as
made here.

### Reads a transaction has checked at commit (2.9.4)

At `repeatable_read` and `serializable` (`IsolationLevel::tracks_reads`) a
transaction's queries read through a tracked `Collection` handle
(`QueryExecutor::get_collection` → `ReadSet::track`), and the keyspace layer
records what passes (`storage/keyspace.rs`, `ReadTracker`): a `get` /
`multi_get` of a `doc:` key is a point read (key and revision, or absent);
an iterator, `for_each_ks`, split points, or a `get` of any other data key
marks the collection as read by range; metadata prefixes (`idx_meta:` …) are
ignored. `StorageEngine::commit_transaction` checks it
(`Collection::validate_reads`): point reads against the stored revision,
under the same key locks as the writes; a ranged collection against its
keyspace's write generation (`KsState`, bumped by
`Collection::invalidate_cached_queries`), holding that keyspace's commit gate
exclusively while other transaction commits hold it shared.

Rules that keep it sound:

- A read that answers from anywhere but the stored keys — a cached count, an
  in-memory index, another store — must call `Collection::note_range_read`
  (as `vector_search` and `COLLECTION_COUNT` do) or be refused with
  `QueryExecutor::refuse_untracked_read` (columnar, sharded).
- A new data key prefix is ranged by default; only add it to
  `METADATA_KEY_PREFIXES` if reading it says nothing about the documents.
- `get_collection_for_write` hands out the untracked handle on purpose: what
  a write reads on its way is checked by the commit itself, and tracking it
  would turn every insert into a range read of its collection.
- A document write that does not reach `invalidate_cached_queries` is
  invisible to the range check.

At every level, the executor's point reads (`key_fast_path`, `get_visible`)
return what the transaction staged (`staged_in_transaction`), and so do its
scans: once the transaction has staged a write to a collection
(`QueryExecutor::tx_wrote`), `scan_bounded` and `get_for_source_docs` lay the
staged documents over the stored ones in key order (`overlay_staged`), and
every path that answers from an index or from the stored bytes steps aside
for that scan, at the same guards as for a row policy
(`row_policy_applies(..) || tx_wrote(..)`). A new fast path needs that guard.
Not overlaid: the functions that search an index (`FULLTEXT`, vector, geo),
graph traversals over the edge index, `AS OF` scans.

### Commits written in groups (2.9.4)

`StorageEngine::prepare_commit` does everything a commit does before its
write (locks, read checks, staging into the batch) and returns a
`PreparedCommit` that owns what it holds: key and unique-value locks, and the
commit gates as `HeldCommitGate` values rather than guards, so that it can
cross threads (`parking_lot`'s `send_guard` feature is on for the same
reason). `commit_transaction` writes it in place. `commit_transaction_grouped`
hands it to the `solidb-commit` thread (`storage/group_commit.rs`), which
merges the batches of every commit waiting (`KvBatch::append`), writes once,
then finishes each commit and wakes its connection.

Rules that keep it sound:

- What a prepared commit holds is released by the commit thread, never by the
  waiting task: a task needs a free worker to be polled, and a worker blocked
  on a key a suspended task held would wait for ever.
- A commit is visible, and its locks free, only after its group's write
  returned: nothing is acknowledged or readable before its fsync.
- The driver takes this path only under load (`commits_in_groups`, the same
  signal as `off_the_worker`): at one client the commit in place is 14 µs
  shorter than any hand-off.
- It relies on `SOLIDB_IO_POLL_INTERVAL` being low: once commits stop putting
  threads to sleep, workers stay busy and, at tokio's default, read arriving
  requests only every 61 tasks.

### Foreign keys

A child collection declares that a top-level field holds the `_key` of a
document of a parent collection of its database, or null
(`storage/foreign_keys.rs`, `storage/collection/references.rs`; stored on the
child under `_stats:foreign_keys`; endpoints in
`server/handlers/foreign_keys.rs`). A write that sets the reference checks the
parent (`Collection::check_references`); a delete in a referenced collection
finds the children through the index on the field and is refused, deletes
them or nulls their field (`on_delete`) in the same batch, by going through
the commit path (`Collection::delete_referenced` → `group_commit::prepare` →
`foreign_keys::check_operations`, which is also what checks a transaction's
commit as a whole).

Rules that keep it sound:

- **The gate, one per database, before any key lock.** Every write to a
  collection that takes part in a key holds it shared
  (`Collection::enter_gate`, taken in `with_key_locked` and at the top of
  each batch path); a delete in a referenced collection holds it exclusively
  and takes no key lock of its own. A new write path that calls `lock_keys`
  directly must call `enter_gate` first, and a new delete path must branch on
  `Collection::referenced()` as `delete`, `delete_batch` and the TTL reaper do.
- A document write that reaches storage without `check_references` (on the
  decoded value, or on the indexed view, which holds the field as long as the
  key's index exists) is unchecked. `raw_write_ready` refuses the raw path
  when the view would not hold the field.
- What a collection has to do with foreign keys is cached per handle and per
  database against an epoch (`foreign_keys::bump_epoch`): anything that adds
  or removes a key, or drops a collection or database that takes part in one,
  must move it.
- A cascade reaches the other collections through the engine's own handle
  caches (`foreign_keys::resolve`), never a handle built aside: counts,
  change events and vector indexes live on the cached one.
- The field holds the parent's `_key`, or with `by: "id"` its `_id` in
  either form (`coll/key`, `db:coll/key`) — the default for an edge's
  `_from` / `_to`. `foreign_keys::reference` is the one place that reads a
  reference out of a document, and `referring_values` the one that says
  which index values name a parent: a new form goes in both.
- **Replication.** Storage reports what a delete cascaded into
  (`StorageEngine::observe_cascades`; `create_router` appends those writes
  to the replication log, which is otherwise fed above storage). A key's
  definition travels as `CreateForeignKey` / `DropForeignKey`
  (`Collection::apply_foreign_key`). Every write a replication path applies
  must run inside `foreign_keys::unchecked`, which makes the collection
  behave as if it had no key: entries arrive out of order, and a cascade
  arrives as its own entries. A new apply path that forgets it will refuse
  or repeat what the origin did.
- A full sync sends each collection's foreign keys and indexes
  (`SyncMessage::FullSyncDefinitions`, `Collection::index_specs`): a
  definition made while the replication log was deferred is in no log.
- Sharded collections are refused (`num_shards > 0`; a stored configuration
  with no shard, which setting a collection's properties leaves behind, is
  not sharding).
- `solidb-dump` writes `foreign_key` records and `solidb-restore` declares
  them after the data, releasing the keys around a `--drop`.

| Variable | Default | Effect |
|---|---|---|
| `SOLIDB_FK_MAX_CASCADE` | 100000 | Documents one delete may cascade into (deleted or nulled) before it is refused. |

### Counter fields

A `document` or `edge` collection declares that a top-level field is
numbered by the database on insert (`storage/counters.rs` for the definition and the rules,
`storage/collection/counters.rs` for the collection side, `sync/counters.rs`
for what is logged; endpoints in `server/handlers/counters.rs`). Each insert
gets the next number, never twice the same and with no gap; the number may
restart each day, week, month or year of a timezone, between a prefix and a
suffix that hold the date. Definitions are stored on the collection under
`_stats:counters`, each counter's state (`next`, the `date` of the last
number, an `epoch`) under `_stats:counter:<field>`.

Rules that keep it sound:

- **The state is written in the batch that stores the numbered documents**
  (`Numbering::stage`, `Collection::write_numbered`), never apart: that is
  what gives a number back when a write is refused or lost. Nothing is kept
  in memory; the state is read under the lock.
- **A durable insert is synced after it leaves the lock**, where a write
  leaves that to do (`Kv::sync_wal`: RocksDB, and solistore unless
  `SOLIDB_DURABILITY=full`): the write goes
  in unsynced under the lock, takes a stamp (`counters::WalSync::wrote`),
  and `Numbering::settle`, which every non-transactional numbered write
  ends with, waits for a sync that covers it; the inserts waiting share
  one. Sound because a log is durable up to a point: what a power loss
  takes is the latest numbers, never one in the middle. The stamp must be
  taken after the write has returned, and nothing may acknowledge the
  insert before `settle`. With `SOLIDB_DURABILITY=full` solistore fsyncs
  every write as it is made, under the lock: about 450 inserts a second
  into a collection with a counter, whatever the clients. Measured over
  HTTP, on either engine: 453 → 1,712 inserts a second from 8 clients,
  3,466 from 32.
- **One lock per collection, held from the number to the write**
  (`KsState::hold_counter_lock`, a value that can cross threads like the
  commit gates). Order: foreign-key gate, commit gates (commits only), then
  per collection in name order the counter lock, the key locks, the unique
  stripes. A number must be in the document before its unique values are
  read and before it is serialized: `insert_batch` numbers before its
  second pass when a counter exists.
- **Every path that creates a document numbers it or refuses.**
  `insert_internal`, `insert_or_replace` (for a key that is not stored) and
  `insert_batch` number; a transaction's inserts are numbered when its
  commit is prepared (`number_operations`, before
  `lock_for_transaction_reading`), the number carried into the updates the
  transaction made to the same key, and a key inserted then deleted takes
  none; `upsert_batch` takes the lock before its keys and numbers the
  documents whose key was not stored (`number_holding`);
  `raw_write_ready` sends the collection the decoded way. A new insert path
  must do one of the two.
- **The client neither sets nor changes the field**: `expect_numbers` /
  `refuse_given_numbers` on insert, `numbers_unchanged` in
  `write_update_core`, `update_batch_guarded`, `upsert_batch` and
  `update_tx_guarded`,
  `keep_numbers` where a replace would drop it. An update path that reaches
  storage without one of them lets a number be rewritten.
- **A replicated write is not numbered**: `Collection::counters` returns
  nothing inside `foreign_keys::unchecked`, which every apply path runs in.
  The numbers travel in the documents.
- **One node numbers** (`CounterDef::node`, checked by `check_owner` under
  the lock). It is pinned only when the instance was given `--node-id`
  (`StorageEngine::set_local_node`): an id made up at startup names
  nothing. The numbering moves by a handover the owner makes
  (`move_counter`), which stops it numbering in the same write and
  replicates the exact state; a peer's state is otherwise only as fresh as
  the last record it was sent. When the owner is lost, another node takes
  the counter (`take_over_counter`, `force` on the node endpoint): it reads
  the numbers back out of the documents it holds (`CounterDef::parse`,
  which must read every value `render` writes) and goes on after the
  highest of the current period. That is best effort by nature: a number
  given to a document that was not replicated is given again, and
  replication does not check unique values when that document arrives.
- **Records are applied in any order** (`PutCounter` / `DropCounter`,
  `SyncMessage::FullSyncCounters`): the higher `epoch` wins, then the state
  further along (`counters::supersedes`), and a dropped counter keeps its
  state key as a record of the drop, so that an older record replayed from
  a log does not declare it again. That is why these operations are not in
  `in_full_sync`.
- **The date a number is rendered with never goes back**
  (`CounterDef::take`): a clock stepping back must not give a closed
  period's numbers again. With a reset, the prefix and suffix must name the
  period (`CounterDef::check`).
- The unique index a counter stands on cannot be dropped while it is
  declared, and a collection with a counter cannot be sharded.
- `solidb-dump` writes `counter` records with their state; `solidb-restore`
  takes the same counter off the target, loads the documents with their
  numbers and declares it afterwards.

### What a joining node replays after its full sync

A node that joins asks one peer for a full sync, then pulls every peer's
log from sequence 0. The full sync ends with `SyncMessage::FullSyncOrigins`:
how far the serving node had applied each origin's log, its own included,
taken before it read the first document. The joining node keeps it as a
floor per origin (`SyncState::raise_snapshot_floor`, persisted) and does not
apply, at or below it, an operation the snapshot reflects
(`worker::in_full_sync`): document writes, and the creation, drop or
truncate of a database, a collection, an index or a foreign key. Without it,
an insert from one peer's log brought back a document another peer had
deleted before the snapshot (card #844), and one peer's `DeleteCollection`
or `TruncateCollection` emptied a collection another had recreated or
refilled (card #853).

Blob chunks are not in a full sync and are still replayed from the logs.
Columnar collections are, when the serving node says so, under a floor of
their own (see the columnar section). A new operation that the full sync carries belongs in
`in_full_sync`, and one it does not carry must stay out of it; whatever a
skipped operation used to bring (a collection's shard configuration came
with `CreateCollection`) has to be in the full sync's own messages. `FullSyncDefinitions` and
`FullSyncOrigins` are appended to `SyncMessage` (bincode identifies variants
by position): an older requester logs an unexpected message and carries on.

### Graph traversals

A traversal (`FOR v[, e[, p]] IN min..max OUTBOUND … edges`) and the path
searches expand a vertex through `EdgeExpander`
(`sdbql/executor/execution/graph.rs`).

- **Edge indexes.** An edge collection carries four: on `_from`, on `_to`,
  and on `[_from, _to]` and `[_to, _from]` (`Database::create_edge_indexes`;
  `EdgeExpander::new` creates the missing ones of an older collection at
  its first traversal). The two-field ones answer "where do this vertex's
  edges lead" from their entries (`Collection::index_pairs`,
  `EdgeExpander::far_ends` / `far_steps`): no edge document is read unless
  the query reads the edge. That relies on the entry key of a two-field
  index, `idx:<name>:<hex(first)>_<hex(second)>:<key>` (`idx_entry_key`),
  and on a string being encoded `04 <utf-8> 00` (`codec::encode_key`): a
  change to either has to be made in `index_pairs` too.
- **Level by level.** A breadth-first traversal with no path variable and
  no `PRUNE` (each vertex once, or once per path) collects the edges of a
  whole level, then reads its vertices, and the edges it returns, in one
  batch each (`read_kept`), decoding only the fields the query reads
  (`projection::traversal_projection` / `traversal_edge_projection`). The
  older frame-by-frame loop below it serves depth-first walks, `PRUNE` and
  paths. Both must return the same rows in the same order.
- **Kept between queries.** `storage/kept.rs` keeps, per collection, the far
  ends of each expanded vertex's edges, the documents read with each
  projection, and whole neighbourhoods with their documents. What keeps it
  right: a single document write (`Collection::emit_change`) forgets that
  document and, for an edge, the neighbourhoods of both its ends
  (`kept::changed`); anything else that calls
  `Collection::invalidate_cached_queries` (a batch, a truncate, an index
  created or dropped) moves the collection's *coarse* generation
  (`KsState::coarse_generation`), which empties its maps. A reader notes a
  map's epoch before it reads storage and its entry is kept only if no
  write forgot anything meanwhile (`Kept::put`). A read served from a map
  calls `note_range_read`. A write that reaches neither function would
  leave a traversal reading what was there before, as it would leave the
  query cache; a new write path that announces documents one by one must
  pass an event with the edge's ends (`data` / `old_data`), or `changed`
  falls back to the coarse generation.
- **One shortest path** without a weight is searched from both ends
  (`EdgeExpander::shortest_unweighted`); the weighted and the several-paths
  searches go through `paths.rs` and the `Neighbours` trait.

| Variable | Default | Effect |
|---|---|---|
| `SOLIDB_GRAPH_CACHE_MB` | 256 | What one map of kept neighbourhoods or documents may hold; a full map stops taking entries. `0` turns the maps off. |

### Column blocks: time series aggregations that read arrays

A `timeseries` collection may declare a time field and the fields its
aggregations read (`PUT …/collection/{name}/blocks`, `storage/blocks.rs`,
stored under `_stats:column_blocks`). Its documents stay the source. Beside
them, for a range of keys `[lo, hi)`, a *block* (`blk:<lo>`) holds those
fields as arrays: rows ordered by the string fields then by time (one series
is a run, found by bisection), a dictionary per string field, the rows' order
by time, and exact subtotals per 512 rows of it. A
`FOR p IN c FILTER … COLLECT … AGGREGATE …` that stays within the blocks —
FILTERs comparing the time with numbers and other fields with strings, keys
that are fields or `TIME_BUCKET(time, "<fixed interval>")`, aggregates of
fields — is folded from them (`sdbql/executor/execution/block_scan.rs`) into
the same `CollectFold` as documents, and whatever no block covers is read as
documents. Measured on 8.64M points (`benchmarks/compare/ts.sh`): a series'
hour 925 → 42,700 requests/s, ten minutes of every series 9.7 → 38,200.

Rules that keep it sound:

- **Only a collection that refuses updates.** A block is wrong only if the
  set of keys in its range changed; `set_column_blocks` refuses anything but
  `timeseries`. A new way to rewrite a time series document in place would
  break every block.
- **Every document write announces its keys**, after the write, through
  `Collection::publish_change(s)` (which take the key, or a `Wrote` span)
  or `emit_change`; truncate announces `Wrote::All`. The block holding a
  key goes (`BlockSet::wrote`). A write path that reaches neither leaves
  stale blocks, as it leaves stale cached queries.
- **A seal registers its range before it takes its snapshot.** A write
  announced before the registration is in the snapshot; one announced after
  finds the pending range and the block is not kept. Keep that order.
- **An append announces nothing, and takes no lock**, when its first key
  is a UUIDv7 generated at or after the set's `frontier` (an instant in
  milliseconds). That is sound only because a seal raises the frontier to
  now *before* its snapshot and ends its block before the first key
  generated from now on, and only while every key in sight is a generated
  one: the first other key a seal meets turns the frontier off for good
  (`u64::MAX`), and that seal gives up. A change to how keys are generated
  or ordered has to be made in `blocks::key_ms` too.
- **A block read from disk is not trusted** until the keys of its range
  have been counted and hashed again (`BlockSet::verified`, once per
  process): the announcement follows the write, so a crash between the two
  leaves a block that no longer matches.
- **Values are the documents'.** A numeric column is all integers, all
  floats, or both with a flag per value (`Column::Mixed`: JSON metrics hold
  `12` beside `12.5`); a range where one document does not fit (a missing
  field, a time that is not an integer) gets a block without columns and is
  read as documents. `SUM` over a slice goes through `ExactSum::add_slice`, which
  adds the float sum only when no addition lost anything, and what a block
  keeps of a column or of 512 rows is the exact state of their sum
  (`Total`, a settled `ExactSum`, merged without loss): results must stay
  those of the ordinary path, which tests check
  (`tests/column_blocks_tests.rs` runs each query both ways). The one
  difference that can show is which of `5` and `5.0` a `MIN` / `MAX`
  returns when both are the extreme.
- The fold steps aside at the same guards as `scan_collect` (row policy,
  a transaction that wrote the collection, columnar, sharded) and calls
  `note_range_read`.
- Sealing runs on one thread per database instance (`solidb-seal`,
  `blocks::Sealer`): a write that leaves 1,536 documents or more outside
  the blocks — new points, or a range whose block it threw away — queues
  its collection, and does not wait. The thread seals 1,024 documents at a
  time, the newest 512 left out, and merges adjacent small blocks up to
  65,536 rows. When whole blocks wait (a bulk load writes faster than one
  thread seals) a pass reads up to several blocks' documents under one
  registration and one snapshot and builds the blocks on as many threads
  (`seal_range`, `at_once`: half the CPUs for the thread, all of them, at
  most 8, for `POST …/blocks/seal`); the blocks follow one another, and
  the first one a write fell in is dropped with those after it. `StorageEngine`'s `Drop` stops and joins it
  (`blocks::stop_sealer`) before the database closes, as it joins the
  column-family droppers: a thread still holding a collection at exit is
  what crashed `rbac_admin_endpoints_tests`. After a restart nothing is
  queued until a write: `POST …/blocks/seal` seals what is left.
- **The newest row of a series** (`FILTER p.series == x SORT p.time DESC
  LIMIT 1`, the small sorted read of the driver) is answered from the
  blocks by `block_scan::newest_from_blocks` when they hold every
  document (`tail_is_empty`, no gap, every block checked and in memory),
  the series is the blocks' one string column and the row is the only one
  at its instant: ties are ordered by key, which a block does not hold,
  so a tie, like any FILTER the row fails, is the index's to answer. It
  must return what the index walk returns
  (`the_newest_point_of_a_series_comes_from_the_blocks`), calls
  `note_range_read`, and sits behind the guards of `sorted_read_target`.
- Blocks are local to a node: not replicated, not in a full sync, not
  dumped. The definition is not replicated either; declare it on each node.

| Variable | Default | Effect |
|---|---|---|
| `SOLIDB_COLUMN_BLOCKS_MB` | 1024 | What the columns kept in memory may take, for the whole process. Past it a block's columns are read from disk at each use. |

### Columnar collections: rows stored in blocks

A columnar collection (`storage/columnar.rs`) stores a row in exactly one
place (`storage/columnar_blocks.rs`): the *tail* (`col_row:{uuid}`, one key
per row, MessagePack) or a sealed *block* (`col_blk:{uuid of its first
row}`), up to 65,536 rows in row-id order with one array per column of the
schema — integers, floats, strings as a sorted dictionary and codes,
booleans, each with a null mask, or JSON text for a column whose values are
of several kinds. An insert of 4,096 rows or more is sealed at once; smaller
inserts wait in the tail and are sealed when it reaches that size. Through
2.10.4 every cell was a key (`col:{column}:{uuid}`) read with a point read;
a collection in that format (`ColumnarCollectionMeta::format` below 2) is
converted when it is loaded (`migrate`).

`FOR r IN c [FILTER …] COLLECT … AGGREGATE …` over one is folded from the
arrays (`sdbql/executor/execution/columnar_scan.rs`) when the FILTERs
compare columns with constants or with each other (`==` `!=` `<` `<=` `>`
`>=`, `IN` / `NOT IN` a list, joined by `AND` / `OR` / `NOT`) or are any
other condition computed from the columns, the keys are columns,
`TIME_BUCKET(column, "<fixed interval>")` or expressions of the columns,
and the aggregates are `COUNT`, `SUM`, `AVG`, `MIN`, `MAX` or
`COUNT_DISTINCT` of a column or of an expression of the columns: on several threads, into a
table of groups handed to the same `CollectFold` as rows, with the exact
state of each sum. A single integer key of small range is grouped in plain
arrays indexed by the key (`Flat`): a sum and, beside it, the total of what
its additions rounded away, which together are the exact sum for as long as
adding those errors rounds nothing (`ok`); when it does, the rows are read
again with exact accumulators. A `SORT … LIMIT` on an aggregate keeps only
the groups that can be returned. Measured on ten million rows against
ClickHouse (`benchmarks/compare/col.sh`): ahead on the eight rows, 7% to 2.4
times; 2.10.4 answered each question in 2.4 s on one million.

Rules that keep it sound:

- **Every write holds the collection's shared write lock** and updates the
  decoded blocks kept for all handles (`BLOCK_STORES`, by database and
  collection) after its batch is written, never before. The entry holds a
  `Weak` on the database so that its address cannot be reused by another.
- **Create, insert, delete, truncate, drop and a column's index go through
  `sync::columnar`**,
  on every transport: it does the work and appends what a peer applies
  (the `apply_*` functions, which the replication worker calls and which
  must stay idempotent: a joining node replays logs from their start).
  Only the HTTP insert used to be logged, so a peer never had the
  collection (cards #882, #883). A delete by filter is logged as the ids
  it deleted, never as the filter: a peer holding other rows would answer
  it differently.
- **A full sync sends a columnar collection whole** (`SyncMessage::
  FullSyncColumnar`, after the `FullSyncCollection` of its backing
  `_columnar_<name>`): definition, rows under their ids in byte-bounded
  batches (MessagePack, as the tail: JSON text loses floats), then
  indexes, each applied by `sync::columnar::apply_full_sync_part`, which
  takes any part twice. The serving node says so first
  (`FullSyncColumnarIncluded`), and only then do its origins raise the
  *columnar* floor (`SyncState::raise_columnar_floor`), below which
  `worker::columnar_in_full_sync` operations are not replayed. An older
  serving node sends neither, and the logs are replayed as before. A new
  columnar operation belongs in `columnar_in_full_sync`.
- **An index answers only for the rows it holds.** It is written once, by
  a build, and names rows by position in id order; inserts do not add to
  it. Its entries are built in memory and written in one batch with the
  schema: written a row at a time, each row rewrote the list of its
  value, which was quadratic (11.7 s for 80,000 rows, card #916). A list
  of positions is stored packed (`encode_positions`: a marker byte, then
  LEB128 distances); `decode_positions` still reads the JSON array of
  2.12.0 and earlier, and is the one place that reads a list.
  `ColumnarCollectionMeta::index_rows` says how many rows it holds, and
  `index_base` how many rows left the front since it was written: a
  stored position is the row's position now plus that base
  (`filter_positions` subtracts it, for every kind of index). Every write
  that moves a position must say so in the batch that writes the rows: a
  delete of exactly the first rows (`delete_found` checks, `prune_before`
  when the rows are in id order) raises the base (`shift_indexes`); any
  other — a delete in the middle, a row with an older id (replication) —
  lowers the count (`cap_indexes`); a truncate zeroes both. A new write
  path that forgets it brings card #901 back (rows missing from filtered
  results). Each of them also calls `note_moved`, which is what makes a
  build that read the rows before it start again. A column with no entry
  in `index_rows` was indexed by 2.10.4 or earlier: its index is not
  used.
- **An index is built outside the write lock** (`build_index`): the lock
  is held to take the rows as they stand and again to write, and the
  write goes ahead only if no position moved in between (`Kept::moved`);
  the third attempt holds the lock throughout. Rows appended meanwhile
  are simply after those the index holds. A background build
  (`start_index_build`, `POST …/index?wait=false`) is written in the
  metadata (`index_builds`) before its thread starts and removed by the
  batch that stores the index, so `ColumnarCollection::load` starts again
  one that a restart interrupted (`resume_index_builds`: no read for a
  collection with none pending). The build gives up when its mark is gone
  (`drop_index` removes it) and when the database closes:
  `StorageEngine`'s `Drop` stops and joins the threads
  (`stop_index_builds`), as it joins the sealer. The index is logged for
  the peers when the build is accepted, not when it ends: a resumed build
  has no log at hand. `?rebuild=true` is local and not logged.
- **The HTTP aggregate and the driver's go through one pass**
  (`group_by_rows`, `aggregate_rows`, `each_row`): the rows a filter
  keeps, by `filter_positions` as `/query`, each read in its block's
  arrays. A group is still the text of its keys joined by `|`, and a key
  that reads as a number comes back as a float: results must stay those
  of 2.12.0, which read each row by id (two minutes for 3.65 million).
- **A block names its columns**, so the schema can change
  (`add_column`, `drop_column`, `rename_column`, all through
  `change_schema`). An add writes the schema only: a block sealed before
  has no array of that name, and every reader takes a missing array as a
  null column (`ColBlock::column` is an `Option`; keep it read that way).
  That is why an added column is nullable. A drop and a rename write every
  block and tail row again without the column, or under its new name, in
  the batch that stores the schema, and empty the decoded blocks kept in
  memory: values left behind would be found by a column added later under
  the same name. A column with an index is left alone until the index is
  dropped (an index is stored under the column's name). Replicated as
  `ColumnarAddColumn` / `ColumnarDropColumn` / `ColumnarRenameColumn`,
  each applied once (`apply_column_change`).
- **A `FOR` over a columnar collection builds what the query needs**
  (`sdbql/executor/execution/columnar_rows.rs`): the columns of the scan's
  projection, and the rows its leading FILTERs keep, through
  `ColumnarCollection::scan_filtered`. The FILTER clauses still run on
  those rows, so a pushed filter may keep too much but must never keep too
  little: a comparison is pushed only when the collection answers it
  exactly as SDBQL does. Today: an equality with a constant of the
  column's own kind (the collection compares an equality as JSON values,
  to which `3` and `3.0` differ), and a range of an integer column against
  an integer or of a float column against a number a float holds. To
  SDBQL null is below every number and the collection's ranges keep
  numbers only, so a `<` / `<=` is pushed with the null rows beside it
  (`or_null`) unless another conjunct bounds the column from below — it
  lost those rows before (card #893). A new comparison pushed down has to
  hold to that, and `tests/columnar_rows_tests.rs` runs each query on a
  document collection with the same rows. A scan limit stops the read and
  pushes no filter (the limit counts rows before the FILTERs). A filter
  whose constant names an outer row is not pushed. EXPLAIN follows the
  same code (`columnar_note`, `columnar_scan`).
- **A count is read from the arrays only when the collection answers the
  whole FILTER** (`columnar_count`, called from `index_count.rs` for
  `COLLECT WITH COUNT INTO` and `LENGTH(FOR … RETURN 1)`): no row is
  built and no FILTER runs afterwards, so there a pushed comparison must
  keep exactly the rows SDBQL keeps, not merely never too few. One
  conjunct that is not pushed sends the query down the ordinary path. A
  comparison pushed down that may keep too many rows would be wrong for
  counts only: keep `pushed_term` exact, or make it say which it is.
- **A filter is answered from the arrays, block by block**
  (`columnar.rs`: `filter_positions`, `scan_leaf`, `scan_block`), for
  SDBQL and the HTTP / driver query alike. `scan_block`'s typed arms
  (integers, floats, a string equality, null) must keep exactly the rows
  `leaf_keeps` keeps value by value, which is what the tail, a column
  stored as JSON and rows out of id order go through. A block of integers
  or of floats is passed over, or taken whole, from its smallest and
  largest value (`Column::int_range` / `float_range`, which count the
  placeholder of a null: bounds only ever wider, and a block with nulls
  is never taken whole); an `AND`
  goes block by block so that one side ruling a block out spares the
  others (`block_rules_out`). An index answers an equality or an `IN`
  (`index_for`), for the rows it holds; a range against a number is never
  an index's. The `minmax` kind that existed for ranges was removed:
  `ColumnarCollection::load` drops one a collection still holds
  (`retire_minmax`, on the raw JSON: the kind no longer deserializes) and
  `sync::columnar::index_kind` takes one from an older peer as no index.
  `drop_index` deletes the entries of every kind of index for the column;
  a new kind with its own key prefix has to be added there, to truncate
  and to drop. Positions come back ascending from every branch: `in_both`
  relies on it.
- **A `timestamp` column is compared by instant** (`columnar.rs`:
  `Instants`, `scan_instants`, `instant`), not as the values written: the
  column takes a string or an integer, so one instant has many writings.
  A range used to keep no row (ranges compare numbers only) and an
  equality only the writing inserted. `filter_positions` takes that way
  for every leaf on such a column, before any index: `index_for` gives
  none, since an index holds the values as written. A date is read as
  the SDBQL date functions read one (`parse_date_str`,
  `number_to_epoch_ms`: seconds below 10^10, else milliseconds), so a
  block's integer bounds decide only when both are in the same unit
  (`instant_ends`, which `by_block` uses in place of `block_rules_out`).
  SDBQL pushes nothing down for a `timestamp` column
  (`columnar_rows.rs`), and must not until its comparison is this one.
- **Compacting merges runs of neighbouring blocks** that fit in one
  (`ColumnarCollection::compact`, `POST …/compact`): the rows of a run are
  sorted by id and sealed again as one block, each run under the write
  lock, which is released between two. Rows and their order stay, so
  positions do not move and the indexes are left alone: a change that
  made compaction reorder or drop rows would have to lower their counts
  (`cap_indexes`). Local to a node, not logged, and only on demand.
- **A retention removes the oldest rows, by blocks**
  (`ColumnarCollectionMeta::retention_secs`, `prune_before`): a row's age
  is the instant in its id, rows are in id order, so the old ones are the
  first blocks. A block wholly older than the cutoff goes as one key; the
  one the cutoff falls in is sealed again from the rows it keeps. The TTL
  worker calls it for every `_columnar_` collection (`ttl::TtlWorker`),
  and `POST …/prune` on demand. It raises the indexes' base
  (`shift_indexes`), so they go on answering. The setting is replicated
  (`ColumnarSetRetention`, and the definition a full sync sends); what a
  node removes is not logged, since each node removes by the same rule.
  A new way to give a row an id that is not the instant it was inserted
  changes what a retention removes.
- **SDBQL writes one way**: `INSERT … INTO` a columnar name is taken by
  `QueryExecutor::insert_into_columnar` before the document path (and the
  streaming insert steps aside for it); `get_collection_for_write` refuses
  a columnar name, which is what answers UPDATE, UPSERT and REMOVE.
- **A client's rows are checked against the schema** (`conform`, in
  `insert_rows`): type, nullability, unknown columns; an integer in a
  `float64` column becomes a float, so the column stays one array. The
  paths that take a row under a given id (`insert_row_with_id`,
  `insert_rows_with_ids`: replication, full sync) do not check: the origin
  accepted the row, and a replica that refused it would diverge.
- **A value read back is the value inserted** (but for that float). The
  array's kind is chosen from the values, and the tail is MessagePack
  because JSON text does not always give a float back (`93.33333333333333`
  came back `…31`). Rows written through 2.10.4 are JSON and are still read.
- **Rows come back in the order of their ids**, as before: filters and the
  secondary indexes work on positions in that order. A replicated row may
  carry an older id; `RowSet` then orders blocks and tail together.
- A block the arrays cannot answer for (a column of several kinds, a
  constant of another kind than the column, a column the block was sealed
  without) and the tail are read as objects and folded the ordinary way:
  `tests/columnar_blocks_tests.rs` runs each query both ways, and checks
  with `BLOCKS_FOLDED_AS_ROWS` that the arrays did answer, so a shape that
  silently falls back fails there.
- **An expression of the columns is a computed column**
  (`columnar_scan.rs`, `Derived`): a key, an aggregate's argument or a
  condition made of the row's columns, constants and pure functions
  (`const_fold::is_pure_function`; anything else keeps the query on the
  rows) is computed once per block and kept for the query (`Cells`,
  shared by the threads). Arithmetic on numeric columns is computed in
  the arrays (`Sum`), and must stay `helpers::arithmetic` value for value
  and kind for kind — `arithmetic_in_the_arrays_is_the_evaluators` holds
  it to that: a change to SDBQL's arithmetic (null as 0, integers that
  overflow into floats, a division by zero as null) has to be made in
  both. Everything else goes through the evaluator, so it is right by
  construction; an expression of one string column is evaluated once per
  string of the block, which is only sound because the functions are
  pure. Integers and floats together make a column only under an
  aggregate (`ColData::Mixed`, never stored); as a key or a condition
  they send the block to the rows. `COUNT_DISTINCT` keeps each group's
  values (`Acc::distinct`, null among them) and hands them to the fold
  as `Slice::Each`; a query that has one goes a row at a time and is
  never cut to its top groups by that aggregate.
- **Nulls in the fold follow SDBQL** (`columnar_scan.rs`): to a
  comparison a null is below every value and equal to none (`select`:
  it passes `!=`, `<`, `<=`); a null key is a group of its own
  (`Atom::Null`); an aggregate passes a null over, which the group says
  with `Acc::missing` and the fold reads as `Slice::TotalOf` /
  `OtherOf` (how many of the rows hold a value). Rows with a null among
  their keys or arguments go one at a time (`fold_with_nulls`); the
  arrays indexed by position (`Flat`, `Share::Ranged`) hold no null, so
  the caller takes that way only when no block's head says a key or an
  argument holds one. The groups of one numeric key are found in
  `Table::numbers` on both ways, so a key is one group in a table: the
  top-k pruning counts on it.
- **The decoded blocks are kept in memory up to a budget**, ids of the
  rows included (0.9 GB for ten million rows of six columns).
  `SOLIDB_COLUMNAR_BLOCKS_MB` (default: a quarter of the memory the
  process may use, at least 1 GiB) bounds what every collection keeps
  together (`columnar::keep_within_budget`): past it the collections read
  longest ago give their blocks back (`Kept::blocks` goes back to `None`,
  as after a start) and read and decode them at their next use. A
  collection that alone is past the budget keeps nothing between two
  queries, and its `COLLECT … AGGREGATE` is folded a few blocks at a time
  (`columnar_scan::columnar_collect_in_windows`, when
  `ColumnarCollection::larger_than_kept`): as many blocks as threads are
  read, decoded, folded into tables that are merged, and dropped, so the
  query holds a few blocks and its groups. It must give the groups of the
  other way (`columnar_memory_tests` runs both). Any other read of such a
  collection (rows returned, the HTTP query) still loads every block for
  its duration. Every place that
  sets the store goes on treating `None` as "read the disk"; `keep_blocks`
  drops a block of the same key before adding one.
  The ids of a block are its rows' UUIDs as 16 bytes each when every one
  is a UUID in its usual text (`columnar_blocks::Ids`), texts otherwise.
  Compare ids through `IdRef`, which orders them as their texts.
- **A block is stored in parts and decoded as it is read**
  (`columnar_blocks.rs`, `FORMAT_PARTS`): a head — the ids of its first
  and last row, the names, where each part is, and for each column its
  bounds and whether it holds nulls — then the ids and each column as a
  part of its own, LZ4-compressed when the collection is. A block read
  from disk decodes its ids and a column the first time they are asked
  for (`ColBlock::uuids()`, `column()`), and gives that part's stored
  bytes back: it takes what is left to decode plus what was decoded.
  `ColBlock::stats` answers from the head with nothing decoded, which is
  how a filter passes a block over or takes it whole
  (`block_rules_out`, `block_taken_whole`), and `ends()` is what orders
  the blocks. Blocks written through 2.11 are one bincode value
  (`FORMAT_WHOLE`): still read, decoded whole, rewritten in parts when a
  write seals them again. A block written in parts is not read by 2.11.
  Rules: what the head says of a column must be what the column says
  decoded (`Column::stats` is what `encode` writes); bounds count the
  placeholder of a null, so they are only ever wider; anything that
  changes `names` changes the columns and their heads with them
  (`rename_column`, `remove_column`). A read ends by counting what it
  decoded against the budget (`RowSet`'s drop, `budget_after_read`): a
  new way to hold blocks across a query should go through `row_set()`.
  A count goes block by block and gathers no positions
  (`count_filtered`, `by_block`).
- A large insert builds its columns before taking the write lock and gets
  its row ids under it: ids of one insert must follow those of the one
  before, or every query would have to order blocks and tail together.
- The hint that lets the fold keep only the groups a `SORT … LIMIT` can
  return (`ExecCaches::collect_top`) carries the address of its query's
  body: a COLLECT of another body must not take it. It keeps every group
  tied with the last one, so the SORT that follows decides as it always did.

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

`clients/` holds ten libraries and four example applications; the count of
"8 SDKs" that used to sit here counted `js-client` twice (as "NodeJS" and as
"JavaScript") and missed two.

- **Libraries**: `rust-client`, `go-client`, `PYTHON-client`, `js-client`
  (Node and browser both), `PHP-client`, `Ruby-client`, `elixir_client`,
  `solidb-laravel-eloquent`, `mobile-sdk`, `iyi-client`.
- **`iyi-client`** is only a pointer: the driver for [iyi](https://iyi.dev)
  is its own repository, `github.com/solisoft/solidb_iyi` (a clone sits in
  `Work/soli/solidb_iyi`), because `iyi get` wants `iyi.mod` at a
  repository's root and its tags as versions. Written in iyi alone, with
  its own MessagePack codec. `./run_tests.sh <solidb>` there starts a
  throwaway server and runs `iyi test`; a driver protocol change that the
  other clients follow has to be made there too, and released as a tag.
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

The solidb skill is also shipped to every new Soli app: `soli new` and
`soli update docs` write a copy into `.claude/skills/solidb/`. That copy is
embedded in the `soli` binary from the Soli repository
(`src/scaffold/templates/skills/solidb/`) and is written only by its
`scripts/sync-solidb-skill.sh`. After editing the skill, run that script
there and commit the result, or apps keep being created with the older
text; `scripts/sync-solidb-skill.sh --check` says when the copy has drifted.

### Which documentation is authoritative

`doc/` (the Soli app) is the live documentation site and is the one to update.
`docs/` is a handful of older engine markdown files kept only where they have no
page on the site — `SDBQL_REFERENCE.md` and `BACKUP.md` are current; treat
anything else there as historical. When the two disagree, `doc/` wins.
