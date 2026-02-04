//! Lua Script Development CLI
//!
//! Provides commands for managing Lua scripts from the IDE with folder-based conventions.
//!
//! ## Folder Convention
//!
//! ```text
//! scripts/
//! ├── hello.lua                → /api/custom/{db}/hello
//! ├── users.lua                → /api/custom/{db}/users
//! ├── users/
//! │   └── _id.lua              → /api/custom/{db}/users/:id
//! └── api/
//!     └── v1/
//!         └── products.lua     → /api/custom/{db}/api/v1/products
//! ```
//!
//! ## Lua Comment Header
//!
//! ```lua
//! -- @methods GET, POST
//! -- @description List or create users
//! -- @collection users
//!
//! local users = db:collection("users")
//! ```

pub mod client;
pub mod config;
pub mod mapper;
pub mod test_http;
pub mod test_runner;
pub mod watcher;

use clap::{Parser, Subcommand};
use colored::Colorize;
use similar::{ChangeTag, TextDiff};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use self::client::ScriptClient;
use self::config::Config;
use self::mapper::{
    api_path_to_file, file_to_api_path, generate_header, has_metadata_header, parse_script_meta,
};
use self::test_runner::{TestRunner, TestRunnerConfig};
use self::watcher::ScriptWatcher;

/// Create an authenticated client and print auth source info
fn create_authenticated_client(config: &Config) -> anyhow::Result<ScriptClient> {
    let auth_token = if config.auth_token.is_empty() {
        None
    } else {
        Some(config.auth_token.clone())
    };

    // Check auth source and print info
    if std::env::var(config::ENV_API_KEY)
        .map(|k| !k.is_empty())
        .unwrap_or(false)
    {
        println!(
            "{} Using API key from {} environment variable",
            "•".dimmed(),
            config::ENV_API_KEY
        );
    } else if auth_token.is_some() {
        println!("{} Using saved authentication token", "•".dimmed());
    } else {
        println!(
            "{} No authentication configured. Run 'solidb scripts login' or set {}",
            "!".yellow(),
            config::ENV_API_KEY
        );
    }

    let client = ScriptClient::new(&config.base_url(), auth_token);
    client.test_connection()?;

    Ok(client)
}

/// Script management subcommand
#[derive(Parser, Debug)]
#[command(name = "scripts")]
#[command(about = "Manage Lua scripts for custom API endpoints")]
pub struct ScriptsArgs {
    #[command(subcommand)]
    pub command: ScriptsCommand,
}

/// Available script commands
#[derive(Subcommand, Debug)]
pub enum ScriptsCommand {
    /// Initialize a scripts directory with configuration
    Init {
        /// Server host
        #[arg(long, default_value = "localhost")]
        host: String,

        /// Server port
        #[arg(long, default_value_t = 6745)]
        port: u16,

        /// Target database
        #[arg(long, short)]
        db: String,

        /// Authentication token
        #[arg(long)]
        auth_token: Option<String>,
    },

    /// Login to the server and save authentication token
    Login {
        /// Username
        #[arg(long, short)]
        username: Option<String>,

        /// Password (will prompt if not provided)
        #[arg(long)]
        password: Option<String>,
    },

    /// Push scripts to server
    Push {
        /// Specific file or directory to push (optional)
        path: Option<PathBuf>,
    },

    /// Pull scripts from server to local
    Pull {
        /// Specific path prefix to pull (optional)
        path: Option<String>,

        /// Overwrite local files without prompting
        #[arg(long)]
        force: bool,
    },

    /// Watch for file changes and auto-sync
    Watch {
        /// Specific directory to watch (optional)
        path: Option<PathBuf>,
    },

    /// List scripts on server
    List,

    /// Show diff between local and server
    Diff {
        /// Specific file or path to diff (optional)
        path: Option<PathBuf>,
    },

    /// Delete a script from server
    Delete {
        /// API path to delete (e.g., "users/:id")
        path: String,
    },

