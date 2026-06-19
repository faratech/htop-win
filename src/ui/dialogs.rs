use crate::terminal::{
    Block, Borders, Clear, Frame, Line, List, ListItem, Modifier, Paragraph, Rect, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Span, Style, Wrap,
};

use crate::app::{App, DialogState, SortColumn};
use crate::system::format_bytes;
use crate::ui::colors::ColorScheme;
use crate::ui::{centered_rect, centered_rect_fixed};

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
    if is_selected {
        selected_style(theme)
    } else {
        normal_style(theme)
    }
}

/// Word-wrap one logical line to `width` columns, preserving its leading
/// indentation and hard-splitting words longer than the available width
/// (e.g. long unbroken paths).
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    if width == 0 || text.width() <= width {
        return vec![text.to_string()];
    }

    let indent: String = text.chars().take_while(|c| *c == ' ').collect();
    let indent_width = indent.width();
    let avail = width.saturating_sub(indent_width).max(1);

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    let rest = &text[indent.len()..];
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut token_is_space = None;
    for ch in rest.chars() {
        let is_space = ch.is_whitespace();
        if token_is_space.is_some_and(|was_space| was_space != is_space) {
            tokens.push((token_is_space.unwrap(), std::mem::take(&mut token)));
        }
        token_is_space = Some(is_space);
        token.push(ch);
    }
    if let Some(is_space) = token_is_space {
        tokens.push((is_space, token));
    }

    for (_is_space, word) in tokens {
        let word_width = word.width();
        if current_width > 0 && current_width + word_width > avail {
            lines.push(format!("{indent}{current}"));
            current.clear();
            current_width = 0;
        }
        if current_width + word_width <= avail {
            current.push_str(&word);
            current_width += word_width;
        } else {
            for c in word.chars() {
                let char_width = UnicodeWidthChar::width(c).unwrap_or(0);
                if current_width > 0 && current_width + char_width > avail {
                    lines.push(format!("{indent}{current}"));
                    current.clear();
                    current_width = 0;
                }
                current.push(c);
                current_width += char_width;
            }
        }
    }
    if !current.is_empty() {
        lines.push(format!("{indent}{current}"));
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Render a scrollable line-based dialog. Clamps `*scroll` so content can't
/// be scrolled past its end — total/visible line counts are only known here
/// at render time, and a draw runs between every two input events, so input
/// handlers leave offsets unbounded (e.g. End sets usize::MAX).
fn render_scrollable_dialog(
    frame: &mut Frame,
    area: Rect,
    block: Block,
    style: Style,
    lines: Vec<Line>,
    scroll: &mut usize,
) {
    let total_lines = lines.len();
    let visible_lines = area.height.saturating_sub(2) as usize; // Account for border
    *scroll = (*scroll).min(total_lines.saturating_sub(visible_lines));

    let items: Vec<ListItem> = lines.into_iter().skip(*scroll).map(ListItem::new).collect();

    let list = List::new(items).block(block).style(style);

    frame.render_widget(Clear, area);
    frame.render_widget(list, area);

    if total_lines > visible_lines {
        let scrollbar_area = Rect::new(
            area.x + area.width - 1,
            area.y + 1,
            1,
            area.height.saturating_sub(2),
        );
        let mut scrollbar_state = ScrollbarState::new(total_lines)
            .viewport_content_length(visible_lines)
            .position(*scroll);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(style.fg.unwrap_or(crate::terminal::Color::Reset)));
        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}

/// Cache a dialog's geometry on `app` for the mouse handler. Text/content
/// dialogs have no selectable rows, so the list offset/header rows are zeroed.
fn cache_dialog_geometry(app: &mut App, area: Rect) {
    app.dialog_area = Some(area);
    app.dialog_inner = Some(area.inner(1));
    app.dialog_list_offset = 0;
    app.dialog_header_rows = 0;
    app.dialog_scroll_rows = 0;
}

/// Render a selectable list dialog with a unified scroll model:
/// `header_rows` leading items pinned to the top, a scrollable middle region
/// that auto-scrolls to keep `selected` visible, and `footer_rows` trailing
/// items pinned to the bottom. A scrollbar is drawn beside the middle region
/// only when it overflows. `selected` is an index into `items` within the
/// scrollable range (`header_rows ..= items.len()-footer_rows-1`). The drawn
/// geometry (area, inner rect, scroll offset, header rows) is cached on `app`
/// so the mouse handler can map a click back to the item under it.
#[allow(clippy::too_many_arguments)]
fn render_list_dialog(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    block: Block,
    style: Style,
    items: Vec<ListItem>,
    selected: usize,
    header_rows: usize,
    footer_rows: usize,
) {
    let inner = area.inner(1);
    let inner_height = inner.height as usize;
    let total = items.len();
    let pinned = header_rows + footer_rows;

    // Rows available to the scrollable middle region, and how many items it holds.
    let scroll_height = inner_height.saturating_sub(pinned);
    let scrollable_len = total.saturating_sub(pinned);

    // Derive the offset that keeps `selected` visible. Anchored so the selection
    // sits at the bottom of the region once scrolled past the first page; this is
    // a pure function of selection + sizes, so no offset needs to be stored.
    let sel_in_scroll = selected
        .saturating_sub(header_rows)
        .min(scrollable_len.saturating_sub(1));
    let max_offset = scrollable_len.saturating_sub(scroll_height);
    let offset = if scroll_height == 0 || sel_in_scroll < scroll_height {
        0
    } else {
        (sel_in_scroll - scroll_height + 1).min(max_offset)
    };

    let footer_start = total.saturating_sub(footer_rows);
    let scroll_start = header_rows + offset;
    let scroll_end = scroll_start + scroll_height;

    // Collect the visible items in order: header, scrolled middle, footer.
    let visible: Vec<ListItem> = items
        .into_iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let keep =
                i < header_rows || i >= footer_start || (i >= scroll_start && i < scroll_end);
            keep.then_some(item)
        })
        .collect();

    let list = List::new(visible).block(block).style(style);
    frame.render_widget(Clear, area);
    frame.render_widget(list, area);

    // Scrollbar beside the middle region only when it overflows.
    if scrollable_len > scroll_height && scroll_height > 0 {
        let scrollbar_area = Rect::new(
            area.x + area.width - 1,
            inner.y + header_rows as u16,
            1,
            scroll_height as u16,
        );
        let mut scrollbar_state = ScrollbarState::new(scrollable_len)
            .viewport_content_length(scroll_height)
            .position(offset);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(app.theme.text_dim));
        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }

    app.dialog_area = Some(area);
    app.dialog_inner = Some(inner);
    app.dialog_list_offset = offset;
    app.dialog_header_rows = header_rows;
    app.dialog_scroll_rows = scroll_height.min(scrollable_len);
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
pub fn draw_help(frame: &mut Frame, app: &mut App) {
    let DialogState::Help { scroll } = &mut app.dialog else {
        return;
    };
    let area = centered_rect(80, 80, frame.area());
    let theme = app.theme.clone();

    let help_text = vec![
        "",
        "  htop-win - Interactive Process Viewer for Windows",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  NAVIGATION",
        "  ─────────────────────────────────────────────────────────────",
        "    Tab                Switch to next screen tab (Main / I/O)",
        "    Shift+Tab          Switch to previous screen tab",
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
        "    F7, ]              Open priority dialog",
        "    F8, [              Open priority dialog",
        "    F9                 Kill selected/tagged process(es)",
        "    F10, q, Q          Quit",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  TAGGING & SELECTION",
        "  ─────────────────────────────────────────────────────────────",
        "    Space              Tag/untag process and move down",
        "    c                  Tag process with all its children",
        "    Ctrl+T             Tag all processes with same name",
        "    Ctrl+A             Toggle tag on all visible processes",
        "    U                  Untag all processes",
        "    u                  Filter by user (show user list)",
        "    F                  Toggle follow mode (track selected PID)",
        "    Note: Tagged processes show a yellow ● indicator and are",
        "          killed together when pressing F9. Count in status bar.",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  TREE VIEW (when enabled with F5)",
        "  ─────────────────────────────────────────────────────────────",
        "    +, =               Expand selected tree node",
        "    -                  Collapse selected tree node",
        "    *                  Toggle expand/collapse all nodes",
        "    Backspace          Collapse to parent",
        "    Double-click       Toggle tag for entire branch (parent + children)",
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
        "    Note: On machines with a GPU or NPU, GPU and NPU meters appear",
        "          in the header; GPU% / GPU-MEM and NPU% / NPU-MEM process",
        "          columns can be enabled via F2 > Configure columns.",
        "",
        "  ─────────────────────────────────────────────────────────────",
        "  MOUSE",
        "  ─────────────────────────────────────────────────────────────",
        "    Click process      Select process",
        "    Double-click       Open process details (or tag branch in tree mode)",
        "    Right-click        Tag/untag process (for batch kill)",
        "    Middle-click       Open kill dialog for process",
        "    Click header       Sort by column",
        "    Click meter        Cycle meter mode (Bar/Text/Graph/Hidden)",
        "    Click F-key        Trigger function key action",
        "    Scroll             Scroll process list (or dialog content)",
        "    Click dialog row   Select item (double-click to activate)",
        "    Click outside      Close dialog",
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
        "    --readonly         Disable kill/priority operations",
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

    let lines: Vec<Line> = help_text.into_iter().map(Line::from).collect();
    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background));

    render_scrollable_dialog(
        frame,
        area,
        block,
        Style::default().fg(theme.text).bg(theme.background),
        lines,
        scroll,
    );
    cache_dialog_geometry(app, area);
}

