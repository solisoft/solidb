# Transactional document endpoints ignore the database in the path

## Severity

high — the `{db}` path segment is discarded, so a transactional document
operation resolves the collection by bare name. Bare names fall back to
`_system:{name}` in the storage engine, so a principal scoped to one database
writes into `_system` collections. Also a plain correctness bug: the same
endpoints cannot address a normal collection at all.

## Location

- `src/server/transaction_handlers.rs:119` — `insert_document_tx`:
  `Path((_db_name, tx_id_str, coll_name))` discards the database, then
  `state.storage.get_collection(&coll_name)`
- `src/server/transaction_handlers.rs:170`, `:200` — `update_document_tx` /
  `delete_document_tx`, same shape
- `src/storage/engine.rs:665` — `get_collection` falls back to
  `_system:{name}` when the bare name has no column family

## Problem

Collections are column families named `{database}:{collection}`. Every other
handler resolves through `Database`, which applies that prefix. The
transactional document handlers bind the database segment to `_db_name` and
never use it, passing the bare collection name to the storage engine. The
engine tries the literal name, then `_system:{name}`.

So `POST /_api/database/tenantX/transaction/{tx}/document/{coll}`:

- writes to `_system:{coll}` when that column family exists — a cross-database
  write that the per-database authorization never sees, because authorization
  ran against `tenantX` from the path;
- returns `CollectionNotFound` otherwise, which is every ordinary collection
  (`tenantX:items` is never tried).

Verified against a v0.33.0 build (2026-07-31), before the SEC-176 guard landed.
An API key with role `editor` and `scoped_databases: ["tenantX"]` — which gets
`403` on any direct `_system` request — overwrote the instance-wide LLM
credential:

```
PUT /_api/database/tenantX/transaction/{tx}/document/_env/OPENAI_API_KEY
    {"value":"sk-HIJACKED"}
 -> 200 {"_id":"_system:_env/OPENAI_API_KEY", "value":"sk-HIJACKED"}

# admin then reads the global credential:
GET /_api/database/_system/env  ->  {"OPENAI_API_KEY":"sk-HIJACKED"}
```

[SEC-176](../done/SEC-176-env-secrets-readable-at-read-permission.md) closed
the credential-collection case specifically (that request is now `403`), but
the underlying defect is untouched: any collection name that exists under
`_system:` is still reachable and writable from any database, and ordinary
collections still 404.

## Fix direction

1. Use the database from the path: resolve through
   `state.storage.get_database(&db_name)?.get_collection(&coll_name)?` in all
   three handlers, so the CF prefix and the credential guard both apply.
2. Check that the transaction being addressed belongs to that database —
   `tx_id` is a bare integer parsed from the path, so a caller can name any
   in-flight transaction id. Worth confirming whether a transaction started by
   one principal can be driven by another (likely a separate ticket).
3. Reconsider the `_system:{name}` fallback in
   `StorageEngine::get_collection` (`src/storage/engine.rs:665`). It exists for
   backward compatibility with unprefixed names, but it turns "collection not
   found in your database" into "silently operate on a `_system` collection".
   Any caller-facing resolution should require an explicit database.
4. Regression test: a scoped key must not reach `_system` through
   `/transaction/{tx}/document/...`, and a transactional insert into an
   ordinary collection in the named database must succeed (it does not today).
