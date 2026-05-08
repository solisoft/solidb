# COV-000: Coverage baseline & roadmap

## Status
- **Category**: Test Coverage
- **Project**: soli/db
- **Scope**: workspace

## Baseline (2026-05-08)
Generated via `cargo llvm-cov --release --no-fail-fast --summary-only --workspace`.

- **Lines**: 42.89% (39,153 of 68,557 missed)
- **Functions**: 40.47%
- **Regions**: 43.75%

### Top-level area breakdown (lines)
| % | Area | Lines | Missed |
|---:|---|---:|---:|
| 0% | `bin/` (solidb-dump/restore/repl) | 1,328 | 1,328 |
| 0% | `cli/tui/` | 2,231 | 2,231 |
| 0% | `driver/handlers/` | 2,280 | 2,280 |
| 11% | `cli/scripts/` | 2,018 | 1,797 |
| 22% | `server/handlers/` | 6,361 | 4,946 |
| 40% | `scripting/` | 3,530 | 2,095 |
| 42% | `sharding/` | 5,490 | 3,195 |
| 50% | `sync/` | 4,320 | 2,175 |
| 51% | `sdbql/executor/` | 8,885 | 4,378 |
| 60% | `storage/` | 4,416 | 1,740 |
| 76% | `sdbql/parser/` | 1,532 | 367 |
| 90% | `sdbql/` (top-level) | 903 | 94 |

## Active coverage tasks (todo/)
- COV-001 — `server/role_handlers.rs` (641 lines @ 0%)
- COV-002 — `server/handlers/blobs.rs` (291 @ 0%)
- COV-003 — `server/handlers/sync.rs` (452 @ 0%)
- COV-004 — `server/handlers/websocket.rs` (632 @ 0%)
- COV-005 — `sdbql/executor/search.rs` (476 @ 0%)
- COV-006 — `server/columnar_handlers.rs` (284 @ 0%)
- COV-007 — `server/nl_handlers.rs` (298 @ 0%)
- COV-008 — `transaction/distributed.rs` (283 @ 0%)
- COV-009 — `sync/worker.rs` + `sync/transport.rs` (905 + 389 @ 0%)
- COV-010 — `server/llm_client.rs` (353 @ 0%)

## Out-of-scope here (separate tracks)
- `bin/`, `cli/tui/`, `cli/scripts/`: CLI entry points. Cover via end-to-end shell-level tests rather than unit tests.
- `driver/handlers/`: binary protocol handlers. Cover via `clients/` SDK round-trip tests.

## How to re-run the report
```bash
cargo llvm-cov --release --no-fail-fast --summary-only --workspace
# HTML report:
cargo llvm-cov --release --no-fail-fast --workspace --html
# open target/llvm-cov/html/index.html
```

## Target
After all COV-001..010 are merged, expect total line coverage ≥55% (rough estimate: ~3,500 newly-covered lines across these files).