/// Draw search dialog
pub fn draw_search(frame: &mut Frame, app: &mut App) {
    let (buffer, cursor) = match app.dialog.input_buffer() {
        Some((b, c)) => (b.to_string(), c),
        None => return,
    };
    let area = centered_rect_fixed(50, 3, frame.area());
    let theme = app.theme.clone();
    let (visible_buffer, cursor_width) =
        input_window(&buffer, cursor, area.width.saturating_sub(3) as usize);

    let input = Paragraph::new(format!("/{}", visible_buffer))
        .block(
            Block::default()
                .title(" Search ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.border)),
        )
        .style(Style::default().fg(theme.text).bg(theme.background));

    frame.render_widget(Clear, area);
    frame.render_widget(input, area);

    let cursor_x = area.x + 2 + cursor_width.min(area.width.saturating_sub(3) as usize) as u16;
    frame.set_cursor_position((cursor_x, area.y + 1));
    cache_dialog_geometry(app, area);
}

/// Draw filter dialog
pub fn draw_filter(frame: &mut Frame, app: &mut App) {
    let (buffer, cursor) = match app.dialog.input_buffer() {
        Some((b, c)) => (b.to_string(), c),
        None => return,
    };
    let area = centered_rect_fixed(50, 3, frame.area());
    let theme = &app.theme;
    let (visible_buffer, cursor_width) =
        input_window(&buffer, cursor, area.width.saturating_sub(10) as usize);

    let input = Paragraph::new(format!("Filter: {}", visible_buffer))
        .block(
            Block::default()
                .title(" Filter ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.border)),
        )
        .style(Style::default().fg(theme.text).bg(theme.background));

    frame.render_widget(Clear, area);
    frame.render_widget(input, area);

    let cursor_x = area.x + 9 + cursor_width.min(area.width.saturating_sub(10) as usize) as u16;
    frame.set_cursor_position((cursor_x, area.y + 1));
    cache_dialog_geometry(app, area);
}

fn input_window(buffer: &str, cursor: usize, width: usize) -> (String, usize) {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    if width == 0 {
        return (String::new(), 0);
    }

    let cursor = cursor.min(buffer.len());
    let cursor = if buffer.is_char_boundary(cursor) {
        cursor
    } else {
        buffer
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|idx| *idx < cursor)
            .last()
            .unwrap_or(0)
    };
    let mut start = 0;
    if UnicodeWidthStr::width(&buffer[..cursor]) > width {
        for (idx, _) in buffer[..cursor].char_indices() {
            start = idx;
            if UnicodeWidthStr::width(&buffer[start..cursor]) <= width {
                break;
            }
        }
    }

    let mut visible = String::new();
    let mut used_width = 0usize;
    for ch in buffer[start..].chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if char_width > 0 && used_width + char_width > width {
            break;
        }
        visible.push(ch);
        used_width += char_width;
    }

    let cursor_width = UnicodeWidthStr::width(&buffer[start..cursor]);
    (visible, cursor_width)
}

