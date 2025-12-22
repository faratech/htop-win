use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::app::{App, SortColumn, ViewMode};

/// Handle scroll keys for dialogs. Returns true if the key was handled.
fn handle_scroll_keys(scroll: &mut usize, key: KeyCode) -> bool {
    match key {
        KeyCode::Up | KeyCode::Char('k') => { *scroll = scroll.saturating_sub(1); true }
        KeyCode::Down | KeyCode::Char('j') => { *scroll += 1; true }
        KeyCode::PageUp => { *scroll = scroll.saturating_sub(10); true }
        KeyCode::PageDown => { *scroll += 10; true }
        KeyCode::Home => { *scroll = 0; true }
        _ => false,
    }
}

/// Handle keyboard events. Returns true if the app should quit.
pub fn handle_key_event(app: &mut App, key: KeyEvent) -> bool {
    // Only handle key press events, ignore release and repeat
    // This prevents "key bounce" issues where dialogs close immediately
    if key.kind != KeyEventKind::Press {
        return false;
    }

    // Clear error on any key press
    if app.last_error.is_some() {
        app.clear_error();
        return false;
    }

    match app.view_mode {
        ViewMode::Normal => handle_normal_keys(app, key),
        ViewMode::Help => handle_help_keys(app, key),
        ViewMode::Search => handle_search_keys(app, key),
        ViewMode::Filter => handle_filter_keys(app, key),
        ViewMode::SortSelect => handle_sort_select_keys(app, key),
        ViewMode::Kill => handle_kill_keys(app, key),
        ViewMode::SignalSelect => handle_signal_select_keys(app, key),
        ViewMode::Nice => handle_nice_keys(app, key),
        ViewMode::Setup => handle_setup_keys(app, key),
        ViewMode::ProcessInfo => handle_process_info_keys(app, key),
        ViewMode::UserSelect => handle_user_select_keys(app, key),
        ViewMode::Environment => handle_environment_keys(app, key),
        ViewMode::ColorScheme => handle_color_scheme_keys(app, key),
        ViewMode::CommandWrap => handle_command_wrap_keys(app, key),
        ViewMode::ColumnConfig => handle_column_config_keys(app, key),
        ViewMode::Affinity => handle_affinity_keys(app, key),
    }
}

