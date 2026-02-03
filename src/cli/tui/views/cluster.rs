//! Cluster view - monitor cluster status and nodes

use super::View;
use crate::cli::tui::app::AppContext;
use crate::cli::tui::client::{ClusterStatus, TuiClient};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};
use std::sync::Arc;

/// Cluster view state
pub struct ClusterView {
    status: Option<ClusterStatus>,
    table_state: TableState,
    loading: bool,
    error: Option<String>,
}

impl ClusterView {
    pub fn new() -> Self {
        Self {
            status: None,
            table_state: TableState::default(),
            loading: false,
            error: None,
        }
    }

    /// Refresh cluster status
    pub fn refresh(&mut self, client: &Arc<TuiClient>) {
        self.loading = true;
        self.error = None;

        match client.get_cluster_status() {
            Ok(status) => {
                self.status = Some(status);
                self.loading = false;

                if let Some(ref s) = self.status {
                    if !s.nodes.is_empty() && self.table_state.selected().is_none() {
                        self.table_state.select(Some(0));
                    }
                }
            }
            Err(e) => {
                self.error = Some(e);
                self.loading = false;
            }
        }
    }

    /// Format node status with color
    fn format_status(status: &str) -> (String, Color) {
        match status.to_lowercase().as_str() {
            "healthy" | "online" | "active" => ("Healthy".to_string(), Color::Green),
            "unhealthy" | "offline" | "down" => ("Unhealthy".to_string(), Color::Red),
            "unknown" => ("Unknown".to_string(), Color::Yellow),
            "suspect" => ("Suspect".to_string(), Color::Yellow),
            _ => (status.to_string(), Color::White),
        }
    }
}

impl View for ClusterView {
    fn draw(&mut self, f: &mut Frame, _ctx: &AppContext, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Cluster Status ")
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
            let error = Paragraph::new(format!("Error: {}", err))
                .block(block)
                .style(Style::default().fg(Color::Red));
            f.render_widget(error, area);
            return;
        }

        let status = match &self.status {
            Some(s) => s,
            None => {
                let empty = Paragraph::new("No cluster status available")
                    .block(block)
                    .style(Style::default().fg(Color::DarkGray));
                f.render_widget(empty, area);
                return;
            }
        };

        // Split into header and nodes table
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(4), Constraint::Min(10)])
            .split(area);

        // Draw header with cluster info
        let mode = if status.is_cluster_mode {
            "Cluster Mode"
        } else {
            "Standalone Mode"
        };
        let mode_color = if status.is_cluster_mode {
            Color::Green
        } else {
            Color::Yellow
        };

        let header = Paragraph::new(vec![
            Line::from(vec![
                Span::raw("Node ID: "),
                Span::styled(&status.node_id, Style::default().fg(Color::Cyan)),
            ]),
            Line::from(vec![
                Span::raw("Mode: "),
                Span::styled(mode, Style::default().fg(mode_color)),
                Span::raw(" | Nodes: "),
                Span::styled(
                    format!("{}", status.nodes.len()),
                    Style::default().fg(Color::White),
                ),
            ]),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" This Node ")
                .title_style(Style::default().fg(Color::Cyan)),
        );

        f.render_widget(header, chunks[0]);

        // Draw nodes table
        if status.nodes.is_empty() {
            let empty = Paragraph::new("No other nodes in cluster")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Cluster Nodes ")
                        .title_style(Style::default().fg(Color::Cyan)),
                )
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(empty, chunks[1]);
            return;
        }

        let rows: Vec<Row> = status
            .nodes
            .iter()
            .map(|node| {
                let (status_text, status_color) = Self::format_status(&node.status);
                let heartbeat = node
                    .last_heartbeat
                    .map(|h| format!("{}s ago", h))
                    .unwrap_or_else(|| "-".to_string());

                Row::new(vec![
                    Cell::from(node.id.chars().take(12).collect::<String>())
                        .style(Style::default().fg(Color::Cyan)),
                    Cell::from(node.address.clone()),
                    Cell::from(node.api_address.clone()),
                    Cell::from(status_text).style(Style::default().fg(status_color)),
                    Cell::from(heartbeat).style(Style::default().fg(Color::DarkGray)),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(14),
            Constraint::Length(22),
            Constraint::Length(22),
            Constraint::Length(10),
            Constraint::Min(10),
        ];

        let table = Table::new(rows, widths)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Cluster Nodes ")
                    .title_style(Style::default().fg(Color::Cyan)),
            )
            .header(
                Row::new(vec![
                    "Node ID",
                    "Repl Address",
                    "API Address",
                    "Status",
                    "Heartbeat",
                ])
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

        f.render_stateful_widget(table, chunks[1], &mut self.table_state);

        // Draw hints
        let hints = " j/k: Navigate | r: Refresh ";
        let hint_area = Rect {
            x: chunks[1].x + 1,
            y: chunks[1].y + chunks[1].height - 2,
            width: chunks[1].width.saturating_sub(2),
            height: 1,
        };
        let hint_text = Paragraph::new(hints).style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint_text, hint_area);
    }

    fn handle_key(&mut self, ctx: &mut AppContext, key: KeyCode, _modifiers: KeyModifiers) {
        let len = self.status.as_ref().map(|s| s.nodes.len()).unwrap_or(0);

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
                self.refresh(&ctx.client);
                ctx.set_status("Refreshed cluster status");
            }
            _ => {}
        }
    }
}

impl Default for ClusterView {
    fn default() -> Self {
        Self::new()
    }
}
