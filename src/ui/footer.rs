use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::{App, ViewMode};

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let function_keys = get_function_keys(app);
    let theme = &app.theme;

    // htop style: F1Help  F2Setup (key is black on cyan, label is white, no space between)
    let spans: Vec<Span> = function_keys
        .iter()
        .flat_map(|(key, label)| {
            if key.is_empty() {
                // Empty key/label pair - just add spacing
                vec![Span::styled("       ", Style::default().bg(theme.background))]
            } else {
                vec![
                    Span::styled(
                        key.to_string(),
                        Style::default()
                            .fg(theme.header_key_fg)
                            .bg(theme.header_key_bg),
                    ),
                    Span::styled(
                        format!("{:<6}", label), // htop uses fixed-width labels with trailing space
                        Style::default().fg(theme.text).bg(theme.background),
                    ),
                ]
            }
        })
        .collect();

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line).style(Style::default().bg(theme.background));
    frame.render_widget(paragraph, area);

    // Second line: filter/search status
    if area.height > 1 {
        let status_area = Rect::new(area.x, area.y + 1, area.width, 1);
        let status_spans = build_status_line(app);
        let status_line = Line::from(status_spans);
        let status_para = Paragraph::new(status_line).style(Style::default().bg(theme.background));
        frame.render_widget(status_para, status_area);
    }
}

fn get_function_keys(app: &App) -> Vec<(&'static str, &'static str)> {
    match app.view_mode {
        ViewMode::Help => vec![
            ("F1", ""),
            ("F2", ""),
            ("F3", ""),
            ("F4", ""),
            ("F5", ""),
            ("F6", ""),
            ("F7", ""),
            ("F8", ""),
            ("F9", ""),
            ("F10", "Quit"),
        ],
        ViewMode::Search => vec![
            ("Enter", "Done"),
            ("Esc", "Cancel"),
            ("F3", "Next"),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
        ],
        ViewMode::Filter => vec![
            ("Enter", "Done"),
            ("Esc", "Cancel"),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
        ],
        ViewMode::SortSelect => vec![
            ("Enter", "Select"),
            ("Esc", "Cancel"),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
        ],
        ViewMode::Kill => vec![
            ("Enter", "Kill"),
            ("Esc", "Cancel"),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
        ],
        ViewMode::Nice => vec![
            ("←/→", "Adjust"),
            ("Enter", "Set"),
            ("Esc", "Cancel"),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
        ],
        ViewMode::SignalSelect => vec![
            ("Enter", "Kill"),
            ("Esc", "Back"),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
        ],
        ViewMode::UserSelect => vec![
            ("Enter", "Select"),
            ("Esc", "Cancel"),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
        ],
        ViewMode::Environment => vec![
            ("Esc", "Close"),
            ("↑↓", "Scroll"),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
        ],
        ViewMode::ColorScheme => vec![
            ("Enter", "Select"),
            ("Esc", "Back"),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
        ],
        ViewMode::CommandWrap => vec![
            ("Esc", "Close"),
            ("↑↓", "Scroll"),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
        ],
        ViewMode::ColumnConfig => vec![
            ("Space", "Toggle"),
            ("Esc", "Done"),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
        ],
        ViewMode::Affinity => vec![
            ("Space", "Toggle"),
            ("a", "All"),
            ("n", "None"),
            ("Enter", "Apply"),
            ("Esc", "Cancel"),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
            ("", ""),
        ],
        ViewMode::Normal | ViewMode::Setup | ViewMode::ProcessInfo => vec![
            ("F1", "Help"),
            ("F2", "Setup"),
            ("F3", "Search"),
            ("F4", "Filter"),
            ("F5", "Tree"),
            ("F6", "Sort"),
            ("F7", "Nice-"),
            ("F8", "Nice+"),
            ("F9", "Kill"),
            ("F10", "Quit"),
        ],
    }
}

fn build_status_line(app: &App) -> Vec<Span<'static>> {
    let mut spans = Vec::new();

    // Show paused indicator (high priority - show first)
    if app.paused {
        spans.push(Span::styled(
            "[PAUSED] ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    }

    // Show follow mode indicator
    if let Some(pid) = app.follow_pid {
        spans.push(Span::styled(
            format!("[Follow:{}] ", pid),
            Style::default().fg(Color::Magenta),
        ));
    }

    // Show user filter if active
    if let Some(ref user) = app.user_filter {
        spans.push(Span::styled(
            format!("User: {} ", user),
            Style::default().fg(Color::Magenta),
        ));
    }

    // Show filter if active
    if !app.filter_string.is_empty() {
        spans.push(Span::styled(
            "Filter: ",
            Style::default().fg(Color::Yellow),
        ));
        spans.push(Span::styled(
            app.filter_string.clone(),
            Style::default().fg(Color::White),
        ));
        spans.push(Span::raw("  "));
    }

    // Show search if active
    if !app.search_string.is_empty() {
        spans.push(Span::styled(
            "Search: ",
            Style::default().fg(Color::Cyan),
        ));
        spans.push(Span::styled(
            app.search_string.clone(),
            Style::default().fg(Color::White),
        ));
        spans.push(Span::raw("  "));
    }

    // Show tree mode
    if app.tree_view {
        spans.push(Span::styled(
            "[Tree] ",
            Style::default().fg(Color::Green),
        ));
    }

    // Show tagged count
    if !app.tagged_pids.is_empty() {
        spans.push(Span::styled(
            format!("[{} tagged] ", app.tagged_pids.len()),
            Style::default().fg(Color::Yellow),
        ));
    }

    // Show sort column
    spans.push(Span::styled(
        format!(
            "Sort: {}{} ",
            app.sort_column.name(),
            if app.sort_ascending { "↑" } else { "↓" }
        ),
        Style::default().fg(Color::DarkGray),
    ));

    spans
}
