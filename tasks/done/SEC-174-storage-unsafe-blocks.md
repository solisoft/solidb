# SEC-174: Additional `unsafe` blocks in storage layer rely on external locking

## Status
- **Severity**: LOW (INFO-level — depends on lock discipline)
- **Category**: Memory Safety
- **Project**: soli/db
- **File**: `src/storage/database.rs`, `src/storage/engine.rs`, `src/storage/columnar.rs`
- **Lines**: database.rs:67, 101; engine.rs:410, 473; columnar.rs:1036

## Description
Five `unsafe { (*db_ptr).create_cf / drop_cf }` blocks cast `Arc<DB>` to `*mut DB` to call mutating column-family APIs. Soundness depends entirely on the engine-level `cf_lock` `RwLock` being held. `engine.rs` does hold it; `database.rs:67` only checks `cf_handle` and does **not** acquire `cf_lock` — racing `create_collection` against `delete_collection` from a different `Database` handle that shares the same `Arc<DB>` is UB per the rust-rocksdb contract.

## Recommendation
- Require `cf_lock` for both code paths (move the lock acquisition into a shared helper).
- Or upgrade to a `rust-rocksdb` version that exposes safe `&self` CF mutation.
- Audit all 5 sites and document the locking invariant inline.

## References
- Related: SEC-107 (acknowledged earlier `unsafe` use).
