//! Parser for a peer's collection export stream, used by shard copy and
//! healing (audit D7).
//!
//! Both callers used to run `String::from_utf8_lossy` on every network chunk
//! and split the result on `'\n'`. A multi-byte UTF-8 character straddling a
//! chunk boundary became U+FFFD on each side — silent corruption during a
//! rebalance — and the line buffer grew without bound on a stream with no
//! newline. This parser buffers raw bytes, splits on `b'\n'`, decodes only
//! complete lines, and caps a line's length.
//!
//! The export writes a blob chunk as a JSON header carrying `_data_length`,
//! then that many raw bytes, then `\n`. The old line splitter tore those raw
//! bytes apart; they are now read by length. A header carrying base64
//! `_blob_data` is accepted as well.

use crate::error::DbError;
use crate::storage::collection::Collection;
use serde_json::Value;

/// Longest export line accepted (one JSON document).
pub const MAX_EXPORT_LINE_BYTES: usize = 64 * 1024 * 1024;

/// Largest raw blob chunk accepted after a `_data_length` header.
pub const MAX_EXPORT_BLOB_CHUNK_BYTES: usize = 64 * 1024 * 1024;

/// One decoded record of an export stream.
#[derive(Debug, PartialEq)]
pub enum ExportItem {
    Doc(Value),
    BlobChunk {
        key: String,
        index: u32,
        data: Vec<u8>,
    },
}

/// Incremental export-stream parser. Feed it network chunks in order.
pub struct ExportStreamParser {
    buf: Vec<u8>,
    /// `(key, index, len)` of a blob chunk whose raw bytes come next.
    pending_blob: Option<(String, u32, usize)>,
    max_line: usize,
    /// Lines that were not valid JSON (skipped, as before).
    pub malformed_lines: usize,
}

impl Default for ExportStreamParser {
    fn default() -> Self {
        Self::new(MAX_EXPORT_LINE_BYTES)
    }
}

impl ExportStreamParser {
    pub fn new(max_line: usize) -> Self {
        Self {
            buf: Vec::new(),
            pending_blob: None,
            max_line,
            malformed_lines: 0,
        }
    }

    /// Consume one chunk; returns every record it completes.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<ExportItem>, String> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        let mut pos = 0usize;

        loop {
            if let Some((_, _, len)) = self.pending_blob {
                if self.buf.len() - pos < len {
                    break; // wait for the rest of the raw chunk
                }
                let (key, index, _) = self.pending_blob.take().expect("checked above");
                let data = self.buf[pos..pos + len].to_vec();
                pos += len;
                // The delimiter may arrive in the next network chunk; then it
                // is read as an empty line and skipped.
                if self.buf.get(pos) == Some(&b'\n') {
                    pos += 1;
                }
                out.push(ExportItem::BlobChunk { key, index, data });
                continue;
            }

            let Some(nl) = self.buf[pos..].iter().position(|b| *b == b'\n') else {
                if self.buf.len() - pos > self.max_line {
                    return Err(format!(
                        "export line exceeds {} bytes without a newline",
                        self.max_line
                    ));
                }
                break;
            };
            let end = pos + nl;
            if nl > self.max_line {
                return Err(format!(
                    "export line of {} bytes exceeds {}",
                    nl, self.max_line
                ));
            }
            let line_range = pos..end;
            pos = end + 1;
            let line = self.buf[line_range].to_vec();
            self.handle_line(&line, &mut out)?;
        }

        self.buf.drain(..pos);
        Ok(out)
    }

    /// End of stream: decode a final line that had no trailing newline.
    pub fn finish(mut self) -> Result<Vec<ExportItem>, String> {
        if let Some((key, _, len)) = &self.pending_blob {
            return Err(format!(
                "export stream ended inside blob chunk for {} ({} of {} bytes)",
                key,
                self.buf.len(),
                len
            ));
        }
        let mut out = Vec::new();
        let rest = std::mem::take(&mut self.buf);
        self.handle_line(&rest, &mut out)?;
        if self.pending_blob.is_some() {
            return Err("export stream ended after a blob chunk header".to_string());
        }
        Ok(out)
    }

    fn handle_line(&mut self, line: &[u8], out: &mut Vec<ExportItem>) -> Result<(), String> {
        let line = line.trim_ascii();
        if line.is_empty() {
            return Ok(());
        }
        let doc: Value = match serde_json::from_slice(line) {
            Ok(v) => v,
            Err(_) => {
                self.malformed_lines += 1;
                return Ok(());
            }
        };

        let is_blob_chunk = doc.get("_type").and_then(|t| t.as_str()) == Some("blob_chunk");
        if !is_blob_chunk {
            out.push(ExportItem::Doc(doc));
            return Ok(());
        }

        let key = doc.get("_doc_key").and_then(|s| s.as_str());
        let index = doc.get("_chunk_index").and_then(|n| n.as_u64());
        let (Some(key), Some(index)) = (key, index) else {
            self.malformed_lines += 1;
            return Ok(());
        };
        let index = u32::try_from(index).map_err(|_| "blob chunk index out of range")?;

        if let Some(b64) = doc.get("_blob_data").and_then(|s| s.as_str()) {
            use base64::{engine::general_purpose, Engine as _};
            match general_purpose::STANDARD.decode(b64) {
                Ok(data) => out.push(ExportItem::BlobChunk {
                    key: key.to_string(),
                    index,
                    data,
                }),
                Err(_) => self.malformed_lines += 1,
            }
            return Ok(());
        }

        if let Some(len) = doc.get("_data_length").and_then(|n| n.as_u64()) {
            let len = usize::try_from(len).unwrap_or(usize::MAX);
            if len > MAX_EXPORT_BLOB_CHUNK_BYTES {
                return Err(format!(
                    "blob chunk of {} bytes exceeds {}",
                    len, MAX_EXPORT_BLOB_CHUNK_BYTES
                ));
            }
            self.pending_blob = Some((key.to_string(), index, len));
            return Ok(());
        }

        self.malformed_lines += 1;
        Ok(())
    }
}

