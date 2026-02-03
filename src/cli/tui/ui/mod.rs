//! UI rendering for TUI

pub mod layout;
pub mod widgets;

use super::app::{App, CurrentView};
use super::views::View;
use ratatui::prelude::*;

/// Main draw function
pub fn draw(f: &mut Frame, app: &mut App) {
    let size = f.area();

    // Create main layout: sidebar + content
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(20), Constraint::Min(60)])
        .split(size);

    // Draw sidebar
    layout::draw_sidebar(f, &app.ctx, chunks[0]);

    // Draw content area with status bar
    let content_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(1)])
        .split(chunks[1]);

    // Draw current view
    match app.ctx.current_view {
        CurrentView::Databases => app.databases_view.draw(f, &app.ctx, content_chunks[0]),
        CurrentView::Documents => app.documents_view.draw(f, &app.ctx, content_chunks[0]),
        CurrentView::Query => app.query_view.draw(f, &app.ctx, content_chunks[0]),
        CurrentView::Indexes => app.indexes_view.draw(f, &app.ctx, content_chunks[0]),
        CurrentView::Jobs => app.jobs_view.draw(f, &app.ctx, content_chunks[0]),
        CurrentView::Cluster => app.cluster_view.draw(f, &app.ctx, content_chunks[0]),
    }

    // Draw status bar
    layout::draw_status_bar(f, &app.ctx, content_chunks[1]);

    // Draw help overlay if visible
    if app.ctx.show_help {
        app.help_view.draw(f, &app.ctx, size);
    }
}
