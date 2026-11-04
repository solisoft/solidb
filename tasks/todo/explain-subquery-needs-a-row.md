# EXPLAIN cannot explain a correlated subquery that no row reaches

## Severity

low — diagnostics.

## Location

`src/sdbql/executor/explain.rs` — `explain_subqueries`, called with
`rows.first()` for body `LET`s, `FILTER`s, post-LIMIT `LET`s and `RETURN`.

## Problem

Since 2.6.4 EXPLAIN lists the loops of subqueries (`"subquery": "LET o"`),
each explained by running it against the first row that reaches it, so the
index choice is the one execution makes for that row. When no row reaches
the subquery — an empty collection, a FILTER matching nothing, a
development database — the subquery is not explained and a warning says
so. On a fresh or test database this hides exactly what an audit needs:
whether a Soli `includes` (`FOR rel IN users FILTER rel._key ==
doc.owner_id …`) or a has-many lookup has an index.

EXPLAIN also *runs* the query to time each step, which is slow on large data
and makes the plan depend on the data.

## Proposal

A static plan mode for a subquery's FILTER: treat an expression that reads
an outer variable (`doc.owner_id`) as an unknown non-null scalar, and choose
the index by shape (field and operator) without evaluating or reading.
Report it with `"estimated": true` when no row was available.

This needs index planning split from reading, the same refactor
`or-union-unindexed-branch` proposes. Once that exists, an `EXPLAIN` that
does not execute (`{"analyze": false}`) becomes possible too.

## Context

CRM index audit, 2026-11-04: the local CRM database was nearly empty, so
subquery index use had to be checked by hand against the index list.
