use crate::terminal::{
    Constraint, Frame, Modifier, Rect, RowSeg, RowSpec, Span, Style, resolve_column_widths,
};

use crate::app::{App, SortColumn};
use crate::system::ProcessArch;
use crate::ui::colors::Theme;

use super::text_pool::{pooled_fmt, pooled_string, recycle_spans, recycle_vec};

/// Format into a pooled String (recycled after its row is painted).
macro_rules! pfmt {
    ($($arg:tt)*) => {{ pooled_fmt(format_args!($($arg)*)) }};
}

/// Float cells: exact integer-tenths formatting straight into a pooled
/// string (replaces format!'s float machinery on the hottest cells).
macro_rules! fmt_tenths {
    ($val:expr, $width:expr) => {{
        let mut s = pooled_string();
        crate::numfmt::tenths_into(&mut s, $val, $width);
        s
    }};
}

/// Scaled byte cells with a unit suffix, same pooled-string treatment.
macro_rules! fmt_scaled_bytes {
    ($bytes:expr, $pow:expr, $unit:literal) => {{
        let mut s = pooled_string();
        crate::numfmt::scaled_bytes_into(&mut s, $bytes, $pow);
        s.push_str($unit);
        s
    }};
}

/// Push CPU time spans, multi-colored like htop's Row_printTime (one span
/// when colors are uniform: selected or !highlight_large_numbers).
#[inline]
fn push_time_colored(
    out: &mut Vec<Span<'_>>,
    duration: std::time::Duration,
    theme: &Theme,
    is_selected: bool,
    highlight_large_numbers: bool,
) {
    let total_secs = duration.as_secs();
    let centis = duration.subsec_millis() / 10;

    // Zero time - always show in shadow (use static str, no allocation)
    // Format: " 0:00.00" (8 chars, consistent with other formats)
    if total_secs == 0 && centis == 0 {
        let shadow = if is_selected {
            theme.selection_fg
        } else {
            theme.process_shadow
        };
        out.push(Span::styled(" 0:00.00", Style::default().fg(shadow)));
        return;
    }

    let total_mins = total_secs / 60;
    let total_hours = total_mins / 60;
    let total_days = total_hours / 24;
    let secs = total_secs % 60;
    let mins = total_mins % 60;
    let hours = total_hours % 24;

    // Fast path: uniform color (selected or no highlighting)
    let use_uniform = is_selected || !highlight_large_numbers;
    let base_color = if is_selected {
        theme.selection_fg
    } else {
        theme.process
    };

    // Every scale renders at exactly 8 characters so the column stays aligned
    // no matter which scale a value falls into:
    // Minutes: " M:SS.cc"      -> "{:2}:{:02}.{:02}"
    // Hours:   "HHhMM:SS"      -> "{:2}h{:02}:{:02}"
    // Days:    "DDDd HHh"      -> "{:3}d {:02}h"
    // Years:   "YYYyDDDDd"     -> "{:3}y{:03}d"
    // (Only absurd runtimes - 1000+ years of CPU time - can exceed this.)
    if use_uniform {
        // Single span - no multi-color needed
        let text = if total_mins < 60 {
            pfmt!("{:2}:{:02}.{:02}", total_mins, secs, centis)
        } else if total_hours < 24 {
            pfmt!("{:2}h{:02}:{:02}", total_hours, mins, secs)
        } else if total_days < 365 {
            pfmt!("{:3}d {:02}h", total_days, hours)
        } else {
            let years = total_days / 365;
            let days = total_days % 365;
            pfmt!("{:3}y{:03}d", years, days)
        };
        out.push(Span::styled(text, Style::default().fg(base_color)));
        return;
    }

    // Multi-color path (highlight_large_numbers enabled, not selected)
    let hour_color = theme.process_megabytes;
    let day_color = theme.process_gigabytes;
    let year_color = theme.large_number;

    if total_mins < 60 {
        out.push(Span::styled(
            pfmt!("{:2}:{:02}.{:02}", total_mins, secs, centis),
            Style::default().fg(base_color),
        ));
    } else if total_hours < 24 {
        out.push(Span::styled(
            pfmt!("{:2}h", total_hours),
            Style::default().fg(hour_color),
        ));
        out.push(Span::styled(
            pfmt!("{:02}:{:02}", mins, secs),
            Style::default().fg(base_color),
        ));
    } else if total_days < 365 {
        // "{:3}d " (5) + "{:02}h" (3) = same 8-char width as the other scales
        out.push(Span::styled(
            pfmt!("{:3}d ", total_days),
            Style::default().fg(day_color),
        ));
        out.push(Span::styled(
            pfmt!("{:02}h", hours),
            Style::default().fg(hour_color),
        ));
    } else {
        let years = total_days / 365;
        let days = total_days % 365;
        out.push(Span::styled(
            pfmt!("{:3}y", years),
            Style::default().fg(year_color),
        ));
        out.push(Span::styled(
            pfmt!("{:03}d", days),
            Style::default().fg(day_color),
        ));
    }
}

