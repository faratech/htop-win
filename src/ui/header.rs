use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::collections::VecDeque;

use crate::app::{App, UIElement, UIRegion};
use crate::config::MeterMode;
use crate::system::format_bytes;

/// Pre-computed bar strings to avoid repeated String::repeat() allocations.
/// Maximum bar width is 128 characters which covers most terminal widths.
const MAX_BAR_WIDTH: usize = 128;

/// Pre-computed string of '|' characters for bar fills
static BAR_FILL: &str = "||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||";

/// Pre-computed string of ' ' characters for bar empty space
static BAR_EMPTY: &str = "                                                                                                                                ";

/// Get a slice of bar fill characters (more efficient than String::repeat)
#[inline]
fn bar_fill(width: usize) -> &'static str {
    &BAR_FILL[..width.min(MAX_BAR_WIDTH)]
}

/// Get a slice of bar empty characters (more efficient than String::repeat)
#[inline]
fn bar_empty(width: usize) -> &'static str {
    &BAR_EMPTY[..width.min(MAX_BAR_WIDTH)]
}

/// Braille characters for sparkline graph - htop style
/// Each character encodes TWO data points (left column, right column)
/// Index = left_height * 5 + right_height (each 0-4 for 4 vertical dots)
/// This gives 25 combinations per character cell
const GRAPH_DOTS_UTF8: [&str; 25] = [
    /*00*/" ", /*01*/"⢀", /*02*/"⢠", /*03*/"⢰", /*04*/"⢸",
    /*10*/"⡀", /*11*/"⣀", /*12*/"⣠", /*13*/"⣰", /*14*/"⣸",
    /*20*/"⡄", /*21*/"⣄", /*22*/"⣤", /*23*/"⣴", /*24*/"⣼",
    /*30*/"⡆", /*31*/"⣆", /*32*/"⣦", /*33*/"⣶", /*34*/"⣾",
    /*40*/"⡇", /*41*/"⣇", /*42*/"⣧", /*43*/"⣷", /*44*/"⣿",
];

