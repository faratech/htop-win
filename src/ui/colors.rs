//! Color scheme system for htop-win
//! Provides 8 different color themes matching htop exactly

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// Available color schemes (matching htop exactly)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ColorScheme {
    #[default]
    Default,
    Monochrome,
    BlackOnWhite,
    LightTerminal,
    Midnight,
    Blacknight,
    BrokenGray,
    Nord,
}

impl ColorScheme {
    /// Get all available color schemes
    pub fn all() -> &'static [ColorScheme] {
        &[
            ColorScheme::Default,
            ColorScheme::Monochrome,
            ColorScheme::BlackOnWhite,
            ColorScheme::LightTerminal,
            ColorScheme::Midnight,
            ColorScheme::Blacknight,
            ColorScheme::BrokenGray,
            ColorScheme::Nord,
        ]
    }

    /// Get the display name of the color scheme
    pub fn name(&self) -> &'static str {
        match self {
            ColorScheme::Default => "Default",
            ColorScheme::Monochrome => "Monochrome",
            ColorScheme::BlackOnWhite => "Black on White",
            ColorScheme::LightTerminal => "Light Terminal",
            ColorScheme::Midnight => "Midnight",
            ColorScheme::Blacknight => "Blacknight",
            ColorScheme::BrokenGray => "Broken Gray",
            ColorScheme::Nord => "Nord",
        }
    }

    /// Get the theme for this color scheme
    pub fn theme(&self) -> Theme {
        match self {
            ColorScheme::Default => Theme::default_theme(),
            ColorScheme::Monochrome => Theme::monochrome(),
            ColorScheme::BlackOnWhite => Theme::black_on_white(),
            ColorScheme::LightTerminal => Theme::light_terminal(),
            ColorScheme::Midnight => Theme::midnight(),
            ColorScheme::Blacknight => Theme::blacknight(),
            ColorScheme::BrokenGray => Theme::broken_gray(),
            ColorScheme::Nord => Theme::nord(),
        }
    }
}

/// Complete color theme definition matching htop's ColorElements
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Theme {
    // === Base colors ===
    pub reset_color: Color,
    pub default_color: Color,
    pub background: Color,

    // === Function bar (footer) ===
    pub function_bar_bg: Color,
    pub function_bar_fg: Color,
    pub function_key: Color,

    // === Panel/header colors ===
    pub header_bg: Color,
    pub header_fg: Color,

    // === Selection colors ===
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub selection_follow_bg: Color,
    pub selection_follow_fg: Color,

    // === Search/filter ===
    pub search_match: Color,
    pub failed_search: Color,

    // === Meter colors ===
    pub meter_text: Color,
    pub meter_value: Color,
    pub meter_value_error: Color,
    pub meter_value_ok: Color,
    pub meter_value_warn: Color,
    pub meter_shadow: Color,
    pub meter_label: Color,

    // === CPU bar multi-segment colors (htop style) ===
    pub cpu_normal: Color,   // User time (green)
    pub cpu_nice: Color,     // Nice time (blue)
    pub cpu_system: Color,   // System time (red)
    pub cpu_iowait: Color,   // IO wait (gray)
    pub cpu_irq: Color,      // IRQ (yellow)
    pub cpu_softirq: Color,  // Soft IRQ (magenta)
    pub cpu_steal: Color,    // Steal (cyan)
    pub cpu_guest: Color,    // Guest (cyan)

    // === Memory bar multi-segment colors (htop style) ===
    pub memory_used: Color,       // Used memory (green)
    pub memory_buffers: Color,    // Buffers (blue)
    pub memory_shared: Color,     // Shared (magenta)
    pub memory_cache: Color,      // Cache (yellow)
    pub memory_compressed: Color, // Compressed (gray)

    // === Swap bar colors ===
    pub swap: Color,           // Swap used (red)
    pub swap_cache: Color,     // Swap cache (yellow)
    pub swap_frontswap: Color, // Frontswap (gray)

    // === Graph colors ===
    pub graph_1: Color,
    pub graph_2: Color,

    // === Process display colors ===
    pub process: Color,                // Normal process text
    pub process_shadow: Color,         // Dimmed process
    pub process_tag: Color,            // Tagged process
    pub process_megabytes: Color,      // Memory values < 1GB (cyan)
    pub process_gigabytes: Color,      // Memory values >= 1GB (green)
    pub process_basename: Color,       // Command basename highlight
    pub process_tree: Color,           // Tree lines
    pub process_run_state: Color,      // Running state
    pub process_d_state: Color,        // Disk wait state
    pub process_high_priority: Color,  // High priority (negative nice)
    pub process_low_priority: Color,   // Low priority (positive nice)
    pub process_new: Color,            // Newly created process
    pub process_tomb: Color,           // Dying/zombie process
    pub process_thread: Color,         // Thread coloring
    pub process_thread_basename: Color,
    pub process_comm: Color,           // Command name
    pub process_priv: Color,           // Privileged process

    // === Task info colors ===
    pub tasks_running: Color,

    // === Load average colors ===
    pub load_average_one: Color,
    pub load_average_five: Color,
    pub load_average_fifteen: Color,
    pub load: Color,

    // === Time/date colors ===
    pub uptime: Color,
    pub clock: Color,
    pub date: Color,
    pub hostname: Color,
    pub battery: Color,

    // === Large number highlighting ===
    pub large_number: Color,

    // === Help colors ===
    pub help_bold: Color,
    pub help_shadow: Color,

    // === Bar border ===
    pub bar_border: Color,
    pub bar_shadow: Color,

    // === Checkbox ===
    pub check_box: Color,
    pub check_mark: Color,
    pub check_text: Color,

    // === LED ===
    pub led_color: Color,

    // === Failed read ===
    pub failed_read: Color,

    // === Paused indicator ===
    pub paused: Color,

    // === General UI ===
    pub border: Color,
    pub text: Color,
    pub text_dim: Color,
    pub label: Color,

    // === Header function key bar (for compatibility) ===
    pub header_key_bg: Color,
    pub header_key_fg: Color,

    // === Aliases for simpler usage ===
    // These map to the multi-segment colors for threshold-based coloring
    pub cpu_low: Color,
    pub cpu_mid: Color,
    pub cpu_high: Color,
    pub mem_low: Color,
    pub mem_mid: Color,
    pub mem_high: Color,
    pub swap_low: Color,
    pub swap_mid: Color,
    pub swap_high: Color,

    // === Process column colors ===
    pub pid_color: Color,
    pub user_color: Color,
    pub priority_color: Color,
    pub threads_color: Color,
    pub time_color: Color,

    // === Status colors ===
    pub status_running: Color,
    pub status_sleeping: Color,
    pub status_disk_wait: Color,
    pub status_zombie: Color,
    pub status_stopped: Color,

    // === Highlight colors ===
    pub tagged: Color,
    pub new_process: Color,
    pub dying_process: Color,
    pub basename_highlight: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::default_theme()
    }
}

