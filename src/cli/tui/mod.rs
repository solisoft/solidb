//! TUI (Terminal User Interface) for SoliDB
//!
//! Provides a rich terminal interface for database management,
//! query execution, and cluster monitoring.

pub mod app;
pub mod client;
pub mod ui;
pub mod views;

use clap::Parser;

/// TUI subcommand arguments
#[derive(Parser, Debug)]
#[command(name = "tui")]
#[command(about = "Launch the Terminal User Interface for database management")]
pub struct TuiArgs {
    /// Server URL to connect to
    #[arg(short, long, default_value = "http://localhost:6745")]
    pub server: String,

    /// Default database to use
    #[arg(short, long, default_value = "_system")]
    pub database: String,

    /// API key for authentication
    #[arg(short = 'k', long)]
    pub api_key: Option<String>,
}

/// Execute the TUI command
pub fn execute(args: TuiArgs) -> anyhow::Result<()> {
    app::run(args)
}
