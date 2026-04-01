//! Application configuration with persistence

use crate::json::{self, Value};
use crate::ui::colors::ColorScheme;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Meter display mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MeterMode {
    #[default]
    Bar,
    Text,
    Graph,
    Hidden,
}

impl MeterMode {
    fn as_str(self) -> &'static str {
        match self {
            MeterMode::Bar => "Bar",
            MeterMode::Text => "Text",
            MeterMode::Graph => "Graph",
            MeterMode::Hidden => "Hidden",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "Text" => MeterMode::Text,
            "Graph" => MeterMode::Graph,
            "Hidden" => MeterMode::Hidden,
            _ => MeterMode::Bar,
        }
    }
}

impl MeterMode {
    /// Cycle to the next meter mode
    pub fn next(self) -> Self {
        match self {
            MeterMode::Bar => MeterMode::Text,
            MeterMode::Text => MeterMode::Graph,
            MeterMode::Graph => MeterMode::Hidden,
            MeterMode::Hidden => MeterMode::Bar,
        }
    }
}

/// Application configuration
#[derive(Debug, Clone)]
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

    // Screen tabs (like htop's Main/I/O tabs)
    pub screen_tabs: Option<Vec<crate::app::ScreenTab>>,

    // Mouse settings
    pub mouse_enabled: bool,

    // Readonly mode (no kill/priority operations)
    pub readonly: bool,

    // Confirmation dialogs
    /// Show confirmation dialog before killing processes
    pub confirm_kill: bool,

}

impl Default for Config {
    fn default() -> Self {
        Self {
            refresh_rate_ms: 1500,  // htop default: 15 tenths of a second
            tree_view_default: false,
            color_scheme: ColorScheme::Default,

            show_kernel_threads: true,
            show_user_threads: true,
            show_program_path: false,
            highlight_running: true,
            highlight_large_numbers: true,
            highlight_new_processes: true,
            highlight_duration_ms: 3000,
            highlight_basename: false,  // htop default: highlightBaseName = false

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
                "CLASS".to_string(),
                "THR".to_string(),
                "VIRT".to_string(),
                "RES".to_string(),
                "SHR".to_string(),
                "S".to_string(),
                "CPU%".to_string(),
                "MEM%".to_string(),
                "TIME+".to_string(),
                "Command".to_string(),
            ],

            screen_tabs: None, // Use defaults from ScreenTab::default_main/default_io

            mouse_enabled: true,
            readonly: false,
            confirm_kill: true,  // Show confirmation dialogs by default
        }
    }
}

