//! Reusable widgets for TUI

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Render JSON value with syntax highlighting
pub fn render_json(value: &serde_json::Value, area: Rect, f: &mut Frame, title: &str) {
    let json_str = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());

    let lines: Vec<Line> = json_str
        .lines()
        .map(|line| {
            let mut spans = Vec::new();
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;

            while i < chars.len() {
                let c = chars[i];

                // Handle string values (in quotes)
                if c == '"' {
                    let start = i;
                    i += 1;
                    while i < chars.len() && chars[i] != '"' {
                        if chars[i] == '\\' && i + 1 < chars.len() {
                            i += 1;
                        }
                        i += 1;
                    }
                    i += 1; // Include closing quote
                    let s: String = chars[start..i].iter().collect();

                    // Check if this is a key (followed by :) or value
                    let is_key = i < chars.len()
                        && chars[i..]
                            .iter()
                            .take_while(|&&c| c == ' ' || c == ':')
                            .any(|&c| c == ':');

                    if is_key {
                        spans.push(Span::styled(s, Style::default().fg(Color::Cyan)));
                    } else {
                        spans.push(Span::styled(s, Style::default().fg(Color::Green)));
                    }
                }
                // Handle numbers
                else if c.is_ascii_digit()
                    || (c == '-' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
                {
                    let start = i;
                    while i < chars.len()
                        && (chars[i].is_ascii_digit()
                            || chars[i] == '.'
                            || chars[i] == '-'
                            || chars[i] == 'e'
                            || chars[i] == 'E')
                    {
                        i += 1;
                    }
                    let s: String = chars[start..i].iter().collect();
                    spans.push(Span::styled(s, Style::default().fg(Color::Yellow)));
                }
                // Handle booleans and null
                else if line[i..].starts_with("true")
                    || line[i..].starts_with("false")
                    || line[i..].starts_with("null")
                {
                    let word = if line[i..].starts_with("true") {
                        "true"
                    } else if line[i..].starts_with("false") {
                        "false"
                    } else {
                        "null"
                    };
                    spans.push(Span::styled(
                        word.to_string(),
                        Style::default().fg(Color::Magenta),
                    ));
                    i += word.len();
                }
                // Handle brackets and braces
                else if c == '{' || c == '}' || c == '[' || c == ']' {
                    spans.push(Span::styled(
                        c.to_string(),
                        Style::default().fg(Color::White),
                    ));
                    i += 1;
                }
                // Handle other characters
                else {
                    spans.push(Span::raw(c.to_string()));
                    i += 1;
                }
            }

            Line::from(spans)
        })
        .collect();

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", title))
                .title_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}

/// Create a centered popup area
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Format a document key for display
pub fn format_key(key: &str) -> String {
    if key.len() > 20 {
        format!("{}...", &key[..17])
    } else {
        key.to_string()
    }
}

/// Format a JSON value preview for table display
pub fn format_value_preview(value: &serde_json::Value, max_len: usize) -> String {
    let s = match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(a) => format!("[{} items]", a.len()),
        serde_json::Value::Object(o) => format!("{{{} fields}}", o.len()),
    };

    if s.len() > max_len {
        format!("{}...", &s[..max_len - 3])
    } else {
        s
    }
}
