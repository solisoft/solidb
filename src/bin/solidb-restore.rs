use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicU64, Ordering};

/// Records that named no target database/collection and could not be routed.
static SKIPPED_UNROUTABLE: AtomicU64 = AtomicU64::new(0);

/// Rows per POST to the columnar insert endpoint.
const MAX_COLUMNAR_ROWS_PER_INSERT: usize = 10_000;

/// Resolve the target database and collection for a record.
///
/// CLI flags are overrides: they win over the names embedded in the dump.
/// Returns `None` when neither the record nor the flags name a target.
fn resolve_target(record: &Value, args: &Args) -> Option<(String, String)> {
    let db = args.database.clone().or_else(|| {
        record
            .get("_database")
            .and_then(|s| s.as_str())
            .map(String::from)
    })?;
    let coll = args.collection.clone().or_else(|| {
        record
            .get("_collection")
            .and_then(|s| s.as_str())
            .map(String::from)
    })?;
    Some((db, coll))
}

/// Classify a dump record as one of the control records solidb-dump emits
/// (`collection`, `index`, `columnar`, `columnar_row`, `columnar_index`,
/// `blob_chunk`) rather than a document to import.
///
/// A stored document always carries a `_key`, and no control record ever does.
/// Requiring that keeps a user document that happens to have a field named
/// `_type` from being mistaken for a control record and silently dropped.
fn control_record_type(record: &Value) -> Option<String> {
    if record.get("_key").is_some() {
        return None;
    }
    let t = record.get("_type")?.as_str()?;
    matches!(
        t,
        "collection" | "index" | "columnar" | "columnar_row" | "columnar_index" | "blob_chunk"
    )
    .then(|| t.to_string())
}

/// Report a record that carries no routing metadata.
///
/// Dumps produced before the blob-metadata fix contain blob records with no
/// `_database`/`_collection`, because solidb-dump streamed the server's
/// single-collection `/export` output verbatim. Skipping is strictly better
/// than the previous behaviour (abort the entire restore) and than guessing a
/// target, which would silently file the records under the wrong collection.
fn note_unroutable() {
    let seen = SKIPPED_UNROUTABLE.fetch_add(1, Ordering::Relaxed);
    if seen == 0 {
        eprintln!(
            "  {} record with no _database/_collection — skipping. This dump \
             predates the blob metadata fix; re-run solidb-dump with an updated \
             binary, or restore that collection on its own with -d/-c.",
            "Warning:".yellow().bold()
        );
    }
}

#[derive(Parser, Debug)]
#[command(name = "solidb-restore")]
#[command(about = "Import SoliDB database or collection from dump. Supports JSONL, JSON Array, CSV, and SQL formats.", long_about = None)]
struct Args {
    /// Database host
    #[arg(short = 'H', long, default_value = "localhost")]
    host: String,

    /// Database port
    #[arg(short = 'P', long, default_value = "6745")]
    port: u16,

    /// Input file (JSONL, JSON Array, CSV, or SQL)
    #[arg(short, long)]
    input: String,

    /// Override database name (use instead of name in dump)
    #[arg(short = 'd', long)]
    database: Option<String>,

    /// Override collection name (only when restoring single collection)
    #[arg(short = 'c', long)]
    collection: Option<String>,

    /// Create database if it doesn't exist
    #[arg(long)]
    create_database: bool,

    /// Drop existing collections before restore
    #[arg(long)]
    drop: bool,

    /// Username for authentication
    #[arg(short = 'u', long)]
    user: Option<String>,

    /// Password for authentication
    #[arg(short = 'p', long)]
    password: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let base_url = format!("http://{}:{}", args.host, args.port);

    // Authentication
    let token = if let (Some(user), Some(password)) = (&args.user, &args.password) {
        let login_url = format!("{}/auth/login", base_url);
        let client = reqwest::Client::new();
        eprintln!("Authenticating as user: {}", user);

        let response = client
            .post(&login_url)
            .json(&serde_json::json!({
                "username": user,
                "password": password
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!("Authentication failed: {}", response.status()).into());
        }

        let login_data: Value = response.json().await?;
        if let Some(token) = login_data["token"].as_str() {
            Some(token.to_string())
        } else {
            return Err("Authentication response missing token".into());
        }
    } else {
        None
    };

    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(t) = token {
        let mut auth_val = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", t))?;
        auth_val.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, auth_val);
    }

    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;

    // Read Input file
    let file = File::open(&args.input)?;
    let metadata = file.metadata()?;
    let total_size = metadata.len();

    let pb = ProgressBar::new(total_size);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    let mut reader = BufReader::new(pb.wrap_read(file));

    // Peek to detect format
    // JSON Array: Starts with '['
    // JSONL: Starts with '{'
    // CSV: Anything else (assume header row)
    let mut format = "csv"; // default

    // Check extension first
    if args.input.to_lowercase().ends_with(".sql") {
        format = "sql";
    }

