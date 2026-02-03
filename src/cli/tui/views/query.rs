//! Query view - SDBQL editor with results

use super::View;
use crate::cli::tui::app::AppContext;
use crate::cli::tui::ui::widgets;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
};
use tui_textarea::{Input, Key, TextArea};

/// SDBQL keywords for syntax highlighting
const SDBQL_KEYWORDS: &[&str] = &[
    "FOR",
    "IN",
    "FILTER",
    "RETURN",
    "SORT",
    "LIMIT",
    "LET",
    "COLLECT",
    "INSERT",
    "UPDATE",
    "REPLACE",
    "REMOVE",
    "UPSERT",
    "AND",
    "OR",
    "NOT",
    "WITH",
    "INTO",
    "ASC",
    "DESC",
    "AGGREGATE",
    "DISTINCT",
    "GRAPH",
    "OUTBOUND",
    "INBOUND",
    "ANY",
    "PRUNE",
    "OPTIONS",
    "SEARCH",
    "LIKE",
    "JOIN",
    "LEFT",
];

/// Query view state
pub struct QueryView {
    textarea: TextArea<'static>,
    results: Vec<serde_json::Value>,
    result_table_state: TableState,
    error: Option<String>,
    editing: bool,
    execution_time: Option<u128>,
    show_result_detail: bool,
    selected_result: Option<serde_json::Value>,
    query_history: Vec<String>,
    history_index: Option<usize>,
}

impl QueryView {
    pub fn new() -> Self {
        let mut textarea = TextArea::default();
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" SDBQL Query ")
                .title_style(Style::default().fg(Color::Cyan)),
        );
        textarea.set_cursor_line_style(Style::default());
        textarea.set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));
        textarea.set_placeholder_text("FOR doc IN collection RETURN doc");

        // Set up keyword highlighting pattern
        let keywords_pattern = SDBQL_KEYWORDS.join("|");
        let pattern = format!(r"(?i)\b({})\b", keywords_pattern);
        let _ = textarea.set_search_pattern(&pattern);
        textarea.set_search_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

        Self {
            textarea,
            results: Vec::new(),
            result_table_state: TableState::default(),
            error: None,
            editing: false,
            execution_time: None,
            show_result_detail: false,
            selected_result: None,
            query_history: Vec::new(),
            history_index: None,
        }
    }

    /// Check if currently editing
    pub fn is_editing(&self) -> bool {
        self.editing
    }

    /// Enter edit mode
    pub fn enter_editing(&mut self) {
        self.editing = true;
    }

    /// Execute the query
    fn execute_query(&mut self, ctx: &mut AppContext) {
        let query = self.textarea.lines().join("\n");
        if query.trim().is_empty() {
            self.error = Some("Query is empty".to_string());
            return;
        }

        // Add to history
        if self.query_history.last() != Some(&query) {
            self.query_history.push(query.clone());
        }
        self.history_index = None;

        let start = std::time::Instant::now();
        match ctx.client.execute_query(&ctx.current_database, &query) {
            Ok(result) => {
                self.execution_time = Some(start.elapsed().as_millis());
                self.results = result.result;
                self.error = None;
                self.editing = false;

                if !self.results.is_empty() {
                    self.result_table_state.select(Some(0));
                }

                ctx.set_status(format!(
                    "Query returned {} results in {}ms",
                    self.results.len(),
                    self.execution_time.unwrap_or(0)
                ));
            }
            Err(e) => {
                self.error = Some(e);
                self.results = Vec::new();
                self.execution_time = Some(start.elapsed().as_millis());
                ctx.set_error("Query execution failed");
            }
        }
    }

    /// Navigate history
    fn history_prev(&mut self) {
        if self.query_history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            Some(i) => i.saturating_sub(1),
            None => self.query_history.len() - 1,
        };
        self.history_index = Some(idx);
        self.textarea = TextArea::new(vec![self.query_history[idx].clone()]);
        self.update_textarea_block();
    }

    fn history_next(&mut self) {
        if self.query_history.is_empty() {
            return;
        }
        if let Some(idx) = self.history_index {
            if idx + 1 < self.query_history.len() {
                self.history_index = Some(idx + 1);
                self.textarea = TextArea::new(vec![self.query_history[idx + 1].clone()]);
            } else {
                self.history_index = None;
                self.textarea = TextArea::default();
            }
            self.update_textarea_block();
        }
    }

    fn update_textarea_block(&mut self) {
        self.textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" SDBQL Query (Ctrl+Enter to execute) ")
                .title_style(Style::default().fg(Color::Cyan)),
        );
    }
}

