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

    if let Some(collection_name) = &args.collection {
        // Dump single collection
        dump_collection_jsonl(
            &client,
            &base_url,
            &args.database,
            collection_name,
            &mut output,
            None,
        )
        .await?;
    } else {
        // Dump all collections
        dump_database_jsonl(&client, &base_url, &args.database, &mut output).await?;
    }

    if let Some(output) = &args.output {
        eprintln!("✓ Dump written to {}", output);
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
        let stats_url = format!(
            "{}/_api/database/{}/collection/{}/stats",
            base_url, database, collection
        );
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
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
            )?
            .progress_chars("#>-"),
    );

    // Check collection type to decide dump method
    let collection_type = collection_info["type"].as_str().unwrap_or("document");

    if collection_type == "blob" {
        eprintln!("  Using streaming export for blob collection...");
        let export_url = format!(
            "{}/_api/database/{}/collection/{}/export",
            base_url, database, collection
        );
        let mut response = client.get(&export_url).send().await?;

        if !response.status().is_success() {
            return Err(format!("Failed to export collection: {}", response.status()).into());
        }

        // Stream response to output
        while let Some(chunk) = response.chunk().await? {
            output.write_all(&chunk)?;
            // Update progress bar roughly? bytes?
            // Without total size, we can only spin or count bytes
            pb.inc(chunk.len() as u64 / 100); // Rough approximation for doc count? No, just spinning
        }
    } else {
        // Standard SDBQL dump for document/edge collections
        let query = format!("FOR doc IN {} RETURN doc", collection);
        let query_url = format!("{}/_api/database/{}/cursor", base_url, database);

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
    }

    pb.finish_with_message("Done");

    Ok(())
}
