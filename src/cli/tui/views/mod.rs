//! View implementations for TUI

pub mod cluster;
pub mod databases;
pub mod documents;
pub mod help;
pub mod indexes;
pub mod jobs;
pub mod query;

pub use cluster::ClusterView;
pub use databases::DatabasesView;
pub use documents::DocumentsView;
pub use help::HelpView;
pub use indexes::IndexesView;
pub use jobs::JobsView;
pub use query::QueryView;

use crate::cli::tui::app::AppContext;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::prelude::*;

/// Trait for TUI views
pub trait View {
    /// Draw the view
    fn draw(&mut self, f: &mut Frame, ctx: &AppContext, area: Rect);

    /// Handle key events - returns true if view changed app state requiring action
    fn handle_key(&mut self, ctx: &mut AppContext, key: KeyCode, modifiers: KeyModifiers);
}