    if format == "csv" {
        // Check start of file for partial content to guess format
        let buf = reader.fill_buf()?;
        for &byte in buf {
            if !byte.is_ascii_whitespace() {
                if byte == b'[' {
                    format = "json_array";
                } else if byte == b'{' {
                    format = "jsonl";
                } else {
                    // Check for SQL INSERT
                    // precise check to avoid confusing CSV header "Id" with SQL
                    let start_idx = buf
                        .iter()
                        .position(|&b| !b.is_ascii_whitespace())
                        .unwrap_or(0);
                    if buf.len() >= start_idx + 6 {
                        let potential_insert = &buf[start_idx..start_idx + 6];
                        if potential_insert.eq_ignore_ascii_case(b"INSERT") {
                            format = "sql";
                        }
                    }
                }
                break;
            }
        }
    }

    // Use Vec<u8> to avoid re-serialization
    let mut current_batch: Vec<Vec<u8>> = Vec::new();
    let mut current_batch_size = 0;
    let mut current_batch_meta: Option<(String, String)> = None;
    let max_batch_count = 20000;
    let max_batch_size = 25 * 1024 * 1024; // 25MB

    // We need to track collections to create them first?
    // If we stream, we might encounter a doc for Collection A, then B, then A.
    // But solidb-dump groups by collection.
    // However, to be robust, we should create on the fly or pre-scan?
    // Pre-scanning a huge file is bad.
    // Solution: "Upsert" collection logic or just try to create when we see a new collection name?
    // We can keep a set of "initialized collections".

    let mut initialized_collections: HashMap<String, bool> = HashMap::new();
    let mut columnar_batch = ColumnarBatch::default();
    let mut total_imported = 0;
    let mut total_failed = 0;

    // We assume JSONL for streaming restore of dumps
    // For other formats (which were loaded fully before), we can just fail or support strictly JSONL for big dumps
    // The previous code supported CSV/SQL/JSONArray by loading ALL.
    // Let's implement streaming for JSONL, and keep full-load for others?
    // But the variable `all_documents` is gone now if we stream.
    // Let's simplify: Only JSONL supports streaming. A Blob dump IS JSONL.

    // Check format first
    // Note: format variable was already set by detection logic above (lines 110-133)

