# SSRF: a tenant with Write picks the LLM URL, and the response body comes back

## Severity

high — any principal with `Write` on a single database makes the server issue
HTTP requests to arbitrary hosts and reads the response body. Full read-SSRF
from the server's network position (cloud metadata, internal admin panels,
anything reachable from the DB host).

## Location

- `src/server/llm_client.rs:187-205` — `from_storage` builds `api_url` from the
  `OLLAMA_URL` value in `_env`
- `src/server/llm_client.rs:604-609` — `chat_ollama` puts the upstream response
  body into the returned error
- `src/server/authz_middleware.rs` — `PUT /_api/database/{db}/env/{key}` is a
  plain `Write`; `POST /_api/database/{db}/nl` is classified `Read`

## Problem

`OLLAMA_URL` and `NL_DEFAULT_PROVIDER` are read from the database's own `_env`
collection, which a tenant with `Write` on that database controls. The Ollama
branch is the one provider whose base URL is not hardcoded — it is taken
verbatim, with only a scheme prefix added if missing:

```rust
let base_url = get_env_var(storage, db_name, "OLLAMA_URL")
    .unwrap_or_else(|| "http://localhost:11434".to_string());
// ... prepends http:// if no scheme; no host/scheme validation
api_url: format!("{}/api/chat", base_url),
```

There is no allowlist, no loopback/link-local/private-range rejection, and no
redirect policy. Worse, a non-2xx upstream reply is echoed to the caller:

```rust
let body = response.text().await.unwrap_or_default();
return Err(DbError::ExecutionError(format!("Ollama API error {}: {}", status, body)));
```

so the attacker reads the internal response, not just triggers the request.

Verified end-to-end against a v0.33.0 release build (2026-07-30) with a key
holding the built-in `editor` role (global read+write, **not** admin):

```
PUT /_api/database/tenant1/env/OLLAMA_URL          {"value":"http://127.0.0.1:7803"}
PUT /_api/database/tenant1/env/NL_DEFAULT_PROVIDER {"value":"ollama"}
POST /_api/database/tenant1/nl                     {"query":"how many users are there"}

-> 500
{"error":"Query execution error: Ollama API error 403 Forbidden: INTERNAL-METADATA-SECRET-abc123"}
```

The stand-in internal service logged `HIT POST /api/chat`, and its body came
back verbatim in the API error. Triggering the call only needs `Read` (the
authz middleware maps `/nl` to Read); `Write` is needed once, to plant the URL.

Reachable from every LLM-backed path, not just `/nl` — embeddings
(`src/queue/embeddings.rs`), graph-RAG, and community summarization all build
their client through `from_storage`.

## Possible directions

- Validate the resolved URL before the request: require http/https, resolve the
  host and reject loopback, link-local (169.254.0.0/16), and RFC1918 ranges
  unless an explicit opt-in (`SOLIDB_ALLOW_PRIVATE_LLM_URL=1`) is set — local
  Ollama is the normal case, so the opt-in has to exist and should be
  re-checked after DNS resolution to avoid rebinding.
- Do not echo upstream bodies. Log them at debug and return a generic
  "upstream LLM request failed (status N)" to the caller. This alone downgrades
  the issue from read-SSRF to blind SSRF.
- Consider making provider *endpoint* configuration an `Admin`-level setting
  (process env or `_system`) while leaving model choice per-database — a tenant
  choosing its model is reasonable, a tenant choosing the URL the server dials
  is not.
- Set an explicit redirect policy (`reqwest::redirect::Policy::none()`) on the
  LLM client.

## Related

Same root cause as [SEC-176](SEC-176-env-secrets-readable-at-read-permission.md):
`_env` is treated as ordinary tenant data while feeding
security-relevant configuration.
