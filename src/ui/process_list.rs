use crate::terminal::{
    Block, Borders, Cell, Color, Constraint, Frame, Line, Modifier, Rect, Row, Span, Style, Table,
};

use crate::app::{App, SortColumn};
use crate::ui::colors::Theme;

/// Format CPU time with multi-colored output like htop's Row_printTime
/// Optimized: returns single span when colors are uniform (selected or !highlight_large_numbers)
#[inline]
fn format_time_colored<'a>(duration: std::time::Duration, theme: &Theme, is_selected: bool, highlight_large_numbers: bool) -> Vec<Span<'a>> {
    let total_secs = duration.as_secs();
    let centis = duration.subsec_millis() / 10;

    // Zero time - always show in shadow (use static str, no allocation)
    // Format: " 0:00.00" (8 chars, consistent with other formats)
    if total_secs == 0 && centis == 0 {
        let shadow = if is_selected { theme.selection_fg } else { theme.process_shadow };
        return vec![Span::styled(" 0:00.00", Style::default().fg(shadow))];
    }

    let total_mins = total_secs / 60;
    let total_hours = total_mins / 60;
    let total_days = total_hours / 24;
    let secs = total_secs % 60;
    let mins = total_mins % 60;
    let hours = total_hours % 24;

    // Fast path: uniform color (selected or no highlighting)
    let use_uniform = is_selected || !highlight_large_numbers;
    let base_color = if is_selected { theme.selection_fg } else { theme.process };

    // All formats produce 8 characters for consistent column width:
    // Minutes: " M:SS.cc" or "MM:SS.cc" (8 chars)
    // Hours:   " Hh MM:SS" or "HHhMM:SS" (8 chars)
    // Days:    "DDDd HHh" (8 chars)
    // Years:   "YYYy DDDd" (9 chars for large values, but rare)
    if use_uniform {
        // Single span - no multi-color needed
        let text = if total_mins < 60 {
            format!("{:2}:{:02}.{:02}", total_mins, secs, centis)
        } else if total_hours < 24 {
            format!("{:2}h{:02}:{:02}", total_hours, mins, secs)
        } else if total_days < 100 {
            format!("{:2}d {:02}h", total_days, hours)
        } else if total_days < 365 {
            format!("{:3}d{:02}h", total_days, hours)
        } else {
            let years = total_days / 365;
            let days = total_days % 365;
            format!("{:3}y{:03}d", years, days)
        };
        return vec![Span::styled(text, Style::default().fg(base_color))];
    }

    // Multi-color path (highlight_large_numbers enabled, not selected)
    let hour_color = theme.process_megabytes;
    let day_color = theme.process_gigabytes;
    let year_color = theme.large_number;

    if total_mins < 60 {
        vec![Span::styled(format!("{:2}:{:02}.{:02}", total_mins, secs, centis), Style::default().fg(base_color))]
    } else if total_hours < 24 {
        vec![
            Span::styled(format!("{:2}h", total_hours), Style::default().fg(hour_color)),
            Span::styled(format!("{:02}:{:02}", mins, secs), Style::default().fg(base_color)),
        ]
    } else if total_days < 100 {
        vec![
            Span::styled(format!("{:2}d ", total_days), Style::default().fg(day_color)),
            Span::styled(format!("{:02}h", hours), Style::default().fg(hour_color)),
        ]
    } else if total_days < 365 {
        vec![
            Span::styled(format!("{:3}d", total_days), Style::default().fg(day_color)),
            Span::styled(format!("{:02}h", hours), Style::default().fg(hour_color)),
        ]
    } else {
        let years = total_days / 365;
        let days = total_days % 365;
        vec![
            Span::styled(format!("{:3}y", years), Style::default().fg(year_color)),
            Span::styled(format!("{:03}d", days), Style::default().fg(day_color)),
        ]
    }
}