    /// Logout (remove saved authentication token)
    Logout,

    /// Run API tests from the tests/ directory
    Test {
        /// Specific test file to run (optional)
        file: Option<PathBuf>,

        /// Show verbose output including debug prints
        #[arg(long, short)]
        verbose: bool,

        /// Filter tests by name pattern
        #[arg(long, short)]
        filter: Option<String>,
    },
}

/// Execute a scripts command
pub fn execute(args: ScriptsArgs) -> anyhow::Result<()> {
    match args.command {
        ScriptsCommand::Init {
            host,
            port,
            db,
            auth_token,
        } => cmd_init(&host, port, &db, auth_token.as_deref()),
        ScriptsCommand::Login { username, password } => cmd_login(username, password),
        ScriptsCommand::Logout => cmd_logout(),
        ScriptsCommand::Push { path } => cmd_push(path.as_deref()),
        ScriptsCommand::Pull { path, force } => cmd_pull(path.as_deref(), force),
        ScriptsCommand::Watch { path } => cmd_watch(path.as_deref()),
        ScriptsCommand::List => cmd_list(),
        ScriptsCommand::Diff { path } => cmd_diff(path.as_deref()),
        ScriptsCommand::Delete { path } => cmd_delete(&path),
        ScriptsCommand::Test {
            file,
            verbose,
            filter,
        } => cmd_test(file, verbose, filter),
    }
}

/// Initialize a scripts directory
fn cmd_init(host: &str, port: u16, db: &str, auth_token: Option<&str>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let config_path = cwd.join(config::CONFIG_FILE_NAME);

    if config_path.exists() {
        anyhow::bail!(
            "Configuration file already exists: {}\nRemove it first if you want to reinitialize.",
            config_path.display()
        );
    }

    let mut config = Config::new(host.to_string(), port, db.to_string());
    if let Some(token) = auth_token {
        config.auth_token = token.to_string();
    }

    config.save(&cwd)?;

    println!("{} Created {}", "✓".green(), config_path.display());
    println!();
    println!("Configuration:");
    println!("  Host:     {}", host);
    println!("  Port:     {}", port);
    println!("  Database: {}", db);
    println!();
    println!("Next steps:");
    println!("  1. Authenticate using one of these methods:");
    println!("     - Run 'solidb scripts login' for interactive login");
    println!("     - Create a .env file with SOLIDB_API_KEY=your_api_key");
    println!("  2. Create .lua files in this directory");
    println!("  3. Run 'solidb scripts push' to deploy them");
    println!("  4. Run 'solidb scripts watch' for auto-sync");
    println!();
    println!("Environment variables (can be set in .env file):");
    println!("  SOLIDB_API_KEY   - API key for authentication");
    println!("  SOLIDB_HOST      - Override server host");
    println!("  SOLIDB_PORT      - Override server port");
    println!("  SOLIDB_DATABASE  - Override target database");

    Ok(())
}

/// Login to the server
fn cmd_login(username: Option<String>, password: Option<String>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let mut config = Config::load(&cwd)?;

    // Check if API key is set via environment
    if let Ok(api_key) = std::env::var(config::ENV_API_KEY) {
        if !api_key.is_empty() {
            println!(
                "{} {} environment variable is set - this will take precedence over login token.",
                "!".yellow(),
                config::ENV_API_KEY
            );
            println!("  Remove it from .env or environment if you want to use login instead.");
            println!();
        }
    }

    let client = ScriptClient::new(&config.base_url(), None);

    // Test connection first
    client.test_connection()?;

    // Get username
    let username = match username {
        Some(u) => u,
        None => {
            print!("Username: ");
            std::io::Write::flush(&mut std::io::stdout())?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            input.trim().to_string()
        }
    };

    if username.is_empty() {
        anyhow::bail!("Username cannot be empty");
    }

    // Get password (securely, without echo)
    let password = match password {
        Some(p) => p,
        None => rpassword::prompt_password("Password: ")?,
    };

    if password.is_empty() {
        anyhow::bail!("Password cannot be empty");
    }

    // Attempt login
    println!("Logging in as {}...", username);
    let token = client.login(&username, &password)?;

    // Save token to config
    config.auth_token = token;
    config.save(&cwd)?;

    println!("{} Logged in successfully!", "✓".green());
    println!("Token saved to {}", config::CONFIG_FILE_NAME);

    Ok(())
}

