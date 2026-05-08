# SEC-123: Empty cluster secret bypasses cluster admin endpoints

## Status
- **Severity**: CRITICAL
- **Category**: Authentication Bypass
- **Project**: soli/db
- **File**: `src/server/handlers/cluster.rs`, `src/server/cluster_handlers.rs`, `src/server/handlers/sync.rs`, `src/server/auth.rs`
- **Lines**: handlers/cluster.rs:471, 524, 692, 773; cluster_handlers.rs:574, 630; auth.rs:819

## Description
Multiple cluster admin handlers use the pattern:
```rust
if !secret.is_empty() && !constant_time_eq(request_secret, secret) {
    return Err(...);
}
```
When `cluster_secret()` returns the empty string (no keyfile loaded), the bad-secret branch is **skipped entirely** — any caller is accepted. Affected handlers include `cluster_cleanup`, `cluster_reshard`, `cluster_blob_rebalance`, several `sync` admin routes, and the cluster-internal X-Cluster-Secret check in `auth_middleware`.

Distinct from SEC-081 which fixed the inter-node sync transport: these are the HTTP control-plane endpoints.

## Exploit Scenario
```http
POST /_api/cluster/reshard
X-Cluster-Secret: anything
{...}
```
On a node started without a keyfile, this triggers reshard migrations (deleting documents from the source shard) without any auth.

## Recommendation
Fail closed: when `secret.is_empty()`, reject the request with 503 ("cluster keyfile not configured"). Tie this to a `SOLIDB_REQUIRE_KEYFILE` mode that mirrors SEC-081.

## References
- Related: SEC-081, SEC-085, SEC-100.
