# SEC-125: SDBQL injection in queue worker via `script_path`

## Status
- **Severity**: CRITICAL
- **Category**: Injection / Privilege Escalation
- **Project**: soli/db
- **File**: `src/queue/jobs.rs`
- **Lines**: 172-175 (worker FILTER), 201-207 (admin context), plus `src/queue/cron.rs`

## Description
`execute_job` interpolates `job.script_path` directly into a SDBQL query as a single-quoted literal. The field is fully attacker-controlled (`EnqueueRequest.script`, `CreateCronJobRequest.script`) and never validated.

Compounding this, queue and cron jobs always run with `ScriptUser { username: "_system", roles: ["admin"] }` (see SEC-139), so a successful injection executes attacker-controlled Lua as admin.

## Exploit Scenario
```http
POST /queues/default/enqueue
{ "script": "x' OR true RETURN s LIMIT 1; //" }
```
The worker's FILTER becomes `FILTER s.path == 'x' OR true RETURN s LIMIT 1; //'` — picks up an arbitrary `_scripts` document and runs it as admin.

## Recommendation
- Use bind variables (`@script_path`) rather than string interpolation.
- Reject `script_path` values that fail a strict allowlist regex (e.g., `^[A-Za-z0-9_/\-.]+$`).
- Cap length to a sane bound.
- Apply the same validation in cron CREATE/UPDATE (see SEC-173).

## References
- Related: SEC-118, SEC-139.
