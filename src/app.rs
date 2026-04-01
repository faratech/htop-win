use crate::config::Config;
use crate::system::{ProcessInfo, SystemMetrics};
use crate::ui::colors::Theme;
use std::collections::{HashSet, VecDeque};
use std::time::Instant;

// ============================================================================
// Unified UI Element System
// ============================================================================

/// Identifies a specific UI element that can be interacted with
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UIElement {
    /// CPU meter bar (index = core number, None = average)
    CpuMeter(Option<usize>),
    /// Memory meter bar
    MemoryMeter,
    /// Swap meter bar
    SwapMeter,
    /// Column header (for sorting)
    ColumnHeader(SortColumn),
    /// Process row (index = visible row index, pid = process ID)
    ProcessRow { index: usize, pid: u32 },
    /// Footer function key (F1-F10)
    FunctionKey(u8),
    /// Screen tab (index in screen_tabs array)
    ScreenTab(usize),
    /// Generic header area
    Header,
    /// Generic footer area
    Footer,
}

/// Actions that can be performed on UI elements
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UIAction {
    /// Single left click
    Click,
    /// Double left click
    DoubleClick,
    /// Right click (context menu)
    RightClick,
    /// Middle click
    MiddleClick,
}

/// Major UI regions for keyboard navigation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum FocusRegion {
    /// Header meters (CPU, Memory, Swap)
    Header,
    /// Process list (default focus)
    #[default]
    ProcessList,
    /// Footer function keys
    Footer,
}

/// Windows priority classes (ordered from lowest to highest priority)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsPriorityClass {
    Idle,
    BelowNormal,
    Normal,
    AboveNormal,
    High,
    Realtime,
}

impl WindowsPriorityClass {
    /// Get all priority classes in order
    pub fn all() -> &'static [WindowsPriorityClass] {
        &[
            WindowsPriorityClass::Idle,
            WindowsPriorityClass::BelowNormal,
            WindowsPriorityClass::Normal,
            WindowsPriorityClass::AboveNormal,
            WindowsPriorityClass::High,
            WindowsPriorityClass::Realtime,
        ]
    }

    /// Display name for the priority class
    pub fn name(&self) -> &'static str {
        match self {
            WindowsPriorityClass::Idle => "Idle",
            WindowsPriorityClass::BelowNormal => "Below Normal",
            WindowsPriorityClass::Normal => "Normal",
            WindowsPriorityClass::AboveNormal => "Above Normal",
            WindowsPriorityClass::High => "High",
            WindowsPriorityClass::Realtime => "Realtime",
        }
    }

    /// Short display name for column display (max 6 chars)
    pub fn short_name(&self) -> &'static str {
        match self {
            WindowsPriorityClass::Idle => "Idle",
            WindowsPriorityClass::BelowNormal => "BelowN",
            WindowsPriorityClass::Normal => "Normal",
            WindowsPriorityClass::AboveNormal => "AboveN",
            WindowsPriorityClass::High => "High",
            WindowsPriorityClass::Realtime => "Rltm",
        }
    }

    /// Get the typical base priority value for this class (with normal thread priority)
    pub fn base_priority(&self) -> i32 {
        match self {
            WindowsPriorityClass::Idle => 4,
            WindowsPriorityClass::BelowNormal => 6,
            WindowsPriorityClass::Normal => 8,
            WindowsPriorityClass::AboveNormal => 10,
            WindowsPriorityClass::High => 13,
            WindowsPriorityClass::Realtime => 24,
        }
    }

    /// Convert from index
    pub fn from_index(index: usize) -> Self {
        Self::all().get(index).copied().unwrap_or(WindowsPriorityClass::Normal)
    }

    /// Convert from Windows base priority value (0-31)
    /// Typical values: Idle=4, BelowNormal=6, Normal=8, AboveNormal=10, High=13, Realtime=24
    pub fn from_base_priority(base_priority: i32) -> Self {
        match base_priority {
            0..=4 => WindowsPriorityClass::Idle,
            5..=6 => WindowsPriorityClass::BelowNormal,
            7..=9 => WindowsPriorityClass::Normal,
            10..=12 => WindowsPriorityClass::AboveNormal,
            13..=15 => WindowsPriorityClass::High,
            _ => WindowsPriorityClass::Realtime, // 16+
        }
    }

    /// Get the index in the all() array
    pub fn index(&self) -> usize {
        Self::all().iter().position(|p| p == self).unwrap_or(2)
    }
}

