use crate::config::Config;
use crate::system::{ProcessIdentity, ProcessInfo, SystemMetrics};
use crate::terminal::Rect;
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
    /// GPU meter bar (only present on machines with a GPU)
    GpuMeter,
    /// NPU meter bar (only present on NPU machines)
    NpuMeter,
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
        Self::all()
            .get(index)
            .copied()
            .unwrap_or(WindowsPriorityClass::Normal)
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
        Self {
            element,
            x,
            y,
            width,
            height,
        }
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
        self.function_keys
            .push(UIRegion::new(UIElement::FunctionKey(key), x, y, width, 1));
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
            && let Some(col) = self.column_at_x(x)
        {
            return Some(UIElement::ColumnHeader(col));
        }

        if let Some(row_index) = self.process_row_index(y) {
            // Note: PID needs to be filled in by caller who has process data
            return Some(UIElement::ProcessRow {
                index: row_index,
                pid: 0,
            });
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
    Elevated,   // Running as admin
    Arch,       // Process architecture (x86/x64/ARM)
    Efficiency, // Efficiency mode (EcoQoS)
    // I/O columns
    HandleCount, // Number of open handles
    IoRate,      // Combined I/O rate (read + write bytes/sec)
    IoReadRate,  // I/O read bytes/sec
    IoWriteRate, // I/O write bytes/sec
    IoRead,      // Cumulative I/O read bytes
    IoWrite,     // Cumulative I/O write bytes
    // GPU columns (Task Manager parity; only meaningful on GPU machines)
    Gpu,    // GPU utilization percent (max across all GPU engine nodes)
    GpuMem, // GPU committed memory across all GPU adapters
    // NPU columns (Task Manager parity; only meaningful on NPU machines)
    Npu,    // NPU utilization percent
    NpuMem, // NPU dedicated + shared memory
}

impl SortColumn {
    /// Canonical display order. This drives the F6 sort menu, the column
    /// picker listing, and where a newly enabled column is inserted
    /// (`App::toggle_column_in_active_tab`): identity, scheduling, memory,
    /// status, usage (CPU/MEM/GPU/NPU together, Task Manager style), I/O,
    /// attributes, times, and Command last since it expands to fill the row.
    /// The default Main and I/O tab layouts follow this relative order.
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
            SortColumn::Gpu,
            SortColumn::GpuMem,
            SortColumn::Npu,
            SortColumn::NpuMem,
            SortColumn::IoRate,
            SortColumn::IoReadRate,
            SortColumn::IoWriteRate,
            SortColumn::IoRead,
            SortColumn::IoWrite,
            SortColumn::HandleCount,
            SortColumn::Elevated,
            SortColumn::Arch,
            SortColumn::Efficiency,
            SortColumn::StartTime,
            SortColumn::Time,
            SortColumn::Command,
        ]
    }

    /// Position in the canonical display order (`usize::MAX` for unknown names).
    fn display_rank(name: &str) -> usize {
        SortColumn::from_name(name)
            .and_then(|col| SortColumn::all().iter().position(|c| *c == col))
            .unwrap_or(usize::MAX)
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
            SortColumn::Gpu => "GPU%",
            SortColumn::GpuMem => "GPU-MEM",
            SortColumn::Npu => "NPU%",
            SortColumn::NpuMem => "NPU-MEM",
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
            "GPU%" => Some(SortColumn::Gpu),
            "GPU-MEM" => Some(SortColumn::GpuMem),
            "NPU%" => Some(SortColumn::Npu),
            "NPU-MEM" => Some(SortColumn::NpuMem),
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
            SortColumn::Gpu => 6,
            SortColumn::GpuMem => 8,
            SortColumn::Npu => 6,
            SortColumn::NpuMem => 8,
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
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            sort_column: SortColumn::IoRate,
            sort_ascending: false,
        }
    }
}

/// One row of the F2 Setup dialog, in display order via `SetupItem::ALL`.
///
/// `draw_setup` renders each item's label and current value, and
/// `handle_setup_keys` dispatches Enter/Left/Right on the item — both iterate
/// this table, so the rendered list and the input handling can never disagree
/// about item order or count (issue #27 was caused by exactly that: the draw
/// list and the numeric match arms drifting apart).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupItem {
    RefreshRate,
    CpuMeterMode,
    MemoryMeterMode,
    GpuMeterMode,
    NpuMeterMode,
    ShowKernelThreads,
    ShowUserThreads,
    ShowProgramPath,
    HighlightNewProcesses,
    HighlightLargeNumbers,
    TreeView,
    ConfirmKill,
    ColorScheme,
    ConfigureColumns,
    GpuMeterAdapter,
    // Destructive action, always kept last in the list.
    ResetAllSettings,
}

impl SetupItem {
    pub const ALL: &'static [SetupItem] = &[
        SetupItem::RefreshRate,
        SetupItem::CpuMeterMode,
        SetupItem::MemoryMeterMode,
        SetupItem::GpuMeterMode,
        SetupItem::NpuMeterMode,
        SetupItem::ShowKernelThreads,
        SetupItem::ShowUserThreads,
        SetupItem::ShowProgramPath,
        SetupItem::HighlightNewProcesses,
        SetupItem::HighlightLargeNumbers,
        SetupItem::TreeView,
        SetupItem::ConfirmKill,
        SetupItem::ColorScheme,
        SetupItem::ConfigureColumns,
        SetupItem::GpuMeterAdapter,
        SetupItem::ResetAllSettings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SetupItem::RefreshRate => "Refresh rate",
            SetupItem::CpuMeterMode => "CPU meter mode",
            SetupItem::MemoryMeterMode => "Memory meter mode",
            SetupItem::GpuMeterMode => "GPU meter mode",
            SetupItem::NpuMeterMode => "NPU meter mode",
            SetupItem::ShowKernelThreads => "Show kernel threads",
            SetupItem::ShowUserThreads => "Show user threads",
            SetupItem::ShowProgramPath => "Show program path",
            SetupItem::HighlightNewProcesses => "Highlight new processes",
            SetupItem::HighlightLargeNumbers => "Highlight large numbers",
            SetupItem::TreeView => "Tree view",
            SetupItem::ConfirmKill => "Confirm force terminate",
            SetupItem::ColorScheme => "Color scheme",
            SetupItem::ConfigureColumns => "Configure columns",
            SetupItem::GpuMeterAdapter => "GPU meter adapter",
            SetupItem::ResetAllSettings => "Reset all settings",
        }
    }

    /// Stable row lookup for submenus that return to Setup.
    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|item| *item == self)
            .expect("every SetupItem must be present in SetupItem::ALL")
    }
}

/// Dialog/view state - encapsulates per-dialog state into enum variants
/// Replaces the previous flat ViewMode enum + scattered dialog fields
#[derive(Debug, Clone, Default)]
pub enum DialogState {
    /// Normal process list view (no dialog open)
    #[default]
    None,
    Help {
        scroll: usize,
    },
    Search {
        buffer: String,
        cursor: usize,
        original: String,
        original_selection: Option<ProcessIdentity>,
    },
    Filter {
        buffer: String,
        cursor: usize,
        original: String,
        original_selection: Option<ProcessIdentity>,
    },
    SortSelect {
        index: usize,
    },
    Kill {
        identity: ProcessIdentity,
        name: String,
        command: String,
    },
    Priority {
        class_index: usize,
        identity: ProcessIdentity,
        name: String,
    },
    Setup {
        selected: usize,
    },
    ProcessInfo {
        target: Box<ProcessInfo>,
        scroll: usize,
    },
    UserSelect {
        index: usize,
        users: Vec<String>,
    },
    Environment {
        scroll: usize,
        identity: ProcessIdentity,
    },
    ColorScheme {
        index: usize,
    },
    /// GPU adapter selector for the meter (index 0 = Auto, then `names`).
    GpuSelect {
        index: usize,
        names: Vec<String>,
    },
    CommandWrap {
        scroll: usize,
        identity: ProcessIdentity,
    },
    ColumnConfig {
        index: usize,
    },
    Affinity {
        mask: u64,
        selected: usize,
        identity: ProcessIdentity,
    },
}

impl DialogState {
    /// Get mutable reference to input buffer and cursor (Search/Filter dialogs)
    pub fn input_buffer_mut(&mut self) -> Option<(&mut String, &mut usize)> {
        match self {
            DialogState::Search { buffer, cursor, .. }
            | DialogState::Filter { buffer, cursor, .. } => Some((buffer, cursor)),
            _ => None,
        }
    }

