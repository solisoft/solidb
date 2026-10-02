//! HTTP `Range` support for blob downloads.
//!
//! Two concerns live here, both pure so they can be unit-tested without a
//! server:
//!
//! - [`parse_range`] reads a single `Range: bytes=…` header against a known
//!   blob size (RFC 9110 §14). Anything it does not understand — another unit,
//!   several ranges, garbage — is *ignored*, which the RFC allows: the caller
//!   then serves the whole blob with `200`.
//! - [`ChunkLayout`] records where each stored chunk starts. Chunk sizes are
//!   not fixed (a resumable upload uses the client's `chunk_size`, a Lua
//!   upload 1 MiB, a multipart upload 1 MiB now and whatever the network
//!   delivered before range support), so a range can only jump straight to its
//!   first chunk when the upload wrote the sizes into the blob document.
//!   [`record_chunk_layout`] does that, compactly: one `chunk_size` number
//!   when every chunk but the last has the same length, a `chunk_sizes` array
//!   otherwise. A blob stored before that has neither, and its download walks
//!   the chunks from the first one.

use crate::storage::Document;
use serde_json::{Map, Value};

/// Field holding the common chunk length when every chunk but the last has it.
pub const CHUNK_SIZE_FIELD: &str = "chunk_size";
/// Field holding every chunk's length when they are not uniform.
pub const CHUNK_SIZES_FIELD: &str = "chunk_sizes";

/// What a `Range` header asks of a blob of a given size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeRequest {
    /// No usable range: serve the whole blob with `200`.
    Full,
    /// Serve bytes `start..=end` (inclusive, both within the blob) with `206`.
    Partial { start: u64, end: u64 },
    /// A well-formed range that selects nothing: answer `416`.
    Unsatisfiable,
}

/// Parse one byte-range spec (`a-b`, `a-` or `-n`) against `total` bytes.
///
/// `None` when the header is absent, malformed, uses another unit or asks for
/// several ranges — all of which mean "send the full representation".
pub fn parse_range(header: Option<&str>, total: u64) -> RangeRequest {
    let Some(raw) = header else {
        return RangeRequest::Full;
    };
    let raw = raw.trim();
    let Some((unit, set)) = raw.split_once('=') else {
        return RangeRequest::Full;
    };
    if !unit.trim().eq_ignore_ascii_case("bytes") {
        return RangeRequest::Full;
    }
    let set = set.trim();
    // Multi-range answers need multipart/byteranges; ignoring the header is
    // the RFC-sanctioned alternative and every media client copes with it.
    if set.is_empty() || set.contains(',') {
        return RangeRequest::Full;
    }
    let Some((first, last)) = set.split_once('-') else {
        return RangeRequest::Full;
    };
    let (first, last) = (first.trim(), last.trim());

    match (first.is_empty(), last.is_empty()) {
        // `-n`: the last n bytes.
        (true, false) => {
            let Some(suffix) = parse_pos(last) else {
                return RangeRequest::Full;
            };
            if suffix == 0 || total == 0 {
                return RangeRequest::Unsatisfiable;
            }
            RangeRequest::Partial {
                start: total.saturating_sub(suffix),
                end: total - 1,
            }
        }
        // `a-`: from a to the end.
        (false, true) => {
            let Some(start) = parse_pos(first) else {
                return RangeRequest::Full;
            };
            if start >= total {
                return RangeRequest::Unsatisfiable;
            }
            RangeRequest::Partial {
                start,
                end: total - 1,
            }
        }
        // `a-b`: inclusive, the end clamped to the blob.
        (false, false) => {
            let (Some(start), Some(end)) = (parse_pos(first), parse_pos(last)) else {
                return RangeRequest::Full;
            };
            if end < start {
                // RFC 9110 §14.1.1: an invalid range-spec, not an
                // unsatisfiable one.
                return RangeRequest::Full;
            }
            if start >= total {
                return RangeRequest::Unsatisfiable;
            }
            RangeRequest::Partial {
                start,
                end: end.min(total - 1),
            }
        }
        (true, true) => RangeRequest::Full,
    }
}

/// A run of ASCII digits. One too large for `u64` saturates: it is still a
/// valid position (past any blob), so it must not turn the header into noise.
fn parse_pos(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(s.parse::<u64>().unwrap_or(u64::MAX))
}

/// Where each chunk of a blob starts, as recorded at upload time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkLayout {
    /// Every chunk but the last is `chunk_size` bytes.
    Uniform { chunk_size: u64, chunks: u32 },
    /// Each chunk's length, in order.
    Explicit(Vec<u64>),
}

impl ChunkLayout {
    /// The layout a blob document declares, if it declares one that agrees
    /// with its `chunks` and `size`. A layout that disagrees is treated as
    /// absent — the download then walks the chunks — rather than trusted to
    /// cut the wrong bytes.
    pub fn from_doc(doc: &Document, chunks: u32, total: u64) -> Option<Self> {
        Self::from_fields(
            doc.get(CHUNK_SIZE_FIELD).as_ref(),
            doc.get(CHUNK_SIZES_FIELD).as_ref(),
            chunks,
            total,
        )
    }

