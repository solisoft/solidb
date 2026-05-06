# SEC-154: Sync transport mixes framing styles and silently drops on parse error

## Status
- **Severity**: MEDIUM
- **Category**: Network Protocol Robustness
- **Project**: soli/db
- **File**: `src/sync/worker.rs`
- **Lines**: 1148-1389 (inbound connection handler), 1186 (`lz4_flex::decompress_size_prepended(...).unwrap_or_default()`)

## Description
The inbound handler reframes messages itself with mixed framing rules: `IncrementalSyncRequest` uses `[compressed_byte][len: u32][bytes]`, while `FullSync*` responses use only `[len: u32][bytes]`. A peer that interleaves the two desynchronizes the parser. Parse errors and oversize messages cause a silent `break`, and `lz4` decompression failures fall back to an empty buffer with no log.

## Exploit Scenario
A malicious or buggy peer sends frames that toggle desync, forcing other nodes to reconnect repeatedly. Each reconnect triggers a full-sync, amplifying load.

## Recommendation
- Unify all framing through `ConnectionPool::write_message` / `read_message`.
- On parse / decompression failure, log the peer ID, increment a per-peer error counter, and apply exponential ban-listing.

## References
- Related: SEC-110, SEC-135.