/// Draw sort selection dialog
pub fn draw_sort_select(frame: &mut Frame, app: &mut App) {
    let DialogState::SortSelect { index } = &app.dialog else {
        return;
    };
    let index = *index;
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

            ListItem::new(Line::from(vec![Span::styled(
                format!(" {:<12}{}", col.name(), indicator),
                item_style(idx == index, theme),
            )]))
        })
        .collect();

    let block = Block::default()
        .title(" Sort by ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background));

    let style = Style::default().fg(theme.text).bg(theme.background);
    render_list_dialog(frame, app, area, block, style, items, index, 0, 0);
}

/// Draw kill confirmation dialog
pub fn draw_kill_confirm(frame: &mut Frame, app: &mut App) {
    let DialogState::Kill { pid, name, command } = &app.dialog else {
        return;
    };
    let tagged_count = app.tagged_pids.len();

    // Determine dialog height based on tagged processes
    let base_height = if tagged_count > 0 { 10 } else { 9 };
    let extra_height = tagged_count.min(8) as u16; // Show up to 8 tagged processes
    let height = base_height + extra_height;

    let area = centered_rect_fixed(55, height, frame.area());
    let theme = &app.theme;

    // Build content lines
    let mut lines: Vec<Line> = Vec::new();

    if tagged_count > 0 {
        // Multiple processes - show list
        lines.push(Line::from(Span::styled(
            format!("Kill {} tagged processes?", tagged_count),
            Style::default()
                .fg(theme.failed_read)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        // List tagged processes (show up to 8)
        for (shown, tagged_pid) in app.tagged_pids.iter().enumerate() {
            if shown >= 8 {
                lines.push(Line::from(Span::styled(
                    format!("  ... and {} more", tagged_count - 8),
                    Style::default().fg(theme.text_dim),
                )));
                break;
            }
            // Try to find process name
            let proc_name = app
                .displayed_processes
                .iter()
                .find(|p| p.pid == *tagged_pid)
                .map(|p| &*p.name)
                .unwrap_or("(unknown)");
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", tagged_pid),
                    Style::default().fg(theme.meter_value_warn),
                ),
                Span::styled(proc_name, Style::default().fg(theme.text)),
            ]));
        }
    } else {
        // Single process
        lines.push(Line::from(Span::styled(
            "Kill this process?",
            Style::default()
                .fg(theme.failed_read)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        lines.push(Line::from(vec![
            Span::styled("PID:  ", Style::default().fg(theme.text_dim)),
            Span::styled(
                format!("{}", pid),
                Style::default().fg(theme.meter_value_warn),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Name: ", Style::default().fg(theme.text_dim)),
            Span::styled(name.clone(), Style::default().fg(theme.text)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Cmd:  ", Style::default().fg(theme.text_dim)),
            Span::styled(
                truncate_str(command, 42),
                Style::default().fg(theme.text_dim),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("[Y/Enter/Click]", Style::default().fg(theme.meter_value_ok)),
        Span::raw(" Yes  "),
        Span::styled(
            "[N/Esc/Right-click]",
            Style::default().fg(theme.failed_read),
        ),
        Span::raw(" No"),
    ]));

    let dialog = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Kill Process ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.failed_read))
                .style(Style::default().bg(theme.background)),
        )
        .style(Style::default().fg(theme.text).bg(theme.background));

    frame.render_widget(Clear, area);
    frame.render_widget(dialog, area);
    cache_dialog_geometry(app, area);
}

/// Draw priority class dialog
pub fn draw_priority(frame: &mut Frame, app: &mut App) {
    use crate::app::WindowsPriorityClass;

    let DialogState::Priority {
        class_index,
        pid,
        name,
        ..
    } = &app.dialog
    else {
        return;
    };
    let class_index = *class_index;
    let pid = *pid;
    let process_info = format!("PID: {} - {}", pid, name);
    let classes = WindowsPriorityClass::all();
    let area = centered_rect_fixed(55, (classes.len() + 8) as u16, frame.area());

    // Read efficiency state from the captured pid (not the live selection): a
    // background re-sort can move a different process under the cursor while the
    // dialog is open, which would otherwise display a mismatched flag.
    let efficiency_mode = app
        .process_by_pid(pid)
        .map(|p| p.efficiency_mode)
        .unwrap_or(false);

    let theme = app.theme.clone();

    // Build list of priority classes with base priority values
    let mut items: Vec<ListItem> = classes
        .iter()
        .enumerate()
        .map(|(idx, class)| {
            let indicator = if idx == class_index { "▶ " } else { "  " };
            let style = if idx == class_index {
                selected_style(&theme)
            } else {
                normal_style(&theme)
            };
            ListItem::new(Line::from(Span::styled(
                format!(
                    "{}{:<14} (base priority: {:>2})",
                    indicator,
                    class.name(),
                    class.base_priority()
                ),
                style,
            )))
        })
        .collect();

    // Add separator and efficiency mode option (rendered inline after the
    // classes; they are not selectable, so they stay out of the scroll range).
    items.push(ListItem::new(Line::from("")));
    let efficiency_status = if efficiency_mode { "ON 🌿" } else { "OFF" };
    items.push(ListItem::new(Line::from(vec![
        Span::styled("  [E] Efficiency Mode: ", Style::default().fg(theme.label)),
        Span::styled(
            efficiency_status,
            Style::default().fg(if efficiency_mode {
                theme.meter_value_ok
            } else {
                theme.text_dim
            }),
        ),
    ])));

    let block = Block::default()
        .title(format!(" Set Priority: {} ", process_info))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background));

    let style = Style::default().fg(theme.text).bg(theme.background);
    render_list_dialog(frame, app, area, block, style, items, class_index, 0, 0);

    // Draw footer hint on the dialog's bottom inner row.
    let hint_area = Rect::new(area.x + 1, area.y + area.height - 2, area.width - 2, 1);
    let hint = Paragraph::new("↑↓ select, E efficiency, Enter apply, Esc cancel")
        .style(Style::default().fg(theme.text_dim).bg(theme.background));
    frame.render_widget(hint, hint_area);
}

/// Draw setup menu
pub fn draw_setup(frame: &mut Frame, app: &mut App) {
    let DialogState::Setup { selected } = &app.dialog else {
        return;
    };
    let selected = *selected;
    let theme = &app.theme;
    let area = centered_rect(60, 60, frame.area());

    // Build setup items with actual config values
    let setup_items: Vec<(&str, String)> = vec![
        ("Refresh rate", format!("{} ms", app.config.refresh_rate_ms)),
        ("CPU meter mode", meter_mode_str(app.config.cpu_meter_mode)),
        (
            "Memory meter mode",
            meter_mode_str(app.config.memory_meter_mode),
        ),
        ("GPU meter mode", meter_mode_str(app.config.gpu_meter_mode)),
        ("NPU meter mode", meter_mode_str(app.config.npu_meter_mode)),
        (
            "Show kernel threads",
            bool_to_str(app.config.show_kernel_threads),
        ),
        (
            "Show user threads",
            bool_to_str(app.config.show_user_threads),
        ),
        (
            "Show program path",
            bool_to_str(app.config.show_program_path),
        ),
        (
            "Highlight new processes",
            bool_to_str(app.config.highlight_new_processes),
        ),
        (
            "Highlight large numbers",
            bool_to_str(app.config.highlight_large_numbers),
        ),
        ("Tree view", bool_to_str(app.tree_view)),
        ("Confirm before kill", bool_to_str(app.config.confirm_kill)),
        ("Color scheme", app.config.color_scheme.name().to_string()),
        ("Configure columns", "→".to_string()),
        ("Reset all settings", "⚠".to_string()),
        (
            "GPU meter adapter",
            app.config
                .gpu_meter_adapter
                .clone()
                .unwrap_or_else(|| "Auto".to_string()),
        ),
    ];

    let items: Vec<ListItem> = setup_items
        .iter()
        .enumerate()
        .map(|(idx, (label, value))| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {:<30} ", label),
                    item_style(idx == selected, theme),
                ),
                Span::styled(value.to_string(), Style::default().fg(theme.meter_value_ok)),
            ]))
        })
        .collect();

    let block = Block::default()
        .title(" Setup (Enter to toggle, Esc to close) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background));

    let style = Style::default().fg(theme.text).bg(theme.background);
    render_list_dialog(frame, app, area, block, style, items, selected, 0, 0);
}