    if format == "csv" {
        let mut csv_reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_reader(reader);

        for result in csv_reader.deserialize() {
            let record: HashMap<String, Value> = match result {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Failed to parse CSV record: {}", e);
                    total_failed += 1;
                    continue;
                }
            };

            let doc = serde_json::to_value(record)?;
            process_doc(
                doc,
                &args,
                &client,
                &base_url,
                &mut current_batch,
                &mut current_batch_size,
                &mut current_batch_meta,
                max_batch_count,
                max_batch_size,
                &mut initialized_collections,
                &mut total_imported,
                &mut total_failed,
            )
            .await?;
        }
    } else if format == "sql" {
        eprintln!(
            "Error: SQL restore is not yet fully implemented. Please convert to CSV or JSONL."
        );
        return Ok(());
    } else if format == "json_array" {
        eprintln!("Warning: JSON Array format loads all data into memory. Not recommended for large restores.");
        let all_documents: Vec<Value> = serde_json::from_reader(reader)?;
        for doc in all_documents {
            process_doc(
                doc,
                &args,
                &client,
                &base_url,
                &mut current_batch,
                &mut current_batch_size,
                &mut current_batch_meta,
                max_batch_count,
                max_batch_size,
                &mut initialized_collections,
                &mut total_imported,
                &mut total_failed,
            )
            .await?;
        }
    } else {
        // Assume JSONL
        eprintln!("Restoring using streaming mode (JSONL/Mixed)...");
        let mut buffer = Vec::new();

        loop {
            let bytes_read = reader.read_until(b'\n', &mut buffer)?;
            if bytes_read == 0 {
                break;
            }

            // Check if line is empty or just whitespace
            let line_slice = if buffer.ends_with(b"\n") {
                &buffer[..buffer.len() - 1]
            } else {
                &buffer
            };
            if line_slice.iter().all(|b| b.is_ascii_whitespace()) {
                buffer.clear();
                continue;
            }

            // Try parse JSON
            match serde_json::from_slice::<Value>(line_slice) {
                Ok(doc) => {
                    let record_type = control_record_type(&doc);

                    // Columnar records use a separate API and a separate row
                    // buffer, so they are handled before anything else.
                    match record_type.as_deref() {
                        Some("columnar") => {
                            flush_columnar_batch(
                                &mut columnar_batch,
                                &client,
                                &base_url,
                                &mut total_imported,
                                &mut total_failed,
                            )
                            .await?;
                            process_columnar_record(
                                doc,
                                &args,
                                &client,
                                &base_url,
                                &mut initialized_collections,
                                &mut total_failed,
                            )
                            .await?;
                            buffer.clear();
                            continue;
                        }
                        Some("columnar_index") => {
                            process_columnar_index_record(
                                doc,
                                &args,
                                &client,
                                &base_url,
                                &mut total_imported,
                                &mut total_failed,
                            )
                            .await?;
                            buffer.clear();
                            continue;
                        }
                        Some("columnar_row") => {
                            let Some(target) = resolve_target(&doc, &args) else {
                                note_unroutable();
                                buffer.clear();
                                continue;
                            };
                            if columnar_batch.target.as_ref() != Some(&target) {
                                flush_columnar_batch(
                                    &mut columnar_batch,
                                    &client,
                                    &base_url,
                                    &mut total_imported,
                                    &mut total_failed,
                                )
                                .await?;
                                columnar_batch.target = Some(target);
                            }
                            if let Some(row) = doc.get("row") {
                                columnar_batch.rows.push(row.clone());
                            }
                            if columnar_batch.rows.len() >= MAX_COLUMNAR_ROWS_PER_INSERT {
                                flush_columnar_batch(
                                    &mut columnar_batch,
                                    &client,
                                    &base_url,
                                    &mut total_imported,
                                    &mut total_failed,
                                )
                                .await?;
                            }
                            buffer.clear();
                            continue;
                        }
                        _ => {}
                    }

                    // Collection declaration: creates the collection with its
                    // real type up front, so empty collections survive the
                    // round trip and edge/blob/timeseries collections do not
                    // come back as plain document collections.
                    if record_type.as_deref() == Some("collection") {
                        if let Some((db, coll)) = current_batch_meta.clone() {
                            flush_batch(
                                &mut current_batch,
                                &mut current_batch_size,
                                &client,
                                &base_url,
                                &db,
                                &coll,
                                &mut total_imported,
                                &mut total_failed,
                            )
                            .await?;
                            current_batch_meta = None;
                        }
                        process_collection_record(
                            doc,
                            &args,
                            &client,
                            &base_url,
                            &mut initialized_collections,
                        )
                        .await?;
                        buffer.clear();
                        continue;
                    }

                    // Index definition record: must be applied after the
                    // collection exists but before mass-imports finish so
                    // newly inserted documents are indexed as they arrive.
                    // Flush the current batch first to keep ordering sane.
                    if record_type.as_deref() == Some("index") {
                        if let Some((db, coll)) = current_batch_meta.clone() {
                            flush_batch(
                                &mut current_batch,
                                &mut current_batch_size,
                                &client,
                                &base_url,
                                &db,
                                &coll,
                                &mut total_imported,
                                &mut total_failed,
                            )
                            .await?;
                        }
                        process_index_record(
                            doc,
                            &args,
                            &client,
                            &base_url,
                            &mut initialized_collections,
                            &mut total_imported,
                            &mut total_failed,
                        )
                        .await?;
                        buffer.clear();
                        continue;
                    }

                    // Check for Blob Chunk Header
                    let is_blob_chunk = record_type.as_deref() == Some("blob_chunk");

                    if is_blob_chunk {
                        if let Some(data_len) = doc.get("_data_length").and_then(|v| v.as_u64()) {
                            // It is a binary chunk header.
                            // 1. Process header (create db/coll, add to batch)
                            // We treat the header as a "doc" but we need to handle the data following it immediately.

                            // The header is followed by exactly `_data_length`
                            // raw bytes and a newline delimiter.
                            let mut data_buffer = vec![0u8; data_len as usize];
                            reader.read_exact(&mut data_buffer).map_err(|e| {
                                format!(
                                    "Truncated dump: expected {} bytes of blob data for key '{}' \
                                     chunk {}: {}",
                                    data_len,
                                    doc.get("_doc_key").and_then(|v| v.as_str()).unwrap_or("?"),
                                    doc.get("_chunk_index")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0),
                                    e
                                )
                            })?;

                            // Consume the trailing newline. If it is not a
                            // newline the stream is misframed and every
                            // following record would be garbage, so stop
                            // rather than silently corrupt the restore.
                            let mut newline_buf = [0u8; 1];
                            reader.read_exact(&mut newline_buf)?;
                            if newline_buf[0] != b'\n' {
                                return Err(format!(
                                    "Misframed dump: expected newline after {} bytes of blob data \
                                     for key '{}' chunk {}, found byte 0x{:02x}",
                                    data_len,
                                    doc.get("_doc_key").and_then(|v| v.as_str()).unwrap_or("?"),
                                    doc.get("_chunk_index")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0),
                                    newline_buf[0]
                                )
                                .into());
                            }

                            process_blob_chunk(
                                doc,
                                data_buffer,
                                &args,
                                &client,
                                &base_url,
                                &mut current_batch,
                                &mut current_batch_size,
                                &mut current_batch_meta,
                                max_batch_count,
                                max_batch_size,
                                &mut initialized_collections,
                                &mut total_imported,
                                &mut total_failed,
                            )
                            .await?;
                        } else {
                            // Legacy chunk
                            process_doc(
                                doc,
                                &args,
                                &client,
                                &base_url,
                                &mut current_batch,
                                &mut current_batch_size,
                                &mut current_batch_meta,
                                max_batch_count,
                                max_batch_size,
                                &mut initialized_collections,
                                &mut total_imported,
                                &mut total_failed,
                            )
                            .await?;
                        }
                    } else {
                        process_doc(
                            doc,
                            &args,
                            &client,
                            &base_url,
                            &mut current_batch,
                            &mut current_batch_size,
                            &mut current_batch_meta,
                            max_batch_count,
                            max_batch_size,
                            &mut initialized_collections,
                            &mut total_imported,
                            &mut total_failed,
                        )
                        .await?;
                    }
                }
                Err(e) => {
                    eprintln!("Failed to parse line: {}", e);
                    total_failed += 1;
                }
            }
            buffer.clear();
        }
    }

    // Flush remaining
    if !current_batch.is_empty() {
        if let Some((db, coll)) = &current_batch_meta {
            flush_batch(
                &mut current_batch,
                &mut current_batch_size,
                &client,
                &base_url,
                db,
                coll,
                &mut total_imported,
                &mut total_failed,
            )
            .await?;
        }
    }
    flush_columnar_batch(
        &mut columnar_batch,
        &client,
        &base_url,
        &mut total_imported,
        &mut total_failed,
    )
    .await?;

    eprintln!("✓ Restore completed");
    eprintln!("  → {} items imported", total_imported.to_string().green());
    if total_failed > 0 {
        eprintln!("  → {} items failed", total_failed.to_string().red());
    }
    let skipped = SKIPPED_UNROUTABLE.load(Ordering::Relaxed);
    if skipped > 0 {
        eprintln!(
            "  → {} items skipped (no target collection in dump)",
            skipped.to_string().yellow()
        );
    }

    Ok(())
}

