pub mod colors;
pub mod dialogs;
mod footer;
mod header;
mod process_list;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::Block,
    Frame,
};

use crate::app::{App, ColumnBounds, SortColumn, ViewMode};

/// Draw the entire UI
pub fn draw(frame: &mut Frame, app: &mut App) {
    let size = frame.area();
    let theme = &app.theme;

    // Clear UI regions from previous frame (they'll be repopulated during this render)
    app.ui_bounds.clear_regions();

    // Fill entire screen with theme background color
    let bg_block = Block::default().style(Style::default().bg(theme.background));
    frame.render_widget(bg_block, size);

    // Main layout: header, process list, footer
    // Header is hidden if app.show_header is false
    let header_height = if app.show_header {
        header::calculate_header_height(app)
    } else {
        0
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(size);

    // Update UI bounds for mouse/keyboard navigation
    app.ui_bounds.header_y_start = 0;
    app.ui_bounds.header_y_end = if app.show_header { chunks[0].y + chunks[0].height } else { 0 };
    app.ui_bounds.column_header_y = chunks[1].y;
    app.ui_bounds.process_list_y_start = chunks[1].y + 1; // +1 to skip header row
    app.ui_bounds.process_list_y_end = chunks[1].y + chunks[1].height;
    app.ui_bounds.footer_y_start = chunks[2].y;

    // Calculate column bounds using the same constraint resolution as the Table widget
    app.ui_bounds.columns = calculate_column_bounds(&app.cached_visible_columns, chunks[1]);

    // Draw header (CPU bars, memory, etc.) if visible
    if app.show_header {
        header::draw(frame, app, chunks[0]);
    }

    // Store visible height for scrolling calculations
    app.visible_height = chunks[1].height.saturating_sub(1) as usize;

    // Draw process list
    process_list::draw(frame, app, chunks[1]);

    // Draw footer (function keys)
    footer::draw(frame, app, chunks[2]);

    // Draw dialog overlays if needed
    match app.view_mode {
        ViewMode::Help => dialogs::draw_help(frame, app),
        ViewMode::Search => dialogs::draw_search(frame, app),
        ViewMode::Filter => dialogs::draw_filter(frame, app),
        ViewMode::SortSelect => dialogs::draw_sort_select(frame, app),
        ViewMode::Kill => dialogs::draw_kill_confirm(frame, app),
        ViewMode::SignalSelect => dialogs::draw_signal_select(frame, app),
        ViewMode::Nice => dialogs::draw_nice(frame, app),
        ViewMode::Setup => dialogs::draw_setup(frame, app),
        ViewMode::ProcessInfo => dialogs::draw_process_info(frame, app),
        ViewMode::UserSelect => dialogs::draw_user_select(frame, app),
        ViewMode::Environment => dialogs::draw_environment(frame, app),
        ViewMode::ColorScheme => dialogs::draw_color_scheme(frame, app),
        ViewMode::CommandWrap => dialogs::draw_command_wrap(frame, app),
        ViewMode::ColumnConfig => dialogs::draw_column_config(frame, app),
        ViewMode::Affinity => dialogs::draw_affinity(frame, app),
        ViewMode::Normal => {}
    }

    // Draw error message if present
    if let Some(ref error) = app.last_error {
        dialogs::draw_error(frame, error);
    }
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