/// Push byte-count spans, multi-colored like htop's Row_printKBytes (one
/// span when colors are uniform: selected or !highlight_large_numbers).
#[inline]
fn push_bytes_colored(
    out: &mut Vec<Span<'_>>,
    bytes: u64,
    theme: &Theme,
    is_selected: bool,
    highlight_large_numbers: bool,
) {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    let base_color = if is_selected {
        theme.selection_fg
    } else {
        theme.process
    };

    // Fast path: uniform color (selected or no highlighting)
    if is_selected || !highlight_large_numbers {
        let text = if bytes >= TB {
            fmt_scaled_bytes!(bytes, 40, "T")
        } else if bytes >= GB {
            fmt_scaled_bytes!(bytes, 30, "G")
        } else if bytes >= MB {
            fmt_scaled_bytes!(bytes, 20, "M")
        } else if bytes >= KB {
            fmt_scaled_bytes!(bytes, 10, "K")
        } else {
            pfmt!("{}B", bytes)
        };
        out.push(Span::styled(text, Style::default().fg(base_color)));
        return;
    }

    // Multi-color path (highlight_large_numbers enabled, not selected)
    let color_mb = theme.process_megabytes;
    let color_gb = theme.process_gigabytes;
    let color_tb = theme.large_number;

    if bytes >= TB {
        out.push(Span::styled(
            pfmt!("{}T", crate::numfmt::scaled_bytes(bytes, 40)),
            Style::default().fg(color_tb),
        ));
    } else if bytes >= GB {
        // floor(bytes*10 / 2^30) reproduces the old float truncation exactly
        let tenths_total = (u128::from(bytes) * 10 / (1u128 << 30)) as u64;
        if tenths_total < 100 {
            let (int_part, dec_part) = (tenths_total / 10, tenths_total % 10);
            out.push(Span::styled(
                pfmt!("{}", int_part),
                Style::default().fg(color_gb),
            ));
            out.push(Span::styled(
                pfmt!(".{}G", dec_part),
                Style::default().fg(color_mb),
            ));
        } else {
            out.push(Span::styled(
                pfmt!("{}", crate::numfmt::scaled_bytes_round0(bytes, 30)),
                Style::default().fg(color_gb),
            ));
            out.push(Span::styled("G", Style::default().fg(color_mb))); // Use static str
        }
    } else if bytes >= MB {
        let tenths_total = (u128::from(bytes) * 10 / (1u128 << 20)) as u64;
        if tenths_total < 100 {
            let (int_part, dec_part) = (tenths_total / 10, tenths_total % 10);
            out.push(Span::styled(
                pfmt!("{}", int_part),
                Style::default().fg(color_mb),
            ));
            out.push(Span::styled(
                pfmt!(".{}M", dec_part),
                Style::default().fg(base_color),
            ));
        } else {
            out.push(Span::styled(
                pfmt!("{}M", crate::numfmt::scaled_bytes_round0(bytes, 20)),
                Style::default().fg(color_mb),
            ));
        }
    } else if bytes >= KB {
        out.push(Span::styled(
            pfmt!("{}K", crate::numfmt::scaled_bytes_round0(bytes, 10)),
            Style::default().fg(base_color),
        ));
    } else {
        out.push(Span::styled(
            pfmt!("{}B", bytes),
            Style::default().fg(base_color),
        ));
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
/// Resolved once per frame by `ui::draw` and shared with both the click-region
/// bookkeeping (`ui::mod::calculate_column_bounds`) and the Table widget below,
/// so both agree on column x/width by construction.
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

/// Draw the process table. Column widths are resolved once per frame by the
/// caller (`ui::draw`) and shared with the click-region bookkeeping there, so
/// the adaptive sizing math runs once instead of twice with identical inputs.
pub fn draw<'a>(frame: &mut Frame, app: &'a App, area: Rect, column_widths: &[Constraint]) {
    let theme = &app.theme;

    // Use cached visible columns (updated when config changes)
    let visible_columns = &app.cached_visible_columns;
    if area.is_empty() {
        return;
    }

    // htop header style: black text on green background
    let header_style = Style::default()
        .fg(theme.header_fg)
        .bg(theme.header_bg)
        .add_modifier(Modifier::BOLD);

    // Reuse the widths Layout::split already resolved for this frame's click
    // regions (ui::draw -> app.ui_bounds.columns) so paint and hit-testing
    // cannot drift apart; otherwise resolve the constraints `ui::draw` passed.
    let mut widths: Vec<u16> = if app.ui_bounds.columns.len() == visible_columns.len() {
        app.ui_bounds.columns.iter().map(|b| b.width).collect()
    } else {
        Vec::new()
    };
    if widths.is_empty() {
        widths = resolve_column_widths(column_widths, COLUMN_SPACING, area.width);
    }

    // Each row's spans go into one buffer (cell boundaries in `cell_ends`)
    // and the row is painted as soon as it is built, so rows need no Row,
    // Cell or Line of their own: the buffers are reused for every row.
    let mut spans: Vec<Span<'a>> = Vec::with_capacity(visible_columns.len() * 2 + 8);
    let mut cell_ends: Vec<usize> = Vec::with_capacity(visible_columns.len());
    let mut segs: Vec<RowSeg<'static>> = Vec::with_capacity(visible_columns.len());

    // Header with sort indicator: two borrowed spans per column.
    for col in visible_columns {
        let indicator = if *col == app.sort_column {
            if app.sort_ascending { "▲" } else { "▼" }
        } else {
            ""
        };
        spans.push(Span::raw(col.name()));
        spans.push(Span::raw(indicator));
        cell_ends.push(spans.len());
    }
    paint_table_row(
        frame,
        area,
        area.y,
        header_style,
        &widths,
        &spans,
        &cell_ends,
        &mut segs,
    );
    spans.clear();
    cell_ends.clear();

    // Cache current time for start_time formatting (avoid syscall per process)
    let now_secs = app.now_unix_secs();

    // Cell strings come from the string pool (`text_pool`) and go back to it
    // once their row is painted.
    macro_rules! fmt {
        ($($arg:tt)*) => {{ pfmt!($($arg)*) }};
    }

    let mut y = area.y.saturating_add(1);
    for (offset, (row, proc)) in app
        .display_rows_from(app.scroll_offset)
        .take(app.visible_height)
        .enumerate()
    {
        if y >= area.bottom() {
            break;
        }
        let is_selected = app.scroll_offset + offset == app.selected_index;
        let is_tagged = app.tagged_pids.contains(&proc.identity());
        let matches_search = row.matches_search;

        // Tree prefix for tree view (zero-allocation: borrows the stored
        // prefix; the collapsed/expanded marker rides as a second static
        // span instead of a format!-allocated "[+]/[-]" suffix).
        let tree_prefix: &str = if app.tree_view { &row.tree_prefix } else { "" };
        // Static collapse marker for parents in tree view (same tree color
        // as the prefix, so rendering is identical to one merged span).
        let tree_marker: &str = if app.tree_view && row.has_children {
            if row.is_collapsed { "[+]" } else { "[-]" }
        } else {
            ""
        };

        // Choose between full command path or just the program name.
        // Typed as &str so its slices can be borrowed directly into Spans
        // (zero-allocation) instead of cloned — valid because `proc` is
        // borrowed from app.processes for the whole draw.
        let display_command: &str = if app.config.show_program_path {
            &proc.command
        } else {
            &proc.name
        };

        // Push one column's spans for this process.
        let push_cell = |col: &SortColumn, out: &mut Vec<Span<'a>>| {
            // Command column uses multi-span for colored indicators (htop style)
            if *col == SortColumn::Command {
                // Tagged indicator - yellow dot prefix for visibility (static str)
                if is_tagged {
                    out.push(Span::styled(
                        "● ",
                        Style::default()
                            .fg(if is_selected {
                                theme.selection_fg
                            } else {
                                theme.process_tag
                            })
                            .add_modifier(Modifier::BOLD),
                    ));
                }

                // Elevated indicator - use theme's privileged process color (static str)
                if proc.is_elevated {
                    out.push(Span::styled(
                        "🛡️ ",
                        Style::default().fg(if is_selected {
                            theme.selection_fg
                        } else {
                            theme.process_priv
                        }),
                    ));
                }

                // Architecture indicator - use theme's megabytes color (cyan in default)
                // Static tags: the only non-native arches are x86/x64/ARM,
                // so no format! allocation is needed.
                let arch_tag: &str = match proc.arch {
                    ProcessArch::Native => "",
                    ProcessArch::X86 => "[x86] ",
                    ProcessArch::X64 => "[x64] ",
                    ProcessArch::ARM64 => "[ARM] ",
                };
                if !arch_tag.is_empty() {
                    out.push(Span::styled(
                        arch_tag,
                        Style::default().fg(if is_selected {
                            theme.selection_fg
                        } else {
                            theme.process_megabytes
                        }),
                    ));
                }

                // Tree prefix with tree color (both parts borrowed)
                if !tree_prefix.is_empty() || !tree_marker.is_empty() {
                    let tree_style = Style::default().fg(if is_selected {
                        theme.selection_fg
                    } else {
                        theme.process_tree
                    });
                    if !tree_prefix.is_empty() {
                        out.push(Span::styled(tree_prefix, tree_style));
                    }
                    if !tree_marker.is_empty() {
                        out.push(Span::styled(tree_marker, tree_style));
                    }
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
                let basename_start = display_command
                    .rfind(['\\', '/'])
                    .map(|i| i + 1)
                    .unwrap_or(0);

                // Determine colors based on state
                let is_deleted_or_updated = proc.exe_updated || proc.exe_deleted;

                if app.config.show_program_path && basename_start > 0 {
                    // Showing full path - split into parts
                    let path_end = basename_start;

                    // Part 1: Shadow prefix (if any) in grey
                    if shadow_prefix_len > 0 && shadow_prefix_len <= path_end {
                        // Borrow the path slices directly (no allocation)
                        out.push(Span::styled(
                            &display_command[..shadow_prefix_len],
                            Style::default().fg(if is_selected {
                                theme.selection_fg
                            } else {
                                theme.process_shadow
                            }),
                        ));
                        // Part 2: Rest of path (after shadow, before basename) in normal color
                        if shadow_prefix_len < path_end {
                            out.push(Span::styled(
                                &display_command[shadow_prefix_len..path_end],
                                Style::default().fg(if is_selected {
                                    theme.selection_fg
                                } else {
                                    theme.process
                                }),
                            ));
                        }
                    } else {
                        // No shadow prefix, just path in normal color
                        out.push(Span::styled(
                            &display_command[..path_end],
                            Style::default().fg(if is_selected {
                                theme.selection_fg
                            } else {
                                theme.process
                            }),
                        ));
                    }

                    // Part 3: Basename - color depends on state and highlight_basename setting
                    let (basename_color, basename_bold) = if is_selected {
                        (theme.selection_fg, false)
                    } else if is_deleted_or_updated {
                        (theme.failed_read, true) // htop: FAILED_READ = A_BOLD | Red
                    } else if app.config.highlight_basename {
                        (theme.process_basename, true) // htop: PROCESS_BASENAME = A_BOLD | Cyan
                    } else {
                        (theme.process, false) // htop default: PROCESS = A_NORMAL
                    };

                    let basename_style = if basename_bold {
                        Style::default()
                            .fg(basename_color)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(basename_color)
                    };
                    out.push(Span::styled(
                        &display_command[basename_start..],
                        basename_style,
                    ));
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
                    out.push(Span::styled(display_command, style));
                }

                return;
            }

            let (text, color) = match col {
                SortColumn::Pid => (
                    if is_selected {
                        fmt!("▶{:>5}", proc.pid)
                    } else {
                        fmt!("{:>6}", proc.pid)
                    },
                    if is_selected {
                        theme.selection_fg
                    } else {
                        theme.pid_color
                    },
                ),
                SortColumn::PPid => (
                    fmt!("{:>6}", proc.parent_pid),
                    if is_selected {
                        theme.selection_fg
                    } else {
                        theme.text_dim
                    },
                ),
                SortColumn::User => {
                    // htop colors: root/SYSTEM = magenta, normal users = different colors
                    let user_color = if is_selected {
                        theme.selection_fg
                    } else if proc.user.eq_ignore_ascii_case("SYSTEM")
                        || proc.user.eq_ignore_ascii_case("root")
                        || proc.user.eq_ignore_ascii_case("LOCAL SERVICE")
                        || proc.user.eq_ignore_ascii_case("NETWORK SERVICE")
                    {
                        theme.process_priv // Magenta for system/privileged users
                    } else {
                        theme.user_color
                    };
                    (fmt!("{:10}", truncate_str(&proc.user, 10)), user_color)
                }
                SortColumn::Priority => (
                    fmt!("{:>3}", proc.priority),
                    if is_selected {
                        theme.selection_fg
                    } else {
                        theme.process
                    }, // htop uses default color
                ),
                SortColumn::PriorityClass => {
                    // Display Windows priority class name with color coding
                    use crate::app::WindowsPriorityClass;
                    let priority_class = WindowsPriorityClass::from_base_priority(proc.priority);
                    let color = if is_selected {
                        theme.selection_fg
                    } else {
                        match priority_class {
                            WindowsPriorityClass::Realtime | WindowsPriorityClass::High => {
                                theme.process_high_priority
                            }
                            WindowsPriorityClass::Idle | WindowsPriorityClass::BelowNormal => {
                                theme.process_low_priority
                            }
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
                    push_bytes_colored(
                        out,
                        proc.virtual_mem,
                        theme,
                        is_selected,
                        app.config.highlight_large_numbers,
                    );
                    return;
                }
                SortColumn::Res => {
                    // htop: Multi-colored memory values (when highlight_large_numbers enabled)
                    push_bytes_colored(
                        out,
                        proc.resident_mem,
                        theme,
                        is_selected,
                        app.config.highlight_large_numbers,
                    );
                    return;
                }
                SortColumn::Shr => {
                    // htop: Multi-colored memory values (when highlight_large_numbers enabled)
                    push_bytes_colored(
                        out,
                        proc.shared_mem,
                        theme,
                        is_selected,
                        app.config.highlight_large_numbers,
                    );
                    return;
                }
                SortColumn::Status => {
                    // Show status char + leaf emoji for efficiency mode
                    // htop: Running processes are green and bold
                    let status_str = if proc.efficiency_mode {
                        fmt!("{}🌿", proc.status_char()) // e.g., "R🌿" for Running+Efficiency
                    } else {
                        fmt!("{}  ", proc.status_char())
                    };
                    (
                        status_str,
                        if is_selected {
                            theme.selection_fg
                        } else {
                            theme.status_color(proc.status_char())
                        },
                    )
                }
                SortColumn::Cpu => {
                    // htop Row_printPercentage: default color, >= 99.9% is cyan (when highlight_large_numbers)
                    let color = if is_selected {
                        theme.selection_fg
                    } else if app.config.highlight_large_numbers && proc.cpu_percent >= 99.9 {
                        theme.process_megabytes
                    } else {
                        theme.process // htop uses default/white for normal values
                    };
                    (fmt_tenths!(proc.cpu_percent, 5), color)
                }
                SortColumn::Mem => {
                    // htop Row_printPercentage: default color, >= 99.9% is cyan (when highlight_large_numbers)
                    let color = if is_selected {
                        theme.selection_fg
                    } else if app.config.highlight_large_numbers && proc.mem_percent >= 99.9 {
                        theme.process_megabytes
                    } else {
                        theme.process // htop uses default/white for normal values
                    };
                    (fmt_tenths!(proc.mem_percent, 5), color)
                }
                SortColumn::Time => {
                    // htop: Multi-colored time display (when highlight_large_numbers enabled)
                    push_time_colored(
                        out,
                        std::time::Duration::from_nanos(proc.cpu_time * 100),
                        theme,
                        is_selected,
                        app.config.highlight_large_numbers,
                    );
                    return;
                }
                SortColumn::StartTime => {
                    let time_str = format_start_time(u64::from(proc.start_time), now_secs);
                    (
                        fmt!("{:>7}", time_str),
                        if is_selected {
                            theme.selection_fg
                        } else {
                            theme.process
                        },
                    )
                }
                SortColumn::Command => unreachable!(), // Handled above
                // Windows-specific columns (use theme colors, static &str to avoid allocation)
                SortColumn::Elevated => {
                    let s: &str = if proc.is_elevated { "🛡️" } else { " " };
                    out.push(Span::styled(
                        s,
                        Style::default().fg(if is_selected {
                            theme.selection_fg
                        } else {
                            theme.process_priv
                        }),
                    ));
                    return;
                }
                SortColumn::Arch => (
                    fmt!("{:>4}", proc.arch.as_str()),
                    if is_selected {
                        theme.selection_fg
                    } else {
                        theme.process_megabytes
                    }, // Cyan for info
                ),
                SortColumn::Efficiency => {
                    let s: &str = if proc.efficiency_mode { "🌿" } else { " " };
                    out.push(Span::styled(
                        s,
                        Style::default().fg(if is_selected {
                            theme.selection_fg
                        } else {
                            theme.process_low_priority
                        }),
                    ));
                    return;
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
                        let color = if is_selected {
                            theme.selection_fg
                        } else {
                            theme.process_shadow
                        };
                        out.push(Span::styled(fmt!("{:>6}", 0), Style::default().fg(color)));
                        return;
                    }
                    push_bytes_colored(
                        out,
                        bytes,
                        theme,
                        is_selected,
                        app.config.highlight_large_numbers,
                    );
                    return;
                }
                SortColumn::IoReadRate | SortColumn::IoWriteRate => {
                    let bytes = if *col == SortColumn::IoReadRate {
                        proc.io_read_rate
                    } else {
                        proc.io_write_rate
                    };
                    if bytes == 0 {
                        let color = if is_selected {
                            theme.selection_fg
                        } else {
                            theme.process_shadow
                        };
                        out.push(Span::styled(fmt!("{:>6}", 0), Style::default().fg(color)));
                        return;
                    }
                    push_bytes_colored(
                        out,
                        bytes,
                        theme,
                        is_selected,
                        app.config.highlight_large_numbers,
                    );
                    return;
                }
                SortColumn::IoRead | SortColumn::IoWrite => {
                    let bytes = if *col == SortColumn::IoRead {
                        proc.io_read_bytes
                    } else {
                        proc.io_write_bytes
                    };
                    if bytes == 0 {
                        let color = if is_selected {
                            theme.selection_fg
                        } else {
                            theme.process_shadow
                        };
                        out.push(Span::styled(fmt!("{:>6}", 0), Style::default().fg(color)));
                        return;
                    }
                    push_bytes_colored(
                        out,
                        bytes,
                        theme,
                        is_selected,
                        app.config.highlight_large_numbers,
                    );
                    return;
                }
                SortColumn::Gpu => {
                    // htop Row_printPercentage style; idle (0.0) is dimmed like I/O
                    let color = if is_selected {
                        theme.selection_fg
                    } else if proc.gpu_percent < 0.05 {
                        theme.process_shadow
                    } else if app.config.highlight_large_numbers && proc.gpu_percent >= 99.9 {
                        theme.process_megabytes
                    } else {
                        theme.process
                    };
                    (fmt_tenths!(proc.gpu_percent, 5), color)
                }
                SortColumn::GpuMem => {
                    if proc.gpu_memory == 0 {
                        let color = if is_selected {
                            theme.selection_fg
                        } else {
                            theme.process_shadow
                        };
                        out.push(Span::styled(fmt!("{:>7}", 0), Style::default().fg(color)));
                        return;
                    }
                    push_bytes_colored(
                        out,
                        proc.gpu_memory,
                        theme,
                        is_selected,
                        app.config.highlight_large_numbers,
                    );
                    return;
                }
                SortColumn::Npu => {
                    // htop Row_printPercentage style; idle (0.0) is dimmed like I/O
                    let color = if is_selected {
                        theme.selection_fg
                    } else if proc.npu_percent < 0.05 {
                        theme.process_shadow
                    } else if app.config.highlight_large_numbers && proc.npu_percent >= 99.9 {
                        theme.process_megabytes
                    } else {
                        theme.process
                    };
                    (fmt_tenths!(proc.npu_percent, 5), color)
                }
                SortColumn::NpuMem => {
                    if proc.npu_memory == 0 {
                        let color = if is_selected {
                            theme.selection_fg
                        } else {
                            theme.process_shadow
                        };
                        out.push(Span::styled(fmt!("{:>7}", 0), Style::default().fg(color)));
                        return;
                    }
                    push_bytes_colored(
                        out,
                        proc.npu_memory,
                        theme,
                        is_selected,
                        app.config.highlight_large_numbers,
                    );
                    return;
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
            } else if *col == SortColumn::Status
                && (proc.status == b'R' || proc.status == b'D' || proc.status == b'Z')
            {
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
            out.push(Span::styled(text, style));
        };
        for col in visible_columns {
            push_cell(col, &mut spans);
            cell_ends.push(spans.len());
        }

        // Check if process is "new" (started within highlight_duration)
        // htop: PROCESS_NEW = ColorPair(Black, Green) - black text on green background
        let highlight_duration_secs = app.config.highlight_duration_ms / 1000;
        let is_new_process = app.config.highlight_new_processes
            && proc.start_time > 0
            && now_secs.saturating_sub(u64::from(proc.start_time)) < highlight_duration_secs;

        // Row styling - always set background from theme
        // htop uses A_BOLD for selected and tagged processes
        // Priority: selected > search match > tagged > new process > normal
        let row_style = if is_selected {
            Style::default()
                .bg(theme.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else if matches_search {
            Style::default().bg(theme.search_match)
        } else if is_tagged {
            // htop: PROCESS_TAG = A_BOLD | ColorPair(Yellow, Black)
            Style::default()
                .fg(theme.process_tag)
                .bg(theme.background)
                .add_modifier(Modifier::BOLD)
        } else if is_new_process {
            // htop: PROCESS_NEW = ColorPair(Black, Green). Only the green
            // background is applied here: every cell span carries its own
            // per-column foreground, which overrides any row-level fg.
            Style::default().bg(theme.new_process)
        } else {
            Style::default().bg(theme.background)
        };

        paint_table_row(
            frame, area, y, row_style, &widths, &spans, &cell_ends, &mut segs,
        );
        recycle_spans(&mut spans);
        cell_ends.clear();
        y = y.saturating_add(1);
    }
}

/// Gap between table columns.
const COLUMN_SPACING: u16 = 1;

/// Paint one table row (a plain, borderless table's header or data row):
/// cell `i` holds `spans[cell_ends[i - 1]..cell_ends[i]]` and sits at its
/// column's offset. `segs` lends its allocation to every row.
#[allow(clippy::too_many_arguments)] // one row's geometry, content and scratch
fn paint_table_row<'a>(
    frame: &mut Frame,
    area: Rect,
    y: u16,
    style: Style,
    widths: &[u16],
    spans: &'a [Span<'a>],
    cell_ends: &[usize],
    segs: &mut Vec<RowSeg<'static>>,
) {
    let mut row_segs: Vec<RowSeg<'a>> = recycle_vec(std::mem::take(segs));
    let mut x = area.x;
    let mut start = 0;
    for (&end, &width) in cell_ends.iter().zip(widths) {
        row_segs.push(RowSeg {
            x,
            width,
            style: Style::default(),
            line_style: Style::default(),
            spans: &spans[start..end],
        });
        start = end;
        x = x.saturating_add(width).saturating_add(COLUMN_SPACING);
    }
    frame.paint_row(
        y,
        &RowSpec {
            x: area.x,
            width: area.width,
            base: Style::default().patch(style),
            segs: &row_segs,
        },
    );
    *segs = recycle_vec(row_segs);
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
        pfmt!("{}s", elapsed_secs)
    } else if elapsed_secs < 3600 {
        pfmt!("{}m", elapsed_secs / 60)
    } else if elapsed_secs < 86400 {
        pfmt!("{}h{}m", elapsed_secs / 3600, (elapsed_secs % 3600) / 60)
    } else {
        let days = elapsed_secs / 86400;
        if days > 99 {
            pfmt!("{}d", days)
        } else {
            pfmt!("{}d{}h", days, (elapsed_secs % 86400) / 3600)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Concatenate the text of every span, so multi-color layouts can be
    /// compared against the single-span uniform layout.
    fn format_time_colored<'a>(
        duration: std::time::Duration,
        theme: &Theme,
        is_selected: bool,
        highlight_large_numbers: bool,
    ) -> Vec<Span<'a>> {
        let mut out = Vec::new();
        push_time_colored(
            &mut out,
            duration,
            theme,
            is_selected,
            highlight_large_numbers,
        );
        out
    }

    fn joined_spans<'a>(spans: &[Span<'a>]) -> String {
        spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn time_column_fixed_width_across_all_scales() {
        let theme = Theme::default_theme();
        // Zero, seconds, minutes, hours, day and year scales - including the
        // boundaries between them - must all render at the same display width.
        let samples: &[u64] = &[
            0,
            1,
            59,
            60,             // 1 minute
            3_599,          // 59:59
            3_600,          // 1 hour
            86_399,         // 23h59:59
            86_400,         // 1 day
            432_000,        // 5 days
            8_553_600,      // 99 days
            8_553_601,      // 99d 00h +1s boundary
            17_280_000,     // 200 days
            31_535_999,     // 364d 23h 59m 59s
            31_536_000,     // 365 days -> year scale
            34_560_000,     // 400 days
            7_889_400_000,  // ~250 years, multi-digit year
        ];
        for &secs in samples {
            let d = std::time::Duration::from_secs(secs);
            for is_selected in [false, true] {
                for highlight in [false, true] {
                    let uniform = format_time_colored(d, &theme, is_selected, false);
                    let colored = format_time_colored(d, &theme, is_selected, highlight);
                    for spans in [&uniform, &colored] {
                        let text = joined_spans(spans);
                        assert_eq!(
                            text.chars().count(),
                            8,
                            "width mismatch for {secs}s (selected={is_selected}, \
                             highlight={highlight}): {text:?}"
                        );
                    }
                    // Uniform and colored layouts must agree on the text so
                    // toggling highlighting never reshuffles the column.
                    assert_eq!(joined_spans(&uniform), joined_spans(&colored));
                }
            }
        }
    }

    #[test]
    fn time_column_exact_strings_per_scale() {
        let theme = Theme::default_theme();
        let d = |secs: u64| std::time::Duration::from_secs(secs);

        // Seconds/minutes scale (" M:SS.cc")
        assert_eq!(
            joined_spans(&format_time_colored(d(5), &theme, false, false)),
            " 0:05.00"
        );
        assert_eq!(
            joined_spans(&format_time_colored(d(65), &theme, false, false)),
            " 1:05.00"
        );
        assert_eq!(
            joined_spans(&format_time_colored(d(3_599), &theme, false, false)),
            "59:59.00"
        );

        // Hours scale ("HhMM:SS")
        assert_eq!(
            joined_spans(&format_time_colored(d(11_565), &theme, false, false)),
            " 3h12:45"
        );
        assert_eq!(
            joined_spans(&format_time_colored(d(86_399), &theme, false, false)),
            "23h59:59"
        );

        // Days scale ("DDDd HHh") - previously 7 chars wide, misaligning the column
        assert_eq!(
            joined_spans(&format_time_colored(d(86_400), &theme, false, false)),
            "  1d 00h"
        );
        assert_eq!(
            joined_spans(&format_time_colored(d(432_000), &theme, false, false)),
            "  5d 00h"
        );
        assert_eq!(
            joined_spans(&format_time_colored(d(8_582_400), &theme, false, false)),
            " 99d 08h"
        );
        assert_eq!(
            joined_spans(&format_time_colored(d(31_449_600), &theme, false, false)),
            "364d 00h"
        );

        // Years scale ("YYYyDDDDd")
        assert_eq!(
            joined_spans(&format_time_colored(d(34_560_000), &theme, false, false)),
            "  1y035d"
        );
    }

    #[test]
    fn time_column_multicolor_splits_align_with_uniform() {
        let theme = Theme::default_theme();
        let d = |secs: u64| std::time::Duration::from_secs(secs);

        // Hours scale splits as "{:2}h" + "{:02}:{:02}"
        let spans = format_time_colored(d(11_565), &theme, false, true);
        assert_eq!(spans.len(), 2);
        assert_eq!(joined_spans(&spans), " 3h12:45");

        // Days scale splits as "{:3}d " + "{:02}h" (was "{:2}d ", 7 chars total)
        let spans = format_time_colored(d(432_000), &theme, false, true);
        assert_eq!(spans.len(), 2);
        assert_eq!(joined_spans(&spans), "  5d 00h");

        // Years scale splits as "{:3}y" + "{:03}d"
        let spans = format_time_colored(d(34_560_000), &theme, false, true);
        assert_eq!(spans.len(), 2);
        assert_eq!(joined_spans(&spans), "  1y035d");

        // Selected rows always collapse to one uniformly-colored span
        let spans = format_time_colored(d(432_000), &theme, true, true);
        assert_eq!(spans.len(), 1);
        assert_eq!(joined_spans(&spans), "  5d 00h");
    }
}
