//! Color scheme system for htop-win
//! Provides 8 different color themes matching htop

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// Available color schemes
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

/// Complete color theme definition
#[derive(Debug, Clone)]
pub struct Theme {
    // CPU meter colors (usage thresholds)
    pub cpu_low: Color,      // < 50%
    pub cpu_mid: Color,      // 50-80%
    pub cpu_high: Color,     // > 80%

    // Memory meter colors
    pub mem_low: Color,
    pub mem_mid: Color,
    pub mem_high: Color,

    // Swap meter colors
    pub swap_low: Color,
    pub swap_mid: Color,
    pub swap_high: Color,

    // UI elements
    pub border: Color,
    pub border_highlight: Color,
    pub text: Color,
    pub text_dim: Color,
    pub label: Color,

    // Selection and highlighting
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub search_match: Color,
    pub tagged: Color,

    // Process status colors
    pub status_running: Color,
    pub status_sleeping: Color,
    pub status_disk_wait: Color,
    pub status_zombie: Color,
    pub status_stopped: Color,

    // Header elements
    pub header_key_bg: Color,
    pub header_key_fg: Color,

    // Special highlighting
    pub new_process: Color,
    pub dying_process: Color,
    pub large_number: Color,
    pub basename_highlight: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::default_theme()
    }
}

impl Theme {
    /// Default htop-like theme
    pub fn default_theme() -> Self {
        Self {
            cpu_low: Color::Green,
            cpu_mid: Color::Yellow,
            cpu_high: Color::Red,

            mem_low: Color::Green,
            mem_mid: Color::Yellow,
            mem_high: Color::Red,

            swap_low: Color::Blue,
            swap_mid: Color::Yellow,
            swap_high: Color::Red,

            border: Color::White,
            border_highlight: Color::Cyan,
            text: Color::White,
            text_dim: Color::DarkGray,
            label: Color::Cyan,

            selection_bg: Color::Cyan,
            selection_fg: Color::Black,
            search_match: Color::Yellow,
            tagged: Color::Yellow,

            status_running: Color::Green,
            status_sleeping: Color::DarkGray,
            status_disk_wait: Color::Yellow,
            status_zombie: Color::Red,
            status_stopped: Color::Cyan,

            header_key_bg: Color::Cyan,
            header_key_fg: Color::Black,

            new_process: Color::Green,
            dying_process: Color::Red,
            large_number: Color::Magenta,
            basename_highlight: Color::Cyan,
        }
    }

    /// Monochrome (no colors) theme
    pub fn monochrome() -> Self {
        Self {
            cpu_low: Color::White,
            cpu_mid: Color::White,
            cpu_high: Color::White,

            mem_low: Color::White,
            mem_mid: Color::White,
            mem_high: Color::White,

            swap_low: Color::White,
            swap_mid: Color::White,
            swap_high: Color::White,

            border: Color::White,
            border_highlight: Color::White,
            text: Color::White,
            text_dim: Color::Gray,
            label: Color::White,

            selection_bg: Color::White,
            selection_fg: Color::Black,
            search_match: Color::White,
            tagged: Color::White,

            status_running: Color::White,
            status_sleeping: Color::Gray,
            status_disk_wait: Color::White,
            status_zombie: Color::White,
            status_stopped: Color::Gray,

            header_key_bg: Color::White,
            header_key_fg: Color::Black,

            new_process: Color::White,
            dying_process: Color::Gray,
            large_number: Color::White,
            basename_highlight: Color::White,
        }
    }

    /// Black on white theme
    pub fn black_on_white() -> Self {
        Self {
            cpu_low: Color::Green,
            cpu_mid: Color::Rgb(200, 150, 0), // Dark yellow
            cpu_high: Color::Red,

            mem_low: Color::Green,
            mem_mid: Color::Rgb(200, 150, 0),
            mem_high: Color::Red,

            swap_low: Color::Blue,
            swap_mid: Color::Rgb(200, 150, 0),
            swap_high: Color::Red,

            border: Color::Black,
            border_highlight: Color::Blue,
            text: Color::Black,
            text_dim: Color::DarkGray,
            label: Color::Blue,

            selection_bg: Color::Blue,
            selection_fg: Color::White,
            search_match: Color::Rgb(200, 150, 0),
            tagged: Color::Magenta,

            status_running: Color::Green,
            status_sleeping: Color::DarkGray,
            status_disk_wait: Color::Rgb(200, 150, 0),
            status_zombie: Color::Red,
            status_stopped: Color::Blue,

            header_key_bg: Color::Blue,
            header_key_fg: Color::White,

            new_process: Color::Green,
            dying_process: Color::Red,
            large_number: Color::Magenta,
            basename_highlight: Color::Blue,
        }
    }

