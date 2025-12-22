//! Application configuration with persistence

use crate::ui::colors::ColorScheme;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// Meter display mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MeterMode {
    #[default]
    Bar,
    Text,
    Graph,
    Hidden,
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // Display settings
    /// Refresh rate in milliseconds
    pub refresh_rate_ms: u64,
    /// Tree view enabled by default
    pub tree_view_default: bool,
    /// Color scheme
    pub color_scheme: ColorScheme,

    // Process display options
    /// Show kernel/system threads
    pub show_kernel_threads: bool,
    /// Show user threads
    pub show_user_threads: bool,
    /// Show full program path
    pub show_program_path: bool,
    /// Highlight running processes
    pub highlight_running: bool,
    /// Highlight large numbers (memory > 1GB, CPU > 50%)
    pub highlight_large_numbers: bool,
    /// Highlight new processes
    pub highlight_new_processes: bool,
    /// Duration to highlight new/dying processes (ms)
    pub highlight_duration_ms: u64,
    /// Highlight program basename in command
    pub highlight_basename: bool,

    // Meter visibility
    pub show_cpu_meters: bool,
    pub show_memory_meter: bool,
    pub show_swap_meter: bool,
    pub show_tasks_meter: bool,
    pub show_uptime_meter: bool,
    pub show_load_average: bool,
    pub show_network_io: bool,
    pub show_disk_io: bool,
    pub show_clock: bool,
    pub show_hostname: bool,
    pub show_battery: bool,

    // Meter modes
    pub cpu_meter_mode: MeterMode,
    pub memory_meter_mode: MeterMode,

    // Column visibility (which columns to show in process list)
    pub visible_columns: Vec<String>,

    // Mouse settings
    pub mouse_enabled: bool,

    // Readonly mode (no kill/nice operations)
    pub readonly: bool,

    // Tree view settings
    /// Default collapsed PIDs (persisted)
    #[serde(skip)]
    pub collapsed_pids: HashSet<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            refresh_rate_ms: 1000,
            tree_view_default: false,
            color_scheme: ColorScheme::Default,

            show_kernel_threads: true,
            show_user_threads: true,
            show_program_path: true,
            highlight_running: true,
            highlight_large_numbers: true,
            highlight_new_processes: true,
            highlight_duration_ms: 3000,
            highlight_basename: true,

            show_cpu_meters: true,
            show_memory_meter: true,
            show_swap_meter: true,
            show_tasks_meter: true,
            show_uptime_meter: true,
            show_load_average: true,
            show_network_io: false,
            show_disk_io: false,
            show_clock: false,
            show_hostname: true,
            show_battery: false,

            cpu_meter_mode: MeterMode::Bar,
            memory_meter_mode: MeterMode::Bar,

            visible_columns: vec![
                "PID".to_string(),
                "USER".to_string(),
                "PRI".to_string(),
                "NI".to_string(),
                "VIRT".to_string(),
                "RES".to_string(),
                "SHR".to_string(),
                "S".to_string(),
                "CPU%".to_string(),
                "MEM%".to_string(),
                "TIME+".to_string(),
                "Command".to_string(),
            ],

            mouse_enabled: true,
            readonly: false,
            collapsed_pids: HashSet::new(),
        }
    }
}

impl Config {
    /// Get the config file path
    pub fn config_path() -> Option<PathBuf> {
        ProjectDirs::from("", "", "htop-win").map(|dirs| {
            dirs.config_dir().join("config.json")
        })
    }

    /// Load configuration from file, or return defaults
    pub fn load() -> Self {
        if let Some(path) = Self::config_path() {
            if path.exists() {
                match fs::read_to_string(&path) {
                    Ok(content) => {
                        match serde_json::from_str(&content) {
                            Ok(config) => return config,
                            Err(e) => {
                                eprintln!("Warning: Failed to parse config: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to read config: {}", e);
                    }
                }
            }
        }
        Self::default()
    }

    /// Save configuration to file
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(path) = Self::config_path() {
            // Ensure directory exists
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }

            let content = serde_json::to_string_pretty(self)?;
            fs::write(&path, content)?;
        }
        Ok(())
    }

    /// Check if a column should be visible
    pub fn is_column_visible(&self, column: &str) -> bool {
        self.visible_columns.iter().any(|c| c == column)
    }

    /// Toggle a column's visibility
    pub fn toggle_column(&mut self, column: &str) {
        if let Some(pos) = self.visible_columns.iter().position(|c| c == column) {
            self.visible_columns.remove(pos);
        } else {
            self.visible_columns.push(column.to_string());
        }
    }

    /// Get the theme for the current color scheme
    pub fn theme(&self) -> crate::ui::colors::Theme {
        self.color_scheme.theme()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.refresh_rate_ms, 1000);
        assert!(!config.tree_view_default);
        assert!(config.show_cpu_meters);
    }

    #[test]
    fn test_serialization() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let loaded: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.refresh_rate_ms, config.refresh_rate_ms);
    }
}
