//! A collection exported as Parquet or as an Arrow IPC stream (card #980),
//! for the tools that read training data that way: pyarrow, pandas,
//! Polars, DuckDB, PyTorch loaders, LanceDB.
//!
//! **Columns.** A columnar collection has a schema: its columns, after the
//! row id (`_id`), with their types. A document collection has none: its
//! documents are read twice, once to find each top-level field's type
//! ([`Kind`]), once to write them. `_id` and `_rev` are left out (`_key`
//! names a document); a field of a vector index is a fixed-size list of
//! `float32`, which readers take as a tensor; a field that holds values of
//! several kinds, or objects, is its JSON text. Every column is nullable.
//!
//! Rows are written in batches of [`BATCH`]; nothing holds more than one
//! batch, so an export is streamed whatever the collection's size.

use super::collection::Collection;
use super::columnar::{ColumnType, ColumnarCollection};
use crate::error::{DbError, DbResult};
use arrow_array::builder::{
    BooleanBuilder, Float32Builder, Float64Builder, Int64Builder, ListBuilder, StringBuilder,
    TimestampMillisecondBuilder,
};
use arrow_array::{ArrayRef, FixedSizeListArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use serde_json::{Map, Value};
use std::io::Write;
use std::sync::Arc;

/// Rows per record batch (and per Parquet row group at most).
const BATCH: usize = 8_192;

/// Field metadata of a column that holds JSON text (values of several
/// kinds, or objects): `"true"`.
pub const JSON_FIELD: &str = "solidb:json";

/// How a collection is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// One Parquet file, zstd-compressed.
    Parquet,
    /// An Arrow IPC stream: record batches as they come.
    Arrow,
}

impl ExportFormat {
    /// `parquet` or `arrow`; `None` for anything else.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "parquet" => Some(Self::Parquet),
            "arrow" | "arrows" | "ipc" => Some(Self::Arrow),
            _ => None,
        }
    }

    /// The media type of the response.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Parquet => "application/vnd.apache.parquet",
            Self::Arrow => "application/vnd.apache.arrow.stream",
        }
    }

    /// The file name's extension.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Parquet => "parquet",
            Self::Arrow => "arrows",
        }
    }
}

/// What a column holds.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Bool,
    Int,
    Float,
    Text,
    /// Any other value, as its JSON text.
    Json,
    /// A list of numbers, as `float64`.
    Floats,
    /// A vector of a vector index: `dim` × `float32`.
    Vector(i32),
    /// Milliseconds since the epoch, UTC.
    Timestamp,
}

impl Kind {
    fn data_type(self) -> DataType {
        match self {
            Kind::Bool => DataType::Boolean,
            Kind::Int => DataType::Int64,
            Kind::Float => DataType::Float64,
            Kind::Text | Kind::Json => DataType::Utf8,
            Kind::Floats => {
                DataType::List(Arc::new(Field::new_list_field(DataType::Float64, true)))
            }
            Kind::Vector(dim) => DataType::FixedSizeList(
                Arc::new(Field::new_list_field(DataType::Float32, true)),
                dim,
            ),
            Kind::Timestamp => DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
        }
    }
}

/// What the first pass saw of a field of a document collection.
#[derive(Default)]
struct Seen {
    bools: bool,
    ints: bool,
    floats: bool,
    texts: bool,
    number_lists: bool,
    others: bool,
}

impl Seen {
    fn note(&mut self, value: &Value) {
        match value {
            Value::Null => {}
            Value::Bool(_) => self.bools = true,
            Value::Number(n) if n.is_i64() || n.is_u64() && n.as_i64().is_some() => {
                self.ints = true
            }
            Value::Number(_) => self.floats = true,
            Value::String(_) => self.texts = true,
            Value::Array(items) if items.iter().all(Value::is_number) => self.number_lists = true,
            _ => self.others = true,
        }
    }

    fn kind(&self) -> Kind {
        let kinds = [
            self.bools,
            self.ints || self.floats,
            self.texts,
            self.number_lists,
            self.others,
        ]
        .iter()
        .filter(|seen| **seen)
        .count();
        if kinds > 1 || self.others {
            return Kind::Json;
        }
        if self.bools {
            Kind::Bool
        } else if self.floats {
            Kind::Float
        } else if self.ints {
            Kind::Int
        } else if self.number_lists {
            Kind::Floats
        } else {
            Kind::Text
        }
    }
}

/// The columns being written.
struct Columns {
    names: Vec<String>,
    kinds: Vec<Kind>,
    schema: Arc<Schema>,
}

impl Columns {
    fn new(columns: Vec<(String, Kind)>) -> Self {
        let schema = Arc::new(Schema::new(
            columns
                .iter()
                .map(|(name, kind)| {
                    let field = Field::new(name, kind.data_type(), true);
                    // A column of JSON text says so, for an import to read
                    // the values back rather than their text (card #1004).
                    if *kind == Kind::Json {
                        field.with_metadata(std::collections::HashMap::from([(
                            JSON_FIELD.to_string(),
                            "true".to_string(),
                        )]))
                    } else {
                        field
                    }
                })
                .collect::<Vec<_>>(),
        ));
        let (names, kinds) = columns.into_iter().unzip();
        Columns {
            names,
            kinds,
            schema,
        }
    }

