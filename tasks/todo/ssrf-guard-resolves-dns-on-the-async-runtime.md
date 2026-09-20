# The SSRF guard resolves DNS synchronously on the async runtime

## Severity

medium — a slow or unreachable resolver blocks a tokio worker thread for the
duration of the lookup, on two request paths. Not a vulnerability; a
latency/availability characteristic, and the exact class the request-safety
audit closed elsewhere ("nothing on the request path waits forever").

## Location

- `src/server/ssrf.rs` — `validate_public_url_target` calls
  `std::net::ToSocketAddrs::to_socket_addrs`, which is blocking
- `src/queue/jobs.rs:440-450` — `execute_webhook` is `async fn` and calls
  `validate_webhook_url` / `validate_webhook_target` directly
- `src/server/llm_client.rs` — `validate_tenant_llm_url`, reached from the
  synchronous `LLMClient::from_storage`, which async handlers call inline
  (`nl_handlers.rs:378`, `handlers/ai/mod.rs:121`, `:137`, and six more)

## Problem

`to_socket_addrs` hands the lookup to the system resolver and blocks until it
answers. On a host whose resolver is slow, misconfigured, or firewalled, that
is seconds per call, on a thread tokio expects back promptly. Enough concurrent
requests naming unresolvable hosts will starve the runtime.

`from_storage` was already a synchronous function called from async contexts —
it reads `_env` from RocksDB — so the shape is not new. The difference is
magnitude: a RocksDB point read is microseconds and bounded; a DNS lookup is
neither.

An attacker does not need much: `OLLAMA_URL` is tenant-writable, so a principal
with `Write` can point it at a hostname served by a resolver that never
answers, then trigger any LLM-backed path. The SSRF guard itself refuses the
request — after it has already paid the wait.

Found while reviewing
[SEC-177](../done/SEC-177-ssrf-via-tenant-writable-ollama-url.md), which
extended the existing guard to a second call site. The webhook path has had
this shape since the guard was written.

## Fix direction

1. Give `ssrf` an async entry point that runs the resolution on the blocking
   pool — `tokio::task::spawn_blocking` around the `to_socket_addrs` call —
   keeping the synchronous one for callers that are genuinely off-runtime
   (`validate_job_target` at enqueue time, tests).
2. `execute_webhook` is already `async`, so it can await the new form directly.
3. The LLM path needs more thought, because `LLMClient::from_storage` is
   synchronous and has nine call sites. Two options, in order of preference:
   - resolve once, at the point the URL is *written* (`PUT /env/{key}`), and
     store the verdict — the URL changes far less often than it is used;
     re-validate at use to defeat rebinding, but from a cache with a short TTL
     so the common path does no lookup at all;
   - or make `from_storage` async and update the nine call sites.
4. Bound the lookup regardless: a resolver that never answers should fail the
   request in single-digit seconds, not whenever the OS gives up.

## Related

- [SEC-177](../done/SEC-177-ssrf-via-tenant-writable-ollama-url.md) — added the
  second call site.
