//! TUI Application state and event loop

use super::client::TuiClient;
use super::ui;
use super::views::{
    ClusterView, DatabasesView, DocumentsView, HelpView, IndexesView, JobsView, QueryView, View,
};
use super::TuiArgs;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::Arc;
use std::time::Duration;

/// Current view in the TUI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentView {
    Databases,
    Documents,
    Query,
    Indexes,
    Jobs,
    Cluster,
}

impl CurrentView {
    pub fn index(&self) -> usize {
        match self {
            CurrentView::Databases => 0,
            CurrentView::Documents => 1,
            CurrentView::Query => 2,
            CurrentView::Indexes => 3,
            CurrentView::Jobs => 4,
            CurrentView::Cluster => 5,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => CurrentView::Databases,
            1 => CurrentView::Documents,
            2 => CurrentView::Query,
            3 => CurrentView::Indexes,
            4 => CurrentView::Jobs,
            5 => CurrentView::Cluster,
            _ => CurrentView::Databases,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            CurrentView::Databases => "Databases",
            CurrentView::Documents => "Documents",
            CurrentView::Query => "Query",
            CurrentView::Indexes => "Indexes",
            CurrentView::Jobs => "Jobs",
            CurrentView::Cluster => "Cluster",
        }
    }

    pub fn all() -> Vec<CurrentView> {
        vec![
            CurrentView::Databases,
            CurrentView::Documents,
            CurrentView::Query,
            CurrentView::Indexes,
            CurrentView::Jobs,
            CurrentView::Cluster,
        ]
    }
}

/// Context shared with views (without the views themselves to avoid borrow issues)
pub struct AppContext {
    pub client: Arc<TuiClient>,
    pub current_view: CurrentView,
    pub should_quit: bool,
    pub show_help: bool,
    pub current_database: String,
    pub current_collection: Option<String>,
    pub status_message: Option<String>,
    pub error_message: Option<String>,
    pub needs_refresh: bool,
}

impl AppContext {
    /// Set status message
    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
        self.error_message = None;
    }

    /// Set error message
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error_message = Some(message.into());
        self.status_message = None;
    }
}

/// Application state
pub struct App {
    pub ctx: AppContext,

    // View states
    pub databases_view: DatabasesView,
    pub documents_view: DocumentsView,
    pub query_view: QueryView,
    pub indexes_view: IndexesView,
    pub jobs_view: JobsView,
    pub cluster_view: ClusterView,
    pub help_view: HelpView,
}

impl App {
    /// Create a new application state
    pub fn new(args: &TuiArgs) -> Result<Self, String> {
        let client = Arc::new(TuiClient::new(&args.server, args.api_key.clone()));

        // Test connection
        client.test_connection()?;

        Ok(Self {
            ctx: AppContext {
                client,
                current_view: CurrentView::Databases,
                should_quit: false,
                show_help: false,
                current_database: args.database.clone(),
                current_collection: None,
                status_message: Some(format!("Connected to {}", args.server)),
                error_message: None,
                needs_refresh: false,
            },

            databases_view: DatabasesView::new(),
            documents_view: DocumentsView::new(),
            query_view: QueryView::new(),
            indexes_view: IndexesView::new(),
            jobs_view: JobsView::new(),
            cluster_view: ClusterView::new(),
            help_view: HelpView::new(),
        })
    }

    /// Switch to next view
    pub fn next_view(&mut self) {
        let idx = self.ctx.current_view.index();
        self.ctx.current_view = CurrentView::from_index((idx + 1) % 6);
        self.on_view_changed();
    }

    /// Switch to previous view
    pub fn prev_view(&mut self) {
        let idx = self.ctx.current_view.index();
        self.ctx.current_view = CurrentView::from_index((idx + 5) % 6);
        self.on_view_changed();
    }

    /// Called when view changes - handle auto-refresh and edit modes
    fn on_view_changed(&mut self) {
        match self.ctx.current_view {
            CurrentView::Query => {
                self.query_view.enter_editing();
            }
            CurrentView::Indexes => {
                if let Some(ref coll) = self.ctx.current_collection {
                    let coll = coll.clone();
                    self.indexes_view
                        .refresh(&self.ctx.client, &self.ctx.current_database, &coll);
                }
            }
            CurrentView::Documents => {
                if let Some(ref coll) = self.ctx.current_collection {
                    let coll = coll.clone();
                    self.documents_view
                        .refresh(&self.ctx.client, &self.ctx.current_database, &coll);
                }
            }
            CurrentView::Jobs => {
                self.jobs_view.refresh(&self.ctx.client, &self.ctx.current_database);
            }
            CurrentView::Cluster => {
                self.cluster_view.refresh(&self.ctx.client);
            }
            _ => {}
        }
    }