fn handle_normal_keys(app: &mut App, key: KeyEvent) -> bool {
    // Check for max iterations exit
    if let Some(max) = app.max_iterations {
        if app.iteration_count >= max {
            return true;
        }
    }

    match key.code {
        // Quit
        KeyCode::F(10) | KeyCode::Char('q') | KeyCode::Char('Q') => return true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,

        // Redraw screen (Ctrl+L)
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.refresh_system();
        }

        // Navigation
        KeyCode::Up => app.select_up(),
        KeyCode::Char('k') if !key.modifiers.contains(KeyModifiers::CONTROL) => app.select_up(),
        KeyCode::Down | KeyCode::Char('j') => app.select_down(),
        KeyCode::PageUp => app.page_up(),
        KeyCode::PageDown => app.page_down(),
        KeyCode::Home | KeyCode::Char('g') => app.select_first(),
        KeyCode::End | KeyCode::Char('G') => app.select_last(),

        // Tagging
        KeyCode::Char(' ') => {
            app.toggle_tag();
            app.select_down();
        }
        KeyCode::Char('U') => app.untag_all(),
        KeyCode::Char('c') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.tag_with_children();
        }

        // User filter
        KeyCode::Char('u') => {
            app.enter_user_select_mode();
        }

        // Follow mode
        KeyCode::Char('F') => {
            app.toggle_follow_mode();
        }

        // Pause updates
        KeyCode::Char('Z') => {
            app.paused = !app.paused;
        }

        // Toggle header meters (#)
        KeyCode::Char('#') => {
            app.show_header = !app.show_header;
        }

        // Toggle kernel threads (K)
        KeyCode::Char('K') => {
            app.config.show_kernel_threads = !app.config.show_kernel_threads;
            app.update_displayed_processes();
        }

        // Toggle user threads (H)
        KeyCode::Char('H') => {
            app.config.show_user_threads = !app.config.show_user_threads;
            app.update_displayed_processes();
        }

        // Toggle program path (p)
        KeyCode::Char('p') => {
            app.config.show_program_path = !app.config.show_program_path;
            app.update_displayed_processes();
        }

        // Wrapped command display (w)
        KeyCode::Char('w') => {
            app.enter_command_wrap_mode();
        }

        // CPU affinity (a)
        KeyCode::Char('a') => {
            app.enter_affinity_mode();
        }

        // Tree expand/collapse
        KeyCode::Char('+') | KeyCode::Char('=') => {
            if app.tree_view {
                app.expand_tree();
            }
        }
        KeyCode::Char('-') => {
            if app.tree_view {
                app.collapse_tree();
            }
        }
        KeyCode::Char('*') => {
            if app.tree_view {
                if app.collapsed_pids.is_empty() {
                    app.collapse_all();
                } else {
                    app.expand_all();
                }
            }
        }
        // Collapse to parent (Backspace)
        KeyCode::Backspace => {
            if app.tree_view {
                app.collapse_to_parent();
            }
        }

        // Environment variables
        KeyCode::Char('e') => {
            app.enter_environment_mode();
        }

        // Function keys
        KeyCode::F(1) | KeyCode::Char('?') => {
            app.view_mode = ViewMode::Help;
            app.help_scroll = 0;
        }
        KeyCode::F(2) | KeyCode::Char('S') => {
            app.view_mode = ViewMode::Setup;
            app.setup_selected = 0;
        }
        KeyCode::F(3) | KeyCode::Char('/') => {
            app.start_search();
        }
        KeyCode::F(4) | KeyCode::Char('\\') => {
            app.start_filter();
        }
        KeyCode::F(5) | KeyCode::Char('t') => {
            app.toggle_tree_view();
        }
        // Sort column menu (F6, >, ., <, ,)
        KeyCode::F(6) | KeyCode::Char('>') | KeyCode::Char('.') | KeyCode::Char('<') | KeyCode::Char(',') => {
            app.view_mode = ViewMode::SortSelect;
            let columns = SortColumn::all();
            app.sort_select_index = columns
                .iter()
                .position(|c| *c == app.sort_column)
                .unwrap_or(0);
        }
        // Decrease priority / higher priority (F7, ])
        KeyCode::F(7) | KeyCode::Char(']') => {
            app.nice_value = -5;
            app.set_nice_selected(app.nice_value);
        }
        // Increase priority / lower priority (F8, [)
        KeyCode::F(8) | KeyCode::Char('[') => {
            app.nice_value = 5;
            app.set_nice_selected(app.nice_value);
        }
        KeyCode::F(9) => {
            app.enter_kill_mode();
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.enter_kill_mode();
        }

        // Search navigation
        KeyCode::Char('n') => app.find_next(),

        // Process details (Enter or h for help-like info)
        KeyCode::Enter => {
            app.enter_process_info_mode();
        }

        // Sort shortcuts
        KeyCode::Char('N') => app.set_sort_column(SortColumn::Pid),  // Sort by PID
        KeyCode::Char('P') => app.set_sort_column(SortColumn::Cpu),  // Sort by CPU
        KeyCode::Char('M') => app.set_sort_column(SortColumn::Mem),  // Sort by Memory
        KeyCode::Char('T') => app.set_sort_column(SortColumn::Time), // Sort by Time

        // Reverse sort
        KeyCode::Char('I') => {
            app.sort_ascending = !app.sort_ascending;
            app.update_displayed_processes();
        }

        // Digit keys for PID search (0-9)
        KeyCode::Char(c) if c.is_ascii_digit() => {
            app.handle_pid_digit(c);
        }

        _ => {}
    }
    false
}

fn handle_help_keys(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('q') | KeyCode::F(10) => {
            app.view_mode = ViewMode::Normal;
        }
        _ if handle_scroll_keys(&mut app.help_scroll, key.code) => {}
        _ => {
            app.view_mode = ViewMode::Normal;
        }
    }
    false
}

fn handle_search_keys(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.exit_mode();
        }
        KeyCode::Enter => {
            app.apply_search();
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::F(3) => {
            app.apply_search();
            app.find_next();
        }
        KeyCode::Backspace => {
            app.input_backspace();
            // Live search
            app.search_string = app.input_buffer.clone();
            app.apply_search();
        }
        KeyCode::Delete => {
            app.input_delete();
        }
        KeyCode::Left => {
            app.input_left();
        }
        KeyCode::Right => {
            app.input_right();
        }
        KeyCode::Char(c) => {
            app.input_char(c);
            // Live search
            app.search_string = app.input_buffer.clone();
            app.apply_search();
        }
        _ => {}
    }
    false
}

