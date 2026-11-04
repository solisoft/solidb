# `!=`, `== null` and `IN [null]` with nothing fixed before them always scan

## Severity

medium — performance. Shapes the Soli CRM runs on every list page, console
poll and dashboard load fall back to a full collection scan.

## Location

- `src/storage/collection/indexes.rs` — the index insert paths skip a row
  whose index fields are all null (`!field_values.iter().all(|v| v.is_null())`);
  `index_lookup_eq` falls back to a scan for a null value.
- `src/sdbql/executor/index_opt.rs` — `in_list_is_indexable` refuses an `IN`
  list holding null; `composite_plan` refuses a plan where some combination
  of the fixed values is all null; an exclusion (`!=` / `NOT IN`) is only
  planned on the field *after* fixed ones.

## Problem

An index entry is written only when at least one of the index's fields is
non-null, so a single-field index never holds null or missing values. Any
condition that null satisfies cannot be answered from it:

- `d.status != "Closed"` (null != "Closed" is true),
- `d.owner_id == null`, `d.erased_at == null`,
- `d.owner_id IN [null, ""]`.

2.6.4 serves these only after a fixed non-null field of a composite index
(`owner_id == @me AND status != "Completed"` with `[owner_id, status]`),
because every row under a non-null prefix is indexed. With nothing before
them they still scan.

Shapes from the CRM audit (2026-11-04) that hit this:

- `support_cases`: `status != "Closed"` (list default, dashboard count, SLA
  crons every 15 min), `owner_id == null AND status != "Closed"` (console,
  polled every 15 s);
- `leads`: `status != "Closed - Converted"` (list default, dashboard);
- `accounts`/`opportunities`: `owner_id IN [null, ""]` (document visibility
  filter, every list of quotes, orders, invoices for a non-admin);
- `users.deactivated_at == null`, `leads.erased_at == null`.

## Proposal

An index option `sparse: false` (default `true`, so existing indexes and
their size are unchanged) that writes an entry for every document, a
missing or null value encoded as null (codec tag `0x01`, which sorts first,
as `compare_values` ranks it).

On such an index the planner can then serve:

- `== null` as an equality lookup on the null key;
- `IN [...]` with null like any other list;
- `!=` / `NOT IN` with nothing fixed before, as the ranges on each side of
  the excluded values (the interval code from `next_field_intervals` works
  unchanged on the first field);
- the composite all-null refusal no longer applies.

Things that rely on "one entry per document under its `encode_key` form" and
must be checked: the raw-rows path (`raw_scan.rs`), index-only counts
(`index_count.rs`), index-ordered reads (`index_sort.rs`), uniqueness checks
(a unique non-sparse index would allow a single null; decide), replication of
the index spec, and `docs/storage-format.md`.

## Context

Found in the CRM index audit that led to 2.6.4 (composite prefix, `!=` after
a prefix, `IN` with null, `OR` unions). The CRM can also work around it by
rewriting `!=` as `IN [open values]` and normalising "no owner" to `""`.
