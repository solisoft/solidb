# `DELETE /_api/database/{db}` is O(collections × OPTIONS-file-size)

> **Read the 2026-09-20 update first.** The sections below are the original
> 2026-06 diagnosis and are kept for history; the drop latency they describe
> is fixed, and re-measurement since showed the dominant costs were elsewhere.
> Only the shared-CF direction is still genuinely open.

## Severity

medium — 7.3s to drop a 41-collection database on an instance with ~800 column families

## Location

- `src/storage/engine.rs` — `delete_database` (drops collections one at a time in a loop)
- `src/storage/database.rs` — `delete_collection` (per-CF `drop_cf`)

## Problem

Every collection is a RocksDB column family. `delete_database` loops over
collections calling `drop_cf` one at a time, and RocksDB rewrites + fsyncs
the entire OPTIONS file (one section per CF) on *every* CF create/drop.
With ~800 CFs the OPTIONS file is ~4MB, so each drop costs ~175-200ms and
the cost grows with total CF count across ALL databases — `soli test
--jobs 16` worker DBs alone add 600+ CFs.

Measured (2026-06-04, local instance, 794 CFs):

- drop empty database: 0.36s
- drop 41-collection database: 7.31s (~178ms/collection)
- collection truncate (range delete): 1-26ms

The C API / rust-rocksdb 0.46 only expose per-CF `drop_cf` — C++
`DropColumnFamilies` (one manifest edit for the batch) is not bindable
without a sys-crate patch.

## Possible directions

- Background the CF drops after removing the DB from `_meta` + cache;
  needs a pending-drop registry so a recreate of the same db/collection
  name either waits or claims (truncate + reuse) the doomed CF.
- Long term: reconsider CF-per-collection (prefix-in-shared-CF for small
  collections?) — CF count also inflates every OPTIONS rewrite and open.

## Update 2026-06-04

Datapoint from a larger instance: 2049 CFs → OPTIONS file is 10.5MB, so
each create/drop rewrites ~10.5MB under the DB mutex; reaching 2049
collections wrote ~10GB of OPTIONS data cumulatively. Also ~6180 SSTs
(~3/CF) from tiny forced flushes (`max_total_wal_size=50MB` shared by all
CFs).

Landed (storage refactor, same date):

- Switched to `DBWithThreadMode<MultiThreaded>` (`storage::RocksDb`) —
  `create_cf`/`drop_cf` now take `&self` with internal synchronization.
  This removes the `Arc::as_ptr as *mut DB` unsafe casts and fixes the
  dual-lock data race mentioned above (the lock-free `cf_handle()` reads
  raced the mutable CF-map writes).
- All CFs (including `Database::create_collection`, which used
  `Options::default()`) now get shared tuned options: LZ4, shared 512MB
  block cache, bloom filters.
- `list_collections` uses live `cf_names()` instead of `DB::list_cf`
  (which re-read the MANIFEST from disk per call).

Still open: the per-drop OPTIONS rewrite itself (no RocksDB API to batch
or skip it from the C API) — the two directions above remain.

## Update 2026-06-05 — background drops landed

Re-measured before the fix: 1794 CFs → 9.2MB OPTIONS file, ~400ms per CF
create/drop, `DELETE /_api/database/bonfire_w28_test` took **18s** from
the admin UI; a fresh 25-collection DB took 10.2s to drop.

Landed (`src/storage/pending_drops.rs`):

- `delete_database` now deletes the `db:{name}` meta key and persists one
  `pending_drop:{cf}` marker per CF **in a single atomic WriteBatch**,
  removes the DB from cache, and returns. A background thread performs
  the expensive `drop_cf` calls (25ms apart so foreground CF ops don't
  starve behind the queue). Measured: drop 25-collection DB **0.002s**.
- Markers are resumed by `StorageEngine::initialize` on startup, so drops
  interrupted by a crash/restart complete eventually.
- Recreate races handled by claiming: `Database::create_collection` on a
  `Pending` CF atomically claims it, drops it synchronously, and creates
  it fresh; on a mid-drop (`Dropping`) CF it waits for the dropper. Doomed
  CFs are filtered from `list_collections` / `get_collection` /
  `delete_collection` / `is_columnar_collection`.

Still open (long term): reconsider CF-per-collection — total CF count
still inflates every OPTIONS rewrite, MANIFEST replay, and DB open.

## Update 2026-09-20 — the diagnosis was too narrow

Re-measured on the dev instance (963 CFs, 46 databases, 7.7 GB, OPTIONS
4.94 MB, MANIFEST 4.37 MB, 3717 SSTs). Three findings reorder this ticket.

### 1. Most of the pain was never about CF count

- **Startup: 22.5-33.0 s, of which RocksDB's own open is 1.81 s**
  (`DB SUMMARY` -> `DB pointer` in `data/LOG`). The other 21-31 s was
  SoliDB: `recalculate_all_counts` walking every `doc:` key of every
  collection, and `Collection::new` walking `blo:` for every collection
  whether or not it held a blob.
