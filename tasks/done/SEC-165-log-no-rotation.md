# SEC-165: Log files have no rotation or size cap

## Status
- **Severity**: LOW
- **Category**: Operational / DoS
- **Project**: soli/db
- **File**: `src/main.rs`, `src/bin/solidb-fuse.rs`
- **Lines**: main.rs:157; solidb-fuse.rs:810

## Description
`File::create(&args.log_file)` writes to a single growing file. A long-running daemon eventually fills the disk.

## Recommendation
Use `tracing-appender::rolling::daily` (or hourly) with retention. The dependency is already pulled in elsewhere.

## References
- Related: SEC-094.
