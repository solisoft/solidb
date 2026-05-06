# SEC-133: Upload session creation accepts unbounded `total_size` / `chunk_size`

## Status
- **Severity**: HIGH
- **Category**: Denial of Service
- **Project**: soli/db
- **File**: `src/server/upload_session.rs`
- **Lines**: 53-69

## Description
`UploadSession` computes `total_chunks = total_size.div_ceil(chunk_size as u64) as u32` and allocates `vec![false; total_chunks as usize]`. Both `total_size` and `chunk_size` come from the request without validation. With `total_size = u64::MAX, chunk_size = 1`, a single call allocates ~4 GiB per session.

## Exploit Scenario
```http
POST /_api/blob/{db}/{coll}/upload
{ "total_size": 18446744073709551615, "chunk_size": 1 }
```
Repeated calls allocate gigabytes per session and exhaust memory.

## Recommendation
- Cap `total_size` (e.g. 10 GiB).
- Require `chunk_size >= 64 KiB` (and a sensible upper bound).
- Reject upfront with 400 when bounds are violated.

## References
- Related: SEC-094.
