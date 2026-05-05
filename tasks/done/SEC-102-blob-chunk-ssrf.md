# SEC-102: Blob Chunk Fetch SSRF Risk in Cluster

## Status
- **Severity**: MEDIUM
- **Category**: SSRF
- **Project**: soli/db
- **File**: `src/server/blob_handlers.rs`
- **Lines**: 442-521

## Description
The `fetch_blob_chunk_from_cluster` function iterates over `node_addresses` from shard coordinator and makes HTTP requests without validation.

## Exploit Scenario
If attacker can manipulate cluster node addresses (via compromised coordinator), they could trigger HTTP requests to internal services.

## Recommendation
Add URL validation and restrict to known cluster node addresses.

## References
- Related: SEC-077 (ssrf solidb fetch), SEC-015 (dns rebound ssrf)