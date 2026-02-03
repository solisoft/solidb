//! Main layout components for TUI

use crate::cli::tui::app::{AppContext, CurrentView};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

/// Draw the sidebar navigation
pub fn draw_sidebar(f: &mut Frame, ctx: &AppContext, area: Rect) {
    let views = CurrentView::all();

    let items: Vec<ListItem> = views
        .iter()
        .enumerate()
        .map(|(i, view)| {
            let content = format!(" {} {}", i + 1, view.name());
            let style = if *view == ctx.current_view {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" SoliDB TUI ")
                .title_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_widget(list, area);

    // Draw navigation hints at the bottom of sidebar
    let hint_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(3),
        width: area.width,
        height: 2,
    };

    let hints = Paragraph::new(vec![
        Line::from(Span::styled(
            " Tab: Next",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            " ?: Help",
            Style::default().fg(Color::DarkGray),
        )),
    ]);

    f.render_widget(hints, hint_area);
}

/// Draw the status bar
pub fn draw_status_bar(f: &mut Frame, ctx: &AppContext, area: Rect) {
    let db_info = format!(
        " DB: {} | Collection: {} ",
        ctx.current_database,
        ctx.current_collection.as_deref().unwrap_or("-")
    );

    let message = if let Some(ref err) = ctx.error_message {
        Span::styled(format!(" Error: {} ", err), Style::default().fg(Color::Red))
    } else if let Some(ref msg) = ctx.status_message {
        Span::styled(format!(" {} ", msg), Style::default().fg(Color::Green))
    } else {
        Span::raw("")
    };

    let status = Paragraph::new(Line::from(vec![
        Span::styled(db_info, Style::default().fg(Color::Cyan)),
        Span::raw(" | "),
        message,
    ]))
    .style(Style::default().bg(Color::DarkGray));

    f.render_widget(status, area);
}
