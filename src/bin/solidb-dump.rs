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
        dump_collection_jsonl(&client, &base_url, &args.database, collection_name, &mut output).await?;
    } else {
        // Dump all collections
        dump_database_jsonl(&client, &base_url, &args.database, &mut output).await?;
    }

    if args.output.is_some() {
        eprintln!("✓ Dump written to {}", args.output.as_ref().unwrap());
    }

    Ok(())
}


async fn dump_database_jsonl(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    output: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
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

    for collection in collections {
        let collection_name = collection["name"]
            .as_str()
            .ok_or("Collection name missing")?;
        
        eprintln!("Dumping collection: {}", collection_name);
        dump_collection_jsonl(client, base_url, database, collection_name, output).await?;
    }

    Ok(())
}

async fn dump_collection_jsonl(
    client: &reqwest::Client,
    base_url: &str,
    database: &str,
    collection: &str,
    output: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get collection metadata for shard config
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
        .ok_or("Invalid query result")?;

    eprintln!("  → {} documents", documents.len());

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
    }

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