/// A rectangular region on screen associated with a UI element
#[derive(Debug, Clone)]
pub struct UIRegion {
    pub element: UIElement,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl UIRegion {
    pub fn new(element: UIElement, x: u16, y: u16, width: u16, height: u16) -> Self {
        Self { element, x, y, width, height }
    }

    /// Check if a point is within this region
    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Bounds of a single column in the process list header
#[derive(Debug, Clone, Default)]
pub struct ColumnBounds {
    pub column: Option<SortColumn>,
    pub x: u16,
    pub width: u16,
}

/// UI layout bounds - populated during render for accurate mouse/keyboard navigation
#[derive(Debug, Clone, Default)]
pub struct UIBounds {
    /// Header meters area (CPU bars, memory, etc.)
    pub header_y_start: u16,
    pub header_y_end: u16,

    /// Tab bar row (y coordinate, 0 if no tab bar)
    pub tab_bar_y: u16,
    /// Whether tab bar is visible
    pub tab_bar_visible: bool,

    /// Process list column headers
    pub column_header_y: u16,
    pub columns: Vec<ColumnBounds>,

    /// Process list data rows
    pub process_list_y_start: u16,
    pub process_list_y_end: u16,

    /// Footer area
    pub footer_y_start: u16,

    /// All interactive UI regions (for unified hit testing)
    pub regions: Vec<UIRegion>,

    /// Function key regions in footer
    pub function_keys: Vec<UIRegion>,
}

impl UIBounds {
    /// Clear all regions (call at start of each render)
    pub fn clear_regions(&mut self) {
        self.regions.clear();
        self.function_keys.clear();
    }

    /// Add a UI region
    pub fn add_region(&mut self, region: UIRegion) {
        self.regions.push(region);
    }

    /// Add a function key region
    pub fn add_function_key(&mut self, key: u8, x: u16, y: u16, width: u16) {
        self.function_keys.push(UIRegion::new(
            UIElement::FunctionKey(key),
            x, y, width, 1,
        ));
    }

    /// Find which element is at the given coordinates
    pub fn element_at(&self, x: u16, y: u16) -> Option<UIElement> {
        // Check function keys first (most specific)
        for region in &self.function_keys {
            if region.contains(x, y) {
                return Some(region.element.clone());
            }
        }

        // Check all other regions
        for region in &self.regions {
            if region.contains(x, y) {
                return Some(region.element.clone());
            }
        }

        // Fall back to area-based detection
        if y < self.header_y_end {
            return Some(UIElement::Header);
        }

        // Tab bar area is handled by registered regions (checked above)

        if y == self.column_header_y
            && let Some(col) = self.column_at_x(x) {
                return Some(UIElement::ColumnHeader(col));
            }

        if let Some(row_index) = self.process_row_index(y) {
            // Note: PID needs to be filled in by caller who has process data
            return Some(UIElement::ProcessRow { index: row_index, pid: 0 });
        }

        if y >= self.footer_y_start {
            return Some(UIElement::Footer);
        }

        None
    }

    /// Find which column contains the given x coordinate
    pub fn column_at_x(&self, x: u16) -> Option<SortColumn> {
        for (i, col) in self.columns.iter().enumerate() {
            let is_last = i == self.columns.len() - 1;
            if is_last {
                if x >= col.x {
                    return col.column;
                }
            } else {
                let col_end = col.x + col.width;
                if x >= col.x && x < col_end {
                    return col.column;
                }
            }
        }
        None
    }

    /// Check if y coordinate is in the process list data area
    fn is_process_row(&self, y: u16) -> bool {
        y > self.column_header_y && y < self.footer_y_start
    }

    /// Get the process row index for a given y coordinate (0-indexed from first visible row)
    pub fn process_row_index(&self, y: u16) -> Option<usize> {
        if self.is_process_row(y) {
            Some((y - self.column_header_y - 1) as usize)
        } else {
            None
        }
    }
}

/// Sort column for process list
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Pid,
    PPid,
    User,
    Priority,
    PriorityClass,
    Threads,
    Virt,
    Res,
    Shr,
    Status,
    Cpu,
    Mem,
    Time,
    StartTime,
    Command,
    // Windows-specific sort columns
    Elevated,    // Running as admin
    Arch,        // Process architecture (x86/x64/ARM)
    Efficiency,  // Efficiency mode (EcoQoS)
    // I/O columns
    HandleCount, // Number of open handles
    IoRate,      // Combined I/O rate (read + write per refresh)
    IoReadRate,  // I/O read bytes per refresh
    IoWriteRate, // I/O write bytes per refresh
    IoRead,      // Cumulative I/O read bytes
    IoWrite,     // Cumulative I/O write bytes
}

impl SortColumn {
    pub fn all() -> &'static [SortColumn] {
        &[
            SortColumn::Pid,
            SortColumn::PPid,
            SortColumn::User,
            SortColumn::Priority,
            SortColumn::PriorityClass,
            SortColumn::Threads,
            SortColumn::Virt,
            SortColumn::Res,
            SortColumn::Shr,
            SortColumn::Status,
            SortColumn::Cpu,
            SortColumn::Mem,
            SortColumn::Time,
            SortColumn::StartTime,
            SortColumn::Command,
            SortColumn::Elevated,
            SortColumn::Arch,
            SortColumn::Efficiency,
            SortColumn::HandleCount,
            SortColumn::IoRate,
            SortColumn::IoReadRate,
            SortColumn::IoWriteRate,
            SortColumn::IoRead,
            SortColumn::IoWrite,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            SortColumn::Pid => "PID",
            SortColumn::PPid => "PPID",
            SortColumn::User => "USER",
            SortColumn::Priority => "PRI",
            SortColumn::PriorityClass => "CLASS",
            SortColumn::Threads => "THR",
            SortColumn::Virt => "VIRT",
            SortColumn::Res => "RES",
            SortColumn::Shr => "SHR",
            SortColumn::Status => "S",
            SortColumn::Cpu => "CPU%",
            SortColumn::Mem => "MEM%",
            SortColumn::Time => "TIME+",
            SortColumn::StartTime => "START",
            SortColumn::Command => "Command",
            SortColumn::Elevated => "ELEV",
            SortColumn::Arch => "ARCH",
            SortColumn::Efficiency => "ECO",
            SortColumn::HandleCount => "HNDL",
            SortColumn::IoRate => "IO_RATE",
            SortColumn::IoReadRate => "IO_R/s",
            SortColumn::IoWriteRate => "IO_W/s",
            SortColumn::IoRead => "IO_RD",
            SortColumn::IoWrite => "IO_WR",
        }
    }

    /// Convert from column name string
    pub fn from_name(name: &str) -> Option<SortColumn> {
        match name {
            "PID" => Some(SortColumn::Pid),
            "PPID" => Some(SortColumn::PPid),
            "USER" => Some(SortColumn::User),
            "PRI" => Some(SortColumn::Priority),
            "CLASS" => Some(SortColumn::PriorityClass),
            "NI" => Some(SortColumn::PriorityClass), // Legacy name
            "THR" => Some(SortColumn::Threads),
            "VIRT" => Some(SortColumn::Virt),
            "RES" => Some(SortColumn::Res),
            "SHR" => Some(SortColumn::Shr),
            "S" => Some(SortColumn::Status),
            "CPU%" => Some(SortColumn::Cpu),
            "MEM%" => Some(SortColumn::Mem),
            "TIME+" => Some(SortColumn::Time),
            "START" => Some(SortColumn::StartTime),
            "Command" => Some(SortColumn::Command),
            "ELEV" => Some(SortColumn::Elevated),
            "ARCH" => Some(SortColumn::Arch),
            "ECO" => Some(SortColumn::Efficiency),
            "HNDL" => Some(SortColumn::HandleCount),
            "IO_RATE" => Some(SortColumn::IoRate),
            "IO_R/s" => Some(SortColumn::IoReadRate),
            "IO_W/s" => Some(SortColumn::IoWriteRate),
            "IO_RD" => Some(SortColumn::IoRead),
            "IO_WR" => Some(SortColumn::IoWrite),
            _ => None,
        }
    }

    /// Get the display width for this column (must match ui/process_list.rs column_width)
    pub fn width(&self) -> u16 {
        match self {
            SortColumn::Pid => 7,
            SortColumn::PPid => 7,
            SortColumn::User => 10,
            SortColumn::Priority => 4,
            SortColumn::PriorityClass => 7,
            SortColumn::Threads => 4,
            SortColumn::Virt => 8,
            SortColumn::Res => 8,
            SortColumn::Shr => 8,
            SortColumn::Status => 3,
            SortColumn::Cpu => 6,
            SortColumn::Mem => 6,
            SortColumn::Time => 10,
            SortColumn::StartTime => 8,
            SortColumn::Command => 20, // Min width, but effectively extends to end
            SortColumn::Elevated => 4,
            SortColumn::Arch => 5,
            SortColumn::Efficiency => 4,
            SortColumn::HandleCount => 6,
            SortColumn::IoRate => 8,
            SortColumn::IoReadRate => 7,
            SortColumn::IoWriteRate => 7,
            SortColumn::IoRead => 7,
            SortColumn::IoWrite => 7,
        }
    }
}

/// A screen tab with its own column set and sort settings (like htop's Main/I/O tabs)
#[derive(Clone, Debug)]
pub struct ScreenTab {
    pub name: String,
    pub columns: Vec<String>,
    pub sort_column: SortColumn,
    pub sort_ascending: bool,
}

impl ScreenTab {
    pub fn default_main(config: &Config) -> Self {
        Self {
            name: "Main".to_string(),
            columns: config.visible_columns.clone(),
            sort_column: SortColumn::Cpu,
            sort_ascending: false,
        }
    }

    pub fn default_io() -> Self {
        Self {
            name: "I/O".to_string(),
            columns: vec![
                "PID", "USER", "IO_RATE", "IO_R/s", "IO_W/s", "HNDL", "Command",
            ].into_iter().map(String::from).collect(),
            sort_column: SortColumn::IoRate,
            sort_ascending: false,
        }
    }
}

/// Dialog/view state - encapsulates per-dialog state into enum variants
/// Replaces the previous flat ViewMode enum + scattered dialog fields
#[derive(Debug, Clone)]
pub enum DialogState {
    /// Normal process list view (no dialog open)
    None,
    Help { scroll: usize },
    Search { buffer: String, cursor: usize },
    Filter { buffer: String, cursor: usize },
    SortSelect { index: usize },
    Kill { pid: u32, name: String, command: String },
    SignalSelect { index: usize, pid: u32, name: String, command: String },
    Priority { class_index: usize, pid: u32, name: String },
    Setup { selected: usize },
    ProcessInfo { target: Box<ProcessInfo> },
    UserSelect { index: usize, users: Vec<String> },
    Environment { scroll: usize },
    ColorScheme { index: usize },
    CommandWrap { scroll: usize },
    ColumnConfig { index: usize },
    Affinity { mask: u64, selected: usize },
}

