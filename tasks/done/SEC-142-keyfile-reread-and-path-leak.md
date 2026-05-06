# SEC-142: Keyfile re-read on every HMAC and path leaked in error

## Status
- **Severity**: HIGH
- **Category**: Cryptographic / Information Disclosure
- **Project**: soli/db
- **File**: `src/sync/transport.rs`
- **Lines**: 243, 601-616 (`compute_hmac_with_timestamp`)

## Description
Two issues in one helper:
1. The keyfile is re-read from disk on every HMAC computation. This creates a TOCTOU window where an attacker who can swap the keyfile mid-handshake influences which key the server uses.
2. The error string `"Failed to read keyfile {path}: {err}"` is included in the wire response, leaking the absolute path of the keyfile to the peer.

## Exploit Scenario
1. Operator rotates the keyfile via in-place write.
2. A handshake in flight reads partial contents and accepts a forged HMAC.
3. A failed handshake reveals `/etc/solidb/cluster.key` to the connecting peer.

## Recommendation
- Load the keyfile once at startup into `Arc<zeroize::Zeroizing<Vec<u8>>>`.
- Return a generic `"keyfile read failed"` to peers; log the detailed message locally only.
- Verify file mode is `0600` and refuse to start otherwise.

## References
- Related: SEC-081, SEC-088.
