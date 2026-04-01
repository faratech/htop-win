pub mod colors;
pub mod dialogs;
mod footer;
mod header;
mod process_list;

use crate::terminal::{
    Block, Constraint, Direction, Frame, Layout, Line, Modifier, Paragraph, Rect, Span, Style,
};

use crate::app::{App, ColumnBounds, DialogState, SortColumn};

/// Draw the entire UI
pub fn draw(frame: &mut Frame, app: &mut App) {
    let size = frame.area();
    let theme = &app.theme;

    // Clear UI regions from previous frame (they'll be repopulated during this render)
    app.ui_bounds.clear_regions();

    // Fill entire screen with theme background color
    let bg_block = Block::default().style(Style::default().bg(theme.background));
    frame.render_widget(bg_block, size);

    // Main layout: header, tab bar, process list, footer
    // Header is hidden if app.show_header is false
    let header_height = if app.show_header {
        header::calculate_header_height(app)
    } else {
        0
    };
    let tab_bar_height = if app.screen_tabs.len() > 1 { 1 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Length(tab_bar_height),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(size);

    // Update UI bounds for mouse/keyboard navigation
    app.ui_bounds.header_y_start = 0;
    app.ui_bounds.header_y_end = if app.show_header { chunks[0].y + chunks[0].height } else { 0 };
    app.ui_bounds.tab_bar_y = chunks[1].y;
    app.ui_bounds.tab_bar_visible = tab_bar_height > 0;
    app.ui_bounds.column_header_y = chunks[2].y;
    app.ui_bounds.process_list_y_start = chunks[2].y + 1; // +1 to skip header row
    app.ui_bounds.process_list_y_end = chunks[2].y + chunks[2].height;
    app.ui_bounds.footer_y_start = chunks[3].y;

    // Calculate column bounds using the same constraint resolution as the Table widget
    app.ui_bounds.columns = calculate_column_bounds(&app.cached_visible_columns, chunks[2]);

    // Draw header (CPU bars, memory, etc.) if visible
    if app.show_header {
        header::draw(frame, app, chunks[0]);
    }

    // Draw tab bar (if multiple tabs)
    if tab_bar_height > 0 {
        draw_tab_bar(frame, app, chunks[1]);
    }

    // Store visible height for scrolling calculations
    app.visible_height = chunks[2].height.saturating_sub(1) as usize;

    // Draw process list
    process_list::draw(frame, app, chunks[2]);

    // Draw footer (function keys)
    footer::draw(frame, app, chunks[3]);

    // Draw dialog overlays if needed
    match &app.dialog {
        DialogState::Help { .. } => dialogs::draw_help(frame, app),
        DialogState::Search { .. } => dialogs::draw_search(frame, app),
        DialogState::Filter { .. } => dialogs::draw_filter(frame, app),
        DialogState::SortSelect { .. } => dialogs::draw_sort_select(frame, app),
        DialogState::Kill { .. } => dialogs::draw_kill_confirm(frame, app),
        DialogState::SignalSelect { .. } => dialogs::draw_signal_select(frame, app),
        DialogState::Priority { .. } => dialogs::draw_priority(frame, app),
        DialogState::Setup { .. } => dialogs::draw_setup(frame, app),
        DialogState::ProcessInfo { .. } => dialogs::draw_process_info(frame, app),
        DialogState::UserSelect { .. } => dialogs::draw_user_select(frame, app),
        DialogState::Environment { .. } => dialogs::draw_environment(frame, app),
        DialogState::ColorScheme { .. } => dialogs::draw_color_scheme(frame, app),
        DialogState::CommandWrap { .. } => dialogs::draw_command_wrap(frame, app),
        DialogState::ColumnConfig { .. } => dialogs::draw_column_config(frame, app),
        DialogState::Affinity { .. } => dialogs::draw_affinity(frame, app),
        DialogState::None => {}
    }

    // Draw error message if present (within expiry window)
    if let Some((ref error, time)) = app.last_error
        && time.elapsed() < std::time::Duration::from_secs(5) {
            dialogs::draw_error(frame, error);
        }
}

/// Draw the screen tab bar (like htop's [Main] [I/O] tabs)
fn draw_tab_bar(frame: &mut Frame, app: &mut App, area: Rect) {
    use crate::app::{UIElement, UIRegion};
    let theme = &app.theme;
    let mut spans: Vec<Span> = Vec::new();
    let mut x_pos = area.x + 1; // Start after leading space

    // Add a leading space
    spans.push(Span::raw(" "));

    for (i, tab) in app.screen_tabs.iter().enumerate() {
        let is_active = i == app.active_tab;
        let label = format!("[{}]", tab.name);
        let label_width = label.len() as u16;

        if is_active {
            // Active tab: same as htop - bold with selection colors
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            // Inactive tab: dim
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(theme.text_dim)
                    .bg(theme.background),
            ));
        }

        // Register click region for this tab
        app.ui_bounds.add_region(UIRegion::new(
            UIElement::ScreenTab(i),
            x_pos, area.y, label_width, 1,
        ));

        x_pos += label_width + 1; // +1 for space separator
        spans.push(Span::raw(" "));
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line).style(Style::default().bg(theme.background));
    frame.render_widget(paragraph, area);
}

/// Center a rectangle within another
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Center a fixed-size rectangle within another
pub fn centered_rect_fixed(width: u16, height: u16, r: Rect) -> Rect {
    let x = r.x + (r.width.saturating_sub(width)) / 2;
    let y = r.y + (r.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(r.width), height.min(r.height))
}

/// Calculate column bounds based on visible columns and available area
/// Uses ratatui's Layout to resolve constraints exactly as the Table widget does
fn calculate_column_bounds(visible_columns: &[SortColumn], area: Rect) -> Vec<ColumnBounds> {
    if visible_columns.is_empty() {
        return Vec::new();
    }

    // Build the same constraints used in process_list.rs
    let constraints: Vec<Constraint> = visible_columns
        .iter()
        .map(|col| {
            if matches!(col, SortColumn::Command) {
                Constraint::Min(col.width())
            } else {
                Constraint::Length(col.width())
            }
        })
        .collect();

    // Use Layout to resolve constraints to actual widths
    // This matches how ratatui's Table internally calculates column positions
    let column_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .spacing(1) // Match Table's column_spacing(1)
        .split(area);

    // Build column bounds from the resolved layout
    visible_columns
        .iter()
        .enumerate()
        .map(|(i, col)| ColumnBounds {
            column: Some(*col),
            x: column_areas[i].x,
            width: column_areas[i].width,
        })
        .collect()
}