    /// One record batch of `rows`.
    fn batch(&self, rows: &[Map<String, Value>]) -> DbResult<RecordBatch> {
        let arrays: Vec<ArrayRef> = self
            .names
            .iter()
            .zip(&self.kinds)
            .map(|(name, kind)| column(rows, name, *kind))
            .collect::<DbResult<_>>()?;
        RecordBatch::try_new(self.schema.clone(), arrays).map_err(failed)
    }
}

fn failed(e: impl std::fmt::Display) -> DbError {
    DbError::InternalError(format!("export: {e}"))
}

/// The milliseconds a timestamp value names, as the SDBQL date functions
/// read it.
fn instant(value: &Value) -> Option<i64> {
    use crate::sdbql::executor::utils::{number_to_epoch_ms, parse_date_str};
    match value {
        Value::Number(n) => number_to_epoch_ms(n).ok(),
        Value::String(s) => parse_date_str(s).map(|d| d.to_utc().timestamp_millis()),
        _ => None,
    }
}

/// The array of the column `name` over `rows`.
fn column(rows: &[Map<String, Value>], name: &str, kind: Kind) -> DbResult<ArrayRef> {
    let values = rows
        .iter()
        .map(|row| row.get(name).filter(|v| !v.is_null()));
    Ok(match kind {
        Kind::Bool => {
            let mut b = BooleanBuilder::with_capacity(rows.len());
            values.for_each(|v| b.append_option(v.and_then(Value::as_bool)));
            Arc::new(b.finish())
        }
        Kind::Int => {
            let mut b = Int64Builder::with_capacity(rows.len());
            values.for_each(|v| b.append_option(v.and_then(Value::as_i64)));
            Arc::new(b.finish())
        }
        Kind::Float => {
            let mut b = Float64Builder::with_capacity(rows.len());
            values.for_each(|v| b.append_option(v.and_then(Value::as_f64)));
            Arc::new(b.finish())
        }
        Kind::Text => {
            let mut b = StringBuilder::with_capacity(rows.len(), rows.len() * 8);
            values.for_each(|v| match v {
                Some(Value::String(s)) => b.append_value(s),
                Some(other) => b.append_value(other.to_string()),
                None => b.append_null(),
            });
            Arc::new(b.finish())
        }
        Kind::Json => {
            let mut b = StringBuilder::with_capacity(rows.len(), rows.len() * 16);
            values.for_each(|v| match v {
                Some(v) => b.append_value(v.to_string()),
                None => b.append_null(),
            });
            Arc::new(b.finish())
        }
        Kind::Timestamp => {
            let mut b = TimestampMillisecondBuilder::with_capacity(rows.len()).with_timezone("UTC");
            values.for_each(|v| b.append_option(v.and_then(instant)));
            Arc::new(b.finish())
        }
        Kind::Floats => {
            let mut b = ListBuilder::new(Float64Builder::new());
            values.for_each(|v| match v.and_then(Value::as_array) {
                Some(items) => {
                    for item in items {
                        b.values().append_option(item.as_f64());
                    }
                    b.append(true);
                }
                None => b.append(false),
            });
            Arc::new(b.finish())
        }
        Kind::Vector(dim) => {
            let width = dim as usize;
            let mut floats = Float32Builder::with_capacity(rows.len() * width);
            let mut present = Vec::with_capacity(rows.len());
            for v in values {
                let vector = v
                    .and_then(Value::as_array)
                    .filter(|items| items.len() == width && items.iter().all(Value::is_number));
                match vector {
                    Some(items) => {
                        for item in items {
                            floats.append_value(item.as_f64().unwrap_or_default() as f32);
                        }
                        present.push(true);
                    }
                    None => {
                        floats.append_value_n(0.0, width);
                        present.push(false);
                    }
                }
            }
            let field = Arc::new(Field::new_list_field(DataType::Float32, true));
            Arc::new(
                FixedSizeListArray::try_new(
                    field,
                    dim,
                    Arc::new(floats.finish()),
                    Some(present.into()),
                )
                .map_err(failed)?,
            )
        }
    })
}

/// Where batches go.
enum Sink<W: Write + Send> {
    Parquet(parquet::arrow::ArrowWriter<W>),
    Arrow(arrow_ipc::writer::StreamWriter<W>),
}

impl<W: Write + Send> Sink<W> {
    fn new(format: ExportFormat, out: W, schema: &Arc<Schema>) -> DbResult<Self> {
        Ok(match format {
            ExportFormat::Parquet => {
                use parquet::basic::{Compression, ZstdLevel};
                use parquet::file::properties::WriterProperties;
                let props = WriterProperties::builder()
                    .set_compression(Compression::ZSTD(ZstdLevel::default()))
                    .set_max_row_group_row_count(Some(BATCH * 16))
                    .build();
                Sink::Parquet(
                    parquet::arrow::ArrowWriter::try_new(out, schema.clone(), Some(props))
                        .map_err(failed)?,
                )
            }
            ExportFormat::Arrow => {
                Sink::Arrow(arrow_ipc::writer::StreamWriter::try_new(out, schema).map_err(failed)?)
            }
        })
    }

