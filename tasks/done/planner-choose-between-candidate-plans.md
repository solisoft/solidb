# The planner takes the first plan that applies, not the cheapest

## Severity

low — performance. Results are right; some queries read far more rows than
another available plan would.

## Location

`src/sdbql/executor/index_opt.rs` — `lookup_index_for_filter_limited`
tries a fixed order: composite (two fields or more) → single-field
equalities and `IN` → composite on its first field → single-field prefixes
and ranges → geo → `OR` union. `plan_index_read` mirrors that order.

## Problem

The first step that can serve the FILTER wins, whatever it reads:

- `d.owner_id IN @team AND (d.opportunity_id IN @deals OR …)` reads every
  row of the team through `owner_id` even when the union of the `OR`
  branches would read a handful;
- an equality on a low-cardinality field (`status == "open"`) is preferred
  to a selective range on another indexed field;
- a single-field equality is preferred to a composite prefix read that also
  fixes a second field through an exclusion it cannot count as a field.

## Proposal

Now that planning is separate from reading (`plan_index_read`, added for
`or-union-unindexed-branch`), let each step return its candidate with an
estimated row count instead of stopping at the first, then read the
cheapest:

- an equality / `IN` / composite prefix: count index keys under each
  prefix, stopping at a cap (say 2× the best estimate so far) so the
  estimate costs at most what the best plan will read;
- a range: the same, bounded by the interval;
- a union: the sum of its branches;
- ties and failures keep today's order, so plans only change when one is
  measurably cheaper.

Report the estimate in EXPLAIN (`estimated_rows`) so the choice can be
checked. Benchmark against the current order on the CRM shapes above.

## Context

Split from `or-union-unindexed-branch` (2026-11-04), whose third point it
is; the first two (no wasted reads, an EXPLAIN warning naming the branch)
were done there.
