//! File watcher for auto-sync on file changes
//!
//! Uses the notify crate to watch for file system events and automatically
//! push changes to the server.

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use super::client::ScriptClient;
use super::config::Config as ScriptConfig;
use super::mapper;

/// File watcher for Lua scripts
pub struct ScriptWatcher {
    watcher: RecommendedWatcher,
    receiver: Receiver<Result<Event, notify::Error>>,
    config: ScriptConfig,
    config_dir: PathBuf,
}

impl ScriptWatcher {
    /// Create a new script watcher
    pub fn new(config: ScriptConfig, config_dir: PathBuf) -> anyhow::Result<Self> {
        let (tx, rx) = channel();

        let watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default().with_poll_interval(Duration::from_millis(500)),
        )?;

        Ok(Self {
            watcher,
            receiver: rx,
            config,
            config_dir,
        })
    }

    /// Start watching a directory
    pub fn watch(&mut self, path: Option<&Path>) -> anyhow::Result<()> {
        let watch_path = path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.config.scripts_dir(&self.config_dir));

        self.watcher.watch(&watch_path, RecursiveMode::Recursive)?;

        println!("Watching {} for changes...", watch_path.display());
        println!("Press Ctrl+C to stop.");
        println!();

        Ok(())
    }

    /// Process events in a loop
    pub fn run(&self) -> anyhow::Result<()> {
        let client = ScriptClient::new(
            &self.config.base_url(),
            if self.config.auth_token.is_empty() {
                None
            } else {
                Some(self.config.auth_token.clone())
            },
        );

        // Test connection first
        if let Err(e) = client.test_connection() {
            eprintln!("Warning: Could not connect to server: {}", e);
            eprintln!("Will retry on file changes...");
            eprintln!();
        }

        let scripts_dir = self.config.scripts_dir(&self.config_dir);

        loop {
            match self.receiver.recv() {
                Ok(Ok(event)) => {
                    self.handle_event(&client, &scripts_dir, event);
                }
                Ok(Err(e)) => {
                    eprintln!("Watch error: {}", e);
                }
                Err(e) => {
                    // Channel closed, exit
                    return Err(anyhow::anyhow!("Watch channel closed: {}", e));
                }
            }
        }
    }

    /// Handle a file system event
    fn handle_event(&self, client: &ScriptClient, scripts_dir: &Path, event: Event) {
        for path in &event.paths {
            // Only handle .lua files
            if path.extension().map(|e| e != "lua").unwrap_or(true) {
                continue;
            }

            // Skip ignored paths
            if self.config.should_ignore(path) {
                continue;
            }

            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) => {
                    self.handle_file_change(client, scripts_dir, path);
                }
                EventKind::Remove(_) => {
                    self.handle_file_remove(client, scripts_dir, path);
                }
                _ => {}
            }
        }
    }

    /// Handle a file creation or modification
    fn handle_file_change(&self, client: &ScriptClient, scripts_dir: &Path, path: &Path) {
        // Read the file
        let code = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error reading {}: {}", path.display(), e);
                return;
            }
        };

        // Skip empty files
        if code.trim().is_empty() {
            return;
        }

        // Parse metadata
        let meta = mapper::parse_script_meta(&code);
        let api_path = mapper::file_to_api_path(path, scripts_dir);

        // Try to find existing script
        match client.find_script_by_path(&self.config.database, &api_path) {
            Ok(Some(existing)) => {
                // Update existing script
                match client.update_script(
                    &self.config.database,
                    &existing.key,
                    &api_path,
                    &meta.methods,
                    &code,
                    meta.description.as_deref(),
                    meta.collection.as_deref(),
                ) {
                    Ok(_) => {
                        println!(
                            "Updated: {} -> {} [{}]",
                            path.display(),
                            api_path,
                            meta.methods.join(", ")
                        );
                    }
                    Err(e) => {
                        eprintln!("Error updating {}: {}", path.display(), e);
                    }
                }
            }
            Ok(None) => {
                // Create new script
                match client.create_script(
                    &self.config.database,
                    &api_path,
                    &meta.methods,
                    &code,
                    meta.description.as_deref(),
                    meta.collection.as_deref(),
                ) {
                    Ok(_) => {
                        println!(
                            "Created: {} -> {} [{}]",
                            path.display(),
                            api_path,
                            meta.methods.join(", ")
                        );
                    }
                    Err(e) => {
                        eprintln!("Error creating {}: {}", path.display(), e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Error checking script {}: {}", path.display(), e);
            }
        }
    }

    /// Handle a file removal
    fn handle_file_remove(&self, client: &ScriptClient, scripts_dir: &Path, path: &Path) {
        let api_path = mapper::file_to_api_path(path, scripts_dir);

        match client.delete_script_by_path(&self.config.database, &api_path) {
            Ok(true) => {
                println!("Deleted: {} -> {}", path.display(), api_path);
            }
            Ok(false) => {
                // Script didn't exist on server, nothing to do
            }
            Err(e) => {
                eprintln!("Error deleting {}: {}", path.display(), e);
            }
        }
    }
}
