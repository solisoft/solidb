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
| `SOLIDB_TX_LOCK_WAIT_MS` | 500 | How long a transaction waits for a document lock another transaction holds before it is refused with a conflict (`transaction/lock_manager.rs`). `0`: refused at once, the behaviour up to 2.9.3. A deadlock (two transactions waiting for each other) is refused at once whatever the value. The wait runs under `block_in_place`, so only on the multi-threaded runtime: a `#[tokio::test]` without `flavor = "multi_thread"` never waits. Transactions at `repeatable_read` / `serializable` do not wait for a write lock. |
| `SOLIDB_TX_GROUP_COMMIT` | on | Under load (when a driver commit would leave its async worker) and on an engine that merges batches (solistore), a driver commit is prepared on its connection and written by the commit thread with every other commit waiting, as one batch and one fsync (`storage/group_commit.rs`). `0`: each commit writes its own batch from its own thread, as up to 2.9.3. HTTP commits and RocksDB are not concerned. |
| `SOLIDB_IO_POLL_INTERVAL` | 4 | Tasks a busy async worker runs between two looks at the sockets (tokio's `event_interval`, whose own default is 61). Set in `main.rs`; applies to every transport. Lower reads arriving requests sooner under load (grouped commits need it: 56,000 transfers/s at 61, 75,600 at 4) and lengthens the p99 of reads at 32 clients. |
| `SOLIDB_SCAN_THREADS` | the CPUs the process may use (cgroup `cpu.max`, else `available_parallelism`) | Threads a full-scan `COLLECT` may split into, and a process-wide cap on scan threads (`execution/scan_threads.rs`). `1` turns parallel scans off. Only engines with snapshots run them, which both have (solistore since 0.3); a collection under 65,536 documents stays on one thread. |
| `SOLIDB_MAX_TOTAL_WAL_SIZE` | 2GB prod / 256MB `--dev` | Total WAL budget. This is a **flush trigger**, not a disk cap. Lowering it to save disk is a false economy — it produces thousands of tiny SSTs. Bound memory with `--memtable-budget`. |

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
whose writes need a decoded document — not a plain `document` collection, a
schema, versioning, a vector index — an active replication log, and any
document it refuses go the decoded way (`Collection::raw_write_ready`,
`document::prepare_raw`).

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
return what the transaction staged (`staged_in_transaction`); scans do not.

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

### What a joining node replays after its full sync

A node that joins asks one peer for a full sync, then pulls every peer's
log from sequence 0. The full sync ends with `SyncMessage::FullSyncOrigins`:
how far the serving node had applied each origin's log, its own included,
taken before it read the first document. The joining node keeps it as a
floor per origin (`SyncState::raise_snapshot_floor`, persisted) and does not
apply a *document* write (insert, update, delete) at or below it
(`is_duplicate_document`): those are in the documents it was sent. Without
it, an insert from one peer's log brought back a document another peer had
deleted before the snapshot (card #844).

Only document writes are skipped. Definitions, blob chunks and columnar rows
are not in a full sync's documents and are still replayed from the logs, so
a new operation that the full sync does carry belongs in that check, and one
it does not carry must stay out of it. `FullSyncDefinitions` and
`FullSyncOrigins` are appended to `SyncMessage` (bincode identifies variants
by position): an older requester logs an unexpected message and carries on.

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
