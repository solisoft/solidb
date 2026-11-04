# An ascending SORT on a sparse index counts every index key first

## Severity

medium — performance regression introduced by the fix for
`sparse-index-drops-null-rows-from-ranges-and-sorts`.

## Location

- `src/sdbql/executor/execution/entry.rs` — `complete_index_sorted` (and
  `bare_sort_index`, which EXPLAIN uses) call
  `Collection::index_entry_count` for an ascending read of a sparse index.
- `src/storage/collection/indexes.rs` — `index_entry_count` iterates every
  key of the index.

## Problem

A sparse index has no entry for a document whose index fields are all null,
and those documents sort first. To read `FOR d IN c SORT d.f ASC LIMIT n`
from such an index exactly, the planner must know that no document is
missing from it, which it now checks by counting the index's keys against
`Collection::count()` (an O(1) atomic). That count is O(index size) on every
query.

Measured with `benchmark_planner_shapes` (200k documents, `idx_a` sparse,
every row has `a`): `SORT d.a ASC LIMIT 10` 0.07 ms before the fix (which
returned wrong rows when nulls existed) → 48.6 ms after. Still cheaper than
the scan and sort it replaces, but a large step back for a common shape.
Descending reads are unaffected (0.07 ms): they only need the read not to
come up short.

## Proposal

Keep an exact, cheap answer to "does every document have an entry in this
index?":

- cache the entry count per index together with a per-collection write
  generation (bumped by every document write path), recounting only when
  the generation moved; or
- maintain a per-index count of the documents left out (all-null rows) at
  the write sites `Index::holds` already guards, initialised by one count
  when the index metadata loads. Any drift would give wrong results, so
  this needs the same differential tests as the fix, plus a crash/restart
  test.

Until then, an index created with `"sparse": false` is read without the
count: the sort reads its first `n` entries, as before.

## Context

Found reviewing `sparse-index-drops-null-rows-from-ranges-and-sorts`
(2026-11-04).