    /// Light terminal theme
    pub fn light_terminal() -> Self {
        Self {
            cpu_low: Color::Rgb(0, 150, 0),    // Dark green
            cpu_mid: Color::Rgb(180, 140, 0),  // Dark yellow
            cpu_high: Color::Rgb(180, 0, 0),   // Dark red

            mem_low: Color::Rgb(0, 150, 0),
            mem_mid: Color::Rgb(180, 140, 0),
            mem_high: Color::Rgb(180, 0, 0),

            swap_low: Color::Rgb(0, 0, 180),
            swap_mid: Color::Rgb(180, 140, 0),
            swap_high: Color::Rgb(180, 0, 0),

            border: Color::DarkGray,
            border_highlight: Color::Rgb(0, 0, 180),
            text: Color::Black,
            text_dim: Color::DarkGray,
            label: Color::Rgb(0, 0, 180),

            selection_bg: Color::Rgb(0, 0, 180),
            selection_fg: Color::White,
            search_match: Color::Rgb(180, 140, 0),
            tagged: Color::Magenta,

            status_running: Color::Rgb(0, 150, 0),
            status_sleeping: Color::DarkGray,
            status_disk_wait: Color::Rgb(180, 140, 0),
            status_zombie: Color::Rgb(180, 0, 0),
            status_stopped: Color::Rgb(0, 0, 180),

            header_key_bg: Color::Rgb(0, 0, 180),
            header_key_fg: Color::White,

            new_process: Color::Rgb(0, 150, 0),
            dying_process: Color::Rgb(180, 0, 0),
            large_number: Color::Magenta,
            basename_highlight: Color::Rgb(0, 0, 180),
        }
    }

    /// Midnight (dark blue) theme
    pub fn midnight() -> Self {
        Self {
            cpu_low: Color::Rgb(0, 255, 0),    // Bright green
            cpu_mid: Color::Rgb(255, 255, 0),  // Yellow
            cpu_high: Color::Rgb(255, 0, 0),   // Red

            mem_low: Color::Rgb(0, 255, 0),
            mem_mid: Color::Rgb(255, 255, 0),
            mem_high: Color::Rgb(255, 0, 0),

            swap_low: Color::Rgb(100, 100, 255),
            swap_mid: Color::Rgb(255, 255, 0),
            swap_high: Color::Rgb(255, 0, 0),

            border: Color::Rgb(100, 100, 200),
            border_highlight: Color::Rgb(150, 150, 255),
            text: Color::Rgb(200, 200, 255),
            text_dim: Color::Rgb(100, 100, 150),
            label: Color::Rgb(150, 150, 255),

            selection_bg: Color::Rgb(100, 100, 200),
            selection_fg: Color::White,
            search_match: Color::Rgb(255, 255, 0),
            tagged: Color::Rgb(255, 150, 255),

            status_running: Color::Rgb(0, 255, 0),
            status_sleeping: Color::Rgb(100, 100, 150),
            status_disk_wait: Color::Rgb(255, 255, 0),
            status_zombie: Color::Rgb(255, 0, 0),
            status_stopped: Color::Rgb(150, 150, 255),

            header_key_bg: Color::Rgb(100, 100, 200),
            header_key_fg: Color::White,

            new_process: Color::Rgb(0, 255, 0),
            dying_process: Color::Rgb(255, 0, 0),
            large_number: Color::Rgb(255, 150, 255),
            basename_highlight: Color::Rgb(150, 150, 255),
        }
    }

    /// Blacknight (pure black) theme
    pub fn blacknight() -> Self {
        Self {
            cpu_low: Color::Green,
            cpu_mid: Color::Yellow,
            cpu_high: Color::Red,

            mem_low: Color::Green,
            mem_mid: Color::Yellow,
            mem_high: Color::Red,

            swap_low: Color::Rgb(80, 80, 255),
            swap_mid: Color::Yellow,
            swap_high: Color::Red,

            border: Color::Rgb(80, 80, 80),
            border_highlight: Color::Rgb(100, 100, 255),
            text: Color::Rgb(200, 200, 200),
            text_dim: Color::Rgb(100, 100, 100),
            label: Color::Rgb(100, 100, 255),

            selection_bg: Color::Rgb(60, 60, 120),
            selection_fg: Color::White,
            search_match: Color::Yellow,
            tagged: Color::Magenta,

            status_running: Color::Green,
            status_sleeping: Color::Rgb(100, 100, 100),
            status_disk_wait: Color::Yellow,
            status_zombie: Color::Red,
            status_stopped: Color::Rgb(100, 100, 255),

            header_key_bg: Color::Rgb(60, 60, 120),
            header_key_fg: Color::White,

            new_process: Color::Green,
            dying_process: Color::Red,
            large_number: Color::Magenta,
            basename_highlight: Color::Rgb(100, 100, 255),
        }
    }

