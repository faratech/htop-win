use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, SortColumn};
use crate::system::format_bytes;
use crate::ui::{centered_rect, centered_rect_fixed};
use crate::ui::colors::ColorScheme;

use crate::ui::colors::Theme;

/// Style for selected item in lists (uses theme colors)
fn selected_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.selection_fg)
        .bg(theme.selection_bg)
        .add_modifier(Modifier::BOLD)
}

/// Style for unselected item in lists (uses theme colors)
fn normal_style(theme: &Theme) -> Style {
    Style::default().fg(theme.text)
}

/// Get style based on selection state
fn item_style(is_selected: bool, theme: &Theme) -> Style {
    if is_selected { selected_style(theme) } else { normal_style(theme) }
}

/// Windows signal names and values
const SIGNALS: &[(u32, &str, &str)] = &[
    (15, "SIGTERM", "Terminate gracefully"),
    (9, "SIGKILL", "Force terminate"),
    (1, "SIGHUP", "Hangup"),
    (2, "SIGINT", "Interrupt (Ctrl+C)"),
    (3, "SIGQUIT", "Quit"),
    (6, "SIGABRT", "Abort"),
    (14, "SIGALRM", "Alarm clock"),
    (18, "SIGCONT", "Continue"),
    (19, "SIGSTOP", "Stop"),
];

