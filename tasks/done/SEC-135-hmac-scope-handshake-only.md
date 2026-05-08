# SEC-135: HMAC authentication covers only the handshake

## Status
- **Severity**: HIGH
- **Category**: Cryptographic / Network
- **Project**: soli/db
- **File**: `src/sync/transport.rs`, `src/sync/worker.rs`
- **Lines**: transport.rs:438-599; worker.rs (post-handshake message handling)

## Description
`authenticate_standalone` validates an HMAC at handshake time. After auth succeeds, every subsequent `SyncMessage` (`SyncBatch`, `IncrementalSyncRequest`, `FullSync*`, `Heartbeat`) flows over **plain TCP with zero per-message authentication**. There is no AEAD wrap, no per-message HMAC, no sequence-number binding.

## Exploit Scenario
A man-in-the-middle on the post-handshake stream (or any attacker with TCP access in the absence of TLS — see SEC-080) can inject arbitrary `SyncBatch`/`FullSync` payloads, poisoning the replication log. Pre-recorded sessions can also be replayed.

## Recommendation
- Wrap the post-handshake stream in an AEAD channel (e.g. Noise XK) keyed from the HMAC handshake.
- Or apply per-message HMAC over `(seq_counter || msg_bytes)` with a session-level monotonic counter.

## References
- Related: SEC-080, SEC-083, SEC-088.
