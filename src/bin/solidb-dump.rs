use clap::Parser;
use serde_json::Value;
use std::fs::File;
use std::io::{self, Write};

#[derive(Parser, Debug)]
#[command(name = "solidb-dump")]
#[command(about = "Export SoliDB database or collection to JSON", long_about = None)]
struct Args {
    /// Database host
    #[arg(short = 'H', long, default_value = "localhost")]
    host: String,

    /// Database port
    #[arg(short = 'P', long, default_value = "6745")]
    port: u16,

    /// Database name
    #[arg(short, long)]
    database: String,

    /// Collection name (if not specified, dumps all collections)
    #[arg(short, long)]
    collection: Option<String>,

    /// Output file (if not specified, writes to stdout)
    #[arg(short, long)]
    output: Option<String>,

    /// Pretty-print JSON
    #[arg(long)]
    pretty: bool,

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

    // Write output
    let mut output: Box<dyn Write> = if let Some(output_file) = &args.output {
        Box::new(File::create(output_file)?)
    } else {
        Box::new(io::stdout())
    };

    // The collection list is the single source of truth for per-collection
    // metadata (type, count, shard config). Fetch it once: the previous code
    // re-fetched it for every collection, which is O(n²) requests.
    let collections = fetch_collections(&client, &base_url, &args.database).await?;

    if let Some(collection_name) = &args.collection {
        // Dump single collection. A columnar collection is not in this list
        // under its own name, so fall back to the columnar API before giving up.
        match collections
            .iter()
            .find(|c| c["name"] == collection_name.as_str())
        {
            Some(info) => {
                dump_collection_jsonl(
                    &client,
                    &base_url,
                    &args.database,
                    collection_name,
                    &mut output,
                    info,
                )
                .await?;
            }
            None => {
                dump_columnar_collection(
                    &client,
                    &base_url,
                    &args.database,
                    collection_name,
                    &mut output,
                )
                .await
                .map_err(|e| format!("Collection '{}' not found: {}", collection_name, e))?;
            }
        }
    } else {
        // Dump all collections
        dump_database_jsonl(
            &client,
            &base_url,
            &args.database,
            &mut output,
            &collections,
        )
        .await?;
    }

    if let Some(output) = &args.output {
        eprintln!("✓ Dump written to {}", output);
    }

    Ok(())
}

use colored::*;
use indicatif::{ProgressBar, ProgressStyle};

/// Server-side cap on `batchSize` for `POST /_api/database/{db}/cursor`
/// (`MAX_BATCH_SIZE` in `src/server/handlers/query.rs`). Asking for more is
/// silently clamped, so request exactly the cap and page through the cursor.
const CURSOR_BATCH_SIZE: usize = 10_000;

/// Column-family prefix used to back a columnar collection. These appear in
/// the ordinary collection list but must be dumped through the /columnar API.
const COLUMNAR_CF_PREFIX: &str = "_columnar_";

async fn fetch_collections(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let collections_url = format!("{}/_api/database/{}/collection", base_url, database);
    let response = client.get(&collections_url).send().await?;

    if !response.status().is_success() {
        return Err(format!("Failed to list collections: {}", response.status()).into());
    }

    let collections_data: Value = response.json().await?;
    Ok(collections_data["collections"]
        .as_array()
        .ok_or("Invalid collections response")?
        .clone())
}

async fn dump_database_jsonl(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    output: &mut dyn Write,
    collections: &[Value],
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("{} {}", "Dumping database:".green().bold(), database.cyan());

    eprintln!(
        "{} {} {}",
        "Found".green(),
        collections.len().to_string().yellow(),
        "collections".green()
    );

    for collection in collections {
        let collection_name = collection["name"]
            .as_str()
            .ok_or("Collection name missing")?;

        // Columnar collections are backed by a `_columnar_<name>` column
        // family that shows up in this list as an empty document collection.
        // Dumping it would emit a phantom collection and none of the actual
        // columnar data; they are dumped properly via /columnar below.
        if collection_name.starts_with(COLUMNAR_CF_PREFIX) {
            continue;
        }

        // We'll trust dump_collection_jsonl to handle its own UI/progress
        dump_collection_jsonl(
            client,
            base_url,
            database,
            collection_name,
            output,
            collection,
        )
        .await?;
    }

    dump_columnar_collections(client, base_url, database, output).await?;

    Ok(())
}

