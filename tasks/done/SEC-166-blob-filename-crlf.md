# SEC-166: Blob filename used in `Content-Disposition` without sanitization

## Status
- **Severity**: LOW
- **Category**: Header Injection
- **Project**: soli/db
- **File**: `src/server/handlers/blobs.rs`
- **Lines**: 69-90 (upload), 318 (download)

## Description
`field.file_name()` and `field.content_type()` from multipart upload are stored verbatim. On download, the filename is interpolated into `Content-Disposition` without escaping. A filename containing CR/LF can inject extra response headers; a filename with `"` can break out of the quoted-string.

## Exploit Scenario
A user uploads a file named `evil"\r\nSet-Cookie: x=y; Path=/`. When the file is downloaded, the response includes the injected header.

## Recommendation
- Apply the existing `sanitize_filename` helper at upload time, before persisting metadata.
- Or sanitize at download time by RFC 6266 encoding (`filename*=UTF-8''…`).

## References
- Related: SEC-098.
