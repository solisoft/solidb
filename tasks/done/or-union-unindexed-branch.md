# An `OR` with a branch no index serves is a full scan, and reads first

## Severity

low — performance and diagnostics.

## Location

`src/sdbql/executor/index_opt.rs` — `lookup_or_union`, step 4 of
`lookup_index_for_filter_limited`.

## Problem

Since 2.6.4 an `OR` whose every branch an index serves is read as the union
of those reads. A branch with no index makes the whole union impossible, and
the scan is then unavoidable. Three things can still be improved:

1. **Wasted reads.** The branches are looked up in order, each one reading
   its documents. When a later branch has no index, the reads already done
   are thrown away and the scan runs anyway.
2. **No diagnostic.** EXPLAIN shows `full_scan` without saying which branch
   lacked an index, so the fix (one more index) is not obvious.
3. **Choice of plan.** The union is tried last: an AND whose other conjunct
   has an index uses that index, even when the union would read far fewer
   rows (e.g. `d.owner_id IN @team AND (d.opportunity_id IN @deals OR …)`).

## Proposal

- Split index planning from reading: a pass that decides, without reading,
  which index (if any) serves each branch; read only when all of them can.
  This is the same refactor `explain-subquery-needs-a-row` needs.
- When a branch is not servable, add an EXPLAIN warning naming it and the
  field that would need an index.
- With the planning pass in place, compare candidate plans by a cheap
  estimate (index entry counts under the prefix, or a sampled count) instead
  of the fixed order composite → single field → composite prefix → geo →
  union.

## Context

CRM index audit, 2026-11-04. The CRM's document visibility filter
(`opportunity_id IN @deals OR (opportunity_id IN [null, ""] AND account_id
IN @accounts)`) is served by union once the document collections have an
index on `[opportunity_id, account_id]`; without it, it scans.