    /// Handle global key events
    pub fn handle_key(&mut self, key: KeyCode, modifiers: KeyModifiers) -> bool {
        // Global shortcuts
        match key {
            // Ctrl+C always quits
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.ctx.should_quit = true;
                return true;
            }
            KeyCode::Char('q') if !self.query_view.is_editing() => {
                self.ctx.should_quit = true;
                return true;
            }
            KeyCode::Char('?') if !self.query_view.is_editing() => {
                self.ctx.show_help = !self.ctx.show_help;
                return true;
            }
            KeyCode::Esc => {
                if self.ctx.show_help {
                    self.ctx.show_help = false;
                    return true;
                }
            }
            KeyCode::Char('r') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.refresh_current_view();
                return true;
            }
            KeyCode::Tab if !self.query_view.is_editing() => {
                self.next_view();
                return true;
            }
            KeyCode::BackTab if !self.query_view.is_editing() => {
                self.prev_view();
                return true;
            }
            KeyCode::Char('1') if !self.query_view.is_editing() => {
                self.ctx.current_view = CurrentView::Databases;
                return true;
            }
            KeyCode::Char('2') if !self.query_view.is_editing() => {
                self.ctx.current_view = CurrentView::Documents;
                return true;
            }
            KeyCode::Char('3') if !self.query_view.is_editing() => {
                self.ctx.current_view = CurrentView::Query;
                self.query_view.enter_editing();
                return true;
            }
            KeyCode::Char('4') if !self.query_view.is_editing() => {
                self.ctx.current_view = CurrentView::Indexes;
                if let Some(ref coll) = self.ctx.current_collection {
                    let coll = coll.clone();
                    self.indexes_view
                        .refresh(&self.ctx.client, &self.ctx.current_database, &coll);
                }
                return true;
            }
            KeyCode::Char('5') if !self.query_view.is_editing() => {
                self.ctx.current_view = CurrentView::Jobs;
                self.jobs_view.refresh(&self.ctx.client, &self.ctx.current_database);
                return true;
            }
            KeyCode::Char('6') if !self.query_view.is_editing() => {
                self.ctx.current_view = CurrentView::Cluster;
                self.cluster_view.refresh(&self.ctx.client);
                return true;
            }
            _ => {}
        }

        false
    }

    /// Refresh the current view's data
    pub fn refresh_current_view(&mut self) {
        match self.ctx.current_view {
            CurrentView::Databases => {
                self.databases_view.refresh(&self.ctx.client);
            }
            CurrentView::Documents => {
                if let Some(ref collection) = self.ctx.current_collection {
                    self.documents_view.refresh(
                        &self.ctx.client,
                        &self.ctx.current_database,
                        collection,
                    );
                }
            }
            CurrentView::Query => {
                // Query view doesn't auto-refresh
            }
            CurrentView::Indexes => {
                if let Some(ref collection) = self.ctx.current_collection {
                    self.indexes_view.refresh(
                        &self.ctx.client,
                        &self.ctx.current_database,
                        collection,
                    );
                }
            }
            CurrentView::Jobs => {
                self.jobs_view.refresh(&self.ctx.client, &self.ctx.current_database);
            }
            CurrentView::Cluster => {
                self.cluster_view.refresh(&self.ctx.client);
            }
        }
        self.ctx.set_status("Refreshed");
    }

    /// Initialize data on startup
    pub fn initialize(&mut self) {
        self.databases_view.refresh(&self.ctx.client);
        self.cluster_view.refresh(&self.ctx.client);
    }
}

/// Run the TUI application
pub fn run(args: TuiArgs) -> anyhow::Result<()> {
    // Setup terminal (mouse capture disabled to allow text selection/copy)
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = match App::new(&args) {
        Ok(app) => app,
        Err(e) => {
            // Restore terminal on error
            disable_raw_mode()?;
            execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
            terminal.show_cursor()?;
            return Err(anyhow::anyhow!("Failed to connect: {}", e));
        }
    };

    // Initialize data
    app.initialize();

    // Run event loop
    let result = run_app(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Main event loop
fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    loop {
        // Check if a view needs refresh (e.g., after selecting a collection)
        if app.ctx.needs_refresh {
            app.ctx.needs_refresh = false;
            if let Some(ref coll) = app.ctx.current_collection {
                let coll = coll.clone();
                match app.ctx.current_view {
                    CurrentView::Documents => {
                        app.documents_view
                            .refresh(&app.ctx.client, &app.ctx.current_database, &coll);
                    }
                    CurrentView::Indexes => {
                        app.indexes_view
                            .refresh(&app.ctx.client, &app.ctx.current_database, &coll);
                    }
                    _ => {}
                }
            }
        }

        // Draw UI
        terminal.draw(|f| ui::draw(f, app))?;

        // Handle events with timeout for responsive UI
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                // Handle help overlay first
                if app.ctx.show_help {
                    if key.code == KeyCode::Esc || key.code == KeyCode::Char('?') {
                        app.ctx.show_help = false;
                    }
                    continue;
                }

                // Try global key handlers first
                if app.handle_key(key.code, key.modifiers) {
                    if app.ctx.should_quit {
                        return Ok(());
                    }
                    continue;
                }

                // Delegate to current view
                match app.ctx.current_view {
                    CurrentView::Databases => {
                        app.databases_view
                            .handle_key(&mut app.ctx, key.code, key.modifiers);
                    }
                    CurrentView::Documents => {
                        app.documents_view
                            .handle_key(&mut app.ctx, key.code, key.modifiers);
                    }
                    CurrentView::Query => {
                        app.query_view
                            .handle_key(&mut app.ctx, key.code, key.modifiers);
                    }
                    CurrentView::Indexes => {
                        app.indexes_view
                            .handle_key(&mut app.ctx, key.code, key.modifiers);
                    }
                    CurrentView::Jobs => {
                        app.jobs_view
                            .handle_key(&mut app.ctx, key.code, key.modifiers);
                    }
                    CurrentView::Cluster => {
                        app.cluster_view
                            .handle_key(&mut app.ctx, key.code, key.modifiers);
                    }
                }
            }
        }

        // Check for quit
        if app.ctx.should_quit {
            return Ok(());
        }
    }
}