impl Config {
    /// Get the config file path
    pub fn config_path() -> Option<PathBuf> {
        // Use Windows API directly instead of `directories` crate
        use windows::core::PWSTR;
        use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

        unsafe {
            let path: PWSTR = SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None).ok()?;
            let len = (0..).take_while(|&i| *path.0.add(i) != 0).count();
            let slice = std::slice::from_raw_parts(path.0, len);
            let appdata = PathBuf::from(String::from_utf16_lossy(slice));
            windows::Win32::System::Com::CoTaskMemFree(Some(path.0 as *const _));
            Some(appdata.join("htop-win").join("config").join("config.json"))
        }
    }

    /// Load configuration from file, or return defaults
    pub fn load() -> Self {
        if let Some(path) = Self::config_path()
            && path.exists()
        {
            match fs::read_to_string(&path) {
                Ok(content) => {
                    if let Some(value) = json::parse(&content) {
                        return Self::from_json(&value);
                    } else {
                        eprintln!("Warning: Failed to parse config");
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to read config: {}", e);
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

            let content = json::to_string_pretty(&self.to_json());
            fs::write(&path, content)?;
        }
        Ok(())
    }

    /// Parse config from JSON value
    fn from_json(v: &Value) -> Self {
        let defaults = Self::default();

        // Helper to get bool with default
        let get_bool = |key: &str, default: bool| -> bool {
            v.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
        };

        // Helper to get u64 with default
        let get_u64 = |key: &str, default: u64| -> u64 {
            v.get(key).and_then(|v| v.as_u64()).unwrap_or(default)
        };

        // Helper to get string with default
        let get_str = |key: &str, default: &str| -> String {
            v.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or(default)
                .to_string()
        };

        // Parse visible_columns array
        let visible_columns = v
            .get("visible_columns")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| defaults.visible_columns.clone());

        Self {
            refresh_rate_ms: get_u64("refresh_rate_ms", defaults.refresh_rate_ms).max(100),
            tree_view_default: get_bool("tree_view_default", defaults.tree_view_default),
            color_scheme: ColorScheme::from_str(&get_str(
                "color_scheme",
                defaults.color_scheme.as_str(),
            )),

            show_kernel_threads: get_bool("show_kernel_threads", defaults.show_kernel_threads),
            show_user_threads: get_bool("show_user_threads", defaults.show_user_threads),
            show_program_path: get_bool("show_program_path", defaults.show_program_path),
            highlight_running: get_bool("highlight_running", defaults.highlight_running),
            highlight_large_numbers: get_bool(
                "highlight_large_numbers",
                defaults.highlight_large_numbers,
            ),
            highlight_new_processes: get_bool(
                "highlight_new_processes",
                defaults.highlight_new_processes,
            ),
            highlight_duration_ms: get_u64("highlight_duration_ms", defaults.highlight_duration_ms),
            highlight_basename: get_bool("highlight_basename", defaults.highlight_basename),

            show_cpu_meters: get_bool("show_cpu_meters", defaults.show_cpu_meters),
            show_memory_meter: get_bool("show_memory_meter", defaults.show_memory_meter),
            show_swap_meter: get_bool("show_swap_meter", defaults.show_swap_meter),
            show_tasks_meter: get_bool("show_tasks_meter", defaults.show_tasks_meter),
            show_uptime_meter: get_bool("show_uptime_meter", defaults.show_uptime_meter),
            show_load_average: get_bool("show_load_average", defaults.show_load_average),
            show_network_io: get_bool("show_network_io", defaults.show_network_io),
            show_disk_io: get_bool("show_disk_io", defaults.show_disk_io),
            show_clock: get_bool("show_clock", defaults.show_clock),
            show_hostname: get_bool("show_hostname", defaults.show_hostname),
            show_battery: get_bool("show_battery", defaults.show_battery),

            cpu_meter_mode: MeterMode::from_str(&get_str(
                "cpu_meter_mode",
                defaults.cpu_meter_mode.as_str(),
            )),
            memory_meter_mode: MeterMode::from_str(&get_str(
                "memory_meter_mode",
                defaults.memory_meter_mode.as_str(),
            )),

            visible_columns,

            // Parse screen_tabs array
            screen_tabs: v.get("screen_tabs").and_then(|v| v.as_array()).map(|arr| {
                arr.iter().filter_map(|tab_val| {
                    let name = tab_val.get("name")?.as_str()?.to_string();
                    let columns: Vec<String> = tab_val.get("columns")?
                        .as_array()?
                        .iter()
                        .filter_map(|c| c.as_str().map(String::from))
                        .collect();
                    let sort_key = tab_val.get("sort_key")
                        .and_then(|v| v.as_str())
                        .and_then(crate::app::SortColumn::from_name)
                        .unwrap_or(crate::app::SortColumn::Cpu);
                    let sort_ascending = tab_val.get("sort_ascending")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    Some(crate::app::ScreenTab {
                        name,
                        columns,
                        sort_column: sort_key,
                        sort_ascending,
                    })
                }).collect()
            }),

            mouse_enabled: get_bool("mouse_enabled", defaults.mouse_enabled),
            readonly: get_bool("readonly", defaults.readonly),
            confirm_kill: get_bool("confirm_kill", defaults.confirm_kill),
        }
    }

    /// Convert config to JSON value
    fn to_json(&self) -> Value {
        let mut map = HashMap::new();

        map.insert(
            "refresh_rate_ms".to_string(),
            Value::Number(self.refresh_rate_ms as i64),
        );
        map.insert(
            "tree_view_default".to_string(),
            Value::Bool(self.tree_view_default),
        );
        map.insert(
            "color_scheme".to_string(),
            Value::String(self.color_scheme.as_str().to_string()),
        );

        map.insert(
            "show_kernel_threads".to_string(),
            Value::Bool(self.show_kernel_threads),
        );
        map.insert(
            "show_user_threads".to_string(),
            Value::Bool(self.show_user_threads),
        );
        map.insert(
            "show_program_path".to_string(),
            Value::Bool(self.show_program_path),
        );
        map.insert(
            "highlight_running".to_string(),
            Value::Bool(self.highlight_running),
        );
        map.insert(
            "highlight_large_numbers".to_string(),
            Value::Bool(self.highlight_large_numbers),
        );
        map.insert(
            "highlight_new_processes".to_string(),
            Value::Bool(self.highlight_new_processes),
        );
        map.insert(
            "highlight_duration_ms".to_string(),
            Value::Number(self.highlight_duration_ms as i64),
        );
        map.insert(
            "highlight_basename".to_string(),
            Value::Bool(self.highlight_basename),
        );

        map.insert(
            "show_cpu_meters".to_string(),
            Value::Bool(self.show_cpu_meters),
        );
        map.insert(
            "show_memory_meter".to_string(),
            Value::Bool(self.show_memory_meter),
        );
        map.insert(
            "show_swap_meter".to_string(),
            Value::Bool(self.show_swap_meter),
        );
        map.insert(
            "show_tasks_meter".to_string(),
            Value::Bool(self.show_tasks_meter),
        );
        map.insert(
            "show_uptime_meter".to_string(),
            Value::Bool(self.show_uptime_meter),
        );
        map.insert(
            "show_load_average".to_string(),
            Value::Bool(self.show_load_average),
        );
        map.insert(
            "show_network_io".to_string(),
            Value::Bool(self.show_network_io),
        );
        map.insert("show_disk_io".to_string(), Value::Bool(self.show_disk_io));
        map.insert("show_clock".to_string(), Value::Bool(self.show_clock));
        map.insert(
            "show_hostname".to_string(),
            Value::Bool(self.show_hostname),
        );
        map.insert("show_battery".to_string(), Value::Bool(self.show_battery));

        map.insert(
            "cpu_meter_mode".to_string(),
            Value::String(self.cpu_meter_mode.as_str().to_string()),
        );
        map.insert(
            "memory_meter_mode".to_string(),
            Value::String(self.memory_meter_mode.as_str().to_string()),
        );

        map.insert(
            "visible_columns".to_string(),
            Value::Array(
                self.visible_columns
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
        );

        // Serialize screen_tabs if present
        if let Some(ref tabs) = self.screen_tabs {
            let tabs_json: Vec<Value> = tabs.iter().map(|tab| {
                let mut tab_map = HashMap::new();
                tab_map.insert("name".to_string(), Value::String(tab.name.clone()));
                tab_map.insert("columns".to_string(), Value::Array(
                    tab.columns.iter().map(|c| Value::String(c.clone())).collect(),
                ));
                tab_map.insert("sort_key".to_string(), Value::String(tab.sort_column.name().to_string()));
                tab_map.insert("sort_ascending".to_string(), Value::Bool(tab.sort_ascending));
                Value::Object(tab_map)
            }).collect();
            map.insert("screen_tabs".to_string(), Value::Array(tabs_json));
        }

        map.insert(
            "mouse_enabled".to_string(),
            Value::Bool(self.mouse_enabled),
        );
        map.insert("readonly".to_string(), Value::Bool(self.readonly));
        map.insert("confirm_kill".to_string(), Value::Bool(self.confirm_kill));

        Value::Object(map)
    }

    /// Reset all settings to defaults
    pub fn reset_to_defaults(&mut self) {
        *self = Self::default();
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
        assert_eq!(config.refresh_rate_ms, 1500);
        assert!(!config.tree_view_default);
        assert!(config.show_cpu_meters);
    }

    #[test]
    fn test_serialization() {
        let config = Config::default();
        let json_value = config.to_json();
        let json_str = json::to_string_pretty(&json_value);
        let parsed = json::parse(&json_str).unwrap();
        let loaded = Config::from_json(&parsed);
        assert_eq!(loaded.refresh_rate_ms, config.refresh_rate_ms);
        assert_eq!(loaded.tree_view_default, config.tree_view_default);
        assert_eq!(loaded.visible_columns, config.visible_columns);
    }
}
