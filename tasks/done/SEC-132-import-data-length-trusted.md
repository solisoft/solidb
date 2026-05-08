# SEC-132: Import/restore trust attacker-supplied `_data_length`

## Status
- **Severity**: HIGH
- **Category**: Denial of Service / Memory Exhaustion
- **Project**: soli/db
- **File**: `src/server/handlers/import_export.rs`, `src/bin/solidb-restore.rs`
- **Lines**: import_export.rs:304-328; solidb-restore.rs:301

## Description
The streaming import path reads a `_data_length: u64` field from the request body and casts it to `usize` to size buffers / drive a read loop. There is no upper bound. Within axum's 500 MB body limit, an attacker can declare `"_data_length": 9999999999999` and stall the connection while the server pins memory. The CLI restore path has the same pattern when reading malicious dump files.

## Exploit Scenario
1. Authenticated user calls `POST /_api/import` with a JSONL header `{"_data_length": 9999999999999}`.
2. Server `Vec`s grow to that size or the read loop hangs awaiting bytes that never arrive.
3. Concurrent uploads multiply impact.

## Recommendation
- Cap `_data_length` per chunk to a sane maximum (e.g. 16 MiB).
- Add a per-`stream.next()` timeout.
- Same fix in `solidb-restore` to harden against malicious dump files.

## References
- Related: SEC-094.
