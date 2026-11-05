# EXPLAIN ignores `indexHint`

## Severity

low — diagnostics. Queries run correctly; EXPLAIN describes a different read
than the one the query makes.

## Location

- `src/sdbql/executor/explain.rs` — the loop access (around the
  `lookup_index_for_filter` call in `explain_in`) and the plan-only path
  (`plan_index_read` in `estimate_loops`) never look at the FOR's
  `OPTIONS { indexHint, forceIndexHint }`.
- `src/sdbql/executor/index_opt.rs` — `lookup_index_for_filter_hinted` /
  `lookup_with_hint` are what execution uses (`clauses.rs`).

## Problem

`FOR d IN rows OPTIONS { indexHint: "idx_status" } FILTER d.status == "open"
AND d.amount >= 199990 RETURN d` reads `idx_status` (190k rows), but EXPLAIN
reports `idx_amount`, the planner's own choice. A `forceIndexHint` that would
make the query fail is not reported either. Found while benchmarking
`planner-choose-between-candidate-plans`, where the hint is used to force
the previous order.

## Proposal

Read the hint in both EXPLAIN paths and go through
`lookup_index_for_filter_hinted` (ran) and a planned equivalent (plan-only),
so `index_used` names the hinted index when it serves, and a forced hint
that cannot serve is reported as the error the run would raise. Add an
EXPLAIN test next to the hint tests in `tests/test_index_optimization.rs`.
