use clap::Parser;
use reqwest;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::collections::HashMap;

#[derive(Parser, Debug)]
#[command(name = "solidb-restore")]
#[command(about = "Import SoliDB database or collection from JSONL dump", long_about = None)]
struct Args {
    /// Database host
    #[arg(short = 'H', long, default_value = "localhost")]
    host: String,

    /// Database port
    #[arg(short = 'P', long, default_value = "6745")]
    port: u16,

    /// Input file (JSONL dump)
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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let base_url = format!("http://{}:{}", args.host, args.port);

    let client = reqwest::Client::new();

    // Read Input file
    let file = File::open(&args.input)?;
    let mut reader = BufReader::new(file);

    // Peek to detect format
    // JSON Array: Starts with '['
    // JSONL: Starts with '{'
    // CSV: Anything else (assume header row)
    let mut format = "csv"; // default
    {
        // Check start of file for partial content to guess format
        let buf = reader.fill_buf()?;
        for &byte in buf {
            if !byte.is_ascii_whitespace() {
                if byte == b'[' {
                    format = "json_array";
                } else if byte == b'{' {
                    format = "jsonl";
                }
                break;
            }
        }
    }

    let all_documents: Vec<Value> = match format {
        "json_array" => {
            eprintln!("Detected JSON Array format");
            serde_json::from_reader(reader)?
        },
        "jsonl" => {
            eprintln!("Detected JSONL format");
            let mut docs = Vec::new();
            for line in reader.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                docs.push(serde_json::from_str(&line)?);
            }
            docs
        },
        _ => { // CSV
            eprintln!("Detected CSV format");
            
            // For CSV, we MUST look for overrides early or validation will fail later
            // logic below will handle regular processing, but let's check args here strictly if needed.
            // Actually, existing logic allows specifying DB/Coll later, but for CSV we have no metadata in file.
            // We'll validate this later or let the user provide them.
            
            // Reset reader position because we peeked? 
            // BufReader::fill_buf doesn't consume, but we need to re-create or seek?
            // Actually fill_buf does not advance, but we need to pass the reader to csv.
            // But serde_json took ownership of reader in other branches.
            // We need to seek back to 0 if we read anything?
            // Logic above: `reader.fill_buf` peeks the buffer. `from_reader` uses the reader.
            // If we use the SAME reader, it's fine as long as we didn't consume.
            // `fill_buf` returns the buffer but doesn't consume it. We need `consume` to advance.
            // We didn't call consume. So we are at the beginning.
            
            let mut csv_rdr = csv::Reader::from_reader(reader);
            let headers = csv_rdr.headers()?.clone();
            
            let mut docs = Vec::new();
            for result in csv_rdr.records() {
                let record = result?;
                let mut map = serde_json::Map::new();
                
                for (i, field) in record.iter().enumerate() {
                    let header = &headers[i];
                    // Attempt type inference
                    let value = if let Ok(n) = field.parse::<i64>() {
                        serde_json::Value::Number(n.into())
                    } else if let Ok(n) = field.parse::<f64>() {
                        if let Some(n_val) = serde_json::Number::from_f64(n) {
                            serde_json::Value::Number(n_val)
                        } else {
                            serde_json::Value::String(field.to_string())
                        }
                    } else if let Ok(b) = field.parse::<bool>() {
                        serde_json::Value::Bool(b)
                    } else {
                        serde_json::Value::String(field.to_string())
                    };
                    
                    map.insert(header.to_string(), value);
                }
                docs.push(Value::Object(map));
            }
            docs
        }
    };

    let mut collections: HashMap<String, Vec<Value>> = HashMap::new();
    let mut shard_configs: HashMap<String, Value> = HashMap::new();
    let mut database_name = None;

    for doc in all_documents {
        
        let doc: Value = doc; // Just to keep type clear, though redundant
        
        // Extract metadata - For CSV, these might be missing!
        let db_opt = doc.get("_database").and_then(|v| v.as_str());
        let coll_opt = doc.get("_collection").and_then(|v| v.as_str());

        // Strategy: 
        // 1. Try to get from document.
        // 2. If missing, check arguments.
        // 3. If missing, Error.
        
        // HOWEVER, the logic below was structure to Group By Collection first.
        // If we don't have _collection in doc, we must use the Arg.
        
        let db = match db_opt {
            Some(d) => d.to_string(),
            None => {
                // If not in doc, must be in args. But args.database is passed to `target_database` later.
                // We use `database_name` variable to track the "dump's database".
                // If it's pure CSV being imported to a specific DB via CLI, we can pretend it came from that DB.
                // Or better: If `_database` is missing, we use a placeholder or the CLI arg.
                // Let's use the CLI arg if available, or error.
                if let Some(arg_db) = &args.database {
                    arg_db.clone()
                } else {
                     return Err("Document missing _database field and --database argument not provided".into());
                }
            }
        };
        
        // Similar for collection
        let coll = match coll_opt {
            Some(c) => c.to_string(),
            None => {
                if let Some(arg_coll) = &args.collection {
                    arg_coll.clone()
                } else {
                    return Err("Document missing _collection field and --collection argument not provided".into());
                }
            }
        };

        if database_name.is_none() {
            database_name = Some(db.clone());
        }

        // Store shard config if present
        if let Some(shard_config) = doc.get("_shardConfig") {
            shard_configs.entry(coll.to_string())
                .or_insert_with(|| shard_config.clone());
        }

        // Remove metadata fields and store document
        let mut clean_doc = doc.clone();
        if let Some(obj) = clean_doc.as_object_mut() {
            obj.remove("_database");
            obj.remove("_collection");
            obj.remove("_shardConfig");
        }

        collections.entry(coll.to_string())
            .or_insert_with(Vec::new)
            .push(clean_doc);
    }

    let target_database = args.database.as_deref()
        .or(database_name.as_deref())
        .ok_or("Database name not specified and not found in dump")?;

    eprintln!("Restoring to database: {}", target_database);
    eprintln!("Found {} collections", collections.len());

    // Create database if requested
    if args.create_database {
        create_database_if_not_exists(&client, &base_url, target_database).await?;
    }

    // Restore each collection
    for (coll_name, documents) in &collections {
        let target_collection = if let Some(override_name) = &args.collection {
            if collections.len() > 1 {
                return Err("Cannot use --collection with multiple collections in dump".into());
            }
            override_name.as_str()
        } else {
            coll_name.as_str()
        };

        let shard_config = shard_configs.get(coll_name);
        restore_collection(
            &client,
            &base_url,
            target_database,
            target_collection,
            documents,
            shard_config,
            args.drop
        ).await?;
    }

    eprintln!("✓ Restore completed successfully");

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
use indicatif::{ProgressBar, ProgressStyle};