impl Default for DialogState {
    fn default() -> Self {
        DialogState::None
    }
}

impl DialogState {
    /// Get mutable reference to input buffer and cursor (Search/Filter dialogs)
    pub fn input_buffer_mut(&mut self) -> Option<(&mut String, &mut usize)> {
        match self {
            DialogState::Search { buffer, cursor }
            | DialogState::Filter { buffer, cursor } => Some((buffer, cursor)),
            _ => None,
        }
    }

    /// Get input buffer contents (Search/Filter dialogs)
    pub fn input_buffer(&self) -> Option<(&str, usize)> {
        match self {
            DialogState::Search { buffer, cursor }
            | DialogState::Filter { buffer, cursor } => Some((buffer, *cursor)),
            _ => None,
        }
    }
}

/// Application state
pub struct App {
    /// Application configuration
    pub config: Config,
    /// Current color theme (derived from config)
    pub theme: Theme,
    /// Current dialog/view state
    pub dialog: DialogState,
    /// System metrics (CPU, memory, etc.)
    pub system_metrics: SystemMetrics,
    /// All processes
    pub processes: Vec<ProcessInfo>,
    /// Filtered/displayed processes
    pub displayed_processes: Vec<ProcessInfo>,
    /// Currently selected process index
    pub selected_index: usize,
    /// Scroll offset for process list
    pub scroll_offset: usize,
    /// Sort column
    pub sort_column: SortColumn,
    /// Sort ascending
    pub sort_ascending: bool,
    /// Screen tabs (like htop's Main/I/O tabs)
    pub screen_tabs: Vec<ScreenTab>,
    /// Active screen tab index
    pub active_tab: usize,
    /// Tree view enabled
    pub tree_view: bool,
    /// Search string
    pub search_string: String,
    /// Cached lowercase search string (updated when search_string changes)
    pub search_string_lower: String,
    /// Filter string
    pub filter_string: String,
    /// Cached lowercase filter string (updated when filter_string changes)
    pub filter_string_lower: String,
    /// User filter (show only this user's processes)
    pub user_filter: Option<String>,
    /// PID filter (show only these PIDs) - from CLI -p option (HashSet for O(1) lookup)
    pub pid_filter: Option<HashSet<u32>>,
    /// Tagged process PIDs
    pub tagged_pids: HashSet<u32>,
    /// Process list visible height (set during render)
    pub visible_height: usize,
    /// Last error message with timestamp for auto-expiry
    pub last_error: Option<(String, Instant)>,
    /// Status message (success/info) with timestamp for auto-expiry
    pub status_message: Option<(String, Instant)>,

    /// Collapsed PIDs in tree view
    pub collapsed_pids: HashSet<u32>,
    /// Follow mode: PID to follow across refreshes
    pub follow_pid: Option<u32>,
    /// Pause updates
    pub paused: bool,
    /// PID search buffer (for incremental PID search with digits)
    pub pid_search_buffer: String,
    /// Last PID search time (for timeout)
    pub pid_search_time: Option<Instant>,
    /// Show header meters
    pub show_header: bool,
    /// Maximum iterations before exit (for -n option)
    pub max_iterations: Option<u64>,
    /// Current iteration count
    pub iteration_count: u64,
    /// CPU usage history for graph mode (per core, last N samples)
    pub cpu_history: Vec<VecDeque<f32>>,
    /// Memory usage history for graph mode (last N samples)
    pub mem_history: VecDeque<f32>,
    pub swap_history: VecDeque<f32>,
    /// Cached visible columns (updated when column config changes)
    pub cached_visible_columns: Vec<SortColumn>,
    /// Deferred process list update flag (flushed once before each render)
    pub needs_process_update: bool,
    /// UI layout bounds (populated during render for accurate mouse/keyboard navigation)
    pub ui_bounds: UIBounds,

    // Mouse interaction state
    /// Last click position for double-click detection
    pub last_click_pos: Option<(u16, u16)>,
    /// Last click time for double-click detection
    pub last_click_time: Option<Instant>,
    /// Double-click threshold in milliseconds
    pub double_click_ms: u64,

    // Update check state
    /// Available update version and path (set by background thread)
    pub update_available: Option<(String, std::path::PathBuf)>,
    /// Whether we've already checked for updates
    pub update_checked: bool,

    // Keyboard navigation state
    /// Currently focused UI region (for Tab navigation)
    pub focus_region: FocusRegion,
    /// Focused index within the current region (e.g., which function key)
    pub focus_index: usize,
}

impl App {
    pub fn new(config: Config) -> Self {
        let theme = config.theme();
        let tree_view = config.tree_view_default;
        let visible_columns = Self::compute_visible_columns(&config);
        let screen_tabs = config.screen_tabs.clone().unwrap_or_else(|| vec![
            ScreenTab::default_main(&config),
            ScreenTab::default_io(),
        ]);
        Self {
            config,
            theme,
            dialog: DialogState::None,
            system_metrics: SystemMetrics::default(),
            processes: Vec::new(),
            displayed_processes: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            sort_column: SortColumn::Cpu,
            sort_ascending: false,
            tree_view,
            search_string: String::new(),
            search_string_lower: String::new(),
            filter_string: String::new(),
            filter_string_lower: String::new(),
            user_filter: None,
            pid_filter: None,
            tagged_pids: HashSet::new(),
            visible_height: 20,
            last_error: None,
            status_message: None,
            collapsed_pids: HashSet::new(),
            follow_pid: None,
            paused: false,
            pid_search_buffer: String::new(),
            pid_search_time: None,
            show_header: true,
            max_iterations: None,
            iteration_count: 0,
            cpu_history: Vec::new(),
            mem_history: VecDeque::new(),
            swap_history: VecDeque::new(),
            cached_visible_columns: visible_columns,
            needs_process_update: false,
            ui_bounds: UIBounds::default(),
            last_click_pos: None,
            last_click_time: None,
            double_click_ms: 500, // Standard double-click threshold
            update_available: None,
            update_checked: false,
            focus_region: FocusRegion::default(),
            focus_index: 0,
            screen_tabs,
            active_tab: 0,
        }
    }

    /// Compute visible columns based on config (used for caching)
    /// Respects the order defined in config.visible_columns
    fn compute_visible_columns(config: &Config) -> Vec<SortColumn> {
        config.visible_columns
            .iter()
            .filter_map(|name| SortColumn::from_name(name))
            .collect()
    }

    /// Update the cached visible columns from the active screen tab
    pub fn update_visible_columns_cache(&mut self) {
        let columns = if let Some(tab) = self.screen_tabs.get(self.active_tab) {
            &tab.columns
        } else {
            &self.config.visible_columns
        };
        self.cached_visible_columns = columns
            .iter()
            .filter_map(|name| SortColumn::from_name(name))
            .collect();
    }

    /// Get the active tab's columns (for column config dialog)
    pub fn active_tab_columns(&self) -> &[String] {
        if let Some(tab) = self.screen_tabs.get(self.active_tab) {
            &tab.columns
        } else {
            &self.config.visible_columns
        }
    }

    /// Check if a column is visible in the active tab
    pub fn is_column_visible_in_active_tab(&self, column: &str) -> bool {
        self.active_tab_columns().iter().any(|c| c == column)
    }

    /// Get the position of a column in the active tab's visible order
    pub fn column_position_in_active_tab(&self, column: &str) -> Option<usize> {
        self.active_tab_columns().iter().position(|c| c == column)
    }

