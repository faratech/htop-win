use crate::config::Config;
use crate::system::{ProcessInfo, SystemMetrics};
use crate::ui::colors::Theme;
use std::collections::{HashSet, VecDeque};
use std::time::Instant;

/// Sort column for process list
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Pid,
    PPid,
    User,
    Priority,
    Nice,
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
}

impl SortColumn {
    pub fn all() -> &'static [SortColumn] {
        &[
            SortColumn::Pid,
            SortColumn::PPid,
            SortColumn::User,
            SortColumn::Priority,
            SortColumn::Nice,
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
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            SortColumn::Pid => "PID",
            SortColumn::PPid => "PPID",
            SortColumn::User => "USER",
            SortColumn::Priority => "PRI",
            SortColumn::Nice => "NI",
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
        }
    }
}

/// Current view mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ViewMode {
    Normal,
    Help,
    Search,
    Filter,
    SortSelect,
    Kill,
    SignalSelect,  // Select signal for kill
    Nice,
    Setup,
    ProcessInfo,
    UserSelect,    // Select user to filter by
    Environment,   // View process environment variables
    ColorScheme,   // Select color scheme
    CommandWrap,   // View wrapped command line
    ColumnConfig,  // Configure visible columns
    Affinity,      // Set CPU affinity
}

/// Application state
pub struct App {
    /// Application configuration
    pub config: Config,
    /// Current color theme (derived from config)
    pub theme: Theme,
    /// Current view mode
    pub view_mode: ViewMode,
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
    /// Input buffer for dialogs
    pub input_buffer: String,
    /// Cursor position in input buffer
    pub input_cursor: usize,
    /// Selected sort column index (for sort select dialog)
    pub sort_select_index: usize,
    /// Process list visible height (set during render)
    pub visible_height: usize,
    /// Help scroll offset
    pub help_scroll: usize,
    /// Setup menu selected item
    pub setup_selected: usize,
    /// Nice value for nice dialog
    pub nice_value: i32,
    /// Last error message
    pub last_error: Option<String>,
    /// Kill target (captured when entering Kill mode to prevent race conditions)
    pub kill_target: Option<(u32, String, String)>,  // (pid, name, command)
    /// Process info target (captured when entering ProcessInfo mode)
    pub process_info_target: Option<crate::system::ProcessInfo>,