impl Theme {
    /// Default htop theme - exact colors from htop's COLORSCHEME_DEFAULT
    pub fn default_theme() -> Self {
        Self {
            // Base colors
            reset_color: Color::White,
            default_color: Color::White,
            background: Color::Black,

            // Function bar - ColorPair(Black, Cyan)
            function_bar_bg: Color::Cyan,
            function_bar_fg: Color::Black,
            function_key: Color::White,

            // Panel header - ColorPair(Black, Green)
            header_bg: Color::Green,
            header_fg: Color::Black,

            // Selection - ColorPair(Black, Cyan)
            selection_bg: Color::Cyan,
            selection_fg: Color::Black,
            selection_follow_bg: Color::Yellow,
            selection_follow_fg: Color::Black,

            // Search
            search_match: Color::Yellow,
            failed_search: Color::Red,

            // Meters
            meter_text: Color::Cyan,
            meter_value: Color::Cyan,
            meter_value_error: Color::Red,
            meter_value_ok: Color::Green,
            meter_value_warn: Color::Yellow,
            meter_shadow: Color::DarkGray,
            meter_label: Color::Cyan,

            // CPU bar segments - exact htop colors
            cpu_normal: Color::Green,    // ColorPair(Green, Black)
            cpu_nice: Color::Blue,       // A_BOLD | ColorPair(Blue, Black)
            cpu_system: Color::Red,      // ColorPair(Red, Black)
            cpu_iowait: Color::DarkGray, // A_BOLD | ColorPairGrayBlack
            cpu_irq: Color::Yellow,      // ColorPair(Yellow, Black)
            cpu_softirq: Color::Magenta, // ColorPair(Magenta, Black)
            cpu_steal: Color::Cyan,      // ColorPair(Cyan, Black)
            cpu_guest: Color::Cyan,      // ColorPair(Cyan, Black)

            // Memory bar segments - exact htop colors
            memory_used: Color::Green,        // ColorPair(Green, Black)
            memory_buffers: Color::Blue,      // A_BOLD | ColorPair(Blue, Black)
            memory_shared: Color::Magenta,    // ColorPair(Magenta, Black)
            memory_cache: Color::Yellow,      // ColorPair(Yellow, Black)
            memory_compressed: Color::DarkGray, // A_BOLD | ColorPairGrayBlack

            // Swap - exact htop colors
            swap: Color::Red,              // ColorPair(Red, Black)
            swap_cache: Color::Yellow,     // ColorPair(Yellow, Black)
            swap_frontswap: Color::DarkGray, // A_BOLD | ColorPairGrayBlack

            // Graph
            graph_1: Color::Cyan,
            graph_2: Color::Cyan,

            // Process colors - exact htop colors
            process: Color::White,
            process_shadow: Color::DarkGray,
            process_tag: Color::Yellow,          // A_BOLD | ColorPair(Yellow, Black)
            process_megabytes: Color::Cyan,      // ColorPair(Cyan, Black)
            process_gigabytes: Color::Green,     // ColorPair(Green, Black)
            process_basename: Color::Cyan,       // A_BOLD | ColorPair(Cyan, Black)
            process_tree: Color::Cyan,           // ColorPair(Cyan, Black)
            process_run_state: Color::Green,     // ColorPair(Green, Black)
            process_d_state: Color::Red,         // A_BOLD | ColorPair(Red, Black)
            process_high_priority: Color::Red,   // ColorPair(Red, Black)
            process_low_priority: Color::Green,  // ColorPair(Green, Black)
            process_new: Color::Green,           // ColorPair(Black, Green) - bg
            process_tomb: Color::Red,            // ColorPair(Black, Red) - bg
            process_thread: Color::Green,        // ColorPair(Green, Black)
            process_thread_basename: Color::Green,
            process_comm: Color::Magenta,        // ColorPair(Magenta, Black)
            process_priv: Color::Magenta,        // ColorPair(Magenta, Black)

            // Tasks
            tasks_running: Color::Green,         // A_BOLD | ColorPair(Green, Black)

            // Load average
            load_average_one: Color::White,
            load_average_five: Color::Cyan,
            load_average_fifteen: Color::Cyan,
            load: Color::White,

            // Time/date
            uptime: Color::Cyan,
            clock: Color::White,
            date: Color::White,
            hostname: Color::White,
            battery: Color::Cyan,

            // Large number
            large_number: Color::Red,  // A_BOLD | ColorPair(Red, Black)

            // Help
            help_bold: Color::Cyan,
            help_shadow: Color::DarkGray,

            // Bar
            bar_border: Color::White,
            bar_shadow: Color::DarkGray,

            // Checkbox
            check_box: Color::Cyan,
            check_mark: Color::White,
            check_text: Color::White,

            // LED
            led_color: Color::Green,

            // Failed read
            failed_read: Color::Red,

            // Paused
            paused: Color::Yellow,

            // General UI
            border: Color::White,
            text: Color::White,
            text_dim: Color::DarkGray,
            label: Color::Cyan,

            // Header key bar
            header_key_bg: Color::Cyan,
            header_key_fg: Color::Black,

            // Threshold-based coloring (for simple usage)
            cpu_low: Color::Green,
            cpu_mid: Color::Yellow,
            cpu_high: Color::Red,
            mem_low: Color::Green,
            mem_mid: Color::Yellow,
            mem_high: Color::Red,
            swap_low: Color::Red,
            swap_mid: Color::Red,
            swap_high: Color::Red,

            // Process columns
            pid_color: Color::Cyan,
            user_color: Color::LightCyan,
            priority_color: Color::Green,
            threads_color: Color::Magenta,
            time_color: Color::Cyan,

            // Status colors
            status_running: Color::Green,
            status_sleeping: Color::DarkGray,
            status_disk_wait: Color::Red,
            status_zombie: Color::Red,
            status_stopped: Color::Cyan,

            // Highlight colors
            tagged: Color::Yellow,
            new_process: Color::Green,
            dying_process: Color::Red,
            basename_highlight: Color::Cyan,
        }
    }

