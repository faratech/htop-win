use crate::config::Config;
use crate::system::{ProcessIdentity, ProcessInfo, SystemMetrics};
use crate::terminal::Rect;
use crate::ui::colors::Theme;
use std::collections::{HashMap, HashSet, VecDeque};
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

/// Process data frozen when a termination action is requested.
#[derive(Debug, Clone)]
pub struct TerminationTarget {
    pub identity: ProcessIdentity,
    pub name: String,
    pub command: String,
}

impl From<&ProcessInfo> for TerminationTarget {
    fn from(process: &ProcessInfo) -> Self {
        Self {
            identity: process.identity(),
            name: process.name.to_string(),
            command: process.command.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum TerminationRequest {
    Single(TerminationTarget),
    Tagged(Vec<TerminationTarget>),
}

impl TerminationRequest {
    pub fn targets(&self) -> &[TerminationTarget] {
        match self {
            Self::Single(target) => std::slice::from_ref(target),
            Self::Tagged(targets) => targets,
        }
    }

    pub fn tagged_count(&self) -> usize {
        match self {
            Self::Single(_) => 0,
            Self::Tagged(targets) => targets.len(),
        }
    }
}

/// One row of the process list: an index into `App::processes` plus what the
/// list view adds to that process (tree layout, search highlight).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayRow {
    /// Index into `App::processes`.
    pub index: usize,
    pub tree_depth: u16,
    /// Tree display prefix (├─, └─, │, etc.).
    pub tree_prefix: String,
    /// Has child processes (tree view).
    pub has_children: bool,
    /// Collapsed in tree view.
    pub is_collapsed: bool,
    /// Matches the active search (highlighted).
    pub matches_search: bool,
}

impl DisplayRow {
    /// A flat (non-tree) row for `processes[index]`.
    fn flat(index: usize) -> Self {
        Self {
            index,
            ..Self::default()
        }
    }
}

/// Resolve ancestry from this snapshot, rejecting reused or unknown parents.
/// Index-space equivalents of the old pid-keyed HashMaps in `build_tree`:
/// pid→index, validated parent links, and children as CSR arrays. Buffers are
/// cleared and reused across ticks; only tree glyphs/prefixes allocate.
#[derive(Default)]
struct TreeScratch {
    idx_of: HashMap<u32, u32>,
    parent_idx: Vec<u32>,
    child_cnt: Vec<u32>,
    child_off: Vec<u32>,
    child_list: Vec<u32>,
    visited: Vec<bool>,
    suppressed: Vec<bool>,
    stack: Vec<PendingNode>,
}

/// DFS work item for `build_tree`.
#[derive(Debug)]
struct PendingNode {
    idx: u32,
    depth: usize,
    is_last: bool,
    /// Shared prefix of the parent: siblings bump the refcount instead of
    /// each cloning the whole ancestor chain (O(depth) allocs).
    parent_prefix: std::sync::Arc<str>,
}

fn validated_parents(processes: &[ProcessInfo]) -> std::collections::HashMap<u32, u32> {
    let times: std::collections::HashMap<_, _> = processes
        .iter()
        .map(|p| (p.pid, p.create_time_100ns))
        .collect();
    processes
        .iter()
        .filter_map(|child| {
            let parent_time = *times.get(&child.parent_pid)?;
            (child.parent_pid != 0
                && child.parent_pid != child.pid
                && parent_time != 0
                && child.create_time_100ns != 0
                && parent_time <= child.create_time_100ns)
                .then_some((child.pid, child.parent_pid))
        })
        .collect()
}

/// Order-preserving integer image of an `f32` for sorting: equal floats get
/// equal keys (`-0.0` sorts with `0.0`), larger floats larger keys.
fn f32_sort_key(value: f32) -> u64 {
    let bits = (value + 0.0).to_bits();
    u64::from(if bits & 0x8000_0000 != 0 {
        !bits
    } else {
        bits | 0x8000_0000
    })
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
        request: TerminationRequest,
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
    /// The process list as displayed: filtered, sorted (or tree-ordered) rows
    /// indexing into `processes`. `processes` only changes together with a
    /// rebuild of these rows (see `apply_snapshot`).
    display: Vec<DisplayRow>,
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
    /// Error identity timestamp; only the footer notice auto-expires.
    pub last_error: Option<(String, Instant)>,
    pub error_scroll: usize,
    pub error_visible_rows: usize,
    error_seen_at: Option<Instant>,
    #[cfg(test)]
    pub termination_log: Option<Vec<ProcessIdentity>>,
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
    /// Reusable index-space buffers for `build_tree` (cleared, not reallocated).
    tree_scratch: TreeScratch,
    /// Spare buffers for the display rows and their sort entries (same
    /// recycle pattern as the collector's snapshot vec).
    display_scratch: Vec<DisplayRow>,
    order_scratch: Vec<(u64, usize)>,
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
    /// Display rows enriched by the last enrichment pass
    /// (see `enrich_viewport`).
    enriched_rows: std::ops::Range<usize>,
    /// Some row in `enriched_rows` still needs a Windows metadata query; it
    /// runs after the frame is drawn (see `run_deferred_enrichment`).
    enrichment_pending: bool,
    /// Set by Ctrl+L: the event loop clears the terminal and repaints every
    /// cell on the next draw instead of diffing against the last frame.
    pub full_redraw_requested: bool,
    /// Fixed wall clock (Unix seconds) for rendering, so time-derived cells
    /// (START, new-process highlight) are deterministic in tests.
    pub clock_override: Option<u64>,
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
            display: Vec::new(),
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
            error_scroll: 0,
            error_visible_rows: 0,
            error_seen_at: None,
            #[cfg(test)]
            termination_log: None,
            status_message: None,
            config_dirty: false,
            config_save_failed: false,
            collapsed_pids: HashSet::new(),
            tree_scratch: TreeScratch::default(),
            display_scratch: Vec::new(),
            order_scratch: Vec::new(),
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
            enriched_rows: 0..0,
            enrichment_pending: false,
            full_redraw_requested: false,
            clock_override: None,
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
        let request = if self.tagged_pids.is_empty() {
            let Some(process) = self.selected_process() else {
                return;
            };
            TerminationRequest::Single(process.into())
        } else {
            self.capture_tagged_termination()
        };
        if self.config.confirm_kill {
            self.dialog = DialogState::Kill { request };
        } else {
            self.execute_termination(request);
        }
    }

    fn capture_tagged_termination(&self) -> TerminationRequest {
        let mut targets: Vec<_> = self
            .tagged_pids
            .iter()
            .map(|identity| {
                self.processes
                    .iter()
                    .find(|process| process.identity() == *identity)
                    .map(TerminationTarget::from)
                    .unwrap_or_else(|| TerminationTarget {
                        identity: *identity,
                        name: "(unavailable)".to_string(),
                        command: String::new(),
                    })
            })
            .collect();
        targets.sort_by_key(|target| target.identity.pid);
        TerminationRequest::Tagged(targets)
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

    /// Wall-clock Unix seconds used for rendering (see `clock_override`).
    pub fn now_unix_secs(&self) -> u64 {
        self.clock_override.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        })
    }

    /// Whether the collector has already populated `requirements` for the
    /// current process set (the event loop keeps waiting for a snapshot while
    /// paused until it has).
    pub fn has_enrichment_for(
        &self,
        requirements: crate::system::ProcessEnrichmentRequirements,
    ) -> bool {
        self.canonical_enrichment.contains(requirements)
    }