    /// Toggle a column's visibility in the active tab
    pub fn toggle_column_in_active_tab(&mut self, column: &str) {
        if let Some(tab) = self.screen_tabs.get_mut(self.active_tab) {
            if let Some(pos) = tab.columns.iter().position(|c| c == column) {
                tab.columns.remove(pos);
            } else {
                tab.columns.push(column.to_string());
            }
        }
        self.sync_config_from_active_tab();
        self.update_visible_columns_cache();
    }

    /// Move a column up in the active tab's order
    pub fn move_column_up_in_active_tab(&mut self, column: &str) -> bool {
        if let Some(tab) = self.screen_tabs.get_mut(self.active_tab) {
            if let Some(pos) = tab.columns.iter().position(|c| c == column)
                && pos > 0 {
                    tab.columns.swap(pos, pos - 1);
                    self.sync_config_from_active_tab();
                    self.update_visible_columns_cache();
                    return true;
                }
        }
        false
    }

    /// Move a column down in the active tab's order
    pub fn move_column_down_in_active_tab(&mut self, column: &str) -> bool {
        if let Some(tab) = self.screen_tabs.get_mut(self.active_tab) {
            if let Some(pos) = tab.columns.iter().position(|c| c == column)
                && pos < tab.columns.len() - 1 {
                    tab.columns.swap(pos, pos + 1);
                    self.sync_config_from_active_tab();
                    self.update_visible_columns_cache();
                    return true;
                }
        }
        false
    }

    /// Sync config.visible_columns from the active tab (for backward compat with config file)
    fn sync_config_from_active_tab(&mut self) {
        if let Some(tab) = self.screen_tabs.get(self.active_tab) {
            self.config.visible_columns = tab.columns.clone();
        }
    }

    /// Reset screen tabs to defaults and apply
    pub fn reset_screen_tabs(&mut self) {
        self.screen_tabs = vec![
            ScreenTab::default_main(&Config::default()),
            ScreenTab::default_io(),
        ];
        self.active_tab = 0;
        self.apply_active_tab();
    }

    /// Update the color theme from config
    pub fn update_theme(&mut self) {
        self.theme = self.config.theme();
    }

    /// Save the current configuration (syncs screen tab state first)
    pub fn save_config(&mut self) {
        // Sync current sort settings to active tab before saving
        if let Some(tab) = self.screen_tabs.get_mut(self.active_tab) {
            tab.sort_column = self.sort_column;
            tab.sort_ascending = self.sort_ascending;
        }
        self.sync_config_from_active_tab();
        self.config.screen_tabs = Some(self.screen_tabs.clone());
        if let Err(e) = self.config.save() {
            eprintln!("Failed to save config: {}", e);
        }
    }

    // =========================================================================
    // Screen Tab Navigation
    // =========================================================================

    /// Switch to next screen tab (Tab key, like htop)
    pub fn next_screen_tab(&mut self) {
        if self.screen_tabs.len() <= 1 {
            return;
        }
        // Save current sort settings to active tab
        if let Some(tab) = self.screen_tabs.get_mut(self.active_tab) {
            tab.sort_column = self.sort_column;
            tab.sort_ascending = self.sort_ascending;
        }
        self.active_tab = (self.active_tab + 1) % self.screen_tabs.len();
        self.apply_active_tab();
    }

    /// Switch to previous screen tab (Shift+Tab key, like htop)
    pub fn prev_screen_tab(&mut self) {
        if self.screen_tabs.len() <= 1 {
            return;
        }
        // Save current sort settings to active tab
        if let Some(tab) = self.screen_tabs.get_mut(self.active_tab) {
            tab.sort_column = self.sort_column;
            tab.sort_ascending = self.sort_ascending;
        }
        self.active_tab = if self.active_tab == 0 {
            self.screen_tabs.len() - 1
        } else {
            self.active_tab - 1
        };
        self.apply_active_tab();
    }

    /// Apply the active tab's settings (sort, columns)
    pub fn apply_active_tab(&mut self) {
        if let Some(tab) = self.screen_tabs.get(self.active_tab) {
            self.sort_column = tab.sort_column;
            self.sort_ascending = tab.sort_ascending;
            // Sync config.visible_columns so column config dialog shows the right columns
            self.config.visible_columns = tab.columns.clone();
        }
        self.cached_visible_columns = self.config.visible_columns
            .iter()
            .filter_map(|name| SortColumn::from_name(name))
            .collect();
    }

    /// Navigate left within the current focus region
    pub fn navigate_left(&mut self) {
        match self.focus_region {
            FocusRegion::Header => {
                // Cycle through meter modes
                self.config.cpu_meter_mode = self.config.cpu_meter_mode.next();
            }
            FocusRegion::ProcessList => {
                // Nothing to do for left in process list
            }
            FocusRegion::Footer => {
                // Move to previous function key
                if self.focus_index > 0 {
                    self.focus_index -= 1;
                } else {
                    self.focus_index = 9; // Wrap to F10
                }
            }
        }
    }

    /// Navigate right within the current focus region
    pub fn navigate_right(&mut self) {
        match self.focus_region {
            FocusRegion::Header => {
                // Cycle through meter modes
                self.config.memory_meter_mode = self.config.memory_meter_mode.next();
            }
            FocusRegion::ProcessList => {
                // Nothing to do for right in process list
            }
            FocusRegion::Footer => {
                // Move to next function key
                if self.focus_index < 9 {
                    self.focus_index += 1;
                } else {
                    self.focus_index = 0; // Wrap to F1
                }
            }
        }
    }

    /// Activate the currently focused element (Enter/Space)
    pub fn activate_focused(&mut self) -> bool {
        match self.focus_region {
            FocusRegion::Header => {
                // Toggle header visibility
                self.show_header = !self.show_header;
                false
            }
            FocusRegion::ProcessList => {
                // Enter on process opens process info
                self.enter_process_info_mode();
                false
            }
            FocusRegion::Footer => {
                // Activate the focused function key (F1-F10)
                let key = (self.focus_index + 1) as u8;
                if key == 10 {
                    true // Quit
                } else {
                    self.handle_function_key(key);
                    false
                }
            }
        }
    }

    /// Handle function key press (used by both keyboard and mouse)
    pub fn handle_function_key(&mut self, key: u8) {
        match key {
            1 => self.dialog = DialogState::Help { scroll: 0 },
            2 => self.dialog = DialogState::Setup { selected: 0 },
            3 => self.start_search(),
            4 => self.start_filter(),
            5 => self.tree_view = !self.tree_view,
            6 => self.dialog = DialogState::SortSelect { index: 0 },
            7 => self.enter_priority_mode(),
            8 => self.enter_priority_mode(),
            9 => self.enter_kill_mode(),
            _ => {}
        }
    }

    /// Enter kill mode and capture the target process
    pub fn enter_kill_mode(&mut self) {
        if let Some(proc) = self.selected_process() {
            let (pid, name, command) = (proc.pid, proc.name.clone(), proc.command.clone());

            // Skip confirmation dialog if disabled in settings
            if !self.config.confirm_kill {
                if !self.tagged_pids.is_empty() {
                    self.kill_tagged(15);
                } else {
                    self.kill_process_by(pid, &name, 15);
                }
            } else {
                self.dialog = DialogState::Kill { pid, name, command };
            }
        }
    }

    /// Enter priority mode and capture the target process
    pub fn enter_priority_mode(&mut self) {
        if let Some(proc) = self.selected_process() {
            let class_index = WindowsPriorityClass::from_base_priority(proc.priority).index();
            self.dialog = DialogState::Priority {
                class_index,
                pid: proc.pid,
                name: proc.name.clone(),
            };
        }
    }