    /// Monochrome theme - all White/DarkGray, no colors
    pub fn monochrome() -> Self {
        let w = Color::White;
        let g = Color::DarkGray;
        Self {
            function_bar_bg: w, function_bar_fg: Color::Black, function_key: w,
            header_bg: w, header_fg: Color::Black,
            selection_bg: w, selection_fg: Color::Black, selection_follow_bg: w, selection_follow_fg: Color::Black,
            search_match: w, failed_search: w,
            meter_text: w, meter_value: w, meter_value_error: w, meter_value_ok: w, meter_value_warn: w, meter_label: w,
            cpu_normal: w, cpu_nice: w, cpu_system: w, cpu_iowait: g, cpu_irq: w, cpu_softirq: w, cpu_steal: g, cpu_guest: g,
            memory_used: w, memory_buffers: w, memory_shared: w, memory_cache: w,
            swap: w, swap_cache: w,
            graph_1: w, graph_2: w,
            process: w, process_tag: w, process_megabytes: w, process_gigabytes: w, process_basename: w,
            process_tree: w, process_run_state: w, process_d_state: w, process_high_priority: w,
            process_low_priority: g, process_new: w, process_tomb: g, process_thread: w, process_thread_basename: w,
            process_comm: w, process_priv: w,
            tasks_running: w,
            load_average_one: w, load_average_five: w, load_average_fifteen: g, load: w,
            uptime: w, clock: w, date: w, hostname: w, battery: w,
            large_number: w, help_bold: w, bar_border: w, check_box: w, check_mark: w, check_text: w,
            led_color: w, failed_read: w, paused: w,
            border: w, text: w, label: w,
            header_key_bg: w, header_key_fg: Color::Black,
            cpu_low: w, cpu_mid: w, cpu_high: w, mem_low: w, mem_mid: w, mem_high: w,
            swap_low: w, swap_mid: w, swap_high: w,
            pid_color: w, user_color: w, priority_color: w, threads_color: w, time_color: w,
            status_running: w, status_sleeping: g, status_disk_wait: w, status_zombie: w, status_stopped: g,
            tagged: w, new_process: w, dying_process: g, basename_highlight: w,
            ..Self::default_theme()
        }
    }

