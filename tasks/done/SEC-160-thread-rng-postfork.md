# SEC-160: Cluster auth uses `thread_rng` rather than `OsRng`

## Status
- **Severity**: MEDIUM
- **Category**: Cryptographic
- **Project**: soli/db
- **File**: `src/sync/transport.rs`
- **Lines**: 531, 536 (challenge + nonce generation)

## Description
`rand::thread_rng()` is ChaCha-based and currently cryptographically adequate, but it is seeded once and shares state across calls. If a daemon ever forks (see `daemon.rs`), child processes can share PRNG state with the parent, leading to nonce reuse. `OsRng` avoids this by going to the OS RNG every call.

## Recommendation
Switch the 32-byte challenge and 16-byte nonce generation to `rand::rngs::OsRng`, matching the pattern already used in `auth.rs`.

## References
- Related: SEC-083, SEC-088.
