# Collection truncate only deletes `doc:` keys — blob chunks survive

## Severity

medium — truncated blob collections keep their `blo:` chunks forever (storage leak + stale data)

## Location

`src/storage/collection/crud.rs` — `truncate()` (`self.all()` → `delete_batch(doc keys)`)

## Problem

`truncate()` collects document keys and batch-deletes them. A collection
CF also holds `blo:` blob chunks, `blo_tmp:` resumable-upload chunks,
fulltext (`ft:`/`ft_term:`), geo, and TTL expiry entries (see prefixes in
`src/storage/collection/mod.rs`). None of these are touched, so:

- blob collections leak chunk data on every truncate (the cached
  `chunk_count` atomic also goes stale),
- TTL expiry index entries point at deleted docs,
- fulltext/geo entries for deleted docs survive unless `delete_batch`
  cleans them per-doc (verify).

Truncate should instead range-delete every known prefix in the CF (or use
`delete_range_cf` per prefix), and reset the cached counters.

## Context

Found while moving `soli test`'s per-run database reset from drop+create
to truncate (drop is ~178ms/collection, see
[slow-database-drop-per-cf-options-rewrite](slow-database-drop-per-cf-options-rewrite.md)).
The runner sidesteps correctness issues by preserving collection *types*
("blob" vs "document") when pre-creating, but truncated blob collections
still accumulate orphan chunks across runs.
