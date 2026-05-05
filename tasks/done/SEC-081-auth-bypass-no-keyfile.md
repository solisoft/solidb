# SEC-081: Authentication Bypass When Keyfile is Empty/Missing

## Status
- **Severity**: CRITICAL
- **Category**: Authentication
- **Project**: soli/db
- **File**: `src/sync/transport.rs`
- **Lines**: 462-480

## Description
When no keyfile is configured or the keyfile is empty, the system skips authentication entirely.

## Exploit Scenario
Any attacker can join the cluster and receive full replication stream without any credentials.

## Recommendation
Require keyfile and fail startup if not present in production.

## References
- Related: SEC-017 (app env test disables ssrf)