/// Stream a successful export response into `coll`: documents are upserted
/// in batches of 1000, blob chunks written as they arrive. Returns the number
/// of documents copied. A transport error or an over-long line fails the copy
/// instead of reporting a partial copy as complete.
pub async fn import_export_response(
    mut resp: reqwest::Response,
    coll: &Collection,
) -> Result<usize, DbError> {
    const BATCH: usize = 1000;
    let mut parser = ExportStreamParser::default();
    let mut batch_docs: Vec<(String, Value)> = Vec::with_capacity(BATCH);
    let mut total_copied = 0usize;

    fn apply(items: Vec<ExportItem>, coll: &Collection, batch_docs: &mut Vec<(String, Value)>) {
        for item in items {
            match item {
                ExportItem::BlobChunk { key, index, data } => {
                    if let Err(e) = coll.put_blob_chunk(&key, index, &data) {
                        tracing::error!("HEAL: Failed to write chunk {} for {}: {}", index, key, e);
                    }
                }
                ExportItem::Doc(mut doc) => {
                    // Clean metadata (same as import)
                    if let Some(obj) = doc.as_object_mut() {
                        obj.remove("_database");
                        obj.remove("_collection");
                        obj.remove("_shardConfig");
                    }
                    let key = doc
                        .get("_key")
                        .and_then(|k| k.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !key.is_empty() {
                        batch_docs.push((key, doc));
                    }
                }
            }
        }
    }

    fn flush(coll: &Collection, batch_docs: &mut Vec<(String, Value)>, total: &mut usize) {
        if batch_docs.is_empty() {
            return;
        }
        let count = batch_docs.len();
        if let Err(e) = coll.upsert_batch(std::mem::take(batch_docs)) {
            tracing::error!("HEAL: Batch upsert failed: {}", e);
        } else {
            *total += count;
        }
    }

    loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                flush(coll, &mut batch_docs, &mut total_copied);
                return Err(DbError::InternalError(format!(
                    "Export stream failed after {} docs: {}",
                    total_copied, e
                )));
            }
        };
        let items = parser
            .feed(&chunk)
            .map_err(|e| DbError::InternalError(format!("Export stream rejected: {}", e)))?;
        apply(items, coll, &mut batch_docs);
        if batch_docs.len() >= BATCH {
            flush(coll, &mut batch_docs, &mut total_copied);
        }
    }

    let malformed = parser.malformed_lines;
    let items = parser
        .finish()
        .map_err(|e| DbError::InternalError(format!("Export stream rejected: {}", e)))?;
    apply(items, coll, &mut batch_docs);
    flush(coll, &mut batch_docs, &mut total_copied);

    if malformed > 0 {
        tracing::warn!("HEAL: skipped {} malformed export line(s)", malformed);
    }
    Ok(total_copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(chunks: &[&[u8]]) -> Vec<ExportItem> {
        let mut p = ExportStreamParser::new(1024);
        let mut out = Vec::new();
        for c in chunks {
            out.extend(p.feed(c).unwrap());
        }
        out.extend(p.finish().unwrap());
        out
    }

    #[test]
    fn a_multibyte_character_split_across_chunks_survives() {
        // "é" is 0xC3 0xA9; split it between two network chunks.
        let line = "{\"_key\":\"k\",\"v\":\"caf\u{e9}\"}\n".as_bytes();
        let split = line.iter().position(|b| *b == 0xC3).unwrap() + 1;
        let out = feed_all(&[&line[..split], &line[split..]]);
        assert_eq!(
            out,
            vec![ExportItem::Doc(
                serde_json::json!({"_key": "k", "v": "café"})
            )]
        );
    }

    #[test]
    fn a_final_line_without_newline_is_decoded() {
        let out = feed_all(&[b"{\"_key\":\"a\"}\n{\"_key\"", b":\"b\"}"]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn an_endless_line_is_refused() {
        let mut p = ExportStreamParser::new(16);
        assert!(p.feed(&[b'x'; 17]).is_err());
    }

    #[test]
    fn raw_blob_chunks_are_read_by_length() {
        let header = b"{\"_type\":\"blob_chunk\",\"_doc_key\":\"b\",\"_chunk_index\":0,\"_data_length\":4}\n";
        let mut stream = header.to_vec();
        stream.extend_from_slice(&[b'\n', 0xff, 0x00, b'\n']); // raw bytes include newlines
        stream.push(b'\n');
        stream.extend_from_slice(b"{\"_key\":\"b\"}\n");
        // Deliver one byte at a time to exercise every boundary.
        let chunks: Vec<&[u8]> = stream.chunks(1).collect();
        let out = feed_all(&chunks);
        assert_eq!(
            out,
            vec![
                ExportItem::BlobChunk {
                    key: "b".into(),
                    index: 0,
                    data: vec![b'\n', 0xff, 0x00, b'\n']
                },
                ExportItem::Doc(serde_json::json!({"_key": "b"})),
            ]
        );
    }

    #[test]
    fn a_stream_cut_inside_a_blob_chunk_is_an_error() {
        let mut p = ExportStreamParser::new(1024);
        p.feed(b"{\"_type\":\"blob_chunk\",\"_doc_key\":\"b\",\"_chunk_index\":0,\"_data_length\":8}\nab")
            .unwrap();
        assert!(p.finish().is_err());
    }
}
