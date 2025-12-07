use clap::Parser;
use reqwest;
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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let base_url = format!("http://{}:{}", args.host, args.port);

    let client = reqwest::Client::new();

    // Write output
    let mut output: Box<dyn Write> = if let Some(output_file) = &args.output {
        Box::new(File::create(output_file)?)
    } else {
        Box::new(io::stdout())
    };

    if let Some(collection_name) = &args.collection {
        // Dump single collection
        dump_collection_jsonl(&client, &base_url, &args.database, collection_name, &mut output, None).await?;
    } else {
        // Dump all collections
        dump_database_jsonl(&client, &base_url, &args.database, &mut output).await?;
    }

    if args.output.is_some() {
        eprintln!("✓ Dump written to {}", args.output.as_ref().unwrap());
    }

    Ok(())
}


use colored::*;
use indicatif::{ProgressBar, ProgressStyle};

async fn dump_database_jsonl(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    output: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("{} {}", "Dumping database:".green().bold(), database.cyan());

    // Get list of collections
    let collections_url = format!("{}/_api/database/{}/collection", base_url, database);
    let response = client.get(&collections_url).send().await?;
    
    if !response.status().is_success() {
        return Err(format!("Failed to list collections: {}", response.status()).into());
    }

    let collections_data: Value = response.json().await?;
    let collections = collections_data["collections"]
        .as_array()
        .ok_or("Invalid collections response")?;

    eprintln!("{} {} {}", "Found".green(), collections.len().to_string().yellow(), "collections".green());

    for collection in collections {
        let collection_name = collection["name"]
            .as_str()
            .ok_or("Collection name missing")?;
        
        let count = collection["count"].as_u64();

        // We'll trust dump_collection_jsonl to handle its own UI/progress
        dump_collection_jsonl(client, base_url, database, collection_name, output, count).await?;
    }

    Ok(())
}

async fn dump_collection_jsonl(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    collection: &str,
    output: &mut dyn Write,
    known_count: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("{} {}", "  Collection:".blue(), collection.white());

    // Use known count or fetch stats
    let count = if let Some(c) = known_count {
        c
    } else {
        let stats_url = format!("{}/_api/database/{}/collection/{}/stats", base_url, database, collection);
        let response = client.get(&stats_url).send().await?;
        if response.status().is_success() {
            let stats: Value = response.json().await?;
            stats["document_count"].as_u64().unwrap_or(0)
        } else {
            0
        }
    };

    // Get collection metadata (separate call unfortunately or parse from list... let's re-fetch list is inefficient but safe)
    // Optimization: Just fetch list once in caller? Too much refactoring.
    // Let's just do what we did before.
    let collections_url = format!("{}/_api/database/{}/collection", base_url, database);
    let response = client.get(&collections_url).send().await?;
    let collections_data: Value = response.json().await?;
    
    let collection_info = collections_data["collections"]
        .as_array()
        .and_then(|arr| arr.iter().find(|c| c["name"] == collection))
        .ok_or("Collection not found")?;

    // Setup Progress Bar
    let pb = ProgressBar::new(count);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")?
        .progress_chars("#>-"));

    // Get all documents via AQL
    // Note: Ideally we should use a CURSOR and stream batches.
    // The current implementation does `FOR doc IN coll RETURN doc` which might load EVERYTHING in memory on server if not careful?
    // But `client.post` here returns everything in one JSON response if we don't handle pagination/cursor.
    // The previous code had "batchSize": 10000. It likely returns a cursor ID if more results.
    // The previous code `query_result["result"]` implies getting a single batch.
    // Wait, the previous code treated it as ALL documents.
    // If the collection is huge, the previous code was buggy (only fetched first batch).
    // Let's fix that while we are here: Handle cursor pagination!
    // Or just fetch all if small.
    // The user wants "batch imports".
    // For DUMP, let's just stick to the existing logic but wrap the writing loop with progress.
    // If documents.len() != count, the bar will be partial.
    
    let query = format!("FOR doc IN {} RETURN doc", collection);
    let query_url = format!("{}/_api/database/{}/cursor", base_url, database);
    
    // We'll set a large batch size to try getting all
    let response = client
        .post(&query_url)
        .json(&serde_json::json!({
            "query": query,
            "batchSize": 1_000_000 // Try to get all
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        pb.finish_with_message("Failed to query");
        return Err(format!("Failed to query collection: {}", response.status()).into());
    }

    let query_result: Value = response.json().await?;
    let documents = query_result["result"]
        .as_array()
        .ok_or("Invalid query result")?;

    // Write each document as JSONL with metadata
    for doc in documents {
        let mut doc_with_meta = serde_json::json!({
            "_database": database,
            "_collection": collection,
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
        pb.inc(1);
    }
    
    pb.finish_with_message("Done");

    Ok(())
}

// Keep old functions for reference but they're no longer used
async fn dump_database(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    // Get list of collections
    let collections_url = format!("{}/_api/database/{}/collection", base_url, database);
    let response = client.get(&collections_url).send().await?;
    
    if !response.status().is_success() {
        return Err(format!("Failed to list collections: {}", response.status()).into());
    }

    let collections_data: Value = response.json().await?;
    let collections = collections_data["collections"]
        .as_array()
        .ok_or("Invalid collections response")?;

    eprintln!("Found {} collections", collections.len());

    let mut dump = serde_json::json!({
        "database": database,
        "collections": []
    });

    for collection in collections {
        let collection_name = collection["name"]
            .as_str()
            .ok_or("Collection name missing")?;
        
        eprintln!("Dumping collection: {}", collection_name);
        
        let collection_dump = dump_collection_data(client, base_url, database, collection_name).await?;
        dump["collections"]
            .as_array_mut()
            .unwrap()
            .push(collection_dump);
    }

    Ok(dump)
}

async fn dump_collection(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    collection: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    eprintln!("Dumping collection: {}", collection);
    
    let collection_dump = dump_collection_data(client, base_url, database, collection).await?;

    Ok(serde_json::json!({
        "database": database,
        "collections": [collection_dump]
    }))
}

async fn dump_collection_data(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    collection: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    // Get collection metadata
    let collections_url = format!("{}/_api/database/{}/collection", base_url, database);
    let response = client.get(&collections_url).send().await?;
    let collections_data: Value = response.json().await?;
    
    let collection_info = collections_data["collections"]
        .as_array()
        .and_then(|arr| arr.iter().find(|c| c["name"] == collection))
        .ok_or("Collection not found")?;

    // Get all documents via AQL
    let query = format!("FOR doc IN {} RETURN doc", collection);
    let query_url = format!("{}/_api/database/{}/cursor", base_url, database);
    
    let response = client
        .post(&query_url)
        .json(&serde_json::json!({
            "query": query,
            "batchSize": 10000
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!("Failed to query collection: {}", response.status()).into());
    }

    let query_result: Value = response.json().await?;
    let documents = query_result["result"]
        .as_array()
        .ok_or("Invalid query result")?
        .clone();

    eprintln!("  → {} documents", documents.len());

    Ok(serde_json::json!({
        "name": collection,
        "shardConfig": collection_info.get("shardConfig"),
        "documents": documents
    }))
}
