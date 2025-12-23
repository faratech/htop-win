use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Row, Table},
    Frame,
};

use crate::app::{App, SortColumn};
use crate::ui::colors::Theme;

/// Format CPU time with multi-colored output like htop's Row_printTime
/// - 0:00.00 = shadow (gray)
/// - < 60 min: base color
/// - Hours: hours in cyan, rest in base
/// - Days: days in green, hours in cyan
/// - Years: years in red, days in green
fn format_time_colored<'a>(duration: std::time::Duration, theme: &Theme, is_selected: bool) -> Vec<Span<'a>> {
    let total_secs = duration.as_secs();
    let centis = duration.subsec_millis() / 10;

    let (base, hour_color, day_color, year_color, shadow) = if is_selected {
        (theme.selection_fg, theme.selection_fg, theme.selection_fg, theme.selection_fg, theme.selection_fg)
    } else {
        (theme.process, theme.process_megabytes, theme.process_gigabytes, theme.large_number, theme.process_shadow)
    };

    // Zero time - show in shadow
    if total_secs == 0 && centis == 0 {
        return vec![Span::styled(" 0:00.00 ".to_string(), Style::default().fg(shadow))];
    }

    let total_mins = total_secs / 60;
    let total_hours = total_mins / 60;
    let total_days = total_hours / 24;

    let secs = total_secs % 60;
    let mins = total_mins % 60;
    let hours = total_hours % 24;

    if total_mins < 60 {
        // Minutes:seconds.centis
        vec![Span::styled(format!("{:2}:{:02}.{:02}", total_mins, secs, centis), Style::default().fg(base))]
    } else if total_hours < 24 {
        // Hours in cyan, rest in base: Xh:MM:SS
        vec![
            Span::styled(format!("{:2}h", total_hours), Style::default().fg(hour_color)),
            Span::styled(format!("{:02}:{:02}", mins, secs), Style::default().fg(base)),
        ]
    } else if total_days < 365 {
        // Days in green, hours in cyan: Xd:XXh
        vec![
            Span::styled(format!("{:3}d", total_days), Style::default().fg(day_color)),
            Span::styled(format!("{:02}h", hours), Style::default().fg(hour_color)),
        ]
    } else {
        // Years in red, days in green
        let years = total_days / 365;
        let days = total_days % 365;
        vec![
            Span::styled(format!("{:3}y", years), Style::default().fg(year_color)),
            Span::styled(format!("{:03}d", days), Style::default().fg(day_color)),
        ]
    }
}