    /// [`ChunkLayout::from_doc`] on the two layout fields themselves.
    pub fn from_fields(
        chunk_size: Option<&Value>,
        chunk_sizes: Option<&Value>,
        chunks: u32,
        total: u64,
    ) -> Option<Self> {
        if chunks == 0 {
            return None;
        }
        if let Some(sizes) = chunk_sizes.and_then(Value::as_array) {
            let sizes: Option<Vec<u64>> = sizes.iter().map(Value::as_u64).collect();
            let sizes = sizes?;
            let sum = sizes.iter().try_fold(0u64, |acc, s| acc.checked_add(*s));
            return (sizes.len() == chunks as usize && sum == Some(total))
                .then_some(ChunkLayout::Explicit(sizes));
        }
        let chunk_size = chunk_size.and_then(Value::as_u64)?;
        if chunk_size == 0 {
            return None;
        }
        // All but the last chunk are full, and the last holds 1..=chunk_size.
        let full = (chunks as u64 - 1).checked_mul(chunk_size)?;
        let max = full.checked_add(chunk_size)?;
        (full < total && total <= max).then_some(ChunkLayout::Uniform { chunk_size, chunks })
    }

    /// The first chunk holding byte `offset`, and the offset that chunk
    /// starts at. `None` when `offset` is past the end.
    pub fn locate(&self, offset: u64, total: u64) -> Option<(u32, u64)> {
        if offset >= total {
            return None;
        }
        match self {
            ChunkLayout::Uniform { chunk_size, chunks } => {
                let idx = offset / chunk_size;
                (idx < *chunks as u64).then(|| (idx as u32, idx * chunk_size))
            }
            ChunkLayout::Explicit(sizes) => {
                let mut start = 0u64;
                for (idx, len) in sizes.iter().enumerate() {
                    if offset < start + len {
                        return Some((idx as u32, start));
                    }
                    start += len;
                }
                None
            }
        }
    }

    /// The recorded length of chunk `idx` in a blob of `total` bytes.
    pub fn chunk_len(&self, idx: u32, total: u64) -> u64 {
        match self {
            ChunkLayout::Uniform { chunk_size, chunks } => {
                if idx + 1 < *chunks {
                    *chunk_size
                } else {
                    total - (*chunks as u64 - 1) * chunk_size
                }
            }
            ChunkLayout::Explicit(sizes) => sizes.get(idx as usize).copied().unwrap_or(0),
        }
    }
}

