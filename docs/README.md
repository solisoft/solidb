# `docs/`

Engine reference kept in the repository. The documentation **site** — the Soli
app in `doc/` — is the one to update; when the two disagree, `doc/` wins.

Only what has no page on the site lives here:

- `SDBQL_REFERENCE.md` — the complete SDBQL function and clause reference.
- `BACKUP.md` — physical checkpoints vs. logical dump/restore.

Seven other files were removed on 2026-09-20 (`CLIENTS.md`, `FUSE.md`,
`LUA_WEBSOCKET.md`, `SCHEMA_VALIDATION.md`, `SHARDING.md`, `TRANSACTIONS.md`,
`lua_enhancements_progress.md`). Six of them had not been touched since
2026-01-16 and were superseded by larger pages on the site — `clients*`,
`tooling`, `scripting-ws`, `api-collections`, `sharding`, `transactions`. The
seventh was a finished progress log. Recover any of them from git history if
needed.