/// Dump every columnar collection in the database.
///
/// Columnar collections live behind their own `/columnar` API and are invisible
/// to the document endpoints, so they need a separate pass: schema first, then
/// explicit indexes, then rows.
async fn dump_columnar_collections(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    output: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let list_url = format!("{}/_api/database/{}/columnar", base_url, database);
    let response = match client.get(&list_url).send().await {
        Ok(r) if r.status().is_success() => r,
        // A server too old to know about columnar collections has nothing to
        // dump here; anything else is worth reporting but not fatal.
        Ok(r) if r.status().as_u16() == 404 => return Ok(()),
        Ok(r) => {
            eprintln!(
                "  {} could not list columnar collections: {}",
                "Warning:".yellow().bold(),
                r.status()
            );
            return Ok(());
        }
        Err(e) => {
            eprintln!(
                "  {} could not list columnar collections: {}",
                "Warning:".yellow().bold(),
                e
            );
            return Ok(());
        }
    };

    let body: Value = response.json().await?;
    let names: Vec<String> = body["collections"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if names.is_empty() {
        return Ok(());
    }

    eprintln!(
        "{} {} {}",
        "Found".green(),
        names.len().to_string().yellow(),
        "columnar collections".green()
    );

    for name in names {
        dump_columnar_collection(client, base_url, database, &name, output).await?;
    }

    Ok(())
}

async fn dump_columnar_collection(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    collection: &str,
    output: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("{} {}", "  Columnar:".blue(), collection.white());

    // Schema. The detail endpoint is used rather than the list because it
    // serialises `compression` as the lowercase form the create endpoint
    // accepts ("lz4"), where the list emits Rust's Debug form ("Lz4").
    let detail_url = format!(
        "{}/_api/database/{}/columnar/{}",
        base_url, database, collection
    );
    let response = client.get(&detail_url).send().await?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to read columnar collection '{}': {}",
            collection,
            response.status()
        )
        .into());
    }
    let detail: Value = response.json().await?;

    // The create endpoint wants {name, type, nullable, indexed}; the read side
    // returns `data_type` rather than `type`.
    let columns: Vec<Value> = detail["columns"]
        .as_array()
        .ok_or("Columnar collection has no column definitions")?
        .iter()
        .map(|c| {
            serde_json::json!({
                "name": c.get("name").cloned().unwrap_or(Value::Null),
                "type": c.get("data_type").cloned().unwrap_or(Value::Null),
                "nullable": c.get("nullable").cloned().unwrap_or(Value::Bool(false)),
                "indexed": c.get("indexed").cloned().unwrap_or(Value::Bool(false)),
            })
        })
        .collect();

    let declaration = serde_json::json!({
        "_type": "columnar",
        "_database": database,
        "_collection": collection,
        "columns": columns,
        "compression": detail.get("compression").cloned().unwrap_or(Value::Null),
    });
    writeln!(output, "{}", serde_json::to_string(&declaration)?)?;

    // Explicitly created indexes. Columns flagged `indexed` in the schema get
    // their index back from the declaration above, so those are not repeated
    // here.
    let indexes_url = format!(
        "{}/_api/database/{}/columnar/{}/indexes",
        base_url, database, collection
    );
    if let Ok(resp) = client.get(&indexes_url).send().await {
        if resp.status().is_success() {
            if let Ok(body) = resp.json::<Value>().await {
                if let Some(arr) = body["indexes"].as_array() {
                    for idx in arr {
                        let record = serde_json::json!({
                            "_type": "columnar_index",
                            "_database": database,
                            "_collection": collection,
                            "column": idx.get("column").cloned().unwrap_or(Value::Null),
                            "index_type": idx.get("index_type").cloned().unwrap_or(Value::Null),
                        });
                        writeln!(output, "{}", serde_json::to_string(&record)?)?;
                    }
                }
            }
        }
    }

    // Rows. The query endpoint returns everything when no limit is given.
    let column_names: Vec<&str> = columns.iter().filter_map(|c| c["name"].as_str()).collect();
    let query_url = format!(
        "{}/_api/database/{}/columnar/{}/query",
        base_url, database, collection
    );
    let response = client
        .post(&query_url)
        .json(&serde_json::json!({ "columns": column_names }))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to read rows of columnar collection '{}': {}",
            collection,
            response.status()
        )
        .into());
    }
    let body: Value = response.json().await?;
    let rows = body["result"].as_array().ok_or("Invalid columnar result")?;

    // Rows are nested under `row` rather than merged into the record, because
    // a column may legitimately be named `_type` or `_collection`.
    for row in rows {
        let record = serde_json::json!({
            "_type": "columnar_row",
            "_database": database,
            "_collection": collection,
            "row": row,
        });
        writeln!(output, "{}", serde_json::to_string(&record)?)?;
    }

    let expected = detail["row_count"].as_u64().unwrap_or(0);
    if expected != rows.len() as u64 {
        eprintln!(
            "  {} {} reports {} rows but {} were dumped",
            "Warning:".yellow().bold(),
            collection,
            expected,
            rows.len()
        );
    }

    Ok(())
}

