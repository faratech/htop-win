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
    let block = Block::default()
        .borders(Borders::NONE);

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

    // Color based on usage
    let bar_color = theme.cpu_color(usage_clamped);

    // Create the bar characters manually for htop-style look
    let bar_width = area.width.saturating_sub(10) as usize;
    let filled = (usage_clamped as usize * bar_width / 100).min(bar_width);
    let empty = bar_width - filled;

    let bar_filled: String = "|".repeat(filled);
    let bar_empty: String = " ".repeat(empty);

    let label = format!("{:3}[", cpu_idx);
    let percent = format!("{:5.1}%]", usage_clamped);

    let line = Line::from(vec![
        Span::styled(label, Style::default().fg(theme.label)),
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

    let bar_width = area.width.saturating_sub(18) as usize;
    let filled = (usage as usize * bar_width / 100).min(bar_width);
    let empty = bar_width - filled;

    let bar_filled: String = "|".repeat(filled);
    let bar_empty: String = " ".repeat(empty);

    let label = "Mem[";
    let mem_info = format!(
        "{:5.1}%] {}/{}",
        usage,
        format_bytes(mem.used),
        format_bytes(mem.total)
    );

    // Use theme color for memory bar
    let bar_color = theme.mem_color(usage);

    let line = Line::from(vec![
        Span::styled(label, Style::default().fg(theme.label)),
        Span::styled(bar_filled, Style::default().fg(bar_color)),
        Span::raw(bar_empty),
        Span::styled(mem_info, Style::default().fg(theme.text)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn draw_swap_bar(frame: &mut Frame, app: &App, area: Rect) {
    let mem = &app.system_metrics.memory;
    let usage = mem.swap_percent.clamp(0.0, 100.0);
    let theme = &app.theme;

    let bar_width = area.width.saturating_sub(18) as usize;
    let filled = (usage as usize * bar_width / 100).min(bar_width);
    let empty = bar_width - filled;

    let bar_filled: String = "|".repeat(filled);
    let bar_empty: String = " ".repeat(empty);

    let label = "Swp[";
    let swap_info = format!(
        "{:5.1}%] {}/{}",
        usage,
        format_bytes(mem.swap_used),
        format_bytes(mem.swap_total)
    );

    // Use theme color for swap bar
    let bar_color = theme.swap_color(usage);

    let line = Line::from(vec![
        Span::styled(label, Style::default().fg(theme.label)),
        Span::styled(bar_filled, Style::default().fg(bar_color)),
        Span::raw(bar_empty),
        Span::styled(swap_info, Style::default().fg(theme.text)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn draw_tasks_info(frame: &mut Frame, app: &App, area: Rect) {
    let metrics = &app.system_metrics;
    let theme = &app.theme;

    let line = Line::from(vec![
        Span::styled("Tasks: ", Style::default().fg(theme.label)),
        Span::styled(
            format!("{}", metrics.tasks_total),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Span::raw(", "),
        Span::styled(
            format!("{} running", metrics.tasks_running),
            Style::default().fg(theme.status_running),
        ),
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

    let uptime_str = if days > 0 {
        format!("{}d {:02}:{:02}", days, hours, mins)
    } else {
        format!("{:02}:{:02}:{:02}", hours, mins, uptime % 60)
    };

    // Calculate load average (simulated on Windows)
    // Using running tasks / CPU count as an approximation
    let cpu_count = app.system_metrics.cpu.core_usage.len().max(1);
    let running = app.system_metrics.tasks_running;
    let load = running as f32 / cpu_count as f32;

    // Get current time
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple time formatting (UTC-based, will be local on most systems)
    let secs_in_day = now % 86400;
    let clock_hours = secs_in_day / 3600;
    let clock_mins = (secs_in_day % 3600) / 60;
    let clock_secs = secs_in_day % 60;
    let clock_str = format!("{:02}:{:02}:{:02}", clock_hours, clock_mins, clock_secs);

    let line = Line::from(vec![
        Span::styled("Load: ", Style::default().fg(theme.label)),
        Span::styled(format!("{:.2}", load), Style::default().fg(theme.text)),
        Span::raw("  "),
        Span::styled("Uptime: ", Style::default().fg(theme.label)),
        Span::styled(uptime_str, Style::default().fg(theme.text)),
        Span::raw("  "),
        Span::styled(clock_str, Style::default().fg(theme.text_dim)),
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
