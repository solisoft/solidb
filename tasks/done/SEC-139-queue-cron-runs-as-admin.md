# SEC-139: Queue and cron jobs always run as `_system` admin

## Status
- **Severity**: HIGH
- **Category**: Privilege Escalation
- **Project**: soli/db
- **File**: `src/queue/jobs.rs`, `src/queue/cron.rs`
- **Lines**: jobs.rs:201-207; cron.rs (job dispatch)

## Description
Every queue/cron job, regardless of who enqueued it, executes with `ScriptUser { username: "_system", roles: ["admin"] }`. There is no record of the principal that scheduled the job.

## Exploit Scenario
Any user with enqueue permission targets a privileged service script. The job runs as admin, bypassing collection ACLs the script normally honors via `solidb.auth`. Combined with SEC-125, this becomes RCE-as-admin for any authenticated user.

## Recommendation
- Persist `enqueued_by` claims at enqueue time (`Job.user`, `CronJob.user`).
- When dispatching, construct `ScriptUser` from those persisted claims (not a static `_system`).
- Require admin role to enqueue jobs targeting admin-only scripts.

## References
- Related: SEC-125, SEC-127.