- **27 904 of 27 910 flushes had `flush_reason: "WAL Full"`**, 97.7% of
  them writing under 4 KB, in 39 bursts averaging ~715 CFs each. Cause:
  `max_total_wal_size = 50 MB` is a *flush trigger*, not a disk cap —
  crossing it makes `DBImpl::SwitchWAL` flush every CF holding data in the
  oldest WAL. It was not even holding its own line: the WAL sat at
  201.8 MB, because a WAL is only deletable once every CF that wrote to it
  has flushed. That is where 3717 SSTs came from, 87.8% under 64 KB.

Both are fixed and neither was a CF-model problem.

### 2. The dominant CF cost is create/delete churn, not database drops

`grep -c 'Auto-creating' solidb.log` -> **11 241 in eighteen days**, across
321 distinct names, one of them 358 times. Each is a `create_cf`. Deleting
a collection then recreating it under the same name cost `drop_cf` +
`create_cf` — two full OPTIONS rewrites to end up where it started.

Fixed: `delete_collection` now wipes the CF with a range tombstone (space
reclaimed at deletion time) and schedules the drop instead of performing
it; a same-name recreate claims the empty shell and reuses it. A single
reaper thread drops the ones nobody reclaims after
`SOLIDB_CF_REUSE_GRACE_SECS` (300 s — the observed recreate gap is
*minutes*, so a grace in seconds would catch almost nothing).

### 3. There is no RocksDB knob, and it is worse than "4.9 MB per op"

Read in the vendored RocksDB 10.10.1: `WriteOptionsFile` is called
unconditionally by both `DropColumnFamily` and
`WrapUpCreateColumnFamilies`, and ends with `VerifyRocksDBOptionsFromFile`
— which re-opens and re-parses all 963 sections. So each CF op costs
~9.5 MB of I/O *plus* a full parse. `avoid_flush_during_shutdown` is not
in the C API at all.

The one real API lever is `rocksdb_create_column_families` (plural: N
creates, **one** `WriteOptionsFile`). It exists in the C API but
rust-rocksdb 0.46 binds only the singular form — an upstream PR.

Worth filing alongside it: rust-rocksdb's `drop_cf` takes the CF-map write
lock in a `match` scrutinee, so the guard lives until the end of the
`match` — **the lock is held across the entire OPTIONS rewrite**. Since
`cf_handle()` takes a read lock on that same map and SoliDB calls it on
essentially every storage operation, each drop freezes reads and writes in
*every* database for ~180-400 ms. One-line fix (bind the guard to a local
before the `match`).

### Also landed

- **Collection registry** (`src/storage/collection_registry.rs`):
  `coll:{db}:{name}` in `_meta`. `list_collections` no longer calls
  `cf_names()`, which cloned every CF name in the instance *and* took the
  read lock that `create_cf`/`drop_cf` hold across their OPTIONS rewrite.
  The CF map stays the truth: startup adopts entry-less CFs, and every
  listing path falls back to the map with no `_meta`.
- **No more eager `_scripts` + `_slow_queries` per database** — 43 and 42
  CFs respectively on this instance, almost all empty, and two OPTIONS
  rewrites per database creation. Both are already created on first use.

### Still open, with a corrected estimate

**Lazy CF materialization was considered and deferred.** 98
`cf_handle(&self.name)` sites across 13 files, 47 of them
`expect("Column family should exist")`, and the failure mode is a write
path that reads instead of materialising. Its two largest justifications
are now gone: the 11 241 auto-creations materialise anyway (each inserts a
document), and the ~85 system CFs are removed above. What remains —
collections created explicitly and never written — does not pay for the
refactor.

**Shared CF with key prefixing** remains the only thing that addresses the
~84% of collections holding under 100 documents. Sketch, for whoever picks
it up: physical key `[4-byte BE collection id][existing logical key]`; one
shared CF **per database** named `{db}:_shared`, so `delete_database`'s
existing `cf_names().filter(starts_with("{db}:"))` sweep disposes of it
with no code change; promotion to a dedicated CF above a byte threshold;
never shared: credential collections, blob-typed, columnar. Key
construction is already centralised in `collection/core.rs` (13 builders),
and no `prefix_extractor` is configured, so lengthening the prefix breaks
no seek semantics. Phase 1 exit criterion: with every collection still
owning its CF the prefix is empty, so the emitted bytes must be
**byte-identical** — that is the only realistic way to review a ~250-site
diff. **Go/no-go before committing to it:** range tombstones become a
cross-tenant tax. Put 500 tiny collections in one shared CF, drop 250,
measure point-get and full-scan latency on a survivor before and after the
reclaim compaction. A durable regression beyond ~2x kills the approach.

## Context

Found while chasing `soli test` 10s timeouts. The lang test runner no
longer drops databases per run (it truncates), so this is latency hygiene
for interactive drops and `SOLI_TEST_FRESH_DB=1`, not a test-suite blocker.