    /// Enter process info mode and capture the target process
    pub fn enter_process_info_mode(&mut self) {
        if let Some(proc) = self.selected_process() {
            let mut proc_copy = proc.clone();
            let (io_read, io_write) = crate::system::get_process_io_counters(proc.pid);
            proc_copy.io_read_bytes = io_read;
            proc_copy.io_write_bytes = io_write;
            if proc_copy.exe_path.is_empty() {
                let exe_path = crate::system::get_process_exe_path(proc.pid);
                if !exe_path.is_empty() {
                    proc_copy.exe_path = exe_path.clone();
                    proc_copy.command = exe_path;
                }
            }
            self.dialog = DialogState::ProcessInfo { target: Box::new(proc_copy) };
        }
    }

    /// Refresh I/O counters for process info dialog (called during tick when dialog is open)
    pub fn refresh_process_info_io(&mut self) {
        if let DialogState::ProcessInfo { ref mut target } = self.dialog {
            let (io_read, io_write) = crate::system::get_process_io_counters(target.pid);
            target.io_read_bytes = io_read;
            target.io_write_bytes = io_write;
        }
    }

    /// Refresh system data (synchronous, used for initial refresh fallback)
    pub fn refresh_system(&mut self) {
        // Use native Windows APIs for all system metrics
        self.system_metrics.refresh();
        // Update processes in-place to avoid re-allocating strings
        self.system_metrics.update_processes_native(&mut self.processes);
        self.update_displayed_processes();

        // Update history for graph mode
        self.update_meter_history();
    }

    /// Apply a snapshot from the background data collector
    pub fn apply_snapshot(&mut self, snapshot: crate::data::SystemSnapshot) {
        self.system_metrics = snapshot.metrics;
        self.processes = snapshot.processes;
        self.update_displayed_processes();
        self.update_meter_history();
    }

    /// Update CPU and memory history for graph mode rendering
    fn update_meter_history(&mut self) {
        // htop uses up to 32768 samples; we use 512 for reasonable memory usage
        // At 1.5s refresh, this is ~12 minutes of history
        // Each char displays 2 samples, so 256 chars width of graph data
        const MAX_HISTORY: usize = 512;

        let cpu_count = self.system_metrics.cpu.core_usage.len();

        // Initialize CPU history if needed
        if self.cpu_history.len() != cpu_count {
            self.cpu_history = vec![VecDeque::with_capacity(MAX_HISTORY); cpu_count];
        }

        // Add current CPU usage to history (O(1) with VecDeque)
        for (i, &usage) in self.system_metrics.cpu.core_usage.iter().enumerate() {
            let history = &mut self.cpu_history[i];
            if history.len() >= MAX_HISTORY {
                history.pop_front(); // O(1) instead of O(n)
            }
            history.push_back(usage);
        }

        // Add current memory usage to history (O(1) with VecDeque)
        if self.mem_history.len() >= MAX_HISTORY {
            self.mem_history.pop_front(); // O(1) instead of O(n)
        }
        self.mem_history.push_back(self.system_metrics.memory.used_percent);

        // Add current swap usage to history
        if self.swap_history.len() >= MAX_HISTORY {
            self.swap_history.pop_front();
        }
        self.swap_history.push_back(self.system_metrics.memory.swap_percent);
    }

    /// Update displayed processes based on filter and sort
    pub fn update_displayed_processes(&mut self) {
        // Use cached lowercase filter string
        let has_filter = !self.filter_string_lower.is_empty();
        let has_search = !self.search_string_lower.is_empty();

        // Pre-format PID filter check to avoid per-process allocation
        let filter_as_pid: Option<u32> = if has_filter {
            self.filter_string_lower.parse().ok()
        } else {
            None
        };

        // Filter-then-clone: only clone processes that pass all filters
        // Also set matches_search flag during this pass to avoid recomputing in render
        let show_kernel = self.config.show_kernel_threads;
        let show_user = self.config.show_user_threads;
        let process_count = self.processes.len();

        let mut processes: Vec<ProcessInfo> = Vec::with_capacity(process_count);
        processes.extend(self.processes
            .iter()
            .filter(|p| {
                // Kernel/System threads filter
                // On Windows, "kernel threads" are SYSTEM user processes
                let is_kernel = p.user_lower == "system"
                    || p.user_lower.starts_with("nt authority")
                    || p.pid == 0
                    || p.pid == 4;

                if !show_kernel && is_kernel {
                    return false;
                }

                // User threads filter
                // On Windows, "user threads" are non-system processes
                if !show_user && !is_kernel {
                    return false;
                }

                // PID filter (from CLI -p option)
                if let Some(ref pids) = self.pid_filter
                    && !pids.contains(&p.pid) {
                        return false;
                    }
                // User filter
                if let Some(ref user) = self.user_filter
                    && &p.user != user {
                        return false;
                    }
                // Text filter - use pre-computed lowercase strings
                if has_filter
                    && !(p.name_lower.contains(&self.filter_string_lower)
                        || p.command_lower.contains(&self.filter_string_lower)
                        || filter_as_pid.is_some_and(|n| p.pid == n)
                        || p.user_lower.contains(&self.filter_string_lower))
                    {
                        return false;
                    }
                true
            })
            .cloned());

        // Set matches_search flag on each process (for render-time highlighting)
        if has_search {
            for proc in &mut processes {
                proc.matches_search = proc.name_lower.contains(&self.search_string_lower)
                    || proc.command_lower.contains(&self.search_string_lower);
            }
        } else {
            for proc in &mut processes {
                proc.matches_search = false;
            }
        }

        // Sort processes
        self.sort_processes(&mut processes);

        // Build tree if needed
        if self.tree_view {
            processes = self.build_tree(processes);
        }

        self.displayed_processes = processes;

        // Enrich visible processes with additional data from Windows APIs
        // Use a buffer zone to handle scrolling smoothly
        const BUFFER_SIZE: usize = 10;
        let visible_start = self.scroll_offset.saturating_sub(BUFFER_SIZE);
        let visible_end = (self.scroll_offset + self.visible_height + BUFFER_SIZE)
            .min(self.displayed_processes.len());

        if visible_start < visible_end {
            // Only query exe paths when show_program_path is enabled (expensive API call)
            crate::system::enrich_processes(
                &mut self.displayed_processes[visible_start..visible_end],
                self.config.show_program_path,
            );
        }

        // Handle follow mode - find and select the followed PID
        if let Some(follow_pid) = self.follow_pid
            && let Some(idx) = self.displayed_processes.iter().position(|p| p.pid == follow_pid) {
                self.selected_index = idx;
                self.ensure_visible();
            }

        // Ensure selection is valid
        if self.selected_index >= self.displayed_processes.len() {
            self.selected_index = self.displayed_processes.len().saturating_sub(1);
        }
    }

