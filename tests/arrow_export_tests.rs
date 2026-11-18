//! Collections exported as Parquet and Arrow (card #980), read back with
//! the Arrow readers: the columns have the types a reader expects, and the
//! values are those stored.
#![cfg(feature = "export-arrow")]

use arrow_array::cast::AsArray;
use arrow_array::types::{Float32Type, Float64Type, Int64Type, TimestampMillisecondType};
use arrow_array::{Array, RecordBatch};
use arrow_schema::{DataType, TimeUnit};
use serde_json::json;
use solidb::storage::arrow_export::{export_collection, export_columnar, ExportFormat};
use solidb::storage::{VectorIndexConfig, VectorMetric};
use solidb::sync::columnar;
use solidb::{ColumnDef, ColumnType, CompressionType, StorageEngine};
use tempfile::TempDir;

fn engine() -> (StorageEngine, TempDir) {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(dir.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    (engine, dir)
}

/// The batches of a Parquet file, and of an Arrow stream, of the same rows.
fn read_both(write: impl Fn(ExportFormat, &mut Vec<u8>) -> u64) -> (Vec<RecordBatch>, u64) {
    let mut parquet = Vec::new();
    let rows = write(ExportFormat::Parquet, &mut parquet);
    let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(
        bytes::Bytes::from(parquet),
    )
    .unwrap()
    .build()
    .unwrap();
    let from_parquet: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
    let mut arrow = Vec::new();
    assert_eq!(write(ExportFormat::Arrow, &mut arrow), rows);
    let from_arrow: Vec<RecordBatch> =
        arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(arrow), None)
            .unwrap()
            .map(|b| b.unwrap())
            .collect();
    let all = |batches: &[RecordBatch]| {
        arrow_select::concat::concat_batches(&batches[0].schema(), batches).unwrap()
    };
    assert_eq!(
        all(&from_parquet),
        all(&from_arrow),
        "Parquet and Arrow differ"
    );
    (from_parquet, rows)
}

fn one(batches: Vec<RecordBatch>) -> RecordBatch {
    arrow_select::concat::concat_batches(&batches[0].schema(), &batches).unwrap()
}

#[test]
fn a_document_collection_with_vectors_reads_back_typed() {
    let (engine, _dir) = engine();
    let db = engine.get_database("app").unwrap();
    db.create_collection("docs".to_string(), None).unwrap();
    let docs = db.get_collection("docs").unwrap();
    let n = 20_000; // more than two batches
    let rows: Vec<_> = (0..n)
        .map(|i| {
            json!({
                "_key": format!("k{i:05}"),
                "n": i,
                "x": if i % 3 == 0 { json!(i as f64 / 2.0) } else { json!(i) },
                "ok": i % 2 == 0,
                "name": format!("doc {i}"),
                "maybe": if i % 5 == 0 { json!(null) } else { json!("here") },
                "mixed": if i % 2 == 0 { json!(1) } else { json!("one") },
                "tags": ["a", "b"],
                "scores": [1.5, i],
                "emb": [i as f64, 1.0, 0.5, 0.25],
            })
        })
        .collect();
    docs.insert_batch(rows).unwrap();
    docs.create_vector_index(
        VectorIndexConfig::new("v".to_string(), "emb".to_string(), 4)
            .with_metric(VectorMetric::Cosine),
    )
    .unwrap();

    let (batches, written) =
        read_both(|format, out| export_collection(&docs, format, out).unwrap());
    assert_eq!(written, n as u64);
    let all = one(batches);
    assert_eq!(all.num_rows(), n);
    let schema = all.schema();
    let kind = |name: &str| schema.field_with_name(name).unwrap().data_type().clone();
    assert_eq!(schema.field(0).name(), "_key");
    assert!(schema.field_with_name("_id").is_err() && schema.field_with_name("_rev").is_err());
    assert_eq!(
        kind("_created_at"),
        DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
    );
    assert_eq!(all.column_by_name("_created_at").unwrap().null_count(), 0);
    assert_eq!(kind("n"), DataType::Int64);
    assert_eq!(kind("x"), DataType::Float64);
    assert_eq!(kind("ok"), DataType::Boolean);
    assert_eq!(kind("name"), DataType::Utf8);
    assert_eq!(kind("maybe"), DataType::Utf8);
    assert_eq!(
        kind("mixed"),
        DataType::Utf8,
        "values of several kinds are JSON text"
    );
    assert_eq!(
        kind("tags"),
        DataType::Utf8,
        "a list of strings is JSON text"
    );
    assert!(matches!(kind("scores"), DataType::List(_)));
    match kind("emb") {
        DataType::FixedSizeList(item, 4) => assert_eq!(item.data_type(), &DataType::Float32),
        other => panic!("emb is {other:?}"),
    }

    // Values, row by row against the documents.
    let keys = all.column_by_name("_key").unwrap().as_string::<i32>();
    let n_col = all.column_by_name("n").unwrap().as_primitive::<Int64Type>();
    let x_col = all
        .column_by_name("x")
        .unwrap()
        .as_primitive::<Float64Type>();
    let maybe = all.column_by_name("maybe").unwrap().as_string::<i32>();
    let mixed = all.column_by_name("mixed").unwrap().as_string::<i32>();
    let emb = all.column_by_name("emb").unwrap().as_fixed_size_list();
    for row in [0usize, 1, 3, 5, 9_999, 19_999] {
        let i = keys.value(row)[1..].parse::<usize>().unwrap();
        assert_eq!(n_col.value(row), i as i64);
        let x = if i % 3 == 0 { i as f64 / 2.0 } else { i as f64 };
        assert_eq!(x_col.value(row), x);
        assert_eq!(maybe.is_null(row), i % 5 == 0);
        assert_eq!(mixed.value(row), if i % 2 == 0 { "1" } else { "\"one\"" });
        let vector = emb.value(row);
        let vector = vector.as_primitive::<Float32Type>();
        assert_eq!(vector.values().to_vec(), vec![i as f32, 1.0, 0.5, 0.25]);
    }
}

