# A single-field index drops null and missing rows from `<` / `<=` and from SORT

## Severity

high — wrong results. A query returns fewer rows once an index exists.

## Location

- `src/sdbql/executor/index_opt.rs` — `use_index_for_condition`: `<` and
  `<=` go to `index_lookup_lt` / `index_lookup_lte` (`index_range_scan` in
  `src/storage/collection/indexes.rs`).
- `src/sdbql/executor/execution/entry.rs` — the SORT + LIMIT and SORT-only
  paths call `collection.index_sorted(field, …)`, which picks any index
  whose first field is the sort field (`get_index_for_field`).

## Problem

A regular index holds no entry for a row whose index fields are all null
(a missing field reads as null). SDBQL ranks null below every other value
(`compare_values` → `type_rank`), so null satisfies `f < x` and `f <= x`,
and sorts first in `SORT f ASC`. Reads served from the index miss them.

Observed on 2.6.4, collection `c` = `{f:1}`, `{f:null}`, `{}`, `{f:9}`:

| query | no index | with an index on `f` |
|---|---|---|
| `FILTER d.f <= 5` | k1, k2, k3 | k1 |
| `FILTER d.f < 5` | k1, k2, k3 | k1 |
| `SORT d.f ASC LIMIT 2` | k2, k3 | k1, k4 |

`SORT d.f` without a LIMIT, through the SORT-only path, drops the null rows
from the result altogether. A composite index `[f, g]` used for the bare
SORT has the same hole for rows where both are null.

Not affected: `>`, `>=` and `==` with a non-null value (null never matches
them), LIKE prefixes, and the composite paths of 2.6.3/2.6.4, whose reads
are under a non-null prefix where every row is indexed.

## Fix

- Serve `<` / `<=` from a single-field index only when null cannot match,
  i.e. never, unless the index holds nulls (see
  `index-nulls-block-ne-and-null-lookups`): otherwise read the range and add
  the null/missing rows, or fall back to the scan.
- The index-sorted SORT paths must not drop rows the index lacks: fall back
  to the scan for a sparse index, or (ASC) emit the unindexed rows first —
  which needs to find them, i.e. a scan or a non-sparse index.
- Tests comparing each shape against the scan on data with null and
  missing values.

## Context

Found while working on `index-nulls-block-ne-and-null-lookups` (2026-11-04).
