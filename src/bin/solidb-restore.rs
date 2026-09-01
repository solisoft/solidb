use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

/// Records that named no target database/collection and could not be routed.
static SKIPPED_UNROUTABLE: AtomicU64 = AtomicU64::new(0);

/// One-shot warning when `-c` overrides a record's embedded collection.
static WARNED_COLLECTION_OVERRIDE: AtomicBool = AtomicBool::new(false);

/// Rows per POST to the columnar insert endpoint.
const MAX_COLUMNAR_ROWS_PER_INSERT: usize = 10_000;

/// Percent-encode a single URL path segment (RFC 3986 unreserved left alone).
fn path_seg(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

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

    // Warn once when -c forces a different collection than the dump named.
    if let Some(ref override_coll) = args.collection {
        if let Some(embedded) = record.get("_collection").and_then(|s| s.as_str()) {
            if embedded != override_coll.as_str()
                && !WARNED_COLLECTION_OVERRIDE.swap(true, Ordering::Relaxed)
            {
                eprintln!(
                    "  {} -c/--collection overrides dump collection names \
                     (e.g. '{}' → '{}'); every record will land in that collection.",
                    "Warning:".yellow().bold(),
                    embedded,
                    override_coll
                );
            }
        }
    }

    Some((db, coll))
}

/// Classify a dump record as one of the control records solidb-dump emits
/// rather than a document to import.
///
/// A stored document always carries a `_key`, and no control record ever does
/// (document envelopes use `_type: "document"` with the payload nested under
/// `doc`). Requiring the absence of `_key` keeps a user document that happens
/// to have a field named `_type` from being mistaken for a control record.
fn control_record_type(record: &Value) -> Option<String> {
    if record.get("_key").is_some() {
        return None;
    }
    let t = record.get("_type")?.as_str()?;
    matches!(
        t,
        "collection"
            | "index"
            | "document"
            | "columnar"
            | "columnar_row"
            | "columnar_index"
            | "blob_chunk"
    )
    .then(|| t.to_string())
}

/// Force every column's `indexed` flag off for columnar create.
///
/// Server create only stores the flag; real indexes must be created via
/// `columnar_index` records. Older dumps may still ship `indexed: true`.
fn sanitize_columnar_columns(columns: &Value) -> Value {
    match columns.as_array() {
        Some(arr) => Value::Array(
            arr.iter()
                .map(|c| {
                    let mut obj = c.as_object().cloned().unwrap_or_default();
                    obj.insert("indexed".to_string(), Value::Bool(false));
                    // Prefer create-endpoint field name `type` over `data_type`.
                    if !obj.contains_key("type") {
                        if let Some(dt) = obj.remove("data_type") {
                            obj.insert("type".to_string(), dt);
                        }
                    }
                    Value::Object(obj)
                })
                .collect(),
        ),
        None => Value::Array(vec![]),
    }
}

/// Records dropped because their collection matched `--exclude-collection`.
static EXCLUDED_RECORDS: AtomicU64 = AtomicU64::new(0);

/// Excluded collection names already announced, so the note prints once each
/// rather than once per record.
static ANNOUNCED_EXCLUSIONS: LazyLock<Mutex<BTreeSet<String>>> =
    LazyLock::new(|| Mutex::new(BTreeSet::new()));

/// Match a collection name against one `--exclude-collection` pattern.
///
/// Exact by default; `*` matches any run of characters, so `events_*` covers a
/// family of collections and `*` excludes everything. Deliberately not a full
/// glob — no `?`, no character classes — because a collection name is a plain
/// identifier and anything richer invites a pattern that quietly matches more
/// than the operator meant.
fn glob_match(pattern: &str, name: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == name;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let Some(mut rest) = name.strip_prefix(parts[0]) else {
        return false;
    };
    let last = parts.len() - 1;
    for (i, part) in parts.iter().enumerate().skip(1) {
        // Empty part: a trailing `*`, or two in a row. Both match anything left.
        if part.is_empty() {
            continue;
        }
        if i == last {
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(idx) => rest = &rest[idx + part.len()..],
            None => return false,
        }
    }
    true
}

