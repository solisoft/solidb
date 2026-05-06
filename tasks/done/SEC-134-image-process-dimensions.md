# SEC-134: `solidb.image_process` accepts unchecked image dimensions

## Status
- **Severity**: HIGH
- **Category**: Denial of Service
- **Project**: soli/db
- **File**: `src/scripting/file_handling.rs`
- **Lines**: 593, 599, 605

## Description
`img.resize_exact(w, h, Lanczos3)` is called with `width`/`height` taken directly from a Lua table. There is no clamp on the requested output size.

## Exploit Scenario
```lua
solidb.image_process(small_blob, { resize = { width = 65535, height = 65535 } })
```
Allocates ~16 GiB for the output framebuffer, OOM-killing the server.

## Recommendation
- Clamp dimensions (e.g. `≤ 8192` on each axis).
- Reject if `w * h * 4 > MAX_IMG_BYTES` (e.g. 64 MiB).
- Apply the same caps in any other `solidb.*` image entry point.

## References
- Related: SEC-150 (decompression bomb).
