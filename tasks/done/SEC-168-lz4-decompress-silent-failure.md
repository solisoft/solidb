# SEC-168: lz4 decompression failures silently produce empty buffers

## Status
- **Severity**: LOW
- **Category**: Network Protocol Robustness
- **Project**: soli/db
- **File**: `src/sync/worker.rs`
- **Lines**: 1186 (`lz4_flex::decompress_size_prepended(&data).unwrap_or_default()`)

## Description
Decompression failures fall back to an empty `Vec`, which then gets fed to `bincode::deserialize`. Malformed compressed frames become indistinguishable from empty messages, hiding attacks in logs.

## Recommendation
Treat decompression error as fatal for the connection. Log the peer ID, error kind, and bytes. Penalize repeat offenders via a per-peer error budget.

## References
- Related: SEC-154.