    /// Apply a snapshot from the background data collector. Returns the
    /// process list it replaced, for the collector to recycle.
    pub fn apply_snapshot(&mut self, snapshot: crate::data::SystemSnapshot) -> Vec<ProcessInfo> {
        self.system_metrics = snapshot.metrics;
        let replaced = if snapshot
            .enrichment
            .contains(self.canonical_enrichment_requirements())
        {
            self.canonical_enrichment = snapshot.enrichment;
            let replaced = std::mem::replace(&mut self.processes, snapshot.processes);
            self.update_displayed_processes();
            self.refresh_process_info_stats();
            if self.pending_user_select && self.canonical_enrichment.user {
                self.open_user_select_dialog();
            } else if self.canonical_enrichment.user
                && matches!(self.dialog, DialogState::UserSelect { .. })
            {
                self.refresh_user_select_dialog();
            }
            replaced
        } else {
            // The filter/sort/dialog needs metadata this snapshot lacks: keep
            // the last authoritative process list (the displayed rows index
            // into it) until the collector returns one with it populated.
            snapshot.processes
        };
        self.update_meter_history();
        replaced
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
        crate::system::set_gpu_selection(self.config.gpu_meter_adapter.as_deref());
    }

    /// Canonical metadata the collector must populate for the active
    /// filter/sort/dialog. The default returns no requirements, keeping normal
    /// steady-state collection free of all-process Windows calls.
    /// Which collector subsystems the visible UI actually needs this tick.
    /// Computed from the same layout arithmetic the header renders with
    /// (`ui::header::filler_plan`), so a gated-off collector is never one the
    /// user could see. Pushed to the collector on change by the event loop.
    pub fn canonical_collect_requirements(&self) -> u8 {
        use crate::system::collect_gates;
        use crate::config::MeterMode;

        if !self.show_header {
            // Nothing in the header renders, so no subsystem is needed.
            return 0;
        }

        let mut bits = 0u8;
        // The Uptime row also renders the average CPU% from the per-core data,
        // so it keeps CPU collection alive even with the CPU meters hidden
        // (issue #97).
        if (self.config.show_cpu_meters && self.config.cpu_meter_mode != MeterMode::Hidden)
            || self.config.show_uptime_meter
        {
            bits |= collect_gates::CPU;
        }

        // Net / Dsk / Bat are filler meters; their visibility comes from the
        // layout plan. (Disk has no dedicated collector — its rates ride with
        // the process scan — so only NET and BATTERY are gated here.)
        let plan = crate::ui::filler_plan(self);
        if plan[0] {
            bits |= collect_gates::NET;
        }
        if plan[2] {
            bits |= collect_gates::BATTERY;
        }

        // GPU/NPU: collect while the meter is enabled OR an adapter is already
        // tracked (presence is then known when the meter is re-enabled, and
        // hardware-aware default columns keep working).
        let gpu_meter_on =
            self.config.show_gpu_meter && self.config.gpu_meter_mode != MeterMode::Hidden;
        let npu_meter_on =
            self.config.show_npu_meter && self.config.npu_meter_mode != MeterMode::Hidden;
        if gpu_meter_on || npu_meter_on || crate::system::has_tracked_adapters() {
            bits |= collect_gates::GPU | collect_gates::NPU;
        }

        bits
    }

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

        let show_kernel = self.config.show_kernel_threads;
        let show_user = self.config.show_user_threads;
        let process_count = self.processes.len();

        let requirements = self.canonical_enrichment_requirements();
        if !self.canonical_enrichment.contains(requirements) {
            // Preserve the last authoritative view until the collector returns
            // this exact process set with its required metadata populated.
            return;
        }

        // Identity pruning needs the live set only when something actually
        // references it: a non-empty tag/follow set, or an oversized collapsed
        // set. In the common steady state (no tags, no follow, modest
        // collapsed set) none of the retains below would remove anything, so
        // skip the O(n) hashed build entirely. Outcomes are identical: each
        // retain below runs exactly when its guard set is non-trivial.
        let live_identities: Option<HashSet<ProcessIdentity>> = (!self.tagged_pids.is_empty()
            || self.follow_pid.is_some()
            || self.collapsed_pids.len() > process_count * 2)
            .then(|| self.processes.iter().map(ProcessInfo::identity).collect());
        if let Some(live) = live_identities.as_ref() {
            self.tagged_pids.retain(|identity| live.contains(identity));
            if self
                .follow_pid
                .is_some_and(|identity| !live.contains(&identity))
            {
                self.follow_pid = None;
            }
        }

        // Prune stale collapsed PIDs only when the set has grown disproportionate to
        // the live process count. Otherwise `collapsed_pids` grows unbounded over long
        // uptime (each collapse_all adds every PID; dead/reused PIDs are never removed).
        if self.collapsed_pids.len() > process_count * 2
            && let Some(live) = live_identities.as_ref()
        {
            self.collapsed_pids
                .retain(|identity| live.contains(identity));
        }

        // Sort entries for the processes that pass every filter, in a reused
        // buffer.
        let sort_entry = self.sort_entry();
        let mut order = std::mem::take(&mut self.order_scratch);
        order.clear();
        order.reserve(process_count);
        order.extend(
            self.processes
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    // Kernel/System threads filter
                    // On Windows, "kernel threads" are SYSTEM user processes.
                    // The string checks are dead when both flags are on (the
                    // default): every process passes both guards regardless.
                    let kernel_check_needed = !show_kernel || !show_user;
                    let is_kernel = kernel_check_needed
                        && (&*p.user_lower == "system"
                            || p.user_lower.starts_with("nt authority")
                            || p.pid == 0
                            || p.pid == 4);

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
                .map(|(index, p)| sort_entry(index, p)),
        );

        self.sort_order(&mut order);

        // Rows are built in the previous display's buffer (capacity included);
        // the old rows move to the spare slot below.
        let mut rows = std::mem::take(&mut self.display_scratch);
        rows.clear();
        if self.tree_view {
            self.build_tree(&order, &mut rows);
        } else {
            rows.extend(order.iter().map(|&(_, index)| DisplayRow::flat(index)));
        }
        if has_search {
            for row in &mut rows {
                let p = &self.processes[row.index];
                row.matches_search = p.name_lower.contains(&self.search_string_lower)
                    || p.command_lower.contains(&self.search_string_lower);
            }
        }
        self.order_scratch = order;
        self.display_scratch = std::mem::replace(&mut self.display, rows);

        // Normal viewing is anchored to the row, not the process: dynamic
        // sorting must not drag the viewport around. Action handlers capture
        // the displayed process identity when invoked; only explicit follow
        // mode below tracks an identity across refreshes.

        // Clamp selection and scroll immediately after replacing the list, before
        // enrichment uses scroll_offset to choose the visible slice.
        if self.selected_index >= self.display.len() {
            self.selected_index = self.display.len().saturating_sub(1);
        }
        if self.display.is_empty() {
            self.scroll_offset = 0;
        } else {
            let max_scroll = self
                .display
                .len()
                .saturating_sub(self.visible_height.max(1));
            self.scroll_offset = self.scroll_offset.min(max_scroll);
            self.ensure_visible();
        }

        // Handle follow mode - find and select the followed PID
        if let Some(follow_identity) = self.follow_pid
            && let Some(idx) = self.display_position(|p| p.identity() == follow_identity)
        {
            self.selected_index = idx;
            self.ensure_visible();
        }

        // Ensure selection is valid
        if self.selected_index >= self.display.len() {
            self.selected_index = self.display.len().saturating_sub(1);
        }