    /// Get input buffer contents (Search/Filter dialogs)
    pub fn input_buffer(&self) -> Option<(&str, usize)> {
        match self {
            DialogState::Search { buffer, cursor, .. }
            | DialogState::Filter { buffer, cursor, .. } => Some((buffer, *cursor)),
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
    /// Metadata dependencies fulfilled by the collector for `processes`.
    canonical_enrichment: crate::system::ProcessEnrichmentRequirements,
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
    /// Open the authoritative user picker after the collector finishes its
    /// one-time all-process user lookup.
    pending_user_select: bool,
    /// PID filter (show only these PIDs) - from CLI -p option (HashSet for O(1) lookup)
    pub pid_filter: Option<HashSet<u32>>,
    /// Tagged process identities. The field keeps its historical name for API
    /// compatibility, but values include creation time so PID reuse is safe.
    pub tagged_pids: HashSet<ProcessIdentity>,
    /// Process list visible height (set during render)
    pub visible_height: usize,
    /// Terminal width in columns (set during render, used for responsive header layout)
    pub terminal_width: u16,
    /// Last error message with timestamp for auto-expiry
    pub last_error: Option<(String, Instant)>,
    /// Status message (success/info) with timestamp for auto-expiry
    pub status_message: Option<(String, Instant)>,
    /// Set when a hot path (meter clicks, arrow-key meter cycling) changes the
    /// config; flushed to disk at most once per tick and on exit, instead of
    /// doing a synchronous file write per input event.
    pub config_dirty: bool,
    /// A write was already attempted for the current dirty state and failed.
    /// The tick loop must not retry it continuously and consume user input by
    /// re-arming the same error on every refresh.
    config_save_failed: bool,

    /// Collapsed process identities in tree view.
    pub collapsed_pids: HashSet<ProcessIdentity>,
    /// Follow mode: stable identity to follow across refreshes.
    pub follow_pid: Option<ProcessIdentity>,
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
    pub gpu_history: VecDeque<f32>,
    pub npu_history: VecDeque<f32>,
    /// Cached visible columns (updated when column config changes)
    pub cached_visible_columns: Vec<SortColumn>,
    /// Deferred process list update flag (flushed once before each render)
    pub needs_process_update: bool,
    /// UI layout bounds (populated during render for accurate mouse/keyboard navigation)
    pub ui_bounds: UIBounds,

    // Cached dialog geometry (written during render, read by the mouse handler so
    // hit-testing matches exactly what was drawn — mirrors the ui_bounds pattern).
    /// Full dialog rect including its border.
    pub dialog_area: Option<Rect>,
    /// Inner content rect (inside the border).
    pub dialog_inner: Option<Rect>,
    /// Index of the first visible scrollable list item this frame.
    pub dialog_list_offset: usize,
    /// Non-selectable rows pinned to the top of a list dialog (for row→index mapping).
    pub dialog_header_rows: usize,
    /// Number of selectable list rows currently visible (excludes header/footer),
    /// so a click below them (on a footer row) maps to no item.
    pub dialog_scroll_rows: usize,

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

    /// Readonly policy supplied by the command line. Unlike persisted config,
    /// this cannot be cleared by Reset All Settings during the session.
    runtime_readonly: bool,
}

fn clamp_char_boundary(buffer: &str, cursor: usize) -> usize {
    let cursor = cursor.min(buffer.len());
    if buffer.is_char_boundary(cursor) {
        cursor
    } else {
        prev_char_boundary(buffer, cursor)
    }
}

fn prev_char_boundary(buffer: &str, cursor: usize) -> usize {
    let cursor = cursor.min(buffer.len());
    buffer
        .char_indices()
        .map(|(idx, _)| idx)
        .take_while(|idx| *idx < cursor)
        .last()
        .unwrap_or(0)
}

fn next_char_boundary(buffer: &str, cursor: usize) -> usize {
    let cursor = clamp_char_boundary(buffer, cursor);
    buffer[cursor..]
        .chars()
        .next()
        .map(|ch| cursor + ch.len_utf8())
        .unwrap_or(buffer.len())
}

impl App {
    fn readonly_blocked(&mut self, action: &str) -> bool {
        if self.runtime_readonly || self.config.readonly {
            self.last_error = Some((format!("Readonly mode: cannot {action}"), Instant::now()));
            true
        } else {
            false
        }
    }

    pub fn new(config: Config) -> Self {
        let theme = config.theme();
        let tree_view = config.tree_view_default;
        let screen_tabs = config
            .screen_tabs
            .clone()
            .unwrap_or_else(|| vec![ScreenTab::default_main(&config), ScreenTab::default_io()]);
        let cached_visible_columns: Vec<SortColumn> = screen_tabs
            .first()
            .map(|tab| {
                tab.columns
                    .iter()
                    .filter_map(|name| SortColumn::from_name(name))
                    .collect()
            })
            .unwrap_or_else(|| Self::compute_visible_columns(&config));
        let (sort_column, sort_ascending) = screen_tabs
            .first()
            .map(|tab| (tab.sort_column, tab.sort_ascending))
            .unwrap_or((SortColumn::Cpu, false));
        Self {
            config,
            theme,
            dialog: DialogState::None,
            system_metrics: SystemMetrics::default(),
            processes: Vec::new(),
            canonical_enrichment: Default::default(),
            displayed_processes: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            sort_column,
            sort_ascending,
            tree_view,
            search_string: String::new(),
            search_string_lower: String::new(),
            filter_string: String::new(),
            filter_string_lower: String::new(),
            user_filter: None,
            pending_user_select: false,
            pid_filter: None,
            tagged_pids: HashSet::new(),
            visible_height: 20,
            terminal_width: 80,
            last_error: None,
            status_message: None,
            config_dirty: false,
            config_save_failed: false,
            collapsed_pids: HashSet::new(),
            follow_pid: None,
            paused: false,
            pid_search_buffer: String::new(),
            pid_search_time: None,
            show_header: true,
            max_iterations: None,
            iteration_count: 0,
            cpu_history: Vec::new(),
            gpu_history: VecDeque::new(),
            mem_history: VecDeque::new(),
            swap_history: VecDeque::new(),
            npu_history: VecDeque::new(),
            cached_visible_columns,
            needs_process_update: false,
            ui_bounds: UIBounds::default(),
            dialog_area: None,
            dialog_inner: None,
            dialog_list_offset: 0,
            dialog_header_rows: 0,
            dialog_scroll_rows: 0,
            last_click_pos: None,
            last_click_time: None,
            double_click_ms: 500, // Standard double-click threshold
            update_available: None,
            update_checked: false,
            focus_region: FocusRegion::default(),
            focus_index: 0,
            screen_tabs,
            active_tab: 0,
            runtime_readonly: false,
        }
    }

    /// Apply an immutable readonly policy for this process lifetime.
    pub fn set_runtime_readonly(&mut self, readonly: bool) {
        self.runtime_readonly |= readonly;
    }

    pub fn is_readonly(&self) -> bool {
        self.runtime_readonly || self.config.readonly
    }

    /// Compute visible columns based on config (used for caching)
    /// Respects the order defined in config.visible_columns
    fn compute_visible_columns(config: &Config) -> Vec<SortColumn> {
        config
            .visible_columns
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

    /// Toggle a column's visibility in the active tab. Newly enabled columns
    /// are inserted at their canonical display position relative to the
    /// columns already shown (so e.g. GPU% lands next to CPU%/MEM% and
    /// Command stays last) instead of being appended after Command. Users
    /// can still rearrange with Shift+Up/Down afterwards.
    pub fn toggle_column_in_active_tab(&mut self, column: &str) -> bool {
        if let Some(tab) = self.screen_tabs.get_mut(self.active_tab) {
            if let Some(pos) = tab.columns.iter().position(|c| c == column) {
                if tab.columns.len() == 1 {
                    self.last_error = Some((
                        "At least one process column must remain visible".to_string(),
                        Instant::now(),
                    ));
                    return false;
                }
                tab.columns.remove(pos);
            } else {
                let insert_at = canonical_insert_index(&tab.columns, column);
                tab.columns.insert(insert_at, column.to_string());
            }
        } else {
            return false;
        }
        self.sync_config_from_active_tab();
        self.update_visible_columns_cache();
        self.mark_config_dirty();
        true
    }

    /// Move a column up in the active tab's order
    pub fn move_column_up_in_active_tab(&mut self, column: &str) -> bool {
        if let Some(tab) = self.screen_tabs.get_mut(self.active_tab)
            && let Some(pos) = tab.columns.iter().position(|c| c == column)
            && pos > 0
        {
            tab.columns.swap(pos, pos - 1);
            self.sync_config_from_active_tab();
            self.update_visible_columns_cache();
            self.mark_config_dirty();
            return true;
        }
        false
    }

    /// Move a column down in the active tab's order
    pub fn move_column_down_in_active_tab(&mut self, column: &str) -> bool {
        if let Some(tab) = self.screen_tabs.get_mut(self.active_tab)
            && let Some(pos) = tab.columns.iter().position(|c| c == column)
            && pos < tab.columns.len() - 1
        {
            tab.columns.swap(pos, pos + 1);
            self.sync_config_from_active_tab();
            self.update_visible_columns_cache();
            self.mark_config_dirty();
            return true;
        }
        false
    }

    /// Sync config.visible_columns from the Main tab for backward compatibility.
    /// Per-tab state is persisted in config.screen_tabs; visible_columns is kept
    /// as the legacy Main-tab layout so saving from the I/O tab cannot corrupt
    /// older consumers or the next no-tabs fallback.
    fn sync_config_from_active_tab(&mut self) {
        if let Some(tab) = self.screen_tabs.first() {
            self.config.visible_columns = tab.columns.clone();
        }
    }

    /// Reset screen tabs to defaults and apply. The Main tab defaults are
    /// hardware-aware: GPU/NPU columns are included when the adapter exists,
    /// so a reset doesn't strip columns the machine actually supports.
    pub fn reset_screen_tabs(&mut self) {
        let mut main = ScreenTab::default_main(&Config::default());
        main.columns = hardware_default_columns(
            self.system_metrics.gpu.is_some(),
            self.system_metrics.npu.is_some(),
        );
        self.screen_tabs = vec![main, ScreenTab::default_io()];
        self.active_tab = 0;
        self.apply_active_tab();
    }

    /// First-run setup: include GPU/NPU columns in the Main tab now that
    /// hardware detection has run (adapter info isn't known at App::new).
    /// Only called when no config file existed, so no user layout is touched.
    pub fn apply_hardware_default_columns(&mut self) {
        if let Some(tab) = self.screen_tabs.first_mut() {
            tab.columns = hardware_default_columns(
                self.system_metrics.gpu.is_some(),
                self.system_metrics.npu.is_some(),
            );
        }
        if self.active_tab == 0 {
            self.sync_config_from_active_tab();
            self.update_visible_columns_cache();
        }
    }

    /// Update the color theme from config
    pub fn update_theme(&mut self) {
        self.theme = self.config.theme();
    }

    /// Save the current configuration (syncs screen tab state first)
    pub fn save_config(&mut self) -> bool {
        // Sync current sort settings to active tab before saving
        if let Some(tab) = self.screen_tabs.get_mut(self.active_tab) {
            tab.sort_column = self.sort_column;
            tab.sort_ascending = self.sort_ascending;
        }
        self.sync_config_from_active_tab();
        self.config.screen_tabs = Some(self.screen_tabs.clone());
        let result = self.config.save().map_err(|error| error.to_string());
        self.record_config_save_result(result)
    }

    fn record_config_save_result(&mut self, result: Result<(), String>) -> bool {
        match result {
            Ok(()) => {
                self.config_dirty = false;
                self.config_save_failed = false;
                true
            }
            Err(error) => {
                self.config_dirty = true;
                self.config_save_failed = true;
                self.last_error = Some((format!("Failed to save config: {error}"), Instant::now()));
                false
            }
        }
    }

    /// Mark the config as changed without writing it. Hot paths (meter
    /// clicks, arrow-key meter cycling) use this instead of a synchronous
    /// file write per input event; the main loop flushes once per tick.
    pub fn mark_config_dirty(&mut self) {
        self.config_dirty = true;
        self.config_save_failed = false;
    }

    /// Write a newly-dirtied config once. A failed write remains dirty but is
    /// not retried on every tick; the next mutation or final exit can retry it.
    pub fn flush_config(&mut self) -> bool {
        if self.config_dirty && !self.config_save_failed {
            self.save_config()
        } else {
            !self.config_dirty
        }
    }

    /// Retry any pending config write once during orderly shutdown.
    pub fn retry_config_save(&mut self) -> bool {
        if self.config_dirty {
            self.config_save_failed = false;
            self.save_config()
        } else {
            true
        }
    }

    /// Synchronize persisted preferences into state cached by the live UI.
    pub fn apply_config_to_live_state(&mut self) {
        self.tree_view = self.config.tree_view_default;
        self.update_theme();
        self.update_visible_columns_cache();
        self.show_header = self.config.show_cpu_meters
            || self.config.show_memory_meter
            || self.config.show_gpu_meter
            || self.config.show_npu_meter;
        if !self.tree_view {
            self.collapsed_pids.clear();
        }
        self.refresh_adapter_collection_flags();
        self.needs_process_update = true;
    }

    /// Reset persisted preferences while preserving immutable runtime policy.
    pub fn reset_settings(&mut self) {
        self.config.reset_to_defaults();
        self.reset_screen_tabs();
        self.apply_config_to_live_state();
        self.mark_config_dirty();
        self.save_config();
        self.status_message = Some(("Settings reset to defaults".to_string(), Instant::now()));
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
        }
        self.sync_config_from_active_tab();
        self.update_visible_columns_cache();
        self.needs_process_update = true;
    }

    /// Navigate left within the current focus region
    pub fn navigate_left(&mut self) {
        match self.focus_region {
            FocusRegion::Header => {
                // Cycle through meter modes (persisted, like meter clicks)
                self.config.cpu_meter_mode = self.config.cpu_meter_mode.next();
                self.mark_config_dirty();
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
                // Cycle through meter modes (persisted, like meter clicks)
                self.config.memory_meter_mode = self.config.memory_meter_mode.next();
                self.mark_config_dirty();
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

    /// Toggle header visibility (`#` key, or Enter with header focus).
    ///
    /// Hiding leaves no on-screen meters, so surface the recovery key in the
    /// status line (issue #28). Showing again rescues configs where every
    /// primary meter mode is Hidden (older builds allowed click-cycling into
    /// that state) — otherwise the "restored" header would come back with no
    /// meters and `#` would still look broken.
    pub fn toggle_header(&mut self) {
        self.show_header = !self.show_header;
        if !self.show_header {
            self.status_message = Some((
                "Header hidden - press # to restore".to_string(),
                Instant::now(),
            ));
        } else if self.config.rescue_hidden_meters() {
            self.save_config();
            self.status_message =
                Some(("Hidden header meters restored".to_string(), Instant::now()));
        }
    }

    /// A Setup change to a meter's display mode must actually become visible:
    /// re-show the header (it may have been toggled off with `#`) and
    /// re-enable the meter's `show_*` config flag, which no dialog exposes —
    /// a stale `false` in config.json would otherwise silently defeat the
    /// mode change (issue #28 follow-up).
    pub fn ensure_meter_visible(&mut self, item: SetupItem) {
        self.show_header = true;
        match item {
            SetupItem::CpuMeterMode => self.config.show_cpu_meters = true,
            SetupItem::MemoryMeterMode => {
                // Swap shares the memory meter mode.
                self.config.show_memory_meter = true;
                self.config.show_swap_meter = true;
            }
            SetupItem::GpuMeterMode => self.config.show_gpu_meter = true,
            SetupItem::NpuMeterMode => self.config.show_npu_meter = true,
            _ => {}
        }
    }

    /// Activate the currently focused element (Enter/Space)
    pub fn activate_focused(&mut self) -> bool {
        match self.focus_region {
            FocusRegion::Header => {
                self.toggle_header();
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
                self.handle_function_key(key)
            }
        }
    }

    /// Handle a function key from keyboard, focused footer, or mouse.
    /// Returns true when the caller should quit.
    pub fn handle_function_key(&mut self, key: u8) -> bool {
        match key {
            1 => self.dialog = DialogState::Help { scroll: 0 },
            2 => {
                if matches!(self.dialog, DialogState::Setup { .. }) {
                    self.close_setup();
                } else {
                    self.dialog = DialogState::Setup { selected: 0 };
                }
            }
            3 => self.start_search(),
            4 => self.start_filter(),
            5 => self.toggle_tree_view(),
            6 => {
                let index = SortColumn::all()
                    .iter()
                    .position(|column| *column == self.sort_column)
                    .unwrap_or(0);
                self.dialog = DialogState::SortSelect { index };
            }
            7 => self.enter_priority_mode(1),
            8 => self.enter_priority_mode(-1),
            9 => self.enter_kill_mode(),
            10 => return true,
            _ => {}
        }
        false
    }

    /// Enter kill mode and capture the target process
    pub fn enter_kill_mode(&mut self) {
        if self.readonly_blocked("kill processes") {
            return;
        }
        if let Some(proc) = self.selected_process() {
            let (identity, name, command) = (
                proc.identity(),
                proc.name.to_string(),
                proc.command.to_string(),
            );

            // Skip confirmation dialog if disabled in settings
            if !self.config.confirm_kill {
                if !self.tagged_pids.is_empty() {
                    self.kill_tagged();
                } else {
                    self.kill_process_by(identity, &name);
                }
            } else {
                self.dialog = DialogState::Kill {
                    identity,
                    name,
                    command,
                };
            }
        }
    }

    /// Enter priority mode and capture the target process
    /// Open the priority dialog for the selected process. `step` pre-selects
    /// a class relative to the process's current one — F7/`]` aim one class
    /// higher (+1), F8/`[` one lower (-1), matching htop's Nice -/Nice + keys
    /// — so pressing Enter applies that step (issue #26).
    pub fn enter_priority_mode(&mut self, step: i32) {
        if self.readonly_blocked("change process priority") {
            return;
        }
        if let Some(proc) = self.selected_process() {
            let current = WindowsPriorityClass::from_base_priority(proc.priority).index() as i32;
            let max = WindowsPriorityClass::all().len() as i32 - 1;
            let class_index = (current + step).clamp(0, max) as usize;
            self.dialog = DialogState::Priority {
                class_index,
                identity: proc.identity(),
                name: proc.name.to_string(),
            };
        }
    }

    /// Enter process info mode and capture the target process
    pub fn enter_process_info_mode(&mut self) {
        if let Some(proc) = self.selected_process() {
            let mut proc_copy = proc.clone();
            let identity = proc.identity();
            let (io_read, io_write) = crate::system::get_process_io_counters(identity);
            proc_copy.io_read_bytes = io_read;
            proc_copy.io_write_bytes = io_write;
            if proc_copy.exe_path.is_empty() {
                let exe_path = crate::system::get_process_exe_path(identity);
                if !exe_path.is_empty() {
                    // Share one allocation between exe_path and command (refcounted).
                    let shared: std::sync::Arc<str> = std::sync::Arc::from(exe_path);
                    proc_copy.exe_path = shared.clone();
                    proc_copy.command = shared;
                }
            }
            self.dialog = DialogState::ProcessInfo {
                target: Box::new(proc_copy),
                scroll: 0,
            };
        }
    }

    /// Refresh I/O counters for process info dialog (called during tick when dialog is open)
    pub fn refresh_process_info_io(&mut self) {
        if let DialogState::ProcessInfo { ref mut target, .. } = self.dialog {
            let (io_read, io_write) = crate::system::get_process_io_counters(target.identity());
            target.io_read_bytes = io_read;
            target.io_write_bytes = io_write;
        }
    }

    /// Refresh the Process Info dialog's stats from the latest snapshot so
    /// the dialog tracks the live process instead of freezing at open time.
    /// The dialog stays pinned to the process captured at open: stats are
    /// only copied when the fresh entry is the same process (creation-time
    /// match, falling back to name when unavailable), so PID reuse can't
    /// swap in a stranger's numbers. If the process exits, the last known
    /// stats stay on screen.
    pub fn refresh_process_info_stats(&mut self) {
        let DialogState::ProcessInfo { ref mut target, .. } = self.dialog else {
            return;
        };
        let identity = target.identity();
        let Some(fresh) = self.processes.iter().find(|p| p.identity() == identity) else {
            return;
        };

        let mut updated = fresh.clone();
        // The dialog owns the cumulative I/O counters: refresh_process_info_io
        // re-reads them every tick, even between snapshots and while paused.
        updated.io_read_bytes = target.io_read_bytes;
        updated.io_write_bytes = target.io_write_bytes;
        // Keep the exe-path enrichment done at open time when the snapshot
        // lacks it (the two share one allocation, see enter_process_info_mode).
        if updated.exe_path.is_empty() && !target.exe_path.is_empty() {
            updated.exe_path = target.exe_path.clone();
            updated.command = target.command.clone();
        }
        **target = updated;
    }

    /// Refresh system data (synchronous, used for initial refresh fallback)
    pub fn refresh_system(&mut self) {
        // Use native Windows APIs for all system metrics
        self.system_metrics.refresh();
        // Update processes in-place to avoid re-allocating strings
        self.system_metrics
            .update_processes_native(&mut self.processes);
        crate::system::hydrate_processes_from_cache(&mut self.processes);
        // This UI-thread refresh only collected raw data. Any required bulk
        // metadata pass remains delegated to the collector.
        self.canonical_enrichment = Default::default();
        self.update_displayed_processes();
        self.refresh_process_info_stats();

        // Update history for graph mode
        self.update_meter_history();
    }

    /// Apply a snapshot from the background data collector
    pub fn apply_snapshot(&mut self, snapshot: crate::data::SystemSnapshot) {
        self.system_metrics = snapshot.metrics;
        self.processes = snapshot.processes;
        self.canonical_enrichment = snapshot.enrichment;
        self.update_displayed_processes();
        self.refresh_process_info_stats();
        if self.pending_user_select && self.canonical_enrichment.user {
            self.open_user_select_dialog();
        } else if self.canonical_enrichment.user
            && matches!(self.dialog, DialogState::UserSelect { .. })
        {
            self.refresh_user_select_dialog();
        }
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
        self.mem_history
            .push_back(self.system_metrics.memory.used_percent);

        // Add current swap usage to history
        if self.swap_history.len() >= MAX_HISTORY {
            self.swap_history.pop_front();
        }
        self.swap_history
            .push_back(self.system_metrics.memory.swap_percent);

        // Add current GPU usage to history (only meaningful on GPU machines)
        if self.gpu_history.len() >= MAX_HISTORY {
            self.gpu_history.pop_front();
        }
        self.gpu_history.push_back(
            self.system_metrics
                .gpu
                .as_ref()
                .map_or(0.0, |g| g.utilization),
        );

        // Add current NPU usage to history (only meaningful on NPU machines)
        if self.npu_history.len() >= MAX_HISTORY {
            self.npu_history.pop_front();
        }
        self.npu_history.push_back(
            self.system_metrics
                .npu
                .as_ref()
                .map_or(0.0, |n| n.utilization),
        );
    }

    /// Keep the per-process GPU/NPU collection gates in sync with what's
    /// displayed. Collection costs a handle open plus a few syscalls per
    /// process per tick, so each class only runs while one of its columns
    /// is visible or sorted.
    fn refresh_adapter_collection_flags(&self) {
        let gpu_wanted = matches!(self.sort_column, SortColumn::Gpu | SortColumn::GpuMem)
            || self
                .cached_visible_columns
                .iter()
                .any(|c| matches!(c, SortColumn::Gpu | SortColumn::GpuMem));
        crate::system::set_gpu_process_stats_enabled(gpu_wanted);

        let npu_wanted = matches!(self.sort_column, SortColumn::Npu | SortColumn::NpuMem)
            || self
                .cached_visible_columns
                .iter()
                .any(|c| matches!(c, SortColumn::Npu | SortColumn::NpuMem));
        crate::system::set_npu_process_stats_enabled(npu_wanted);

        // Keep the pinned-GPU selection in sync with config (Setup can change it).
        crate::system::set_gpu_selection(self.config.gpu_meter_adapter.clone());
    }

    /// Canonical metadata the collector must populate for the active
    /// filter/sort/dialog. The default returns no requirements, keeping normal
    /// steady-state collection free of all-process Windows calls.
    pub fn canonical_enrichment_requirements(
        &self,
    ) -> crate::system::ProcessEnrichmentRequirements {
        let has_filter = !self.filter_string_lower.is_empty();
        crate::system::ProcessEnrichmentRequirements {
            user: has_filter
                || self.user_filter.is_some()
                || !self.config.show_kernel_threads
                || !self.config.show_user_threads
                || matches!(self.sort_column, SortColumn::User)
                || self.pending_user_select
                || matches!(self.dialog, DialogState::UserSelect { .. }),
            elevation: matches!(self.sort_column, SortColumn::Elevated),
            arch: matches!(self.sort_column, SortColumn::Arch),
            efficiency: matches!(self.sort_column, SortColumn::Efficiency),
            exe_path: has_filter
                || (self.config.show_program_path
                    && matches!(self.sort_column, SortColumn::Command)),
        }
    }

    /// Update displayed processes based on filter and sort
    pub fn update_displayed_processes(&mut self) {
        self.refresh_adapter_collection_flags();

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

        let requirements = self.canonical_enrichment_requirements();
        if !self.canonical_enrichment.contains(requirements) {
            // Preserve the last authoritative view until the collector returns
            // this exact process set with its required metadata populated.
            return;
        }

        let live_identities: HashSet<ProcessIdentity> =
            self.processes.iter().map(ProcessInfo::identity).collect();
        self.tagged_pids
            .retain(|identity| live_identities.contains(identity));
        if self
            .follow_pid
            .is_some_and(|identity| !live_identities.contains(&identity))
        {
            self.follow_pid = None;
        }

        // Prune stale collapsed PIDs only when the set has grown disproportionate to
        // the live process count. Otherwise `collapsed_pids` grows unbounded over long
        // uptime (each collapse_all adds every PID; dead/reused PIDs are never removed).
        if self.collapsed_pids.len() > process_count * 2 {
            self.collapsed_pids
                .retain(|identity| live_identities.contains(identity));
        }

        let mut processes: Vec<ProcessInfo> = Vec::with_capacity(process_count);
        processes.extend(
            self.processes
                .iter()
                .filter(|p| {
                    // Kernel/System threads filter
                    // On Windows, "kernel threads" are SYSTEM user processes
                    let is_kernel = &*p.user_lower == "system"
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
                        && !pids.contains(&p.pid)
                    {
                        return false;
                    }
                    // User filter
                    if let Some(ref user) = self.user_filter
                        && &*p.user != user.as_str()
                    {
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
                .cloned(),
        );

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

        // Clamp selection and scroll immediately after replacing the list, before
        // enrichment uses scroll_offset to choose the visible slice.
        if self.selected_index >= self.displayed_processes.len() {
            self.selected_index = self.displayed_processes.len().saturating_sub(1);
        }
        if self.displayed_processes.is_empty() {
            self.scroll_offset = 0;
        } else {
            let max_scroll = self
                .displayed_processes
                .len()
                .saturating_sub(self.visible_height.max(1));
            self.scroll_offset = self.scroll_offset.min(max_scroll);
            self.ensure_visible();
        }

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
        if let Some(follow_identity) = self.follow_pid
            && let Some(idx) = self
                .displayed_processes
                .iter()
                .position(|p| p.identity() == follow_identity)
        {
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
                    processes.sort_unstable_by(|a, b| {
                        a.cpu_percent
                            .partial_cmp(&b.cpu_percent)
                            .unwrap_or(Ordering::Equal)
                    });
                } else {
                    processes.sort_unstable_by(|a, b| {
                        b.cpu_percent
                            .partial_cmp(&a.cpu_percent)
                            .unwrap_or(Ordering::Equal)
                    });
                }
            }
            SortColumn::Mem => {
                if ascending {
                    processes.sort_unstable_by(|a, b| {
                        a.mem_percent
                            .partial_cmp(&b.mem_percent)
                            .unwrap_or(Ordering::Equal)
                    });
                } else {
                    processes.sort_unstable_by(|a, b| {
                        b.mem_percent
                            .partial_cmp(&a.mem_percent)
                            .unwrap_or(Ordering::Equal)
                    });
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
                        SortColumn::IoRate => (a.io_read_rate + a.io_write_rate)
                            .cmp(&(b.io_read_rate + b.io_write_rate)),
                        SortColumn::IoReadRate => a.io_read_rate.cmp(&b.io_read_rate),
                        SortColumn::IoWriteRate => a.io_write_rate.cmp(&b.io_write_rate),
                        SortColumn::IoRead => a.io_read_bytes.cmp(&b.io_read_bytes),
                        SortColumn::IoWrite => a.io_write_bytes.cmp(&b.io_write_bytes),
                        SortColumn::Gpu => a
                            .gpu_percent
                            .partial_cmp(&b.gpu_percent)
                            .unwrap_or(Ordering::Equal),
                        SortColumn::GpuMem => a.gpu_memory.cmp(&b.gpu_memory),
                        SortColumn::Npu => a
                            .npu_percent
                            .partial_cmp(&b.npu_percent)
                            .unwrap_or(Ordering::Equal),
                        SortColumn::NpuMem => a.npu_memory.cmp(&b.npu_memory),
                        // Already handled above
                        SortColumn::Cpu
                        | SortColumn::Mem
                        | SortColumn::Pid
                        | SortColumn::Res
                        | SortColumn::Time => Ordering::Equal,
                    };
                    if ascending { ord } else { ord.reverse() }
                };
                processes.sort_unstable_by(cmp_fn);
            }
        }
    }

    fn build_tree(&self, processes: Vec<ProcessInfo>) -> Vec<ProcessInfo> {
        use std::collections::HashMap;

        #[derive(Debug)]
        struct PendingNode {
            pid: u32,
            depth: usize,
            is_last: bool,
            parent_prefix: String,
        }

        let process_count = processes.len();
        let all_pids: HashSet<u32> = processes.iter().map(|process| process.pid).collect();
        let order: Vec<u32> = processes.iter().map(|process| process.pid).collect();
        let roots: Vec<u32> = processes
            .iter()
            .filter(|process| {
                process.parent_pid == 0
                    || process.parent_pid == process.pid
                    || !all_pids.contains(&process.parent_pid)
            })
            .map(|process| process.pid)
            .collect();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::with_capacity(process_count / 4);
        let mut nodes: HashMap<u32, ProcessInfo> = HashMap::with_capacity(process_count);
        for process in processes {
            if process.parent_pid != 0
                && process.parent_pid != process.pid
                && all_pids.contains(&process.parent_pid)
            {
                children
                    .entry(process.parent_pid)
                    .or_default()
                    .push(process.pid);
            }
            nodes.insert(process.pid, process);
        }

        let mut result = Vec::with_capacity(process_count);
        let mut visited = HashSet::with_capacity(process_count);
        let mut suppressed = HashSet::new();

        // Traverse rooted components first, then cycle-only components in the
        // original sort order. Iteration avoids both the old depth cutoff and
        // call-stack overflow while the visited set emits every node once.
        let mut component_roots = roots;
        component_roots.extend(order.iter().copied());
        for root_pid in component_roots {
            if visited.contains(&root_pid) || suppressed.contains(&root_pid) {
                continue;
            }
            let mut stack = vec![PendingNode {
                pid: root_pid,
                depth: 0,
                is_last: true,
                parent_prefix: String::new(),
            }];
            while let Some(pending) = stack.pop() {
                if !visited.insert(pending.pid) || suppressed.contains(&pending.pid) {
                    continue;
                }
                let Some(mut process) = nodes.remove(&pending.pid) else {
                    continue;
                };
                let identity = process.identity();
                let child_pids = children.get(&pending.pid).cloned().unwrap_or_default();
                let is_collapsed = self.collapsed_pids.contains(&identity);

                process.tree_depth = pending.depth;
                process.has_children = !child_pids.is_empty();
                process.is_collapsed = is_collapsed;
                process.tree_prefix = if pending.depth == 0 {
                    String::new()
                } else {
                    let branch = if pending.is_last {
                        "└─ "
                    } else {
                        "├─ "
                    };
                    let mut prefix = pending.parent_prefix.clone();
                    prefix.push_str(branch);
                    prefix
                };
                result.push(process);

                if is_collapsed {
                    // Descendants of a collapsed node are intentionally hidden,
                    // not orphans. Mark the entire branch so fallback traversal
                    // cannot append it at the top level.
                    let mut descendants = child_pids;
                    while let Some(descendant) = descendants.pop() {
                        if visited.contains(&descendant) || !suppressed.insert(descendant) {
                            continue;
                        }
                        if let Some(grandchildren) = children.get(&descendant) {
                            descendants.extend(grandchildren.iter().copied());
                        }
                    }
                    continue;
                }

                let mut child_parent_prefix = if pending.depth == 0 {
                    String::new()
                } else {
                    pending.parent_prefix
                };
                if pending.depth > 0 {
                    child_parent_prefix.push_str(if pending.is_last { "   " } else { "│  " });
                }
                let child_count = child_pids.len();
                for (index, child_pid) in child_pids.into_iter().enumerate().rev() {
                    stack.push(PendingNode {
                        pid: child_pid,
                        depth: pending.depth + 1,
                        is_last: index + 1 == child_count,
                        parent_prefix: child_parent_prefix.clone(),
                    });
                }
            }
        }

        result
    }

    /// Collapse tree branch at selected process
    pub fn collapse_tree(&mut self) {
        let identity = self.selected_process().map(ProcessInfo::identity);
        if let Some(identity) = identity {
            self.collapsed_pids.insert(identity);
            self.needs_process_update = true;
        }
    }

    /// Expand tree branch at selected process
    pub fn expand_tree(&mut self) {
        let identity = self.selected_process().map(ProcessInfo::identity);
        if let Some(identity) = identity {
            self.collapsed_pids.remove(&identity);
            self.needs_process_update = true;
        }
    }

    /// Collapse all tree branches
    pub fn collapse_all(&mut self) {
        // Collapse all processes that have children
        for proc in &self.processes {
            self.collapsed_pids.insert(proc.identity());
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
        self.selected_index =
            (self.selected_index + page_size).min(self.displayed_processes.len().saturating_sub(1));
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
            let identity = proc.identity();
            if self.tagged_pids.contains(&identity) {
                self.tagged_pids.remove(&identity);
            } else {
                self.tagged_pids.insert(identity);
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
            let identities_to_tag: Vec<ProcessIdentity> = self
                .displayed_processes
                .iter()
                .filter(|p| p.name == name)
                .map(ProcessInfo::identity)
                .collect();
            for identity in identities_to_tag {
                self.tagged_pids.insert(identity);
            }
        }
    }

    /// Toggle tag on all visible/filtered processes
    pub fn tag_all_visible(&mut self) {
        // If all visible are already tagged, untag them
        let all_tagged = self
            .displayed_processes
            .iter()
            .all(|p| self.tagged_pids.contains(&p.identity()));

        if all_tagged {
            // Untag all visible
            for proc in &self.displayed_processes {
                self.tagged_pids.remove(&proc.identity());
            }
        } else {
            // Tag all visible
            for proc in &self.displayed_processes {
                self.tagged_pids.insert(proc.identity());
            }
        }
    }

    /// Get selected process
    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        self.displayed_processes.get(self.selected_index)
    }

    pub fn process_by_identity(&self, identity: ProcessIdentity) -> Option<&ProcessInfo> {
        self.displayed_processes
            .iter()
            .find(|process| process.identity() == identity)
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
        self.update_search_from_dialog(true);
    }

    /// Synchronize the live query with the Search dialog. Editing establishes
    /// a deterministic first match; F3/Enter can retain the current match.
    pub fn update_search_from_dialog(&mut self, select_first: bool) {
        if let DialogState::Search { ref buffer, .. } = self.dialog {
            self.search_string = buffer.clone();
        } else {
            return;
        }
        self.search_string_lower = self.search_string.to_lowercase();
        // Find first matching process using pre-computed lowercase strings
        if select_first
            && !self.search_string_lower.is_empty()
            && let Some(idx) = self.displayed_processes.iter().position(|p| {
                p.name_lower.contains(&self.search_string_lower)
                    || p.command_lower.contains(&self.search_string_lower)
            })
        {
            self.selected_index = idx;
            self.ensure_visible();
        }
        // Update matches_search flags for highlighting
        self.needs_process_update = true;
    }

    /// Find next search match
    pub fn find_next(&mut self) {
        if self.search_string_lower.is_empty() || self.displayed_processes.is_empty() {
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
    pub fn kill_target_process(&mut self) {
        let (identity, name) = match &self.dialog {
            DialogState::Kill { identity, name, .. } => (*identity, name.clone()),
            _ => return,
        };
        self.kill_process_by(identity, &name);
    }

    /// Force-terminate a process after validating the captured identity.
    fn kill_process_by(&mut self, identity: ProcessIdentity, name: &str) {
        if self.readonly_blocked("kill processes") {
            return;
        }
        match crate::system::kill_process(identity) {
            Ok(_) => {
                self.status_message = Some((
                    format!("Force terminated {} (PID {})", name, identity.pid),
                    Instant::now(),
                ));
            }
            Err(e) => {
                self.last_error = Some((
                    format!("Failed to terminate {} ({}): {}", name, identity.pid, e),
                    Instant::now(),
                ));
            }
        }
    }

    /// Kill all tagged processes
    pub fn kill_tagged(&mut self) {
        if self.readonly_blocked("kill processes") {
            return;
        }
        let identities: Vec<ProcessIdentity> = self.tagged_pids.iter().copied().collect();
        let total = identities.len();
        let mut killed = 0;
        let mut failed = 0;

        for identity in identities {
            match crate::system::kill_process(identity) {
                Ok(_) => killed += 1,
                Err(e) => {
                    failed += 1;
                    self.last_error = Some((
                        format!("Failed to terminate process {}: {}", identity.pid, e),
                        Instant::now(),
                    ));
                }
            }
        }

        if failed == 0 {
            self.status_message = Some((
                format!(
                    "Force terminated {} process{}",
                    killed,
                    if killed == 1 { "" } else { "es" }
                ),
                Instant::now(),
            ));
        } else {
            self.status_message = Some((
                format!(
                    "Force terminated {}/{} processes ({} failed)",
                    killed, total, failed
                ),
                Instant::now(),
            ));
        }

        self.tagged_pids.clear();
    }

    /// Set priority class for selected process
    pub fn set_priority_selected(&mut self, priority_class: WindowsPriorityClass) {
        if self.readonly_blocked("change process priority") {
            return;
        }
        let identity = match &self.dialog {
            DialogState::Priority { identity, .. } => *identity,
            _ => return,
        };
        if let Err(e) = crate::system::set_priority_class(identity, priority_class) {
            self.last_error = Some((
                format!("Failed to set priority for {}: {}", identity.pid, e),
                Instant::now(),
            ));
        }
    }

    /// Toggle efficiency mode for selected process
    pub fn toggle_efficiency_mode(&mut self) {
        if self.readonly_blocked("change process efficiency mode") {
            return;
        }
        let (identity, name) = match &self.dialog {
            DialogState::Priority { identity, name, .. } => (*identity, name.clone()),
            _ => return,
        };
        // Read the current state from the captured pid, not selected_process():
        // a background re-sort can move a different process under selected_index
        // while the Priority dialog is open, which would otherwise flip the wrong
        // direction or silently no-op.
        let current = self
            .process_by_identity(identity)
            .map(|p| p.efficiency_mode)
            .unwrap_or(false);
        let new_state = !current;
        match crate::system::set_efficiency_mode(identity, new_state) {
            Ok(_) => {
                let state_str = if new_state { "enabled" } else { "disabled" };
                self.status_message = Some((
                    format!("Efficiency mode {} for {}", state_str, name),
                    Instant::now(),
                ));
                for proc in &mut self.displayed_processes {
                    if proc.identity() == identity {
                        proc.efficiency_mode = new_state;
                        break;
                    }
                }
                for proc in &mut self.processes {
                    if proc.identity() == identity {
                        proc.efficiency_mode = new_state;
                        break;
                    }
                }
            }
            Err(e) => {
                self.last_error = Some((
                    format!("Failed to set efficiency mode: {}", e),
                    Instant::now(),
                ));
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
            *cursor = clamp_char_boundary(buffer, *cursor);
            buffer.insert(*cursor, c);
            *cursor += c.len_utf8();
        }
    }

    /// Delete character before cursor
    pub fn input_backspace(&mut self) {
        if let Some((buffer, cursor)) = self.dialog.input_buffer_mut()
            && *cursor > 0
        {
            *cursor = clamp_char_boundary(buffer, *cursor);
            let prev = prev_char_boundary(buffer, *cursor);
            buffer.drain(prev..*cursor);
            *cursor = prev;
        }
    }

    /// Delete character at cursor
    pub fn input_delete(&mut self) {
        if let Some((buffer, cursor)) = self.dialog.input_buffer_mut()
            && *cursor < buffer.len()
        {
            *cursor = clamp_char_boundary(buffer, *cursor);
            let next = next_char_boundary(buffer, *cursor);
            buffer.drain(*cursor..next);
        }
    }

    /// Move cursor left
    pub fn input_left(&mut self) {
        if let Some((buffer, cursor)) = self.dialog.input_buffer_mut()
            && *cursor > 0
        {
            *cursor = prev_char_boundary(buffer, *cursor);
        }
    }

    /// Move cursor right
    pub fn input_right(&mut self) {
        if let Some((buffer, cursor)) = self.dialog.input_buffer_mut()
            && *cursor < buffer.len()
        {
            *cursor = next_char_boundary(buffer, *cursor);
        }
    }

    /// Start search mode
    pub fn start_search(&mut self) {
        let buffer = self.search_string.clone();
        let cursor = buffer.len();
        self.dialog = DialogState::Search {
            original: buffer.clone(),
            original_selection: self.selected_process().map(ProcessInfo::identity),
            buffer,
            cursor,
        };
    }

    /// Start filter mode
    pub fn start_filter(&mut self) {
        let buffer = self.filter_string.clone();
        let cursor = buffer.len();
        self.dialog = DialogState::Filter {
            original: buffer.clone(),
            original_selection: self.selected_process().map(ProcessInfo::identity),
            buffer,
            cursor,
        };
    }

    /// Cancel/close a dialog using the active dialog's semantics. Search and
    /// Filter roll back live edits; Setup commits its settings.
    pub fn cancel_dialog(&mut self) {
        let dialog = std::mem::take(&mut self.dialog);
        let mut restore_selection = None;
        match dialog {
            DialogState::Search {
                original,
                original_selection,
                ..
            } => {
                self.search_string = original;
                self.search_string_lower = self.search_string.to_lowercase();
                restore_selection = original_selection;
                self.update_displayed_processes();
                self.needs_process_update = false;
            }
            DialogState::Filter {
                original,
                original_selection,
                ..
            } => {
                self.filter_string = original;
                self.filter_string_lower = self.filter_string.to_lowercase();
                restore_selection = original_selection;
                self.update_displayed_processes();
                self.needs_process_update = false;
            }
            DialogState::Setup { .. } => {
                self.save_config();
            }
            _ => {}
        }
        if let Some(identity) = restore_selection
            && let Some(index) = self
                .displayed_processes
                .iter()
                .position(|process| process.identity() == identity)
        {
            self.selected_index = index;
            self.ensure_visible();
        }
    }

    /// Close Setup and persist any changes made through keyboard or mouse.
    pub fn close_setup(&mut self) {
        self.dialog = DialogState::None;
        self.save_config();
    }

    /// Tag selected process and all its children
    pub fn tag_with_children(&mut self) {
        let selected = self.selected_process().map(|process| process.identity());
        if let Some(identity) = selected {
            for descendant in self.branch_identities(identity) {
                self.tagged_pids.insert(descendant);
            }
        }
    }

    fn branch_identities(&self, root: ProcessIdentity) -> Vec<ProcessIdentity> {
        if !self
            .processes
            .iter()
            .any(|process| process.identity() == root)
        {
            return Vec::new();
        }

        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut pending = vec![root.pid];
        while let Some(parent_pid) = pending.pop() {
            if !visited.insert(parent_pid) {
                continue;
            }
            if let Some(process) = self
                .processes
                .iter()
                .find(|process| process.pid == parent_pid)
            {
                result.push(process.identity());
            }
            pending.extend(
                self.processes
                    .iter()
                    .filter(|process| process.parent_pid == parent_pid && process.pid != parent_pid)
                    .map(|process| process.pid),
            );
        }
        result
    }

    /// Toggle tag for a process and all its descendants (for tree mode double-click)
    pub fn toggle_tag_branch(&mut self, identity: ProcessIdentity) {
        // Collect the process and all its descendants
        let branch_identities = self.branch_identities(identity);

        // Check if all are already tagged
        let all_tagged = branch_identities
            .iter()
            .all(|identity| self.tagged_pids.contains(identity));

        if all_tagged {
            // Untag all
            for identity in branch_identities {
                self.tagged_pids.remove(&identity);
            }
        } else {
            // Tag all
            for identity in branch_identities {
                self.tagged_pids.insert(identity);
            }
        }
    }

    /// Enter user select mode
    pub fn enter_user_select_mode(&mut self) {
        if self.pending_user_select {
            self.cancel_pending_user_select();
            return;
        }
        if !self.canonical_enrichment.user {
            self.pending_user_select = true;
            self.status_message = Some(("Loading process owners...".to_string(), Instant::now()));
            return;
        }
        self.open_user_select_dialog();
    }

    fn open_user_select_dialog(&mut self) {
        self.pending_user_select = false;
        if self
            .status_message
            .as_ref()
            .is_some_and(|(message, _)| message == "Loading process owners...")
        {
            self.status_message = None;
        }
        let selected_user = self.user_filter.clone();
        self.rebuild_user_select_dialog(selected_user.as_deref());
    }

    fn refresh_user_select_dialog(&mut self) {
        let selected_user = match &self.dialog {
            DialogState::UserSelect { index, users } if *index > 0 => {
                users.get(*index - 1).cloned()
            }
            DialogState::UserSelect { .. } => None,
            _ => return,
        };
        self.rebuild_user_select_dialog(selected_user.as_deref());
    }

    fn rebuild_user_select_dialog(&mut self, selected_user: Option<&str>) {
        let mut users: Vec<String> = self
            .processes
            .iter()
            .map(|p| p.user.to_string())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        users.sort();

        let index = if let Some(filter) = selected_user {
            users
                .iter()
                .position(|u| u == filter)
                .map(|i| i + 1)
                .unwrap_or(0)
        } else {
            0
        };

        self.dialog = DialogState::UserSelect { index, users };
    }

    pub fn cancel_pending_user_select(&mut self) {
        self.pending_user_select = false;
        if self
            .status_message
            .as_ref()
            .is_some_and(|(message, _)| message == "Loading process owners...")
        {
            self.status_message = None;
        }
    }

    /// Toggle follow mode
    pub fn toggle_follow_mode(&mut self) {
        if self.follow_pid.is_some() {
            self.follow_pid = None;
        } else if let Some(proc) = self.selected_process() {
            self.follow_pid = Some(proc.identity());
        }
    }

    /// Enter environment view mode
    pub fn enter_environment_mode(&mut self) {
        if let Some(proc) = self.selected_process() {
            self.dialog = DialogState::Environment {
                scroll: 0,
                identity: proc.identity(),
            };
        }
    }

    /// Enter command wrap view mode
    pub fn enter_command_wrap_mode(&mut self) {
        if let Some(proc) = self.selected_process() {
            self.dialog = DialogState::CommandWrap {
                scroll: 0,
                identity: proc.identity(),
            };
        }
    }

    /// Enter CPU affinity mode
    pub fn enter_affinity_mode(&mut self) {
        if self.readonly_blocked("change CPU affinity") {
            return;
        }
        if let Some(proc) = self.selected_process() {
            let cpu_count = self.system_metrics.cpu.core_usage.len();
            if cpu_count > 64 {
                self.last_error = Some((
                    "CPU affinity editing is not supported on systems with more than 64 logical CPUs".to_string(),
                    Instant::now(),
                ));
                return;
            }
            let all_cpus = if cpu_count >= 64 {
                u64::MAX
            } else {
                (1u64 << cpu_count) - 1
            };
            let identity = proc.identity();
            let mask = crate::system::get_process_affinity(identity).unwrap_or(all_cpus);
            self.dialog = DialogState::Affinity {
                mask,
                selected: 0,
                identity,
            };
        }
    }

    /// Apply CPU affinity to the process captured when the dialog was opened.
    pub fn apply_affinity(&mut self) {
        if self.readonly_blocked("change CPU affinity") {
            return;
        }
        let (mask, identity) = match &self.dialog {
            DialogState::Affinity { mask, identity, .. } => (*mask, *identity),
            _ => return,
        };
        if mask == 0 {
            self.last_error = Some(("Cannot set empty affinity mask".to_string(), Instant::now()));
            return;
        }
        // Use the captured pid, not selected_process(): a background refresh can
        // re-sort the list and shift selected_index while the dialog is open.
        if let Err(e) = crate::system::set_process_affinity(identity, mask) {
            self.last_error = Some((format!("Failed to set affinity: {}", e), Instant::now()));
        }
    }

    /// Handle digit key for PID search
    pub fn handle_pid_digit(&mut self, digit: char) {
        use std::time::Duration;

        let now = Instant::now();

        // Clear buffer if too much time has passed (1 second timeout)
        // or if buffer exceeds max PID length (u32 max is 10 digits)
        if let Some(last_time) = self.pid_search_time
            && now.duration_since(last_time) > Duration::from_secs(1)
        {
            self.pid_search_buffer.clear();
        }
        if self.pid_search_buffer.len() >= 10 {
            self.pid_search_buffer.clear();
        }

        // Add digit to buffer
        self.pid_search_buffer.push(digit);
        self.pid_search_time = Some(now);

        // Prefer an exact PID; otherwise choose the numerically smallest PID
        // whose decimal representation has the typed prefix. Display sort
        // order never influences which process wins.
        if let Ok(search_pid) = self.pid_search_buffer.parse::<u32>() {
            let match_identity = self
                .displayed_processes
                .iter()
                .find(|process| process.pid == search_pid)
                .or_else(|| {
                    self.displayed_processes
                        .iter()
                        .filter(|process| {
                            process.pid.to_string().starts_with(&self.pid_search_buffer)
                        })
                        .min_by_key(|process| process.pid)
                })
                .map(ProcessInfo::identity);
            if let Some(identity) = match_identity
                && let Some(index) = self
                    .displayed_processes
                    .iter()
                    .position(|process| process.identity() == identity)
            {
                self.selected_index = index;
                self.ensure_visible();
            }
        }
    }

    /// Collapse to parent in tree view
    pub fn collapse_to_parent(&mut self) {
        if let Some(proc) = self.selected_process() {
            let parent_pid = proc.parent_pid;
            // Find parent in displayed processes and select it
            if let Some((index, identity)) = self
                .displayed_processes
                .iter()
                .enumerate()
                .find(|(_, process)| process.pid == parent_pid)
                .map(|(index, process)| (index, process.identity()))
            {
                self.selected_index = index;
                self.ensure_visible();
                self.collapsed_pids.insert(identity);
                self.needs_process_update = true;
            }
        }
    }

    /// Enter column configuration mode
    pub fn enter_column_config_mode(&mut self) {
        self.dialog = DialogState::ColumnConfig { index: 0 };
    }

    /// Open the GPU-adapter selector. Entry 0 is "Auto"; the rest are the
    /// detected GPU names. Pre-selects the currently pinned adapter, if any.
    pub fn enter_gpu_select_mode(&mut self) {
        let names = crate::system::gpu_names();
        let index = self
            .config
            .gpu_meter_adapter
            .as_ref()
            .and_then(|sel| names.iter().position(|n| n == sel))
            .map(|i| i + 1)
            .unwrap_or(0);
        self.dialog = DialogState::GpuSelect { index, names };
    }
}

/// Index at which `column` should be inserted into `columns` to keep the
/// canonical display order (`SortColumn::all()`); appends when every visible
/// column canonically precedes it.
fn canonical_insert_index(columns: &[String], column: &str) -> usize {
    let rank = SortColumn::display_rank(column);
    columns
        .iter()
        .position(|c| SortColumn::display_rank(c) > rank)
        .unwrap_or(columns.len())
}

/// Default Main-tab columns for the detected hardware: the static defaults
/// plus GPU%/GPU-MEM and NPU%/NPU-MEM when the corresponding adapter exists,
/// inserted at their canonical positions (right after MEM%).
fn hardware_default_columns(has_gpu: bool, has_npu: bool) -> Vec<String> {
    let mut columns = Config::default().visible_columns;
    let mut add = |name: &str| {
        let at = canonical_insert_index(&columns, name);
        columns.insert(at, name.to_string());
    };
    if has_gpu {
        add("GPU%");
        add("GPU-MEM");
    }
    if has_npu {
        add("NPU%");
        add("NPU-MEM");
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::ProcessArch;
    use std::sync::Arc;
    use std::time::Duration;

    fn process(pid: u32, parent_pid: u32) -> ProcessInfo {
        let name = format!("p{pid}");
        ProcessInfo {
            pid,
            parent_pid,
            name: Arc::from(name.clone()),
            exe_path: Arc::from(""),
            command: Arc::from(name.clone()),
            user: Arc::from("user"),
            status: 'S',
            cpu_percent: 0.0,
            mem_percent: 0.0,
            virtual_mem: 0,
            resident_mem: 0,
            shared_mem: 0,
            priority: 8,
            cpu_time: Duration::ZERO,
            tree_depth: 0,
            tree_prefix: String::new(),
            has_children: false,
            is_collapsed: false,
            thread_count: 1,
            start_time: 0,
            create_time_100ns: 10_000 + pid as u64,
            handle_count: 0,
            io_read_bytes: 0,
            io_write_bytes: 0,
            io_read_rate: 0,
            io_write_rate: 0,
            gpu_percent: 0.0,
            gpu_memory: 0,
            npu_percent: 0.0,
            npu_memory: 0,
            name_lower: Arc::from(name),
            command_lower: Arc::from(""),
            user_lower: Arc::from("user"),
            matches_search: false,
            efficiency_mode: false,
            is_elevated: false,
            arch: ProcessArch::Native,
            exe_updated: false,
            exe_deleted: false,
        }
    }

    fn process_as_user(pid: u32, user: &str) -> ProcessInfo {
        let mut process = process(pid, 0);
        process.user = Arc::from(user);
        process.user_lower = Arc::from(user.to_lowercase());
        process
    }

    fn assert_canonical_order(columns: &[String]) {
        let ranks: Vec<usize> = columns
            .iter()
            .map(|c| SortColumn::display_rank(c))
            .collect();
        let mut sorted = ranks.clone();
        sorted.sort_unstable();
        assert_eq!(
            ranks, sorted,
            "columns not in canonical display order: {:?}",
            columns
        );
    }

    #[test]
    fn destructive_reset_is_last_setup_item() {
        // Keep the destructive action at the bottom of the Setup list
        // (issue #27) — draw and input both derive order from this table.
        assert_eq!(SetupItem::ALL.last(), Some(&SetupItem::ResetAllSettings));
        assert_eq!(SetupItem::GpuMeterAdapter.index(), SetupItem::ALL.len() - 2);
    }

    #[test]
    fn default_layouts_follow_display_order() {
        assert_canonical_order(&crate::config::Config::default().visible_columns);
        assert_canonical_order(&ScreenTab::default_io().columns);
    }

    #[test]
    fn display_order_groups_usage_and_keeps_command_last() {
        assert_eq!(
            SortColumn::display_rank("Command"),
            SortColumn::all().len() - 1
        );
        // Usage block: CPU% MEM% GPU% GPU-MEM NPU% NPU-MEM, in that order
        let usage = ["CPU%", "MEM%", "GPU%", "GPU-MEM", "NPU%", "NPU-MEM"];
        let base = SortColumn::display_rank("CPU%");
        for (i, name) in usage.iter().enumerate() {
            assert_eq!(SortColumn::display_rank(name), base + i);
        }
    }

    #[test]
    fn hardware_defaults_add_adapter_columns_after_mem() {
        // No adapters: identical to the static defaults
        assert_eq!(
            hardware_default_columns(false, false),
            crate::config::Config::default().visible_columns
        );

        // Both adapters: usage block sits between MEM% and TIME+, Command last
        let full = hardware_default_columns(true, true);
        let mem = full.iter().position(|c| c == "MEM%").unwrap();
        let window: Vec<&str> = full[mem..mem + 5].iter().map(String::as_str).collect();
        assert_eq!(window, ["MEM%", "GPU%", "GPU-MEM", "NPU%", "NPU-MEM"]);
        assert_eq!(full.last().map(String::as_str), Some("Command"));
        assert_canonical_order(&full);

        // GPU without NPU (the common case)
        let gpu_only = hardware_default_columns(true, false);
        assert!(gpu_only.iter().any(|c| c == "GPU-MEM"));
        assert!(!gpu_only.iter().any(|c| c == "NPU%"));
    }

    #[test]
    fn enabling_columns_inserts_at_canonical_position() {
        let defaults = crate::config::Config::default().visible_columns;
        // GPU% goes right after MEM%, not after Command
        let mem_pos = defaults.iter().position(|c| c == "MEM%").unwrap();
        assert_eq!(canonical_insert_index(&defaults, "GPU%"), mem_pos + 1);
        // Command (canonically last) still appends at the very end
        let no_command: Vec<String> = defaults
            .iter()
            .filter(|c| *c != "Command")
            .cloned()
            .collect();
        assert_eq!(
            canonical_insert_index(&no_command, "Command"),
            no_command.len()
        );
        // PPID slots in directly after PID
        assert_eq!(canonical_insert_index(&defaults, "PPID"), 1);
    }

    #[test]
    fn final_visible_column_cannot_be_removed() {
        let mut app = App::new(Config::default());
        app.screen_tabs[0].columns = vec!["PID".to_string()];
        app.active_tab = 0;

        assert!(!app.toggle_column_in_active_tab("PID"));
        assert_eq!(app.active_tab_columns(), ["PID"]);
        assert!(app.last_error.is_some());
    }

    #[test]
    fn pid_prefix_search_ignores_display_sort_order() {
        let mut app = App::new(Config::default());
        app.displayed_processes = vec![process(5000, 0), process(1299, 0), process(1234, 0)];

        for digit in "1234".chars() {
            app.handle_pid_digit(digit);
        }
        assert_eq!(app.selected_process().map(|p| p.pid), Some(1234));

        app.pid_search_buffer.clear();
        app.handle_pid_digit('1');
        assert_eq!(app.selected_process().map(|p| p.pid), Some(1234));
    }

    #[test]
    fn repeated_search_next_visits_all_matches() {
        let mut app = App::new(Config::default());
        app.displayed_processes = vec![process(1, 0), process(2, 0), process(3, 0)];
        app.search_string = "p".to_string();
        app.search_string_lower = "p".to_string();
        app.start_search();

        app.update_search_from_dialog(false);
        app.find_next();
        assert_eq!(app.selected_process().map(|p| p.pid), Some(2));
        app.update_search_from_dialog(false);
        app.find_next();
        assert_eq!(app.selected_process().map(|p| p.pid), Some(3));
        app.update_search_from_dialog(false);
        app.find_next();
        assert_eq!(app.selected_process().map(|p| p.pid), Some(1));
    }

    #[test]
    fn dependent_sort_waits_for_collector_enrichment() {
        let mut app = App::new(Config::default());
        app.processes = vec![process(1, 0)];
        app.displayed_processes = vec![process(99, 0)];
        app.sort_column = SortColumn::User;

        app.update_displayed_processes();
        assert_eq!(app.displayed_processes[0].pid, 99);

        app.canonical_enrichment = crate::system::ProcessEnrichmentRequirements {
            user: true,
            ..Default::default()
        };
        app.update_displayed_processes();
        assert_eq!(app.displayed_processes[0].pid, 1);
    }

    #[test]
    fn text_filter_requests_only_its_canonical_dependencies() {
        let mut app = App::new(Config::default());
        app.filter_string = "alice".to_string();
        app.filter_string_lower = "alice".to_string();

        let requirements = app.canonical_enrichment_requirements();
        assert!(requirements.user);
        assert!(requirements.exe_path);
        assert!(!requirements.arch);
        assert!(!requirements.elevation);
        assert!(!requirements.efficiency);
    }

    #[test]
    fn open_user_picker_refreshes_owners_and_preserves_selection() {
        let mut app = App::new(Config::default());
        app.canonical_enrichment = crate::system::ProcessEnrichmentRequirements {
            user: true,
            ..Default::default()
        };
        app.user_filter = Some("bob".to_string());
        app.processes = vec![process_as_user(1, "alice"), process_as_user(2, "bob")];
        app.enter_user_select_mode();

        app.apply_snapshot(crate::data::SystemSnapshot {
            metrics: SystemMetrics::default(),
            processes: vec![process_as_user(2, "bob"), process_as_user(3, "carol")],
            refresh_duration: Duration::ZERO,
            enrichment: crate::system::ProcessEnrichmentRequirements {
                user: true,
                ..Default::default()
            },
        });

        let DialogState::UserSelect { index, users } = &app.dialog else {
            panic!("user picker should remain open");
        };
        assert!(users.iter().any(|user| user == "carol"));
        assert_eq!(users.get(*index - 1).map(String::as_str), Some("bob"));
    }

    #[test]
    fn collapsed_tree_suppresses_entire_branch() {
        let mut app = App::new(Config::default());
        let root = process(1, 0);
        app.collapsed_pids.insert(root.identity());
        app.processes = vec![root, process(2, 1), process(3, 2), process(4, 0)];
        app.tree_view = true;
        app.sort_column = SortColumn::Pid;
        app.sort_ascending = true;

        app.update_displayed_processes();
        let pids: Vec<u32> = app.displayed_processes.iter().map(|p| p.pid).collect();
        assert_eq!(pids, [1, 4]);
    }

    #[test]
    fn deep_tree_keeps_every_node_without_recursion_limit() {
        let mut app = App::new(Config::default());
        app.processes = (1..=80)
            .map(|pid| process(pid, if pid == 1 { 0 } else { pid - 1 }))
            .collect();
        app.tree_view = true;
        app.sort_column = SortColumn::Pid;
        app.sort_ascending = true;

        app.update_displayed_processes();
        assert_eq!(app.displayed_processes.len(), 80);
        assert_eq!(app.displayed_processes.last().unwrap().tree_depth, 79);
    }

    #[test]
    fn tree_cycle_emits_each_process_once() {
        let mut app = App::new(Config::default());
        app.processes = vec![process(1, 2), process(2, 1), process(3, 0)];
        app.tree_view = true;
        app.sort_column = SortColumn::Pid;
        app.sort_ascending = true;

        app.update_displayed_processes();
        let identities: HashSet<ProcessIdentity> = app
            .displayed_processes
            .iter()
            .map(ProcessInfo::identity)
            .collect();
        assert_eq!(app.displayed_processes.len(), 3);
        assert_eq!(identities.len(), 3);
    }

    #[test]
    fn stale_identity_tag_is_pruned_on_pid_reuse() {
        let mut app = App::new(Config::default());
        let original = process(42, 0);
        app.tagged_pids.insert(original.identity());
        let mut replacement = original;
        replacement.create_time_100ns += 1;
        app.processes = vec![replacement];

        app.update_displayed_processes();
        assert!(app.tagged_pids.is_empty());
    }

    #[test]
    fn stale_follow_and_branch_identity_do_not_attach_to_reused_pid() {
        let mut app = App::new(Config::default());
        let original = process(42, 0);
        let stale_identity = original.identity();
        app.follow_pid = Some(stale_identity);

        let mut replacement = original;
        replacement.create_time_100ns += 1;
        app.processes = vec![replacement];
        app.update_displayed_processes();

        assert_eq!(app.follow_pid, None);
        app.toggle_tag_branch(stale_identity);
        assert!(app.tagged_pids.is_empty());
    }

    #[test]
    fn runtime_readonly_survives_persisted_config_reset() {
        let mut app = App::new(Config::default());
        app.set_runtime_readonly(true);
        app.config.readonly = true;
        app.config.reset_to_defaults();
        app.apply_config_to_live_state();

        assert!(app.is_readonly());
        assert!(!app.config.readonly);
    }

    #[test]
    fn failed_config_save_stays_dirty_and_visible() {
        let mut app = App::new(Config::default());
        app.config_dirty = true;

        assert!(!app.record_config_save_result(Err("disk full".to_string())));
        assert!(app.config_dirty);
        assert!(
            app.last_error
                .as_ref()
                .is_some_and(|(message, _)| message.contains("disk full"))
        );

        let first_error_time = app.last_error.as_ref().unwrap().1;
        assert!(!app.flush_config());
        assert_eq!(app.last_error.as_ref().unwrap().1, first_error_time);

        // A later user mutation makes one new tick-time attempt eligible.
        app.mark_config_dirty();
        assert!(!app.config_save_failed);

        assert!(app.record_config_save_result(Ok(())));
        assert!(!app.config_dirty);
    }
}
