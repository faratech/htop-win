use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Row, Table},
    Frame,
};

use crate::app::{App, SortColumn};
use crate::system::format_bytes;

/// Get column width constraint for a given column
fn column_width(col: &SortColumn) -> Constraint {
    match col {
        SortColumn::Pid => Constraint::Length(7),
        SortColumn::PPid => Constraint::Length(7),
        SortColumn::User => Constraint::Length(10),
        SortColumn::Priority => Constraint::Length(4),
        SortColumn::Nice => Constraint::Length(4),
        SortColumn::Threads => Constraint::Length(4),
        SortColumn::Virt => Constraint::Length(8),
        SortColumn::Res => Constraint::Length(8),
        SortColumn::Shr => Constraint::Length(8),
        SortColumn::Status => Constraint::Length(2),
        SortColumn::Cpu => Constraint::Length(6),
        SortColumn::Mem => Constraint::Length(6),
        SortColumn::Time => Constraint::Length(10),
        SortColumn::StartTime => Constraint::Length(8),
        SortColumn::Command => Constraint::Min(20),
    }
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    // Get visible columns
    let visible_columns: Vec<SortColumn> = SortColumn::all()
        .iter()
        .filter(|col| app.config.is_column_visible(col.name()))
        .copied()
        .collect();

    // htop header style: black text on green background
    let header_style = Style::default()
        .fg(theme.header_fg)
        .bg(theme.header_bg)
        .add_modifier(Modifier::BOLD);

    // Build header with sort indicator - only for visible columns
    let header_cells: Vec<Span> = visible_columns
        .iter()
        .map(|col| {
            let name = col.name();
            let indicator = if *col == app.sort_column {
                if app.sort_ascending { "▲" } else { "▼" }
            } else {
                ""
            };
            Span::raw(format!("{}{}", name, indicator))
        })
        .collect();

    let header = Row::new(header_cells).style(header_style).height(1);

    // Column widths for visible columns only
    let widths: Vec<Constraint> = visible_columns.iter().map(column_width).collect();

    // Build rows
    let rows: Vec<Row> = app
        .displayed_processes
        .iter()
        .enumerate()
        .skip(app.scroll_offset)
        .take(app.visible_height)
        .map(|(idx, proc)| {
            let is_selected = idx == app.selected_index;
            let is_tagged = app.tagged_pids.contains(&proc.pid);
            let matches_search = proc.matches_search;

            // Tree prefix for tree view
            let tree_prefix = if app.tree_view {
                if proc.has_children {
                    if proc.is_collapsed { format!("{}[+]", proc.tree_prefix) }
                    else { format!("{}[-]", proc.tree_prefix) }
                } else { proc.tree_prefix.clone() }
            } else { String::new() };

            // Choose between full command path or just the program name
            let display_command = if app.config.show_program_path {
                &proc.command
            } else {
                &proc.name
            };

            // Build cells only for visible columns
            let cells: Vec<Span> = visible_columns
                .iter()
                .map(|col| {
                    let (text, color) = match col {
                        SortColumn::Pid => (
                            if is_selected { format!("▶{:>5}", proc.pid) } else { format!("{:>6}", proc.pid) },
                            if is_selected { theme.selection_fg } else { theme.pid_color }
                        ),
                        SortColumn::PPid => (
                            format!("{:>6}", proc.parent_pid),
                            if is_selected { theme.selection_fg } else { theme.text_dim }
                        ),
                        SortColumn::User => (
                            format!("{:10}", truncate_str(&proc.user, 10)),
                            if is_selected { theme.selection_fg } else { theme.user_color }
                        ),
                        SortColumn::Priority => (
                            format!("{:>3}", proc.priority),
                            if is_selected { theme.selection_fg } else { theme.priority_color_for_nice(proc.nice) }
                        ),
                        SortColumn::Nice => (
                            format!("{:>3}", proc.nice),
                            if is_selected { theme.selection_fg } else { theme.priority_color_for_nice(proc.nice) }
                        ),
                        SortColumn::Threads => (
                            format!("{:>3}", proc.thread_count),
                            if is_selected { theme.selection_fg } else { theme.threads_color }
                        ),
                        SortColumn::Virt => (
                            format!("{:>7}", format_bytes(proc.virtual_mem)),
                            if is_selected { theme.selection_fg } else { theme.memory_size_color(proc.virtual_mem) }
                        ),
                        SortColumn::Res => (
                            format!("{:>7}", format_bytes(proc.resident_mem)),
                            if is_selected { theme.selection_fg } else { theme.memory_size_color(proc.resident_mem) }
                        ),
                        SortColumn::Shr => (
                            format!("{:>7}", format_bytes(proc.shared_mem)),
                            if is_selected { theme.selection_fg } else { theme.memory_size_color(proc.shared_mem) }
                        ),
                        SortColumn::Status => (
                            format!("{}", proc.status),
                            if is_selected { theme.selection_fg } else { theme.status_color(proc.status) }
                        ),
                        SortColumn::Cpu => (
                            format!("{:>5.1}", proc.cpu_percent),
                            if is_selected { theme.selection_fg } else { theme.cpu_color(proc.cpu_percent) }
                        ),
                        SortColumn::Mem => (
                            format!("{:>5.1}", proc.mem_percent),
                            if is_selected { theme.selection_fg } else { theme.mem_low }
                        ),
                        SortColumn::Time => (
                            format!("{:>9}", proc.format_cpu_time()),
                            if is_selected { theme.selection_fg } else { theme.time_color }
                        ),
                        SortColumn::StartTime => (
                            format!("{:>7}", format_start_time(proc.start_time)),
                            if is_selected { theme.selection_fg } else { theme.text_dim }
                        ),
                        SortColumn::Command => {
                            let cmd_color = if app.tree_view && !tree_prefix.is_empty() {
                                theme.process_tree
                            } else {
                                theme.text
                            };
                            (
                                format!("{}{}", tree_prefix, display_command),
                                if is_selected { theme.selection_fg } else { cmd_color }
                            )
                        }
                    };
                    Span::styled(text, Style::default().fg(color))
                })
                .collect();

            // Row styling - always set background from theme
            let row_style = if is_selected {
                Style::default().bg(theme.selection_bg).add_modifier(Modifier::BOLD)
            } else if matches_search {
                Style::default().bg(theme.search_match)
            } else if is_tagged {
                Style::default().fg(theme.process_tag).bg(theme.background)
            } else {
                Style::default().bg(theme.background)
            };

            Row::new(cells).style(row_style)
        })
        .collect();

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::NONE))
        .row_highlight_style(Style::default())
        .column_spacing(1);

    frame.render_widget(table, area);
}

