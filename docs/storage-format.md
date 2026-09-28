# Storage format (2.x)

How SoliDB 2.x lays data out in RocksDB. For the code, start at
`src/storage/keyspace.rs` and `src/storage/collection_registry.rs`.

## Column families

| Name | Holds |
|---|---|
| `default` | Nothing (RocksDB requires it). |
| `_meta` | The catalog: databases, collections, id counters, markers. |
| `__keyspaces__` | Every collection's data, each under its own 8-byte prefix. |
| `db:coll` (legacy) | A 1.x collection not migrated yet (or whose migration failed). Served as before; retried at every start. |

A 2.x instance that has finished migrating has exactly the first three, so
creating or dropping collections and databases never changes the column-family
set — and never rewrites the OPTIONS file, which in 1.x cost time proportional
to the total number of collections on every create and drop.

## Keyspaces

A collection's keys live under an 8-byte prefix:

```
┌──────────────────┬──────────────────┬─────────────────────────────┐
│ db_id  u32 BE    │ coll_id  u32 BE  │ logical key (doc:…, idx:…)  │
└──────────────────┴──────────────────┴─────────────────────────────┘
```

- `db_id` is allocated from `_meta ks:next_db_id` on a database's first
  collection and stored in `_meta dbid:{name}`. `0` is reserved for
  engine-level collections created without a database (`bare`), `0xFFFFFFFF`
  is never allocated (so `prefix + 1` never overflows).
- `coll_id` comes from the per-database counter `_meta ks:next_coll:{db_id}`.
- Ids are never reused. A dropped and recreated collection gets a new prefix:
  it cannot see its predecessor's keys, and caches keyed by keyspace miss.
- A collection is the range `[prefix, prefix + 1)`; a database is
  `[db_id ‖ 0, (db_id + 1) ‖ 0)`. Dropping either is one `DeleteRange`.

The logical keys inside a keyspace are the same as in 1.x (`doc:`, `docv:`,
`idx:`, `idx_meta:`, `geo:`, `ft_term:`, `blo:`, `ttl_exp:`, `vec_meta:`,
`vec_data:`, `_stats:count`, `_stats:type`, `col:` for columnar …).

## `_meta` keys

| Key | Value |
|---|---|
| `db:{name}` | `"1"` — the database exists (unchanged from 1.x). |
| `dbid:{name}` | Its `db_id`, decimal. |
| `ks:next_db_id` | Next database id. |
| `ks:next_coll:{db_id}` | Next collection id in that database. |
| `coll:{db}:{coll}` | JSON `{type_, created_ms, ks?, migrating_to?}`. `ks` present = shared layout; absent = legacy (every 1.x record reads this way). `migrating_to` = a migration copy in progress. |
| `dead_ks:{hex lo}` | Upper bound of a dropped range awaiting compaction (`keyspace_gc`). |
| `pending_drop:{cf}` | A legacy column family awaiting its background drop. |
| `storage_format` | `"2"` once every movable collection is in the shared keyspace. |
| `migration:v2:report` | JSON report of the last migration run. |
| `shutdown:clean` | Clean-shutdown marker (skip the document recount). |

## Migration from 1.x

Automatic at the first 2.x start, inside `StorageEngine::initialize`, before
the server binds its port (`--migrate-only` runs it and exits). Collections move
one at a time, smallest first:

1. `migrating_to = ks` is written to the collection's record (synced).
2. Every key is copied into `ks`, counting keys and hashing key + value.
3. The copy is re-read and compared.
4. The record flips to `ks` (synced) and the old column family is scheduled
   for a background drop; its files are freed at once
   (`DeleteFilesInRange` + range delete), so the extra disk in use at any moment
   is about one collection.

A crash during 2–3 leaves `migrating_to`: the next start wipes the target and
redoes the copy. A crash after 4 re-schedules the old column family. A
collection that fails verification or hits an I/O error stays legacy and is
served from its own column family; `storage_format` is only set to `2` once a
run moves everything it can. A `db:coll` column family whose database record
is gone (the remains of an interrupted 1.x drop) is left alone and listed in the
report.

Before migrating, free disk space must be at least 1.2 × the largest collection
+ 1 GiB (`SOLIDB_MIGRATION_SKIP_SPACE_CHECK=1` overrides).

**The change is one-way**: 1.x cannot open a data directory once collections
have moved. Take a checkpoint first (`POST /_api/backup`); rolling back means
restoring it.

## Stats

`disk_usage` of a shared collection is RocksDB's approximate size of its key
range — flushed data only; the memtable and the SST file count cannot be
attributed to one keyspace and read 0.
