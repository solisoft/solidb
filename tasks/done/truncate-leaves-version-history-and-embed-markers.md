# Truncate leaves `docv:` version history and `embed_pending:` markers

## Severity

medium — a truncated collection keeps the version history of every document it
just deleted, so a time-travel read resurrects pre-truncate content from a
collection that reports zero documents. Pending auto-embedding markers survive
the same way and keep the process-wide gauge above zero.

## Location

- `src/storage/collection/crud.rs` — `truncate()`, the `data_ranges` array
- `src/storage/collection/mod.rs` — `DOCV_PREFIX` (`docv:`),
  `EMBED_PENDING_PREFIX` (`embed_pending:`)
- `src/storage/collection/versioning.rs` — version keys are
  `docv:{key}:{inverted_ts}`
- `src/storage/collection/vector.rs:530` — `clear_embed_pending`, the only
  thing that removes a marker today

## Problem

`truncate()` range-deletes nine data prefixes and deliberately preserves the
`*_meta:` definitions and `_stats:*` config, which is the right semantics:
index definitions, schema, collection type and shard config survive a truncate.
Vector index *data* is handled separately — each defined index is cleared in
memory and the empty state persisted over `vec_data:`.

Two data prefixes fall through both nets:

- **`docv:`** — document version history. Nothing in `truncate()` touches it.
  After truncating a versioned collection, `count()` is 0 and every document is
  gone, but `docv:{key}:{ts}` records remain, so a read `AS OF` a timestamp
  before the truncate returns documents from a collection that is empty. That
  is a storage leak *and* a surprising visibility result.
- **`embed_pending:`** — markers written by `mark_embed_pending` and removed
  only by `clear_embed_pending` on a successful embedding, per document. After
  a truncate the documents are gone but the markers point at them, so the queue
  worker keeps finding work that can never complete.

The second has a known downstream cost. `release_pending_embed`
(`vector.rs:594`) documents that the pending gauge is advisory precisely
because markers can vanish without a `Collection` to decrement through, and
that a gauge stuck above zero "keeps `check_embeddings` past its fast path —
enumerating every collection in the instance on every worker tick, forever".
Truncate is another way to desynchronise it, in the opposite direction:
markers that persist with no document behind them.

Found while reviewing
[truncate-leaves-blob-chunks-and-index-state](../done/truncate-leaves-blob-chunks-and-index-state.md),
whose own list — blob chunks, temp upload chunks, fulltext, geo, TTL expiry,
cached counters — is fully fixed and tested. These two prefixes were not named
in it.

## Fix direction

1. Decide the intended semantics for `docv:` first, and write it down. Deleting
   the history alongside the documents is the consistent reading — truncate is
   "remove all documents", and history of a removed document has no subject.
   If history is meant to survive, `truncate()` needs a comment saying so,
   because the current behaviour reads as an oversight either way.
2. If history goes: add `(b"docv:", b"docv;")` to `data_ranges`.
3. Add `(b"embed_pending:", b"embed_pending;")` to `data_ranges`, and
   decrement the process-wide gauge by the number of markers removed — count
   them before the range delete, or call `release_pending_embed` with the
   count, mirroring what the sweep does once a pass proves no markers exist.
4. Tests in `tests/test_truncate.rs`, alongside the existing
   `test_truncate_clears_*` family: a versioned collection has no history after
   truncate (or has it, per (1)), and a collection with pending embed markers
   has none afterwards with the gauge back down.

## Related

- [truncate-leaves-blob-chunks-and-index-state](../done/truncate-leaves-blob-chunks-and-index-state.md)
  — the same defect class for the prefixes it did name. Fixed and closed.