use std::io::Read; // Needed for read_exact

/// Buffered rows for a columnar collection, keyed by "{db}/{collection}".
///
/// Columnar rows go to their own insert endpoint with a different payload
/// shape than document imports, so they cannot ride along in the normal batch.
#[derive(Default)]
struct ColumnarBatch {
    target: Option<(String, String)>,
    rows: Vec<Value>,
}

/// Create a columnar collection from a `_type: "columnar"` record.
async fn process_columnar_record(
    record: Value,
    args: &Args,
    client: &reqwest::Client,
    base_url: &str,
    initialized_cols: &mut HashMap<String, bool>,
    total_failed: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((db, coll)) = resolve_target(&record, args) else {
        note_unroutable();
        return Ok(());
    };

    if args.create_database {
        create_database_if_not_exists(client, base_url, &db, initialized_cols).await?;
    }

    if args.drop {
        let url = format!("{}/_api/database/{}/columnar/{}", base_url, db, coll);
        let _ = client.delete(&url).send().await; // Ignore errors (e.g. not found)
    }

    let url = format!("{}/_api/database/{}/columnar", base_url, db);
    let payload = serde_json::json!({
        "name": coll,
        "columns": record.get("columns").cloned().unwrap_or_else(|| Value::Array(vec![])),
        "compression": record.get("compression").cloned().unwrap_or(Value::Null),
    });

    let response = client.post(&url).json(&payload).send().await?;
    let status = response.status();
    if !status.is_success() && status.as_u16() != 409 {
        let body = response.text().await.unwrap_or_default();
        eprintln!(
            "  {} failed to create columnar collection {}/{}: {} {}",
            "Warning:".yellow().bold(),
            db,
            coll,
            status,
            body
        );
        *total_failed += 1;
    }

    initialized_cols.insert(format!("{}/{}", db, coll), true);
    Ok(())
}

/// Recreate a columnar index from a `_type: "columnar_index"` record.
async fn process_columnar_index_record(
    record: Value,
    args: &Args,
    client: &reqwest::Client,
    base_url: &str,
    total_imported: &mut u64,
    total_failed: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((db, coll)) = resolve_target(&record, args) else {
        note_unroutable();
        return Ok(());
    };

    let url = format!("{}/_api/database/{}/columnar/{}/index", base_url, db, coll);
    let payload = serde_json::json!({
        "column": record.get("column").cloned().unwrap_or(Value::Null),
        "index_type": record.get("index_type").cloned().unwrap_or(Value::Null),
    });

    let response = client.post(&url).json(&payload).send().await?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    // A column declared `indexed` in the schema is already indexed by the
    // create call, so "already has an index" is the expected outcome, not a
    // failure.
    if status.is_success() || body.contains("already has an index") {
        *total_imported += 1;
    } else {
        eprintln!(
            "  {} failed to create columnar index on {}/{}.{}: {} {}",
            "Warning:".yellow().bold(),
            db,
            coll,
            record
                .get("column")
                .and_then(|v| v.as_str())
                .unwrap_or("<unknown>"),
            status,
            body
        );
        *total_failed += 1;
    }
    Ok(())
}