/// Logout (remove saved token)
fn cmd_logout() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let mut config = Config::load(&cwd)?;

    if config.auth_token.is_empty() {
        println!("Not logged in.");
        return Ok(());
    }

    config.auth_token = String::new();
    config.save(&cwd)?;

    println!("{} Logged out successfully!", "✓".green());
    println!("Token removed from {}", config::CONFIG_FILE_NAME);

    Ok(())
}

/// Push scripts to server
fn cmd_push(path: Option<&Path>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&cwd)?;
    let scripts_dir = config.scripts_dir(&cwd);

    let client = create_authenticated_client(&config)?;

    // Collect files to push
    let files = collect_lua_files(
        path.map(|p| {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                scripts_dir.join(p)
            }
        })
        .as_deref()
        .unwrap_or(&scripts_dir),
        &config,
    )?;

    if files.is_empty() {
        println!("No .lua files found to push.");
        return Ok(());
    }

    println!(
        "Pushing {} file(s) to {}...",
        files.len(),
        config.base_url()
    );
    println!();

    let mut created = 0;
    let mut updated = 0;
    let mut errors = 0;

    for file in files {
        let code = std::fs::read_to_string(&file)?;
        let meta = parse_script_meta(&code);
        let api_path = file_to_api_path(&file, &scripts_dir);

        // Check if script exists
        match client.find_script_by_path(&config.database, &api_path) {
            Ok(Some(existing)) => {
                // Update existing
                match client.update_script(
                    &config.database,
                    &existing.key,
                    &api_path,
                    &meta.methods,
                    &code,
                    meta.description.as_deref(),
                    meta.collection.as_deref(),
                ) {
                    Ok(_) => {
                        println!(
                            "{} Updated: {} -> {} [{}]",
                            "✓".green(),
                            file.strip_prefix(&scripts_dir).unwrap_or(&file).display(),
                            api_path,
                            meta.methods.join(", ")
                        );
                        updated += 1;
                    }
                    Err(e) => {
                        eprintln!("{} Error updating {}: {}", "✗".red(), file.display(), e);
                        errors += 1;
                    }
                }
            }
            Ok(None) => {
                // Create new
                match client.create_script(
                    &config.database,
                    &api_path,
                    &meta.methods,
                    &code,
                    meta.description.as_deref(),
                    meta.collection.as_deref(),
                ) {
                    Ok(_) => {
                        println!(
                            "{} Created: {} -> {} [{}]",
                            "✓".green(),
                            file.strip_prefix(&scripts_dir).unwrap_or(&file).display(),
                            api_path,
                            meta.methods.join(", ")
                        );
                        created += 1;
                    }
                    Err(e) => {
                        eprintln!("{} Error creating {}: {}", "✗".red(), file.display(), e);
                        errors += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("{} Error checking {}: {}", "✗".red(), file.display(), e);
                errors += 1;
            }
        }
    }

    println!();
    println!(
        "Done: {} created, {} updated, {} errors",
        created, updated, errors
    );

    if errors > 0 {
        std::process::exit(1);
    }

    Ok(())
}

/// Pull scripts from server
fn cmd_pull(path: Option<&str>, force: bool) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&cwd)?;
    let scripts_dir = config.scripts_dir(&cwd);

    let client = create_authenticated_client(&config)?;

    // List scripts from server
    let scripts = client.list_scripts(&config.database)?;

    // Filter by path prefix if specified
    let scripts: Vec<_> = if let Some(prefix) = path {
        let normalized = prefix.trim_start_matches('/');
        scripts
            .into_iter()
            .filter(|s| {
                let script_path = s.path.trim_start_matches('/');
                script_path.starts_with(normalized) || script_path == normalized
            })
            .collect()
    } else {
        scripts
    };

    if scripts.is_empty() {
        println!("No scripts found on server.");
        return Ok(());
    }

    println!("Pulling {} script(s) from server...", scripts.len());
    println!();

    let mut pulled = 0;
    let mut skipped = 0;
    let mut errors = 0;

    for summary in scripts {
        // Get full script with code
        let script = match client.get_script(&config.database, &summary.id) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{} Error fetching {}: {}", "✗".red(), summary.path, e);
                errors += 1;
                continue;
            }
        };

        let file_path = api_path_to_file(&script.path, &scripts_dir);

        // Generate content with header if needed
        let content = if has_metadata_header(&script.code) {
            script.code.clone()
        } else {
            let header = generate_header(
                &script.methods,
                script.description.as_deref(),
                script.collection.as_deref(),
            );
            format!("{}{}", header, script.code)
        };

        // Check if file exists
        if file_path.exists() && !force {
            let local_content = std::fs::read_to_string(&file_path)?;
            if local_content.trim() == content.trim() {
                println!(
                    "{} Unchanged: {}",
                    "•".dimmed(),
                    file_path
                        .strip_prefix(&scripts_dir)
                        .unwrap_or(&file_path)
                        .display()
                );
                skipped += 1;
                continue;
            }

            println!(
                "{} Conflict: {} (use --force to overwrite)",
                "!".yellow(),
                file_path
                    .strip_prefix(&scripts_dir)
                    .unwrap_or(&file_path)
                    .display()
            );
            skipped += 1;
            continue;
        }

        // Create parent directories
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write the file
        std::fs::write(&file_path, &content)?;
        println!(
            "{} Pulled: {} -> {}",
            "✓".green(),
            script.path,
            file_path
                .strip_prefix(&scripts_dir)
                .unwrap_or(&file_path)
                .display()
        );
        pulled += 1;
    }

    println!();
    println!(
        "Done: {} pulled, {} skipped, {} errors",
        pulled, skipped, errors
    );

    Ok(())
}

