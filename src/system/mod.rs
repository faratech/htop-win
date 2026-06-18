mod cpu;
#[cfg(windows)]
mod d3dkmt;
#[cfg(not(windows))]
mod d3dkmt {
    use std::collections::HashMap;

    #[derive(Clone, Default)]
    pub struct AdapterMetrics {
        pub name: String,
        pub utilization: f32,
        pub mem_used: u64,
        pub mem_total: u64,
        pub dedicated_used: u64,
        pub dedicated_total: u64,
        pub shared_used: u64,
    }

    impl AdapterMetrics {
        pub fn meter_memory(&self) -> (u64, u64) {
            (self.mem_used, self.mem_total)
        }
    }

    pub type NpuInfo = AdapterMetrics;
    pub type GpuInfo = AdapterMetrics;

    #[derive(Clone, Default)]
    pub struct AdapterSnapshot {
        pub gpu: Option<GpuInfo>,
        pub npu: Option<NpuInfo>,
    }

    #[derive(Clone, Copy, Default)]
    pub struct ProcAdapterStats {
        pub gpu_percent: f32,
        pub gpu_memory: u64,
        pub npu_percent: f32,
        pub npu_memory: u64,
    }

    pub fn set_gpu_process_stats_enabled(_enabled: bool) {}
    pub fn set_npu_process_stats_enabled(_enabled: bool) {}
    pub fn set_gpu_selection(_name: Option<String>) {}
    pub fn gpu_names() -> Vec<String> {
        Vec::new()
    }
    pub fn process_stats_enabled() -> bool {
        false
    }
    pub fn refresh() -> AdapterSnapshot {
        AdapterSnapshot::default()
    }
    pub fn process_stats(_processes: &[(u32, u64)]) -> HashMap<u32, ProcAdapterStats> {
        HashMap::new()
    }
    pub fn debug_dump() -> String {
        "D3DKMT adapter metrics are only available on Windows".to_string()
    }
}
pub mod cache;
mod memory;
#[cfg(windows)]
mod native;
mod process;

pub use cpu::{CpuInfo, debug_dump as cpu_debug_dump};
pub use d3dkmt::{
    GpuInfo, NpuInfo, debug_dump as gpu_debug_dump, gpu_names, set_gpu_process_stats_enabled,
    set_gpu_selection, set_npu_process_stats_enabled,
};
pub use memory::{MemoryInfo, format_bytes};
pub use process::{
    ProcessArch, ProcessInfo, enable_debug_privilege, enrich_processes, get_process_affinity,
    get_process_exe_path, get_process_io_counters, kill_process, set_efficiency_mode,
    set_priority_class, set_process_affinity,
};

/// System metrics
#[derive(Clone)]
pub struct SystemMetrics {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub uptime: u64,
    pub hostname: String,
    pub tasks_total: usize,
    pub tasks_running: usize,
    pub tasks_sleeping: usize,
    pub threads_total: usize,
    // Network I/O
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub net_rx_rate: u64,
    pub net_tx_rate: u64,
    // Disk I/O
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub disk_read_rate: u64,
    pub disk_write_rate: u64,
    // Battery
    pub battery_percent: Option<f32>,
    pub battery_charging: bool,
    // GPU (None when no render-capable hardware adapter exists)
    pub gpu: Option<GpuInfo>,
    // NPU (None when no MCDM compute-only adapter exists)
    pub npu: Option<NpuInfo>,
    // Previous values for rate calculation
    prev_net_rx: u64,
    prev_net_tx: u64,
    prev_net_sample: std::time::Instant,
    prev_disk_read: u64,
    prev_disk_write: u64,
    // Native process enumeration state
    prev_total_cpu_time: u64,
    last_native_refresh: std::time::Instant,
}

