//! SoliDB Interactive REPL
//!
//! A command-line REPL (Read-Eval-Print Loop) for interacting with SoliDB
//! using Lua scripts. Similar to irb (Ruby) or lua -i.
//!
//! Usage: solidb-repl [OPTIONS]
//!
//! Options:
//!   -s, --server <URL>      Server URL (default: http://localhost:6745)
//!   -d, --database <NAME>   Database name (default: _system)
//!   -k, --api-key <KEY>     API key for authentication

use clap::Parser;
use colored::Colorize;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Editor, Helper};
use std::borrow::Cow;
use std::collections::HashMap;

#[derive(Parser, Debug)]
#[command(name = "solidb-repl")]
#[command(about = "SoliDB Interactive Lua REPL", long_about = None)]
struct Args {
    /// Server URL
    #[arg(short, long, default_value = "http://localhost:6745")]
    server: String,

    /// Database name
    #[arg(short, long, default_value = "_system")]
    database: String,

    /// API key for authentication
    #[arg(short = 'k', long)]
    api_key: Option<String>,
}

/// REPL response from server
#[derive(Debug, serde::Deserialize)]
struct ReplResponse {
    result: serde_json::Value,
    output: Vec<String>,
    error: Option<ReplError>,
    execution_time_ms: f64,
    session_id: String,
}

#[derive(Debug, serde::Deserialize)]
struct ReplError {
    message: String,
    line: Option<u32>,
}

/// Tab completion helper
struct SoliDBHelper {
    completions: Vec<String>,
}

impl SoliDBHelper {
    fn new() -> Self {
        let completions = vec![
            // solidb namespace
            "solidb.log".into(),
            "solidb.stats".into(),
            "solidb.now".into(),
            "solidb.fetch".into(),
            // db namespace
            "db:collection".into(),
            "db:query".into(),
            "db:transaction".into(),
            "db:enqueue".into(),
            // collection methods
            ":get".into(),
            ":insert".into(),
            ":update".into(),
            ":delete".into(),
            ":count".into(),
            // crypto namespace
            "crypto.md5".into(),
            "crypto.sha256".into(),
            "crypto.sha512".into(),
            "crypto.hmac_sha256".into(),
            "crypto.hmac_sha512".into(),
            "crypto.base64_encode".into(),
            "crypto.base64_decode".into(),
            "crypto.base32_encode".into(),
            "crypto.base32_decode".into(),
            "crypto.hex_encode".into(),
            "crypto.hex_decode".into(),
            "crypto.uuid".into(),
            "crypto.uuid_v7".into(),
            "crypto.random_bytes".into(),
            "crypto.curve25519".into(),
            "crypto.hash_password".into(),
            "crypto.verify_password".into(),
            "crypto.jwt_encode".into(),
            "crypto.jwt_decode".into(),
            // time namespace
            "time.now".into(),
            "time.now_ms".into(),
            "time.iso".into(),
            "time.sleep".into(),
            "time.format".into(),
            "time.parse".into(),
            "time.add".into(),
            "time.subtract".into(),
            // string extensions
            "string.regex".into(),
            "string.regex_replace".into(),
            // request/response
            "request.method".into(),
            "request.path".into(),
            "request.query".into(),
            "request.params".into(),
            "request.headers".into(),
            "request.body".into(),
            "response.json".into(),
            // Lua keywords
            "local".into(),
            "function".into(),
            "end".into(),
            "if".into(),
            "then".into(),
            "else".into(),
            "elseif".into(),
            "for".into(),
            "while".into(),
            "do".into(),
            "return".into(),
            "nil".into(),
            "true".into(),
            "false".into(),
            "and".into(),
            "or".into(),
            "not".into(),
            "print".into(),
        ];
        Self { completions }
    }
}

impl Completer for SoliDBHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Pair>), ReadlineError> {
        let start = line[..pos]
            .rfind(|c: char| c.is_whitespace() || c == '(' || c == ',')
            .map(|i| i + 1)
            .unwrap_or(0);

        let word = &line[start..pos];

        let matches: Vec<Pair> = self
            .completions
            .iter()
            .filter(|c| c.starts_with(word))
            .map(|c| Pair {
                display: c.clone(),
                replacement: c.clone(),
            })
            .collect();

        Ok((start, matches))
    }
}

impl Hinter for SoliDBHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<String> {
        if pos < line.len() {
            return None;
        }

        let start = line
            .rfind(|c: char| c.is_whitespace() || c == '(' || c == ',')
            .map(|i| i + 1)
            .unwrap_or(0);

        let word = &line[start..];
        if word.is_empty() {
            return None;
        }

