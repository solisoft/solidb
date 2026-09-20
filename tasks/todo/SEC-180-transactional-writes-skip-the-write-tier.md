# Transactional write paths resolve collections with the read getter

## Severity

high — eight write paths in `src/server/transaction_handlers.rs` resolve their
target with `get_collection` instead of `get_collection_for_write`, so
`check_write_access` never runs. A principal holding **Write** on any database
can write `_scripts` by name through them, which is how Lua gets installed for
the service router to execute. Same reach for `_services`, `_triggers`,
`_views`, `_graphs`, `_config`, `_rag_pipelines` and `_jobs`.

## Location

- `src/server/transaction_handlers.rs:130-139` — `collection_in_database` calls
  `Database::get_collection`; used by `insert_document_tx` (:162),
  `update_document_tx` (:193) and `delete_document_tx` (:223)
- `src/server/transaction_handlers.rs:419, :432, :464, :493` — the
  `BodyClause::Insert` / `Update` / `Remove` / `Upsert` arms of
  `execute_transactional_sdbql` call `StorageEngine::get_collection`
- `src/storage/database.rs` — `get_collection` vs `get_collection_for_write`

## Problem

`src/storage/protected.rs` defines three tiers, and the difference between the
two getters is which ones they enforce:

```rust
pub fn get_collection(&self, name: &str) -> DbResult<Collection> {
    if crate::storage::is_protected_collection(name) { ... }   // 5 credential collections
    self.system_collection(name)
}

pub fn get_collection_for_write(&self, name: &str, actor: WriteActor) -> DbResult<Collection> {
    crate::storage::check_write_access(name, actor)?;          // all three tiers
    self.system_collection(name)
}
```

`is_protected_collection` covers only `PROTECTED_COLLECTIONS` — `_env`,
`_admins`, `_api_keys`, `_roles`, `_user_roles`. It does **not** cover
`WRITE_PROTECTED_COLLECTIONS` (`_scripts`, `_services`, `_triggers`, `_views`,
`_graphs`, `_config`, `_rag_pipelines`) or `ADMIN_WRITE_COLLECTIONS`
(`_jobs`). Those are exactly the collections the server later executes or
schedules.

`grep -c check_write_access src/server/transaction_handlers.rs` returns **0**.

`execute_transactional_sdbql` does upgrade the required permission to `Write`
when the query mutates (`transaction_handlers.rs:252-264`), but that is the
permission *level*. The write tier is the check that says a Write principal
still may not name `_scripts` — it is what makes CLAUDE.md's claim true, that
"a public `insert into request.body.collection` script cannot be steered at
`_scripts`".

CLAUDE.md lists the paths the check covers: "the document API, SDBQL, the
driver, import, truncate, blob uploads, and the Lua bindings". The
transactional endpoints are not in that list, and are not covered.

Reachability, for `POST /_api/database/{db}/transaction/{tx}/document/_scripts`:
`collection_in_database` → `Database::get_collection("_scripts")` →
`is_protected_collection("_scripts")` is `false` (it is write-protected, not
credential-protected) → the collection is returned and written
transactionally.

Found while reviewing SEC-179, whose fix corrected the *database* the
collection resolves in but kept the read getter on the write paths. SEC-179's
own fix direction asked for "the CF prefix **and the credential guard**" — it
got both; the write tier was not part of that ticket's framing.

## Fix direction

1. `collection_in_database` takes a `WriteActor` and calls
   `get_collection_for_write`. The actor comes from the request's claims via
   the existing `write_actor_from_claims`, never `WriteActor::Server` — the
   collection name arrives over the wire.
2. The four mutation arms of `execute_transactional_sdbql` resolve through the
   same write getter rather than `StorageEngine::get_collection`. Note they
   build a qualified `{db}:{coll}` name; prefer resolving through
   `get_database(&db_name)?.get_collection_for_write(&clause.collection, actor)`
   so the tier check sees the bare name, as `check_write_access` expects.
3. Audit for the same shape elsewhere: `grep -rn 'get_collection(' src/server/`
   and check each hit that precedes a write.
4. Regression tests, mirroring the SEC-179 ones in
   `tests/db_authorization_tests.rs`: a key with Write on its own database must
   get 403 for `_scripts` through both `/transaction/{tx}/document/...` and
   `/transaction/{tx}/query` with an `INSERT INTO _scripts`, and `_jobs` must
   require Admin on both.

## Related

- [SEC-179](../done/SEC-179-transactional-document-ops-ignore-database.md) —
  same file, the database-resolution half. Fixed and closed.
- [SEC-176](../done/SEC-176-env-secrets-readable-at-read-permission.md) —
  established the credential tier that these paths *do* enforce.
