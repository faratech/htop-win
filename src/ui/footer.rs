use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::{App, FocusRegion, ViewMode};

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let function_keys = get_function_keys_with_num(app);
    let theme = &app.theme;

    // Check if footer is focused for keyboard navigation
    let footer_focused = app.focus_region == FocusRegion::Footer;
    let focused_key_index = app.focus_index;

    // Track x position for registering function key bounds
    let mut x_pos = area.x;
    let mut key_index = 0usize;

    // htop style: F1Help  F2Setup (key is black on cyan, label is white, no space between)
    let spans: Vec<Span> = function_keys
        .iter()
        .flat_map(|(key_num, key_str, label)| {
            if key_str.is_empty() {
                // Empty key/label pair - just add spacing
                let spacing_width = 7u16;
                x_pos += spacing_width;
                vec![Span::styled("       ", Style::default().bg(theme.background))]
            } else {
                // Calculate total width: key text + label (6 chars fixed width)
                let key_width = key_str.len() as u16;
                let label_width = 6u16;
                let total_width = key_width + label_width;

                // Register function key region if it's a valid F-key
                if let Some(num) = key_num {
                    app.ui_bounds.add_function_key(*num, x_pos, area.y, total_width);
                }

                x_pos += total_width;

                // Check if this key is focused
                let is_focused = footer_focused && key_num.is_some() && key_index == focused_key_index;
                key_index += if key_num.is_some() { 1 } else { 0 };

                // Use inverted colors for focused key
                let (key_fg, key_bg, label_fg, label_bg) = if is_focused {
                    // Highlighted/focused: invert colors
                    (theme.header_key_bg, theme.header_key_fg, theme.background, theme.text)
                } else {
                    // Normal
                    (theme.header_key_fg, theme.header_key_bg, theme.text, theme.background)
                };

                vec![
                    Span::styled(
                        key_str.to_string(),
                        Style::default().fg(key_fg).bg(key_bg),
                    ),
                    Span::styled(
                        format!("{:<6}", label), // htop uses fixed-width labels with trailing space
                        Style::default().fg(label_fg).bg(label_bg),
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

/// Returns function keys with: (Option<function_key_number>, key_text, label)
/// The function key number is used for registering click regions (e.g., Some(1) for F1)
fn get_function_keys_with_num(app: &App) -> Vec<(Option<u8>, &'static str, &'static str)> {
    match app.view_mode {
        ViewMode::Help => vec![
            (Some(1), "F1", ""),
            (Some(2), "F2", ""),
            (Some(3), "F3", ""),
            (Some(4), "F4", ""),
            (Some(5), "F5", ""),
            (Some(6), "F6", ""),
            (Some(7), "F7", ""),
            (Some(8), "F8", ""),
            (Some(9), "F9", ""),
            (Some(10), "F10", "Quit"),
        ],
        ViewMode::Search => vec![
            (None, "Enter", "Done"),
            (None, "Esc", "Cancel"),
            (Some(3), "F3", "Next"),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
        ],
        ViewMode::Filter => vec![
            (None, "Enter", "Done"),
            (None, "Esc", "Cancel"),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
        ],
        ViewMode::SortSelect => vec![
            (None, "Enter", "Select"),
            (None, "Esc", "Cancel"),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
        ],
        ViewMode::Kill => vec![
            (None, "Enter", "Kill"),
            (None, "Esc", "Cancel"),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
        ],
        ViewMode::Nice => vec![
            (None, "←/→", "Adjust"),
            (None, "Enter", "Set"),
            (None, "Esc", "Cancel"),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
        ],
        ViewMode::SignalSelect => vec![
            (None, "Enter", "Kill"),
            (None, "Esc", "Back"),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
        ],
        ViewMode::UserSelect => vec![
            (None, "Enter", "Select"),
            (None, "Esc", "Cancel"),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
        ],
        ViewMode::Environment => vec![
            (None, "Esc", "Close"),
            (None, "↑↓", "Scroll"),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
        ],
        ViewMode::ColorScheme => vec![
            (None, "Enter", "Select"),
            (None, "Esc", "Back"),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
        ],
        ViewMode::CommandWrap => vec![
            (None, "Esc", "Close"),
            (None, "↑↓", "Scroll"),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
        ],
        ViewMode::ColumnConfig => vec![
            (None, "Space", "Toggle"),
            (None, "Esc", "Done"),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
        ],
        ViewMode::Affinity => vec![
            (None, "Space", "Toggle"),
            (None, "a", "All"),
            (None, "n", "None"),
            (None, "Enter", "Apply"),
            (None, "Esc", "Cancel"),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
            (None, "", ""),
        ],
        ViewMode::Normal | ViewMode::Setup | ViewMode::ProcessInfo => vec![
            (Some(1), "F1", "Help"),
            (Some(2), "F2", "Setup"),
            (Some(3), "F3", "Search"),
            (Some(4), "F4", "Filter"),
            (Some(5), "F5", "Tree"),
            (Some(6), "F6", "Sort"),
            (Some(7), "F7", "Nice-"),
            (Some(8), "F8", "Nice+"),
            (Some(9), "F9", "Kill"),
            (Some(10), "F10", "Quit"),
        ],
    }
}

fn build_status_line(app: &App) -> Vec<Span<'static>> {
    let mut spans = Vec::new();

    // Show focus region indicator (Tab to switch)
    let focus_indicator = match app.focus_region {
        FocusRegion::Header => "[Focus:Header] ",
        FocusRegion::ProcessList => "", // Don't show when on default
        FocusRegion::Footer => "[Focus:Footer] ",
    };
    if !focus_indicator.is_empty() {
        spans.push(Span::styled(
            focus_indicator,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    }

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
