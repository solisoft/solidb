# EXPLAIN always runs the query it explains

## Severity

low — diagnostics and load.

## Location

`src/sdbql/executor/explain.rs` — `explain_in` executes every clause (index
reads, scans, FILTERs, LETs, SORT, LIMIT) to time each step.

## Problem

EXPLAIN is really EXPLAIN ANALYZE: explaining a slow query costs as much as
running it, on the server it is meant to protect, and the plan it shows can
depend on the data (an index-ordered read that falls back when short, a
subquery planned against the first row).

## Proposal

`POST /_api/database/:db/explain` with `{"analyze": false}`: build the plan
without reading documents, using `plan_index_read` for each loop (as
`estimate_subquery` already does for subqueries no row reaches) and the
static SORT / count checks, every access marked `estimated: true`, timings
and document counts omitted. `analyze` stays the default so existing
callers see no change. The admin console could offer both.

## Context

Proposed in `explain-subquery-needs-a-row` (2026-11-04) once index planning
was split from reading; filed separately to keep that change to subqueries.