/// Format bytes with multi-colored output like htop's Row_printKBytes
/// Optimized: returns single span when colors are uniform (selected or !highlight_large_numbers)
#[inline]
fn format_bytes_colored<'a>(bytes: u64, theme: &Theme, is_selected: bool, highlight_large_numbers: bool) -> Vec<Span<'a>> {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    let base_color = if is_selected { theme.selection_fg } else { theme.process };

    // Fast path: uniform color (selected or no highlighting)
    if is_selected || !highlight_large_numbers {
        let text = if bytes >= TB {
            format!("{:.1}T", bytes as f64 / TB as f64)
        } else if bytes >= GB {
            format!("{:.1}G", bytes as f64 / GB as f64)
        } else if bytes >= MB {
            format!("{:.0}M", bytes as f64 / MB as f64)
        } else if bytes >= KB {
            format!("{:.0}K", bytes as f64 / KB as f64)
        } else {
            format!("{}B", bytes)
        };
        return vec![Span::styled(text, Style::default().fg(base_color))];
    }

    // Multi-color path (highlight_large_numbers enabled, not selected)
    let color_mb = theme.process_megabytes;
    let color_gb = theme.process_gigabytes;
    let color_tb = theme.large_number;

    if bytes >= TB {
        let tb = bytes as f64 / TB as f64;
        vec![Span::styled(format!("{:.1}T", tb), Style::default().fg(color_tb))]
    } else if bytes >= GB {
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
                Span::styled("G", Style::default().fg(color_mb)),  // Use static str
            ]
        }
    } else if bytes >= MB {
        let mb = bytes as f64 / MB as f64;
        if mb < 10.0 {
            let int_part = mb as u64;
            let dec_part = ((mb - int_part as f64) * 10.0) as u64;
            vec![
                Span::styled(format!("{}", int_part), Style::default().fg(color_mb)),
                Span::styled(format!(".{}M", dec_part), Style::default().fg(base_color)),
            ]
        } else {
            vec![Span::styled(format!("{:.0}M", mb), Style::default().fg(color_mb))]
        }
    } else if bytes >= KB {
        vec![Span::styled(format!("{:.0}K", bytes as f64 / KB as f64), Style::default().fg(base_color))]
    } else {
        vec![Span::styled(format!("{}B", bytes), Style::default().fg(base_color))]
    }
}

/// Check if path starts with a common Windows system path prefix
/// Returns the length of the prefix if found, or 0 if not a system path
/// Like htop's shadowDistPathPrefix feature for /usr/bin/, /lib/, etc.
/// Optimized: uses case-insensitive byte comparison without allocation
#[inline]
fn get_shadow_prefix_len(path: &str) -> usize {
    // Check common Windows system path prefixes (order: longer prefixes first)
    // Using byte-level case-insensitive comparison to avoid allocation
    const SHADOW_PREFIXES: &[&[u8]] = &[
        b"c:\\windows\\system32\\",
        b"c:\\windows\\syswow64\\",
        b"c:\\windows\\",
        b"c:\\program files (x86)\\",
        b"c:\\program files\\",
        b"c:\\programdata\\",
    ];

    let path_bytes = path.as_bytes();
    for prefix in SHADOW_PREFIXES {
        if path_bytes.len() >= prefix.len()
            && path_bytes[..prefix.len()].eq_ignore_ascii_case(prefix)
        {
            return prefix.len();
        }
    }
    0
}

