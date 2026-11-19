//! Parquet and Arrow IPC read into a collection (card #1004), the way the
//! export writes them (`storage/arrow_export.rs`) and the way pyarrow,
//! pandas, Polars, DuckDB and LanceDB write them.
//!
//! A row is a document: each column a field, a null left out. Numbers,
//! strings and booleans are themselves; a timestamp is its ISO-8601 text;
//! a list (fixed-size or not) is an array, so an embedding comes back as a
//! vector; a struct or a map is an object. A column the export marked as
//! JSON text ([`JSON_FIELD`]) is read back as the value it was.
//!
//! The file is read by record batches: nothing holds more than one.

use super::arrow_export::JSON_FIELD;
use crate::error::{DbError, DbResult};
use arrow_array::RecordBatch;
use serde_json::Value;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// How an imported file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Parquet,
    /// An Arrow IPC stream, or an Arrow IPC file.
    Arrow,
}

impl ImportFormat {
    /// `?format=` of an import.
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "parquet" => Some(ImportFormat::Parquet),
            "arrow" | "ipc" | "feather" => Some(ImportFormat::Arrow),
            _ => None,
        }
    }

    /// The format a file's first bytes say: `PAR1` for Parquet, `ARROW1`
    /// for an Arrow IPC file, the continuation marker `FF FF FF FF` for an
    /// Arrow IPC stream. `None` for anything else (JSON lines).
    pub fn sniff(head: &[u8]) -> Option<Self> {
        if head.starts_with(b"PAR1") {
            Some(ImportFormat::Parquet)
        } else if head.starts_with(b"ARROW1") || head.starts_with(&[0xFF, 0xFF, 0xFF, 0xFF]) {
            Some(ImportFormat::Arrow)
        } else {
            None
        }
    }
}

fn failed(e: impl std::fmt::Display) -> DbError {
    DbError::BadRequest(format!("import: {e}"))
}

/// The documents of `batch`, row by row.
pub fn documents(batch: &RecordBatch) -> DbResult<Vec<Value>> {
    let mut text = Vec::new();
    {
        let mut writer = arrow_json::LineDelimitedWriter::new(&mut text);
        writer.write(batch).map_err(failed)?;
        writer.finish().map_err(failed)?;
    }
    let json_columns: Vec<String> = batch
        .schema()
        .fields()
        .iter()
        .filter(|f| f.metadata().get(JSON_FIELD).map(String::as_str) == Some("true"))
        .map(|f| f.name().clone())
        .collect();
    let mut docs = Vec::with_capacity(batch.num_rows());
    for line in text.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
        let mut doc: Value = serde_json::from_slice(line).map_err(failed)?;
        if let Some(fields) = doc.as_object_mut() {
            for name in &json_columns {
                if let Some(Value::String(s)) = fields.get(name) {
                    if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                        fields.insert(name.clone(), parsed);
                    }
                }
            }
        }
        docs.push(doc);
    }
    Ok(docs)
}

/// Read the file at `path` batch by batch, handing `each` the documents of
/// each one. Returns the rows read.
pub fn read_file(
    path: &Path,
    format: ImportFormat,
    mut each: impl FnMut(Vec<Value>) -> DbResult<()>,
) -> DbResult<u64> {
    let mut rows = 0u64;
    let mut feed = |batch: RecordBatch| -> DbResult<()> {
        rows += batch.num_rows() as u64;
        each(documents(&batch)?)
    };
    match format {
        ImportFormat::Parquet => {
            let file = File::open(path).map_err(failed)?;
            let reader =
                parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
                    .map_err(failed)?
                    .with_batch_size(8_192)
                    .build()
                    .map_err(failed)?;
            for batch in reader {
                feed(batch.map_err(failed)?)?;
            }
        }
        ImportFormat::Arrow => {
            let mut file = File::open(path).map_err(failed)?;
            let mut magic = [0u8; 6];
            let is_file = file.read_exact(&mut magic).is_ok() && &magic == b"ARROW1";
            file.seek(SeekFrom::Start(0)).map_err(failed)?;
            if is_file {
                let reader = arrow_ipc::reader::FileReader::try_new(file, None).map_err(failed)?;
                for batch in reader {
                    feed(batch.map_err(failed)?)?;
                }
            } else {
                let reader = arrow_ipc::reader::StreamReader::try_new(BufReader::new(file), None)
                    .map_err(failed)?;
                for batch in reader {
                    feed(batch.map_err(failed)?)?;
                }
            }
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::ImportFormat;

    #[test]
    fn a_file_says_its_format() {
        assert_eq!(
            ImportFormat::sniff(b"PAR1\x15\x00"),
            Some(ImportFormat::Parquet)
        );
        assert_eq!(
            ImportFormat::sniff(b"ARROW1\x00\x00"),
            Some(ImportFormat::Arrow)
        );
        assert_eq!(
            ImportFormat::sniff(&[0xFF, 0xFF, 0xFF, 0xFF, 8, 0]),
            Some(ImportFormat::Arrow)
        );
        assert_eq!(ImportFormat::sniff(b"{\"a\": 1}"), None);
        assert_eq!(ImportFormat::parse("Parquet"), Some(ImportFormat::Parquet));
        assert_eq!(ImportFormat::parse("csv"), None);
    }
}