fn bool_to_str(val: bool) -> String {
    if val {
        "Yes".to_string()
    } else {
        "No".to_string()
    }
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
pub fn draw_process_info(frame: &mut Frame, app: &mut App) {
    let theme = &app.theme;
    let DialogState::ProcessInfo { target, scroll } = &mut app.dialog else {
        return;
    };
    let area = centered_rect(70, 80, frame.area());

    // Use captured target to prevent race condition with list refresh
    let content = {
        let proc = &**target;
        let status_desc = match proc.status {
            'R' => "Running",
            'S' => "Sleeping",
            'I' => "Idle",
            'Z' => "Zombie",
            'T' => "Stopped",
            '?' => "Unknown (not exposed by Windows native process query)",
            _ => "Unknown",
        };

        let exe_display = if proc.exe_path.is_empty() {
            "(access denied)".to_string()
        } else {
            proc.exe_path.to_string()
        };

        let arch_str = match proc.arch.as_str() {
            "" => "Native",
            s => s,
        };

        let elevated_str = if proc.is_elevated {
            "Yes 🛡️"
        } else {
            "No"
        };
        let efficiency_str = if proc.efficiency_mode {
            "Yes 🌿"
        } else {
            "No"
        };

        format!(
            " PROCESS\n\
             \n\
             PID             {}\n\
             Parent PID      {}\n\
             Name            {}\n\
             User            {}\n\
             Status          {} ({})\n\
             Architecture    {}\n\
             \n\
             ───────────────────────────────────────────────────────\n\
             \n\
             SCHEDULING\n\
             \n\
             Base Priority   {}\n\
             Priority Class  {}\n\
             Elevated        {}\n\
             Efficiency Mode {}\n\
             \n\
             ───────────────────────────────────────────────────────\n\
             \n\
             RESOURCES\n\
             \n\
             Threads         {}\n\
             Handles         {}\n\
             CPU Usage       {:.1}%\n\
             Memory Usage    {:.1}%\n\
             Virtual Memory  {}\n\
             Resident Memory {}\n\
             Shared Memory   {}\n\
             CPU Time        {}\n\
             \n\
             ───────────────────────────────────────────────────────\n\
             \n\
             DISK I/O\n\
             \n\
             Read Rate       {}/s\n\
             Write Rate      {}/s\n\
             Read (total)    {}\n\
             Write (total)   {}\n\
             \n\
             ───────────────────────────────────────────────────────\n\
             \n\
             PATHS\n\
             \n\
             Executable\n   {}\n\
             \n\
             Command Line\n   {}\n\
             \n\
             Esc/q close   ↑/↓ PgUp/PgDn scroll",
            proc.pid,
            proc.parent_pid,
            proc.name,
            proc.user,
            proc.status,
            status_desc,
            arch_str,
            proc.priority,
            crate::app::WindowsPriorityClass::from_base_priority(proc.priority).name(),
            elevated_str,
            efficiency_str,
            proc.thread_count,
            proc.handle_count,
            proc.cpu_percent,
            proc.mem_percent,
            format_bytes(proc.virtual_mem),
            format_bytes(proc.resident_mem),
            format_bytes(proc.shared_mem),
            proc.format_cpu_time(),
            format_bytes(proc.io_read_rate),
            format_bytes(proc.io_write_rate),
            format_bytes(proc.io_read_bytes),
            format_bytes(proc.io_write_bytes),
            exe_display,
            proc.command,
        )
    };

    let wrap_width = area.width.saturating_sub(2) as usize;
    let lines: Vec<Line> = content
        .lines()
        .flat_map(|line| wrap_text(line, wrap_width))
        .map(Line::from)
        .collect();

    let block = Block::default()
        .title(format!(" {} ", target.name))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background));

    render_scrollable_dialog(
        frame,
        area,
        block,
        Style::default().fg(theme.text).bg(theme.background),
        lines,
        scroll,
    );
    cache_dialog_geometry(app, area);
}