impl Default for SystemMetrics {
    fn default() -> Self {
        Self {
            cpu: CpuInfo::default(),
            memory: MemoryInfo::default(),
            uptime: 0,
            hostname: String::new(),
            tasks_total: 0,
            tasks_running: 0,
            tasks_sleeping: 0,
            threads_total: 0,
            net_rx_bytes: 0,
            net_tx_bytes: 0,
            net_rx_rate: 0,
            net_tx_rate: 0,
            disk_read_bytes: 0,
            disk_write_bytes: 0,
            disk_read_rate: 0,
            disk_write_rate: 0,
            battery_percent: None,
            battery_charging: false,
            gpu: None,
            npu: None,
            prev_net_rx: 0,
            prev_net_tx: 0,
            prev_net_sample: std::time::Instant::now(),
            prev_disk_read: 0,
            prev_disk_write: 0,
            prev_total_cpu_time: 0,
            last_native_refresh: std::time::Instant::now(),
        }
    }
}

/// Get system uptime in seconds using native Windows API
#[cfg(windows)]
fn get_uptime() -> u64 {
    use windows::Win32::System::SystemInformation::GetTickCount64;
    unsafe { GetTickCount64() / 1000 }
}

#[cfg(not(windows))]
fn get_uptime() -> u64 {
    0
}

/// Get hostname using native Windows API
#[cfg(windows)]
fn get_hostname() -> String {
    use windows::Win32::System::SystemInformation::{ComputerNameDnsHostname, GetComputerNameExW};
    use windows::core::PWSTR;

    let mut size: u32 = 0;
    // First call to get required buffer size
    unsafe {
        let _ = GetComputerNameExW(ComputerNameDnsHostname, None, &mut size);
    }

    if size == 0 {
        return "unknown".to_string();
    }

    let mut buffer: Vec<u16> = vec![0; size as usize];
    unsafe {
        if GetComputerNameExW(
            ComputerNameDnsHostname,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut size,
        )
        .is_ok()
        {
            String::from_utf16_lossy(&buffer[..size as usize])
        } else {
            "unknown".to_string()
        }
    }
}

#[cfg(not(windows))]
fn get_hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Network interface statistics
struct NetworkStats {
    rx_bytes: u64,
    tx_bytes: u64,
}

#[inline]
fn bytes_per_second(delta: u64, elapsed_secs: f64) -> u64 {
    if elapsed_secs <= 0.0 {
        0
    } else {
        (delta as f64 / elapsed_secs) as u64
    }
}

/// Get network I/O stats using native Windows IP Helper API
#[cfg(windows)]
fn get_network_stats() -> NetworkStats {
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, IF_TYPE_SOFTWARE_LOOPBACK, MIB_IF_TABLE2,
    };
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

    let mut total_rx: u64 = 0;
    let mut total_tx: u64 = 0;

    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table) == WIN32_ERROR(0) && !table.is_null() {
            let num_entries = (*table).NumEntries as usize;
            let entries = std::slice::from_raw_parts((*table).Table.as_ptr(), num_entries);

            for entry in entries {
                // Skip loopback and non-operational interfaces
                if entry.OperStatus == IfOperStatusUp && entry.Type != IF_TYPE_SOFTWARE_LOOPBACK {
                    total_rx += entry.InOctets;
                    total_tx += entry.OutOctets;
                }
            }

            FreeMibTable(table as *const _);
        }
    }

    NetworkStats {
        rx_bytes: total_rx,
        tx_bytes: total_tx,
    }
}

#[cfg(not(windows))]
fn get_network_stats() -> NetworkStats {
    NetworkStats {
        rx_bytes: 0,
        tx_bytes: 0,
    }
}

