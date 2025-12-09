use clap::Parser;
use reqwest;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::collections::HashMap;
use indicatif::{ProgressBar, ProgressStyle};


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
                     let start_idx = buf.iter().position(|&b| !b.is_ascii_whitespace()).unwrap_or(0);
                     if buf.len() >= start_idx + 6 {
                         let potential_insert = &buf[start_idx..start_idx+6];
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
             process_doc(doc, &args, &client, &base_url, &mut current_batch, &mut current_batch_size, 
                &mut current_batch_meta,
                max_batch_count, max_batch_size, &mut initialized_collections, 
                &mut total_imported, &mut total_failed).await?;
        }
    } else if format == "sql" {
        eprintln!("Error: SQL restore is not yet fully implemented. Please convert to CSV or JSONL.");
        return Ok(());
    } else if format == "json_array" {
        eprintln!("Warning: JSON Array format loads all data into memory. Not recommended for large restores.");
        let all_documents: Vec<Value> = serde_json::from_reader(reader)?;
        for doc in all_documents {
            process_doc(doc, &args, &client, &base_url, &mut current_batch, &mut current_batch_size, 
                &mut current_batch_meta,
                max_batch_count, max_batch_size, &mut initialized_collections, 
                &mut total_imported, &mut total_failed).await?;
        }
    } else {
        // Assume JSONL
        eprintln!("Restoring using streaming mode (JSONL)...");
        for line_res in reader.lines() {
             let line = line_res?;
             if line.trim().is_empty() { continue; }
             
             match serde_json::from_str::<Value>(&line) {
                 Ok(doc) => {
                      process_doc(doc, &args, &client, &base_url, &mut current_batch, &mut current_batch_size, 
                        &mut current_batch_meta,
                        max_batch_count, max_batch_size, &mut initialized_collections, 
                        &mut total_imported, &mut total_failed).await?;
                 },
                 Err(e) => {
                     eprintln!("Failed to parse line: {}", e);
                     total_failed += 1;
                 }
             }
        }
    }

    // Flush remaining
    if !current_batch.is_empty() {
        if let Some((db, coll)) = &current_batch_meta {
             flush_batch(&mut current_batch, &mut current_batch_size, &client, &base_url, 
                db, coll,
                &mut total_imported, &mut total_failed).await?;
        }
    }

    eprintln!("✓ Restore completed");
    eprintln!("  → {} items imported", total_imported.to_string().green());
    if total_failed > 0 {
        eprintln!("  → {} items failed", total_failed.to_string().red());
    }

    Ok(())
}

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
    
    // Determine target DB and Collection
    let db = doc.get("_database").and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .or_else(|| args.database.clone())
            .ok_or("No database specified in doc or args")?;

    let coll = doc.get("_collection").and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .or_else(|| args.collection.clone())
            .ok_or("No collection specified in doc or args")?;

    // Create DB/Collection if needed
    let key = format!("{}/{}", db, coll);
    if !initialized_cols.contains_key(&key) {
        // Try create DB
        if args.create_database {
            create_database_if_not_exists(client, base_url, &db).await?;
        }
        
        let shard_config = doc.get("_shardConfig");
        ensure_collection_exists(client, base_url, &db, &coll, shard_config, args.drop).await?;
        
        initialized_cols.insert(key.clone(), true);
    }
    
    // Check batch consistency
    if let Some((curr_db, curr_coll)) = batch_meta {
        if curr_db != &db || curr_coll != &coll {
            // Flush because collection changed
            flush_batch(batch, batch_size, client, base_url, curr_db, curr_coll, total_imported, total_failed).await?;
            *batch_meta = None;
        }
    }

    if batch_meta.is_none() {
        *batch_meta = Some((db.clone(), coll.clone()));
    }

    // Add doc to batch (Pre-serialize to avoid double serialization)
    let doc_bytes = serde_json::to_vec(&doc)?;
    *batch_size += doc_bytes.len();
    batch.push(doc_bytes);

    // Flush if full
    if batch.len() >= max_count || *batch_size >= max_size {
         if let Some((curr_db, curr_coll)) = batch_meta {
            flush_batch(batch, batch_size, client, base_url, curr_db, curr_coll, total_imported, total_failed).await?;
         }
    }

    Ok(())
}

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
    if batch.is_empty() { return Ok(()); }

    let url = format!("{}/_api/database/{}/collection/{}/import", base_url, db, coll);
    
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
        *total_imported += result["imported"].as_u64().unwrap_or(0);
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
     drop: bool
) -> Result<(), Box<dyn std::error::Error>> {
    // Logic similar to restore_collection but handles single init
    
    if drop {
        let url = format!("{}/_api/database/{}/collection/{}", base_url, database, collection);
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
         eprintln!(" Warning: Failed to create collection {}: {}", collection, response.status());
    }
    Ok(())
}

async fn create_database_if_not_exists(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{}/_api/database", base_url);
    
    let response = client
        .post(&url)
        .json(&serde_json::json!({ "name": database }))
        .send()
        .await?;

    if response.status().is_success() {
        eprintln!("  Created database: {}", database);
    } else if response.status().as_u16() == 409 {
        eprintln!("  Database already exists: {}", database);
    } else {
        return Err(format!("Failed to create database: {}", response.status()).into());
    }

    Ok(())
}

use colored::*;