/// Draw error message
pub fn draw_error(frame: &mut Frame, app: &mut App, error: &str) {
    let area = centered_rect_fixed(60, 5, frame.area());
    let theme = &app.theme;

    let dialog = Paragraph::new(format!("\n{}\n\nPress any key to dismiss", error))
        .block(
            Block::default()
                .title(" Error ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.failed_read))
                .style(Style::default().bg(theme.background)),
        )
        .style(Style::default().fg(theme.failed_read).bg(theme.background))
        .wrap(Wrap { trim: true });

    frame.render_widget(Clear, area);
    frame.render_widget(dialog, area);
    cache_dialog_geometry(app, area);
}

fn truncate_str(s: &str, max_len: usize) -> String {
    use unicode_width::UnicodeWidthStr;

    if s.width() <= max_len {
        s.to_string()
    } else if max_len == 0 {
        String::new()
    } else {
        let mut result = String::with_capacity(max_len + 3);
        let mut current_width = 0;
        for c in s.chars() {
            let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
            if current_width + char_width >= max_len {
                result.push('\u{2026}'); // ellipsis
                break;
            }
            result.push(c);
            current_width += char_width;
        }
        result
    }
}

/// Draw signal selection dialog
pub fn draw_signal_select(frame: &mut Frame, app: &mut App) {
    let DialogState::SignalSelect {
        index, pid, name, ..
    } = &app.dialog
    else {
        return;
    };
    let index = *index;
    let title = format!(" Send Signal to {} ({}) ", name, pid);
    let theme = &app.theme;
    let area = centered_rect_fixed(40, (SIGNALS.len() + 4) as u16, frame.area());

    let items: Vec<ListItem> = SIGNALS
        .iter()
        .enumerate()
        .map(|(idx, (num, sig_name, desc))| {
            let style = item_style(idx == index, theme);
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {:2} ", num), style),
                Span::styled(format!("{:<10}", sig_name), style),
                Span::styled(desc.to_string(), style),
            ]))
        })
        .collect();

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.failed_read))
        .style(Style::default().bg(theme.background));

    let style = Style::default().fg(theme.text).bg(theme.background);
    render_list_dialog(frame, app, area, block, style, items, index, 0, 0);
}