/// Send the buffered columnar rows to the insert endpoint.
async fn flush_columnar_batch(
    batch: &mut ColumnarBatch,
    client: &reqwest::Client,
    base_url: &str,
    total_imported: &mut u64,
    total_failed: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((db, coll)) = batch.target.clone() else {
        batch.rows.clear();
        return Ok(());
    };
    if batch.rows.is_empty() {
        return Ok(());
    }

    let url = format!("{}/_api/database/{}/columnar/{}/insert", base_url, db, coll);
    let count = batch.rows.len();
    let response = client
        .post(&url)
        .json(&serde_json::json!({ "rows": batch.rows }))
        .send()
        .await?;

    if response.status().is_success() {
        let result: Value = response.json().await.unwrap_or(Value::Null);
        *total_imported += result
            .get("inserted")
            .and_then(|v| v.as_u64())
            .unwrap_or(count as u64);
    } else {
        eprintln!(
            "  {} columnar insert into {}/{} failed: {}",
            "Warning:".yellow().bold(),
            db,
            coll,
            response.status()
        );
        *total_failed += count as u64;
    }

    batch.rows.clear();
    Ok(())
}

/// Create a collection from a `_type: "collection"` record produced by
/// solidb-dump.
///
/// This is the first record emitted for every collection, so it is where the
/// collection gets created, dropped (with `--drop`) and given its real type.
/// Creating it here rather than lazily on the first document is what lets an
/// empty collection survive a round trip.
async fn process_collection_record(
    record: Value,
    args: &Args,
    client: &reqwest::Client,
    base_url: &str,
    initialized_cols: &mut HashMap<String, bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((db, coll)) = resolve_target(&record, args) else {
        note_unroutable();
        return Ok(());
    };

    let key = format!("{}/{}", db, coll);
    if initialized_cols.contains_key(&key) {
        return Ok(());
    }

    if args.create_database {
        create_database_if_not_exists(client, base_url, &db, initialized_cols).await?;
    }

    let shard_config = record.get("_shardConfig");
    let collection_type = record.get("_collectionType").and_then(|v| v.as_str());
    ensure_collection_exists(
        client,
        base_url,
        &db,
        &coll,
        shard_config,
        collection_type,
        args.drop,
    )
    .await?;

    // POST /collection only honours `type` for blob collections, and does not
    // honour it at all when the collection already exists (409). Setting the
    // type explicitly is the only way "edge" and "timeseries" survive.
    if let Some(ctype) = collection_type {
        if ctype != "document" {
            let url = format!(
                "{}/_api/database/{}/collection/{}/properties",
                base_url, db, coll
            );
            let mut payload = serde_json::json!({ "type": ctype });
            // Echo the existing shard settings back so this call cannot
            // reset them.
            if let Some(config) = shard_config {
                if let Some(num_shards) = config.get("num_shards") {
                    payload["numShards"] = num_shards.clone();
                }
                if let Some(rf) = config.get("replication_factor") {
                    payload["replicationFactor"] = rf.clone();
                }
            }
            let response = client.put(&url).json(&payload).send().await?;
            if !response.status().is_success() {
                eprintln!(
                    "  {} could not set type '{}' on {}/{}: {}",
                    "Warning:".yellow().bold(),
                    ctype,
                    db,
                    coll,
                    response.status()
                );
            }
        }
    }

    initialized_cols.insert(key, true);
    Ok(())
}