        // The rows changed, so enrich the final viewport (after follow mode
        // may have scrolled it) unconditionally.
        self.enrich_rows_around_viewport();
    }

    /// Enrich the rows in and around the viewport with per-row Windows
    /// metadata (user, elevation, architecture, efficiency, exe path), and
    /// remember which rows were covered.
    fn enrich_rows_around_viewport(&mut self) {
        // Buffer zone so short scrolls stay inside the enriched window.
        const BUFFER_SIZE: usize = 10;
        let start = self.scroll_offset.saturating_sub(BUFFER_SIZE);
        let end = self
            .scroll_offset
            .saturating_add(self.visible_height)
            .saturating_add(BUFFER_SIZE)
            .min(self.display.len());
        self.enriched_rows = start..end.max(start);

        // Cached facts now; any Windows query waits until the frame is out,
        // so a snapshot or scroll is drawn without per-process syscalls in
        // the way (see `run_deferred_enrichment`).
        let requirements = self.visible_enrichment();
        let rows = self.enriched_process_indices();
        self.enrichment_pending = !rows.is_empty()
            && crate::system::apply_cached_metadata(&mut self.processes, &rows, requirements);
    }

    /// `processes` indices of the display rows in `enriched_rows`.
    fn enriched_process_indices(&self) -> Vec<usize> {
        let end = self.enriched_rows.end.min(self.display.len());
        let start = self.enriched_rows.start.min(end);
        self.display[start..end]
            .iter()
            .map(|row| row.index)
            .collect()
    }

    /// Metadata the rows in view need (exe paths only when shown, since
    /// that query is expensive).
    fn visible_enrichment(&self) -> crate::system::ProcessEnrichmentRequirements {
        crate::system::ProcessEnrichmentRequirements::visible(self.config.show_program_path)
    }

    /// Run the metadata queries deferred by the last enrichment pass (rows
    /// new to the view whose facts are not cached yet). Returns true when it
    /// ran, so the caller redraws with the results right away.
    pub fn run_deferred_enrichment(&mut self) -> bool {
        if !std::mem::take(&mut self.enrichment_pending) {
            return false;
        }
        let requirements = self.visible_enrichment();
        let rows = self.enriched_process_indices();
        crate::system::enrich_processes_at(&mut self.processes, &rows, requirements);
        true
    }

    /// Enrich newly visible rows when navigation (End, PgDn, mouse wheel, …)
    /// moved the viewport outside the window enriched by the last list update,
    /// so they don't show placeholder metadata until the next snapshot
    /// (issue #99). A no-op while the viewport stays inside that window.
    pub fn enrich_viewport(&mut self) {
        let view_end = self
            .scroll_offset
            .saturating_add(self.visible_height)
            .min(self.display.len());
        let covered =
            self.enriched_rows.start <= self.scroll_offset && view_end <= self.enriched_rows.end;
        if !covered {
            self.enrich_rows_around_viewport();
        }
    }

    /// Sort key of the active column (`None` for the text columns, which
    /// compare in place; see `sort_order`).
    fn sort_key(&self) -> Option<fn(&ProcessInfo) -> u64> {
        Some(match self.sort_column {
            SortColumn::Cpu => |p| f32_sort_key(p.cpu_percent),
            SortColumn::Mem => |p| f32_sort_key(p.mem_percent),
            SortColumn::Pid => |p| u64::from(p.pid),
            SortColumn::Res => |p| p.resident_mem,
            SortColumn::Time => |p| p.cpu_time,
            SortColumn::PPid => |p| u64::from(p.parent_pid),
            SortColumn::Priority | SortColumn::PriorityClass => {
                |p| u64::from(p.priority.cast_unsigned() ^ 0x8000_0000)
            }
            SortColumn::Threads => |p| u64::from(p.thread_count),
            SortColumn::Virt => |p| p.virtual_mem,
            SortColumn::Shr => |p| p.shared_mem,
            SortColumn::Status => |p| u64::from(p.status),
            SortColumn::StartTime => |p| u64::from(p.start_time),
            SortColumn::Elevated => |p| u64::from(p.is_elevated),
            SortColumn::Arch => |p| u64::from(p.arch.sort_rank()),
            SortColumn::Efficiency => |p| u64::from(p.efficiency_mode),
            SortColumn::HandleCount => |p| u64::from(p.handle_count),
            SortColumn::IoRate => |p| p.io_read_rate.wrapping_add(p.io_write_rate),
            SortColumn::IoReadRate => |p| p.io_read_rate,
            SortColumn::IoWriteRate => |p| p.io_write_rate,
            SortColumn::IoRead => |p| p.io_read_bytes,
            SortColumn::IoWrite => |p| p.io_write_bytes,
            SortColumn::Gpu => |p| f32_sort_key(p.gpu_percent),
            SortColumn::GpuMem => |p| p.gpu_memory,
            SortColumn::Npu => |p| f32_sort_key(p.npu_percent),
            SortColumn::NpuMem => |p| p.npu_memory,
            SortColumn::User | SortColumn::Command => return None,
        })
    }

    /// Builds the `(key, index)` entry `sort_order` sorts for
    /// `processes[index]`: the column's key, inverted for a descending sort.
    fn sort_entry(&self) -> impl Fn(usize, &ProcessInfo) -> (u64, usize) + use<> {
        let key = self.sort_key();
        let flip = if self.sort_ascending { 0 } else { u64::MAX };
        move |index, process| (key.map_or(0, |key| key(process)) ^ flip, index)
    }

    /// Sort `sort_entry` entries by the active sort column. Ties keep
    /// collector order, so equal rows don't trade places between snapshots.
    /// Numeric columns sort the entries' keys in place (built during the
    /// filter pass, so this reads no process); text columns compare in place.
    fn sort_order(&self, order: &mut [(u64, usize)]) {
        let text: fn(&ProcessInfo) -> &str = match self.sort_column {
            SortColumn::User => |p| &p.user,
            SortColumn::Command => |p| &p.command,
            _ => {
                order.sort_unstable();
                return;
            }
        };
        let processes = &self.processes[..];
        let ascending = self.sort_ascending;
        order.sort_unstable_by(|&(_, a), &(_, b)| {
            let ord = text(&processes[a]).cmp(text(&processes[b]));
            if ascending { ord } else { ord.reverse() }.then(a.cmp(&b))
        });
    }

    /// Append the tree-ordered rows for `order` (sorted `sort_entry`
    /// entries) to `rows`. Position `i` below means `processes[order[i].1]`.
    fn build_tree(&mut self, order: &[(u64, usize)], rows: &mut Vec<DisplayRow>) {
        use std::sync::Arc;

        const NONE: u32 = u32::MAX;
        let all_processes = &self.processes;
        let scratch = &mut self.tree_scratch;
        let TreeScratch {
            idx_of,
            parent_idx,
            child_cnt,
            child_off,
            child_list,
            visited,
            suppressed,
            stack,
        } = scratch;

        let process_count = order.len();
        let processes = || order.iter().map(|&(_, index)| &all_processes[index]);

        // pid → position (last occurrence wins, matching the old HashMap insert).
        idx_of.clear();
        idx_of.reserve(process_count);
        for (idx, process) in processes().enumerate() {
            idx_of.insert(process.pid, idx as u32);
        }

        // Validated parent links in index space, with the same rules as
        // `validated_parents` (parent in list, nonzero create times, parent
        // not created after the child).
        parent_idx.clear();
        parent_idx.reserve(process_count);
        child_cnt.clear();
        child_cnt.resize(process_count, 0);
        for process in processes() {
            let validated = (process.parent_pid != 0 && process.parent_pid != process.pid)
                .then(|| idx_of.get(&process.parent_pid).copied())
                .flatten()
                .filter(|&parent| {
                    let parent_time = all_processes[order[parent as usize].1].create_time_100ns;
                    parent_time != 0
                        && process.create_time_100ns != 0
                        && parent_time <= process.create_time_100ns
                });
            match validated {
                Some(parent) => {
                    parent_idx.push(parent);
                    child_cnt[parent as usize] += 1;
                }
                None => parent_idx.push(NONE),
            }
        }

        // CSR children arrays, children in original (sort) order per parent.
        child_off.clear();
        child_off.resize(process_count + 1, 0);
        for i in 0..process_count {
            child_off[i + 1] = child_off[i] + child_cnt[i];
        }
        child_list.clear();
        child_list.resize(child_off[process_count] as usize, 0);
        let mut fill = child_off[..process_count].to_vec();
        for (idx, &parent) in parent_idx.iter().enumerate() {
            if parent != NONE {
                let slot = &mut fill[parent as usize];
                child_list[*slot as usize] = idx as u32;
                *slot += 1;
            }
        }
        drop(fill);

        visited.clear();
        visited.resize(process_count, false);
        suppressed.clear();
        suppressed.resize(process_count, false);
        rows.reserve(process_count);

        // Traverse rooted components first, then cycle-only components in the
        // original sort order (the old `roots ++ order` iteration). Iteration
        // avoids both the old depth cutoff and call-stack overflow while the
        // visited set emits every node once.
        stack.clear();
        stack.reserve(process_count);
        for pass in 0..2 {
            for start in 0..process_count as u32 {
                if pass == 0 && parent_idx[start as usize] != NONE {
                    continue; // pass 0: validated roots only
                }
                if visited[start as usize] || suppressed[start as usize] {
                    continue;
                }
                stack.push(PendingNode {
                    idx: start,
                    depth: 0,
                    is_last: true,
                    parent_prefix: Arc::from(""),
                });
                while let Some(pending) = stack.pop() {
                    let pending_idx = pending.idx as usize;
                    if visited[pending_idx] || suppressed[pending_idx] {
                        continue;
                    }
                    visited[pending_idx] = true;
                    let index = order[pending_idx].1;
                    let identity = all_processes[index].identity();
                    let child_range =
                        child_off[pending_idx] as usize..child_off[pending_idx + 1] as usize;
                    let child_slice = &child_list[child_range];
                    let is_collapsed = self.collapsed_pids.contains(&identity);

                    let tree_prefix = if pending.depth == 0 {
                        String::new()
                    } else {
                        let branch = if pending.is_last { "└─ " } else { "├─ " };
                        // One allocation sized up front; the old clone-then-push
                        // allocated twice (clone at parent length, then regrow).
                        let mut prefix =
                            String::with_capacity(pending.parent_prefix.len() + branch.len());
                        prefix.push_str(&pending.parent_prefix);
                        prefix.push_str(branch);
                        prefix
                    };
                    rows.push(DisplayRow {
                        index,
                        tree_depth: pending.depth.min(u16::MAX as usize) as u16,
                        tree_prefix,
                        has_children: !child_slice.is_empty(),
                        is_collapsed,
                        matches_search: false,
                    });

                    if is_collapsed {
                        // Descendants of a collapsed node are intentionally
                        // hidden, not orphans. Mark the entire branch so the
                        // fallback traversal cannot append it at the top level.
                        let mut descendants: Vec<u32> = child_slice.to_vec();
                        while let Some(descendant) = descendants.pop() {
                            let d = descendant as usize;
                            if visited[d] || suppressed[d] {
                                continue;
                            }
                            suppressed[d] = true;
                            let d_range = child_off[d] as usize..child_off[d + 1] as usize;
                            descendants.extend_from_slice(&child_list[d_range]);
                        }
                        continue;
                    }

                    // One shared allocation per visited node; every child bumps
                    // the refcount instead of cloning the ancestor chain.
                    let child_parent_prefix: Arc<str> = if pending.depth == 0 {
                        Arc::from("")
                    } else {
                        let continuation = if pending.is_last { "   " } else { "│  " };
                        let mut prefix =
                            String::with_capacity(pending.parent_prefix.len() + continuation.len());
                        prefix.push_str(&pending.parent_prefix);
                        prefix.push_str(continuation);
                        Arc::from(prefix)
                    };
                    let child_count = child_slice.len();
                    for (index, &child) in child_slice.iter().enumerate().rev() {
                        stack.push(PendingNode {
                            idx: child,
                            depth: pending.depth + 1,
                            is_last: index + 1 == child_count,
                            parent_prefix: child_parent_prefix.clone(),
                        });
                    }
                }
            }
        }
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
        if self.selected_index < self.display.len().saturating_sub(1) {
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
            (self.selected_index + page_size).min(self.display.len().saturating_sub(1));
        self.ensure_visible();
    }

    /// Go to first process
    pub fn select_first(&mut self) {
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Go to last process
    pub fn select_last(&mut self) {
        self.selected_index = self.display.len().saturating_sub(1);
        self.ensure_visible();
    }

    /// Apply geometry before rendering, including while collection is paused.
    pub fn set_visible_height(&mut self, height: usize) {
        self.visible_height = height;
        let len = self.display.len();
        self.selected_index = self.selected_index.min(len.saturating_sub(1));
        self.scroll_offset = self.scroll_offset.min(len.saturating_sub(height.max(1)));
        self.ensure_visible();
    }

    /// Ensure selected item is visible
    fn ensure_visible(&mut self) {
        if self.visible_height == 0 {
            // Issue #75: with no rows drawn the window below degenerates and
            // would set scroll_offset = selected_index + 1, one row past the
            // last valid index. There is no viewport to satisfy, so just keep
            // the offset anchored inside the list.
            let last = self.display.len().saturating_sub(1);
            self.scroll_offset = self.scroll_offset.min(last);
            return;
        }
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + self.visible_height {
            self.scroll_offset = self.selected_index - self.visible_height + 1;
        }
    }

    /// Toggle tag on selected process
    pub fn toggle_tag(&mut self) {
        if let Some(proc) = self.selected_process() {
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
                .displayed_processes()
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
            .displayed_processes()
            .all(|p| self.tagged_pids.contains(&p.identity()));
        let visible: Vec<ProcessIdentity> = self
            .displayed_processes()
            .map(ProcessInfo::identity)
            .collect();

        if all_tagged {
            // Untag all visible
            for identity in visible {
                self.tagged_pids.remove(&identity);
            }
        } else {
            // Tag all visible
            self.tagged_pids.extend(visible);
        }
    }

    /// Get selected process
    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        self.displayed(self.selected_index)
    }

    pub fn process_by_identity(&self, identity: ProcessIdentity) -> Option<&ProcessInfo> {
        self.displayed_processes()
            .find(|process| process.identity() == identity)
    }

    /// Number of rows in the process list.
    pub fn displayed_len(&self) -> usize {
        self.display.len()
    }

    /// The process shown at `row` of the process list.
    pub fn displayed(&self, row: usize) -> Option<&ProcessInfo> {
        self.display
            .get(row)
            .and_then(|row| self.processes.get(row.index))
    }

    /// The process list's rows from `start` on, with their processes.
    pub fn display_rows_from(
        &self,
        start: usize,
    ) -> impl Iterator<Item = (&DisplayRow, &ProcessInfo)> + Clone {
        self.display
            .get(start..)
            .unwrap_or_default()
            .iter()
            .filter_map(|row| Some((row, self.processes.get(row.index)?)))
    }

    /// The processes of the process list, in display order.
    pub fn displayed_processes(&self) -> impl Iterator<Item = &ProcessInfo> + Clone {
        self.display_rows_from(0).map(|(_, process)| process)
    }

    /// Row of the first displayed process matching `predicate`.
    fn display_position(&self, predicate: impl FnMut(&ProcessInfo) -> bool) -> Option<usize> {
        self.displayed_processes().position(predicate)
    }

    /// Show `processes` in exactly this order, bypassing filter and sort.
    #[cfg(test)]
    fn set_display_for_test(&mut self, processes: Vec<ProcessInfo>) {
        self.display = (0..processes.len()).map(DisplayRow::flat).collect();
        self.processes = processes;
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
            && let Some(idx) = self.display_position(|p| {
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
        if self.search_string_lower.is_empty() || self.display.is_empty() {
            return;
        }
        let start = self.selected_index + 1;
        for i in 0..self.display.len() {
            let idx = (start + i) % self.display.len();
            let Some(p) = self.displayed(idx) else {
                continue;
            };
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

    /// Execute only the immutable request captured when the dialog opened.
    pub fn kill_target_process(&mut self) {
        if let DialogState::Kill { request } = &self.dialog {
            self.execute_termination(request.clone());
        }
    }

    fn terminate_identity(&mut self, identity: ProcessIdentity) -> Result<(), String> {
        #[cfg(test)]
        if let Some(log) = &mut self.termination_log {
            log.push(identity);
            return Ok(());
        }
        crate::system::kill_process(identity)
    }

    fn execute_termination(&mut self, request: TerminationRequest) {
        if self.readonly_blocked("kill processes") {
            return;
        }
        let total = request.targets().len();
        let mut killed = 0;
        for target in request.targets() {
            match self.terminate_identity(target.identity) {
                Ok(()) => killed += 1,
                Err(error) => {
                    self.last_error = Some((
                        format!(
                            "Failed to terminate {} ({}): {}",
                            target.name, target.identity.pid, error
                        ),
                        Instant::now(),
                    ))
                }
            }
            self.tagged_pids.remove(&target.identity);
        }
        self.status_message = Some((
            format!(
                "Force terminated {killed}/{total} processes ({} failed)",
                total - killed
            ),
            Instant::now(),
        ));
    }

    /// Terminate the tags captured at the instant this action is invoked.
    pub fn kill_tagged(&mut self) {
        self.execute_termination(self.capture_tagged_termination());
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

    /// Reset error navigation when a newly reported error replaces the overlay.
    pub fn sync_error_scroll(&mut self) {
        let at = self.last_error.as_ref().map(|(_, at)| *at);
        if self.error_seen_at != at {
            self.error_scroll = 0;
            self.error_seen_at = at;
        }
    }

    /// Dismiss the error overlay and reset its navigation.
    pub fn clear_error(&mut self) {
        self.last_error = None;
        self.sync_error_scroll();
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
            && let Some(index) = self.display_position(|process| process.identity() == identity)
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
        let parents = validated_parents(&self.processes);
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
                    .filter(|process| parents.get(&process.pid) == Some(&parent_pid))
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
                .displayed_processes()
                .find(|process| process.pid == search_pid)
                .or_else(|| {
                    self.displayed_processes()
                        .filter(|process| {
                            process.pid.to_string().starts_with(&self.pid_search_buffer)
                        })
                        .min_by_key(|process| process.pid)
                })
                .map(ProcessInfo::identity);
            if let Some(identity) = match_identity
                && let Some(index) = self.display_position(|process| process.identity() == identity)
            {
                self.selected_index = index;
                self.ensure_visible();
            }
        }
    }

    /// Collapse to parent in tree view
    pub fn collapse_to_parent(&mut self) {
        if let Some(proc) = self.selected_process() {
            let parents = validated_parents(&self.processes);
            let Some(&parent_pid) = parents.get(&proc.pid) else {
                return;
            };
            // Find parent in displayed processes and select it
            let parent = self
                .displayed_processes()
                .enumerate()
                .find(|(_, process)| process.pid == parent_pid)
                .map(|(index, process)| (index, process.identity()));
            if let Some((index, identity)) = parent {
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
            status: b'S',
            cpu_percent: 0.0,
            mem_percent: 0.0,
            virtual_mem: 0,
            resident_mem: 0,
            shared_mem: 0,
            priority: 8,
            cpu_time: 0,
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
    fn tagged_confirmation_never_retargets_after_exits_or_pid_reuse() {
        use crossterm::event::{
            KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        for mouse in [false, true] {
            for reuse in [false, true] {
                let mut app = App::new(Config::default());
                app.termination_log = Some(Vec::new());
                app.processes = vec![process(1, 0), process(2, 0), process(3, 0)];
                let expected = vec![app.processes[0].identity(), app.processes[1].identity()];
                app.tagged_pids.extend(expected.iter().copied());
                // Only untagged B is displayed; tags retain canonical names.
                app.filter_string_lower = "p3".into();
                app.canonical_enrichment = app.canonical_enrichment_requirements();
                app.update_displayed_processes();
                crate::input::handle_key_event(
                    &mut app,
                    KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE),
                );
                let DialogState::Kill { request } = &app.dialog else {
                    panic!("kill dialog");
                };
                assert_eq!(request.tagged_count(), 2);
                assert_eq!(request.targets()[0].name, "p1");
                app.processes.remove(0);
                if reuse {
                    app.processes[0].create_time_100ns += 1;
                } else {
                    app.processes.remove(0);
                }
                app.update_displayed_processes();
                assert!(app.tagged_pids.is_empty());
                if mouse {
                    let mut buffer = crate::terminal::Buffer::empty(Rect::new(0, 0, 80, 25));
                    crate::ui::draw(&mut crate::terminal::Frame::new(&mut buffer), &mut app);
                    let inner = app.dialog_inner.unwrap();
                    let click = MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: inner.x + 1,
                        row: inner.y + 5,
                        modifiers: KeyModifiers::NONE,
                    };
                    crate::input::handle_mouse_event(&mut app, click);
                    assert!(app.termination_log.as_ref().unwrap().is_empty());
                    crate::input::handle_mouse_event(&mut app, click);
                } else {
                    crate::input::handle_key_event(
                        &mut app,
                        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    );
                }
                assert_eq!(app.termination_log.unwrap(), expected);
            }
        }
    }

    #[test]
    fn partial_batch_exits_and_readonly_preserve_captured_targets() {
        let mut app = App::new(Config::default());
        app.termination_log = Some(Vec::new());
        app.processes = vec![process(1, 0), process(2, 0)];
        let expected: Vec<_> = app.processes.iter().map(ProcessInfo::identity).collect();
        app.tagged_pids.extend(expected.iter().copied());
        app.update_displayed_processes();
        app.enter_kill_mode();
        app.processes.remove(0);
        app.update_displayed_processes();
        app.config.readonly = true;
        app.kill_target_process();
        assert!(app.termination_log.as_ref().unwrap().is_empty());
        app.config.readonly = false;
        app.kill_target_process();
        assert_eq!(app.termination_log.unwrap(), expected);
    }

    #[test]
    fn parent_edges_are_validated_for_tree_tags_and_navigation() {
        for (parent_time, child_time, linked) in [
            (200, 100, false),
            (0, 100, false),
            (100, 0, false),
            (100, 100, true),
            (100, 200, true),
        ] {
            let mut app = App::new(Config::default());
            app.processes = vec![process(1, 0), process(2, 1), process(3, 2)];
            app.processes[0].create_time_100ns = parent_time;
            app.processes[1].create_time_100ns = child_time;
            app.processes[2].create_time_100ns = child_time + 1;
            app.tree_view = true;
            app.sort_column = SortColumn::Pid;
            app.sort_ascending = true;
            app.update_displayed_processes();
            assert_eq!(usize::from(app.display[1].tree_depth), usize::from(linked));
            app.tag_with_children();
            assert_eq!(app.tagged_pids.len(), if linked { 3 } else { 1 });
            app.selected_index = 1;
            app.collapse_to_parent();
            assert_eq!(app.selected_index, if linked { 0 } else { 1 });
        }
        let mut cycle = vec![process(1, 2), process(2, 1), process(3, 99)];
        for p in &mut cycle {
            p.create_time_100ns = 100;
        }
        let mut app = App::new(Config::default());
        app.processes = cycle;
        app.tree_view = true;
        app.update_displayed_processes();
        assert_eq!(app.displayed_len(), 3);
        assert_eq!(app.branch_identities(app.processes[0].identity()).len(), 2);
    }

    #[test]
    fn virtual_memory_sort_uses_virtual_size() {
        let mut app = App::new(Config::default());
        app.sort_column = SortColumn::Virt;
        app.processes = vec![process(1, 0), process(2, 0)];
        app.processes[0].virtual_mem = 1 << 30;
        app.processes[1].virtual_mem = 1 << 20;
        app.update_displayed_processes();
        assert_eq!(app.displayed(0).unwrap().pid, 1);
        app.sort_ascending = true;
        app.update_displayed_processes();
        assert_eq!(app.displayed(0).unwrap().pid, 2);
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
        app.set_display_for_test(vec![process(5000, 0), process(1299, 0), process(1234, 0)]);

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
        app.set_display_for_test(vec![process(1, 0), process(2, 0), process(3, 0)]);
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
        let snapshot = |pid, enrichment| crate::data::SystemSnapshot {
            metrics: SystemMetrics::default(),
            processes: vec![process(pid, 0)],
            refresh_duration: Duration::ZERO,
            enrichment,
            published_at: Instant::now(),
        };
        let with_user = crate::system::ProcessEnrichmentRequirements {
            user: true,
            ..Default::default()
        };
        let mut app = App::new(Config::default());
        app.apply_snapshot(snapshot(99, Default::default()));
        app.sort_column = SortColumn::User;

        // The sort needs owners: the view (and the list it indexes) stays put.
        app.update_displayed_processes();
        assert_eq!(app.displayed(0).unwrap().pid, 99);
        let returned = app.apply_snapshot(snapshot(1, Default::default()));
        assert_eq!(
            returned[0].pid, 1,
            "the unusable snapshot goes back to be recycled"
        );
        assert_eq!(app.displayed(0).unwrap().pid, 99);

        let returned = app.apply_snapshot(snapshot(2, with_user));
        assert_eq!(returned[0].pid, 99);
        assert_eq!(app.displayed(0).unwrap().pid, 2);
    }

    #[test]
    fn dynamic_sorts_keep_selection_and_viewport_stationary() {
        for column in [
            SortColumn::Cpu,
            SortColumn::Mem,
            SortColumn::Gpu,
            SortColumn::Npu,
            SortColumn::IoRate,
            SortColumn::Res,
            SortColumn::Time,
        ] {
            for ascending in [false, true] {
                let mut app = App::new(Config::default());
                app.sort_column = column;
                app.sort_ascending = ascending;
                app.visible_height = 3;
                app.processes = (1..=12).map(|pid| process(pid, 0)).collect();
                for tick in 0..12 {
                    // Rotate the ranking so every identity crosses the viewport.
                    for p in &mut app.processes {
                        let value = (p.pid + tick) % 12;
                        p.cpu_percent = value as f32;
                        p.mem_percent = value as f32;
                        p.gpu_percent = value as f32;
                        p.npu_percent = value as f32;
                        p.io_read_rate = value as u64;
                        p.resident_mem = value as u64;
                        p.cpu_time = value as u64 * 10_000_000;
                    }
                    app.update_displayed_processes();
                    if tick == 0 {
                        app.selected_index = 6;
                        app.scroll_offset = 5;
                    }
                    assert_eq!(app.selected_index, 6, "{column:?}, tick {tick}");
                    assert_eq!(app.scroll_offset, 5, "{column:?}, tick {tick}");
                    let expected: Vec<_> = if ascending {
                        (0..12).collect()
                    } else {
                        (0..12).rev().collect()
                    };
                    let actual: Vec<_> = app
                        .displayed_processes()
                        .map(|p| (p.pid + tick) % 12)
                        .collect();
                    assert_eq!(actual, expected, "{column:?}, tick {tick}");
                }
            }
        }
    }

    /// The comparator each column sorted by before the key sort.
    fn reference_cmp(column: SortColumn, a: &ProcessInfo, b: &ProcessInfo) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        let f = |x: f32, y: f32| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
        match column {
            SortColumn::Cpu => f(a.cpu_percent, b.cpu_percent),
            SortColumn::Mem => f(a.mem_percent, b.mem_percent),
            SortColumn::Pid => a.pid.cmp(&b.pid),
            SortColumn::Res => a.resident_mem.cmp(&b.resident_mem),
            SortColumn::Time => a.cpu_time.cmp(&b.cpu_time),
            SortColumn::PPid => a.parent_pid.cmp(&b.parent_pid),
            SortColumn::User => a.user.cmp(&b.user),
            SortColumn::Priority | SortColumn::PriorityClass => a.priority.cmp(&b.priority),
            SortColumn::Threads => a.thread_count.cmp(&b.thread_count),
            SortColumn::Virt => a.virtual_mem.cmp(&b.virtual_mem),
            SortColumn::Shr => a.shared_mem.cmp(&b.shared_mem),
            SortColumn::Status => a.status.cmp(&b.status),
            SortColumn::StartTime => a.start_time.cmp(&b.start_time),
            SortColumn::Command => a.command.cmp(&b.command),
            SortColumn::Elevated => a.is_elevated.cmp(&b.is_elevated),
            SortColumn::Arch => a.arch.sort_rank().cmp(&b.arch.sort_rank()),
            SortColumn::Efficiency => a.efficiency_mode.cmp(&b.efficiency_mode),
            SortColumn::HandleCount => a.handle_count.cmp(&b.handle_count),
            SortColumn::IoRate => {
                (a.io_read_rate + a.io_write_rate).cmp(&(b.io_read_rate + b.io_write_rate))
            }
            SortColumn::IoReadRate => a.io_read_rate.cmp(&b.io_read_rate),
            SortColumn::IoWriteRate => a.io_write_rate.cmp(&b.io_write_rate),
            SortColumn::IoRead => a.io_read_bytes.cmp(&b.io_read_bytes),
            SortColumn::IoWrite => a.io_write_bytes.cmp(&b.io_write_bytes),
            SortColumn::Gpu => f(a.gpu_percent, b.gpu_percent),
            SortColumn::GpuMem => a.gpu_memory.cmp(&b.gpu_memory),
            SortColumn::Npu => f(a.npu_percent, b.npu_percent),
            SortColumn::NpuMem => a.npu_memory.cmp(&b.npu_memory),
        }
    }

    #[test]
    fn key_sort_matches_the_column_comparators_with_ties_in_collector_order() {
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |modulus: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % modulus
        };
        let arches = [
            ProcessArch::Native,
            ProcessArch::X86,
            ProcessArch::X64,
            ProcessArch::ARM64,
        ];
        // Few distinct values per field, so every column has many ties.
        let processes: Vec<ProcessInfo> = (0..300)
            .map(|i| {
                let mut p = process(1000 + (next(50) as u32) * 7 + i, next(20) as u32);
                p.cpu_percent = [0.0, -0.0, 0.5, 3.25, 99.9, 100.0][next(6) as usize];
                p.mem_percent = next(4) as f32 / 3.0;
                p.gpu_percent = [0.0, 12.5, 0.1][next(3) as usize];
                p.npu_percent = [0.0, 7.0][next(2) as usize];
                p.resident_mem = next(5) << 20;
                p.virtual_mem = next(5) << 30;
                p.shared_mem = next(3) << 12;
                p.cpu_time = next(6) * 10_000_000;
                p.priority = [-15, -1, 0, 4, 8, 13, 24][next(7) as usize];
                p.thread_count = next(4) as u32;
                p.status = b"RSD"[next(3) as usize];
                p.start_time = next(4) as u32 * 60;
                p.is_elevated = next(2) == 1;
                p.efficiency_mode = next(2) == 1;
                p.arch = arches[next(4) as usize];
                p.handle_count = next(5) as u32;
                p.io_read_rate = next(4) * 512;
                p.io_write_rate = next(4) * 512;
                p.io_read_bytes = next(4) << 10;
                p.io_write_bytes = next(4) << 10;
                p.gpu_memory = next(3) << 20;
                p.npu_memory = next(3) << 20;
                p.user = Arc::from(["alice", "bob", "SYSTEM"][next(3) as usize]);
                p.command = Arc::from(["C:\\a.exe", "C:\\b.exe", "b"][next(3) as usize]);
                p
            })
            .collect();

        let mut app = App::new(Config::default());
        app.processes = processes;
        for &column in SortColumn::all() {
            for ascending in [false, true] {
                app.sort_column = column;
                app.sort_ascending = ascending;
                let sort_entry = app.sort_entry();
                let mut entries: Vec<_> = (app.processes.iter().enumerate())
                    .map(|(index, p)| sort_entry(index, p))
                    .collect();
                app.sort_order(&mut entries);
                let order: Vec<usize> = entries.iter().map(|&(_, index)| index).collect();

                // A stable sort keeps ties in collector order.
                let mut expected: Vec<usize> = (0..app.processes.len()).collect();
                expected.sort_by(|&a, &b| {
                    let ord = reference_cmp(column, &app.processes[a], &app.processes[b]);
                    if ascending { ord } else { ord.reverse() }
                });
                assert_eq!(order, expected, "{column:?}, ascending {ascending}");
            }
        }
    }

    #[test]
    fn explicit_follow_tracks_identity_until_disabled() {
        let mut app = App::new(Config::default());
        app.visible_height = 2;
        app.processes = (1..=6)
            .map(|pid| {
                let mut p = process(pid, 0);
                p.cpu_percent = pid as f32;
                p
            })
            .collect();
        app.update_displayed_processes();
        let identity = app.selected_process().unwrap().identity();
        app.toggle_follow_mode();
        app.processes[5].cpu_percent = 0.0;
        app.update_displayed_processes();
        assert_eq!(app.selected_process().unwrap().identity(), identity);
        assert_eq!(app.selected_index, 5);
        assert_eq!(app.scroll_offset, 4);

        app.toggle_follow_mode();
        app.processes[5].cpu_percent = 10.0;
        app.update_displayed_processes();
        assert_eq!(app.selected_index, 5);
        assert_eq!(app.scroll_offset, 4);
        assert_ne!(app.selected_process().unwrap().identity(), identity);
    }

    #[test]
    fn action_keys_capture_displayed_identity_and_keep_it_after_resort_and_pid_reuse() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        for key in [KeyCode::F(9), KeyCode::F(7), KeyCode::Char('a')] {
            let mut app = App::new(Config::default());
            app.config.confirm_kill = true;
            app.processes = vec![process(1, 0), process(2, 0)];
            app.processes[0].cpu_percent = 50.0;
            app.update_displayed_processes();
            // A refresh changes the process under the stationary selection.
            app.processes[1].cpu_percent = 90.0;
            app.update_displayed_processes();
            let target = app.selected_process().unwrap().identity();
            assert_eq!(target.pid, 2);
            crate::input::handle_key_event(&mut app, KeyEvent::new(key, KeyModifiers::NONE));
            let captured = |app: &App| match &app.dialog {
                DialogState::Kill { request } => request.targets()[0].identity,
                DialogState::Priority { identity, .. } | DialogState::Affinity { identity, .. } => {
                    *identity
                }
                _ => panic!("expected an action dialog for {key:?}"),
            };
            assert_eq!(captured(&app), target);
            app.processes[0].cpu_percent = 100.0;
            app.update_displayed_processes();
            assert_eq!(app.selected_process().unwrap().pid, 1);
            assert_eq!(captured(&app), target);
            app.processes[1].create_time_100ns += 1;
            app.update_displayed_processes();
            assert_eq!(captured(&app), target);
            assert!(app.process_by_identity(target).is_none());
        }
    }

    #[test]
    fn selection_clamps_when_the_selected_process_exits() {
        let mut app = App::new(Config::default());
        app.processes = vec![process(1, 0), process(2, 0)];
        app.update_displayed_processes();
        app.selected_index = 1;

        app.processes = vec![process(1, 0)];
        app.update_displayed_processes();
        assert_eq!(app.selected_process().map(|p| p.pid), Some(1));
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
            published_at: Instant::now(),
        });

        let DialogState::UserSelect { index, users } = &app.dialog else {
            panic!("user picker should remain open");
        };
        assert!(users.iter().any(|user| user == "carol"));
        assert_eq!(users.get(*index - 1).map(String::as_str), Some("bob"));
    }

    #[test]
    fn uptime_meter_keeps_cpu_collection_alive() {
        // Issue #97: the Uptime row renders the average CPU%, so hiding only
        // the CPU meters must not freeze it.
        use crate::config::MeterMode;
        use crate::system::collect_gates;

        let mut app = App::new(Config::default());
        app.config.cpu_meter_mode = MeterMode::Hidden;
        assert!(app.config.show_uptime_meter);
        assert_ne!(app.canonical_collect_requirements() & collect_gates::CPU, 0);

        app.config.show_uptime_meter = false;
        assert_eq!(app.canonical_collect_requirements() & collect_gates::CPU, 0);

        app.config.show_uptime_meter = true;
        app.show_header = false;
        assert_eq!(app.canonical_collect_requirements(), 0);
    }

    #[test]
    fn navigation_enriches_rows_scrolled_into_view() {
        // Issue #99: jumping past the enriched window must enrich the new
        // viewport before it is drawn, not at the next snapshot.
        let mut app = App::new(Config::default());
        app.visible_height = 10;
        app.sort_column = SortColumn::Pid;
        app.sort_ascending = true;
        app.processes = (1..=200).map(|pid| process(pid, 0)).collect();
        app.update_displayed_processes();
        assert_eq!(app.enriched_rows, 0..20);

        // Inside the window: no new pass.
        app.select_down();
        app.enrich_viewport();
        assert_eq!(app.enriched_rows, 0..20);

        app.select_last();
        assert_eq!(app.scroll_offset, 190);
        app.enrich_viewport();
        assert_eq!(app.enriched_rows, 180..200);

        app.select_first();
        app.enrich_viewport();
        assert_eq!(app.enriched_rows, 0..20);
    }

    #[test]
    fn uncached_metadata_queries_wait_until_after_the_draw() {
        // Rows new to the view are drawn with cached facts; their Windows
        // queries run once, after the frame (fixture PIDs are never cached).
        let mut app = App::new(Config::default());
        app.visible_height = 5;
        app.processes = (1..=20)
            .map(|pid| process(0x7FF0_0000 + pid * 4, 0))
            .collect();
        app.update_displayed_processes();
        assert!(app.enrichment_pending);
        assert!(app.run_deferred_enrichment());
        assert!(!app.run_deferred_enrichment(), "runs once per pass");
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
        let pids: Vec<u32> = app.displayed_processes().map(|p| p.pid).collect();
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
        assert_eq!(app.displayed_len(), 80);
        assert_eq!(app.display.last().unwrap().tree_depth, 79);
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
            .displayed_processes()
            .map(ProcessInfo::identity)
            .collect();
        assert_eq!(app.displayed_len(), 3);
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

    fn zero_height_app() -> App {
        let mut app = App::new(Config::default());
        // A viewport with no rows: collapsed window, or a layout pass that
        // gave the process table no space (issue #75).
        app.visible_height = 0;
        app.set_display_for_test((1..=10).map(|pid| process(pid * 100, 0)).collect());
        app
    }

    #[test]
    fn zero_visible_height_select_down_keeps_scroll_offset_in_bounds() {
        let mut app = zero_height_app();

        for step in 1..=12 {
            app.select_down();
            assert_eq!(
                app.selected_index,
                step.min(9),
                "selection should stop at the last row"
            );
            assert!(
                app.scroll_offset < app.displayed_len(),
                "scroll_offset {} escaped the list after {} select_down calls",
                app.scroll_offset,
                step
            );
        }

        app.page_down();
        assert!(app.scroll_offset < app.displayed_len());
        app.select_last();
        assert!(app.scroll_offset < app.displayed_len());
    }

    #[test]
    fn update_displayed_processes_with_zero_visible_height_clamps_scroll() {
        let mut app = App::new(Config::default());
        app.processes = (1..=10).map(|pid| process(pid * 100, 0)).collect();
        app.visible_height = 0;

        app.update_displayed_processes();
        assert_ne!(app.displayed_len(), 0);

        // Selection at the last row used to push scroll_offset one past it.
        app.select_last();
        app.update_displayed_processes();
        assert!(app.scroll_offset < app.displayed_len());

        // And a stale offset from a bigger list is clamped, not preserved.
        app.scroll_offset = 50;
        app.update_displayed_processes();
        assert!(app.scroll_offset < app.displayed_len());
    }

    #[test]
    fn non_zero_viewport_scrolling_is_unchanged() {
        let mut app = zero_height_app();
        app.visible_height = 3;
        app.scroll_offset = 0;
        app.selected_index = 0;

        for _ in 0..9 {
            app.select_down();
        }
        assert_eq!(app.selected_index, 9);
        assert_eq!(app.scroll_offset, 7);

        for _ in 0..9 {
            app.select_up();
        }
        assert_eq!(app.scroll_offset, 0);
    }

    /// Reference implementation of the pre-index build_tree algorithm
    /// (pid-keyed HashMaps), used to prove the index-space rewrite emits
    /// identical order and stamps.
    fn reference_build_tree(
        processes: Vec<ProcessInfo>,
        collapsed_pids: &HashSet<ProcessIdentity>,
    ) -> Vec<(u32, u16, String, bool, bool)> {
        use std::collections::HashMap;
        use std::sync::Arc;

        #[derive(Debug)]
        struct PendingNode {
            pid: u32,
            depth: usize,
            is_last: bool,
            parent_prefix: Arc<str>,
        }

        let process_count = processes.len();
        let parents = validated_parents(&processes);
        let order: Vec<u32> = processes.iter().map(|process| process.pid).collect();
        let roots: Vec<u32> = processes
            .iter()
            .filter(|process| !parents.contains_key(&process.pid))
            .map(|process| process.pid)
            .collect();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::with_capacity(process_count / 4);
        let mut nodes: HashMap<u32, ProcessInfo> = HashMap::with_capacity(process_count);
        for process in processes {
            if let Some(parent) = parents.get(&process.pid) {
                children.entry(*parent).or_default().push(process.pid);
            }
            nodes.insert(process.pid, process);
        }

        let mut result = Vec::with_capacity(process_count);
        let mut visited = HashSet::with_capacity(process_count);
        let mut suppressed = HashSet::new();

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
                parent_prefix: Arc::from(""),
            }];
            while let Some(pending) = stack.pop() {
                if !visited.insert(pending.pid) || suppressed.contains(&pending.pid) {
                    continue;
                }
                let Some(process) = nodes.remove(&pending.pid) else {
                    continue;
                };
                let identity = process.identity();
                let child_pids = children.remove(&pending.pid).unwrap_or_default();
                let is_collapsed = collapsed_pids.contains(&identity);

                let tree_prefix = if pending.depth == 0 {
                    String::new()
                } else {
                    let branch = if pending.is_last { "\u{2514}\u{2500} " } else { "\u{251c}\u{2500} " };
                    let mut prefix =
                        String::with_capacity(pending.parent_prefix.len() + branch.len());
                    prefix.push_str(&pending.parent_prefix);
                    prefix.push_str(branch);
                    prefix
                };
                result.push((
                    process.pid,
                    pending.depth.min(u16::MAX as usize) as u16,
                    tree_prefix,
                    !child_pids.is_empty(),
                    is_collapsed,
                ));

                if is_collapsed {
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

                let child_parent_prefix: Arc<str> = if pending.depth == 0 {
                    Arc::from("")
                } else {
                    let continuation = if pending.is_last { "   " } else { "\u{2502}  " };
                    let mut prefix =
                        String::with_capacity(pending.parent_prefix.len() + continuation.len());
                    prefix.push_str(&pending.parent_prefix);
                    prefix.push_str(continuation);
                    Arc::from(prefix)
                };
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

    #[test]
    fn index_build_tree_matches_reference_on_random_forests() {
        // Deterministic LCG so failures reproduce.
        let mut state: u64 = 0x9E3779B97F4A7C15;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for scenario in 0..200 {
            let count = (next() % 80) as u32;
            // Unique pids from a sparse range, shuffled.
            let mut pids: Vec<u32> = (1..=count * 2 + 2).map(|i| i * 3).collect();
            for i in (1..pids.len()).rev() {
                let j = (next() % (i as u64 + 1)) as usize;
                pids.swap(i, j);
            }
            pids.truncate(count as usize);

            let processes: Vec<ProcessInfo> = pids
                .iter()
                .map(|&pid| {
                    let roll = next() % 10;
                    let parent_pid = if roll < 3 {
                        0 // root candidate
                    } else if roll == 4 {
                        pid // self-parent (invalid)
                    } else if roll == 5 {
                        99_999 // missing parent (invalid)
                    } else {
                        pids[(next() % pids.len() as u64) as usize] // cycle-prone
                    };
                    let mut p = process(pid, parent_pid);
                    // Varied create times make some parent links invalid.
                    p.create_time_100ns = 10_000 + (next() % 5) * 1_000 + pid as u64;
                    p
                })
                .collect();

            let mut app = App::new(Config::default());
            app.tree_view = true;
            // Collapse a random subset by identity.
            for p in &processes {
                if next() % 4 == 0 {
                    app.collapsed_pids.insert(p.identity());
                }
            }

            let expected = reference_build_tree(processes.clone(), &app.collapsed_pids);
            // Sorted position i holds processes[order[i]]: a shuffled order
            // checks the tree is built over positions, not raw indices.
            let order: Vec<(u64, usize)> = (0..processes.len()).rev().map(|i| (0, i)).collect();
            app.processes = order.iter().map(|&(_, i)| processes[i].clone()).collect();
            let mut rows = Vec::new();
            app.build_tree(&order, &mut rows);
            let actual: Vec<_> = rows
                .into_iter()
                .map(|row| {
                    (
                        app.processes[row.index].pid,
                        row.tree_depth,
                        row.tree_prefix,
                        row.has_children,
                        row.is_collapsed,
                    )
                })
                .collect();
            assert_eq!(
                actual,
                expected,
                "scenario {scenario} diverged ({} processes)",
                processes.len()
            );
        }
    }
}