    fn write(&mut self, batch: &RecordBatch) -> DbResult<()> {
        match self {
            Sink::Parquet(w) => w.write(batch).map_err(failed),
            Sink::Arrow(w) => w.write(batch).map_err(failed),
        }
    }

    fn finish(self) -> DbResult<()> {
        match self {
            Sink::Parquet(w) => w.close().map(|_| ()).map_err(failed),
            Sink::Arrow(mut w) => w.finish().map_err(failed),
        }
    }
}

/// Write rows as they come, a batch at a time.
struct Writer<W: Write + Send> {
    columns: Columns,
    sink: Sink<W>,
    pending: Vec<Map<String, Value>>,
    rows: u64,
}

impl<W: Write + Send> Writer<W> {
    fn new(columns: Columns, format: ExportFormat, out: W) -> DbResult<Self> {
        let sink = Sink::new(format, out, &columns.schema)?;
        Ok(Writer {
            columns,
            sink,
            pending: Vec::with_capacity(BATCH),
            rows: 0,
        })
    }

    fn push(&mut self, row: Map<String, Value>) -> DbResult<()> {
        self.pending.push(row);
        if self.pending.len() >= BATCH {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> DbResult<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let batch = self.columns.batch(&self.pending)?;
        self.sink.write(&batch)?;
        self.rows += self.pending.len() as u64;
        self.pending.clear();
        Ok(())
    }

    fn finish(mut self) -> DbResult<u64> {
        self.flush()?;
        self.sink.finish()?;
        Ok(self.rows)
    }
}

/// Fields of a document left out of an export: `_key` names it.
fn left_out(field: &str) -> bool {
    matches!(field, "_id" | "_rev")
}

/// Export the documents of `collection` to `out`. Returns the rows written.
pub fn export_collection(
    collection: &Collection,
    format: ExportFormat,
    out: impl Write + Send,
) -> DbResult<u64> {
    // First pass: the fields, in the order they are first met, and what
    // they hold.
    let vectors: std::collections::HashMap<String, i32> = collection
        .get_all_vector_index_configs()
        .into_iter()
        .filter(|c| !c.field.contains('.'))
        .map(|c| (c.field, c.dimension as i32))
        .collect();
    let mut order: Vec<String> = vec!["_key".to_string()];
    let mut seen: std::collections::HashMap<String, Seen> = std::collections::HashMap::new();
    collection.for_each_value(None, |doc| {
        if let Value::Object(fields) = doc {
            for (name, value) in &fields {
                if left_out(name) {
                    continue;
                }
                let entry = seen.entry(name.clone()).or_insert_with(|| {
                    if name != "_key" {
                        order.push(name.clone());
                    }
                    Seen::default()
                });
                entry.note(value);
            }
        }
        Ok(())
    })?;
    let columns = Columns::new(
        order
            .into_iter()
            .map(|name| {
                let kind = match vectors.get(&name) {
                    Some(dim) => Kind::Vector(*dim),
                    None => match seen.get(&name).map_or(Kind::Text, Seen::kind) {
                        // The instants the server keeps, as timestamps.
                        Kind::Text if matches!(name.as_str(), "_created_at" | "_updated_at") => {
                            Kind::Timestamp
                        }
                        kind => kind,
                    },
                };
                (name, kind)
            })
            .collect(),
    );
    // Second pass: the rows.
    let mut writer = Writer::new(columns, format, out)?;
    collection.for_each_value(None, |doc| match doc {
        Value::Object(fields) => writer.push(fields),
        _ => Ok(()),
    })?;
    writer.finish()
}

/// Export the rows of the columnar collection `columnar` to `out`, the row
/// id first. Returns the rows written.
pub fn export_columnar(
    columnar: &ColumnarCollection,
    format: ExportFormat,
    out: impl Write + Send,
) -> DbResult<u64> {
    let meta = columnar.metadata()?;
    let mut columns = vec![("_id".to_string(), Kind::Text)];
    for def in &meta.columns {
        let kind = match def.data_type {
            ColumnType::Int64 => Kind::Int,
            ColumnType::Float64 => Kind::Float,
            ColumnType::String => Kind::Text,
            ColumnType::Bool => Kind::Bool,
            ColumnType::Timestamp => Kind::Timestamp,
            ColumnType::Json => Kind::Json,
        };
        columns.push((def.name.clone(), kind));
    }
    let mut writer = Writer::new(Columns::new(columns), format, out)?;
    columnar.for_each_blocks(4, None, |blocks| {
        for block in blocks {
            let ids = block.uuids();
            for at in 0..block.len() {
                let mut row = block.row(at);
                row.insert("_id".to_string(), Value::String(ids.text(at)));
                writer.push(row)?;
            }
        }
        Ok(())
    })?;
    for (id, mut row) in columnar.tail_rows()? {
        row.insert("_id".to_string(), Value::String(id));
        writer.push(row)?;
    }
    writer.finish()
}