impl SystemMetrics {
    /// Refresh system metrics (CPU, memory, uptime, hostname, battery, network)
    /// Does NOT refresh processes - use get_processes_native() for that
    pub fn refresh(&mut self) {
        // Update CPU info using native API
        self.cpu = CpuInfo::from_native();

        // Update memory info using native API
        self.memory = MemoryInfo::from_native();

        // Update uptime
        self.uptime = get_uptime();

        // Hostname rarely changes – compute once to avoid repeated allocations
        if self.hostname.is_empty() {
            self.hostname = get_hostname();
        }

        // Update network I/O using native API
        {
            let net_stats = get_network_stats();
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(self.prev_net_sample).as_secs_f64();
            self.net_rx_rate =
                bytes_per_second(net_stats.rx_bytes.saturating_sub(self.prev_net_rx), elapsed);
            self.net_tx_rate =
                bytes_per_second(net_stats.tx_bytes.saturating_sub(self.prev_net_tx), elapsed);
            self.prev_net_sample = now;
            self.prev_net_rx = net_stats.rx_bytes;
            self.prev_net_tx = net_stats.tx_bytes;
            self.net_rx_bytes = net_stats.rx_bytes;
            self.net_tx_bytes = net_stats.tx_bytes;
        }

        // Update battery status
        self.update_battery();

        // Update GPU/NPU metrics (no-op on machines without tracked adapters)
        let adapters = d3dkmt::refresh();
        self.gpu = adapters.gpu;
        self.npu = adapters.npu;
    }

    fn update_battery(&mut self) {
        // Use Windows API for battery status
        #[cfg(windows)]
        {
            use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
            let mut status = SYSTEM_POWER_STATUS::default();
            unsafe {
                if GetSystemPowerStatus(&mut status).is_ok() {
                    if status.BatteryLifePercent <= 100 {
                        self.battery_percent = Some(status.BatteryLifePercent as f32);
                    } else {
                        self.battery_percent = None; // No battery or unknown
                    }
                    // AC power connected means charging or full
                    self.battery_charging = status.ACLineStatus == 1;
                }
            }
        }
        #[cfg(not(windows))]
        {
            self.battery_percent = None;
            self.battery_charging = false;
        }
    }

