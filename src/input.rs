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
        ViewMode::Priority => handle_priority_keys(app, key),
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
    use crate::app::FocusRegion;

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

        // Tab navigation between regions
        KeyCode::Tab => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                app.cycle_focus_prev();
            } else {
                app.cycle_focus_next();
            }
        }
        KeyCode::BackTab => {
            app.cycle_focus_prev();
        }

        // Redraw screen (Ctrl+L)
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.refresh_system();
        }

        // Arrow key navigation - depends on focus region
        KeyCode::Up => {
            match app.focus_region {
                FocusRegion::ProcessList => app.select_up(),
                FocusRegion::Header | FocusRegion::Footer => {
                    // Up in header/footer goes to process list
                    app.focus_region = FocusRegion::ProcessList;
                }
            }
        }
        KeyCode::Char('k') if !key.modifiers.contains(KeyModifiers::CONTROL) => app.select_up(),
        KeyCode::Down | KeyCode::Char('j') => {
            match app.focus_region {
                FocusRegion::ProcessList => app.select_down(),
                FocusRegion::Header | FocusRegion::Footer => {
                    // Down in header/footer goes to process list
                    app.focus_region = FocusRegion::ProcessList;
                }
            }
        }
        KeyCode::Left => app.navigate_left(),
        KeyCode::Right => app.navigate_right(),
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
        // Tag all processes with the same name (Ctrl+T)
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.tag_all_by_name();
        }
        // Tag/untag all visible processes (Ctrl+A)
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.tag_all_visible();
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
        // Higher priority (F7, ])
        KeyCode::F(7) | KeyCode::Char(']') => {
            app.enter_priority_mode();
        }
        // Lower priority (F8, [)
        KeyCode::F(8) | KeyCode::Char('[') => {
            app.enter_priority_mode();
        }
        KeyCode::F(9) => {
            app.enter_kill_mode();
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.enter_kill_mode();
        }

        // Search navigation
        KeyCode::Char('n') => app.find_next(),

        // Activate focused element (Enter)
        // In process list: opens process info
        // In footer: activates the focused function key
        // In header: toggles header visibility
        KeyCode::Enter => {
            if app.activate_focused() {
                return true; // Quit was requested
            }
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
        // Cancel: Esc, n, N, Delete, Backspace
        KeyCode::Esc | KeyCode::Delete | KeyCode::Backspace => {
            app.kill_target = None;
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            app.kill_target = None;
            app.view_mode = ViewMode::Normal;
        }
        // Confirm: Enter, y, Y, Space
        KeyCode::Enter | KeyCode::Char(' ') => {
            // Kill process with SIGTERM equivalent (15)
            if !app.tagged_pids.is_empty() {
                app.kill_tagged(15);
            } else {
                app.kill_target_process(15);
            }
            app.kill_target = None;
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
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

fn handle_priority_keys(app: &mut App, key: KeyEvent) -> bool {
    use crate::app::WindowsPriorityClass;

    let max_index = WindowsPriorityClass::all().len() - 1;

    match key.code {
        KeyCode::Esc => {
            app.view_mode = ViewMode::Normal;
        }
        KeyCode::Enter => {
            let priority_class = WindowsPriorityClass::from_index(app.priority_class_index);
            app.set_priority_selected(priority_class);
            app.view_mode = ViewMode::Normal;
        }
        // Up = move up in list (lower index)
        KeyCode::Up => {
            if app.priority_class_index > 0 {
                app.priority_class_index -= 1;
            }
        }
        // Down = move down in list (higher index)
        KeyCode::Down => {
            if app.priority_class_index < max_index {
                app.priority_class_index += 1;
            }
        }
        // Right = increase priority (higher index)
        KeyCode::Right => {
            if app.priority_class_index < max_index {
                app.priority_class_index += 1;
            }
        }
        // Left = decrease priority (lower index)
        KeyCode::Left => {
            if app.priority_class_index > 0 {
                app.priority_class_index -= 1;
            }
        }
        // E = toggle efficiency mode
        KeyCode::Char('e') | KeyCode::Char('E') => {
            app.toggle_efficiency_mode();
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
            if app.setup_selected < 12 {
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
                    // Toggle confirm before kill
                    app.config.confirm_kill = !app.config.confirm_kill;
                }
                10 => {
                    // Open color scheme selection
                    let schemes = ColorScheme::all();
                    app.color_scheme_index = schemes.iter()
                        .position(|s| *s == app.config.color_scheme)
                        .unwrap_or(0);
                    app.view_mode = ViewMode::ColorScheme;
                }
                11 => {
                    // Open column configuration
                    app.enter_column_config_mode();
                }
                12 => {
                    // Reset all settings to defaults
                    app.config.reset_to_defaults();
                    app.update_theme();
                    app.update_visible_columns_cache();
                    app.save_config();
                    app.status_message = Some((
                        "Settings reset to defaults".to_string(),
                        std::time::Instant::now(),
                    ));
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
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                // Shift+Up: Move column up in order
                if let Some(col) = all_columns.get(app.column_config_index) {
                    let col_name = col.name().to_string();
                    if app.config.move_column_up(&col_name) {
                        app.update_visible_columns_cache();
                        app.save_config();
                    }
                }
            } else {
                // Regular Up: Navigate
                if app.column_config_index > 0 {
                    app.column_config_index -= 1;
                }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                // Shift+Down: Move column down in order
                if let Some(col) = all_columns.get(app.column_config_index) {
                    let col_name = col.name().to_string();
                    if app.config.move_column_down(&col_name) {
                        app.update_visible_columns_cache();
                        app.save_config();
                    }
                }
            } else {
                // Regular Down: Navigate
                if app.column_config_index < all_columns.len() - 1 {
                    app.column_config_index += 1;
                }
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

/// Handle mouse events with unified element detection
pub fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    use crate::app::UIAction;
    use std::time::Instant;

    let x = mouse.column;
    let y = mouse.row;

    // Check if we're in a dialog/modal mode
    let is_in_dialog = matches!(
        app.view_mode,
        ViewMode::Help
            | ViewMode::Search
            | ViewMode::Filter
            | ViewMode::SortSelect
            | ViewMode::Kill
            | ViewMode::SignalSelect
            | ViewMode::Priority
            | ViewMode::Setup
            | ViewMode::ProcessInfo
            | ViewMode::UserSelect
            | ViewMode::Environment
            | ViewMode::ColorScheme
            | ViewMode::CommandWrap
            | ViewMode::ColumnConfig
            | ViewMode::Affinity
    );

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Handle dialogs specially
            if is_in_dialog {
                match app.view_mode {
                    // Kill dialog: left-click confirms the kill
                    ViewMode::Kill => {
                        if !app.tagged_pids.is_empty() {
                            app.kill_tagged(15);
                        } else {
                            app.kill_target_process(15);
                        }
                        app.kill_target = None;
                        app.view_mode = ViewMode::Normal;
                        return;
                    }
                    // SignalSelect: left-click confirms
                    ViewMode::SignalSelect => {
                        let signal = crate::ui::dialogs::get_signal_by_index(app.signal_select_index);
                        if !app.tagged_pids.is_empty() {
                            app.kill_tagged(signal);
                        } else {
                            app.kill_target_process(signal);
                        }
                        app.kill_target = None;
                        app.view_mode = ViewMode::Normal;
                        return;
                    }
                    // Other dialogs: close on click
                    _ => {
                        app.view_mode = ViewMode::Normal;
                        return;
                    }
                }
            }

            // Check for double-click
            let now = Instant::now();
            let is_double_click = if let (Some(last_pos), Some(last_time)) =
                (app.last_click_pos, app.last_click_time)
            {
                let same_position = last_pos == (x, y);
                let within_threshold = now.duration_since(last_time).as_millis() < app.double_click_ms as u128;
                same_position && within_threshold
            } else {
                false
            };

            // Update click tracking
            app.last_click_pos = Some((x, y));
            app.last_click_time = Some(now);

            let action = if is_double_click {
                // Clear for next potential double-click sequence
                app.last_click_pos = None;
                app.last_click_time = None;
                UIAction::DoubleClick
            } else {
                UIAction::Click
            };

            handle_element_action(app, x, y, action);
        }
        MouseEventKind::Down(MouseButton::Right) => {
            // Right-click in dialog mode closes the dialog (like Escape)
            if is_in_dialog {
                app.view_mode = ViewMode::Normal;
                return;
            }
            handle_element_action(app, x, y, UIAction::RightClick);
        }
        MouseEventKind::Down(MouseButton::Middle) => {
            handle_element_action(app, x, y, UIAction::MiddleClick);
        }
        MouseEventKind::ScrollUp => {
            // Scroll in dialogs should scroll the dialog content
            if is_in_dialog {
                match app.view_mode {
                    ViewMode::Help => app.help_scroll = app.help_scroll.saturating_sub(3),
                    ViewMode::ProcessInfo | ViewMode::Environment => {
                        app.env_scroll = app.env_scroll.saturating_sub(3);
                    }
                    ViewMode::CommandWrap => {
                        app.command_wrap_scroll = app.command_wrap_scroll.saturating_sub(3);
                    }
                    ViewMode::SortSelect => app.sort_select_index = app.sort_select_index.saturating_sub(3),
                    ViewMode::UserSelect => app.user_select_index = app.user_select_index.saturating_sub(3),
                    ViewMode::SignalSelect => app.signal_select_index = app.signal_select_index.saturating_sub(3),
                    _ => {}
                }
            } else {
                app.select_up();
                app.select_up();
                app.select_up();
            }
        }
        MouseEventKind::ScrollDown => {
            if is_in_dialog {
                match app.view_mode {
                    ViewMode::Help => app.help_scroll += 3,
                    ViewMode::ProcessInfo | ViewMode::Environment => app.env_scroll += 3,
                    ViewMode::CommandWrap => app.command_wrap_scroll += 3,
                    ViewMode::SortSelect => app.sort_select_index += 3,
                    ViewMode::UserSelect => app.user_select_index += 3,
                    ViewMode::SignalSelect => app.signal_select_index += 3,
                    _ => {}
                }
            } else {
                app.select_down();
                app.select_down();
                app.select_down();
            }
        }
        _ => {}
    }
}

/// Handle an action on a UI element at the given position
fn handle_element_action(app: &mut App, x: u16, y: u16, action: crate::app::UIAction) {
    use crate::app::{UIAction, UIElement};

    // Get the element at this position
    let element = app.ui_bounds.element_at(x, y);

    // For process rows, fill in the actual PID
    let element = match element {
        Some(UIElement::ProcessRow { index, .. }) => {
            let actual_index = app.scroll_offset + index;
            if actual_index < app.displayed_processes.len() {
                let pid = app.displayed_processes[actual_index].pid;
                Some(UIElement::ProcessRow { index, pid })
            } else {
                None
            }
        }
        other => other,
    };

    // Handle the action based on element type
    if let Some(element) = element {
        match (&element, action) {
            // CPU meter click - cycle meter mode
            (UIElement::CpuMeter(_), UIAction::Click) => {
                app.config.cpu_meter_mode = app.config.cpu_meter_mode.next();
                app.save_config();
            }

            // Memory meter click - cycle meter mode
            (UIElement::MemoryMeter, UIAction::Click) => {
                app.config.memory_meter_mode = app.config.memory_meter_mode.next();
                app.save_config();
            }

            // Swap meter click - cycle meter mode (shares with memory)
            (UIElement::SwapMeter, UIAction::Click) => {
                app.config.memory_meter_mode = app.config.memory_meter_mode.next();
                app.save_config();
            }

            // Column header clicks - sort
            (UIElement::ColumnHeader(col), UIAction::Click) => {
                if app.sort_column == *col {
                    app.sort_ascending = !app.sort_ascending;
                } else {
                    app.sort_column = *col;
                    app.sort_ascending = false;
                }
                app.update_displayed_processes();
            }

            // Process row single click - select
            (UIElement::ProcessRow { index, .. }, UIAction::Click) => {
                let actual_index = app.scroll_offset + index;
                if actual_index < app.displayed_processes.len() {
                    app.selected_index = actual_index;
                }
            }

            // Process row double click - open process info, or toggle tag branch in tree mode
            (UIElement::ProcessRow { index, pid }, UIAction::DoubleClick) => {
                let actual_index = app.scroll_offset + index;
                if actual_index < app.displayed_processes.len() {
                    app.selected_index = actual_index;
                    if app.tree_view {
                        // In tree mode, double-click toggles tag for entire branch
                        app.toggle_tag_branch(*pid);
                    } else {
                        // In normal mode, open process info dialog
                        app.enter_process_info_mode();
                    }
                }
            }

            // Process row right click - tag process
            (UIElement::ProcessRow { index, pid }, UIAction::RightClick) => {
                let actual_index = app.scroll_offset + index;
                if actual_index < app.displayed_processes.len() {
                    app.selected_index = actual_index;
                    // Toggle tag on the process
                    if app.tagged_pids.contains(pid) {
                        app.tagged_pids.remove(pid);
                    } else {
                        app.tagged_pids.insert(*pid);
                    }
                }
            }

            // Process row middle click - kill process
            (UIElement::ProcessRow { index, pid: _ }, UIAction::MiddleClick) => {
                let actual_index = app.scroll_offset + index;
                if actual_index < app.displayed_processes.len() {
                    app.selected_index = actual_index;
                    // Open kill dialog
                    app.enter_kill_mode();
                }
            }

            // Function key click - trigger the key
            (UIElement::FunctionKey(key), UIAction::Click) => {
                handle_function_key(app, *key);
            }

            // Header area double-click - toggle header visibility
            (UIElement::Header, UIAction::DoubleClick) => {
                app.show_header = !app.show_header;
            }

            // Footer area double-click - open setup
            (UIElement::Footer, UIAction::DoubleClick) => {
                app.view_mode = ViewMode::Setup;
            }

            _ => {}
        }
    }
}

/// Handle function key press (F1-F10) - delegates to App::handle_function_key
fn handle_function_key(app: &mut App, key: u8) {
    app.handle_function_key(key);
}
