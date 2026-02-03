//! Jobs view - monitor job queues

use super::View;
use crate::cli::tui::app::AppContext;
use crate::cli::tui::client::{JobInfo, QueueStats, TuiClient};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};
use std::sync::Arc;

/// Jobs view state
pub struct JobsView {
    queues: Vec<QueueStats>,
    jobs: Vec<JobInfo>,
    queue_state: TableState,
    job_state: TableState,
    selected_queue: Option<String>,
    loading: bool,
    error: Option<String>,
    focus_jobs: bool,
}

impl JobsView {
    pub fn new() -> Self {
        Self {
            queues: Vec::new(),
            jobs: Vec::new(),
            queue_state: TableState::default(),
            job_state: TableState::default(),
            selected_queue: None,
            loading: false,
            error: None,
            focus_jobs: false,
        }
    }

    /// Refresh queues list
    pub fn refresh(&mut self, client: &Arc<TuiClient>, database: &str) {
        self.loading = true;
        self.error = None;

        match client.list_queues(database) {
            Ok(queues) => {
                self.queues = queues;
                self.loading = false;

                if !self.queues.is_empty() && self.queue_state.selected().is_none() {
                    self.queue_state.select(Some(0));
                    self.selected_queue = Some(self.queues[0].name.clone());
                    self.load_jobs(client, database);
                }
            }
            Err(e) => {
                self.error = Some(e);
                self.loading = false;
            }
        }
    }

    /// Load jobs for selected queue
    fn load_jobs(&mut self, client: &Arc<TuiClient>, database: &str) {
        if let Some(ref queue) = self.selected_queue {
            match client.list_jobs(database, queue, 50) {
                Ok(jobs) => {
                    self.jobs = jobs;
                    if !self.jobs.is_empty() && self.job_state.selected().is_none() {
                        self.job_state.select(Some(0));
                    }
                }
                Err(e) => {
                    self.error = Some(e);
                }
            }
        }
    }

    fn format_status(status: &serde_json::Value) -> (String, Color) {
        let status_str = match status {
            serde_json::Value::String(s) => s.to_lowercase(),
            _ => status.to_string().to_lowercase().trim_matches('"').to_string(),
        };
        match status_str.as_str() {
            "pending" => ("Pending".to_string(), Color::Yellow),
            "running" => ("Running".to_string(), Color::Blue),
            "completed" => ("Done".to_string(), Color::Green),
            "failed" => ("Failed".to_string(), Color::Red),
            _ => (status_str, Color::White),
        }
    }
}