    /// Update existing processes using native NtQuerySystemInformation
    /// Reuse existing ProcessInfo structs to avoid memory allocation for strings
    #[cfg(windows)]
    pub fn update_processes_native(&mut self, processes: &mut Vec<ProcessInfo>) {
        use self::cache::CACHE;
        use self::native::{calculate_process_rates, with_process_list};
        use std::collections::{HashMap, HashSet};

        // Periodically clean up stale PIDs from caches
        if CACHE.should_cleanup() {
            let current_pids: HashSet<u32> = processes.iter().map(|p| p.pid).collect();
            self::process::cleanup_stale_caches(&current_pids);
        }

        // On query failure (None), keep the previous process list and baselines
        // untouched rather than blanking the table for a frame.
        let _ = with_process_list(|proc_list| {
            // Update time tracking for CPU delta calculation
            let now = std::time::Instant::now();
            let disk_elapsed = now.duration_since(self.last_native_refresh).as_secs_f64();
            self.last_native_refresh = now;

            // First pass: Calculate totals and CPU percentages
            let mut total_cpu_time: u64 = 0;
            let mut tasks_total = 0;
            let mut threads_total = 0;
            let mut total_disk_read: u64 = 0;
            let mut total_disk_write: u64 = 0;

            for proc in proc_list.iter() {
                total_cpu_time += proc.kernel_time() + proc.user_time();
                tasks_total += 1;
                threads_total += proc.thread_count() as usize;
                total_disk_read += proc.read_bytes();
                total_disk_write += proc.write_bytes();
            }

            // Calculate delta (100-nanosecond units)
            let cpu_delta = total_cpu_time.saturating_sub(self.prev_total_cpu_time);
            self.prev_total_cpu_time = total_cpu_time;

            // Get CPU percentages and I/O rates based on cache deltas
            let rates = calculate_process_rates(&proc_list, cpu_delta);

            // Update global stats
            self.tasks_total = tasks_total;
            // Windows doesn't expose per-process running/sleeping state like Linux.
            // Exclude System Idle Process (PID 0) from task count; don't fabricate sleep counts.
            self.tasks_running = 0;
            self.tasks_sleeping = 0;
            self.threads_total = threads_total;

            self.disk_read_rate = bytes_per_second(
                total_disk_read.saturating_sub(self.prev_disk_read),
                disk_elapsed,
            );
            self.disk_write_rate = bytes_per_second(
                total_disk_write.saturating_sub(self.prev_disk_write),
                disk_elapsed,
            );
            self.prev_disk_read = total_disk_read;
            self.prev_disk_write = total_disk_write;
            self.disk_read_bytes = total_disk_read;
            self.disk_write_bytes = total_disk_write;

            let total_mem = MemoryInfo::total_memory();

            // Track which processes we've seen in this update
            let mut seen_pids = HashSet::with_capacity(processes.len());

            // Build a map of existing processes index by PID for fast lookup
            let mut existing_map: HashMap<u32, usize> = HashMap::with_capacity(processes.len());
            for (i, p) in processes.iter().enumerate() {
                existing_map.insert(p.pid, i);
            }

            let mut new_processes = Vec::new();

            // Per-process GPU/NPU stats (empty unless the hardware exists and
            // one of its columns is currently visible or sorted). Skip allocating
            // the all-PIDs vector on the common path where stats are disabled;
            // still call process_stats(&[]) so its gate-change bookkeeping runs.
            let adapter_stats = if d3dkmt::process_stats_enabled() {
                let pids: Vec<(u32, u64)> = proc_list
                    .iter()
                    .map(|p| (p.pid(), p.create_time()))
                    .collect();
                d3dkmt::process_stats(&pids)
            } else {
                d3dkmt::process_stats(&[])
            };

            // Iterate raw processes
            for raw_proc in proc_list.iter() {
                let pid = raw_proc.pid();
                seen_pids.insert(pid);
                let cpu_pct = rates.cpu_percentages.get(&pid).copied().unwrap_or(0.0);
                let (io_read_rate, io_write_rate) =
                    rates.io_rates.get(&pid).copied().unwrap_or((0, 0));
                let proc_adapter = adapter_stats.get(&pid).copied().unwrap_or_default();

                if let Some(&idx) = existing_map.get(&pid) {
                    let native_start = raw_proc.create_time();
                    let existing_proc = &mut processes[idx];

                    if native_start == existing_proc.create_time_100ns {
                        // Update existing process (reuses string allocations)
                        existing_proc.update_from_raw(&raw_proc, cpu_pct, total_mem);
                    } else {
                        // PID reuse: replace entirely
                        *existing_proc = ProcessInfo::from_raw(&raw_proc, cpu_pct, total_mem);
                    }
                    existing_proc.io_read_rate = io_read_rate;
                    existing_proc.io_write_rate = io_write_rate;
                    existing_proc.gpu_percent = proc_adapter.gpu_percent;
                    existing_proc.gpu_memory = proc_adapter.gpu_memory;
                    existing_proc.npu_percent = proc_adapter.npu_percent;
                    existing_proc.npu_memory = proc_adapter.npu_memory;
                } else {
                    let mut proc_info = ProcessInfo::from_raw(&raw_proc, cpu_pct, total_mem);
                    proc_info.io_read_rate = io_read_rate;
                    proc_info.io_write_rate = io_write_rate;
                    proc_info.gpu_percent = proc_adapter.gpu_percent;
                    proc_info.gpu_memory = proc_adapter.gpu_memory;
                    proc_info.npu_percent = proc_adapter.npu_percent;
                    proc_info.npu_memory = proc_adapter.npu_memory;
                    new_processes.push(proc_info);
                }
            }

            // Remove dead processes
            processes.retain(|p| seen_pids.contains(&p.pid));

            // Append new processes
            if !new_processes.is_empty() {
                processes.append(&mut new_processes);
            }
        });
    }

    #[cfg(not(windows))]
    pub fn update_processes_native(&mut self, processes: &mut Vec<ProcessInfo>) {
        processes.clear();
        self.tasks_total = 0;
        self.tasks_running = 0;
        self.tasks_sleeping = 0;
        self.threads_total = 0;
        self.disk_read_bytes = 0;
        self.disk_write_bytes = 0;
        self.disk_read_rate = 0;
        self.disk_write_rate = 0;
    }
}