    fn sort_processes(&self, processes: &mut [ProcessInfo]) {
        use std::cmp::Ordering;

        // Use sort_unstable_by for better performance (no stability guarantee needed)
        // The closure still has the match, but sort_unstable is faster overall
        let ascending = self.sort_ascending;

        match self.sort_column {
            // Specialize common sort columns for best performance (avoid match in hot loop)
            SortColumn::Cpu => {
                if ascending {
                    processes.sort_unstable_by(|a, b| a.cpu_percent.partial_cmp(&b.cpu_percent).unwrap_or(Ordering::Equal));
                } else {
                    processes.sort_unstable_by(|a, b| b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap_or(Ordering::Equal));
                }
            }
            SortColumn::Mem => {
                if ascending {
                    processes.sort_unstable_by(|a, b| a.mem_percent.partial_cmp(&b.mem_percent).unwrap_or(Ordering::Equal));
                } else {
                    processes.sort_unstable_by(|a, b| b.mem_percent.partial_cmp(&a.mem_percent).unwrap_or(Ordering::Equal));
                }
            }
            SortColumn::Pid => {
                if ascending {
                    processes.sort_unstable_by_key(|p| p.pid);
                } else {
                    processes.sort_unstable_by_key(|p| std::cmp::Reverse(p.pid));
                }
            }
            SortColumn::Res => {
                if ascending {
                    processes.sort_unstable_by_key(|p| p.resident_mem);
                } else {
                    processes.sort_unstable_by_key(|p| std::cmp::Reverse(p.resident_mem));
                }
            }
            SortColumn::Time => {
                if ascending {
                    processes.sort_unstable_by_key(|p| p.cpu_time);
                } else {
                    processes.sort_unstable_by_key(|p| std::cmp::Reverse(p.cpu_time));
                }
            }
            // Less common columns - use generic approach
            _ => {
                let cmp_fn = |a: &ProcessInfo, b: &ProcessInfo| -> Ordering {
                    let ord = match self.sort_column {
                        SortColumn::PPid => a.parent_pid.cmp(&b.parent_pid),
                        SortColumn::User => a.user.cmp(&b.user),
                        SortColumn::Priority => a.priority.cmp(&b.priority),
                        SortColumn::PriorityClass => a.priority.cmp(&b.priority),
                        SortColumn::Threads => a.thread_count.cmp(&b.thread_count),
                        SortColumn::Virt => a.virtual_mem.cmp(&b.virtual_mem),
                        SortColumn::Shr => a.shared_mem.cmp(&b.shared_mem),
                        SortColumn::Status => a.status.cmp(&b.status),
                        SortColumn::StartTime => a.start_time.cmp(&b.start_time),
                        SortColumn::Command => a.command.cmp(&b.command),
                        SortColumn::Elevated => a.is_elevated.cmp(&b.is_elevated),
                        SortColumn::Arch => a.arch.as_str().cmp(b.arch.as_str()),
                        SortColumn::Efficiency => a.efficiency_mode.cmp(&b.efficiency_mode),
                        SortColumn::HandleCount => a.handle_count.cmp(&b.handle_count),
                        SortColumn::IoRate => (a.io_read_rate + a.io_write_rate).cmp(&(b.io_read_rate + b.io_write_rate)),
                        SortColumn::IoReadRate => a.io_read_rate.cmp(&b.io_read_rate),
                        SortColumn::IoWriteRate => a.io_write_rate.cmp(&b.io_write_rate),
                        SortColumn::IoRead => a.io_read_bytes.cmp(&b.io_read_bytes),
                        SortColumn::IoWrite => a.io_write_bytes.cmp(&b.io_write_bytes),
                        // Already handled above
                        SortColumn::Cpu | SortColumn::Mem | SortColumn::Pid | SortColumn::Res | SortColumn::Time => Ordering::Equal,
                    };
                    if ascending { ord } else { ord.reverse() }
                };
                processes.sort_unstable_by(cmp_fn);
            }
        }
    }

    fn build_tree(&self, processes: Vec<ProcessInfo>) -> Vec<ProcessInfo> {
        use std::collections::HashMap;

        // First, build a set of all PIDs in our list
        let all_pids: HashSet<u32> = processes.iter().map(|p| p.pid).collect();

        // Build parent-child relationships
        let process_count = processes.len();
        let mut children_map: HashMap<u32, Vec<ProcessInfo>> = HashMap::with_capacity(process_count / 4);
        let mut root_processes: Vec<ProcessInfo> = Vec::with_capacity(process_count / 8);

        // Group by parent - a process is a root if:
        // 1. parent_pid == 0 (no parent)
        // 2. parent_pid == pid (self-referential)
        // 3. parent_pid is not in our process list (orphan)
        for proc in processes {
            let is_root = proc.parent_pid == 0
                || proc.parent_pid == proc.pid
                || !all_pids.contains(&proc.parent_pid);

            if is_root {
                root_processes.push(proc);
            } else {
                children_map.entry(proc.parent_pid).or_default().push(proc);
            }
        }

        // Sort roots by PID
        root_processes.sort_by(|a, b| a.pid.cmp(&b.pid));

        // Build tree recursively
        let mut result = Vec::new();
        let root_count = root_processes.len();
        for (idx, root) in root_processes.into_iter().enumerate() {
            let is_last = idx == root_count - 1;
            self.add_tree_node(&mut result, root, &mut children_map, 0, is_last, String::new());
        }

        // Collect orphaned processes (e.g. from PID reuse cycles) that weren't
        // reached from any root and add them as top-level entries.
        // After tree traversal, only unreached entries remain in the map.
        for (_, children) in children_map.drain() {
            for mut orphan in children {
                orphan.tree_depth = 0;
                orphan.tree_prefix = String::new();
                result.push(orphan);
            }
        }

        result
    }

    fn add_tree_node(
        &self,
        result: &mut Vec<ProcessInfo>,
        mut process: ProcessInfo,
        children_map: &mut std::collections::HashMap<u32, Vec<ProcessInfo>>,
        depth: usize,
        is_last: bool,
        parent_prefix: String,
    ) {
        // Guard against cycles from PID reuse and excessively deep trees
        const MAX_TREE_DEPTH: usize = 64;
        if depth >= MAX_TREE_DEPTH {
            return;
        }

        process.tree_depth = depth;
        let pid = process.pid;
        let has_children = children_map.contains_key(&pid);
        let is_collapsed = self.collapsed_pids.contains(&pid);
        process.has_children = has_children;
        process.is_collapsed = is_collapsed;

        // Build the tree prefix for display
        // Use push_str instead of format! to reduce allocations
        if depth > 0 {
            let branch = if is_last { "└─ " } else { "├─ " };
            let mut prefix = String::with_capacity(parent_prefix.len() + 6);
            prefix.push_str(&parent_prefix);
            prefix.push_str(branch);
            process.tree_prefix = prefix;
        } else {
            process.tree_prefix = String::new();
        }

        result.push(process);

        // Only add children if not collapsed - take ownership to avoid cloning
        if !is_collapsed
            && let Some(mut sorted_children) = children_map.remove(&pid) {
                sorted_children.sort_by(|a, b| a.pid.cmp(&b.pid));
                let child_count = sorted_children.len();

                // Calculate the prefix for children
                // Use push_str instead of format! to reduce allocations
                let child_parent_prefix = if depth > 0 {
                    let connector = if is_last { "   " } else { "│  " };
                    let mut prefix = String::with_capacity(parent_prefix.len() + 3);
                    prefix.push_str(&parent_prefix);
                    prefix.push_str(connector);
                    prefix
                } else {
                    String::new()
                };

                for (idx, child) in sorted_children.into_iter().enumerate() {
                    let child_is_last = idx == child_count - 1;
                    self.add_tree_node(result, child, children_map, depth + 1, child_is_last, child_parent_prefix.clone());
                }
            }
    }

    /// Collapse tree branch at selected process
    pub fn collapse_tree(&mut self) {
        let pid = self.selected_process().map(|p| p.pid);
        if let Some(pid) = pid {
            self.collapsed_pids.insert(pid);
            self.needs_process_update = true;
        }
    }

    /// Expand tree branch at selected process
    pub fn expand_tree(&mut self) {
        let pid = self.selected_process().map(|p| p.pid);
        if let Some(pid) = pid {
            self.collapsed_pids.remove(&pid);
            self.needs_process_update = true;
        }
    }

    /// Collapse all tree branches
    pub fn collapse_all(&mut self) {
        // Collapse all processes that have children
        for proc in &self.processes {
            self.collapsed_pids.insert(proc.pid);
        }
        self.needs_process_update = true;
    }

    /// Expand all tree branches
    pub fn expand_all(&mut self) {
        self.collapsed_pids.clear();
        self.needs_process_update = true;
    }