impl View for QueryView {
    fn draw(&mut self, f: &mut Frame, _ctx: &AppContext, area: Rect) {
        // Show result detail if selected
        if self.show_result_detail {
            if let Some(ref result) = self.selected_result {
                widgets::render_json(result, area, f, "Result Detail (Esc to close)");
                return;
            }
        }

        // Split into query editor and results
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Min(10)])
            .split(area);

        // Draw query editor
        let editor_style = if self.editing {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        self.textarea.set_style(editor_style);
        f.render_widget(&self.textarea, chunks[0]);

        // Draw results area
        let results_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " Results ({}) {}",
                self.results.len(),
                self.execution_time
                    .map(|t| format!("- {}ms", t))
                    .unwrap_or_default()
            ))
            .title_style(Style::default().fg(Color::Cyan));

        if let Some(ref err) = self.error {
            let error_text = Paragraph::new(format!("Error: {}", err))
                .block(results_block)
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: false });
            f.render_widget(error_text, chunks[1]);
            return;
        }

        if self.results.is_empty() {
            let empty =
                Paragraph::new("No results. Press 'i' to edit query, Ctrl+Enter to execute.")
                    .block(results_block)
                    .style(Style::default().fg(Color::DarkGray));
            f.render_widget(empty, chunks[1]);
            return;
        }

        // Build table from results
        let rows: Vec<Row> = self
            .results
            .iter()
            .enumerate()
            .map(|(i, result)| {
                let preview = match result {
                    serde_json::Value::Object(map) => map
                        .iter()
                        .take(4)
                        .map(|(k, v)| format!("{}: {}", k, widgets::format_value_preview(v, 15)))
                        .collect::<Vec<_>>()
                        .join(", "),
                    _ => widgets::format_value_preview(result, 80),
                };
                Row::new(vec![
                    Cell::from(format!("{}", i + 1)).style(Style::default().fg(Color::DarkGray)),
                    Cell::from(preview),
                ])
            })
            .collect();

        let widths = [Constraint::Length(5), Constraint::Min(40)];

        let table = Table::new(rows, widths)
            .block(results_block)
            .header(
                Row::new(vec!["#", "Result"])
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

        f.render_stateful_widget(table, chunks[1], &mut self.result_table_state);

        // Draw hints
        let hints = if self.editing {
            " F5/Ctrl+E: Execute | Esc: Exit editor "
        } else {
            " i: Edit | Enter: View result | Tab: Switch views "
        };
        let hint_area = Rect {
            x: chunks[1].x + 1,
            y: chunks[1].y + chunks[1].height - 2,
            width: chunks[1].width.saturating_sub(2),
            height: 1,
        };
        let hint_text = Paragraph::new(hints).style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint_text, hint_area);
    }

    fn handle_key(&mut self, ctx: &mut AppContext, key: KeyCode, modifiers: KeyModifiers) {
        // Handle result detail view
        if self.show_result_detail {
            if key == KeyCode::Esc {
                self.show_result_detail = false;
                self.selected_result = None;
            }
            return;
        }

        if self.editing {
            // In edit mode, handle textarea input
            match key {
                KeyCode::Esc => {
                    self.editing = false;
                }
                // Execute query: F5, F9, Ctrl+E, or Ctrl+Enter
                KeyCode::F(5) | KeyCode::F(9) => {
                    self.execute_query(ctx);
                }
                KeyCode::Char('e') if modifiers.contains(KeyModifiers::CONTROL) => {
                    self.execute_query(ctx);
                }
                KeyCode::Enter if modifiers.contains(KeyModifiers::CONTROL) => {
                    self.execute_query(ctx);
                }
                KeyCode::Up if modifiers.contains(KeyModifiers::ALT) => {
                    self.history_prev();
                }
                KeyCode::Down if modifiers.contains(KeyModifiers::ALT) => {
                    self.history_next();
                }
                _ => {
                    // Convert crossterm key to tui-textarea input
                    let input = Input {
                        key: match key {
                            KeyCode::Char(c) => Key::Char(c),
                            KeyCode::Enter => Key::Enter,
                            KeyCode::Backspace => Key::Backspace,
                            KeyCode::Delete => Key::Delete,
                            KeyCode::Left => Key::Left,
                            KeyCode::Right => Key::Right,
                            KeyCode::Up => Key::Up,
                            KeyCode::Down => Key::Down,
                            KeyCode::Home => Key::Home,
                            KeyCode::End => Key::End,
                            KeyCode::Tab => Key::Tab,
                            _ => return,
                        },
                        ctrl: modifiers.contains(KeyModifiers::CONTROL),
                        alt: modifiers.contains(KeyModifiers::ALT),
                        shift: modifiers.contains(KeyModifiers::SHIFT),
                    };
                    self.textarea.input(input);
                }
            }
        } else {
            // In results mode
            let len = self.results.len();
            match key {
                KeyCode::Char('i') => {
                    self.editing = true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if len > 0 {
                        let i = self
                            .result_table_state
                            .selected()
                            .map(|i| (i + 1) % len)
                            .unwrap_or(0);
                        self.result_table_state.select(Some(i));
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if len > 0 {
                        let i = self
                            .result_table_state
                            .selected()
                            .map(|i| if i == 0 { len - 1 } else { i - 1 })
                            .unwrap_or(0);
                        self.result_table_state.select(Some(i));
                    }
                }
                KeyCode::Enter => {
                    if let Some(selected) = self.result_table_state.selected() {
                        if selected < self.results.len() {
                            self.selected_result = Some(self.results[selected].clone());
                            self.show_result_detail = true;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

impl Default for QueryView {
    fn default() -> Self {
        Self::new()
    }
}
