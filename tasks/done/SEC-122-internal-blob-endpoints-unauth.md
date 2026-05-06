# SEC-122: `/_internal/blob/*` endpoints accept unauthenticated requests

## Status
- **Severity**: CRITICAL
- **Category**: Authentication Bypass
- **Project**: soli/db
- **File**: `src/server/routes.rs`, `src/sync/blob_replication.rs`
- **Lines**: routes.rs:1031-1046; blob_replication.rs:51, 105

## Description
Three "internal" routes are mounted in the public router with no auth middleware, no cluster-secret check, and no origin validation:
- `POST /_internal/blob/replicate/{db}/{collection}/{key}`
- `POST /_internal/blob/upload/{db}/{collection}` (auto-creates blob collections)
- `GET /_internal/blob/replicate/{db}/{collection}/{key}/chunk/{idx}`

The handlers accept attacker-controlled JSON metadata (the `_key` from the body is trusted, not the URL path) and write blob chunks to any database/collection — up to the 500 MB body limit per request.

## Exploit Scenario
A remote attacker reads or overwrites blobs in any collection, exfiltrates chunks via `get_blob_chunk`, or fills disk with large multipart uploads. Combined with the auto-create behavior, the attacker can also create new collections with arbitrary names.

## Recommendation
Gate all `/_internal/*` routes behind a cluster-secret middleware that requires a valid `X-Cluster-Secret` matching the keyfile, and refuses when the keyfile is empty (see SEC-123). Validate that the metadata `_key` matches the URL path. Verify per-chunk integrity (e.g., chunk hash supplied by the coordinator).

## References
- Related: SEC-081, SEC-102, SEC-123.