fn handle_filter_keys(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.exit_mode();
        }
        KeyCode::Enter => {
            app.apply_filter();
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Backspace => {
            app.input_backspace();
            // Live filter
            app.filter_string = app.input_buffer.clone();
            app.update_displayed_processes();
        }
        KeyCode::Delete => {
            app.input_delete();
        }
        KeyCode::Left => {
            app.input_left();
        }
        KeyCode::Right => {
            app.input_right();
        }
        KeyCode::Char(c) => {
            app.input_char(c);
            // Live filter
            app.filter_string = app.input_buffer.clone();
            app.update_displayed_processes();
        }
        _ => {}
    }
    false
}

fn handle_sort_select_keys(app: &mut App, key: KeyEvent) -> bool {
    let columns = SortColumn::all();
    match key.code {
        KeyCode::Esc => {
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Enter => {
            if app.sort_select_index < columns.len() {
                app.set_sort_column(columns[app.sort_select_index]);
            }
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.sort_select_index > 0 {
                app.sort_select_index -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.sort_select_index < columns.len() - 1 {
                app.sort_select_index += 1;
            }
        }
        KeyCode::Home => {
            app.sort_select_index = 0;
        }
        KeyCode::End => {
            app.sort_select_index = columns.len() - 1;
        }
        _ => {}
    }
    false
}

fn handle_kill_keys(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.kill_target = None;
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Enter => {
            // Kill process with SIGTERM equivalent (15)
            if !app.tagged_pids.is_empty() {
                app.kill_tagged(15);
            } else {
                app.kill_target_process(15);
            }
            app.kill_target = None;
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Char('9') => {
            // SIGKILL equivalent
            if !app.tagged_pids.is_empty() {
                app.kill_tagged(9);
            } else {
                app.kill_target_process(9);
            }
            app.kill_target = None;
            app.view_mode = ViewMode::Normal;
        }
        _ => {}
    }
    false
}

fn handle_nice_keys(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Enter => {
            app.set_nice_selected(app.nice_value);
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Left => {
            app.nice_value = (app.nice_value - 1).max(-20);
        }
        KeyCode::Right => {
            app.nice_value = (app.nice_value + 1).min(19);
        }
        KeyCode::Up => {
            app.nice_value = (app.nice_value - 5).max(-20);
        }
        KeyCode::Down => {
            app.nice_value = (app.nice_value + 5).min(19);
        }
        _ => {}
    }
    false
}

fn handle_setup_keys(app: &mut App, key: KeyEvent) -> bool {
    use crate::config::MeterMode;
    use crate::ui::colors::ColorScheme;

    // Helper to cycle meter mode forward
    let cycle_meter_mode = |mode: MeterMode| -> MeterMode {
        match mode {
            MeterMode::Bar => MeterMode::Text,
            MeterMode::Text => MeterMode::Graph,
            MeterMode::Graph => MeterMode::Hidden,
            MeterMode::Hidden => MeterMode::Bar,
        }
    };

    // Helper to cycle meter mode backward
    let cycle_meter_mode_rev = |mode: MeterMode| -> MeterMode {
        match mode {
            MeterMode::Bar => MeterMode::Hidden,
            MeterMode::Text => MeterMode::Bar,
            MeterMode::Graph => MeterMode::Text,
            MeterMode::Hidden => MeterMode::Graph,
        }
    };

    match key.code {
        KeyCode::Esc | KeyCode::F(2) => {
            app.save_config();
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.setup_selected > 0 {
                app.setup_selected -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.setup_selected < 10 {
                // Number of setup items - 1
                app.setup_selected += 1;
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            // Toggle selected setting or open submenu
            match app.setup_selected {
                0 => {
                    // Cycle refresh rate: 100 -> 250 -> 500 -> 1000 -> 1500 -> 2000 -> 5000 -> 100
                    app.config.refresh_rate_ms = match app.config.refresh_rate_ms {
                        100 => 250,
                        250 => 500,
                        500 => 1000,
                        1000 => 1500,
                        1500 => 2000,
                        2000 => 5000,
                        _ => 100,
                    };
                }
                1 => {
                    // Cycle CPU meter mode
                    app.config.cpu_meter_mode = cycle_meter_mode(app.config.cpu_meter_mode);
                }
                2 => {
                    // Cycle Memory meter mode
                    app.config.memory_meter_mode = cycle_meter_mode(app.config.memory_meter_mode);
                }
                3 => {
                    // Toggle show kernel threads
                    app.config.show_kernel_threads = !app.config.show_kernel_threads;
                }
                4 => {
                    // Toggle show user threads
                    app.config.show_user_threads = !app.config.show_user_threads;
                }
                5 => {
                    // Toggle show program path
                    app.config.show_program_path = !app.config.show_program_path;
                    app.update_displayed_processes();
                }
                6 => {
                    // Toggle highlight new processes
                    app.config.highlight_new_processes = !app.config.highlight_new_processes;
                }
                7 => {
                    // Toggle highlight large numbers
                    app.config.highlight_large_numbers = !app.config.highlight_large_numbers;
                }
                8 => {
                    // Toggle tree view
                    app.toggle_tree_view();
                    app.config.tree_view_default = app.tree_view;
                }
                9 => {
                    // Open color scheme selection
                    let schemes = ColorScheme::all();
                    app.color_scheme_index = schemes.iter()
                        .position(|s| *s == app.config.color_scheme)
                        .unwrap_or(0);
                    app.view_mode = ViewMode::ColorScheme;
                }
                10 => {
                    // Open column configuration
                    app.enter_column_config_mode();
                }
                _ => {}
            }
        }
        KeyCode::Left | KeyCode::Right => {
            // Allow left/right to adjust values for some settings
            match app.setup_selected {
                0 => {
                    // Adjust refresh rate
                    if key.code == KeyCode::Right {
                        app.config.refresh_rate_ms = match app.config.refresh_rate_ms {
                            100 => 250,
                            250 => 500,
                            500 => 1000,
                            1000 => 1500,
                            1500 => 2000,
                            2000 => 5000,
                            _ => 100,
                        };
                    } else {
                        app.config.refresh_rate_ms = match app.config.refresh_rate_ms {
                            5000 => 2000,
                            2000 => 1500,
                            1500 => 1000,
                            1000 => 500,
                            500 => 250,
                            250 => 100,
                            _ => 5000,
                        };
                    }
                }
                1 => {
                    // Adjust CPU meter mode
                    if key.code == KeyCode::Right {
                        app.config.cpu_meter_mode = cycle_meter_mode(app.config.cpu_meter_mode);
                    } else {
                        app.config.cpu_meter_mode = cycle_meter_mode_rev(app.config.cpu_meter_mode);
                    }
                }
                2 => {
                    // Adjust Memory meter mode
                    if key.code == KeyCode::Right {
                        app.config.memory_meter_mode = cycle_meter_mode(app.config.memory_meter_mode);
                    } else {
                        app.config.memory_meter_mode = cycle_meter_mode_rev(app.config.memory_meter_mode);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
    false
}

fn handle_process_info_keys(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        _ => {
            app.process_info_target = None;
            app.view_mode = ViewMode::Normal;
        }
    }
    false
}

fn handle_signal_select_keys(app: &mut App, key: KeyEvent) -> bool {
    use crate::ui::dialogs::{get_signal_by_index, signal_count};

    match key.code {
        KeyCode::Esc => {
            app.view_mode = ViewMode::Kill;
        }
        KeyCode::Enter => {
            let signal = get_signal_by_index(app.signal_select_index);
            if !app.tagged_pids.is_empty() {
                app.kill_tagged(signal);
            } else {
                app.kill_target_process(signal);
            }
            app.kill_target = None;
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.signal_select_index > 0 {
                app.signal_select_index -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.signal_select_index < signal_count() - 1 {
                app.signal_select_index += 1;
            }
        }
        _ => {}
    }
    false
}

fn handle_user_select_keys(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Enter => {
            if app.user_select_index == 0 {
                // "All users" option
                app.user_filter = None;
            } else if let Some(user) = app.user_list.get(app.user_select_index - 1) {
                app.user_filter = Some(user.clone());
            }
            app.update_displayed_processes();
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.user_select_index > 0 {
                app.user_select_index -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.user_select_index < app.user_list.len() {
                app.user_select_index += 1;
            }
        }
        _ => {}
    }
    false
}

fn handle_environment_keys(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.view_mode = ViewMode::Normal;
        }
        _ => { handle_scroll_keys(&mut app.env_scroll, key.code); }
    }
    false
}

fn handle_color_scheme_keys(app: &mut App, key: KeyEvent) -> bool {
    use crate::ui::colors::ColorScheme;
    let schemes = ColorScheme::all();

    match key.code {
        KeyCode::Esc => {
            app.view_mode = ViewMode::Setup;
        }
        KeyCode::Enter => {
            if let Some(scheme) = schemes.get(app.color_scheme_index) {
                app.config.color_scheme = *scheme;
                app.update_theme();
                app.save_config();
            }
            app.view_mode = ViewMode::Setup;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.color_scheme_index > 0 {
                app.color_scheme_index -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.color_scheme_index < schemes.len() - 1 {
                app.color_scheme_index += 1;
            }
        }
        _ => {}
    }
    false
}

fn handle_command_wrap_keys(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('w') => {
            app.view_mode = ViewMode::Normal;
        }
        _ => { handle_scroll_keys(&mut app.command_wrap_scroll, key.code); }
    }
    false
}

fn handle_column_config_keys(app: &mut App, key: KeyEvent) -> bool {
    let all_columns = SortColumn::all();

    match key.code {
        KeyCode::Esc => {
            app.view_mode = ViewMode::Setup;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.column_config_index > 0 {
                app.column_config_index -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.column_config_index < all_columns.len() - 1 {
                app.column_config_index += 1;
            }
        }
        KeyCode::Char(' ') | KeyCode::Enter => {
            // Toggle column visibility
            if let Some(col) = all_columns.get(app.column_config_index) {
                let col_name = col.name().to_string();
                app.config.toggle_column(&col_name);
                app.update_visible_columns_cache();
                app.save_config();
            }
        }
        _ => {}
    }
    false
}

fn handle_affinity_keys(app: &mut App, key: KeyEvent) -> bool {
    let cpu_count = app.system_metrics.cpu.core_usage.len();

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.affinity_selected > 0 {
                app.affinity_selected -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.affinity_selected < cpu_count.saturating_sub(1) {
                app.affinity_selected += 1;
            }
        }
        KeyCode::Char(' ') => {
            // Toggle CPU in affinity mask
            let bit = 1u64 << app.affinity_selected;
            app.affinity_mask ^= bit;
        }
        KeyCode::Enter => {
            // Apply affinity
            app.apply_affinity();
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Char('a') => {
            // Select all CPUs
            app.affinity_mask = (1u64 << cpu_count) - 1;
        }
        KeyCode::Char('n') => {
            // Select no CPUs (will be invalid, but user might want to start fresh)
            app.affinity_mask = 0;
        }
        _ => {}
    }
    false
}

/// Handle mouse events
pub fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            handle_mouse_click(app, mouse.column, mouse.row);
        }
        MouseEventKind::ScrollUp => {
            app.select_up();
            app.select_up();
            app.select_up();
        }
        MouseEventKind::ScrollDown => {
            app.select_down();
            app.select_down();
            app.select_down();
        }
        _ => {}
    }
}

fn handle_mouse_click(app: &mut App, x: u16, y: u16) {
    // Calculate header height to determine click location
    // This must match the calculation in ui/mod.rs
    let header_height = if app.show_header {
        let cpu_count = app.system_metrics.cpu.core_usage.len();
        let cpu_rows = (cpu_count + 1) / 2;
        let meter_rows = cpu_rows.max(4);
        (meter_rows + 2) as u16 + 2
    } else {
        0
    };

    if y < header_height {
        // Clicked in header - ignore or could add CPU bar interactions
        return;
    }

    let process_area_start = header_height;
    let relative_y = y.saturating_sub(process_area_start);

    if relative_y == 0 {
        // Clicked on column header - determine which column and toggle sort
        let column = get_column_from_x(x);
        if let Some(col) = column {
            if app.sort_column == col {
                // Same column - toggle ascending/descending
                app.sort_ascending = !app.sort_ascending;
            } else {
                // Different column - switch to it with default descending
                app.sort_column = col;
                app.sort_ascending = false;
            }
            app.update_displayed_processes();
        }
    } else {
        // Clicked on a process row
        let clicked_index = app.scroll_offset + (relative_y as usize).saturating_sub(1);
        if clicked_index < app.displayed_processes.len() {
            app.selected_index = clicked_index;
        }
    }
}

fn get_column_from_x(x: u16) -> Option<SortColumn> {
    // Column widths: [7, 7, 10, 4, 4, 4, 8, 8, 8, 2, 6, 6, 10, 8, 20+]
    // With column_spacing(1), each column occupies: width + 1 gap
    // Cumulative end positions (exclusive):
    const WIDTHS: [u16; 15] = [7, 7, 10, 4, 4, 4, 8, 8, 8, 2, 6, 6, 10, 8, 0];
    const COLUMNS: [SortColumn; 15] = [
        SortColumn::Pid, SortColumn::PPid, SortColumn::User, SortColumn::Priority,
        SortColumn::Nice, SortColumn::Threads, SortColumn::Virt, SortColumn::Res,
        SortColumn::Shr, SortColumn::Status, SortColumn::Cpu, SortColumn::Mem,
        SortColumn::Time, SortColumn::StartTime, SortColumn::Command,
    ];

    let mut pos: u16 = 0;
    for i in 0..14 {
        pos += WIDTHS[i] + 1; // width + gap
        if x < pos {
            return Some(COLUMNS[i]);
        }
    }
    Some(SortColumn::Command)
}
