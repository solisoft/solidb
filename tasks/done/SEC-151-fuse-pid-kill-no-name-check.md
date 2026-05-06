# SEC-151: `solidb-fuse` PID-file kill skips process-name verification

## Status
- **Severity**: MEDIUM
- **Category**: Local Privilege Escalation
- **Project**: soli/db
- **File**: `src/bin/solidb-fuse.rs`
- **Lines**: 789-803

## Description
`solidb-fuse` reads a PID from `--pid-file` (default `./solidb-fuse.pid`, attacker-writable when CWD is shared) and sends `SIGTERM`. Unlike `src/main.rs:107-117`, it does **not** verify the target process name matches `solidb-fuse` (or `solidb`).

## Exploit Scenario
A local attacker writes a target PID into `./solidb-fuse.pid` before a legitimate restart. The next `solidb-fuse --stop` (or restart logic) kills the unrelated victim process.

## Recommendation
Mirror the validation from `src/main.rs:107-117`: use `sysinfo` (or `/proc/<pid>/comm`) to verify the process name before sending the signal.

## References
- Related: SEC-098.