/// Watch for file changes
fn cmd_watch(path: Option<&Path>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&cwd)?;

    let mut watcher = ScriptWatcher::new(config, cwd)?;
    watcher.watch(path)?;
    watcher.run()
}

/// List scripts on server
fn cmd_list() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&cwd)?;

    let client = create_authenticated_client(&config)?;

    let scripts = client.list_scripts(&config.database)?;

    if scripts.is_empty() {
        println!("No scripts found in database '{}'.", config.database);
        return Ok(());
    }

    println!(
        "Scripts in database '{}' ({}):",
        config.database,
        scripts.len()
    );
    println!();

    for script in scripts {
        println!(
            "  {} {} [{}]",
            script.path.cyan(),
            script.description.as_deref().unwrap_or("").dimmed(),
            script.methods.join(", ").yellow()
        );
    }

    Ok(())
}

/// Show diff between local and server
fn cmd_diff(path: Option<&Path>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&cwd)?;
    let scripts_dir = config.scripts_dir(&cwd);

    let client = create_authenticated_client(&config)?;

    // Collect local files
    let local_files = collect_lua_files(
        path.map(|p| {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                scripts_dir.join(p)
            }
        })
        .as_deref()
        .unwrap_or(&scripts_dir),
        &config,
    )?;

    // Get server scripts
    let server_scripts = client.list_scripts(&config.database)?;

    let mut has_diff = false;

    // Check local files against server
    for file in &local_files {
        let local_code = std::fs::read_to_string(file)?;
        let api_path = file_to_api_path(file, &scripts_dir);

        let server_script = server_scripts
            .iter()
            .find(|s| s.path.trim_start_matches('/') == api_path.trim_start_matches('/'));

        match server_script {
            Some(summary) => {
                // Get full script
                let script = client.get_script(&config.database, &summary.id)?;
                if local_code.trim() != script.code.trim() {
                    has_diff = true;
                    println!(
                        "{} {} (modified)",
                        "M".yellow(),
                        file.strip_prefix(&scripts_dir).unwrap_or(file).display()
                    );
                    print_diff(&script.code, &local_code);
                }
            }
            None => {
                has_diff = true;
                println!(
                    "{} {} (new)",
                    "+".green(),
                    file.strip_prefix(&scripts_dir).unwrap_or(file).display()
                );
            }
        }
    }

    // Check server scripts not in local
    for script in &server_scripts {
        let file_path = api_path_to_file(&script.path, &scripts_dir);
        if !file_path.exists() {
            has_diff = true;
            println!("{} {} (only on server)", "-".red(), script.path);
        }
    }

    if !has_diff {
        println!("No differences found.");
    }

    Ok(())
}

