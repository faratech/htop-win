use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Row, Table},
    Frame,
};

use crate::app::{App, SortColumn};
use crate::system::format_bytes;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let header_style = Style::default()
        .fg(theme.header_key_fg)
        .bg(Color::Green)
        .add_modifier(Modifier::BOLD);

    // Build header with sort indicator
    let header_cells: Vec<Span> = SortColumn::all()
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

    // Column widths
    let widths = [
        Constraint::Length(7),  // PID
        Constraint::Length(7),  // PPID
        Constraint::Length(10), // USER
        Constraint::Length(4),  // PRI
        Constraint::Length(4),  // NI
        Constraint::Length(4),  // THR (Threads)
        Constraint::Length(8),  // VIRT
        Constraint::Length(8),  // RES
        Constraint::Length(8),  // SHR
        Constraint::Length(2),  // S
        Constraint::Length(6),  // CPU%
        Constraint::Length(6),  // MEM%
        Constraint::Length(10), // TIME+
        Constraint::Length(8),  // START
        Constraint::Min(20),    // Command
    ];

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

            // Search highlighting
            let matches_search = !app.search_string.is_empty() && {
                let search_lower = app.search_string.to_lowercase();
                proc.name.to_lowercase().contains(&search_lower)
                    || proc.command.to_lowercase().contains(&search_lower)
            };

            // Process status color using theme
            let status_color = theme.status_color(proc.status);

            // CPU color based on usage using theme
            let cpu_color = theme.cpu_color(proc.cpu_percent);

            // Memory color based on usage using theme
            let mem_color = theme.mem_color(proc.mem_percent);

            // Tree prefix - use pre-computed tree_prefix for proper tree lines
            let tree_prefix = if app.tree_view {
                // Add collapse/expand indicator for processes with children
                if proc.has_children {
                    if proc.is_collapsed {
                        format!("{}[+]", proc.tree_prefix)
                    } else {
                        format!("{}[-]", proc.tree_prefix)
                    }
                } else {
                    proc.tree_prefix.clone()
                }
            } else {
                String::new()
            };

            let command_display = format!("{}{}", tree_prefix, proc.command);

            // Format start time as elapsed time or time of day
            let start_time_str = format_start_time(proc.start_time);

            let cells = vec![
                Span::styled(format!("{:>6}", proc.pid), Style::default().fg(Color::Cyan)),
                Span::styled(format!("{:>6}", proc.parent_pid), Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:10}", truncate_str(&proc.user, 10)),
                    Style::default().fg(Color::White),
                ),
                Span::styled(format!("{:>3}", proc.priority), Style::default().fg(Color::White)),
                Span::styled(format!("{:>3}", proc.nice), Style::default().fg(Color::White)),
                Span::styled(format!("{:>3}", proc.thread_count), Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:>7}", format_bytes(proc.virtual_mem)),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("{:>7}", format_bytes(proc.resident_mem)),
                    Style::default().fg(Color::Green),
                ),
                Span::styled(
                    format!("{:>7}", format_bytes(proc.shared_mem)),
                    Style::default().fg(Color::White),
                ),
                Span::styled(format!("{}", proc.status), Style::default().fg(status_color)),
                Span::styled(format!("{:>5.1}", proc.cpu_percent), Style::default().fg(cpu_color)),
                Span::styled(format!("{:>5.1}", proc.mem_percent), Style::default().fg(mem_color)),
                Span::styled(
                    format!("{:>9}", proc.format_cpu_time()),
                    Style::default().fg(Color::White),
                ),
                Span::styled(format!("{:>7}", start_time_str), Style::default().fg(Color::DarkGray)),
                Span::styled(command_display, Style::default().fg(Color::White)),
            ];

            let mut row_style = Style::default().fg(theme.text);

            if is_selected {
                row_style = row_style
                    .bg(theme.selection_bg)
                    .fg(theme.selection_fg)
                    .add_modifier(Modifier::BOLD);
            }

            if is_tagged {
                row_style = row_style.fg(theme.tagged);
            }

            if matches_search {
                row_style = row_style.bg(theme.search_match);
            }

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
