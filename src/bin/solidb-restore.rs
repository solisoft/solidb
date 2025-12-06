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
    #[arg(long)]
    database: Option<String>,

    /// Override collection name (only when restoring single collection)
    #[arg(long)]
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

    // Read JSONL file and group documents by collection
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);

    let mut collections: HashMap<String, Vec<Value>> = HashMap::new();
    let mut shard_configs: HashMap<String, Value> = HashMap::new();
    let mut database_name = None;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let doc: Value = serde_json::from_str(&line)?;
        
        // Extract metadata
        let db = doc.get("_database")
            .and_then(|v| v.as_str())
            .ok_or("Document missing _database field")?;
        
        let coll = doc.get("_collection")
            .and_then(|v| v.as_str())
            .ok_or("Document missing _collection field")?;

        if database_name.is_none() {
            database_name = Some(db.to_string());
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

async fn restore_collection(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    collection_name: &str,
    documents: &[Value],
    shard_config: Option<&Value>,
    drop_existing: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Restoring collection: {}", collection_name);

    // Drop existing collection if requested
    if drop_existing {
        let url = format!("{}/_api/database/{}/collection/{}", base_url, database, collection_name);
        let response = client.delete(&url).send().await?;
        
        if response.status().is_success() || response.status().as_u16() == 404 {
            eprintln!("  Dropped existing collection");
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
        eprintln!("  Created collection");
    } else {
        eprintln!("  Collection already exists");
    }

    // Restore documents
    eprintln!("  Restoring {} documents...", documents.len());

    let mut success_count = 0;
    let mut error_count = 0;

    for doc in documents {
        let url = format!("{}/_api/database/{}/document/{}", base_url, database, collection_name);
        
        let response = client
            .post(&url)
            .json(doc)
            .send()
            .await?;

        if response.status().is_success() {
            success_count += 1;
        } else {
            error_count += 1;
            if error_count <= 5 {
                eprintln!("    Warning: Failed to insert document: {}", response.status());
            }
        }
    }

    eprintln!("  → {} documents restored successfully", success_count);
    if error_count > 0 {
        eprintln!("  → {} documents failed", error_count);
    }

    Ok(())
}