/// Calculate the header height based on CPU count
pub fn calculate_header_height(app: &App) -> u16 {
    let cpu_count = app.system_metrics.cpu.core_usage.len();
    // We display CPUs in two columns, plus memory and swap rows, plus task info
    let cpu_rows = (cpu_count + 1) / 2;
    // CPU rows + Mem row + Swap row + Net/Disk row + Tasks row + borders
    // Minimum of 4 rows for the meters
    let meter_rows = cpu_rows.max(4);
    (meter_rows + 2) as u16 + 2
}

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let theme = &app.theme;
    let block = Block::default()
        .borders(Borders::NONE)
        .style(Style::default().bg(theme.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split into left and right columns
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    draw_left_column(frame, app, columns[0]);
    draw_right_column(frame, app, columns[1]);
}

fn draw_left_column(frame: &mut Frame, app: &mut App, area: Rect) {
    let cpu_count = app.system_metrics.cpu.core_usage.len();
    let cpu_rows = (cpu_count + 1) / 2;
    let meter_rows = cpu_rows.max(4);

    // Create constraints for CPU bars (left half) plus meters
    let mut constraints: Vec<Constraint> = (0..meter_rows)
        .map(|_| Constraint::Length(1))
        .collect();
    // Add memory row
    constraints.push(Constraint::Length(1));
    // Add swap row
    constraints.push(Constraint::Length(1));

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    // Draw CPU bars (left column of CPUs) and register their regions
    for (i, row) in rows.iter().enumerate().take(meter_rows) {
        let cpu_idx = i * 2;
        if cpu_idx < cpu_count {
            // Register CPU meter region
            app.ui_bounds.add_region(UIRegion {
                element: UIElement::CpuMeter(Some(cpu_idx)),
                x: row.x,
                y: row.y,
                width: row.width,
                height: row.height,
            });
            draw_cpu_bar(frame, app, cpu_idx, app.system_metrics.cpu.core_usage[cpu_idx], *row);
        }
    }

    // Draw Memory bar and register region
    if meter_rows < rows.len() {
        let row = rows[meter_rows];
        app.ui_bounds.add_region(UIRegion {
            element: UIElement::MemoryMeter,
            x: row.x,
            y: row.y,
            width: row.width,
            height: row.height,
        });
        draw_memory_bar(frame, app, row);
    }

    // Draw Swap bar and register region
    if meter_rows + 1 < rows.len() {
        let row = rows[meter_rows + 1];
        app.ui_bounds.add_region(UIRegion {
            element: UIElement::SwapMeter,
            x: row.x,
            y: row.y,
            width: row.width,
            height: row.height,
        });
        draw_swap_bar(frame, app, row);
    }
}

fn draw_right_column(frame: &mut Frame, app: &mut App, area: Rect) {
    let cpu_count = app.system_metrics.cpu.core_usage.len();
    let cpu_rows = (cpu_count + 1) / 2;
    let meter_rows = cpu_rows.max(4);

    // Create constraints
    let mut constraints: Vec<Constraint> = (0..meter_rows)
        .map(|_| Constraint::Length(1))
        .collect();
    // Add tasks info row
    constraints.push(Constraint::Length(1));
    // Add load/uptime/net/disk row
    constraints.push(Constraint::Length(1));

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    // Draw CPU bars (right column of CPUs) and additional meters
    let mut row_idx = 0;
    for i in 0..meter_rows {
        let cpu_idx = i * 2 + 1;
        if cpu_idx < cpu_count {
            // Register CPU meter region
            let row = rows[i];
            app.ui_bounds.add_region(UIRegion {
                element: UIElement::CpuMeter(Some(cpu_idx)),
                x: row.x,
                y: row.y,
                width: row.width,
                height: row.height,
            });
            draw_cpu_bar(frame, app, cpu_idx, app.system_metrics.cpu.core_usage[cpu_idx], row);
        } else {
            // Draw additional meters in empty CPU slots
            match row_idx {
                0 => draw_network_info(frame, app, rows[i]),
                1 => draw_disk_info(frame, app, rows[i]),
                2 => draw_battery_info(frame, app, rows[i]),
                _ => {}
            }
            row_idx += 1;
        }
    }

    // Draw tasks info
    if meter_rows < rows.len() {
        draw_tasks_info(frame, app, rows[meter_rows]);
    }

    // Draw uptime
    if meter_rows + 1 < rows.len() {
        draw_uptime_info(frame, app, rows[meter_rows + 1]);
    }
}

fn draw_cpu_bar(frame: &mut Frame, app: &App, cpu_idx: usize, usage: f32, area: Rect) {
    let mode = app.config.cpu_meter_mode;

    // Hidden mode: don't render anything
    if mode == MeterMode::Hidden {
        return;
    }

    let usage_clamped = usage.clamp(0.0, 100.0);
    let theme = &app.theme;
    let label = format!("{:>2}", cpu_idx);

    let line = match mode {
        MeterMode::Text => {
            // Text mode: just show "N: XX.X%"
            Line::from(vec![
                Span::styled(label, Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
                Span::styled(": ", Style::default().fg(theme.text)),
                Span::styled(
                    format!("{:5.1}%", usage_clamped),
                    Style::default().fg(theme.cpu_color(usage_clamped)).add_modifier(Modifier::BOLD),
                ),
            ])
        }
        MeterMode::Graph => {
            // Graph mode: sparkline using history
            let history = app.cpu_history.get(cpu_idx);
            let graph_width = area.width.saturating_sub(10) as usize; // label + percent

            let graph_str = if let Some(hist) = history {
                render_sparkline(hist, graph_width)
            } else {
                bar_empty(graph_width).to_string()
            };

            Line::from(vec![
                Span::styled(format!("{}[", label), Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
                Span::styled(graph_str, Style::default().fg(theme.cpu_color(usage_clamped)).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:5.1}%]", usage_clamped), Style::default().fg(theme.text)),
            ])
        }
        MeterMode::Bar | MeterMode::Hidden => {
            // Bar mode (default): multi-segment bar with user/system breakdown (htop style)
            // htop uses: nice(blue) + user(green) + system(red) + iowait(gray)
            let bar_width = area.width.saturating_sub(11) as usize;
            let percent = format!("{:5.1}%]", usage_clamped);

            let breakdown = app
                .system_metrics
                .cpu
                .core_breakdown
                .get(cpu_idx)
                .copied();

            if let Some(bd) = breakdown {
                // Calculate widths for each segment
                let user_pct = bd.user.clamp(0.0, 100.0);
                let system_pct = bd.system.clamp(0.0, 100.0);
                let idle_pct = bd.idle.clamp(0.0, 100.0);

                // htop draws in order: nice, normal(user), system, iowait, irq, softirq, steal, guest
                // We have user and system, with remaining being "other" or idle
                let user_width = ((user_pct * bar_width as f32 / 100.0) as usize).min(bar_width);
                let system_width = ((system_pct * bar_width as f32 / 100.0) as usize).min(bar_width.saturating_sub(user_width));
                // iowait/other shows as gray - estimated from non-idle, non-user, non-system
                let other_pct = (100.0 - user_pct - system_pct - idle_pct).max(0.0);
                let other_width = ((other_pct * bar_width as f32 / 100.0) as usize).min(bar_width.saturating_sub(user_width + system_width));
                let empty_width = bar_width.saturating_sub(user_width + system_width + other_width);

                Line::from(vec![
                    Span::styled(format!("{}[", label), Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
                    // User time - green (htop: CPU_NORMAL)
                    Span::styled(bar_fill(user_width), Style::default().fg(theme.cpu_normal)),
                    // System/kernel time - red (htop: CPU_SYSTEM)
                    Span::styled(bar_fill(system_width), Style::default().fg(theme.cpu_system)),
                    // IO wait/other - gray (htop: CPU_IOWAIT)
                    Span::styled(bar_fill(other_width), Style::default().fg(theme.cpu_iowait)),
                    // Empty space
                    Span::styled(bar_empty(empty_width), Style::default().fg(theme.meter_shadow)),
                    Span::styled(percent, Style::default().fg(theme.text)),
                ])
            } else {
                // Fallback: single color bar based on usage threshold
                let bar_color = theme.cpu_color(usage_clamped);
                let filled = ((usage_clamped as usize) * bar_width / 100).min(bar_width);
                let empty = bar_width - filled;

                Line::from(vec![
                    Span::styled(format!("{}[", label), Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
                    Span::styled(bar_fill(filled), Style::default().fg(bar_color)),
                    Span::styled(bar_empty(empty), Style::default().fg(theme.meter_shadow)),
                    Span::styled(percent, Style::default().fg(theme.text)),
                ])
            }
        }
    };

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

/// Render a sparkline graph from history data - htop style
/// Each character encodes TWO consecutive values (left and right halves)
/// This doubles the effective horizontal resolution
fn render_sparkline(history: &VecDeque<f32>, width: usize) -> String {
    if history.is_empty() || width == 0 {
        return bar_empty(width).to_string();
    }

    // We need width*2 samples since each char shows 2 values
    let samples_needed = width * 2;
    let available_samples = history.len();
    let start = available_samples.saturating_sub(samples_needed);

    // Calculate how many graph chars we can generate and how many spaces we need
    let graph_chars = ((available_samples - start) + 1) / 2;
    let graph_chars = graph_chars.min(width);
    let padding_chars = width.saturating_sub(graph_chars);

    let mut result = String::with_capacity(width * 3); // UTF-8 braille is 3 bytes

    // Pre-add padding spaces (O(n) instead of O(n²) from repeated insert(0))
    for _ in 0..padding_chars {
        result.push(' ');
    }

    // Process samples in pairs using index-based access
    let mut i = start;
    let mut char_count = 0;
    while i < available_samples && char_count < graph_chars {
        // Left value (older)
        let v1 = history[i];
        // Right value (newer) - use same as left if at end
        let v2 = if i + 1 < available_samples { history[i + 1] } else { v1 };

        // Map 0-100% to 0-4 (5 levels for braille dots)
        let left = ((v1 / 100.0 * 4.0).round() as usize).min(4);
        let right = ((v2 / 100.0 * 4.0).round() as usize).min(4);

        // Index into 5x5 braille grid
        let idx = left * 5 + right;
        result.push_str(GRAPH_DOTS_UTF8[idx]);
        char_count += 1;
        i += 2;
    }

    result
}

fn draw_memory_bar(frame: &mut Frame, app: &App, area: Rect) {
    let mode = app.config.memory_meter_mode;

    if mode == MeterMode::Hidden {
        return;
    }

    let mem = &app.system_metrics.memory;
    let usage = mem.used_percent.clamp(0.0, 100.0);
    let theme = &app.theme;
    // htop shows "used + shared + compressed" in the info text (line 53-57 of MemoryMeter.c)
    let total_used = mem.used + mem.shared + mem.buffers;
    let mem_info = format!("{}/{}", format_bytes(total_used), format_bytes(mem.total));

    let line = match mode {
        MeterMode::Text => {
            // Text mode: just show "Mem: XX.X% (used/total)"
            Line::from(vec![
                Span::styled("Mem: ", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{:5.1}%", usage),
                    Style::default().fg(theme.memory_used).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" ({})", mem_info), Style::default().fg(theme.text)),
            ])
        }
        MeterMode::Graph => {
            // Graph mode: sparkline using history
            let graph_width = area.width.saturating_sub(mem_info.len() as u16 + 6) as usize;
            let graph_str = render_sparkline(&app.mem_history, graph_width);

            Line::from(vec![
                Span::styled("Mem[", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
                Span::styled(graph_str, Style::default().fg(theme.memory_used).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{}]", mem_info), Style::default().fg(theme.text)),
            ])
        }
        MeterMode::Bar | MeterMode::Hidden => {
            // Bar mode (default): multi-segment bar matching htop exactly
            // htop order: used (green) + shared (magenta) + buffers (blue) + cache (yellow)
            // See htop MemoryMeter.c: MemoryMeter_attributes[]
            let info_len = mem_info.len() + 1;
            let bar_width = area.width.saturating_sub(4 + info_len as u16) as usize;

            // Calculate segment percentages (htop style)
            let total_f = mem.total as f32;
            let used_pct = if total_f > 0.0 { (mem.used as f32 / total_f * 100.0).clamp(0.0, 100.0) } else { 0.0 };
            let shared_pct = if total_f > 0.0 { (mem.shared as f32 / total_f * 100.0).clamp(0.0, 100.0) } else { 0.0 };
            let buffers_pct = if total_f > 0.0 { (mem.buffers as f32 / total_f * 100.0).clamp(0.0, 100.0) } else { 0.0 };
            let cached_pct = if total_f > 0.0 { (mem.cached as f32 / total_f * 100.0).clamp(0.0, 100.0) } else { 0.0 };

            // Calculate widths ensuring they don't exceed bar_width
            let used_width = ((used_pct * bar_width as f32 / 100.0) as usize).min(bar_width);
            let shared_width = ((shared_pct * bar_width as f32 / 100.0) as usize).min(bar_width.saturating_sub(used_width));
            let buffers_width = ((buffers_pct * bar_width as f32 / 100.0) as usize).min(bar_width.saturating_sub(used_width + shared_width));
            let cached_width = ((cached_pct * bar_width as f32 / 100.0) as usize).min(bar_width.saturating_sub(used_width + shared_width + buffers_width));
            let empty_width = bar_width.saturating_sub(used_width + shared_width + buffers_width + cached_width);

            Line::from(vec![
                Span::styled("Mem[", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
                // Used memory - green (htop: MEMORY_USED)
                Span::styled(bar_fill(used_width), Style::default().fg(theme.memory_used)),
                // Shared memory - magenta (htop: MEMORY_SHARED)
                Span::styled(bar_fill(shared_width), Style::default().fg(theme.memory_shared)),
                // Buffer cache - blue bold (htop: MEMORY_BUFFERS)
                Span::styled(bar_fill(buffers_width), Style::default().fg(theme.memory_buffers).add_modifier(Modifier::BOLD)),
                // Page cache/standby - yellow (htop: MEMORY_CACHE)
                Span::styled(bar_fill(cached_width), Style::default().fg(theme.memory_cache)),
                // Empty/free space
                Span::styled(bar_empty(empty_width), Style::default().fg(theme.meter_shadow)),
                Span::styled(format!("{}]", mem_info), Style::default().fg(theme.text)),
            ])
        }
    };

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn draw_swap_bar(frame: &mut Frame, app: &App, area: Rect) {
    let mode = app.config.memory_meter_mode;

    if mode == MeterMode::Hidden {
        return;
    }

    let mem = &app.system_metrics.memory;
    let usage = mem.swap_percent.clamp(0.0, 100.0);
    let theme = &app.theme;

    // htop format: "Swp[||||...    X.XXG/X.XXG]"
    let swap_info = format!("{}/{}", format_bytes(mem.swap_used), format_bytes(mem.swap_total));

    let line = match mode {
        MeterMode::Text => {
            // Text mode: just show "Swp: XX.X% (used/total)"
            Line::from(vec![
                Span::styled("Swp: ", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{:5.1}%", usage),
                    Style::default().fg(theme.swap).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" ({})", swap_info), Style::default().fg(theme.text)),
            ])
        }
        MeterMode::Graph => {
            // Graph mode: sparkline using history
            let graph_width = area.width.saturating_sub(swap_info.len() as u16 + 6) as usize;
            let graph_str = render_sparkline(&app.swap_history, graph_width);

            Line::from(vec![
                Span::styled("Swp[", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
                Span::styled(graph_str, Style::default().fg(theme.swap).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{}]", swap_info), Style::default().fg(theme.text)),
            ])
        }
        MeterMode::Bar | MeterMode::Hidden => {
            // Bar mode (default)
            let info_len = swap_info.len() + 1; // +1 for the closing bracket
            let bar_width = area.width.saturating_sub(4 + info_len as u16) as usize; // 4 for "Swp["
            let filled = ((usage as usize) * bar_width / 100).min(bar_width);
            let empty = bar_width - filled;

            // Use theme color for swap bar (htop uses red for swap)
            let bar_color = theme.swap;

            Line::from(vec![
                Span::styled("Swp[", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
                Span::styled(bar_fill(filled), Style::default().fg(bar_color)),
                Span::styled(bar_empty(empty), Style::default().fg(theme.meter_shadow)),
                Span::styled(format!("{}]", swap_info), Style::default().fg(theme.text)),
            ])
        }
    };

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn draw_tasks_info(frame: &mut Frame, app: &App, area: Rect) {
    let metrics = &app.system_metrics;
    let theme = &app.theme;

    // htop format: "Tasks: N, M thr; K running"
    // Count total threads from all processes
    let total_threads: u32 = app.processes.iter().map(|p| p.thread_count).sum();

    let line = Line::from(vec![
        Span::styled("Tasks: ", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("{}", metrics.tasks_total),
            Style::default().fg(theme.meter_value).add_modifier(Modifier::BOLD),
        ),
        Span::styled(", ", Style::default().fg(theme.text)),
        Span::styled(
            format!("{}", total_threads),
            Style::default().fg(theme.meter_value).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" thr; ", Style::default().fg(theme.text)),
        Span::styled(
            format!("{}", metrics.tasks_running),
            Style::default().fg(theme.tasks_running).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" running", Style::default().fg(theme.tasks_running).add_modifier(Modifier::BOLD)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn draw_uptime_info(frame: &mut Frame, app: &App, area: Rect) {
    let uptime = app.system_metrics.uptime;
    let theme = &app.theme;
    let days = uptime / 86400;
    let hours = (uptime % 86400) / 3600;
    let mins = (uptime % 3600) / 60;
    let secs = uptime % 60;

    // htop format: "Uptime: D day(s), HH:MM:SS"
    let uptime_str = if days > 0 {
        let day_word = if days == 1 { "day" } else { "days" };
        format!("{} {}, {:02}:{:02}:{:02}", days, day_word, hours, mins, secs)
    } else {
        format!("{:02}:{:02}:{:02}", hours, mins, secs)
    };

    // Calculate overall CPU percentage
    let core_usage = &app.system_metrics.cpu.core_usage;
    let cpu_percent: f32 = if core_usage.is_empty() {
        0.0
    } else {
        core_usage.iter().sum::<f32>() / core_usage.len() as f32
    };

    let line = Line::from(vec![
        Span::styled("CPU: ", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("{:5.1}%", cpu_percent),
            Style::default().fg(theme.cpu_color(cpu_percent)).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("Uptime: ", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
        Span::styled(uptime_str, Style::default().fg(theme.uptime).add_modifier(Modifier::BOLD)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn draw_network_info(frame: &mut Frame, app: &App, area: Rect) {
    let metrics = &app.system_metrics;
    let theme = &app.theme;

    let rx_rate = format_bytes(metrics.net_rx_rate);
    let tx_rate = format_bytes(metrics.net_tx_rate);

    // htop style: use meter colors for I/O
    let line = Line::from(vec![
        Span::styled("Net[", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
        Span::styled("↓", Style::default().fg(theme.meter_value_ok)),  // Green for download
        Span::styled(format!("{}/s ", rx_rate), Style::default().fg(theme.meter_value).add_modifier(Modifier::BOLD)),
        Span::styled("↑", Style::default().fg(theme.meter_value_warn)),  // Yellow for upload
        Span::styled(format!("{}/s", tx_rate), Style::default().fg(theme.meter_value).add_modifier(Modifier::BOLD)),
        Span::styled("]", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn draw_disk_info(frame: &mut Frame, app: &App, area: Rect) {
    let metrics = &app.system_metrics;
    let theme = &app.theme;

    let read_rate = format_bytes(metrics.disk_read_rate);
    let write_rate = format_bytes(metrics.disk_write_rate);

    // htop style: use meter I/O read (green) and write (blue) colors
    let line = Line::from(vec![
        Span::styled("Dsk[", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
        Span::styled("R:", Style::default().fg(theme.meter_value_ok)),  // Green for read
        Span::styled(format!("{}/s ", read_rate), Style::default().fg(theme.meter_value).add_modifier(Modifier::BOLD)),
        Span::styled("W:", Style::default().fg(theme.memory_buffers)),  // Blue for write
        Span::styled(format!("{}/s", write_rate), Style::default().fg(theme.meter_value).add_modifier(Modifier::BOLD)),
        Span::styled("]", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn draw_battery_info(frame: &mut Frame, app: &App, area: Rect) {
    let metrics = &app.system_metrics;
    let theme = &app.theme;

    let line = if let Some(percent) = metrics.battery_percent {
        let status = if metrics.battery_charging { "+" } else { "-" };
        let color = if percent > 50.0 {
            theme.meter_value_ok  // Green
        } else if percent > 20.0 {
            theme.meter_value_warn  // Yellow
        } else {
            theme.meter_value_error  // Red
        };

        Line::from(vec![
            Span::styled("Bat[", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
            Span::styled(status, Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:.0}%", percent), Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::styled("]", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
        ])
    } else {
        // No battery detected, show hostname instead (htop style)
        Line::from(vec![
            Span::styled("Host: ", Style::default().fg(theme.meter_label).add_modifier(Modifier::BOLD)),
            Span::styled(
                metrics.hostname.clone(),
                Style::default().fg(theme.hostname).add_modifier(Modifier::BOLD),
            ),
        ])
    };

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}