/// Format bytes with multi-colored output like htop's Row_printKBytes
/// Returns spans with different colors for different magnitude parts
fn format_bytes_colored<'a>(bytes: u64, theme: &Theme, is_selected: bool) -> Vec<Span<'a>> {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    // htop color scheme:
    // - Default: PROCESS (white)
    // - MB range: PROCESS_MEGABYTES (cyan)
    // - GB range: PROCESS_GIGABYTES (green)
    // - TB+ range: LARGE_NUMBER (red)
    let (color_default, color_mb, color_gb, color_tb) = if is_selected {
        (theme.selection_fg, theme.selection_fg, theme.selection_fg, theme.selection_fg)
    } else {
        (theme.process, theme.process_megabytes, theme.process_gigabytes, theme.large_number)
    };

    if bytes >= TB {
        // TB range: show in red/large_number color
        let tb = bytes as f64 / TB as f64;
        if tb < 10.0 {
            vec![
                Span::styled(format!("{:.1}", tb), Style::default().fg(color_tb)),
                Span::styled("T".to_string(), Style::default().fg(color_tb)),
            ]
        } else {
            vec![Span::styled(format!("{:.0}T", tb), Style::default().fg(color_tb))]
        }
    } else if bytes >= GB {
        // GB range: integer part in green, decimal in cyan
        let gb = bytes as f64 / GB as f64;
        if gb < 10.0 {
            let int_part = gb as u64;
            let dec_part = ((gb - int_part as f64) * 10.0) as u64;
            vec![
                Span::styled(format!("{}", int_part), Style::default().fg(color_gb)),
                Span::styled(format!(".{}G", dec_part), Style::default().fg(color_mb)),
            ]
        } else {
            vec![
                Span::styled(format!("{:.0}", gb), Style::default().fg(color_gb)),
                Span::styled("G".to_string(), Style::default().fg(color_mb)),
            ]
        }
    } else if bytes >= MB {
        // MB range: show in cyan
        let mb = bytes as f64 / MB as f64;
        if mb < 10.0 {
            let int_part = mb as u64;
            let dec_part = ((mb - int_part as f64) * 10.0) as u64;
            vec![
                Span::styled(format!("{}", int_part), Style::default().fg(color_mb)),
                Span::styled(format!(".{}M", dec_part), Style::default().fg(color_default)),
            ]
        } else {
            vec![Span::styled(format!("{:.0}M", mb), Style::default().fg(color_mb))]
        }
    } else if bytes >= KB {
        // KB range: show in default
        vec![Span::styled(format!("{:.0}K", bytes as f64 / KB as f64), Style::default().fg(color_default))]
    } else {
        vec![Span::styled(format!("{}B", bytes), Style::default().fg(color_default))]
    }
}

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
        SortColumn::Status => Constraint::Length(3),  // Status + efficiency indicator
        SortColumn::Cpu => Constraint::Length(6),
        SortColumn::Mem => Constraint::Length(6),
        SortColumn::Time => Constraint::Length(10),
        SortColumn::StartTime => Constraint::Length(8),
        SortColumn::Command => Constraint::Min(20),
        // Windows-specific columns
        SortColumn::Elevated => Constraint::Length(4),
        SortColumn::Arch => Constraint::Length(5),
        SortColumn::Efficiency => Constraint::Length(4),
    }
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    // Use cached visible columns (updated when config changes)
    let visible_columns = &app.cached_visible_columns;

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

    // Cache current time for start_time formatting (avoid syscall per process)
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

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
            let cells: Vec<Cell> = visible_columns
                .iter()
                .map(|col| {
                    // Command column uses multi-span for colored indicators (htop style)
                    if *col == SortColumn::Command {
                        // Build spans with distinct colors
                        let mut spans: Vec<Span> = Vec::new();

                        // Elevated indicator - use theme's privileged process color
                        if proc.is_elevated {
                            spans.push(Span::styled(
                                "🛡️ ",
                                Style::default().fg(if is_selected { theme.selection_fg } else { theme.process_priv })
                            ));
                        }

                        // Architecture indicator - use theme's megabytes color (cyan in default)
                        let arch_str = proc.arch.as_str();
                        if !arch_str.is_empty() {
                            spans.push(Span::styled(
                                format!("[{}] ", arch_str),
                                Style::default().fg(if is_selected { theme.selection_fg } else { theme.process_megabytes })
                            ));
                        }

                        // Tree prefix with tree color
                        if !tree_prefix.is_empty() {
                            spans.push(Span::styled(
                                tree_prefix.clone(),
                                Style::default().fg(if is_selected { theme.selection_fg } else { theme.process_tree })
                            ));
                        }

                        // htop style: path in dim color, basename in bold/bright color
                        if app.config.show_program_path {
                            // Split into path and basename
                            if let Some(last_sep) = display_command.rfind(|c| c == '\\' || c == '/') {
                                let path = &display_command[..=last_sep];
                                let basename = &display_command[last_sep + 1..];
                                // Path in dimmer color
                                spans.push(Span::styled(
                                    path.to_string(),
                                    Style::default().fg(if is_selected { theme.selection_fg } else { theme.text_dim })
                                ));
                                // Basename in bright color (htop: PROCESS_BASENAME = bold cyan)
                                spans.push(Span::styled(
                                    basename.to_string(),
                                    Style::default().fg(if is_selected { theme.selection_fg } else { theme.process_basename }).add_modifier(Modifier::BOLD)
                                ));
                            } else {
                                // No path separator, just show as basename
                                spans.push(Span::styled(
                                    display_command.to_string(),
                                    Style::default().fg(if is_selected { theme.selection_fg } else { theme.process_basename }).add_modifier(Modifier::BOLD)
                                ));
                            }
                        } else {
                            // Just show name (already is the basename)
                            spans.push(Span::styled(
                                display_command.to_string(),
                                Style::default().fg(if is_selected { theme.selection_fg } else { theme.process_basename }).add_modifier(Modifier::BOLD)
                            ));
                        }

                        return Cell::from(Line::from(spans));
                    }

                    let (text, color) = match col {
                        SortColumn::Pid => (
                            if is_selected { format!("▶{:>5}", proc.pid) } else { format!("{:>6}", proc.pid) },
                            if is_selected { theme.selection_fg } else { theme.pid_color }
                        ),
                        SortColumn::PPid => (
                            format!("{:>6}", proc.parent_pid),
                            if is_selected { theme.selection_fg } else { theme.text_dim }
                        ),
                        SortColumn::User => {
                            // htop colors: root/SYSTEM = magenta, normal users = different colors
                            let user_color = if is_selected {
                                theme.selection_fg
                            } else if proc.user.eq_ignore_ascii_case("SYSTEM")
                                    || proc.user.eq_ignore_ascii_case("root")
                                    || proc.user.eq_ignore_ascii_case("LOCAL SERVICE")
                                    || proc.user.eq_ignore_ascii_case("NETWORK SERVICE") {
                                theme.process_priv  // Magenta for system/privileged users
                            } else {
                                theme.user_color
                            };
                            (format!("{:10}", truncate_str(&proc.user, 10)), user_color)
                        }
                        SortColumn::Priority => (
                            format!("{:>3}", proc.priority),
                            if is_selected { theme.selection_fg } else { theme.priority_color_for_nice(proc.nice) }
                        ),
                        SortColumn::Nice => (
                            format!("{:>3}", proc.nice),
                            if is_selected { theme.selection_fg } else { theme.priority_color_for_nice(proc.nice) }
                        ),
                        SortColumn::Threads => {
                            // htop: If nlwp == 1, use PROCESS_SHADOW (dimmed)
                            let color = if is_selected {
                                theme.selection_fg
                            } else if proc.thread_count == 1 {
                                theme.process_shadow
                            } else {
                                theme.threads_color
                            };
                            (format!("{:>3}", proc.thread_count), color)
                        }
                        SortColumn::Virt => {
                            // htop: Multi-colored memory values
                            let spans = format_bytes_colored(proc.virtual_mem, theme, is_selected);
                            return Cell::from(Line::from(spans));
                        }
                        SortColumn::Res => {
                            // htop: Multi-colored memory values
                            let spans = format_bytes_colored(proc.resident_mem, theme, is_selected);
                            return Cell::from(Line::from(spans));
                        }
                        SortColumn::Shr => {
                            // htop: Multi-colored memory values
                            let spans = format_bytes_colored(proc.shared_mem, theme, is_selected);
                            return Cell::from(Line::from(spans));
                        }
                        SortColumn::Status => {
                            // Show status char + leaf emoji for efficiency mode
                            // htop: Running processes are green and bold
                            let status_str = if proc.efficiency_mode {
                                format!("{}🌿", proc.status)  // e.g., "R🌿" for Running+Efficiency
                            } else {
                                format!("{}  ", proc.status)
                            };
                            (
                                status_str,
                                if is_selected { theme.selection_fg } else { theme.status_color(proc.status) }
                            )
                        }
                        SortColumn::Cpu => {
                            // htop: use gradient coloring, highlight >= 99.9%
                            let color = if is_selected {
                                theme.selection_fg
                            } else if proc.cpu_percent >= 99.9 {
                                theme.process_megabytes
                            } else {
                                theme.cpu_color(proc.cpu_percent)
                            };
                            (format!("{:>5.1}", proc.cpu_percent), color)
                        }
                        SortColumn::Mem => {
                            // htop: use threshold coloring, highlight >= 99.9%
                            let color = if is_selected {
                                theme.selection_fg
                            } else if proc.mem_percent >= 99.9 {
                                theme.process_megabytes
                            } else {
                                theme.mem_color(proc.mem_percent)
                            };
                            (format!("{:>5.1}", proc.mem_percent), color)
                        }
                        SortColumn::Time => {
                            // htop: Multi-colored time display
                            let spans = format_time_colored(proc.cpu_time, theme, is_selected);
                            return Cell::from(Line::from(spans));
                        }
                        SortColumn::StartTime => (
                            format!("{:>7}", format_start_time(proc.start_time, now_secs)),
                            if is_selected { theme.selection_fg } else { theme.text_dim }
                        ),
                        SortColumn::Command => unreachable!(), // Handled above
                        // Windows-specific columns (use theme colors)
                        SortColumn::Elevated => (
                            if proc.is_elevated { "🛡️".to_string() } else { " ".to_string() },
                            if is_selected { theme.selection_fg } else { theme.process_priv }  // Magenta for privileged
                        ),
                        SortColumn::Arch => (
                            format!("{:>4}", proc.arch.as_str()),
                            if is_selected { theme.selection_fg } else { theme.process_megabytes }  // Cyan for info
                        ),
                        SortColumn::Efficiency => (
                            if proc.efficiency_mode { "🌿".to_string() } else { " ".to_string() },
                            if is_selected { theme.selection_fg } else { theme.process_low_priority }  // Green for eco mode
                        ),
                    };
                    // Add bold modifier matching htop's A_BOLD usage:
                    // - High CPU (>50%) - bold for visibility
                    // - Running status ('R') - htop uses PROCESS_RUN_STATE
                    // - Disk wait/zombie ('D', 'Z') - htop uses A_BOLD | PROCESS_D_STATE
                    // - High priority (nice < 0) - htop uses PROCESS_HIGH_PRIORITY
                    // - Large memory (>1GB) - bold for visibility
                    let style = if *col == SortColumn::Cpu && proc.cpu_percent > 50.0 {
                        Style::default().fg(color).add_modifier(Modifier::BOLD)
                    } else if *col == SortColumn::Status && (proc.status == 'R' || proc.status == 'D' || proc.status == 'Z') {
                        // htop: Running is green, D/Z states are A_BOLD | Red
                        Style::default().fg(color).add_modifier(Modifier::BOLD)
                    } else if *col == SortColumn::Priority && proc.nice < 0 {
                        Style::default().fg(color).add_modifier(Modifier::BOLD)
                    } else if *col == SortColumn::Res && proc.resident_mem >= 1_073_741_824 {
                        // Bold for processes using > 1GB memory
                        Style::default().fg(color).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(color)
                    };
                    Cell::from(Span::styled(text, style))
                })
                .collect();

            // Row styling - always set background from theme
            // htop uses A_BOLD for selected and tagged processes
            let row_style = if is_selected {
                Style::default().bg(theme.selection_bg).add_modifier(Modifier::BOLD)
            } else if matches_search {
                Style::default().bg(theme.search_match)
            } else if is_tagged {
                // htop: PROCESS_TAG = A_BOLD | ColorPair(Yellow, Black)
                Style::default().fg(theme.process_tag).bg(theme.background).add_modifier(Modifier::BOLD)
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
/// Takes pre-computed `now` to avoid syscall per process
fn format_start_time(start_time: u64, now: u64) -> String {
    if start_time == 0 {
        return "-".to_string();
    }

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
