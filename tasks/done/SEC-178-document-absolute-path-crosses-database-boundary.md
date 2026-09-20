# `DOCUMENT("db:collection/key")` reads any database, bypassing all authorization

## Severity

critical — complete tenant isolation bypass. A **read-only**, database-scoped
API key reads documents from every other database on the instance, including
`_system:_admins` (argon2 password hashes) and other tenants' `_env`
credentials. No admin rights, no write rights, no cluster access needed.

## Location

- `src/sdbql/executor/evaluate.rs:330-340` — `DOCUMENT()` single-id branch:
  a name containing `:` is routed to `self.storage.get_collection(...)`,
  explicitly "bypass context"
- `src/sdbql/executor/evaluate.rs:356-362` — same for the id-array branch
- `src/sdbql/executor/evaluate.rs:386-390` — same for the 2-arg
  `DOCUMENT("collection", "key")` form
- `src/storage/engine.rs:665` — `StorageEngine::get_collection` resolves a
  column family by its literal `db:collection` name, and falls back to
  `_system:{name}`; it is a physical accessor with no notion of a caller

## Problem

Collections are RocksDB column families named `"{db}:{collection}"`.
`QueryExecutor::get_collection` normally resolves through the executor's
database context, which keeps a query inside the database named in the URL —
and therefore inside what `db_authz_middleware` checked. The `DOCUMENT()`
builtin has a deliberate escape hatch: if the argument contains `:` it is
treated as an absolute path and handed straight to the storage engine, which
happily opens any CF by name.

Nothing re-checks the caller's permissions against that second database. The
per-database authorization is enforced once, on the `{db}` path parameter, and
`DOCUMENT()` never touches that parameter.

Verified against a v0.33.0 release build (2026-07-30).

Setup: databases `victim` and `attacker`; an API key with role `editor` and
`scoped_databases: ["attacker"]`.

```
# control — the scoping works for direct access
GET /_api/database/victim/document/secrets/k1
  -> 403 {"error":"Access denied: API key not authorized for database 'victim'"}

# same key, same target, via a query against its OWN database
POST /_api/database/attacker/cursor
  {"query":"RETURN DOCUMENT(\"victim:secrets/k1\")"}
  -> 200 {"card":"4111-1111-1111-1111","note":"VICTIM-TENANT-DATA", ...}

POST /_api/database/attacker/cursor
  {"query":"RETURN DOCUMENT(\"victim:_env/OPENAI_API_KEY\")"}
  -> 200 {"value":"sk-VICTIM-KEY", ...}
```

And with a key holding only the read-only `viewer` role, still scoped to
`attacker`:

```
POST /_api/database/attacker/cursor
  {"query":"RETURN DOCUMENT(\"_system:_admins/admin\")"}
  -> 200 {"_key":"admin",
          "password_hash":"$argon2id$v=19$m=19456,t=2,p=1$w9KU...$fECA..."}
```

`_system:_api_keys/{id}` is readable the same way (the probe above returned
`null` only because the guessed key id did not exist).

The `FOR` path is not affected — `FOR d IN \`victim:secrets\`` returns 404,
because it resolves through the database context. `DOCUMENT()` is the only
builtin found taking this shortcut, but the underlying primitive
(`StorageEngine::get_collection` accepting a qualified name from query input)
is what makes it reachable.

## Fix direction

1. Stop `DOCUMENT()` from crossing databases. The absolute form should resolve
   *only* to the executor's own database context: if the prefix before `:`
   is not the executor's `database`, return `CollectionNotFound` (not a
   distinguishable error — a different error would confirm existence of other
   databases). The "bypass context" branch exists for cross-database reads that
   the authorization model does not support at all today; it should be removed
   rather than permission-checked, because the executor has no `Claims` to
   check against.
2. Give the executor a `database` invariant it can enforce centrally: route the
   absolute-path branches through `QueryExecutor::get_collection` and have that
   function reject any name containing `:` whose prefix != `self.database`.
3. Audit the other qualified-name entry points for the same shortcut —
   `evaluate.rs` also uses `self.storage.get_collection` at the array and
   2-arg branches, and `src/sdbql/executor/materialized_views.rs` builds
   `db:collection` names internally (those are not user-controlled, but should
   be kept distinct from the user-facing path).
4. Add a regression test: a scoped key querying its own database must not be
   able to `DOCUMENT()` another database, `_system` included.

## Related

- [SEC-176](SEC-176-env-secrets-readable-at-read-permission.md) — `_env`
  readable at `Read`. This ticket is the cross-database version and is
  strictly worse: it also exposes `_system:_admins` password hashes.
- The `_system` fallback in `StorageEngine::get_collection` means an
  unqualified name can also land in `_system` when the CF is absent from the
  current database; worth checking whether that is reachable from query input.