    /// Black on White theme - exact colors from htop's COLORSCHEME_BLACKONWHITE
    /// Key: White background, Blue BASENAME, Green TREE
    pub fn black_on_white() -> Self {
        let b = Color::Black;
        let w = Color::White;
        let bl = Color::Blue;
        let g = Color::Green;
        let y = Color::Yellow;
        let c = Color::Cyan;
        let m = Color::Magenta;
        Self {
            // Base - ColorPair(Black, White)
            reset_color: b, default_color: b, background: w, function_key: b,
            // Function bar - ColorPair(Black, Cyan)
            function_bar_bg: c, function_bar_fg: b,
            // Header - ColorPair(Black, Green)
            header_bg: g, header_fg: b, header_key_bg: c, header_key_fg: b,
            // Selection - ColorPair(Black, Cyan), follow = Yellow
            selection_bg: c, selection_fg: b, selection_follow_bg: y, selection_follow_fg: b,
            // Search
            search_match: y, failed_search: Color::Red,
            // Meters - ColorPair(Blue/Black, White)
            meter_text: bl, meter_value: b, meter_shadow: bl, meter_label: bl,
            meter_value_error: Color::Red, meter_value_ok: g, meter_value_warn: y,
            // CPU - on White bg
            cpu_normal: g, cpu_nice: c, cpu_system: Color::Red, cpu_iowait: b,
            cpu_irq: bl, cpu_softirq: bl, cpu_steal: c, cpu_guest: c,
            // Memory - on White bg
            memory_used: g, memory_buffers: c, memory_shared: m, memory_cache: y, memory_compressed: b,
            // Swap
            swap: Color::Red, swap_cache: y, swap_frontswap: b,
            // Graph
            graph_1: bl, graph_2: bl,
            // Process - htop: PROCESS_TAG = ColorPair(White, Blue) = White on Blue
            process: b, process_shadow: b, process_tag: w,
            process_megabytes: bl, process_gigabytes: g,
            process_basename: bl,    // htop: ColorPair(Blue, White)
            process_tree: g,         // htop: ColorPair(Green, White)
            process_run_state: g, process_d_state: Color::Red,
            process_high_priority: Color::Red, process_low_priority: g,
            process_new: g, process_tomb: Color::Red,
            process_thread: bl, process_thread_basename: bl,
            process_comm: m, process_priv: m,
            // Tasks
            tasks_running: g,
            // Load average - all Black on White
            load_average_one: b, load_average_five: b, load_average_fifteen: b, load: b,
            // Time/date
            uptime: y, clock: b, date: b, hostname: b, battery: y,
            // Large number
            large_number: Color::Red,
            // Help
            help_bold: bl, help_shadow: b,
            // Bar
            bar_border: bl, bar_shadow: b,
            // Checkbox
            check_box: bl, check_mark: b, check_text: b,
            // LED
            led_color: g,
            // Failed/paused
            failed_read: Color::Red, paused: y,
            // General UI
            border: b, text: b, text_dim: b, label: bl,
            // Threshold colors
            cpu_low: g, cpu_mid: y, cpu_high: Color::Red,
            mem_low: g, mem_mid: y, mem_high: Color::Red,
            swap_low: Color::Red, swap_mid: Color::Red, swap_high: Color::Red,
            // Process columns
            pid_color: bl, user_color: b, priority_color: g, threads_color: m, time_color: bl,
            // Status
            status_running: g, status_sleeping: b, status_disk_wait: Color::Red,
            status_zombie: Color::Red, status_stopped: bl,
            // Highlight - process_tag uses Blue bg
            tagged: bl, new_process: g, dying_process: Color::Red, basename_highlight: bl,
        }
    }