/// Draw help dialog
pub fn draw_help(frame: &mut Frame, app: &App) {
    let area = centered_rect(80, 80, frame.area());

    let help_text = vec![
        "",
        "  htop-win - Interactive Process Viewer for Windows",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  NAVIGATION",
        "  ─────────────────────────────────────────────────────────────",
        "    Tab                Cycle focus: Process List → Footer → Header",
        "    Shift+Tab          Cycle focus backwards",
        "    Up/Down, j/k       Move selection up/down",
        "    Left/Right         Navigate within focused region",
        "    Enter              Activate focused element",
        "    PgUp/PgDown        Page up/down",
        "    Home/End, g/G      Go to first/last process",
        "    0-9                Incremental PID search",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  FUNCTION KEYS",
        "  ─────────────────────────────────────────────────────────────",
        "    F1, ?              Show this help",
        "    F2, S              Setup menu (settings, color schemes)",
        "    F3, /              Search processes (live search)",
        "    F4, \\              Filter processes (hide non-matching)",
        "    F5, t              Toggle tree view",
        "    F6, >, ., <, ,     Select sort column",
        "    F7, ]              Decrease priority (higher priority)",
        "    F8, [              Increase priority (lower priority)",
        "    F9                 Kill selected/tagged process(es)",
        "    F10, q, Q          Quit",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  TAGGING & SELECTION",
        "  ─────────────────────────────────────────────────────────────",
        "    Space              Tag/untag process (tagged = batch actions)",
        "    c                  Tag process with all children",
        "    U                  Untag all processes",
        "    u                  Filter by user (show user list)",
        "    F                  Toggle follow mode (track selected PID)",
        "    Note: Tagged processes are highlighted and killed together",
        "          when pressing F9 (Kill). Count shown in status bar.",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  TREE VIEW (when enabled with F5)",
        "  ─────────────────────────────────────────────────────────────",
        "    +, =               Expand selected tree node",
        "    -                  Collapse selected tree node",
        "    *                  Toggle expand/collapse all nodes",
        "    Backspace          Collapse to parent",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  SEARCH & SORT",
        "  ─────────────────────────────────────────────────────────────",
        "    n                  Find next search match",
        "    N                  Sort by PID",
        "    P                  Sort by CPU%",
        "    M                  Sort by Memory%",
        "    T                  Sort by Time",
        "    I                  Reverse sort order",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  PROCESS ACTIONS",
        "  ─────────────────────────────────────────────────────────────",
        "    Enter              Show process details (PID, memory, I/O)",
        "    e                  Show environment variables",
        "    w                  Show wrapped command line",
        "    a                  Set CPU affinity",
        "    Z                  Pause/resume process list updates",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  DISPLAY OPTIONS",
        "  ─────────────────────────────────────────────────────────────",
        "    #                  Toggle header meters visibility",
        "    p                  Toggle program path display",
        "    K                  Toggle kernel threads visibility",
        "    H                  Toggle user threads visibility",
        "    Ctrl+L             Redraw/refresh screen",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  MOUSE",
        "  ─────────────────────────────────────────────────────────────",
        "    Click process      Select process",
        "    Double-click       Open process details",
        "    Right-click        Tag/untag process (for batch kill)",
        "    Middle-click       Open kill dialog for process",
        "    Click header       Sort by column",
        "    Click meter        Cycle meter mode (Bar/Text/Graph/Hidden)",
        "    Click F-key        Trigger function key action",
        "    Scroll             Scroll process list (or dialog content)",
        "    Click in dialog    Close dialog",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  COMMAND LINE OPTIONS",
        "  ─────────────────────────────────────────────────────────────",
        "    -d, --delay MS     Refresh rate in milliseconds",
        "    -u, --user USER    Filter by user",
        "    -t, --tree         Start in tree view",
        "    -s, --sort COLUMN  Initial sort column",
        "    -p, --pid PID,...  Show only specific PIDs",
        "    -F, --filter STR   Initial filter string",
        "    -n, --max-iterations N  Exit after N updates",
        "    --no-mouse         Disable mouse support",
        "    --no-color         Monochrome mode",
        "    --no-meters        Hide header meters",
        "    --readonly         Disable kill/nice operations",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  GENERAL",
        "  ─────────────────────────────────────────────────────────────",
        "    Ctrl+C             Quit",
        "    Esc                Close dialog / cancel operation",
        "",
        "  Use Up/Down or PgUp/PgDown to scroll this help.",
        "  Press Esc or q to close.",
        "",
    ];

    let items: Vec<ListItem> = help_text
        .iter()
        .skip(app.help_scroll)
        .map(|line| ListItem::new(Line::from(*line)))
        .collect();

    let help_list = List::new(items)
        .block(
            Block::default()
                .title(" Help ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .style(Style::default().fg(Color::White));

    frame.render_widget(Clear, area);
    frame.render_widget(help_list, area);
}

/// Draw search dialog
pub fn draw_search(frame: &mut Frame, app: &App) {
    let area = centered_rect_fixed(50, 3, frame.area());

    let input = Paragraph::new(format!("/{}", app.input_buffer))
        .block(
            Block::default()
                .title(" Search ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .style(Style::default().fg(Color::White));

    frame.render_widget(Clear, area);
    frame.render_widget(input, area);

    // Set cursor position
    frame.set_cursor_position((area.x + 1 + app.input_cursor as u16 + 1, area.y + 1));
}

/// Draw filter dialog
pub fn draw_filter(frame: &mut Frame, app: &App) {
    let area = centered_rect_fixed(50, 3, frame.area());

    let input = Paragraph::new(format!("Filter: {}", app.input_buffer))
        .block(
            Block::default()
                .title(" Filter ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .style(Style::default().fg(Color::White));

    frame.render_widget(Clear, area);
    frame.render_widget(input, area);

    // Set cursor position
    frame.set_cursor_position((area.x + 9 + app.input_cursor as u16, area.y + 1));
}

/// Draw sort selection dialog
pub fn draw_sort_select(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let columns = SortColumn::all();
    let area = centered_rect_fixed(30, (columns.len() + 2) as u16, frame.area());

    let items: Vec<ListItem> = columns
        .iter()
        .enumerate()
        .map(|(idx, col)| {
            let indicator = if *col == app.sort_column {
                if app.sort_ascending { " ▲" } else { " ▼" }
            } else {
                ""
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!(" {:<12}{}", col.name(), indicator), item_style(idx == app.sort_select_index, theme)),
            ]))
        })
        .collect();

    let block = Block::default()
        .title(" Sort by ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .style(Style::default().bg(theme.background));

    let list = List::new(items)
        .block(block)
        .style(Style::default().fg(theme.text).bg(theme.background));

    frame.render_widget(Clear, area);
    frame.render_widget(list, area);
}

/// Draw kill confirmation dialog
pub fn draw_kill_confirm(frame: &mut Frame, app: &App) {
    let area = centered_rect_fixed(50, 8, frame.area());

    // Use captured kill_target to prevent race condition with list refresh
    let process_info = if let Some((pid, ref name, ref command)) = app.kill_target {
        format!(
            "PID: {}\nName: {}\nCommand: {}",
            pid,
            name,
            truncate_str(command, 40)
        )
    } else {
        "No process selected".to_string()
    };

    let tagged_info = if !app.tagged_pids.is_empty() {
        format!("\n\n{} tagged processes will also be killed", app.tagged_pids.len())
    } else {
        String::new()
    };

    let content = format!(
        "Kill this process?\n\n{}{}\n\nPress Enter to confirm, Esc to cancel",
        process_info, tagged_info
    );

    let dialog = Paragraph::new(content)
        .block(
            Block::default()
                .title(" Kill Process ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red)),
        )
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: true });

    frame.render_widget(Clear, area);
    frame.render_widget(dialog, area);
}

/// Draw nice value dialog
pub fn draw_nice(frame: &mut Frame, app: &App) {
    let area = centered_rect_fixed(40, 6, frame.area());

    // Use captured kill_target for consistency (Nice shares target with Kill)
    let process_info = if let Some((pid, ref name, _)) = app.kill_target {
        format!("PID: {} - {}", pid, name)
    } else {
        "No process selected".to_string()
    };

    let content = format!(
        "{}\n\nNew nice value: {}\n\n← → to adjust, Enter to set, Esc to cancel",
        process_info, app.nice_value
    );

    let dialog = Paragraph::new(content)
        .block(
            Block::default()
                .title(" Set Priority ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .style(Style::default().fg(Color::White));

    frame.render_widget(Clear, area);
    frame.render_widget(dialog, area);
}

/// Draw setup menu
pub fn draw_setup(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = centered_rect(60, 60, frame.area());

    // Build setup items with actual config values
    let setup_items: Vec<(&str, String)> = vec![
        ("Refresh rate", format!("{} ms", app.config.refresh_rate_ms)),
        ("CPU meter mode", meter_mode_str(app.config.cpu_meter_mode)),
        ("Memory meter mode", meter_mode_str(app.config.memory_meter_mode)),
        ("Show kernel threads", bool_to_str(app.config.show_kernel_threads)),
        ("Show user threads", bool_to_str(app.config.show_user_threads)),
        ("Show program path", bool_to_str(app.config.show_program_path)),
        ("Highlight new processes", bool_to_str(app.config.highlight_new_processes)),
        ("Highlight large numbers", bool_to_str(app.config.highlight_large_numbers)),
        ("Tree view", bool_to_str(app.tree_view)),
        ("Color scheme", app.config.color_scheme.name().to_string()),
        ("Configure columns", "→".to_string()),
    ];

    let items: Vec<ListItem> = setup_items
        .iter()
        .enumerate()
        .map(|(idx, (label, value))| {
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {:<30} ", label), item_style(idx == app.setup_selected, theme)),
                Span::styled(value.to_string(), Style::default().fg(Color::Green)),
            ]))
        })
        .collect();

    let block = Block::default()
        .title(" Setup (Enter to toggle, Esc to close) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(theme.background));

    let list = List::new(items)
        .block(block)
        .style(Style::default().fg(theme.text).bg(theme.background));

    frame.render_widget(Clear, area);
    frame.render_widget(list, area);
}

fn bool_to_str(val: bool) -> String {
    if val { "Yes".to_string() } else { "No".to_string() }
}

fn meter_mode_str(mode: crate::config::MeterMode) -> String {
    use crate::config::MeterMode;
    match mode {
        MeterMode::Bar => "Bar".to_string(),
        MeterMode::Text => "Text".to_string(),
        MeterMode::Graph => "Graph".to_string(),
        MeterMode::Hidden => "Hidden".to_string(),
    }
}

/// Draw process info dialog
pub fn draw_process_info(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 70, frame.area());

    // Use captured process_info_target to prevent race condition with list refresh
    let content = if let Some(ref proc) = app.process_info_target {
        let status_desc = match proc.status {
            'R' => "Running",
            'S' => "Sleeping",
            'I' => "Idle",
            'Z' => "Zombie",
            'T' => "Stopped",
            _ => "Unknown",
        };

        let exe_display = if proc.exe_path.is_empty() {
            "(not available)".to_string()
        } else {
            proc.exe_path.clone()
        };

        format!(
            "Process Information\n\
             ─────────────────────────────────────\n\
             PID:           {}\n\
             Parent PID:    {}\n\
             Name:          {}\n\
             User:          {}\n\
             Status:        {} ({})\n\
             Priority:      {}\n\
             Nice:          {}\n\
             Threads:       {}\n\
             Handles:       {}\n\
             \n\
             CPU Usage:     {:.1}%\n\
             Memory Usage:  {:.1}%\n\
             Virtual Mem:   {}\n\
             Resident Mem:  {}\n\
             Shared Mem:    {}\n\
             CPU Time:      {}\n\
             \n\
             I/O Read:      {}\n\
             I/O Write:     {}\n\
             \n\
             Executable:\n{}\n\
             \n\
             Command Line:\n{}\n\
             \n\
             Press any key to close",
            proc.pid,
            proc.parent_pid,
            proc.name,
            proc.user,
            proc.status, status_desc,
            proc.priority,
            proc.nice,
            proc.thread_count,
            proc.handle_count,
            proc.cpu_percent,
            proc.mem_percent,
            format_bytes(proc.virtual_mem),
            format_bytes(proc.resident_mem),
            format_bytes(proc.shared_mem),
            proc.format_cpu_time(),
            format_bytes(proc.io_read_bytes),
            format_bytes(proc.io_write_bytes),
            exe_display,
            proc.command,
        )
    } else {
        "No process selected".to_string()
    };

    let dialog = Paragraph::new(content)
        .block(
            Block::default()
                .title(" Process Details ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, area);
    frame.render_widget(dialog, area);
}

/// Draw error message
pub fn draw_error(frame: &mut Frame, error: &str) {
    let area = centered_rect_fixed(60, 5, frame.area());

    let dialog = Paragraph::new(format!("\n{}\n\nPress any key to dismiss", error))
        .block(
            Block::default()
                .title(" Error ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red)),
        )
        .style(Style::default().fg(Color::Red))
        .wrap(Wrap { trim: true });

    frame.render_widget(Clear, area);
    frame.render_widget(dialog, area);
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

/// Draw signal selection dialog
pub fn draw_signal_select(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = centered_rect_fixed(40, (SIGNALS.len() + 4) as u16, frame.area());

    let items: Vec<ListItem> = SIGNALS
        .iter()
        .enumerate()
        .map(|(idx, (num, name, desc))| {
            let style = item_style(idx == app.signal_select_index, theme);
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {:2} ", num), style),
                Span::styled(format!("{:<10}", name), style),
                Span::styled(format!("{}", desc), style),
            ]))
        })
        .collect();

    let title = if let Some((pid, ref name, _)) = app.kill_target {
        format!(" Send Signal to {} ({}) ", name, pid)
    } else {
        " Send Signal ".to_string()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(theme.background));

    let list = List::new(items)
        .block(block)
        .style(Style::default().fg(theme.text).bg(theme.background));

    frame.render_widget(Clear, area);
    frame.render_widget(list, area);
}

/// Draw user selection dialog
pub fn draw_user_select(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let num_items = app.user_list.len() + 1; // +1 for "All users"
    let area = centered_rect_fixed(35, (num_items + 2).min(20) as u16, frame.area());

    let mut items: Vec<ListItem> = Vec::with_capacity(num_items);

    // "All users" option
    items.push(ListItem::new(Line::from(Span::styled(
        " [All users]",
        item_style(app.user_select_index == 0, theme),
    ))));

    // Individual users
    for (idx, user) in app.user_list.iter().enumerate() {
        items.push(ListItem::new(Line::from(Span::styled(
            format!(" {}", user),
            item_style(idx + 1 == app.user_select_index, theme),
        ))));
    }

    let block = Block::default()
        .title(" Filter by User ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(theme.background));

    let list = List::new(items)
        .block(block)
        .style(Style::default().fg(theme.text).bg(theme.background));

    frame.render_widget(Clear, area);
    frame.render_widget(list, area);
}

/// Draw environment variables dialog
pub fn draw_environment(frame: &mut Frame, app: &App) {
    let area = centered_rect(80, 80, frame.area());

    let content = app.process_info_target.as_ref()
        .or_else(|| app.selected_process())
        .map(|proc| format!(
            "Environment Variables for {} (PID: {})\n\n\
             Note: Environment variables cannot be read from \n\
             other processes on Windows without elevated privileges.\n\n\
             Command line:\n{}\n\n\
             Press Esc to close",
            proc.name, proc.pid, proc.command
        ))
        .unwrap_or_else(|| "No process selected".to_string());

    let dialog = Paragraph::new(content)
        .block(
            Block::default()
                .title(" Environment ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta)),
        )
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, area);
    frame.render_widget(dialog, area);
}

/// Draw color scheme selection dialog
pub fn draw_color_scheme(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let schemes = ColorScheme::all();
    let area = centered_rect_fixed(30, (schemes.len() + 2) as u16, frame.area());

    let items: Vec<ListItem> = schemes
        .iter()
        .enumerate()
        .map(|(idx, scheme)| {
            let indicator = if *scheme == app.config.color_scheme { " ●" } else { "  " };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} {}", indicator, scheme.name()), item_style(idx == app.color_scheme_index, theme)),
            ]))
        })
        .collect();

    let block = Block::default()
        .title(" Color Scheme ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .style(Style::default().bg(theme.background));

    let list = List::new(items)
        .block(block)
        .style(Style::default().fg(theme.text).bg(theme.background));

    frame.render_widget(Clear, area);
    frame.render_widget(list, area);
}

/// Get signal value by index
pub fn get_signal_by_index(index: usize) -> u32 {
    SIGNALS.get(index).map(|(val, _, _)| *val).unwrap_or(15)
}

/// Get number of signals
pub fn signal_count() -> usize {
    SIGNALS.len()
}

/// Draw wrapped command display dialog
pub fn draw_command_wrap(frame: &mut Frame, app: &App) {
    let area = centered_rect(80, 70, frame.area());

    let content = if let Some(proc) = app.selected_process() {
        // Wrap command line nicely
        let mut lines = vec![
            format!("Process: {} (PID: {})", proc.name, proc.pid),
            String::new(),
            "Command Line:".to_string(),
            String::new(),
        ];

        // Split command into wrapped lines
        let max_width = area.width.saturating_sub(4) as usize;
        let command = &proc.command;
        let mut current_line = String::new();

        for word in command.split_whitespace() {
            if current_line.is_empty() {
                current_line = word.to_string();
            } else if current_line.len() + 1 + word.len() <= max_width {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                lines.push(format!("  {}", current_line));
                current_line = word.to_string();
            }
        }
        if !current_line.is_empty() {
            lines.push(format!("  {}", current_line));
        }

        lines.push(String::new());
        lines.push("Executable Path:".to_string());
        lines.push(format!("  {}", proc.exe_path));

        lines.join("\n")
    } else {
        "No process selected".to_string()
    };

    let items: Vec<ListItem> = content
        .lines()
        .skip(app.command_wrap_scroll)
        .map(|line| ListItem::new(Line::from(line.to_string())))
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Command Line (w to close) ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );

    frame.render_widget(Clear, area);
    frame.render_widget(list, area);
}