        self.completions
            .iter()
            .find(|c| c.starts_with(word) && c.len() > word.len())
            .map(|c| c[word.len()..].to_string())
    }
}

impl Highlighter for SoliDBHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(hint.truecolor(100, 100, 100).to_string())
    }
}

impl Validator for SoliDBHelper {}

impl Helper for SoliDBHelper {}

struct ReplClient {
    server: String,
    database: String,
    api_key: Option<String>,
    session_id: Option<String>,
    client: reqwest::blocking::Client,
}

impl ReplClient {
    fn new(server: String, database: String, api_key: Option<String>) -> Self {
        Self {
            server,
            database,
            api_key,
            session_id: None,
            client: reqwest::blocking::Client::new(),
        }
    }

    fn execute(&mut self, code: &str) -> Result<ReplResponse, String> {
        let url = format!("{}/_api/database/{}/repl", self.server, self.database);

        let mut body = HashMap::new();
        body.insert("code", code);

        let session_id_string;
        if let Some(ref sid) = self.session_id {
            session_id_string = sid.clone();
            body.insert("session_id", &session_id_string);
        }

        let mut req = self.client.post(&url).json(&body);

        if let Some(ref key) = self.api_key {
            req = req.header("X-API-Key", key);
        }

        let response = req.send().map_err(|e| format!("Connection error: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            return Err(format!("Server error {}: {}", status, text));
        }

        let repl_response: ReplResponse =
            response.json().map_err(|e| format!("Parse error: {}", e))?;

        // Save session ID for future requests
        self.session_id = Some(repl_response.session_id.clone());

        Ok(repl_response)
    }

    fn set_database(&mut self, database: String) {
        self.database = database;
        self.session_id = None; // Reset session when changing database
    }
}

fn print_banner() {
    println!(
        "{}",
        r#"
   ___       _ _ ___  ___
  / __| ___ | (_)   \| _ )
  \__ \/ _ \| | | |) | _ \
  |___/\___/|_|_|___/|___/
"#
        .cyan()
    );
    println!(
        "  {} {}",
        "SoliDB Interactive REPL".white().bold(),
        env!("CARGO_PKG_VERSION").dimmed()
    );
    println!(
        "  Type {} for help, {} to quit\n",
        ".help".yellow(),
        ".exit".yellow()
    );
}

fn print_help() {
    println!("\n{}", "Commands:".white().bold());
    println!("  {}        Show this help", ".help".yellow());
    println!("  {}        Exit the REPL", ".exit".yellow());
    println!("  {}       Clear the screen", ".clear".yellow());
    println!("  {} <name>  Switch database", ".db".yellow());
    println!("  {}      Show current connection", ".status".yellow());
    println!("  {}       Reset session state", ".reset".yellow());

    println!("\n{}", "Lua API:".white().bold());
    println!(
        "  {}  Database operations",
        "db:collection(), db:query(), db:transaction()".cyan()
    );
    println!(
        "  {}   Crypto functions",
        "crypto.sha256(), crypto.jwt_encode()".cyan()
    );
    println!(
        "  {}   Time utilities",
        "time.now(), time.iso(), time.format()".cyan()
    );
    println!("  {}   HTTP requests", "solidb.fetch(url, options)".cyan());
    println!("  {}   Logging", "solidb.log(message), print(...)".cyan());

    println!("\n{}", "Examples:".white().bold());
    println!("  {} Get all users", "-- ".dimmed());
    println!("  {}", "db:query(\"FOR u IN users RETURN u\")".green());
    println!();
    println!("  {} Insert a document", "-- ".dimmed());
    println!(
        "  {}",
        "db:collection(\"users\"):insert({ name = \"Alice\" })".green()
    );
    println!();
    println!("  {} Generate a UUID", "-- ".dimmed());
    println!("  {}", "crypto.uuid()".green());
    println!();
}

fn format_value(value: &serde_json::Value, indent: usize) -> String {
    let prefix = "  ".repeat(indent);
    match value {
        serde_json::Value::Null => "nil".dimmed().to_string(),
        serde_json::Value::Bool(b) => if *b { "true".green() } else { "false".red() }.to_string(),
        serde_json::Value::Number(n) => n.to_string().yellow().to_string(),
        serde_json::Value::String(s) => format!("\"{}\"", s).green().to_string(),
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else if arr.len() <= 3 && arr.iter().all(|v| !v.is_object() && !v.is_array()) {
                // Compact format for small simple arrays
                let items: Vec<String> = arr.iter().map(|v| format_value(v, 0)).collect();
                format!("[ {} ]", items.join(", "))
            } else {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| format!("{}  {}", prefix, format_value(v, indent + 1)))
                    .collect();
                format!("[\n{}\n{}]", items.join(",\n"), prefix)
            }
        }
        serde_json::Value::Object(obj) => {
            if obj.is_empty() {
                "{}".to_string()
            } else {
                let items: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        format!("{}  {}: {}", prefix, k.cyan(), format_value(v, indent + 1))
                    })
                    .collect();
                format!("{{\n{}\n{}}}", items.join(",\n"), prefix)
            }
        }
    }
}