    /// Light Terminal theme - uses transparent background with visible text colors
    /// Key differences from BlackOnWhite:
    /// - BASENAME = Green, TREE = Blue (swapped)
    /// - Uses terminal's own background (transparent)
    /// - Text colors adjusted to be visible on both light and dark backgrounds
    pub fn light_terminal() -> Self {
        let g = Color::Green;
        let y = Color::Yellow;
        let c = Color::Cyan;
        let m = Color::Magenta;
        let dg = Color::DarkGray;
        let lg = Color::LightGreen;
        let lc = Color::LightCyan;
        let lb = Color::LightBlue;
        Self {
            // Base - transparent background
            reset_color: lc, default_color: lc, background: Color::Reset, function_key: lc,
            // Function bar - visible on both backgrounds
            function_bar_bg: c, function_bar_fg: Color::Black,
            // Header
            header_bg: g, header_fg: Color::Black, header_key_bg: c, header_key_fg: Color::Black,
            // Selection
            selection_bg: c, selection_fg: Color::Black, selection_follow_bg: y, selection_follow_fg: Color::Black,
            // Search
            search_match: y, failed_search: Color::Red,
            // Meters - use light colors visible on dark bg
            meter_text: lb, meter_value: lc, meter_shadow: dg, meter_label: lb,
            meter_value_error: Color::Red, meter_value_ok: lg, meter_value_warn: y,
            // CPU
            cpu_normal: lg, cpu_nice: lc, cpu_system: Color::Red, cpu_iowait: dg,
            cpu_irq: lb, cpu_softirq: lb, cpu_steal: dg, cpu_guest: dg,
            // Memory
            memory_used: lg, memory_buffers: lc, memory_shared: m, memory_cache: y, memory_compressed: dg,
            // Swap
            swap: Color::Red, swap_cache: y, swap_frontswap: dg,
            // Graph
            graph_1: lc, graph_2: c,
            // Process - use light colors for visibility
            process: lc, process_shadow: dg, process_tag: Color::White,
            process_megabytes: lb, process_gigabytes: lg,
            process_basename: lg,    // Green basename (KEY DIFFERENCE from BlackOnWhite)
            process_tree: lb,        // Blue tree (KEY DIFFERENCE from BlackOnWhite)
            process_run_state: lg, process_d_state: Color::Red,
            process_high_priority: Color::Red, process_low_priority: lg,
            process_new: lg, process_tomb: Color::Red,
            process_thread: lb, process_thread_basename: lb,
            process_comm: m, process_priv: m,
            // Tasks
            tasks_running: lg,
            // Load average - use visible colors
            load_average_one: lc, load_average_five: lc, load_average_fifteen: dg, load: lc,
            // Time/date
            uptime: y, clock: lc, date: lc, hostname: lc, battery: y,
            // Large number
            large_number: Color::Red,
            // Help
            help_bold: lb, help_shadow: dg,
            // Bar
            bar_border: lb, bar_shadow: dg,
            // Checkbox
            check_box: lb, check_mark: lc, check_text: lc,
            // LED
            led_color: lg,
            // Failed/paused
            failed_read: Color::Red, paused: y,
            // General UI - use light colors for visibility
            border: dg, text: lc, text_dim: dg, label: lb,
            // Threshold colors
            cpu_low: lg, cpu_mid: y, cpu_high: Color::Red,
            mem_low: lg, mem_mid: y, mem_high: Color::Red,
            swap_low: Color::Red, swap_mid: Color::Red, swap_high: Color::Red,
            // Process columns - use light colors
            pid_color: lb, user_color: lc, priority_color: lg, threads_color: m, time_color: lb,
            // Status
            status_running: lg, status_sleeping: dg, status_disk_wait: Color::Red,
            status_zombie: Color::Red, status_stopped: lc,
            // Highlight
            tagged: lb, new_process: lg, dying_process: Color::Red, basename_highlight: lg,
        }
    }

