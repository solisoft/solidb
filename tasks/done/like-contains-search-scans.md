# `LIKE "%term%"` (every search box) is always a full scan

## Severity

low-medium — performance. Every list page of the Soli CRM runs one, even
with an empty search box.

## Location

- `src/sdbql/executor/index_opt.rs` — `like_literal_prefix` turns a LIKE
  pattern into an index prefix read; a pattern that starts with `%` has no
  prefix, so no index is used.
- Soli's query builder (`lang/src/db/hash_filter.rs`): `{"ilike": p}`
  compiles to `LIKE(doc.f, @p, true)`.

## Problem

Soli's `Model.search(q)` builds `(LIKE(doc.a, "%q%", true) OR LIKE(doc.b,
"%q%", true) …)`. A substring match cannot use an ordered index, so the
whole collection is read. Two things make it worse in the CRM:

- list controllers call `.search(q)` unconditionally, so a blank box sends
  `LIKE(…, "%%", true)`; the pagination COUNT then scans the collection on
  every list load, where an index count or `COLLECTION_COUNT` would answer;
- global search runs the same OR group on five collections per keystroke
  (after two characters).

`LIKE(x, "%%")` cannot simply be dropped by the planner: it is false for a
null or missing field, so it is not a no-op.

## Proposal

Pick one, or both:

1. **An n-gram (trigram) index type** that the planner uses for
   `LIKE "%term%"` (and its case-insensitive form) when the term has three
   characters or more: intersect the posting lists of the term's trigrams,
   then re-check the FILTER on the candidates. The fulltext index
   tokenises on words, so it cannot answer a substring match exactly.
2. **On the Soli side**, skip the search group when `q` is blank, and map
   `search` to `FULLTEXT(…)` on a model that declares a fulltext index.

## Context

CRM index audit, 2026-11-04 (see the 2.6.4 changelog). Global search and
the five main list pages (accounts, contacts, leads, opportunities, cases)
all use this shape.
