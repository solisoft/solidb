//! Documents view - browse and manage documents with pagination

use super::View;
use crate::cli::tui::app::AppContext;
use crate::cli::tui::client::TuiClient;
use crate::cli::tui::ui::widgets;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};
use std::sync::Arc;

/// Documents view state
pub struct DocumentsView {
    documents: Vec<serde_json::Value>,
    table_state: TableState,
    loading: bool,
    error: Option<String>,

    // Pagination
    offset: u64,
    limit: u64,
    total: u64,

    // Detail view
    selected_doc: Option<serde_json::Value>,
    show_detail: bool,
}

impl DocumentsView {
    pub fn new() -> Self {
        Self {
            documents: Vec::new(),
            table_state: TableState::default(),
            loading: false,
            error: None,
            offset: 0,
            limit: 50,
            total: 0,
            selected_doc: None,
            show_detail: false,
        }
    }

    /// Refresh documents list
    pub fn refresh(&mut self, client: &Arc<TuiClient>, database: &str, collection: &str) {
        self.loading = true;
        self.error = None;

        match client.list_documents(database, collection, self.offset, self.limit) {
            Ok(response) => {
                self.documents = response.documents;
                self.total = response.total;
                self.loading = false;

                // Select first item if available
                if !self.documents.is_empty() && self.table_state.selected().is_none() {
                    self.table_state.select(Some(0));
                }
            }
            Err(e) => {
                self.error = Some(e);
                self.loading = false;
            }
        }
    }

    /// Load next page
    fn next_page(&mut self, client: &Arc<TuiClient>, database: &str, collection: &str) {
        if self.offset + self.limit < self.total {
            self.offset += self.limit;
            self.table_state.select(Some(0));
            self.refresh(client, database, collection);
        }
    }

    /// Load previous page
    fn prev_page(&mut self, client: &Arc<TuiClient>, database: &str, collection: &str) {
        if self.offset >= self.limit {
            self.offset -= self.limit;
            self.table_state.select(Some(0));
            self.refresh(client, database, collection);
        }
    }

    /// Get key from document
    fn get_key(doc: &serde_json::Value) -> String {
        doc.get("_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "-".to_string())
    }

    /// Load selected document detail
    fn load_detail(&mut self, client: &Arc<TuiClient>, database: &str, collection: &str) {
        if let Some(selected) = self.table_state.selected() {
            if selected < self.documents.len() {
                let doc = &self.documents[selected];
                let key = Self::get_key(doc);
                match client.get_document(database, collection, &key) {
                    Ok(full_doc) => {
                        self.selected_doc = Some(full_doc);
                        self.show_detail = true;
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
            }
        }
    }
}

impl View for DocumentsView {
    fn draw(&mut self, f: &mut Frame, ctx: &AppContext, area: Rect) {
        // If showing detail view
        if self.show_detail {
            if let Some(ref doc) = self.selected_doc {
                widgets::render_json(doc, area, f, "Document Detail (Esc to close)");
                return;
            }
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " Documents - {} ",
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

        if self.documents.is_empty() {
            let empty = Paragraph::new("No documents found")
                .block(block)
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(empty, area);
            return;
        }

        // Build table rows
        let rows: Vec<Row> = self
            .documents
            .iter()
            .map(|doc| {
                let key = Self::get_key(doc);

                // Get preview of first few fields
                let preview = if let serde_json::Value::Object(map) = doc {
                    map.iter()
                        .filter(|(k, _)| !k.starts_with('_'))
                        .take(3)
                        .map(|(k, v)| format!("{}: {}", k, widgets::format_value_preview(v, 20)))
                        .collect::<Vec<_>>()
                        .join(", ")
                } else {
                    widgets::format_value_preview(doc, 60)
                };

                Row::new(vec![
                    Cell::from(widgets::format_key(&key)).style(Style::default().fg(Color::Cyan)),
                    Cell::from(preview),
                ])
            })
            .collect();

        let widths = [Constraint::Length(22), Constraint::Min(40)];

        let table = Table::new(rows, widths)
            .block(block)
            .header(
                Row::new(vec!["Key", "Preview"])
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

        // Draw pagination info
        let page_info = format!(
            " Page {}/{} ({} docs) | n/p: Next/Prev page | Enter: View | r: Refresh ",
            (self.offset / self.limit) + 1,
            (self.total + self.limit - 1) / self.limit.max(1),
            self.total
        );
        let page_area = Rect {
            x: area.x + 1,
            y: area.y + area.height - 2,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        let page_text = Paragraph::new(page_info).style(Style::default().fg(Color::DarkGray));
        f.render_widget(page_text, page_area);
    }

    fn handle_key(&mut self, ctx: &mut AppContext, key: KeyCode, _modifiers: KeyModifiers) {
        // Handle detail view
        if self.show_detail {
            if key == KeyCode::Esc {
                self.show_detail = false;
                self.selected_doc = None;
            }
            return;
        }

        let len = self.documents.len();
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
            KeyCode::Enter => {
                self.load_detail(&ctx.client, &ctx.current_database, &collection);
            }
            KeyCode::Char('n') => {
                self.next_page(&ctx.client, &ctx.current_database, &collection);
            }
            KeyCode::Char('p') => {
                self.prev_page(&ctx.client, &ctx.current_database, &collection);
            }
            KeyCode::Char('r') => {
                self.refresh(&ctx.client, &ctx.current_database, &collection);
                ctx.set_status("Refreshed documents");
            }
            _ => {}
        }
    }
}

impl Default for DocumentsView {
    fn default() -> Self {
        Self::new()
    }
}