async fn restore_collection(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    collection_name: &str,
    documents: &[Value],
    shard_config: Option<&Value>,
    drop_existing: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("{} {}", "Restoring collection:".blue(), collection_name.white());

    // Drop existing collection if requested
    if drop_existing {
        let url = format!("{}/_api/database/{}/collection/{}", base_url, database, collection_name);
        let response = client.delete(&url).send().await?;
        
        if response.status().is_success() || response.status().as_u16() == 404 {
            eprintln!("  {}", "Dropped existing collection".yellow());
        }
    }

    // Create collection with shard config if present
    let url = format!("{}/_api/database/{}/collection", base_url, database);
    
    let mut create_payload = serde_json::json!({ "name": collection_name });
    
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

    let response = client
        .post(&url)
        .json(&create_payload)
        .send()
        .await?;

    if !response.status().is_success() && response.status().as_u16() != 409 {
        return Err(format!("Failed to create collection: {}", response.status()).into());
    }

    if response.status().is_success() {
        eprintln!("  {}", "Created collection".green());
    } else {
        eprintln!("  {}", "Collection already exists".yellow());
    }

    // Chunk and Upload
    eprintln!("  Uploading Batches of {} documents (batch size: 10,000)...", documents.len());
    let pb = ProgressBar::new(documents.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")?
        .progress_chars("#>-"));

    let chunk_size = 10_000;
    let url = format!("{}/_api/database/{}/collection/{}/import", base_url, database, collection_name);

    let mut total_imported = 0;
    let mut total_failed = 0;

    for chunk in documents.chunks(chunk_size) {
        let mut jsonl_data = Vec::with_capacity(chunk.len() * 100);
        for doc in chunk {
            serde_json::to_writer(&mut jsonl_data, doc)?;
            jsonl_data.push(b'\n');
        }

        // Create multipart form
        let part = reqwest::multipart::Part::bytes(jsonl_data)
            .file_name("restore.jsonl")
            .mime_str("application/x-ndjson")?;
            
        let form = reqwest::multipart::Form::new()
            .part("file", part);

        let response = client
            .post(&url)
            .multipart(form)
            .send()
            .await?;

        if !response.status().is_success() {
            pb.finish_with_message("Failed");
            return Err(format!("Failed to import batch: {}", response.status()).into());
        }

        let result: serde_json::Value = response.json().await?;
        total_imported += result["imported"].as_u64().unwrap_or(0);
        total_failed += result["failed"].as_u64().unwrap_or(0);
        
        pb.inc(chunk.len() as u64);
    }
    
    pb.finish_with_message("Done");

    eprintln!("  → {} documents restored successfully", total_imported.to_string().green());
    if total_failed > 0 {
        eprintln!("  → {} documents failed", total_failed.to_string().red().bold());
    }
    
    Ok(())
}