async fn dump_collection_jsonl(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    collection: &str,
    output: &mut dyn Write,
    collection_info: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("{} {}", "  Collection:".blue(), collection.white());

    let count = collection_info["count"].as_u64().unwrap_or(0);

    // Setup Progress Bar
    let pb = ProgressBar::new(count);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
            )?
            .progress_chars("#>-"),
    );

    // Check collection type to decide dump method
    let collection_type = collection_info["type"].as_str().unwrap_or("document");

    // Declare the collection before anything else. Without this record a
    // collection holding no documents and no indexes writes nothing at all,
    // so it vanishes from the dump and is missing after a restore. It also
    // carries the type ("edge", "blob", "timeseries"), which is otherwise
    // only inferable from the documents that follow — and not at all when
    // there are none.
    let mut declaration = serde_json::json!({
        "_type": "collection",
        "_database": database,
        "_collection": collection,
        "_collectionType": collection_type,
    });
    if let Some(shard_config) = collection_info.get("shardConfig") {
        declaration["_shardConfig"] = shard_config.clone();
    }
    writeln!(output, "{}", serde_json::to_string(&declaration)?)?;

    // Export index definitions before documents so they exist by the time
    // documents are imported back (and so they can be applied even if a
    // collection is empty)
    dump_collection_indexes(
        client,
        base_url,
        database,
        collection,
        collection_type,
        output,
    )
    .await?;

    if collection_type == "blob" {
        eprintln!("  Using streaming export for blob collection...");
        dump_blob_collection(
            client,
            base_url,
            database,
            collection,
            collection_info,
            output,
            &pb,
        )
        .await?;
    } else {
        // Standard SDBQL dump for document/edge collections.
        // The server caps batchSize at CURSOR_BATCH_SIZE and returns a cursor
        // id when more results remain — page through it, otherwise every
        // collection larger than the cap is silently truncated.
        let query = format!("FOR doc IN {} RETURN doc", collection);
        let query_url = format!("{}/_api/database/{}/cursor", base_url, database);

        let response = client
            .post(&query_url)
            .json(&serde_json::json!({
                "query": query,
                "batchSize": CURSOR_BATCH_SIZE,
                // Dumps must see current data, never a cached result set
                "cache": false
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            pb.finish_with_message("Failed to query");
            return Err(format!("Failed to query collection: {}", response.status()).into());
        }

        let mut query_result: Value = response.json().await?;
        let mut dumped: u64 = 0;

        loop {
            let documents = query_result["result"]
                .as_array()
                .ok_or("Invalid query result")?;

            // Write each document as JSONL with metadata
            for doc in documents {
                let mut doc_with_meta = serde_json::json!({
                    "_database": database,
                    "_collection": collection,
                    "_collectionType": collection_type,
                });

                // Add shard config if present
                if let Some(shard_config) = collection_info.get("shardConfig") {
                    doc_with_meta["_shardConfig"] = shard_config.clone();
                }

                // Merge document data
                if let Some(obj) = doc.as_object() {
                    for (k, v) in obj {
                        doc_with_meta[k] = v.clone();
                    }
                }

                writeln!(output, "{}", serde_json::to_string(&doc_with_meta)?)?;
                dumped += 1;
                pb.inc(1);
            }

            let has_more = query_result["has_more"].as_bool().unwrap_or(false);
            let cursor_id = query_result["id"].as_str().map(String::from);

            let (has_more, cursor_id) = match (has_more, cursor_id) {
                (true, Some(id)) => (true, id),
                _ => break,
            };
            let _ = has_more;

            // PUT /_api/cursor/{id} returns the next batch
            let next_url = format!("{}/_api/cursor/{}", base_url, cursor_id);
            let response = client.put(&next_url).send().await?;
            if !response.status().is_success() {
                pb.finish_with_message("Cursor failed");
                return Err(format!(
                    "Failed to fetch next batch for '{}' after {} documents: {}",
                    collection,
                    dumped,
                    response.status()
                )
                .into());
            }
            query_result = response.json().await?;
        }

        // The reported count is a running statistic and can legitimately drift
        // from what a scan returns; only warn so the operator can check.
        if count > 0 && dumped != count {
            eprintln!(
                "  {} {} reports {} documents but {} were dumped",
                "Warning:".yellow().bold(),
                collection,
                count,
                dumped
            );
        }
    }

    pb.finish_with_message("Done");

    Ok(())
}

/// Stream a blob collection's export, injecting the routing metadata that
/// `solidb-restore` needs on every record.
///
/// The server's `/export` endpoint emits records that identify neither the
/// database nor the collection — it is a single-collection endpoint, so it has
/// no reason to. Passing that stream through verbatim (what this tool used to
/// do) produced a dump whose blob records had no `_database`/`_collection`,
/// and restore aborted on the first one with "No collection specified in doc
/// or args".
///
/// The stream is framed: JSON lines, except a line with
/// `{"_type":"blob_chunk", "_data_length":N}` is followed by exactly N raw
/// bytes and a newline. Binary payloads are copied through byte for byte;
/// only the JSON headers are rewritten.
async fn dump_blob_collection(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    collection: &str,
    collection_info: &Value,
    output: &mut dyn Write,
    pb: &ProgressBar,
) -> Result<(), Box<dyn std::error::Error>> {
    let export_url = format!(
        "{}/_api/database/{}/collection/{}/export",
        base_url, database, collection
    );
    let mut response = client.get(&export_url).send().await?;

    if !response.status().is_success() {
        return Err(format!("Failed to export collection: {}", response.status()).into());
    }

    let mut buffer: Vec<u8> = Vec::new();
    let mut eof = false;
    let mut chunk_count: u64 = 0;

    // Pull more bytes from the response into `buffer`; returns false at EOF.
    macro_rules! fill {
        () => {
            match response.chunk().await? {
                Some(bytes) => {
                    buffer.extend_from_slice(&bytes);
                    true
                }
                None => false,
            }
        };
    }

    loop {
        // Extract one line
        let newline_pos = loop {
            if let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                break Some(pos);
            }
            if eof {
                break None;
            }
            if !fill!() {
                eof = true;
            }
        };

        let line_bytes: Vec<u8> = match newline_pos {
            Some(pos) => buffer.drain(0..=pos).collect(),
            None => {
                // Trailing bytes with no newline: emit as a final line if
                // they hold anything, then stop.
                if buffer.iter().all(|b| b.is_ascii_whitespace()) {
                    break;
                }
                std::mem::take(&mut buffer)
            }
        };

        let line_slice = line_bytes.strip_suffix(b"\n").unwrap_or(&line_bytes);

        if line_slice.iter().all(|b| b.is_ascii_whitespace()) {
            if newline_pos.is_none() {
                break;
            }
            continue;
        }

        let mut record: Value = match serde_json::from_slice(line_slice) {
            Ok(v) => v,
            Err(e) => {
                return Err(format!(
                    "Malformed export stream for '{}': {} (offending line: {:.200})",
                    collection,
                    e,
                    String::from_utf8_lossy(line_slice)
                )
                .into());
            }
        };

        let is_blob_chunk = record.get("_type").and_then(|t| t.as_str()) == Some("blob_chunk");
        let data_length = record.get("_data_length").and_then(|v| v.as_u64());

        // Inject the routing metadata restore needs on every record.
        if let Some(obj) = record.as_object_mut() {
            obj.insert("_database".to_string(), Value::String(database.to_string()));
            obj.insert(
                "_collection".to_string(),
                Value::String(collection.to_string()),
            );
            obj.insert(
                "_collectionType".to_string(),
                Value::String("blob".to_string()),
            );
            if let Some(shard_config) = collection_info.get("shardConfig") {
                obj.entry("_shardConfig".to_string())
                    .or_insert_with(|| shard_config.clone());
            }
        }

        writeln!(output, "{}", serde_json::to_string(&record)?)?;

        // A chunk header is followed by exactly `_data_length` raw bytes plus
        // a newline; copy them through untouched.
        if is_blob_chunk {
            if let Some(len) = data_length {
                let len = len as usize;
                while buffer.len() < len + 1 {
                    if !fill!() {
                        eof = true;
                        break;
                    }
                }
                if buffer.len() < len {
                    return Err(format!(
                        "Truncated export stream for '{}': expected {} bytes of blob data for \
                         key '{}' chunk {}, got {}",
                        collection,
                        len,
                        record
                            .get("_doc_key")
                            .and_then(|v| v.as_str())
                            .unwrap_or("<unknown>"),
                        record
                            .get("_chunk_index")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        buffer.len()
                    )
                    .into());
                }
                let data: Vec<u8> = buffer.drain(0..len).collect();
                output.write_all(&data)?;
                output.write_all(b"\n")?;
                // Consume the delimiter the server wrote after the payload
                if !buffer.is_empty() && buffer[0] == b'\n' {
                    buffer.drain(0..1);
                }
                chunk_count += 1;
            }
        } else {
            pb.inc(1);
        }

        if newline_pos.is_none() {
            break;
        }
        if eof && buffer.is_empty() {
            break;
        }
    }

    if chunk_count > 0 {
        eprintln!(
            "    {} blob chunks exported",
            chunk_count.to_string().cyan()
        );
    }

    Ok(())
}

