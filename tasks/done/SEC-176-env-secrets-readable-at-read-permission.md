# `_env` secrets (LLM API keys) are readable by any principal with Read

## Severity

high — a read-only principal exfiltrates every credential stored in `_env`,
including `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` / `GEMINI_API_KEY`. The
documented model says otherwise: `doc/app/views/docs/nl-queries.html.slv:348`
states "API keys stored in `_env` are accessible to database administrators."

## Location

- `src/server/env_handlers.rs` — `list_env_vars_handler` (no permission check
  of its own; relies on the middleware's default)
- `src/server/authz_middleware.rs:127` — `required_action`: GET → `Read`
- `src/driver/protocol/command.rs:680` — `ListEnvVars` → `Read`
- `_env` is an ordinary collection, so the generic document API and SDBQL
  reach it at `Read` as well

## Problem

`_env` is where the product tells users to put provider credentials, but it is
stored as a normal collection and read through the normal read paths. Nothing
in the stack treats it as privileged: the underscore-prefix checks that exist
(`src/server/handlers/documents.rs:263`, `src/server/nl_handlers.rs:124`) gate
trigger-firing and NL schema sampling, not access.

Four independent paths expose it at `Read`:

1. `GET /_api/database/{db}/env`
2. `GET /_api/database/{db}/document/_env/{key}`
3. `POST /_api/database/{db}/cursor` — `FOR d IN _env RETURN d`
4. driver protocol `ListEnvVars`

Verified against a v0.33.0 release build (2026-07-30). A key with the built-in
`viewer` role (global read, no write) returned:

```
GET /_api/database/tenant1/env                     -> 200
{"OPENAI_API_KEY":"sk-live-SUPERSECRET-do-not-leak"}

GET /_api/database/tenant1/document/_env/OPENAI_API_KEY -> 200
{... "value":"sk-live-SUPERSECRET-do-not-leak"}

POST /_api/database/tenant1/cursor
  {"query":"FOR d IN _env RETURN d"}               -> 200  (same value)

PUT /_api/database/tenant1/env/X                   -> 403  (writes correctly denied)
```

The 403 on write is the control: authorization works, the classification of
`_env` as ordinary read data is what is wrong. Note `get_env_var`
(`src/server/llm_client.rs:87`) also falls back to `_system/_env`, so Read on
`_system` yields the instance-wide credentials.

## Possible directions

- Treat `_env` as privileged: require `Admin` for the env endpoints and the
  driver's `ListEnvVars`, and deny `_env` through the generic document/SDBQL
  paths (a system-collection ACL, which the codebase currently lacks — the
  same gap would cover `_admins` / `_api_keys`).
- Return values redacted by default (`{"OPENAI_API_KEY":"sk-…4f2"}`) and make
  reading a cleartext value an explicit `Admin` operation.
- Longer term: keep credentials out of a queryable collection entirely — a
  separate secret store keyed per database, write-only via the API.

## Scope note

Only LLM provider settings are read from `_env` today, so this is credential
disclosure and not privilege escalation — no JWT/cluster secret is sourced
from it. Confirmed by auditing every `get_env_var` call site.