    /// Broken gray theme
    pub fn broken_gray() -> Self {
        Self {
            cpu_low: Color::Rgb(120, 120, 120),
            cpu_mid: Color::Rgb(180, 180, 180),
            cpu_high: Color::Rgb(240, 240, 240),

            mem_low: Color::Rgb(120, 120, 120),
            mem_mid: Color::Rgb(180, 180, 180),
            mem_high: Color::Rgb(240, 240, 240),

            swap_low: Color::Rgb(100, 100, 120),
            swap_mid: Color::Rgb(160, 160, 180),
            swap_high: Color::Rgb(220, 220, 240),

            border: Color::Rgb(150, 150, 150),
            border_highlight: Color::Rgb(200, 200, 200),
            text: Color::Rgb(220, 220, 220),
            text_dim: Color::Rgb(120, 120, 120),
            label: Color::Rgb(200, 200, 200),

            selection_bg: Color::Rgb(100, 100, 100),
            selection_fg: Color::White,
            search_match: Color::Rgb(255, 255, 200),
            tagged: Color::Rgb(200, 200, 255),

            status_running: Color::Rgb(200, 200, 200),
            status_sleeping: Color::Rgb(100, 100, 100),
            status_disk_wait: Color::Rgb(180, 180, 180),
            status_zombie: Color::Rgb(255, 200, 200),
            status_stopped: Color::Rgb(150, 150, 200),

            header_key_bg: Color::Rgb(100, 100, 100),
            header_key_fg: Color::White,

            new_process: Color::Rgb(200, 255, 200),
            dying_process: Color::Rgb(255, 200, 200),
            large_number: Color::Rgb(200, 200, 255),
            basename_highlight: Color::Rgb(200, 200, 200),
        }
    }

    /// Nord color palette theme
    pub fn nord() -> Self {
        // Nord palette colors
        let nord0 = Color::Rgb(46, 52, 64);     // Polar Night
        let nord3 = Color::Rgb(76, 86, 106);    // Polar Night (lighter)
        let nord4 = Color::Rgb(216, 222, 233);  // Snow Storm
        let nord6 = Color::Rgb(236, 239, 244);  // Snow Storm (brightest)
        let nord7 = Color::Rgb(143, 188, 187);  // Frost (teal)
        let nord8 = Color::Rgb(136, 192, 208);  // Frost (light blue)
        let nord9 = Color::Rgb(129, 161, 193);  // Frost (blue)
        let nord10 = Color::Rgb(94, 129, 172);  // Frost (dark blue)
        let nord11 = Color::Rgb(191, 97, 106);  // Aurora (red)
        let nord12 = Color::Rgb(208, 135, 112); // Aurora (orange)
        let nord13 = Color::Rgb(235, 203, 139); // Aurora (yellow)
        let nord14 = Color::Rgb(163, 190, 140); // Aurora (green)
        let nord15 = Color::Rgb(180, 142, 173); // Aurora (purple)

        Self {
            cpu_low: nord14,
            cpu_mid: nord13,
            cpu_high: nord11,

            mem_low: nord14,
            mem_mid: nord13,
            mem_high: nord11,

            swap_low: nord9,
            swap_mid: nord13,
            swap_high: nord11,

            border: nord3,
            border_highlight: nord8,
            text: nord4,
            text_dim: nord3,
            label: nord8,

            selection_bg: nord10,
            selection_fg: nord6,
            search_match: nord13,
            tagged: nord15,

            status_running: nord14,
            status_sleeping: nord3,
            status_disk_wait: nord12,
            status_zombie: nord11,
            status_stopped: nord9,

            header_key_bg: nord10,
            header_key_fg: nord6,

            new_process: nord14,
            dying_process: nord11,
            large_number: nord15,
            basename_highlight: nord7,
        }
    }

    /// Get CPU color based on usage percentage
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
}