/// Draw user selection dialog
pub fn draw_user_select(frame: &mut Frame, app: &mut App) {
    let DialogState::UserSelect { index, users } = &app.dialog else {
        return;
    };
    let index = *index;
    let theme = &app.theme;
    let num_items = users.len() + 1; // +1 for "All users"
    let area = centered_rect_fixed(35, (num_items + 2).min(20) as u16, frame.area());

    let mut items: Vec<ListItem> = Vec::with_capacity(num_items);

    // "All users" option
    items.push(ListItem::new(Line::from(Span::styled(
        " [All users]",
        item_style(index == 0, theme),
    ))));

    // Individual users
    for (idx, user) in users.iter().enumerate() {
        items.push(ListItem::new(Line::from(Span::styled(
            format!(" {}", user),
            item_style(idx + 1 == index, theme),
        ))));
    }

    let block = Block::default()
        .title(" Filter by User ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background));

    let style = Style::default().fg(theme.text).bg(theme.background);
    render_list_dialog(frame, app, area, block, style, items, index, 0, 0);
}

/// Draw environment variables dialog
pub fn draw_environment(frame: &mut Frame, app: &mut App) {
    let DialogState::Environment { pid, .. } = &app.dialog else {
        return;
    };
    let pid = *pid;
    let area = centered_rect(80, 80, frame.area());
    let theme = app.theme.clone();

    let content = app
        .process_by_pid(pid)
        .map(|proc| {
            format!(
                "Environment Variables for {} (PID: {})\n\n\
             Note: Environment variables cannot be read from \n\
             other processes on Windows without elevated privileges.\n\n\
             Command line:\n{}\n\n\
             Press Esc to close",
                proc.name, proc.pid, proc.command
            )
        })
        .unwrap_or_else(|| "No process selected".to_string());

    let wrap_width = area.width.saturating_sub(2) as usize;
    let lines: Vec<Line> = content
        .lines()
        .flat_map(|line| wrap_text(line, wrap_width))
        .map(Line::from)
        .collect();

    let DialogState::Environment { scroll, .. } = &mut app.dialog else {
        return;
    };
    let block = Block::default()
        .title(" Environment ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background));

    render_scrollable_dialog(
        frame,
        area,
        block,
        Style::default().fg(theme.text).bg(theme.background),
        lines,
        scroll,
    );
    cache_dialog_geometry(app, area);
}

/// Draw color scheme selection dialog
pub fn draw_color_scheme(frame: &mut Frame, app: &mut App) {
    let DialogState::ColorScheme { index } = &app.dialog else {
        return;
    };
    let index = *index;
    let theme = &app.theme;
    let schemes = ColorScheme::all();
    let area = centered_rect_fixed(30, (schemes.len() + 2) as u16, frame.area());

    let items: Vec<ListItem> = schemes
        .iter()
        .enumerate()
        .map(|(idx, scheme)| {
            let indicator = if *scheme == app.config.color_scheme {
                " ●"
            } else {
                "  "
            };
            ListItem::new(Line::from(vec![Span::styled(
                format!("{} {}", indicator, scheme.name()),
                item_style(idx == index, theme),
            )]))
        })
        .collect();

    let block = Block::default()
        .title(" Color Scheme ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background));

    let style = Style::default().fg(theme.text).bg(theme.background);
    render_list_dialog(frame, app, area, block, style, items, index, 0, 0);
}

/// Draw GPU meter adapter selection dialog
pub fn draw_gpu_select(frame: &mut Frame, app: &mut App) {
    let DialogState::GpuSelect { index, names } = &app.dialog else {
        return;
    };
    let index = *index;
    let names = names.clone();
    let theme = &app.theme;
    let num = names.len() + 1;
    let area = centered_rect_fixed(44, (num + 2).min(16) as u16, frame.area());

    let current = app.config.gpu_meter_adapter.as_deref();
    let mut items: Vec<ListItem> = Vec::with_capacity(num);
    items.push(ListItem::new(Line::from(Span::styled(
        format!(
            "{} Auto (most VRAM)",
            if current.is_none() { "●" } else { " " }
        ),
        item_style(index == 0, theme),
    ))));
    for (i, name) in names.iter().enumerate() {
        let marker = if current == Some(name.as_str()) {
            "●"
        } else {
            " "
        };
        items.push(ListItem::new(Line::from(Span::styled(
            format!("{} {}", marker, name),
            item_style(i + 1 == index, theme),
        ))));
    }

    let block = Block::default()
        .title(" GPU Meter Adapter ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background));
    let style = Style::default().fg(theme.text).bg(theme.background);
    render_list_dialog(frame, app, area, block, style, items, index, 0, 0);
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
pub fn draw_command_wrap(frame: &mut Frame, app: &mut App) {
    let DialogState::CommandWrap { pid, .. } = &app.dialog else {
        return;
    };
    let pid = *pid;
    let area = centered_rect(80, 70, frame.area());
    let theme = app.theme.clone();

    let text_lines = if let Some(proc) = app.process_by_pid(pid) {
        let mut lines = vec![
            format!("Process: {} (PID: {})", proc.name, proc.pid),
            String::new(),
            "Command Line:".to_string(),
            String::new(),
        ];

        // Wrap command and path with a 2-space indent
        let max_width = area.width.saturating_sub(4) as usize;
        for segment in wrap_text(&proc.command, max_width) {
            lines.push(format!("  {}", segment));
        }

        lines.push(String::new());
        lines.push("Executable Path:".to_string());
        for segment in wrap_text(&proc.exe_path, max_width) {
            lines.push(format!("  {}", segment));
        }

        lines
    } else {
        vec!["No process selected".to_string()]
    };

    let lines: Vec<Line> = text_lines.into_iter().map(Line::from).collect();

    let DialogState::CommandWrap { scroll, .. } = &mut app.dialog else {
        return;
    };
    let block = Block::default()
        .title(" Command Line (w to close) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background));

    render_scrollable_dialog(
        frame,
        area,
        block,
        Style::default().fg(theme.text).bg(theme.background),
        lines,
        scroll,
    );
    cache_dialog_geometry(app, area);
}

/// Draw column configuration dialog
pub fn draw_column_config(frame: &mut Frame, app: &mut App) {
    let DialogState::ColumnConfig { index } = &app.dialog else {
        return;
    };
    let index = *index;
    let theme = &app.theme;
    let columns = SortColumn::all();
    let area = centered_rect_fixed(50, (columns.len() + 4) as u16, frame.area());

    let mut items: Vec<ListItem> = columns
        .iter()
        .enumerate()
        .map(|(idx, col)| {
            let col_name = col.name();
            let is_visible = app.is_column_visible_in_active_tab(col_name);
            let checkbox = if is_visible { "[✓]" } else { "[ ]" };
            // Show position in visible order if visible
            let order_str = if let Some(pos) = app.column_position_in_active_tab(col_name) {
                format!("{:>2}", pos + 1)
            } else {
                "  ".to_string()
            };
            let style = if idx == index {
                selected_style(theme)
            } else if is_visible {
                Style::default()
                    .fg(theme.meter_value_ok)
                    .bg(theme.background)
            } else {
                Style::default().fg(theme.text_dim).bg(theme.background)
            };
            ListItem::new(Line::from(vec![Span::styled(
                format!("{} {} {}", order_str, checkbox, col_name),
                style,
            )]))
        })
        .collect();

    // Add help text at bottom (pinned as footer rows so it stays visible while
    // the column list scrolls on short terminals).
    items.push(ListItem::new(Line::from("")));
    items.push(ListItem::new(Line::from(vec![Span::styled(
        "Shift+↑↓ to reorder",
        Style::default().fg(theme.text_dim),
    )])));

    let block = Block::default()
        .title(" Columns (Space to toggle) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background));

    let style = Style::default().fg(theme.text).bg(theme.background);
    render_list_dialog(frame, app, area, block, style, items, index, 0, 2);
}

/// Draw CPU affinity dialog
pub fn draw_affinity(frame: &mut Frame, app: &mut App) {
    let DialogState::Affinity {
        mask,
        selected,
        pid,
    } = &app.dialog
    else {
        return;
    };
    let mask = *mask;
    let selected = *selected;
    let pid = *pid;
    let cpu_count = app.system_metrics.cpu.core_usage.len();
    let height = (cpu_count + 4).min(20) as u16;
    let area = centered_rect_fixed(35, height, frame.area());

    // Display the captured target, matching the process apply_affinity() acts on.
    let proc_name = app
        .process_by_pid(pid)
        .map(|p| format!("{} (PID: {})", p.name, p.pid))
        .unwrap_or_else(|| "Unknown".to_string());

    let theme = &app.theme;

    // Rows 0 and 1 (process name + blank) are pinned header rows; the CPU rows
    // that follow are the selectable region, so the navigable `selected` (a CPU
    // index) maps to item index `selected + 2`.
    let mut items: Vec<ListItem> = vec![ListItem::new(Line::from(vec![Span::styled(
        proc_name,
        Style::default().fg(theme.meter_label).bg(theme.background),
    )]))];

    items.push(ListItem::new(Line::from("")));

    for cpu_idx in 0..cpu_count {
        let is_set = (mask & (1u64 << cpu_idx)) != 0;
        let checkbox = if is_set { "[✓]" } else { "[ ]" };
        let style = if cpu_idx == selected {
            selected_style(theme)
        } else if is_set {
            Style::default()
                .fg(theme.meter_value_ok)
                .bg(theme.background)
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
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background));

    let style = Style::default().fg(theme.text).bg(theme.background);
    render_list_dialog(frame, app, area, block, style, items, selected + 2, 2, 0);
}

#[cfg(test)]
mod tests {
    use super::wrap_text;

    #[test]
    fn wrap_text_short_line_unchanged() {
        assert_eq!(wrap_text("hello world", 20), vec!["hello world"]);
        assert_eq!(wrap_text("", 10), vec![""]);
        // Internal alignment spacing survives when no wrapping is needed
        assert_eq!(
            wrap_text("PID             123", 40),
            vec!["PID             123"]
        );
    }

    #[test]
    fn wrap_text_exact_width() {
        assert_eq!(wrap_text("abcde", 5), vec!["abcde"]);
    }

    #[test]
    fn wrap_text_wraps_at_word_boundaries() {
        assert_eq!(wrap_text("foo bar baz", 7), vec!["foo bar", " baz"]);
    }

    #[test]
    fn wrap_text_hard_splits_long_words() {
        assert_eq!(
            wrap_text("C:\\averyveryverylongpath", 10),
            vec!["C:\\averyve", "ryverylong", "path"]
        );
    }

    #[test]
    fn wrap_text_preserves_indent() {
        assert_eq!(
            wrap_text("   foo bar baz", 7),
            vec!["   foo ", "   bar ", "   baz"]
        );
    }

    #[test]
    fn wrap_text_zero_width_is_passthrough() {
        assert_eq!(wrap_text("abc", 0), vec!["abc"]);
    }

    #[test]
    fn wrap_text_wide_chars() {
        // Each ideograph is two columns wide, so only two fit in four columns
        assert_eq!(wrap_text("日日日", 4), vec!["日日", "日"]);
    }
}
