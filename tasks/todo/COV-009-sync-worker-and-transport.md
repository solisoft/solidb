# COV-009: Cover `sync/worker.rs` + `sync/transport.rs` (0% → ≥40%)

## Status
- **Category**: Test Coverage
- **Project**: soli/db
- **Files**:
  - `src/sync/worker.rs` (905 lines uncovered)
  - `src/sync/transport.rs` (389 lines uncovered)
- **Current coverage**: 0% / 0%

## Description
The replication worker (master-master eventual-consistency loop) and its HTTP transport are entirely uncovered. Failures here cause silent data loss between nodes.

## Recommendation
Layered approach:

1. **Pure-logic unit tests on `worker.rs`** — extract decision functions and test them directly:
   - Conflict-resolution policy (last-writer-wins via HLC, etc.).
   - Backoff / retry timing.
   - Per-peer queue draining order.
   - "Caught-up" predicate.

2. **In-process two-engine integration test** — `tests/replication_worker_tests.rs`:
   - Two `StorageEngine` instances, each with cluster keyfile configured.
   - Inject a stubbed transport (trait swap) so no real sockets are needed.
   - Insert on node A → run the worker once → assert the doc lands on node B with correct HLC.
   - Insert concurrently on both → assert convergence after both workers run.
   - Peer offline → entries queue → peer back → drain.

3. **`transport.rs` HTTP path** — exercise the HTTP send/receive helpers against a `wiremock` server (or a minimal axum app) verifying:
   - `X-Cluster-Secret` header is set.
   - Retry on 5xx, give up on 4xx.
   - Decompression failure path is handled (regression for SEC-168).

## Goal
Raise both files to ≥40% line coverage.

## References
- Related: SEC-122, SEC-154, SEC-168
- Pattern: `tests/sync_protocol_tests.rs`