fn main() {
    let args = Args::parse();

    print_banner();

    let mut client = ReplClient::new(args.server.clone(), args.database.clone(), args.api_key);

    println!("  {} {}", "Connected to:".dimmed(), args.server.white());
    println!("  {} {}\n", "Database:".dimmed(), args.database.white());

    let helper = SoliDBHelper::new();
    let mut rl = Editor::new().expect("Failed to create editor");
    rl.set_helper(Some(helper));

    // Load history
    let history_file = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".solidb_history"))
        .unwrap_or_else(|_| std::path::PathBuf::from(".solidb_history"));
    let _ = rl.load_history(&history_file);

    let mut multiline_buffer = String::new();
    let mut in_multiline = false;

    loop {
        let prompt = if in_multiline {
            format!("{} ", "...".dimmed())
        } else {
            format!("{}{} ", client.database.cyan(), ">".white())
        };

        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();

                // Handle multiline input
                if line.ends_with('\\') {
                    multiline_buffer.push_str(&line[..line.len() - 1]);
                    multiline_buffer.push('\n');
                    in_multiline = true;
                    continue;
                }

                let code = if in_multiline {
                    multiline_buffer.push_str(line);
                    let code = multiline_buffer.clone();
                    multiline_buffer.clear();
                    in_multiline = false;
                    code
                } else {
                    line.to_string()
                };

                if code.is_empty() {
                    continue;
                }

                // Add to history
                let _ = rl.add_history_entry(&code);

                // Handle special commands
                if code.starts_with('.') {
                    let parts: Vec<&str> = code.splitn(2, ' ').collect();
                    match parts[0] {
                        ".exit" | ".quit" | ".q" => {
                            println!("{}", "Goodbye!".dimmed());
                            break;
                        }
                        ".help" | ".h" | ".?" => {
                            print_help();
                        }
                        ".clear" => {
                            print!("\x1B[2J\x1B[1;1H");
                            print_banner();
                        }
                        ".db" | ".database" => {
                            if parts.len() > 1 {
                                client.set_database(parts[1].to_string());
                                println!(
                                    "  {} {}",
                                    "Switched to database:".dimmed(),
                                    parts[1].cyan()
                                );
                            } else {
                                println!("  {}", "Usage: .db <database_name>".yellow());
                            }
                        }
                        ".status" => {
                            println!("  {} {}", "Server:".dimmed(), client.server.white());
                            println!("  {} {}", "Database:".dimmed(), client.database.cyan());
                            println!(
                                "  {} {}",
                                "Session:".dimmed(),
                                client.session_id.as_deref().unwrap_or("(none)").dimmed()
                            );
                        }
                        ".reset" => {
                            client.session_id = None;
                            println!("  {}", "Session reset".dimmed());
                        }
                        _ => {
                            println!("  {} {}", "Unknown command:".red(), parts[0]);
                            println!("  Type {} for help", ".help".yellow());
                        }
                    }
                    continue;
                }

                // Execute Lua code
                match client.execute(&code) {
                    Ok(response) => {
                        // Print console output
                        for line in &response.output {
                            println!("{} {}", "  >".dimmed(), line);
                        }

                        // Print error or result
                        if let Some(err) = response.error {
                            if let Some(line) = err.line {
                                println!(
                                    "{} {} (line {})",
                                    "Error:".red().bold(),
                                    err.message,
                                    line
                                );
                            } else {
                                println!("{} {}", "Error:".red().bold(), err.message);
                            }
                        } else if !response.result.is_null() {
                            println!("{}", format_value(&response.result, 0));
                        }

                        // Print timing
                        println!(
                            "{}",
                            format!("  ({:.2}ms)", response.execution_time_ms).dimmed()
                        );
                    }
                    Err(e) => {
                        println!("{} {}", "Error:".red().bold(), e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                if in_multiline {
                    println!("{}", "Cancelled".dimmed());
                    multiline_buffer.clear();
                    in_multiline = false;
                } else {
                    println!("{}", "Type .exit to quit".dimmed());
                }
            }
            Err(ReadlineError::Eof) => {
                println!("{}", "Goodbye!".dimmed());
                break;
            }
            Err(err) => {
                println!("{} {:?}", "Error:".red(), err);
                break;
            }
        }
    }

    // Save history
    let _ = rl.save_history(&history_file);
}