    /// Midnight theme - exact colors from htop's COLORSCHEME_MIDNIGHT
    /// Blue background with Cyan/White text
    pub fn midnight() -> Self {
        let b = Color::Black;
        let w = Color::White;
        let bl = Color::Blue;
        let c = Color::Cyan;
        let g = Color::Green;
        let y = Color::Yellow;
        let r = Color::Red;
        let m = Color::Magenta;
        Self {
            // Base - ColorPair(White, Blue)
            reset_color: w, default_color: w, background: bl, function_key: w,
            // Function bar - ColorPair(Black, Cyan)
            function_bar_bg: c, function_bar_fg: b,
            // Header - ColorPair(Black, Cyan)
            header_bg: c, header_fg: b, header_key_bg: c, header_key_fg: b,
            // Selection - ColorPair(Black, White), follow = Yellow
            selection_bg: w, selection_fg: b, selection_follow_bg: y, selection_follow_fg: b,
            // Search
            search_match: y, failed_search: r,
            // Meters - Cyan on Blue
            meter_text: c, meter_value: c, meter_shadow: c, meter_label: c,
            meter_value_error: r, meter_value_ok: g, meter_value_warn: y,
            // CPU - on Blue bg
            cpu_normal: g, cpu_nice: c, cpu_system: r, cpu_iowait: b,
            cpu_irq: b, cpu_softirq: b, cpu_steal: w, cpu_guest: w,
            // Memory - on Blue bg
            memory_used: g, memory_buffers: c, memory_shared: m, memory_cache: y, memory_compressed: b,
            // Swap
            swap: r, swap_cache: y, swap_frontswap: b,
            // Graph
            graph_1: c, graph_2: c,
            // Process - ColorPair(White, Blue) base
            process: w, process_shadow: b, process_tag: y,
            process_megabytes: c, process_gigabytes: g,
            process_basename: c, process_tree: c,
            process_run_state: g, process_d_state: r,
            process_high_priority: r, process_low_priority: g,
            process_new: g, process_tomb: r,
            process_thread: g, process_thread_basename: g,
            process_comm: m, process_priv: m,
            // Tasks
            tasks_running: g,
            // Load average
            load_average_one: w, load_average_five: w, load_average_fifteen: b, load: w,
            // Time/date
            uptime: y, clock: w, date: w, hostname: w, battery: y,
            // Large number
            large_number: r,
            // Help
            help_bold: c, help_shadow: b,
            // Bar
            bar_border: y, bar_shadow: c,
            // Checkbox
            check_box: c, check_mark: w, check_text: w,
            // LED
            led_color: g,
            // Failed/paused
            failed_read: r, paused: y,
            // General UI
            border: c, text: w, text_dim: b, label: c,
            // Threshold colors
            cpu_low: g, cpu_mid: y, cpu_high: r,
            mem_low: g, mem_mid: y, mem_high: r,
            swap_low: r, swap_mid: r, swap_high: r,
            // Process columns
            pid_color: c, user_color: w, priority_color: g, threads_color: m, time_color: c,
            // Status
            status_running: g, status_sleeping: b, status_disk_wait: r,
            status_zombie: r, status_stopped: c,
            // Highlight
            tagged: y, new_process: g, dying_process: r, basename_highlight: c,
        }
    }