/// Delete a script from server
fn cmd_delete(api_path: &str) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&cwd)?;

    let client = create_authenticated_client(&config)?;

    // Find and delete the script
    match client.find_script_by_path(&config.database, api_path)? {
        Some(script) => {
            client.delete_script(&config.database, &script.key)?;
            println!("{} Deleted: {}", "✓".green(), api_path);
        }
        None => {
            println!("{} Script not found: {}", "!".yellow(), api_path);
        }
    }

    Ok(())
}

/// Collect all .lua files from a path
fn collect_lua_files(path: &Path, config: &Config) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    if path.is_file() {
        if path.extension().map(|e| e == "lua").unwrap_or(false) {
            files.push(path.to_path_buf());
        }
    } else if path.is_dir() {
        for entry in WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let entry_path = entry.path();

            // Skip ignored paths
            if config.should_ignore(entry_path) {
                continue;
            }

            // Only include .lua files
            if entry_path.is_file() && entry_path.extension().map(|e| e == "lua").unwrap_or(false) {
                files.push(entry_path.to_path_buf());
            }
        }
    }

    Ok(files)
}

/// Print a colored diff
fn print_diff(old: &str, new: &str) {
    let diff = TextDiff::from_lines(old, new);

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-".red(),
            ChangeTag::Insert => "+".green(),
            ChangeTag::Equal => " ".normal(),
        };
        print!("  {}{}", sign, change);
    }
    println!();
}

/// Run tests from the tests/ directory
fn cmd_test(
    file: Option<PathBuf>,
    verbose: bool,
    filter: Option<String>,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;

    // Load config with test environment
    let config = Config::load_for_test(&cwd)?;
    let scripts_dir = config.scripts_dir(&cwd);

    // Check auth
    if !config.has_auth() {
        println!(
            "{} No authentication configured.",
            "!".yellow()
        );
        println!(
            "  Set {} in .env.test or run 'solidb scripts login'",
            config::ENV_API_KEY
        );
        println!();
    } else if std::env::var(config::ENV_API_KEY)
        .map(|k| !k.is_empty())
        .unwrap_or(false)
    {
        println!(
            "{} Using API key from {} environment variable",
            "•".dimmed(),
            config::ENV_API_KEY
        );
    } else {
        println!("{} Using saved authentication token", "•".dimmed());
    }

    // Print test config info
    println!(
        "{} Testing against {} (database: {}, service: {})",
        "•".dimmed(),
        config.base_url(),
        config.database,
        config.default_service()
    );
    println!();

    // Create and run the test runner
    let runner_config = TestRunnerConfig {
        verbose,
        filter,
        file,
    };

    let runner = TestRunner::new(config, scripts_dir, runner_config);
    let summary = runner.run()?;

    // Exit with error code if tests failed
    if summary.failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}
