# SEC-150: `image::load_from_memory` runs without decompression-bomb limits

## Status
- **Severity**: MEDIUM
- **Category**: Denial of Service
- **Project**: soli/db
- **File**: `src/scripting/file_handling.rs`
- **Lines**: 255, 581

## Description
Image loading is performed via `image::load_from_memory(&bytes)` with no `Limits`. A small compressed input can describe a huge canvas (PNG, WebP, etc.), allocating gigabytes during decode.

## Exploit Scenario
A few-KB PNG declares dimensions of `2^15 × 2^15`. Decoding allocates ~4 GiB before the script can even use the result.

## Recommendation
Use `image::io::Reader::new(...).with_guessed_format()?.limits(Limits { max_alloc: Some(64 * 1024 * 1024), max_image_width: Some(8192), max_image_height: Some(8192) }).decode()`.

## References
- Related: SEC-134.
