//! Databases view - browse databases and collections

use super::View;
use crate::cli::tui::app::{AppContext, CurrentView};
use crate::cli::tui::client::{CollectionInfo, DatabaseInfo, TuiClient};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use std::sync::Arc;

/// Tree node for databases/collections
#[derive(Debug, Clone)]
pub enum TreeNode {
    Database {
        info: DatabaseInfo,
        expanded: bool,
        collections: Vec<CollectionInfo>,
    },
}

impl TreeNode {
    pub fn name(&self) -> &str {
        match self {
            TreeNode::Database { info, .. } => &info.name,
        }
    }
}

/// Flat list item for rendering
#[derive(Debug, Clone)]
struct FlatItem {
    display: String,
    is_database: bool,
    db_name: String,
    coll_name: Option<String>,
}

/// Databases view state
pub struct DatabasesView {
    nodes: Vec<TreeNode>,
    list_state: ListState,
    loading: bool,
    error: Option<String>,
}

impl DatabasesView {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            list_state: ListState::default(),
            loading: false,
            error: None,
        }
    }

    /// Build flat list for rendering
    fn build_flat_list(&self) -> Vec<FlatItem> {
        let mut items = Vec::new();
        for node in &self.nodes {
            match node {
                TreeNode::Database {
                    info,
                    expanded,
                    collections,
                } => {
                    let prefix = if *expanded { "[-]" } else { "[+]" };
                    items.push(FlatItem {
                        display: format!("{} {}", prefix, info.name),
                        is_database: true,
                        db_name: info.name.clone(),
                        coll_name: None,
                    });
                    if *expanded {
                        for coll in collections {
                            items.push(FlatItem {
                                display: format!("    {} ({})", coll.name, coll.count),
                                is_database: false,
                                db_name: info.name.clone(),
                                coll_name: Some(coll.name.clone()),
                            });
                        }
                    }
                }
            }
        }
        items
    }

    /// Refresh databases list
    pub fn refresh(&mut self, client: &Arc<TuiClient>) {
        self.loading = true;
        self.error = None;

        match client.list_databases() {
            Ok(dbs) => {
                self.nodes = dbs
                    .into_iter()
                    .map(|info| TreeNode::Database {
                        info,
                        expanded: false,
                        collections: Vec::new(),
                    })
                    .collect();
                self.loading = false;

                // Select first item if available
                if !self.nodes.is_empty() && self.list_state.selected().is_none() {
                    self.list_state.select(Some(0));
                }
            }
            Err(e) => {
                self.error = Some(e);
                self.loading = false;
            }
        }
    }

    /// Toggle expansion of selected database
    fn toggle_expand(&mut self, client: &Arc<TuiClient>, selected: usize) {
        let items = self.build_flat_list();
        if selected >= items.len() {
            return;
        }

        let item = &items[selected];
        if !item.is_database {
            return;
        }

        // Find the database node and toggle
        for node in &mut self.nodes {
            let TreeNode::Database {
                info,
                expanded,
                collections,
            } = node;
            if info.name == item.db_name {
                *expanded = !*expanded;
                // Load collections if expanding and empty
                if *expanded && collections.is_empty() {
                    if let Ok(colls) = client.list_collections(&info.name) {
                        *collections = colls;
                    }
                }
                break;
            }
        }
    }

    /// Get selected item info
    fn get_selected(&self) -> Option<FlatItem> {
        let items = self.build_flat_list();
        let selected = self.list_state.selected()?;
        items.get(selected).cloned()
    }
}

impl View for DatabasesView {
    fn draw(&mut self, f: &mut Frame, _ctx: &AppContext, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Databases ")
            .title_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );

        if self.loading {
            let loading = Paragraph::new("Loading...")
                .block(block)
                .style(Style::default().fg(Color::Yellow));
            f.render_widget(loading, area);
            return;
        }

        if let Some(ref err) = self.error {
            let error_msg = if err.contains("401") {
                format!(
                    "Error: {}\n\nAuthentication required. Restart with:\n  solidb tui -k <your-api-key>",
                    err
                )
            } else {
                format!("Error: {}\n\nPress 'r' to retry, 'q' to quit", err)
            };
            let error = Paragraph::new(error_msg)
                .block(block)
                .style(Style::default().fg(Color::Red));
            f.render_widget(error, area);
            return;
        }

        let items = self.build_flat_list();
        let list_items: Vec<ListItem> = items
            .iter()
            .map(|item| {
                let style = if item.is_database {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(item.display.clone()).style(style)
            })
            .collect();

        let list = List::new(list_items)
            .block(block)
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        f.render_stateful_widget(list, area, &mut self.list_state);

        // Draw hints at bottom
        let hints = " Enter: Expand/Select | j/k: Navigate | r: Refresh ";
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
        let items = self.build_flat_list();
        let len = items.len();

        match key {
            KeyCode::Down | KeyCode::Char('j') => {
                if len > 0 {
                    let i = self
                        .list_state
                        .selected()
                        .map(|i| (i + 1) % len)
                        .unwrap_or(0);
                    self.list_state.select(Some(i));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if len > 0 {
                    let i = self
                        .list_state
                        .selected()
                        .map(|i| if i == 0 { len - 1 } else { i - 1 })
                        .unwrap_or(0);
                    self.list_state.select(Some(i));
                }
            }
            KeyCode::Enter => {
                if let Some(item) = self.get_selected() {
                    if item.coll_name.is_some() {
                        // Collection selected - switch to documents view
                        ctx.current_database = item.db_name.clone();
                        ctx.current_collection = item.coll_name.clone();
                        ctx.current_view = CurrentView::Documents;
                        ctx.needs_refresh = true;
                        ctx.set_status(format!(
                            "Selected {}/{}",
                            item.db_name,
                            item.coll_name.as_deref().unwrap_or("")
                        ));
                    } else {
                        // Database selected - toggle expand
                        let selected = self.list_state.selected().unwrap_or(0);
                        self.toggle_expand(&ctx.client, selected);
                        ctx.current_database = item.db_name.clone();
                        ctx.set_status(format!("Database: {}", item.db_name));
                    }
                }
            }
            KeyCode::Char('r') => {
                self.refresh(&ctx.client);
                ctx.set_status("Refreshed databases");
            }
            _ => {}
        }
    }
}

impl Default for DatabasesView {
    fn default() -> Self {
        Self::new()
    }
}