/// Recreate an index from a `_type: "index"` record produced by solidb-dump.
///
/// Routes to the appropriate create endpoint based on `_index_kind`:
/// regular indexes → `/index`, geo → `/geo`, ttl → `/ttl`, vector → `/vector`.
/// Existing indexes (409) are tolerated; other errors are counted as failed.
#[allow(clippy::too_many_arguments)]
async fn process_index_record(
    record: Value,
    args: &Args,
    client: &reqwest::Client,
    base_url: &str,
    initialized_cols: &mut HashMap<String, bool>,
    total_imported: &mut u64,
    total_failed: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((db, coll)) = resolve_target(&record, args) else {
        note_unroutable();
        return Ok(());
    };

    // Make sure DB and collection exist before posting the index.
    // Pass the collection type through: an index record is often the first
    // record seen for a collection, and creating a blob collection as a plain
    // document collection here would leave its chunks unreadable.
    let key = format!("{}/{}", db, coll);
    if !initialized_cols.contains_key(&key) {
        if args.create_database {
            create_database_if_not_exists(client, base_url, &db, initialized_cols).await?;
        }
        let collection_type = record.get("_collectionType").and_then(|v| v.as_str());
        ensure_collection_exists(
            client,
            base_url,
            &db,
            &coll,
            None,
            collection_type,
            args.drop,
        )
        .await?;
        initialized_cols.insert(key, true);
    }

    let kind = record
        .get("_index_kind")
        .and_then(|v| v.as_str())
        .unwrap_or("persistent");

    let (url, payload) = match kind {
        "geo" => {
            let url = format!("{}/_api/database/{}/geo/{}", base_url, db, coll);
            let payload = serde_json::json!({
                "name": record.get("name").cloned().unwrap_or(Value::Null),
                "field": record.get("field").cloned().unwrap_or(Value::Null),
            });
            (url, payload)
        }
        "ttl" => {
            let url = format!("{}/_api/database/{}/ttl/{}", base_url, db, coll);
            let payload = serde_json::json!({
                "name": record.get("name").cloned().unwrap_or(Value::Null),
                "field": record.get("field").cloned().unwrap_or(Value::Null),
                "expire_after_seconds": record
                    .get("expire_after_seconds")
                    .cloned()
                    .unwrap_or(Value::Null),
            });
            (url, payload)
        }
        "vector" => {
            let url = format!("{}/_api/database/{}/vector/{}", base_url, db, coll);
            let mut payload = serde_json::json!({
                "name": record.get("name").cloned().unwrap_or(Value::Null),
                "field": record.get("field").cloned().unwrap_or(Value::Null),
                "dimension": record.get("dimension").cloned().unwrap_or(Value::Null),
            });
            if let Some(metric) = record.get("metric") {
                payload["metric"] = metric.clone();
            }
            if let Some(m) = record.get("m") {
                if !m.is_null() {
                    payload["m"] = m.clone();
                }
            }
            if let Some(ef) = record.get("ef_construction") {
                if !ef.is_null() {
                    payload["ef_construction"] = ef.clone();
                }
            }
            if let Some(q) = record.get("quantization") {
                payload["quantization"] = q.clone();
            }
            (url, payload)
        }
        // hash / persistent / fulltext / bloom / cuckoo
        other => {
            let url = format!("{}/_api/database/{}/index/{}", base_url, db, coll);
            let mut payload = serde_json::json!({
                "name": record.get("name").cloned().unwrap_or(Value::Null),
                "type": other,
                "unique": record.get("unique").cloned().unwrap_or(Value::Bool(false)),
            });
            if let Some(fields) = record.get("fields") {
                payload["fields"] = fields.clone();
            }
            if let Some(field) = record.get("field") {
                payload["field"] = field.clone();
            }
            (url, payload)
        }
    };

    let response = client.post(&url).json(&payload).send().await?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    // An index that is already there is a success for restore purposes. Edge
    // collections create `_edge_from_idx`/`_edge_to_idx` themselves and report
    // the clash as 400 rather than 409, so the body has to be checked too.
    if status.is_success() || status.as_u16() == 409 || body.contains("already exists") {
        *total_imported += 1;
    } else {
        eprintln!(
            "  Failed to create {} index '{}' on {}/{}: {} {}",
            kind,
            record
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("<unknown>"),
            db,
            coll,
            status,
            body
        );
        *total_failed += 1;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn process_blob_chunk(
    header_doc: Value,
    data: Vec<u8>,
    args: &Args,
    client: &reqwest::Client,
    base_url: &str,
    batch: &mut Vec<Vec<u8>>,
    batch_size: &mut usize,
    batch_meta: &mut Option<(String, String)>,
    max_count: usize,
    max_size: usize,
    initialized_cols: &mut HashMap<String, bool>,
    total_imported: &mut u64,
    total_failed: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Determine target DB and Collection from header.
    let Some((db, coll)) = resolve_target(&header_doc, args) else {
        note_unroutable();
        return Ok(());
    };

    // Create DB/Collection if needed
    let key = format!("{}/{}", db, coll);
    if !initialized_cols.contains_key(&key) {
        if args.create_database {
            create_database_if_not_exists(client, base_url, &db, initialized_cols).await?;
        }
        let shard_config = header_doc.get("_shardConfig");
        let collection_type = header_doc.get("_collectionType").and_then(|v| v.as_str());
        ensure_collection_exists(
            client,
            base_url,
            &db,
            &coll,
            shard_config,
            collection_type,
            args.drop,
        )
        .await?;
        initialized_cols.insert(key.clone(), true);
    }

    // Check batch consistency
    if let Some((curr_db, curr_coll)) = batch_meta {
        if curr_db != &db || curr_coll != &coll {
            flush_batch(
                batch,
                batch_size,
                client,
                base_url,
                curr_db,
                curr_coll,
                total_imported,
                total_failed,
            )
            .await?;
            *batch_meta = None;
        }
    }

    if batch_meta.is_none() {
        *batch_meta = Some((db.clone(), coll.clone()));
    }

    // Add Header
    let header_bytes = serde_json::to_vec(&header_doc)?;
    *batch_size += header_bytes.len();
    batch.push(header_bytes);

    // Add Data
    *batch_size += data.len();
    batch.push(data);

    // Flush if full
    if batch.len() >= max_count || *batch_size >= max_size {
        if let Some((curr_db, curr_coll)) = batch_meta {
            flush_batch(
                batch,
                batch_size,
                client,
                base_url,
                curr_db,
                curr_coll,
                total_imported,
                total_failed,
            )
            .await?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn process_doc(
    doc: Value,
    args: &Args,
    client: &reqwest::Client,
    base_url: &str,
    batch: &mut Vec<Vec<u8>>,
    batch_size: &mut usize,
    batch_meta: &mut Option<(String, String)>,
    max_count: usize,
    max_size: usize,
    initialized_cols: &mut HashMap<String, bool>,
    total_imported: &mut u64,
    total_failed: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Determine target DB and Collection.
    let Some((db, coll)) = resolve_target(&doc, args) else {
        note_unroutable();
        return Ok(());
    };

    // Create DB/Collection if needed
    let key = format!("{}/{}", db, coll);
    if !initialized_cols.contains_key(&key) {
        // Try create DB
        if args.create_database {
            create_database_if_not_exists(client, base_url, &db, initialized_cols).await?;
        }

        let shard_config = doc.get("_shardConfig");
        let collection_type = doc.get("_collectionType").and_then(|v| v.as_str());
        ensure_collection_exists(
            client,
            base_url,
            &db,
            &coll,
            shard_config,
            collection_type,
            args.drop,
        )
        .await?;

        initialized_cols.insert(key.clone(), true);
    }

    // Check batch consistency
    if let Some((curr_db, curr_coll)) = batch_meta {
        if curr_db != &db || curr_coll != &coll {
            // Flush because collection changed
            flush_batch(
                batch,
                batch_size,
                client,
                base_url,
                curr_db,
                curr_coll,
                total_imported,
                total_failed,
            )
            .await?;
            *batch_meta = None;
        }
    }

    if batch_meta.is_none() {
        *batch_meta = Some((db.clone(), coll.clone()));
    }

    // Strip restore metadata fields so they don't end up persisted as
    // document fields. The dump tool adds these as routing hints, not data.
    let mut clean_doc = doc;
    if let Some(obj) = clean_doc.as_object_mut() {
        obj.remove("_database");
        obj.remove("_collection");
        obj.remove("_shardConfig");
        obj.remove("_collectionType");
    }

    // Add doc to batch (Pre-serialize to avoid double serialization)
    let doc_bytes = serde_json::to_vec(&clean_doc)?;
    *batch_size += doc_bytes.len();
    batch.push(doc_bytes);

    // Flush if full
    if batch.len() >= max_count || *batch_size >= max_size {
        if let Some((curr_db, curr_coll)) = batch_meta {
            flush_batch(
                batch,
                batch_size,
                client,
                base_url,
                curr_db,
                curr_coll,
                total_imported,
                total_failed,
            )
            .await?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn flush_batch(
    batch: &mut Vec<Vec<u8>>,
    batch_size: &mut usize,
    client: &reqwest::Client,
    base_url: &str,
    db: &str,
    coll: &str,
    total_imported: &mut u64,
    total_failed: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    if batch.is_empty() {
        return Ok(());
    }

    let url = format!(
        "{}/_api/database/{}/collection/{}/import",
        base_url, db, coll
    );

    // Create JSONL payload from pre-serialized bytes
    let mut jsonl_data = Vec::with_capacity(*batch_size + batch.len()); // + newlines
    for doc_bytes in batch.iter() {
        jsonl_data.extend_from_slice(doc_bytes);
        jsonl_data.push(b'\n');
    }

    let part = reqwest::multipart::Part::bytes(jsonl_data)
        .file_name("restore.jsonl")
        .mime_str("application/x-ndjson")?;

    let form = reqwest::multipart::Form::new().part("file", part);

    let response = client.post(&url).multipart(form).send().await?;

    if !response.status().is_success() {
        eprintln!("  Batch failed: {}", response.status());
        *total_failed += batch.len() as u64;
    } else {
        let result: Value = response.json().await?;
        // Server returns `count` for imported documents; older versions used `imported`
        let imported = result
            .get("count")
            .and_then(|v| v.as_u64())
            .or_else(|| result.get("imported").and_then(|v| v.as_u64()))
            .unwrap_or(0);
        *total_imported += imported;
        *total_failed += result["failed"].as_u64().unwrap_or(0);
    }

    batch.clear();
    *batch_size = 0;
    Ok(())
}

async fn ensure_collection_exists(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    collection: &str,
    shard_config: Option<&Value>,
    collection_type: Option<&str>,
    drop: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Logic similar to restore_collection but handles single init

    if drop {
        let url = format!(
            "{}/_api/database/{}/collection/{}",
            base_url, database, collection
        );
        let _ = client.delete(&url).send().await; // Ignore errors (e.g. not found)
    }

    let url = format!("{}/_api/database/{}/collection", base_url, database);
    let mut create_payload = serde_json::json!({ "name": collection });

    // In dump, blob chunks also have _shardConfig if replicated?
    // The dump logic adds _shardConfig to every doc.

    if let Some(config) = shard_config {
        if let Some(num_shards) = config.get("num_shards") {
            create_payload["numShards"] = num_shards.clone();
        }
        if let Some(replication_factor) = config.get("replication_factor") {
            create_payload["replicationFactor"] = replication_factor.clone();
        }
        if let Some(shard_key) = config.get("shard_key") {
            create_payload["shardKey"] = shard_key.clone();
        }
    }

    if let Some(ctype) = collection_type {
        create_payload["type"] = serde_json::Value::String(ctype.to_string());
    }

    // Are we restoring a blob collection?
    // The dump format for blob chunks: {"_type": "blob_chunk", ...}.
    // But the dump *does not* explicitly say "this is a blob collection" in the doc metadata,
    // UNLESS the prompt explicitly asked to include it?
    // Wait, `export_collection` DOES NOT include collection type in the output JSONL.
    // It yields `doc`.
    // It yields `chunk_doc`.
    // The chunks have `_type: blob_chunk`.
    // If simple docs come first, we might create as "document" type default.
    // Then chunks arrive. Import will try to put_blob_chunk on a "document" collection -> Error?
    // Correct. `put_blob_chunk` might fail if collection type is not blob?
    // `Collection::put_blob_chunk` implementation: It doesn't check type strictly?
    // But `handlers.rs:upload_blob` checks type.
    // `handlers.rs:import_collection` (my update) calls `put_blob_chunk` directly.
    // Does `put_blob_chunk` enforce type?
    // `src/storage/collection.rs`: `put_blob_chunk` writes to `blo:...`. It doesn't check `self.collection_type`.
    // SO it might "work" but metadata says "document".
    // Ideally we should create as "blob" if we see chunks. BUT we create collection at first doc.
    // Issue: First doc is metadata doc. It looks like standard doc.
    // We create "document" collection.
    // Then chunks come. We write chunks.
    // Collection thinks it's "document".
    // API logic might block regular blob upload later.
    // FIX: We need `type` in the dump!
    // `solidb-dump` does NOT export `type`.
    // I should fix `solidb-dump` (`export_collection` and `dump_collection_jsonl`) to include `collectionType: "blob"` in the metadata of every doc?
    // Or just `_type: blob`?
    // Let's assume standard collections for now or default.
    // Wait, `export_collection` handler does: `yield ... json`.
    // I should insert `_collectionType` into that JSON.

    // Let's assume for now user creates collection manually or we default to document.
    // But for "blob restore" to work fully, we probably want the type.
    // However, I can't easily change previous logic too much in this single Step.
    // I'll stick to basic create.

    let response = client.post(&url).json(&create_payload).send().await?;
    if !response.status().is_success() && response.status().as_u16() != 409 {
        eprintln!(
            " Warning: Failed to create collection {}: {}",
            collection,
            response.status()
        );
    }
    Ok(())
}

/// Create the database unless we already tried during this run.
///
/// `ensured` is the same map used for collections; collection keys are
/// `"{db}/{coll}"`, so a bare database name cannot collide. Without this the
/// database is re-created once per collection, which does nothing but print
/// "Database already exists" on every collection boundary.
async fn create_database_if_not_exists(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    ensured: &mut HashMap<String, bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    if ensured.contains_key(database) {
        return Ok(());
    }

    let url = format!("{}/_api/database", base_url);

    let response = client
        .post(&url)
        .json(&serde_json::json!({ "name": database }))
        .send()
        .await?;

    let status = response.status();
    if status.is_success() {
        eprintln!("  Created database: {}", database);
    } else if status.as_u16() == 409 {
        eprintln!("  Database already exists: {}", database);
    } else {
        return Err(format!("Failed to create database: {}", status).into());
    }

    ensured.insert(database.to_string(), true);
    Ok(())
}

use colored::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(database: Option<&str>, collection: Option<&str>) -> Args {
        Args {
            host: "localhost".to_string(),
            port: 6745,
            input: "dump.jsonl".to_string(),
            database: database.map(String::from),
            collection: collection.map(String::from),
            create_database: false,
            drop: false,
            user: None,
            password: None,
        }
    }

    #[test]
    fn control_records_are_recognised() {
        for t in [
            "collection",
            "index",
            "columnar",
            "columnar_row",
            "columnar_index",
            "blob_chunk",
        ] {
            let rec = json!({"_type": t, "_database": "db", "_collection": "c"});
            assert_eq!(control_record_type(&rec).as_deref(), Some(t));
        }
    }

    #[test]
    fn documents_are_never_control_records() {
        // A document that happens to carry a `_type` field must be imported,
        // not swallowed as an index definition.
        let doc = json!({
            "_key": "42",
            "_type": "index",
            "_database": "db",
            "_collection": "articles",
            "title": "Indexing for beginners",
        });
        assert_eq!(control_record_type(&doc), None);
    }

    #[test]
    fn unknown_type_values_are_documents() {
        let doc = json!({"_type": "invoice", "_database": "db", "_collection": "c"});
        assert_eq!(control_record_type(&doc), None);
    }

    #[test]
    fn cli_flags_override_the_names_in_the_dump() {
        let rec = json!({"_database": "prod", "_collection": "users"});
        assert_eq!(
            resolve_target(&rec, &args(Some("staging"), None)),
            Some(("staging".to_string(), "users".to_string()))
        );
        assert_eq!(
            resolve_target(&rec, &args(Some("staging"), Some("people"))),
            Some(("staging".to_string(), "people".to_string()))
        );
    }

    #[test]
    fn records_without_a_target_are_unroutable() {
        // Blob records from a pre-fix dump: no routing metadata at all.
        let rec = json!({"_type": "blob_chunk", "_doc_key": "k", "_chunk_index": 0});
        assert_eq!(resolve_target(&rec, &args(None, None)), None);
        // A database override alone is not enough to place the record.
        assert_eq!(resolve_target(&rec, &args(Some("db"), None)), None);
        // Both overrides given: routable again.
        assert!(resolve_target(&rec, &args(Some("db"), Some("c"))).is_some());
    }
}