/// Write the layout of chunks of the given lengths into a blob document.
///
/// `chunk_size` alone when every chunk but the last has the same length and
/// the last is no longer than the others — the shape of every resumable, Lua
/// and multipart upload that behaves — so a 150 MB file costs one number,
/// not a 150-entry array. Anything else gets the full `chunk_sizes` list.
pub fn record_chunk_layout(metadata: &mut Map<String, Value>, sizes: &[u64]) {
    metadata.remove(CHUNK_SIZE_FIELD);
    metadata.remove(CHUNK_SIZES_FIELD);
    let Some((&first, _)) = sizes.split_first() else {
        return;
    };
    let (body, last) = sizes.split_at(sizes.len() - 1);
    let uniform = first > 0 && body.iter().all(|&s| s == first) && last[0] > 0 && last[0] <= first;
    if uniform {
        metadata.insert(CHUNK_SIZE_FIELD.to_string(), Value::from(first));
    } else {
        metadata.insert(
            CHUNK_SIZES_FIELD.to_string(),
            Value::Array(sizes.iter().map(|&s| Value::from(s)).collect()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(h: &str, total: u64) -> RangeRequest {
        parse_range(Some(h), total)
    }

    fn layout_of(doc: &Value, chunks: u32, total: u64) -> Option<ChunkLayout> {
        ChunkLayout::from_fields(
            doc.get(CHUNK_SIZE_FIELD),
            doc.get(CHUNK_SIZES_FIELD),
            chunks,
            total,
        )
    }

    #[test]
    fn parses_the_three_single_range_forms() {
        assert_eq!(
            p("bytes=0-9", 100),
            RangeRequest::Partial { start: 0, end: 9 }
        );
        assert_eq!(
            p("bytes=10-", 100),
            RangeRequest::Partial { start: 10, end: 99 }
        );
        assert_eq!(
            p("bytes=-5", 100),
            RangeRequest::Partial { start: 95, end: 99 }
        );
        assert_eq!(
            p(" Bytes = 3 - 4 ", 100),
            RangeRequest::Partial { start: 3, end: 4 }
        );
    }

    #[test]
    fn clamps_ends_past_the_blob() {
        assert_eq!(
            p("bytes=90-500", 100),
            RangeRequest::Partial { start: 90, end: 99 }
        );
        assert_eq!(
            p("bytes=-500", 100),
            RangeRequest::Partial { start: 0, end: 99 }
        );
        assert_eq!(
            p("bytes=0-99999999999999999999999", 100),
            RangeRequest::Partial { start: 0, end: 99 }
        );
    }

    #[test]
    fn unsatisfiable_ranges() {
        assert_eq!(p("bytes=100-", 100), RangeRequest::Unsatisfiable);
        assert_eq!(p("bytes=100-200", 100), RangeRequest::Unsatisfiable);
        assert_eq!(p("bytes=-0", 100), RangeRequest::Unsatisfiable);
        assert_eq!(p("bytes=0-", 0), RangeRequest::Unsatisfiable);
        assert_eq!(p("bytes=-1", 0), RangeRequest::Unsatisfiable);
    }

    #[test]
    fn malformed_or_multi_range_is_ignored() {
        assert_eq!(parse_range(None, 100), RangeRequest::Full);
        for h in [
            "",
            "bytes",
            "bytes=",
            "bytes=-",
            "bytes=abc",
            "bytes=5-2",
            "bytes=1-2-3",
            "bytes=0-1,5-6",
            "items=0-9",
            "bytes=+1-2",
            "bytes=0x1-2",
        ] {
            assert_eq!(p(h, 100), RangeRequest::Full, "header {h:?}");
        }
    }

    #[test]
    fn records_uniform_layouts_compactly() {
        let mut m = Map::new();
        record_chunk_layout(&mut m, &[4, 4, 4, 2]);
        assert_eq!(m.get(CHUNK_SIZE_FIELD), Some(&json!(4)));
        assert!(m.get(CHUNK_SIZES_FIELD).is_none());

        record_chunk_layout(&mut m, &[7]);
        assert_eq!(m.get(CHUNK_SIZE_FIELD), Some(&json!(7)));

        record_chunk_layout(&mut m, &[3, 5, 2]);
        assert!(m.get(CHUNK_SIZE_FIELD).is_none());
        assert_eq!(m.get(CHUNK_SIZES_FIELD), Some(&json!([3, 5, 2])));

        // A last chunk longer than the others is not uniform.
        record_chunk_layout(&mut m, &[2, 2, 3]);
        assert_eq!(m.get(CHUNK_SIZES_FIELD), Some(&json!([2, 2, 3])));

        record_chunk_layout(&mut m, &[]);
        assert!(m.is_empty());
    }

    #[test]
    fn reads_back_what_was_recorded() {
        let mut m = Map::new();
        record_chunk_layout(&mut m, &[4, 4, 2]);
        let doc = Value::Object(m.clone());
        let layout = layout_of(&doc, 3, 10).unwrap();
        assert_eq!(layout.locate(0, 10), Some((0, 0)));
        assert_eq!(layout.locate(3, 10), Some((0, 0)));
        assert_eq!(layout.locate(4, 10), Some((1, 4)));
        assert_eq!(layout.locate(9, 10), Some((2, 8)));
        assert_eq!(layout.locate(10, 10), None);
        assert_eq!(layout.chunk_len(1, 10), 4);
        assert_eq!(layout.chunk_len(2, 10), 2);

        record_chunk_layout(&mut m, &[3, 5, 2]);
        let doc = Value::Object(m);
        let layout = layout_of(&doc, 3, 10).unwrap();
        assert_eq!(layout.locate(2, 10), Some((0, 0)));
        assert_eq!(layout.locate(3, 10), Some((1, 3)));
        assert_eq!(layout.locate(8, 10), Some((2, 8)));
        assert_eq!(layout.chunk_len(1, 10), 5);
    }

    #[test]
    fn rejects_layouts_that_disagree_with_the_document() {
        // Old blob: nothing recorded.
        assert_eq!(layout_of(&json!({}), 3, 10), None);
        // Sizes that do not add up, or a wrong count.
        assert_eq!(layout_of(&json!({"chunk_sizes": [3, 5, 1]}), 3, 10), None);
        assert_eq!(layout_of(&json!({"chunk_sizes": [5, 5]}), 3, 10), None);
        assert_eq!(layout_of(&json!({"chunk_sizes": [5, "5"]}), 2, 10), None);
        // A uniform size that cannot produce `size` from `chunks` chunks.
        assert_eq!(layout_of(&json!({"chunk_size": 4}), 3, 8), None);
        assert_eq!(layout_of(&json!({"chunk_size": 4}), 3, 13), None);
        assert_eq!(layout_of(&json!({"chunk_size": 0}), 3, 10), None);
        assert_eq!(
            layout_of(&json!({"chunk_size": 4}), 3, 12),
            Some(ChunkLayout::Uniform {
                chunk_size: 4,
                chunks: 3
            })
        );
    }
}