impl View for JobsView {
    fn draw(&mut self, f: &mut Frame, _ctx: &AppContext, area: Rect) {
        // Split into queues (left) and jobs (right)
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(area);

        // Draw queues panel
        let queue_block = Block::default()
            .borders(Borders::ALL)
            .title(" Queues ")
            .title_style(Style::default().fg(if !self.focus_jobs {
                Color::Cyan
            } else {
                Color::White
            }));

        if self.loading {
            let loading = Paragraph::new("Loading...")
                .block(queue_block)
                .style(Style::default().fg(Color::Yellow));
            f.render_widget(loading, chunks[0]);
        } else if let Some(ref err) = self.error {
            let error = Paragraph::new(format!("Error: {}", err))
                .block(queue_block)
                .style(Style::default().fg(Color::Red));
            f.render_widget(error, chunks[0]);
        } else if self.queues.is_empty() {
            let empty = Paragraph::new("No queues found")
                .block(queue_block)
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(empty, chunks[0]);
        } else {
            let rows: Vec<Row> = self
                .queues
                .iter()
                .map(|q| {
                    Row::new(vec![
                        Cell::from(q.name.clone()).style(Style::default().fg(Color::Cyan)),
                        Cell::from(format!("{}", q.pending)).style(Style::default().fg(Color::Yellow)),
                        Cell::from(format!("{}", q.running)).style(Style::default().fg(Color::Blue)),
                        Cell::from(format!("{}", q.failed)).style(Style::default().fg(Color::Red)),
                    ])
                })
                .collect();

            let widths = [
                Constraint::Min(10),
                Constraint::Length(5),
                Constraint::Length(5),
                Constraint::Length(5),
            ];

            let table = Table::new(rows, widths)
                .block(queue_block)
                .header(
                    Row::new(vec!["Queue", "Pend", "Run", "Fail"])
                        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                        .bottom_margin(1),
                )
                .row_highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("> ");

            f.render_stateful_widget(table, chunks[0], &mut self.queue_state);
        }

        // Draw jobs panel
        let jobs_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " Jobs - {} ",
                self.selected_queue.as_deref().unwrap_or("None")
            ))
            .title_style(Style::default().fg(if self.focus_jobs {
                Color::Cyan
            } else {
                Color::White
            }));

        if self.jobs.is_empty() {
            let empty = Paragraph::new("No jobs in queue")
                .block(jobs_block)
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(empty, chunks[1]);
        } else {
            let rows: Vec<Row> = self
                .jobs
                .iter()
                .map(|j| {
                    let (status_text, status_color) = Self::format_status(&j.status);
                    Row::new(vec![
                        Cell::from(j.id[..8.min(j.id.len())].to_string())
                            .style(Style::default().fg(Color::DarkGray)),
                        Cell::from(status_text).style(Style::default().fg(status_color)),
                        Cell::from(j.script_path.clone()),
                        Cell::from(format!("{}", j.retry_count)),
                    ])
                })
                .collect();

            let widths = [
                Constraint::Length(10),
                Constraint::Length(12),
                Constraint::Min(20),
                Constraint::Length(5),
            ];

            let table = Table::new(rows, widths)
                .block(jobs_block)
                .header(
                    Row::new(vec!["ID", "Status", "Script", "Retry"])
                        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                        .bottom_margin(1),
                )
                .row_highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("> ");

            f.render_stateful_widget(table, chunks[1], &mut self.job_state);
        }

        // Draw hints
        let hints = " Tab: Switch panel | j/k: Navigate | r: Refresh | Enter: Select queue ";
        let hint_area = Rect {
            x: area.x + 1,
            y: area.y + area.height - 1,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        let hint_text = Paragraph::new(hints).style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint_text, hint_area);
    }

    fn handle_key(&mut self, ctx: &mut AppContext, key: KeyCode, _modifiers: KeyModifiers) {
        match key {
            KeyCode::Tab => {
                self.focus_jobs = !self.focus_jobs;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.focus_jobs {
                    let len = self.jobs.len();
                    if len > 0 {
                        let i = self.job_state.selected().map(|i| (i + 1) % len).unwrap_or(0);
                        self.job_state.select(Some(i));
                    }
                } else {
                    let len = self.queues.len();
                    if len > 0 {
                        let i = self.queue_state.selected().map(|i| (i + 1) % len).unwrap_or(0);
                        self.queue_state.select(Some(i));
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.focus_jobs {
                    let len = self.jobs.len();
                    if len > 0 {
                        let i = self
                            .job_state
                            .selected()
                            .map(|i| if i == 0 { len - 1 } else { i - 1 })
                            .unwrap_or(0);
                        self.job_state.select(Some(i));
                    }
                } else {
                    let len = self.queues.len();
                    if len > 0 {
                        let i = self
                            .queue_state
                            .selected()
                            .map(|i| if i == 0 { len - 1 } else { i - 1 })
                            .unwrap_or(0);
                        self.queue_state.select(Some(i));
                    }
                }
            }
            KeyCode::Enter => {
                if !self.focus_jobs {
                    if let Some(selected) = self.queue_state.selected() {
                        if selected < self.queues.len() {
                            self.selected_queue = Some(self.queues[selected].name.clone());
                            self.job_state.select(None);
                            self.load_jobs(&ctx.client, &ctx.current_database);
                            self.focus_jobs = true;
                        }
                    }
                }
            }
            KeyCode::Char('r') => {
                self.refresh(&ctx.client, &ctx.current_database);
                ctx.set_status("Refreshed jobs");
            }
            _ => {}
        }
    }
}

impl Default for JobsView {
    fn default() -> Self {
        Self::new()
    }
}
