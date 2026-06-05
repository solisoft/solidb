# `DELETE /_api/database/{db}` is O(collections × OPTIONS-file-size)

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

## Context

Found while chasing `soli test` 10s timeouts. The lang test runner no
longer drops databases per run (it truncates), so this is latency hygiene
for interactive drops and `SOLI_TEST_FRESH_DB=1`, not a test-suite blocker.
