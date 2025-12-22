use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Row, Table},
    Frame,
};

use crate::app::{App, SortColumn};
use crate::system::format_bytes;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    // htop header style: black text on green background
    let header_style = Style::default()
        .fg(theme.header_fg)
        .bg(theme.header_bg)
        .add_modifier(Modifier::BOLD);

    // Build header with sort indicator - htop style
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
            let matches_search = proc.matches_search;

            // Tree prefix for tree view
            let tree_prefix = if app.tree_view {
                if proc.has_children {
                    if proc.is_collapsed { format!("{}[+]", proc.tree_prefix) }
                    else { format!("{}[-]", proc.tree_prefix) }
                } else { proc.tree_prefix.clone() }
            } else { String::new() };

            // Pre-format all cell text once (avoid duplication)
            let texts: [String; 15] = [
                if is_selected { format!("▶{:>5}", proc.pid) } else { format!("{:>6}", proc.pid) },
                format!("{:>6}", proc.parent_pid),
                format!("{:10}", truncate_str(&proc.user, 10)),
                format!("{:>3}", proc.priority),
                format!("{:>3}", proc.nice),
                format!("{:>3}", proc.thread_count),
                format!("{:>7}", format_bytes(proc.virtual_mem)),
                format!("{:>7}", format_bytes(proc.resident_mem)),
                format!("{:>7}", format_bytes(proc.shared_mem)),
                format!("{}", proc.status),
                format!("{:>5.1}", proc.cpu_percent),
                format!("{:>5.1}", proc.mem_percent),
                format!("{:>9}", proc.format_cpu_time()),
                format!("{:>7}", format_start_time(proc.start_time)),
                format!("{}{}", tree_prefix, proc.command),
            ];

            // Build cells - uniform color when selected, varied colors otherwise
            let cells: Vec<Span> = if is_selected {
                let s = Style::default().fg(theme.selection_fg);
                texts.into_iter().map(|t| Span::styled(t, s)).collect()
            } else {
                let pri_color = theme.priority_color_for_nice(proc.nice);
                let cmd_color = if app.tree_view && !tree_prefix.is_empty() { theme.process_tree } else { theme.text };
                vec![
                    Span::styled(texts[0].clone(), Style::default().fg(theme.pid_color)),
                    Span::styled(texts[1].clone(), Style::default().fg(theme.text_dim)),
                    Span::styled(texts[2].clone(), Style::default().fg(theme.user_color)),
                    Span::styled(texts[3].clone(), Style::default().fg(pri_color)),
                    Span::styled(texts[4].clone(), Style::default().fg(pri_color)),
                    Span::styled(texts[5].clone(), Style::default().fg(theme.threads_color)),
                    Span::styled(texts[6].clone(), Style::default().fg(theme.memory_size_color(proc.virtual_mem))),
                    Span::styled(texts[7].clone(), Style::default().fg(theme.memory_size_color(proc.resident_mem))),
                    Span::styled(texts[8].clone(), Style::default().fg(theme.memory_size_color(proc.shared_mem))),
                    Span::styled(texts[9].clone(), Style::default().fg(theme.status_color(proc.status))),
                    Span::styled(texts[10].clone(), Style::default().fg(theme.cpu_color(proc.cpu_percent))),
                    Span::styled(texts[11].clone(), Style::default().fg(theme.mem_low)),
                    Span::styled(texts[12].clone(), Style::default().fg(theme.time_color)),
                    Span::styled(texts[13].clone(), Style::default().fg(theme.text_dim)),
                    Span::styled(texts[14].clone(), Style::default().fg(cmd_color)),
                ]
            };

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