    /// Blacknight theme - exact colors from htop's COLORSCHEME_BLACKNIGHT
    /// Black background with Cyan/Green text and accents
    pub fn blacknight() -> Self {
        let b = Color::Black;
        let w = Color::White;
        let bl = Color::Blue;
        let c = Color::Cyan;
        let g = Color::Green;
        let y = Color::Yellow;
        let r = Color::Red;
        let m = Color::Magenta;
        let dg = Color::DarkGray;
        Self {
            // Base - ColorPair(Cyan, Black)
            reset_color: c, default_color: c, background: b, function_key: c,
            // Function bar - ColorPair(Black, Green)
            function_bar_bg: g, function_bar_fg: b,
            // Header - ColorPair(Black, Green)
            header_bg: g, header_fg: b, header_key_bg: g, header_key_fg: b,
            // Selection - ColorPair(Black, Cyan), follow = Yellow
            selection_bg: c, selection_fg: b, selection_follow_bg: y, selection_follow_fg: b,
            // Search
            search_match: y, failed_search: r,
            // Meters - Cyan/Green on Black
            meter_text: c, meter_value: g, meter_shadow: dg, meter_label: c,
            meter_value_error: r, meter_value_ok: g, meter_value_warn: y,
            // CPU - on Black bg
            cpu_normal: g, cpu_nice: bl, cpu_system: r, cpu_iowait: y,
            cpu_irq: bl, cpu_softirq: bl, cpu_steal: c, cpu_guest: c,
            // Memory - on Black bg
            memory_used: g, memory_buffers: bl, memory_shared: m, memory_cache: y, memory_compressed: y,
            // Swap
            swap: r, swap_cache: y, swap_frontswap: y,
            // Graph - Green
            graph_1: g, graph_2: g,
            // Process - Cyan base with Green highlights
            process: c, process_shadow: dg, process_tag: y,
            process_megabytes: g, process_gigabytes: y,
            process_basename: g, process_tree: c,
            process_run_state: g, process_d_state: r,
            process_high_priority: r, process_low_priority: g,
            process_new: g, process_tomb: r,
            process_thread: g, process_thread_basename: bl,
            process_comm: m, process_priv: m,
            // Tasks
            tasks_running: g,
            // Load average - all Green
            load_average_one: g, load_average_five: g, load_average_fifteen: g, load: w,
            // Time/date - Green
            uptime: g, clock: g, date: g, hostname: g, battery: g,
            // Large number
            large_number: r,
            // Help
            help_bold: c, help_shadow: dg,
            // Bar - Green border
            bar_border: g, bar_shadow: c,
            // Checkbox - Green
            check_box: g, check_mark: g, check_text: c,
            // LED
            led_color: g,
            // Failed/paused
            failed_read: r, paused: y,
            // General UI - Green/Cyan
            border: g, text: c, text_dim: dg, label: c,
            // Threshold colors
            cpu_low: g, cpu_mid: y, cpu_high: r,
            mem_low: g, mem_mid: y, mem_high: r,
            swap_low: r, swap_mid: r, swap_high: r,
            // Process columns - Green/Cyan
            pid_color: g, user_color: c, priority_color: g, threads_color: m, time_color: g,
            // Status
            status_running: g, status_sleeping: dg, status_disk_wait: r,
            status_zombie: r, status_stopped: c,
            // Highlight
            tagged: y, new_process: g, dying_process: r, basename_highlight: g,
        }
    }

    /// Broken Gray theme - dynamically generated in htop from Default
    /// Replaces GrayBlack with White
    pub fn broken_gray() -> Self {
        let mut theme = Self::default_theme();
        // In htop, BrokenGray replaces ColorPairGrayBlack with White
        theme.meter_shadow = Color::White;
        theme.process_shadow = Color::White;
        theme.cpu_iowait = Color::White;
        theme.memory_compressed = Color::White;
        theme.swap_frontswap = Color::White;
        theme.help_shadow = Color::White;
        theme.bar_shadow = Color::White;
        theme
    }

