# The docs-site spec hardcodes a released version

## Severity

low — test hygiene. The site is right; its spec is stale.

## Location

`doc/tests/docs_pages_spec.sl:6` — "the home page carries the version being
released" asserts `res_body(get("/")).include?("v2.4.1")`.

## Problem

The version pill moved on with every release (it shows v2.7.1 now), so the
test has failed since 2.4.2 and nobody saw it: CI runs no `soli test` for
`doc/` (deploy-www.yml builds the site and checks the version against
`Cargo.toml` with `check_docs_sync.sh`). Found running the spec while
documenting `planner-choose-between-candidate-plans`.

## Proposal

Read the version from `Cargo.toml` (or from `_version_pill.html.slv`) in the
spec instead of a literal, so it follows releases, and consider running
`soli test` for `doc/` in deploy-www.yml so a stale spec fails where it is
seen.