    /// Move selection up
    pub fn select_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.ensure_visible();
        }
    }

    /// Move selection down
    pub fn select_down(&mut self) {
        if self.selected_index < self.displayed_processes.len().saturating_sub(1) {
            self.selected_index += 1;
            self.ensure_visible();
        }
    }

    /// Page up
    pub fn page_up(&mut self) {
        let page_size = self.visible_height.saturating_sub(1);
        self.selected_index = self.selected_index.saturating_sub(page_size);
        self.ensure_visible();
    }

    /// Page down
    pub fn page_down(&mut self) {
        let page_size = self.visible_height.saturating_sub(1);
        self.selected_index = (self.selected_index + page_size)
            .min(self.displayed_processes.len().saturating_sub(1));
        self.ensure_visible();
    }

    /// Go to first process
    pub fn select_first(&mut self) {
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Go to last process
    pub fn select_last(&mut self) {
        self.selected_index = self.displayed_processes.len().saturating_sub(1);
        self.ensure_visible();
    }

    /// Ensure selected item is visible
    fn ensure_visible(&mut self) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + self.visible_height {
            self.scroll_offset = self.selected_index - self.visible_height + 1;
        }
    }

    /// Toggle tag on selected process
    pub fn toggle_tag(&mut self) {
        if let Some(proc) = self.displayed_processes.get(self.selected_index) {
            let pid = proc.pid;
            if self.tagged_pids.contains(&pid) {
                self.tagged_pids.remove(&pid);
            } else {
                self.tagged_pids.insert(pid);
            }
        }
    }

    /// Untag all processes
    pub fn untag_all(&mut self) {
        self.tagged_pids.clear();
    }

    /// Tag all processes with the same name as the selected process
    pub fn tag_all_by_name(&mut self) {
        if let Some(proc) = self.selected_process() {
            let name = proc.name.clone();
            // Find all visible processes with the same name and tag them
            let pids_to_tag: Vec<u32> = self.displayed_processes
                .iter()
                .filter(|p| p.name == name)
                .map(|p| p.pid)
                .collect();
            for pid in pids_to_tag {
                self.tagged_pids.insert(pid);
            }
        }
    }

    /// Toggle tag on all visible/filtered processes
    pub fn tag_all_visible(&mut self) {
        // If all visible are already tagged, untag them
        let all_tagged = self.displayed_processes
            .iter()
            .all(|p| self.tagged_pids.contains(&p.pid));

        if all_tagged {
            // Untag all visible
            for proc in &self.displayed_processes {
                self.tagged_pids.remove(&proc.pid);
            }
        } else {
            // Tag all visible
            for proc in &self.displayed_processes {
                self.tagged_pids.insert(proc.pid);
            }
        }
    }

    /// Get selected process
    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        self.displayed_processes.get(self.selected_index)
    }

    /// Toggle tree view
    pub fn toggle_tree_view(&mut self) {
        self.tree_view = !self.tree_view;
        self.needs_process_update = true;
    }

    /// Set sort column
    pub fn set_sort_column(&mut self, column: SortColumn) {
        if self.sort_column == column {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_column = column;
            self.sort_ascending = false;
        }
        self.needs_process_update = true;
    }

    /// Apply filter from dialog input buffer
    pub fn apply_filter(&mut self) {
        if let DialogState::Filter { ref buffer, .. } = self.dialog {
            self.filter_string = buffer.clone();
            self.filter_string_lower = self.filter_string.to_lowercase();
            self.needs_process_update = true;
        }
    }

    /// Apply search from dialog input buffer
    pub fn apply_search(&mut self) {
        if let DialogState::Search { ref buffer, .. } = self.dialog {
            self.search_string = buffer.clone();
        } else {
            return;
        }
        self.search_string_lower = self.search_string.to_lowercase();
        // Find first matching process using pre-computed lowercase strings
        if !self.search_string_lower.is_empty()
            && let Some(idx) = self.displayed_processes.iter().position(|p| {
                p.name_lower.contains(&self.search_string_lower)
                    || p.command_lower.contains(&self.search_string_lower)
            }) {
                self.selected_index = idx;
                self.ensure_visible();
            }
        // Update matches_search flags for highlighting
        self.needs_process_update = true;
    }

    /// Find next search match
    pub fn find_next(&mut self) {
        if self.search_string_lower.is_empty() {
            return;
        }
        let start = self.selected_index + 1;
        for i in 0..self.displayed_processes.len() {
            let idx = (start + i) % self.displayed_processes.len();
            let p = &self.displayed_processes[idx];
            // Use pre-computed lowercase strings
            if p.name_lower.contains(&self.search_string_lower)
                || p.command_lower.contains(&self.search_string_lower)
            {
                self.selected_index = idx;
                self.ensure_visible();
                break;
            }
        }
    }

    /// Kill the captured target process (used by kill confirmation dialog)
    pub fn kill_target_process(&mut self, signal: u32) {
        let (pid, name) = match &self.dialog {
            DialogState::Kill { pid, name, .. }
            | DialogState::SignalSelect { pid, name, .. } => (*pid, name.clone()),
            _ => return,
        };
        self.kill_process_by(pid, &name, signal);
    }

    /// Kill a process by PID (direct, no dialog needed)
    fn kill_process_by(&mut self, pid: u32, name: &str, signal: u32) {
        match crate::system::kill_process(pid, signal) {
            Ok(_) => {
                self.status_message = Some((
                    format!("Killed {} (PID {})", name, pid),
                    Instant::now(),
                ));
            }
            Err(e) => {
                self.last_error = Some((format!("Failed to kill {} ({}): {}", name, pid, e), Instant::now()));
            }
        }
    }

    /// Kill all tagged processes
    pub fn kill_tagged(&mut self, signal: u32) {
        let pids: Vec<u32> = self.tagged_pids.iter().copied().collect();
        let total = pids.len();
        let mut killed = 0;
        let mut failed = 0;

        for pid in pids {
            match crate::system::kill_process(pid, signal) {
                Ok(_) => killed += 1,
                Err(e) => {
                    failed += 1;
                    self.last_error = Some((format!("Failed to kill process {}: {}", pid, e), Instant::now()));
                }
            }
        }

        if failed == 0 {
            self.status_message = Some((
                format!("Killed {} process{}", killed, if killed == 1 { "" } else { "es" }),
                Instant::now(),
            ));
        } else {
            self.status_message = Some((
                format!("Killed {}/{} processes ({} failed)", killed, total, failed),
                Instant::now(),
            ));
        }

        self.tagged_pids.clear();
    }

    /// Set priority class for selected process
    pub fn set_priority_selected(&mut self, priority_class: WindowsPriorityClass) {
        let pid = match &self.dialog {
            DialogState::Priority { pid, .. } => *pid,
            _ => return,
        };
        if let Err(e) = crate::system::set_priority_class(pid, priority_class) {
            self.last_error = Some((format!("Failed to set priority for {}: {}", pid, e), Instant::now()));
        }
    }

    /// Toggle efficiency mode for selected process
    pub fn toggle_efficiency_mode(&mut self) {
        let (pid, name) = match &self.dialog {
            DialogState::Priority { pid, name, .. } => (*pid, name.clone()),
            _ => return,
        };
        // Get current efficiency mode status
        let current = self.selected_process()
            .map(|p| p.efficiency_mode)
            .unwrap_or(false);
        let new_state = !current;
        match crate::system::set_efficiency_mode(pid, new_state) {
            Ok(_) => {
                let state_str = if new_state { "enabled" } else { "disabled" };
                self.status_message = Some((
                    format!("Efficiency mode {} for {}", state_str, name),
                    Instant::now(),
                ));
                for proc in &mut self.displayed_processes {
                    if proc.pid == pid {
                        proc.efficiency_mode = new_state;
                        break;
                    }
                }
                for proc in &mut self.processes {
                    if proc.pid == pid {
                        proc.efficiency_mode = new_state;
                        break;
                    }
                }
            }
            Err(e) => {
                self.last_error = Some((format!("Failed to set efficiency mode: {}", e), Instant::now()));
            }
        }
    }

    /// Clear error message
    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    /// Add character to input buffer
    pub fn input_char(&mut self, c: char) {
        if let Some((buffer, cursor)) = self.dialog.input_buffer_mut() {
            buffer.insert(*cursor, c);
            *cursor += 1;
        }
    }

    /// Delete character before cursor
    pub fn input_backspace(&mut self) {
        if let Some((buffer, cursor)) = self.dialog.input_buffer_mut() {
            if *cursor > 0 {
                *cursor -= 1;
                buffer.remove(*cursor);
            }
        }
    }

    /// Delete character at cursor
    pub fn input_delete(&mut self) {
        if let Some((buffer, cursor)) = self.dialog.input_buffer_mut() {
            if *cursor < buffer.len() {
                buffer.remove(*cursor);
            }
        }
    }

    /// Move cursor left
    pub fn input_left(&mut self) {
        if let Some((_, cursor)) = self.dialog.input_buffer_mut() {
            if *cursor > 0 {
                *cursor -= 1;
            }
        }
    }

    /// Move cursor right
    pub fn input_right(&mut self) {
        if let Some((buffer, cursor)) = self.dialog.input_buffer_mut() {
            if *cursor < buffer.len() {
                *cursor += 1;
            }
        }
    }

    /// Start search mode
    pub fn start_search(&mut self) {
        let buffer = self.search_string.clone();
        let cursor = buffer.len();
        self.dialog = DialogState::Search { buffer, cursor };
    }

    /// Start filter mode
    pub fn start_filter(&mut self) {
        let buffer = self.filter_string.clone();
        let cursor = buffer.len();
        self.dialog = DialogState::Filter { buffer, cursor };
    }

    /// Exit current mode
    pub fn exit_mode(&mut self) {
        self.dialog = DialogState::None;
    }

    /// Tag selected process and all its children
    pub fn tag_with_children(&mut self) {
        let pid = self.selected_process().map(|p| p.pid);
        if let Some(pid) = pid {
            self.tagged_pids.insert(pid);
            // Find and tag all descendants
            self.tag_descendants(pid);
        }
    }

    /// Recursively tag all descendants of a process
    fn tag_descendants(&mut self, parent_pid: u32) {
        let children: Vec<u32> = self.processes
            .iter()
            .filter(|p| p.parent_pid == parent_pid)
            .map(|p| p.pid)
            .collect();

        for child_pid in children {
            self.tagged_pids.insert(child_pid);
            self.tag_descendants(child_pid);
        }
    }

    /// Collect all descendant PIDs of a process (recursive)
    fn collect_descendants(&self, parent_pid: u32, result: &mut Vec<u32>) {
        for proc in &self.processes {
            if proc.parent_pid == parent_pid {
                result.push(proc.pid);
                self.collect_descendants(proc.pid, result);
            }
        }
    }

    /// Toggle tag for a process and all its descendants (for tree mode double-click)
    pub fn toggle_tag_branch(&mut self, pid: u32) {
        // Collect the process and all its descendants
        let mut branch_pids = vec![pid];
        self.collect_descendants(pid, &mut branch_pids);

        // Check if all are already tagged
        let all_tagged = branch_pids.iter().all(|p| self.tagged_pids.contains(p));

        if all_tagged {
            // Untag all
            for p in branch_pids {
                self.tagged_pids.remove(&p);
            }
        } else {
            // Tag all
            for p in branch_pids {
                self.tagged_pids.insert(p);
            }
        }
    }

    /// Enter user select mode
    pub fn enter_user_select_mode(&mut self) {
        let mut users: Vec<String> = self.processes
            .iter()
            .map(|p| p.user.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        users.sort();

        let index = if let Some(ref filter) = self.user_filter {
            users.iter().position(|u| u == filter).map(|i| i + 1).unwrap_or(0)
        } else {
            0
        };

        self.dialog = DialogState::UserSelect { index, users };
    }

    /// Toggle follow mode
    pub fn toggle_follow_mode(&mut self) {
        if self.follow_pid.is_some() {
            self.follow_pid = None;
        } else if let Some(proc) = self.selected_process() {
            self.follow_pid = Some(proc.pid);
        }
    }

    /// Enter environment view mode
    pub fn enter_environment_mode(&mut self) {
        if self.selected_process().is_some() {
            self.dialog = DialogState::Environment { scroll: 0 };
        }
    }

    /// Enter command wrap view mode
    pub fn enter_command_wrap_mode(&mut self) {
        if self.selected_process().is_some() {
            self.dialog = DialogState::CommandWrap { scroll: 0 };
        }
    }

    /// Enter CPU affinity mode
    pub fn enter_affinity_mode(&mut self) {
        if let Some(proc) = self.selected_process() {
            let cpu_count = self.system_metrics.cpu.core_usage.len();
            let all_cpus = if cpu_count >= 64 { u64::MAX } else { (1u64 << cpu_count) - 1 };
            let mask = crate::system::get_process_affinity(proc.pid).unwrap_or(all_cpus);
            self.dialog = DialogState::Affinity { mask, selected: 0 };
        }
    }

    /// Apply CPU affinity to selected process
    pub fn apply_affinity(&mut self) {
        let mask = match &self.dialog {
            DialogState::Affinity { mask, .. } => *mask,
            _ => return,
        };
        if mask == 0 {
            self.last_error = Some(("Cannot set empty affinity mask".to_string(), Instant::now()));
            return;
        }
        if let Some(proc) = self.selected_process() {
            if let Err(e) = crate::system::set_process_affinity(proc.pid, mask) {
                self.last_error = Some((format!("Failed to set affinity: {}", e), Instant::now()));
            }
        }
    }

    /// Handle digit key for PID search
    pub fn handle_pid_digit(&mut self, digit: char) {
        use std::time::Duration;

        let now = Instant::now();

        // Clear buffer if too much time has passed (1 second timeout)
        // or if buffer exceeds max PID length (u32 max is 10 digits)
        if let Some(last_time) = self.pid_search_time
            && now.duration_since(last_time) > Duration::from_secs(1) {
                self.pid_search_buffer.clear();
            }
        if self.pid_search_buffer.len() >= 10 {
            self.pid_search_buffer.clear();
        }

        // Add digit to buffer
        self.pid_search_buffer.push(digit);
        self.pid_search_time = Some(now);

        // Search for PID starting with these digits
        if let Ok(search_pid) = self.pid_search_buffer.parse::<u32>() {
            // Find first process with PID >= search_pid
            for (idx, proc) in self.displayed_processes.iter().enumerate() {
                if proc.pid >= search_pid {
                    self.selected_index = idx;
                    self.ensure_visible();
                    break;
                }
            }
        }
    }

    /// Collapse to parent in tree view
    pub fn collapse_to_parent(&mut self) {
        if let Some(proc) = self.selected_process() {
            let parent_pid = proc.parent_pid;
            // Find parent in displayed processes and select it
            for (idx, p) in self.displayed_processes.iter().enumerate() {
                if p.pid == parent_pid {
                    self.selected_index = idx;
                    self.ensure_visible();
                    // Collapse the parent
                    self.collapsed_pids.insert(parent_pid);
                    self.needs_process_update = true;
                    break;
                }
            }
        }
    }

    /// Enter column configuration mode
    pub fn enter_column_config_mode(&mut self) {
        self.dialog = DialogState::ColumnConfig { index: 0 };
    }
}
