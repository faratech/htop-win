use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::App;
use crate::system::format_bytes;

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

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
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

fn draw_left_column(frame: &mut Frame, app: &App, area: Rect) {
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

    // Draw CPU bars (left column of CPUs)
    for (i, row) in rows.iter().enumerate().take(meter_rows) {
        let cpu_idx = i * 2;
        if cpu_idx < cpu_count {
            draw_cpu_bar(frame, app, cpu_idx, app.system_metrics.cpu.core_usage[cpu_idx], *row);
        }
    }

    // Draw Memory bar
    if meter_rows < rows.len() {
        draw_memory_bar(frame, app, rows[meter_rows]);
    }

    // Draw Swap bar
    if meter_rows + 1 < rows.len() {
        draw_swap_bar(frame, app, rows[meter_rows + 1]);
    }
}

fn draw_right_column(frame: &mut Frame, app: &App, area: Rect) {
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
            draw_cpu_bar(frame, app, cpu_idx, app.system_metrics.cpu.core_usage[cpu_idx], rows[i]);
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
    let usage_clamped = usage.clamp(0.0, 100.0);
    let theme = &app.theme;

    // Color based on usage - htop uses green for user CPU (normal)
    // For now we use threshold-based coloring since we don't have per-mode CPU breakdown
    let bar_color = theme.cpu_color(usage_clamped);

    // htop format: "  N[||||...     XX.X%]"
    // Label is 4 chars (2 for number + "["), percentage is 6 chars (5.1f + "]")
    let bar_width = area.width.saturating_sub(11) as usize; // 4 label + 7 percent = 11
    let filled = ((usage_clamped as usize) * bar_width / 100).min(bar_width);
    let empty = bar_width - filled;

    let bar_filled: String = "|".repeat(filled);
    let bar_empty: String = " ".repeat(empty);

    // htop uses right-aligned 2-char CPU number
    let label = format!("{:>2}[", cpu_idx);
    let percent = format!("{:5.1}%]", usage_clamped);

    let line = Line::from(vec![
        Span::styled(label, Style::default().fg(theme.meter_label)),
        Span::styled(bar_filled, Style::default().fg(bar_color)),
        Span::raw(bar_empty),
        Span::styled(percent, Style::default().fg(theme.text)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn draw_memory_bar(frame: &mut Frame, app: &App, area: Rect) {
    let mem = &app.system_metrics.memory;
    let usage = mem.used_percent.clamp(0.0, 100.0);
    let theme = &app.theme;

    // htop format: "Mem[||||...    X.XXG/X.XXG]"
    let mem_info = format!("{}/{}", format_bytes(mem.used), format_bytes(mem.total));
    let info_len = mem_info.len() + 1; // +1 for the closing bracket
    let bar_width = area.width.saturating_sub(4 + info_len as u16) as usize; // 4 for "Mem["
    let filled = ((usage as usize) * bar_width / 100).min(bar_width);
    let empty = bar_width - filled;

    let bar_filled: String = "|".repeat(filled);
    let bar_empty: String = " ".repeat(empty);

    // Use theme color for memory bar (htop uses green for used memory)
    let bar_color = theme.memory_used;

    let line = Line::from(vec![
        Span::styled("Mem[", Style::default().fg(theme.meter_label)),
        Span::styled(bar_filled, Style::default().fg(bar_color)),
        Span::raw(bar_empty),
        Span::styled(format!("{}]", mem_info), Style::default().fg(theme.text)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn draw_swap_bar(frame: &mut Frame, app: &App, area: Rect) {
    let mem = &app.system_metrics.memory;
    let usage = mem.swap_percent.clamp(0.0, 100.0);
    let theme = &app.theme;

    // htop format: "Swp[||||...    X.XXG/X.XXG]"
    let swap_info = format!("{}/{}", format_bytes(mem.swap_used), format_bytes(mem.swap_total));
    let info_len = swap_info.len() + 1; // +1 for the closing bracket
    let bar_width = area.width.saturating_sub(4 + info_len as u16) as usize; // 4 for "Swp["
    let filled = ((usage as usize) * bar_width / 100).min(bar_width);
    let empty = bar_width - filled;

    let bar_filled: String = "|".repeat(filled);
    let bar_empty: String = " ".repeat(empty);

    // Use theme color for swap bar (htop uses red for swap)
    let bar_color = theme.swap;

    let line = Line::from(vec![
        Span::styled("Swp[", Style::default().fg(theme.meter_label)),
        Span::styled(bar_filled, Style::default().fg(bar_color)),
        Span::raw(bar_empty),
        Span::styled(format!("{}]", swap_info), Style::default().fg(theme.text)),
    ]);

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
        Span::styled("Tasks: ", Style::default().fg(theme.meter_label)),
        Span::styled(
            format!("{}", metrics.tasks_total),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Span::styled(", ", Style::default().fg(theme.text)),
        Span::styled(
            format!("{}", total_threads),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" thr; ", Style::default().fg(theme.text)),
        Span::styled(
            format!("{}", metrics.tasks_running),
            Style::default().fg(theme.status_running).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" running", Style::default().fg(theme.status_running)),
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

    // htop format: "Uptime: HH:MM:SS" or "D day(s), HH:MM:SS"
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
        Span::styled("CPU: ", Style::default().fg(theme.meter_label)),
        Span::styled(
            format!("{:5.1}%", cpu_percent),
            Style::default().fg(theme.cpu_color(cpu_percent)),
        ),
        Span::raw("  "),
        Span::styled("Uptime: ", Style::default().fg(theme.meter_label)),
        Span::styled(uptime_str, Style::default().fg(theme.text)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn draw_network_info(frame: &mut Frame, app: &App, area: Rect) {
    let metrics = &app.system_metrics;
    let theme = &app.theme;

    let rx_rate = format_bytes(metrics.net_rx_rate);
    let tx_rate = format_bytes(metrics.net_tx_rate);

    let line = Line::from(vec![
        Span::styled("Net[", Style::default().fg(theme.label)),
        Span::styled("↓", Style::default().fg(Color::Green)),
        Span::styled(format!("{}/s ", rx_rate), Style::default().fg(theme.text)),
        Span::styled("↑", Style::default().fg(Color::Red)),
        Span::styled(format!("{}/s", tx_rate), Style::default().fg(theme.text)),
        Span::styled("]", Style::default().fg(theme.label)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn draw_disk_info(frame: &mut Frame, app: &App, area: Rect) {
    let metrics = &app.system_metrics;
    let theme = &app.theme;

    let read_rate = format_bytes(metrics.disk_read_rate);
    let write_rate = format_bytes(metrics.disk_write_rate);

    let line = Line::from(vec![
        Span::styled("Dsk[", Style::default().fg(theme.label)),
        Span::styled("R:", Style::default().fg(Color::Cyan)),
        Span::styled(format!("{}/s ", read_rate), Style::default().fg(theme.text)),
        Span::styled("W:", Style::default().fg(Color::Yellow)),
        Span::styled(format!("{}/s", write_rate), Style::default().fg(theme.text)),
        Span::styled("]", Style::default().fg(theme.label)),
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
            Color::Green
        } else if percent > 20.0 {
            Color::Yellow
        } else {
            Color::Red
        };

        Line::from(vec![
            Span::styled("Bat[", Style::default().fg(theme.label)),
            Span::styled(status, Style::default().fg(color)),
            Span::styled(format!("{:.0}%", percent), Style::default().fg(color)),
            Span::styled("]", Style::default().fg(theme.label)),
        ])
    } else {
        // No battery detected, show hostname instead
        Line::from(vec![
            Span::styled(
                format!("Host: {}", metrics.hostname),
                Style::default().fg(theme.text_dim),
            ),
        ])
    };

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}