/// Draw column configuration dialog
pub fn draw_column_config(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let columns = SortColumn::all();
    let area = centered_rect_fixed(40, (columns.len() + 2) as u16, frame.area());

    let items: Vec<ListItem> = columns
        .iter()
        .enumerate()
        .map(|(idx, col)| {
            let col_name = col.name();
            let is_visible = app.config.is_column_visible(col_name);
            let checkbox = if is_visible { "[✓]" } else { "[ ]" };
            let style = if idx == app.column_config_index {
                selected_style(theme)
            } else if is_visible {
                Style::default().fg(Color::Green).bg(theme.background)
            } else {
                Style::default().fg(theme.text_dim).bg(theme.background)
            };
            ListItem::new(Line::from(vec![Span::styled(format!("{} {}", checkbox, col_name), style)]))
        })
        .collect();

    let block = Block::default()
        .title(" Columns (Space to toggle) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(theme.background));

    let list = List::new(items)
        .block(block)
        .style(Style::default().fg(theme.text).bg(theme.background));

    frame.render_widget(Clear, area);
    frame.render_widget(list, area);
}

/// Draw CPU affinity dialog
pub fn draw_affinity(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let cpu_count = app.system_metrics.cpu.core_usage.len();
    let height = (cpu_count + 4).min(20) as u16;
    let area = centered_rect_fixed(35, height, frame.area());

    let proc_name = app
        .selected_process()
        .map(|p| format!("{} (PID: {})", p.name, p.pid))
        .unwrap_or_else(|| "Unknown".to_string());

    let mut items: Vec<ListItem> = vec![ListItem::new(Line::from(vec![
        Span::styled(proc_name, Style::default().fg(theme.meter_label).bg(theme.background)),
    ]))];

    items.push(ListItem::new(Line::from("")));

    for cpu_idx in 0..cpu_count {
        let is_set = (app.affinity_mask & (1u64 << cpu_idx)) != 0;
        let checkbox = if is_set { "[✓]" } else { "[ ]" };
        let style = if cpu_idx == app.affinity_selected {
            selected_style(theme)
        } else if is_set {
            Style::default().fg(Color::Green).bg(theme.background)
        } else {
            Style::default().fg(theme.text_dim).bg(theme.background)
        };
        items.push(ListItem::new(Line::from(vec![Span::styled(
            format!("{} CPU {}", checkbox, cpu_idx),
            style,
        )])));
    }

    let block = Block::default()
        .title(" CPU Affinity ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .style(Style::default().bg(theme.background));

    let list = List::new(items)
        .block(block)
        .style(Style::default().fg(theme.text).bg(theme.background));

    frame.render_widget(Clear, area);
    frame.render_widget(list, area);
}