    /// Nord theme - exact colors from htop's COLORSCHEME_NORD
    /// Uses the Nord color palette (https://www.nordtheme.com/)
    pub fn nord() -> Self {
        // Nord Polar Night (dark backgrounds)
        let n0 = Color::Rgb(46, 52, 64);     // Background
        let n3 = Color::Rgb(76, 86, 106);    // Comments/subtle
        // Nord Snow Storm (light text)
        let n4 = Color::Rgb(216, 222, 233);  // Main text
        let n6 = Color::Rgb(236, 239, 244);  // Bright text
        // Nord Frost (cyan/blue accents)
        let n8 = Color::Rgb(136, 192, 208);  // Cyan (main accent)
        // Nord Aurora (colored accents)
        let n11 = Color::Rgb(191, 97, 106);  // Red
        let n13 = Color::Rgb(235, 203, 139); // Yellow
        let n14 = Color::Rgb(163, 190, 140); // Green
        let n15 = Color::Rgb(180, 142, 173); // Purple
        Self {
            // Base - Nord dark bg with light text
            reset_color: n4, default_color: n4, background: n0, function_key: n4,
            // Function bar - Black on Cyan
            function_bar_bg: n8, function_bar_fg: n0,
            // Header - Black on Cyan
            header_bg: n8, header_fg: n0, header_key_bg: n8, header_key_fg: n0,
            // Selection - Black on Cyan
            selection_bg: n8, selection_fg: n0, selection_follow_bg: n4, selection_follow_fg: n0,
            // Search
            search_match: n13, failed_search: n13,
            // Meters
            meter_text: n4, meter_value: n4, meter_shadow: n3, meter_label: n4,
            meter_value_error: n4, meter_value_ok: n4, meter_value_warn: n4,
            // CPU - htop uses mostly A_NORMAL/A_BOLD for Nord
            cpu_normal: n4, cpu_nice: n4, cpu_system: n13, cpu_iowait: n4,
            cpu_irq: n4, cpu_softirq: n4, cpu_steal: n3, cpu_guest: n3,
            // Memory
            memory_used: n13, memory_buffers: n4, memory_shared: n4, memory_cache: n4, memory_compressed: n3,
            // Swap
            swap: n4, swap_cache: n4, swap_frontswap: n3,
            // Graph
            graph_1: n4, graph_2: n4,
            // Process
            process: n4, process_shadow: n3, process_tag: n8,
            process_megabytes: n6, process_gigabytes: n8,
            process_basename: n4, process_tree: n4,
            process_run_state: n4, process_d_state: n13,
            process_high_priority: n4, process_low_priority: n3,
            process_new: n4, process_tomb: n3,
            process_thread: n4, process_thread_basename: n4,
            process_comm: n4, process_priv: n8,
            // Tasks
            tasks_running: n4,
            // Load average
            load_average_one: n4, load_average_five: n4, load_average_fifteen: n3, load: n4,
            // Time/date
            uptime: n4, clock: n4, date: n4, hostname: n8, battery: n4,
            // Large number
            large_number: n13,
            // Help
            help_bold: n4, help_shadow: n3,
            // Bar
            bar_border: n4, bar_shadow: n3,
            // Checkbox
            check_box: n4, check_mark: n4, check_text: n4,
            // LED
            led_color: n4,
            // Failed/paused
            failed_read: n13, paused: n8,
            // General UI
            border: n3, text: n4, text_dim: n3, label: n8,
            // Threshold colors - using Aurora palette
            cpu_low: n14, cpu_mid: n13, cpu_high: n11,
            mem_low: n14, mem_mid: n13, mem_high: n11,
            swap_low: n4, swap_mid: n4, swap_high: n4,
            // Process columns
            pid_color: n8, user_color: n4, priority_color: n14, threads_color: n15, time_color: n8,
            // Status
            status_running: n14, status_sleeping: n3, status_disk_wait: n13,
            status_zombie: n11, status_stopped: n8,
            // Highlight
            tagged: n15, new_process: n14, dying_process: n11, basename_highlight: n8,
        }
    }

    /// Get CPU color based on usage percentage (for simple threshold-based coloring)
    pub fn cpu_color(&self, percent: f32) -> Color {
        if percent < 50.0 {
            self.cpu_low
        } else if percent < 80.0 {
            self.cpu_mid
        } else {
            self.cpu_high
        }
    }

    /// Get memory color based on usage percentage
    #[allow(dead_code)]
    pub fn mem_color(&self, percent: f32) -> Color {
        if percent < 50.0 {
            self.mem_low
        } else if percent < 80.0 {
            self.mem_mid
        } else {
            self.mem_high
        }
    }

    /// Get swap color based on usage percentage
    #[allow(dead_code)]
    pub fn swap_color(&self, percent: f32) -> Color {
        if percent < 50.0 {
            self.swap_low
        } else if percent < 80.0 {
            self.swap_mid
        } else {
            self.swap_high
        }
    }

    /// Get process status color
    pub fn status_color(&self, status: char) -> Color {
        match status {
            'R' => self.status_running,
            'S' | 'I' => self.status_sleeping,
            'D' => self.status_disk_wait,
            'Z' => self.status_zombie,
            'T' | 't' => self.status_stopped,
            _ => self.text,
        }
    }

    /// Get color for memory size display (htop uses different colors for MB vs GB)
    #[allow(dead_code)]
    pub fn memory_size_color(&self, bytes: u64) -> Color {
        if bytes >= 1_073_741_824 {
            // >= 1GB
            self.process_gigabytes
        } else {
            self.process_megabytes
        }
    }

    /// Get color for process priority
    pub fn priority_color_for_nice(&self, nice: i32) -> Color {
        if nice < 0 {
            self.process_high_priority
        } else if nice > 0 {
            self.process_low_priority
        } else {
            self.text
        }
    }
}