/// Resolve a record's target, dropping it if `--exclude-collection` matches.
///
/// The name matched is the one the *dump* carries, falling back to the
/// resolved target when the record names no collection of its own. That way
/// `--exclude-collection events` filters the dump's `events` records even when
/// `-c` is rewriting everything into one collection — the operator is
/// reasoning about what is in the file, not about what it will be renamed to.
///
/// Exclusion is not a failure: unlike an unroutable record it does not warn
/// per record and does not affect the exit status, because dropping these is
/// exactly what was asked for.
fn route(record: &Value, args: &Args) -> Option<(String, String)> {
    let (db, coll) = match resolve_target(record, args) {
        Some(t) => t,
        None => {
            note_unroutable();
            return None;
        }
    };

    if args.exclude_collection.is_empty() {
        return Some((db, coll));
    }

    let matched_against = record
        .get("_collection")
        .and_then(|v| v.as_str())
        .unwrap_or(coll.as_str());

    let Some(pattern) = args
        .exclude_collection
        .iter()
        .find(|p| glob_match(p, matched_against))
    else {
        return Some((db, coll));
    };

    EXCLUDED_RECORDS.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut announced) = ANNOUNCED_EXCLUSIONS.lock() {
        if announced.insert(matched_against.to_string()) {
            eprintln!(
                "  {} excluding {} (matched --exclude-collection {})",
                "Note:".yellow().bold(),
                matched_against.cyan(),
                pattern
            );
        }
    }
    None
}

/// Report a record that carries no routing metadata.
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
#[command(
    about = "Import SoliDB database or collection from dump (JSONL, JSON Array, CSV). SQL is not supported.",
    long_about = None
)]
struct Args {
    /// Database host
    #[arg(short = 'H', long, default_value = "localhost")]
    host: String,

    /// Database port
    #[arg(short = 'P', long, default_value = "6745")]
    port: u16,

    /// URL scheme (http or https)
    #[arg(long, default_value = "http")]
    scheme: String,

    /// Input file (JSONL, JSON Array, or CSV)
    #[arg(short, long)]
    input: String,

    /// Override database name (use instead of name in dump)
    #[arg(short = 'd', long)]
    database: Option<String>,

    /// Override collection name (routes every record into this collection)
    #[arg(short = 'c', long)]
    collection: Option<String>,

    /// Skip these collections. Repeatable, and comma-separated values are
    /// accepted: `--exclude-collection a,b --exclude-collection c`.
    ///
    /// `*` matches any run of characters, so `--exclude-collection 'events_*'`
    /// drops a whole family (quote it so the shell does not expand it first).
    /// Matched against the collection name in the dump, so it still selects
    /// the right records when -c is rewriting the target. Excluded records are
    /// counted and reported, and do not affect the exit status.
    #[arg(long = "exclude-collection", value_delimiter = ',')]
    exclude_collection: Vec<String>,

    /// Create database if it doesn't exist
    #[arg(long)]
    create_database: bool,

    /// Drop existing collections before restore
    #[arg(long)]
    drop: bool,

    /// Upsert documents by `_key` instead of failing when a key already exists.
    /// Requires a server that accepts `?mode=upsert` on the import endpoint.
    /// Prefer `--drop` for a clean restore when possible.
    #[arg(long)]
    overwrite: bool,

    /// Do not fail the process when records lack routing metadata (legacy dumps).
    /// Failed imports still cause a non-zero exit.
    #[arg(long)]
    allow_skipped: bool,

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

