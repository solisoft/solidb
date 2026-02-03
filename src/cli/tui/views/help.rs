//! Help overlay view

use super::View;
use crate::cli::tui::app::AppContext;
use crate::cli::tui::ui::widgets;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

/// Help view state
pub struct HelpView;

impl HelpView {
    pub fn new() -> Self {
        Self
    }
}

impl View for HelpView {
    fn draw(&mut self, f: &mut Frame, _ctx: &AppContext, area: Rect) {
        // Create centered popup
        let popup_area = widgets::centered_rect(70, 80, area);

        // Clear the area behind the popup
        f.render_widget(Clear, popup_area);

        let help_text = vec![
            Line::from(vec![Span::styled(
                "SoliDB TUI Help",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Global Navigation",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  1-5      ", Style::default().fg(Color::Green)),
                Span::raw("Switch to view (Databases/Documents/Query/Indexes/Cluster)"),
            ]),
            Line::from(vec![
                Span::styled("  Tab      ", Style::default().fg(Color::Green)),
                Span::raw("Next view"),
            ]),
            Line::from(vec![
                Span::styled("  Shift+Tab", Style::default().fg(Color::Green)),
                Span::raw("Previous view"),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+R   ", Style::default().fg(Color::Green)),
                Span::raw("Refresh current view"),
            ]),
            Line::from(vec![
                Span::styled("  ?        ", Style::default().fg(Color::Green)),
                Span::raw("Toggle this help"),
            ]),
            Line::from(vec![
                Span::styled("  q        ", Style::default().fg(Color::Green)),
                Span::raw("Quit"),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "List Navigation",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  j / Down ", Style::default().fg(Color::Green)),
                Span::raw("Move down"),
            ]),
            Line::from(vec![
                Span::styled("  k / Up   ", Style::default().fg(Color::Green)),
                Span::raw("Move up"),
            ]),
            Line::from(vec![
                Span::styled("  Enter    ", Style::default().fg(Color::Green)),
                Span::raw("Select / Expand / View detail"),
            ]),
            Line::from(vec![
                Span::styled("  Esc      ", Style::default().fg(Color::Green)),
                Span::raw("Close detail / Cancel"),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Databases View",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Enter    ", Style::default().fg(Color::Green)),
                Span::raw("Expand database / Select collection"),
            ]),
            Line::from(vec![
                Span::styled("  r        ", Style::default().fg(Color::Green)),
                Span::raw("Refresh database list"),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Documents View",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  n        ", Style::default().fg(Color::Green)),
                Span::raw("Next page"),
            ]),
            Line::from(vec![
                Span::styled("  p        ", Style::default().fg(Color::Green)),
                Span::raw("Previous page"),
            ]),
            Line::from(vec![
                Span::styled("  Enter    ", Style::default().fg(Color::Green)),
                Span::raw("View document detail"),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Query View",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  i        ", Style::default().fg(Color::Green)),
                Span::raw("Enter edit mode"),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+Enter", Style::default().fg(Color::Green)),
                Span::raw("Execute query"),
            ]),
            Line::from(vec![
                Span::styled("  Alt+Up/Down", Style::default().fg(Color::Green)),
                Span::raw("Navigate query history"),
            ]),
            Line::from(vec![
                Span::styled("  Esc      ", Style::default().fg(Color::Green)),
                Span::raw("Exit edit mode"),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Press Esc or ? to close",
                Style::default().fg(Color::DarkGray),
            )]),
        ];

        let help = Paragraph::new(help_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Help ")
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                    .style(Style::default().bg(Color::Black)),
            )
            .wrap(Wrap { trim: false });

        f.render_widget(help, popup_area);
    }

    fn handle_key(&mut self, _ctx: &mut AppContext, _key: KeyCode, _modifiers: KeyModifiers) {
        // Help view key handling is done in app.rs
    }
}

impl Default for HelpView {
    fn default() -> Self {
        Self::new()
    }
}
