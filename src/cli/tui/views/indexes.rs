//! Indexes view - manage collection indexes

use super::View;
use crate::cli::tui::app::AppContext;
use crate::cli::tui::client::{IndexInfo, TuiClient};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};
use std::sync::Arc;

/// Indexes view state
pub struct IndexesView {
    indexes: Vec<IndexInfo>,
    table_state: TableState,
    loading: bool,
    error: Option<String>,
}

impl IndexesView {
    pub fn new() -> Self {
        Self {
            indexes: Vec::new(),
            table_state: TableState::default(),
            loading: false,
            error: None,
        }
    }

    /// Refresh indexes list
    pub fn refresh(&mut self, client: &Arc<TuiClient>, database: &str, collection: &str) {
        self.loading = true;
        self.error = None;

        match client.list_indexes(database, collection) {
            Ok(indexes) => {
                self.indexes = indexes;
                self.loading = false;

                if !self.indexes.is_empty() && self.table_state.selected().is_none() {
                    self.table_state.select(Some(0));
                }
            }
            Err(e) => {
                self.error = Some(e);
                self.loading = false;
            }
        }
    }
}

impl View for IndexesView {
    fn draw(&mut self, f: &mut Frame, ctx: &AppContext, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " Indexes - {} ",
                ctx.current_collection.as_deref().unwrap_or("None")
            ))
            .title_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );

        if ctx.current_collection.is_none() {
            let msg = Paragraph::new("Select a collection from the Databases view (press 1)")
                .block(block)
                .style(Style::default().fg(Color::Yellow));
            f.render_widget(msg, area);
            return;
        }

        if self.loading {
            let loading = Paragraph::new("Loading...")
                .block(block)
                .style(Style::default().fg(Color::Yellow));
            f.render_widget(loading, area);
            return;
        }

        if let Some(ref err) = self.error {
            let error = Paragraph::new(format!("Error: {}", err))
                .block(block)
                .style(Style::default().fg(Color::Red));
            f.render_widget(error, area);
            return;
        }

        if self.indexes.is_empty() {
            let empty = Paragraph::new("No indexes defined")
                .block(block)
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(empty, area);
            return;
        }

        // Build table rows
        let rows: Vec<Row> = self
            .indexes
            .iter()
            .map(|idx| {
                let flags = format!(
                    "{}{}",
                    if idx.unique { "U" } else { "-" },
                    if idx.sparse { "S" } else { "-" }
                );
                Row::new(vec![
                    Cell::from(idx.name.clone()).style(Style::default().fg(Color::Cyan)),
                    Cell::from(idx.index_type.clone()).style(Style::default().fg(Color::Yellow)),
                    Cell::from(idx.fields.join(", ")),
                    Cell::from(flags).style(Style::default().fg(Color::Magenta)),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(20),
            Constraint::Length(12),
            Constraint::Min(30),
            Constraint::Length(6),
        ];

        let table = Table::new(rows, widths)
            .block(block)
            .header(
                Row::new(vec!["Name", "Type", "Fields", "Flags"])
                    .style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )
                    .bottom_margin(1),
            )
            .row_highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        f.render_stateful_widget(table, area, &mut self.table_state);

        // Draw hints
        let hints = " U=Unique S=Sparse | j/k: Navigate | r: Refresh ";
        let hint_area = Rect {
            x: area.x + 1,
            y: area.y + area.height - 2,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        let hint_text = Paragraph::new(hints).style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint_text, hint_area);
    }

    fn handle_key(&mut self, ctx: &mut AppContext, key: KeyCode, _modifiers: KeyModifiers) {
        let len = self.indexes.len();
        let collection = match &ctx.current_collection {
            Some(c) => c.clone(),
            None => return,
        };

        match key {
            KeyCode::Down | KeyCode::Char('j') => {
                if len > 0 {
                    let i = self
                        .table_state
                        .selected()
                        .map(|i| (i + 1) % len)
                        .unwrap_or(0);
                    self.table_state.select(Some(i));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if len > 0 {
                    let i = self
                        .table_state
                        .selected()
                        .map(|i| if i == 0 { len - 1 } else { i - 1 })
                        .unwrap_or(0);
                    self.table_state.select(Some(i));
                }
            }
            KeyCode::Char('r') => {
                self.refresh(&ctx.client, &ctx.current_database, &collection);
                ctx.set_status("Refreshed indexes");
            }
            _ => {}
        }
    }
}

impl Default for IndexesView {
    fn default() -> Self {
        Self::new()
    }
}
