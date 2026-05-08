# SEC-164: Auto-update downloads tarballs without signature verification

## Status
- **Severity**: LOW
- **Category**: Supply Chain
- **Project**: soli/db
- **File**: `src/cli/update.rs`
- **Lines**: 60-83

## Description
Auto-update fetches a tar.gz from a GitHub release URL and unpacks into the executable directory. Integrity relies solely on TLS + GitHub. There is no signature or checksum verification.

## Recommendation
Publish minisign-signed releases (or at minimum a SHA-256 checksum file) and verify before `replace_binary`. Bake the public key into the binary.

## References
- Related: SEC-097.