#[test]
fn a_columnar_collection_reads_back_typed() {
    let (engine, _dir) = engine();
    let db = engine.get_database("app").unwrap();
    let column = |name: &str, data_type| ColumnDef {
        name: name.to_string(),
        data_type,
        nullable: true,
        indexed: false,
        index_type: None,
    };
    let col = columnar::create(
        &db,
        None,
        "m",
        vec![
            column("host", ColumnType::String),
            column("v", ColumnType::Int64),
            column("ms", ColumnType::Float64),
            column("ok", ColumnType::Bool),
            column("at", ColumnType::Timestamp),
        ],
        CompressionType::Lz4,
    )
    .unwrap();
    let rows =
        |from: i64, to: i64| -> Vec<serde_json::Value> {
            (from..to)
            .map(|i| {
                let at = if i % 2 == 0 {
                    json!("2026-11-18T10:00:00Z")
                } else {
                    json!(1_795_000_000_000i64)
                };
                let ms = if i % 4 == 0 { json!(null) } else { json!(i as f64 / 8.0) };
                json!({"host": format!("h{}", i % 7), "v": i, "ms": ms, "ok": i % 3 == 0, "at": at})
            })
            .collect()
        };
    // Two blocks and a tail.
    col.insert_rows(rows(0, 9_000)).unwrap();
    col.insert_rows(rows(9_000, 18_000)).unwrap();
    col.insert_rows(rows(18_000, 18_010)).unwrap();

    let (batches, written) = read_both(|format, out| export_columnar(&col, format, out).unwrap());
    assert_eq!(written, 18_010);
    let all = one(batches);
    let schema = all.schema();
    assert_eq!(schema.field(0).name(), "_id");
    let kind = |name: &str| schema.field_with_name(name).unwrap().data_type().clone();
    assert_eq!(kind("v"), DataType::Int64);
    assert_eq!(kind("ms"), DataType::Float64);
    assert_eq!(kind("ok"), DataType::Boolean);
    assert_eq!(
        kind("at"),
        DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
    );
    let v = all.column_by_name("v").unwrap().as_primitive::<Int64Type>();
    let ms = all
        .column_by_name("ms")
        .unwrap()
        .as_primitive::<Float64Type>();
    let at = all
        .column_by_name("at")
        .unwrap()
        .as_primitive::<TimestampMillisecondType>();
    let mut seen: Vec<i64> = v.values().to_vec();
    seen.sort_unstable();
    assert_eq!(seen, (0..18_010).collect::<Vec<_>>());
    for row in [0usize, 1, 4, 9_000, 18_009] {
        let i = v.value(row);
        assert_eq!(ms.is_null(row), i % 4 == 0);
        if i % 4 != 0 {
            assert_eq!(ms.value(row), i as f64 / 8.0);
        }
        let expected = if i % 2 == 0 {
            chrono::DateTime::parse_from_rfc3339("2026-11-18T10:00:00Z")
                .unwrap()
                .timestamp_millis()
        } else {
            1_795_000_000_000
        };
        assert_eq!(at.value(row), expected);
    }
}