/// Dump index definitions for a collection as `_type: "index"` records.
///
/// Covers all index kinds exposed by the HTTP API: regular (hash, persistent,
/// fulltext, bloom, cuckoo), geo, vector, and TTL. Each record includes the
/// minimum fields needed to recreate the index via the matching POST endpoint.
async fn dump_collection_indexes(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    collection: &str,
    collection_type: &str,
    output: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    // Regular indexes (hash, persistent, fulltext, bloom, cuckoo)
    let url = format!(
        "{}/_api/database/{}/index/{}",
        base_url, database, collection
    );
    if let Ok(resp) = client.get(&url).send().await {
        if resp.status().is_success() {
            if let Ok(body) = resp.json::<Value>().await {
                if let Some(arr) = body["indexes"].as_array() {
                    for idx in arr {
                        // Server-side IndexType is serialized with PascalCase
                        // ("Persistent", "Hash", ...). Lowercase it for the
                        // create endpoint, which expects strings like
                        // "persistent", "hash", "fulltext".
                        let kind = idx["index_type"]
                            .as_str()
                            .map(|s| s.to_lowercase())
                            .unwrap_or_else(|| "persistent".to_string());
                        let mut record = serde_json::json!({
                            "_type": "index",
                            "_database": database,
                            "_collection": collection,
                            "_collectionType": collection_type,
                            "_index_kind": kind,
                            "name": idx.get("name").cloned().unwrap_or(Value::Null),
                            "fields": idx.get("fields").cloned().unwrap_or_else(|| Value::Array(vec![])),
                            "unique": idx.get("unique").cloned().unwrap_or(Value::Bool(false)),
                        });
                        if let Some(field) = idx.get("field") {
                            record["field"] = field.clone();
                        }
                        writeln!(output, "{}", serde_json::to_string(&record)?)?;
                    }
                }
            }
        }
    }

    // Geo indexes
    let url = format!("{}/_api/database/{}/geo/{}", base_url, database, collection);
    if let Ok(resp) = client.get(&url).send().await {
        if resp.status().is_success() {
            if let Ok(body) = resp.json::<Value>().await {
                if let Some(arr) = body["indexes"].as_array() {
                    for idx in arr {
                        let record = serde_json::json!({
                            "_type": "index",
                            "_database": database,
                            "_collection": collection,
                            "_collectionType": collection_type,
                            "_index_kind": "geo",
                            "name": idx.get("name").cloned().unwrap_or(Value::Null),
                            "field": idx.get("field").cloned().unwrap_or(Value::Null),
                        });
                        writeln!(output, "{}", serde_json::to_string(&record)?)?;
                    }
                }
            }
        }
    }

    // TTL indexes
    let url = format!("{}/_api/database/{}/ttl/{}", base_url, database, collection);
    if let Ok(resp) = client.get(&url).send().await {
        if resp.status().is_success() {
            if let Ok(body) = resp.json::<Value>().await {
                if let Some(arr) = body["indexes"].as_array() {
                    for idx in arr {
                        let record = serde_json::json!({
                            "_type": "index",
                            "_database": database,
                            "_collection": collection,
                            "_collectionType": collection_type,
                            "_index_kind": "ttl",
                            "name": idx.get("name").cloned().unwrap_or(Value::Null),
                            "field": idx.get("field").cloned().unwrap_or(Value::Null),
                            "expire_after_seconds": idx.get("expire_after_seconds").cloned().unwrap_or(Value::Null),
                        });
                        writeln!(output, "{}", serde_json::to_string(&record)?)?;
                    }
                }
            }
        }
    }

    // Vector indexes
    let url = format!(
        "{}/_api/database/{}/vector/{}",
        base_url, database, collection
    );
    if let Ok(resp) = client.get(&url).send().await {
        if resp.status().is_success() {
            if let Ok(body) = resp.json::<Value>().await {
                if let Some(arr) = body["indexes"].as_array() {
                    for idx in arr {
                        // VectorMetric serializes as PascalCase ("Cosine", "Euclidean", "DotProduct")
                        // but the create endpoint expects lowercase ("cosine", "euclidean", "dot")
                        let metric = idx
                            .get("metric")
                            .and_then(|v| v.as_str())
                            .map(|s| match s.to_lowercase().as_str() {
                                "dotproduct" => "dot".to_string(),
                                other => other.to_string(),
                            })
                            .unwrap_or_else(|| "cosine".to_string());
                        let quantization = idx
                            .get("quantization")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_lowercase())
                            .unwrap_or_else(|| "none".to_string());
                        let record = serde_json::json!({
                            "_type": "index",
                            "_database": database,
                            "_collection": collection,
                            "_collectionType": collection_type,
                            "_index_kind": "vector",
                            "name": idx.get("name").cloned().unwrap_or(Value::Null),
                            "field": idx.get("field").cloned().unwrap_or(Value::Null),
                            "dimension": idx.get("dimension").cloned().unwrap_or(Value::Null),
                            "metric": metric,
                            "m": idx.get("m").cloned().unwrap_or(Value::Null),
                            "ef_construction": idx.get("ef_construction").cloned().unwrap_or(Value::Null),
                            "quantization": quantization,
                        });
                        writeln!(output, "{}", serde_json::to_string(&record)?)?;
                    }
                }
            }
        }
    }

    Ok(())
}