/// Build adaptive column-width constraints. When the sum of natural column
/// widths exceeds the available area, each fixed-width (non-Command) column
/// is scaled down proportionally so every column stays visible — columns
/// shrink instead of getting cut off at the right edge.
///
/// Shared with `ui::mod::calculate_column_bounds` so the click-region x/width
/// bookkeeping matches what the Table widget actually renders.
pub fn adaptive_column_widths(columns: &[SortColumn], area_width: u16) -> Vec<Constraint> {
    const COLUMN_SPACING: u16 = 1;
    /// Minimum rendered width for any non-Command column — picked so values like
    /// small PIDs (`123`) and short user names (`root`) stay legible.
    const MIN_COL_WIDTH: u16 = 3;
    /// How much width to leave for the Command column when the rest has to
    /// shrink. Command is the most important field, so keep it readable.
    const COMMAND_RESERVE: u16 = 12;

    if columns.is_empty() {
        return Vec::new();
    }

    let spacing_total = COLUMN_SPACING * (columns.len().saturating_sub(1) as u16);
    let available = area_width.saturating_sub(spacing_total);

    // Natural sum of Length widths (excluding Command, which is Min/flexible).
    let fixed_natural: u16 = columns
        .iter()
        .filter(|c| !matches!(c, SortColumn::Command))
        .map(|c| c.width())
        .sum();

    let has_command = columns.iter().any(|c| matches!(c, SortColumn::Command));
    let command_reserve: u16 = if has_command { COMMAND_RESERVE } else { 0 };

    // Only shrink when fixed columns + command reserve don't fit.
    let needs_shrink = fixed_natural + command_reserve > available;

    if !needs_shrink {
        return columns
            .iter()
            .map(|col| {
                if matches!(col, SortColumn::Command) {
                    Constraint::Min(col.width())
                } else {
                    Constraint::Length(col.width())
                }
            })
            .collect();
    }

    // Compute scaled widths and track the rounding remainder so we can
    // redistribute it and avoid leaving a one-or-two-char sliver unused.
    let budget = available.saturating_sub(command_reserve);
    let mut scaled: Vec<u16> = columns
        .iter()
        .map(|col| {
            if matches!(col, SortColumn::Command) {
                0 // placeholder; Command uses Min
            } else if fixed_natural > 0 {
                ((col.width() as u32 * budget as u32 / fixed_natural as u32) as u16)
                    .max(MIN_COL_WIDTH)
            } else {
                MIN_COL_WIDTH
            }
        })
        .collect();

    // Redistribute integer-division remainder: every non-Command column gets
    // +1 until the remainder is exhausted. Prefers widening the leftmost cols.
    let assigned_fixed: u16 = scaled
        .iter()
        .zip(columns.iter())
        .filter(|(_, col)| !matches!(col, SortColumn::Command))
        .map(|(w, _)| *w)
        .sum();
    let mut leftover = budget.saturating_sub(assigned_fixed);
    for (w, col) in scaled.iter_mut().zip(columns.iter()) {
        if leftover == 0 {
            break;
        }
        if !matches!(col, SortColumn::Command) {
            *w += 1;
            leftover -= 1;
        }
    }

    columns
        .iter()
        .zip(scaled.iter())
        .map(|(col, &w)| {
            if matches!(col, SortColumn::Command) {
                Constraint::Min(COMMAND_RESERVE)
            } else {
                Constraint::Length(w)
            }
        })
        .collect()
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
    let header_cells: Vec<Cell> = visible_columns
        .iter()
        .map(|col| {
            let name = col.name();
            let indicator = if *col == app.sort_column {
                if app.sort_ascending { "▲" } else { "▼" }
            } else {
                ""
            };
            Cell::new(format!("{}{}", name, indicator))
        })
        .collect();

    let header = Row::new(header_cells).style(header_style).height(1);

    // Column widths for visible columns only. Use adaptive sizing so columns
    // shrink proportionally when the terminal is too narrow to fit them all at
    // natural width, instead of getting cut off.
    let widths: Vec<Constraint> = adaptive_column_widths(visible_columns, area.width);

    // Cache current time for start_time formatting (avoid syscall per process)
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Reusable format buffer to avoid per-cell String allocations.
    // write!() reuses the buffer's capacity across clear() calls.
    use std::fmt::Write;
    let mut fmt_buf = String::with_capacity(32);

    // Macro to format into the reusable buffer and return an owned String.
    // Avoids format!()'s internal allocation of a fresh String::new() per call.
    macro_rules! fmt {
        ($($arg:tt)*) => {{
            fmt_buf.clear();
            write!(fmt_buf, $($arg)*).unwrap();
            fmt_buf.clone()
        }};
    }

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

            // Tree prefix for tree view (avoid allocation when not in tree mode)
            let tree_prefix: std::borrow::Cow<str> = if app.tree_view {
                if proc.has_children {
                    if proc.is_collapsed { format!("{}[+]", proc.tree_prefix).into() }
                    else { format!("{}[-]", proc.tree_prefix).into() }
                } else { std::borrow::Cow::Borrowed(&proc.tree_prefix) }
            } else { std::borrow::Cow::Borrowed("") };

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
                        // Pre-allocate spans with typical capacity (tagged + elevated + arch + tree + path parts = ~8)
                        let mut spans: Vec<Span> = Vec::with_capacity(8);

                        // Tagged indicator - yellow dot prefix for visibility (static str)
                        if is_tagged {
                            spans.push(Span::styled(
                                "● ",
                                Style::default().fg(if is_selected { theme.selection_fg } else { theme.process_tag })
                                    .add_modifier(Modifier::BOLD)
                            ));
                        }

                        // Elevated indicator - use theme's privileged process color (static str)
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

                        // Tree prefix with tree color (avoid clone when empty)
                        if !tree_prefix.is_empty() {
                            spans.push(Span::styled(
                                tree_prefix.to_string(),
                                Style::default().fg(if is_selected { theme.selection_fg } else { theme.process_tree })
                            ));
                        }

                        // htop style command coloring:
                        // 1. Shadow common system path prefixes (grey) - like htop's shadowDistPathPrefix
                        // 2. If highlight_basename: path in PROCESS (white), basename in PROCESS_BASENAME (bold cyan)
                        // 3. If !highlight_basename: everything in PROCESS (white)
                        // 4. Bold red for updated/deleted executables (FAILED_READ) overrides above

                        // Check for shadow path prefix (C:\Windows\, C:\Program Files\, etc.)
                        let shadow_prefix_len = if app.config.show_program_path {
                            get_shadow_prefix_len(display_command)
                        } else {
                            0
                        };

                        // Find basename position (after last path separator)
                        let basename_start = display_command.rfind(['\\', '/'])
                            .map(|i| i + 1)
                            .unwrap_or(0);

                        // Determine colors based on state
                        let is_deleted_or_updated = proc.exe_updated || proc.exe_deleted;

                        if app.config.show_program_path && basename_start > 0 {
                            // Showing full path - split into parts
                            let path_end = basename_start;

                            // Part 1: Shadow prefix (if any) in grey
                            if shadow_prefix_len > 0 && shadow_prefix_len <= path_end {
                                // Use String::from to get owned value from slice
                                spans.push(Span::styled(
                                    String::from(&display_command[..shadow_prefix_len]),
                                    Style::default().fg(if is_selected { theme.selection_fg } else { theme.process_shadow })
                                ));
                                // Part 2: Rest of path (after shadow, before basename) in normal color
                                if shadow_prefix_len < path_end {
                                    spans.push(Span::styled(
                                        String::from(&display_command[shadow_prefix_len..path_end]),
                                        Style::default().fg(if is_selected { theme.selection_fg } else { theme.process })
                                    ));
                                }
                            } else {
                                // No shadow prefix, just path in normal color
                                spans.push(Span::styled(
                                    String::from(&display_command[..path_end]),
                                    Style::default().fg(if is_selected { theme.selection_fg } else { theme.process })
                                ));
                            }

                            // Part 3: Basename - color depends on state and highlight_basename setting
                            let (basename_color, basename_bold) = if is_selected {
                                (theme.selection_fg, false)
                            } else if is_deleted_or_updated {
                                (theme.failed_read, true)  // htop: FAILED_READ = A_BOLD | Red
                            } else if app.config.highlight_basename {
                                (theme.process_basename, true)  // htop: PROCESS_BASENAME = A_BOLD | Cyan
                            } else {
                                (theme.process, false)  // htop default: PROCESS = A_NORMAL
                            };

                            let basename_style = if basename_bold {
                                Style::default().fg(basename_color).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(basename_color)
                            };
                            spans.push(Span::styled(String::from(&display_command[basename_start..]), basename_style));
                        } else {
                            // Not showing path, or no path separator - show as single span
                            let (color, bold) = if is_selected {
                                (theme.selection_fg, false)
                            } else if is_deleted_or_updated {
                                (theme.failed_read, true)
                            } else if app.config.highlight_basename {
                                (theme.process_basename, true)
                            } else {
                                (theme.process, false)
                            };

                            let style = if bold {
                                Style::default().fg(color).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(color)
                            };
                            // Clone the display_command to avoid lifetime issues
                            spans.push(Span::styled(display_command.clone(), style));
                        }

                        return Cell::from(Line::from(spans));
                    }

                    let (text, color) = match col {
                        SortColumn::Pid => (
                            if is_selected { fmt!("▶{:>5}", proc.pid) } else { fmt!("{:>6}", proc.pid) },
                            if is_selected { theme.selection_fg } else { theme.pid_color }
                        ),
                        SortColumn::PPid => (
                            fmt!("{:>6}", proc.parent_pid),
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
                            (fmt!("{:10}", truncate_str(&proc.user, 10)), user_color)
                        }
                        SortColumn::Priority => (
                            fmt!("{:>3}", proc.priority),
                            if is_selected { theme.selection_fg } else { theme.process }  // htop uses default color
                        ),
                        SortColumn::PriorityClass => {
                            // Display Windows priority class name with color coding
                            use crate::app::WindowsPriorityClass;
                            let priority_class = WindowsPriorityClass::from_base_priority(proc.priority);
                            let color = if is_selected {
                                theme.selection_fg
                            } else {
                                match priority_class {
                                    WindowsPriorityClass::Realtime | WindowsPriorityClass::High => theme.process_high_priority,
                                    WindowsPriorityClass::Idle | WindowsPriorityClass::BelowNormal => theme.process_low_priority,
                                    _ => theme.process_shadow,
                                }
                            };
                            (fmt!("{:>6}", priority_class.short_name()), color)
                        }
                        SortColumn::Threads => {
                            // htop: If nlwp == 1, use PROCESS_SHADOW (dimmed)
                            let color = if is_selected {
                                theme.selection_fg
                            } else if proc.thread_count == 1 {
                                theme.process_shadow
                            } else {
                                theme.threads_color
                            };
                            (fmt!("{:>3}", proc.thread_count), color)
                        }
                        SortColumn::Virt => {
                            // htop: Multi-colored memory values (when highlight_large_numbers enabled)
                            let spans = format_bytes_colored(proc.virtual_mem, theme, is_selected, app.config.highlight_large_numbers);
                            return Cell::from(Line::from(spans));
                        }
                        SortColumn::Res => {
                            // htop: Multi-colored memory values (when highlight_large_numbers enabled)
                            let spans = format_bytes_colored(proc.resident_mem, theme, is_selected, app.config.highlight_large_numbers);
                            return Cell::from(Line::from(spans));
                        }
                        SortColumn::Shr => {
                            // htop: Multi-colored memory values (when highlight_large_numbers enabled)
                            let spans = format_bytes_colored(proc.shared_mem, theme, is_selected, app.config.highlight_large_numbers);
                            return Cell::from(Line::from(spans));
                        }
                        SortColumn::Status => {
                            // Show status char + leaf emoji for efficiency mode
                            // htop: Running processes are green and bold
                            let status_str = if proc.efficiency_mode {
                                fmt!("{}🌿", proc.status)  // e.g., "R🌿" for Running+Efficiency
                            } else {
                                fmt!("{}  ", proc.status)
                            };
                            (
                                status_str,
                                if is_selected { theme.selection_fg } else { theme.status_color(proc.status) }
                            )
                        }
                        SortColumn::Cpu => {
                            // htop Row_printPercentage: default color, >= 99.9% is cyan (when highlight_large_numbers)
                            let color = if is_selected {
                                theme.selection_fg
                            } else if app.config.highlight_large_numbers && proc.cpu_percent >= 99.9 {
                                theme.process_megabytes
                            } else {
                                theme.process  // htop uses default/white for normal values
                            };
                            (fmt!("{:>5.1}", proc.cpu_percent), color)
                        }
                        SortColumn::Mem => {
                            // htop Row_printPercentage: default color, >= 99.9% is cyan (when highlight_large_numbers)
                            let color = if is_selected {
                                theme.selection_fg
                            } else if app.config.highlight_large_numbers && proc.mem_percent >= 99.9 {
                                theme.process_megabytes
                            } else {
                                theme.process  // htop uses default/white for normal values
                            };
                            (fmt!("{:>5.1}", proc.mem_percent), color)
                        }
                        SortColumn::Time => {
                            // htop: Multi-colored time display (when highlight_large_numbers enabled)
                            let spans = format_time_colored(proc.cpu_time, theme, is_selected, app.config.highlight_large_numbers);
                            return Cell::from(Line::from(spans));
                        }
                        SortColumn::StartTime => {
                            let time_str = format_start_time(proc.start_time, now_secs);
                            (fmt!("{:>7}", time_str), if is_selected { theme.selection_fg } else { theme.process })
                        }
                        SortColumn::Command => unreachable!(), // Handled above
                        // Windows-specific columns (use theme colors, static &str to avoid allocation)
                        SortColumn::Elevated => {
                            let s: &str = if proc.is_elevated { "🛡️" } else { " " };
                            return Cell::from(Span::styled(s, Style::default().fg(
                                if is_selected { theme.selection_fg } else { theme.process_priv }
                            )));
                        }
                        SortColumn::Arch => (
                            fmt!("{:>4}", proc.arch.as_str()),
                            if is_selected { theme.selection_fg } else { theme.process_megabytes }  // Cyan for info
                        ),
                        SortColumn::Efficiency => {
                            let s: &str = if proc.efficiency_mode { "🌿" } else { " " };
                            return Cell::from(Span::styled(s, Style::default().fg(
                                if is_selected { theme.selection_fg } else { theme.process_low_priority }
                            )));
                        }
                        SortColumn::HandleCount => {
                            let color = if is_selected {
                                theme.selection_fg
                            } else if proc.handle_count == 0 {
                                theme.process_shadow
                            } else {
                                theme.process
                            };
                            (fmt!("{:>5}", proc.handle_count), color)
                        }
                        SortColumn::IoRate => {
                            let bytes = proc.io_read_rate + proc.io_write_rate;
                            if bytes == 0 {
                                let color = if is_selected { theme.selection_fg } else { theme.process_shadow };
                                return Cell::from(Span::styled(fmt!("{:>6}", 0), Style::default().fg(color)));
                            }
                            let spans = format_bytes_colored(bytes, theme, is_selected, app.config.highlight_large_numbers);
                            return Cell::from(Line::from(spans));
                        }
                        SortColumn::IoReadRate | SortColumn::IoWriteRate => {
                            let bytes = if *col == SortColumn::IoReadRate { proc.io_read_rate } else { proc.io_write_rate };
                            if bytes == 0 {
                                let color = if is_selected { theme.selection_fg } else { theme.process_shadow };
                                return Cell::from(Span::styled(fmt!("{:>6}", 0), Style::default().fg(color)));
                            }
                            let spans = format_bytes_colored(bytes, theme, is_selected, app.config.highlight_large_numbers);
                            return Cell::from(Line::from(spans));
                        }
                        SortColumn::IoRead | SortColumn::IoWrite => {
                            let bytes = if *col == SortColumn::IoRead { proc.io_read_bytes } else { proc.io_write_bytes };
                            if bytes == 0 {
                                let color = if is_selected { theme.selection_fg } else { theme.process_shadow };
                                return Cell::from(Span::styled(fmt!("{:>6}", 0), Style::default().fg(color)));
                            }
                            let spans = format_bytes_colored(bytes, theme, is_selected, app.config.highlight_large_numbers);
                            return Cell::from(Line::from(spans));
                        }
                    };
                    // Add bold modifier matching htop's A_BOLD usage:
                    // - High CPU (>50%) - bold for visibility
                    // - Running status ('R') - htop uses PROCESS_RUN_STATE
                    // - Disk wait/zombie ('D', 'Z') - htop uses A_BOLD | PROCESS_D_STATE
                    // - High priority (base priority > 8) - htop uses PROCESS_HIGH_PRIORITY
                    // - Large memory (>1GB) - bold for visibility
                    let style = if *col == SortColumn::Cpu && proc.cpu_percent > 50.0 {
                        Style::default().fg(color).add_modifier(Modifier::BOLD)
                    } else if *col == SortColumn::Status && (proc.status == 'R' || proc.status == 'D' || proc.status == 'Z') {
                        // htop: Running is green, D/Z states are A_BOLD | Red
                        Style::default().fg(color).add_modifier(Modifier::BOLD)
                    } else if *col == SortColumn::Priority && proc.priority > 8 {
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

            // Check if process is "new" (started within highlight_duration)
            // htop: PROCESS_NEW = ColorPair(Black, Green) - black text on green background
            let highlight_duration_secs = app.config.highlight_duration_ms / 1000;
            let is_new_process = app.config.highlight_new_processes
                && proc.start_time > 0
                && now_secs.saturating_sub(proc.start_time) < highlight_duration_secs;

            // Row styling - always set background from theme
            // htop uses A_BOLD for selected and tagged processes
            // Priority: selected > search match > tagged > new process > normal
            let row_style = if is_selected {
                Style::default().bg(theme.selection_bg).add_modifier(Modifier::BOLD)
            } else if matches_search {
                Style::default().bg(theme.search_match)
            } else if is_tagged {
                // htop: PROCESS_TAG = A_BOLD | ColorPair(Yellow, Black)
                Style::default().fg(theme.process_tag).bg(theme.background).add_modifier(Modifier::BOLD)
            } else if is_new_process {
                // htop: PROCESS_NEW = ColorPair(Black, Green) - black text on green bg
                Style::default().fg(Color::Black).bg(theme.new_process)
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

/// Truncate string to max display width, using Cow to avoid allocation when no truncation needed
#[inline]
fn truncate_str(s: &str, max_len: usize) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    use unicode_width::UnicodeWidthStr;

    let width = s.width();
    if width <= max_len {
        Cow::Borrowed(s)
    } else {
        // Safely truncate by characters, not bytes
        let mut result = String::with_capacity(max_len + 3); // +3 for ellipsis
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
        Cow::Owned(result)
    }
}

/// Format a Unix timestamp as elapsed time or time of day
/// Takes pre-computed `now` to avoid syscall per process
/// Returns Cow to avoid allocation for static "-" case
#[inline]
fn format_start_time(start_time: u64, now: u64) -> std::borrow::Cow<'static, str> {
    use std::borrow::Cow;

    if start_time == 0 || start_time > now {
        return Cow::Borrowed("-");
    }

    let elapsed_secs = now - start_time;

    // If started today, show as HH:MM
    // If started more than a day ago, show as days
    Cow::Owned(if elapsed_secs < 60 {
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
    })
}