    let scheme = args.scheme.to_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Invalid --scheme '{}': expected http or https", args.scheme).into());
    }
    let base_url = format!("{}://{}:{}", scheme, args.host, args.port);

    let token = match (&args.user, &args.password) {
        (None, None) => None,
        (Some(user), Some(password)) => {
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
        }
        _ => {
            return Err("Both -u/--user and -p/--password are required for authentication".into());
        }
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

    let file = File::open(&args.input)?;
    let metadata = file.metadata()?;
    let total_size = metadata.len();

    let pb = ProgressBar::new(total_size);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    let mut reader = BufReader::new(pb.wrap_read(file));

    let mut format = "csv";

    if args.input.to_lowercase().ends_with(".sql") {
        format = "sql";
    }

    if format == "csv" {
        let buf = reader.fill_buf()?;
        for &byte in buf {
            if !byte.is_ascii_whitespace() {
                if byte == b'[' {
                    format = "json_array";
                } else if byte == b'{' {
                    format = "jsonl";
                } else {
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

    let mut current_batch: Vec<Vec<u8>> = Vec::new();
    let mut current_batch_size = 0;
    let mut current_batch_meta: Option<(String, String)> = None;
    let max_batch_count = 20000;
    let max_batch_size = 25 * 1024 * 1024; // 25MB

    let mut initialized_collections: HashMap<String, bool> = HashMap::new();
    let mut columnar_batch = ColumnarBatch::default();
    let mut total_imported = 0;
    let mut total_failed = 0;

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
        return Err(
            "SQL restore is not implemented. Convert to JSONL or CSV and re-run solidb-restore."
                .into(),
        );
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
        eprintln!("Restoring using streaming mode (JSONL/Mixed)...");
        let mut buffer = Vec::new();

        loop {
            let bytes_read = reader.read_until(b'\n', &mut buffer)?;
            if bytes_read == 0 {
                break;
            }

            let line_slice = if buffer.ends_with(b"\n") {
                &buffer[..buffer.len() - 1]
            } else {
                &buffer
            };
            if line_slice.iter().all(|b| b.is_ascii_whitespace()) {
                buffer.clear();
                continue;
            }

            match serde_json::from_slice::<Value>(line_slice) {
                Ok(doc) => {
                    let record_type = control_record_type(&doc);

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
                            let Some(target) = route(&doc, &args) else {
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
                        Some("document") => {
                            // Envelope: routing on the outer record, payload in `doc`.
                            // Do not merge routing into the payload — user documents
                            // may legitimately store fields named `_database` etc.
                            let Some((db, coll)) = route(&doc, &args) else {
                                buffer.clear();
                                continue;
                            };
                            let payload = doc
                                .get("doc")
                                .cloned()
                                .unwrap_or(Value::Object(Default::default()));
                            let collection_type =
                                doc.get("_collectionType").and_then(|v| v.as_str());
                            let shard_config = doc.get("_shardConfig");
                            process_doc_routed(
                                payload,
                                &db,
                                &coll,
                                collection_type,
                                shard_config,
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
                            buffer.clear();
                            continue;
                        }
                        _ => {}
                    }

                    if record_type.as_deref() == Some("collection") {
                        if let Some((db, coll)) = current_batch_meta.clone() {
                            flush_batch(
                                &mut current_batch,
                                &mut current_batch_size,
                                &client,
                                &base_url,
                                &db,
                                &coll,
                                args.overwrite,
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

                    if record_type.as_deref() == Some("index") {
                        if let Some((db, coll)) = current_batch_meta.clone() {
                            flush_batch(
                                &mut current_batch,
                                &mut current_batch_size,
                                &client,
                                &base_url,
                                &db,
                                &coll,
                                args.overwrite,
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

                    let is_blob_chunk = record_type.as_deref() == Some("blob_chunk");

                    if is_blob_chunk {
                        if let Some(data_len) = doc.get("_data_length").and_then(|v| v.as_u64()) {
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

    if !current_batch.is_empty() {
        if let Some((db, coll)) = &current_batch_meta {
            flush_batch(
                &mut current_batch,
                &mut current_batch_size,
                &client,
                &base_url,
                db,
                coll,
                args.overwrite,
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
    let excluded = EXCLUDED_RECORDS.load(Ordering::Relaxed);
    if excluded > 0 {
        let names = ANNOUNCED_EXCLUSIONS
            .lock()
            .map(|n| n.iter().cloned().collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        eprintln!(
            "  → {} items excluded ({})",
            excluded.to_string().yellow(),
            names
        );
    } else if !args.exclude_collection.is_empty() {
        // A typo'd pattern silently restores everything, which is the opposite
        // of what was asked for. Say so rather than reporting a clean run.
        eprintln!(
            "  {} --exclude-collection {:?} matched nothing in this dump",
            "Warning:".yellow().bold(),
            args.exclude_collection
        );
    }

    if total_failed > 0 {
        return Err(format!("Restore finished with {} failed item(s)", total_failed).into());
    }
    if skipped > 0 && !args.allow_skipped {
        return Err(format!(
            "Restore skipped {} unroutable record(s); re-dump with a current solidb-dump, \
             pass -d/-c, or use --allow-skipped",
            skipped
        )
        .into());
    }

    Ok(())
}

use std::io::Read;

#[derive(Default)]
struct ColumnarBatch {
    target: Option<(String, String)>,
    rows: Vec<Value>,
}

async fn process_columnar_record(
    record: Value,
    args: &Args,
    client: &reqwest::Client,
    base_url: &str,
    initialized_cols: &mut HashMap<String, bool>,
    total_failed: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((db, coll)) = route(&record, args) else {
        return Ok(());
    };

    if args.create_database {
        create_database_if_not_exists(client, base_url, &db, initialized_cols).await?;
    }

    if args.drop {
        let url = format!(
            "{}/_api/database/{}/columnar/{}",
            base_url,
            path_seg(&db),
            path_seg(&coll)
        );
        let _ = client.delete(&url).send().await;
    }

    let url = format!("{}/_api/database/{}/columnar", base_url, path_seg(&db));
    let columns = sanitize_columnar_columns(
        &record
            .get("columns")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])),
    );
    let payload = serde_json::json!({
        "name": coll,
        "columns": columns,
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

async fn process_columnar_index_record(
    record: Value,
    args: &Args,
    client: &reqwest::Client,
    base_url: &str,
    total_imported: &mut u64,
    total_failed: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((db, coll)) = route(&record, args) else {
        return Ok(());
    };

    let url = format!(
        "{}/_api/database/{}/columnar/{}/index",
        base_url,
        path_seg(&db),
        path_seg(&coll)
    );
    let payload = serde_json::json!({
        "column": record.get("column").cloned().unwrap_or(Value::Null),
        "index_type": record.get("index_type").cloned().unwrap_or(Value::Null),
    });

    let response = client.post(&url).json(&payload).send().await?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    // 409 / "already has an index" is acceptable on re-run without --drop.
    // It is NOT expected on a fresh create after sanitize_columnar_columns.
    if status.is_success() || status.as_u16() == 409 || body.contains("already has an index") {
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

    let url = format!(
        "{}/_api/database/{}/columnar/{}/insert",
        base_url,
        path_seg(&db),
        path_seg(&coll)
    );
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

async fn process_collection_record(
    record: Value,
    args: &Args,
    client: &reqwest::Client,
    base_url: &str,
    initialized_cols: &mut HashMap<String, bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((db, coll)) = route(&record, args) else {
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
                base_url,
                path_seg(&db),
                path_seg(&coll)
            );
            let mut payload = serde_json::json!({ "type": ctype });
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
    let Some((db, coll)) = route(&record, args) else {
        return Ok(());
    };

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

    let db_enc = path_seg(&db);
    let coll_enc = path_seg(&coll);

    let (url, payload) = match kind {
        "geo" => {
            let url = format!("{}/_api/database/{}/geo/{}", base_url, db_enc, coll_enc);
            let payload = serde_json::json!({
                "name": record.get("name").cloned().unwrap_or(Value::Null),
                "field": record.get("field").cloned().unwrap_or(Value::Null),
            });
            (url, payload)
        }
        "ttl" => {
            let url = format!("{}/_api/database/{}/ttl/{}", base_url, db_enc, coll_enc);
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
            let url = format!("{}/_api/database/{}/vector/{}", base_url, db_enc, coll_enc);
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
        other => {
            let url = format!("{}/_api/database/{}/index/{}", base_url, db_enc, coll_enc);
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
    if status.is_success() || status.as_u16() == 409 {
        *total_imported += 1;
    } else {
        let body = response.text().await.unwrap_or_default();
        // Edge collections sometimes report duplicate indexes as 400.
        if body.to_lowercase().contains("already exists") {
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
    // The chunk's payload bytes were already consumed by the caller, so the
    // stream stays framed whether or not this record is imported.
    let Some((db, coll)) = route(&header_doc, args) else {
        return Ok(());
    };

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

    if let Some((curr_db, curr_coll)) = batch_meta {
        if curr_db != &db || curr_coll != &coll {
            flush_batch(
                batch,
                batch_size,
                client,
                base_url,
                curr_db,
                curr_coll,
                args.overwrite,
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

    let header_bytes = serde_json::to_vec(&header_doc)?;
    *batch_size += header_bytes.len();
    batch.push(header_bytes);

    *batch_size += data.len();
    batch.push(data);

    if batch.len() >= max_count || *batch_size >= max_size {
        if let Some((curr_db, curr_coll)) = batch_meta {
            flush_batch(
                batch,
                batch_size,
                client,
                base_url,
                curr_db,
                curr_coll,
                args.overwrite,
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
    // Legacy flat records: routing fields live on the same object as user data.
    let Some((db, coll)) = route(&doc, args) else {
        return Ok(());
    };
    let collection_type = doc
        .get("_collectionType")
        .and_then(|v| v.as_str())
        .map(String::from);
    let shard_config = doc.get("_shardConfig").cloned();

    let mut clean_doc = doc;
    if let Some(obj) = clean_doc.as_object_mut() {
        obj.remove("_database");
        obj.remove("_collection");
        obj.remove("_shardConfig");
        obj.remove("_collectionType");
    }

    process_doc_routed(
        clean_doc,
        &db,
        &coll,
        collection_type.as_deref(),
        shard_config.as_ref(),
        args,
        client,
        base_url,
        batch,
        batch_size,
        batch_meta,
        max_count,
        max_size,
        initialized_cols,
        total_imported,
        total_failed,
    )
    .await
}

/// Import a document whose target collection is already known.
///
/// Used by envelope dumps (routing is outside the payload) and by the legacy
/// flat path after routing fields have been stripped.
#[allow(clippy::too_many_arguments)]
async fn process_doc_routed(
    clean_doc: Value,
    db: &str,
    coll: &str,
    collection_type: Option<&str>,
    shard_config: Option<&Value>,
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
    let key = format!("{}/{}", db, coll);
    if !initialized_cols.contains_key(&key) {
        if args.create_database {
            create_database_if_not_exists(client, base_url, db, initialized_cols).await?;
        }
        ensure_collection_exists(
            client,
            base_url,
            db,
            coll,
            shard_config,
            collection_type,
            args.drop,
        )
        .await?;
        initialized_cols.insert(key.clone(), true);
    }

    if let Some((curr_db, curr_coll)) = batch_meta {
        if curr_db != db || curr_coll != coll {
            flush_batch(
                batch,
                batch_size,
                client,
                base_url,
                curr_db,
                curr_coll,
                args.overwrite,
                total_imported,
                total_failed,
            )
            .await?;
            *batch_meta = None;
        }
    }

    if batch_meta.is_none() {
        *batch_meta = Some((db.to_string(), coll.to_string()));
    }

    let doc_bytes = serde_json::to_vec(&clean_doc)?;
    *batch_size += doc_bytes.len();
    batch.push(doc_bytes);

    if batch.len() >= max_count || *batch_size >= max_size {
        if let Some((curr_db, curr_coll)) = batch_meta {
            flush_batch(
                batch,
                batch_size,
                client,
                base_url,
                curr_db,
                curr_coll,
                args.overwrite,
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
    overwrite: bool,
    total_imported: &mut u64,
    total_failed: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    if batch.is_empty() {
        return Ok(());
    }

    let mut url = format!(
        "{}/_api/database/{}/collection/{}/import",
        base_url,
        path_seg(db),
        path_seg(coll)
    );
    if overwrite {
        url.push_str("?mode=upsert");
    }

    let mut jsonl_data = Vec::with_capacity(*batch_size + batch.len());
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
    if drop {
        let url = format!(
            "{}/_api/database/{}/collection/{}",
            base_url,
            path_seg(database),
            path_seg(collection)
        );
        let _ = client.delete(&url).send().await;
    }

    let url = format!(
        "{}/_api/database/{}/collection",
        base_url,
        path_seg(database)
    );
    let mut create_payload = serde_json::json!({ "name": collection });

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

    // Type is also set via PUT .../properties for non-document collections
    // (see process_collection_record). Blob is honoured on create.

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
/// `"{db}/{coll}"`, so a bare database name cannot collide.
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

    fn args(
        database: Option<&str>,
        collection: Option<&str>,
        overwrite: bool,
        allow_skipped: bool,
    ) -> Args {
        Args {
            host: "localhost".to_string(),
            port: 6745,
            scheme: "http".to_string(),
            input: "dump.jsonl".to_string(),
            database: database.map(String::from),
            collection: collection.map(String::from),
            exclude_collection: Vec::new(),
            create_database: false,
            drop: false,
            overwrite,
            allow_skipped,
            user: None,
            password: None,
        }
    }

    fn args_excluding(patterns: &[&str]) -> Args {
        let mut a = args(None, None, false, false);
        a.exclude_collection = patterns.iter().map(|s| s.to_string()).collect();
        a
    }

    #[test]
    fn control_records_are_recognised() {
        for t in [
            "collection",
            "index",
            "document",
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
            resolve_target(&rec, &args(Some("staging"), None, false, false)),
            Some(("staging".to_string(), "users".to_string()))
        );
        assert_eq!(
            resolve_target(&rec, &args(Some("staging"), Some("people"), false, false)),
            Some(("staging".to_string(), "people".to_string()))
        );
    }

    #[test]
    fn records_without_a_target_are_unroutable() {
        let rec = json!({"_type": "blob_chunk", "_doc_key": "k", "_chunk_index": 0});
        assert_eq!(resolve_target(&rec, &args(None, None, false, false)), None);
        assert_eq!(
            resolve_target(&rec, &args(Some("db"), None, false, false)),
            None
        );
        assert!(resolve_target(&rec, &args(Some("db"), Some("c"), false, false)).is_some());
    }

    #[test]
    fn sanitize_columnar_forces_indexed_false() {
        let cols = json!([
            {"name": "host", "type": "string", "indexed": true},
            {"name": "ts", "data_type": "timestamp", "indexed": true},
        ]);
        let out = sanitize_columnar_columns(&cols);
        let arr = out.as_array().unwrap();
        assert_eq!(arr[0]["indexed"], false);
        assert_eq!(arr[1]["indexed"], false);
        // data_type promoted to type for create endpoint
        assert_eq!(arr[1]["type"], "timestamp");
        assert!(arr[1].get("data_type").is_none());
    }

    #[test]
    fn glob_match_is_exact_without_a_star() {
        assert!(glob_match("users", "users"));
        assert!(!glob_match("users", "users2"));
        assert!(!glob_match("users", "Users"));
        assert!(!glob_match("user", "users"));
    }

    #[test]
    fn glob_match_handles_stars() {
        assert!(glob_match("events_*", "events_2024"));
        assert!(glob_match("events_*", "events_"));
        assert!(!glob_match("events_*", "events"));
        assert!(glob_match("*_2024", "events_2024"));
        assert!(!glob_match("*_2024", "events_2025"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a*c", "abc"));
        // `*` may match nothing at all.
        assert!(glob_match("a*b", "ab"));
        assert!(glob_match("a*b*c", "axbyc"));
        assert!(!glob_match("a*b*c", "axbyd"));
        assert!(!glob_match("a*b", "xab"));
    }

    #[test]
    fn excluded_collections_are_dropped_without_being_unroutable() {
        let before = SKIPPED_UNROUTABLE.load(Ordering::Relaxed);

        let rec = json!({"_database": "prod", "_collection": "events"});
        assert_eq!(route(&rec, &args_excluding(&["events"])), None);

        // Dropping an excluded record must not count as unroutable — that
        // counter fails the process unless --allow-skipped is passed.
        assert_eq!(SKIPPED_UNROUTABLE.load(Ordering::Relaxed), before);

        // Anything not matched still routes.
        let rec = json!({"_database": "prod", "_collection": "users"});
        assert_eq!(
            route(&rec, &args_excluding(&["events"])),
            Some(("prod".to_string(), "users".to_string()))
        );
    }

    #[test]
    fn exclusion_matches_the_dump_name_not_the_c_override() {
        let rec = json!({"_database": "prod", "_collection": "events"});

        // -c rewrites every record into `merged`, but the operator excluded
        // `events` — the name in the file — so this record still goes.
        let mut a = args(None, Some("merged"), false, false);
        a.exclude_collection = vec!["events".to_string()];
        assert_eq!(route(&rec, &a), None);

        // And a record from another collection survives the same run.
        let other = json!({"_database": "prod", "_collection": "users"});
        assert_eq!(
            route(&other, &a),
            Some(("prod".to_string(), "merged".to_string()))
        );
    }

    #[test]
    fn exclusion_falls_back_to_the_resolved_name() {
        // A record with no `_collection` of its own is matched on the target
        // the flags gave it, otherwise -c records could never be excluded.
        let rec = json!({"_key": "1"});
        let mut a = args(Some("db"), Some("scratch"), false, false);
        a.exclude_collection = vec!["scratch".to_string()];
        assert_eq!(route(&rec, &a), None);
    }

    #[test]
    fn no_exclusions_configured_routes_everything() {
        let rec = json!({"_database": "prod", "_collection": "events"});
        assert_eq!(
            route(&rec, &args(None, None, false, false)),
            Some(("prod".to_string(), "events".to_string()))
        );
    }

    #[test]
    fn path_seg_encodes_special_chars() {
        assert_eq!(path_seg("users"), "users");
        assert_eq!(path_seg("a b"), "a%20b");
    }
}