    // New fields for additional features
    /// Collapsed PIDs in tree view
    pub collapsed_pids: HashSet<u32>,
    /// Follow mode: PID to follow across refreshes
    pub follow_pid: Option<u32>,
    /// Pause updates
    pub paused: bool,
    /// Selected signal index for kill dialog
    pub signal_select_index: usize,
    /// Selected user index for user filter dialog
    pub user_select_index: usize,
    /// List of unique users (populated for user select dialog)
    pub user_list: Vec<String>,
    /// Color scheme select index
    pub color_scheme_index: usize,
    /// Environment variables scroll offset
    pub env_scroll: usize,
    /// PID search buffer (for incremental PID search with digits)
    pub pid_search_buffer: String,
    /// Last PID search time (for timeout)
    pub pid_search_time: Option<Instant>,
    /// Show header meters
    pub show_header: bool,
    /// Command wrap scroll offset
    pub command_wrap_scroll: usize,
    /// Maximum iterations before exit (for -n option)
    pub max_iterations: Option<u64>,
    /// Current iteration count
    pub iteration_count: u64,
    /// Column config scroll/selection
    pub column_config_index: usize,
    /// CPU affinity mask for affinity dialog
    pub affinity_mask: u64,
    /// Selected CPU in affinity dialog
    pub affinity_selected: usize,
    /// CPU usage history for graph mode (per core, last N samples)
    pub cpu_history: Vec<VecDeque<f32>>,
    /// Memory usage history for graph mode (last N samples)
    pub mem_history: VecDeque<f32>,
    /// Cached visible columns (updated when column config changes)
    pub cached_visible_columns: Vec<SortColumn>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let theme = config.theme();
        let tree_view = config.tree_view_default;
        let visible_columns = Self::compute_visible_columns(&config);
        Self {
            config,
            theme,
            view_mode: ViewMode::Normal,
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
            input_buffer: String::new(),
            input_cursor: 0,
            sort_select_index: 0,
            visible_height: 20,
            help_scroll: 0,
            setup_selected: 0,
            nice_value: 0,
            last_error: None,
            kill_target: None,
            process_info_target: None,
            // New fields
            collapsed_pids: HashSet::new(),
            follow_pid: None,
            paused: false,
            signal_select_index: 0,
            user_select_index: 0,
            user_list: Vec::new(),
            color_scheme_index: 0,
            env_scroll: 0,
            pid_search_buffer: String::new(),
            pid_search_time: None,
            show_header: true,
            command_wrap_scroll: 0,
            max_iterations: None,
            iteration_count: 0,
            column_config_index: 0,
            affinity_mask: 0,
            affinity_selected: 0,
            cpu_history: Vec::new(),
            mem_history: VecDeque::new(),
            cached_visible_columns: visible_columns,
        }
    }

    /// Compute visible columns based on config (used for caching)
    fn compute_visible_columns(config: &Config) -> Vec<SortColumn> {
        SortColumn::all()
            .iter()
            .filter(|col| config.is_column_visible(col.name()))
            .copied()
            .collect()
    }

    /// Update the cached visible columns (call when column config changes)
    pub fn update_visible_columns_cache(&mut self) {
        self.cached_visible_columns = Self::compute_visible_columns(&self.config);
    }

    /// Update the color theme from config
    pub fn update_theme(&mut self) {
        self.theme = self.config.theme();
    }

    /// Save the current configuration
    pub fn save_config(&self) {
        if let Err(e) = self.config.save() {
            eprintln!("Failed to save config: {}", e);
        }
    }

    /// Enter kill mode and capture the target process
    pub fn enter_kill_mode(&mut self) {
        if let Some(proc) = self.selected_process() {
            self.kill_target = Some((proc.pid, proc.name.clone(), proc.command.clone()));
            self.view_mode = ViewMode::Kill;
        }
    }

    /// Enter process info mode and capture the target process
    pub fn enter_process_info_mode(&mut self) {
        if let Some(proc) = self.selected_process() {
            let mut proc_copy = proc.clone();
            // Query I/O counters on-demand (skipped during normal refresh for performance)
            let (io_read, io_write) = crate::system::get_process_io_counters(proc.pid);
            proc_copy.io_read_bytes = io_read;
            proc_copy.io_write_bytes = io_write;
            self.process_info_target = Some(proc_copy);
            self.view_mode = ViewMode::ProcessInfo;
        }
    }

    /// Refresh system data
    pub fn refresh_system(&mut self) {
        // Use native Windows APIs for all system metrics
        self.system_metrics.refresh();
        self.processes = self.system_metrics.get_processes_native();
        self.update_displayed_processes();

        // Update history for graph mode
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
    }

    /// Update displayed processes based on filter and sort
    pub fn update_displayed_processes(&mut self) {
        // Use cached lowercase filter string
        let has_filter = !self.filter_string_lower.is_empty();
        let has_search = !self.search_string_lower.is_empty();

        // Filter-then-clone: only clone processes that pass all filters
        // Also set matches_search flag during this pass to avoid recomputing in render
        let show_kernel = self.config.show_kernel_threads;
        let show_user = self.config.show_user_threads;

        let mut processes: Vec<ProcessInfo> = self.processes
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
                if let Some(ref pids) = self.pid_filter {
                    if !pids.contains(&p.pid) {
                        return false;
                    }
                }
                // User filter
                if let Some(ref user) = self.user_filter {
                    if &p.user != user {
                        return false;
                    }
                }
                // Text filter - use pre-computed lowercase strings
                if has_filter {
                    if !(p.name_lower.contains(&self.filter_string_lower)
                        || p.command_lower.contains(&self.filter_string_lower)
                        || p.pid.to_string().contains(&self.filter_string_lower)
                        || p.user_lower.contains(&self.filter_string_lower))
                    {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

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
        if let Some(follow_pid) = self.follow_pid {
            if let Some(idx) = self.displayed_processes.iter().position(|p| p.pid == follow_pid) {
                self.selected_index = idx;
                self.ensure_visible();
            }
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
                        SortColumn::Nice => a.nice.cmp(&b.nice),
                        SortColumn::Threads => a.thread_count.cmp(&b.thread_count),
                        SortColumn::Virt => a.virtual_mem.cmp(&b.virtual_mem),
                        SortColumn::Shr => a.shared_mem.cmp(&b.shared_mem),
                        SortColumn::Status => a.status.cmp(&b.status),
                        SortColumn::StartTime => a.start_time.cmp(&b.start_time),
                        SortColumn::Command => a.command.cmp(&b.command),
                        SortColumn::Elevated => a.is_elevated.cmp(&b.is_elevated),
                        SortColumn::Arch => a.arch.as_str().cmp(b.arch.as_str()),
                        SortColumn::Efficiency => a.efficiency_mode.cmp(&b.efficiency_mode),
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
        let mut children_map: HashMap<u32, Vec<ProcessInfo>> = HashMap::new();
        let mut root_processes: Vec<ProcessInfo> = Vec::new();

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
            self.add_tree_node(&mut result, root, &children_map, 0, is_last, String::new());
        }

        result
    }

    fn add_tree_node(
        &self,
        result: &mut Vec<ProcessInfo>,
        mut process: ProcessInfo,
        children_map: &std::collections::HashMap<u32, Vec<ProcessInfo>>,
        depth: usize,
        is_last: bool,
        parent_prefix: String,
    ) {
        process.tree_depth = depth;
        let pid = process.pid;
        let has_children = children_map.contains_key(&pid);
        let is_collapsed = self.collapsed_pids.contains(&pid);
        process.has_children = has_children;
        process.is_collapsed = is_collapsed;

        // Build the tree prefix for display
        if depth > 0 {
            let branch = if is_last { "└─ " } else { "├─ " };
            process.tree_prefix = format!("{}{}", parent_prefix, branch);
        } else {
            process.tree_prefix = String::new();
        }

        result.push(process);

        // Only add children if not collapsed
        if !is_collapsed {
            if let Some(children) = children_map.get(&pid) {
                let mut sorted_children = children.clone();
                sorted_children.sort_by(|a, b| a.pid.cmp(&b.pid));
                let child_count = sorted_children.len();

                // Calculate the prefix for children
                let child_parent_prefix = if depth > 0 {
                    let connector = if is_last { "   " } else { "│  " };
                    format!("{}{}", parent_prefix, connector)
                } else {
                    String::new()
                };

                for (idx, child) in sorted_children.into_iter().enumerate() {
                    let child_is_last = idx == child_count - 1;
                    self.add_tree_node(result, child, children_map, depth + 1, child_is_last, child_parent_prefix.clone());
                }
            }
        }
    }

    /// Collapse tree branch at selected process
    pub fn collapse_tree(&mut self) {
        let pid = self.selected_process().map(|p| p.pid);
        if let Some(pid) = pid {
            self.collapsed_pids.insert(pid);
            self.update_displayed_processes();
        }
    }

    /// Expand tree branch at selected process
    pub fn expand_tree(&mut self) {
        let pid = self.selected_process().map(|p| p.pid);
        if let Some(pid) = pid {
            self.collapsed_pids.remove(&pid);
            self.update_displayed_processes();
        }
    }

    /// Collapse all tree branches
    pub fn collapse_all(&mut self) {
        // Collapse all processes that have children
        for proc in &self.processes {
            self.collapsed_pids.insert(proc.pid);
        }
        self.update_displayed_processes();
    }

    /// Expand all tree branches
    pub fn expand_all(&mut self) {
        self.collapsed_pids.clear();
        self.update_displayed_processes();
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

    /// Get selected process
    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        self.displayed_processes.get(self.selected_index)
    }

    /// Toggle tree view
    pub fn toggle_tree_view(&mut self) {
        self.tree_view = !self.tree_view;
        self.update_displayed_processes();
    }

    /// Set sort column
    pub fn set_sort_column(&mut self, column: SortColumn) {
        if self.sort_column == column {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_column = column;
            self.sort_ascending = false;
        }
        self.update_displayed_processes();
    }

    /// Apply filter from input buffer
    pub fn apply_filter(&mut self) {
        self.filter_string = self.input_buffer.clone();
        self.filter_string_lower = self.filter_string.to_lowercase();
        self.update_displayed_processes();
    }

    /// Apply search from input buffer
    pub fn apply_search(&mut self) {
        self.search_string = self.input_buffer.clone();
        self.search_string_lower = self.search_string.to_lowercase();
        // Find first matching process using pre-computed lowercase strings
        if !self.search_string_lower.is_empty() {
            if let Some(idx) = self.displayed_processes.iter().position(|p| {
                p.name_lower.contains(&self.search_string_lower)
                    || p.command_lower.contains(&self.search_string_lower)
            }) {
                self.selected_index = idx;
                self.ensure_visible();
            }
        }
        // Update matches_search flags for highlighting
        self.update_displayed_processes();
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
        if let Some((pid, _, _)) = self.kill_target {
            if let Err(e) = crate::system::kill_process(pid, signal) {
                self.last_error = Some(format!("Failed to kill process {}: {}", pid, e));
            }
        }
    }

    /// Kill all tagged processes
    pub fn kill_tagged(&mut self, signal: u32) {
        let pids: Vec<u32> = self.tagged_pids.iter().copied().collect();
        for pid in pids {
            if let Err(e) = crate::system::kill_process(pid, signal) {
                self.last_error = Some(format!("Failed to kill process {}: {}", pid, e));
            }
        }
        self.tagged_pids.clear();
    }

    /// Set nice value for selected process
    pub fn set_nice_selected(&mut self, nice: i32) {
        if let Some(proc) = self.selected_process() {
            let pid = proc.pid;
            if let Err(e) = crate::system::set_priority(pid, nice) {
                self.last_error = Some(format!("Failed to set priority for {}: {}", pid, e));
            }
        }
    }

    /// Clear error message
    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    /// Add character to input buffer
    pub fn input_char(&mut self, c: char) {
        self.input_buffer.insert(self.input_cursor, c);
        self.input_cursor += 1;
    }

    /// Delete character before cursor
    pub fn input_backspace(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
            self.input_buffer.remove(self.input_cursor);
        }
    }

    /// Delete character at cursor
    pub fn input_delete(&mut self) {
        if self.input_cursor < self.input_buffer.len() {
            self.input_buffer.remove(self.input_cursor);
        }
    }

    /// Move cursor left
    pub fn input_left(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
        }
    }

    /// Move cursor right
    pub fn input_right(&mut self) {
        if self.input_cursor < self.input_buffer.len() {
            self.input_cursor += 1;
        }
    }

    /// Clear input buffer
    pub fn input_clear(&mut self) {
        self.input_buffer.clear();
        self.input_cursor = 0;
    }

    /// Start search mode
    pub fn start_search(&mut self) {
        self.view_mode = ViewMode::Search;
        self.input_buffer = self.search_string.clone();
        self.input_cursor = self.input_buffer.len();
    }

    /// Start filter mode
    pub fn start_filter(&mut self) {
        self.view_mode = ViewMode::Filter;
        self.input_buffer = self.filter_string.clone();
        self.input_cursor = self.input_buffer.len();
    }

    /// Exit current mode
    pub fn exit_mode(&mut self) {
        self.view_mode = ViewMode::Normal;
        self.input_clear();
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

    /// Enter user select mode
    pub fn enter_user_select_mode(&mut self) {
        // Build unique user list
        let mut users: Vec<String> = self.processes
            .iter()
            .map(|p| p.user.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        users.sort();
        self.user_list = users;

        // Set current selection to current filter
        self.user_select_index = if let Some(ref filter) = self.user_filter {
            self.user_list.iter().position(|u| u == filter).map(|i| i + 1).unwrap_or(0)
        } else {
            0 // "All users"
        };

        self.view_mode = ViewMode::UserSelect;
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
            self.env_scroll = 0;
            self.view_mode = ViewMode::Environment;
        }
    }

    /// Enter command wrap view mode
    pub fn enter_command_wrap_mode(&mut self) {
        if self.selected_process().is_some() {
            self.command_wrap_scroll = 0;
            self.view_mode = ViewMode::CommandWrap;
        }
    }

    /// Enter CPU affinity mode
    pub fn enter_affinity_mode(&mut self) {
        if let Some(proc) = self.selected_process() {
            // Get current affinity or default to all CPUs
            let cpu_count = self.system_metrics.cpu.core_usage.len();
            self.affinity_mask = crate::system::get_process_affinity(proc.pid)
                .unwrap_or((1u64 << cpu_count) - 1);
            self.affinity_selected = 0;
            self.view_mode = ViewMode::Affinity;
        }
    }

    /// Apply CPU affinity to selected process
    pub fn apply_affinity(&mut self) {
        if let Some(proc) = self.selected_process() {
            if self.affinity_mask == 0 {
                self.last_error = Some("Cannot set empty affinity mask".to_string());
                return;
            }
            if let Err(e) = crate::system::set_process_affinity(proc.pid, self.affinity_mask) {
                self.last_error = Some(format!("Failed to set affinity: {}", e));
            }
        }
    }

    /// Handle digit key for PID search
    pub fn handle_pid_digit(&mut self, digit: char) {
        use std::time::Duration;

        let now = Instant::now();

        // Clear buffer if too much time has passed (1 second timeout)
        if let Some(last_time) = self.pid_search_time {
            if now.duration_since(last_time) > Duration::from_secs(1) {
                self.pid_search_buffer.clear();
            }
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
                    self.update_displayed_processes();
                    break;
                }
            }
        }
    }

    /// Enter column configuration mode
    pub fn enter_column_config_mode(&mut self) {
        self.column_config_index = 0;
        self.view_mode = ViewMode::ColumnConfig;
    }
}
