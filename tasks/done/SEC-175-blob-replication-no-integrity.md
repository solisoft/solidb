# SEC-175: Blob replication has no per-chunk integrity check

## Status
- **Severity**: LOW (escalates to MEDIUM once SEC-122 is fixed)
- **Category**: Data Integrity
- **Project**: soli/db
- **File**: `src/sync/blob_replication.rs`
- **Lines**: 51-105 (receive_blob_replication), 226-265 (receive_blob_upload)

## Description
Even after SEC-122 closes the unauthenticated-endpoint hole, the receiving side has no per-chunk hash, no signature on metadata, and trusts the `_key` field from the body. A compromised peer (or any actor with the cluster secret) can overwrite arbitrary blob keys with corrupted data.

## Recommendation
- The coordinator includes a SHA-256 of each chunk in the metadata; receivers verify before `put_blob_chunk`.
- Optionally sign metadata with the cluster keyfile and require signature verification on the receiver.

## References
- Related: SEC-102, SEC-122, SEC-135.