fn truncate_str(s: &str, max_len: usize) -> String {
    use unicode_width::UnicodeWidthStr;

    let width = s.width();
    if width <= max_len {
        s.to_string()
    } else {
        // Safely truncate by characters, not bytes
        let mut result = String::new();
        let mut current_width = 0;
        for c in s.chars() {
            let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
            if current_width + char_width >= max_len {
                result.push('…');
                break;
            }
            result.push(c);
            current_width += char_width;
        }
        result
    }
}

/// Format a Unix timestamp as elapsed time or time of day
fn format_start_time(start_time: u64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    if start_time == 0 {
        return "-".to_string();
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if start_time > now {
        return "-".to_string();
    }

    let elapsed_secs = now - start_time;

    // If started today, show as HH:MM
    // If started more than a day ago, show as days
    if elapsed_secs < 60 {
        format!("{}s", elapsed_secs)
    } else if elapsed_secs < 3600 {
        format!("{}m", elapsed_secs / 60)
    } else if elapsed_secs < 86400 {
        format!("{}h{}m", elapsed_secs / 3600, (elapsed_secs % 3600) / 60)
    } else {
        let days = elapsed_secs / 86400;
        if days > 99 {
            format!("{}d", days)
        } else {
            format!("{}d{}h", days, (elapsed_secs % 86400) / 3600)
        }
    }
}
