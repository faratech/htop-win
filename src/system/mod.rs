mod cpu;
mod memory;
mod native;
pub mod cache;
mod process;

pub use cpu::CpuInfo;
pub use memory::{format_bytes, MemoryInfo};
pub use process::{
    enable_debug_privilege, enrich_processes, get_process_affinity, get_process_exe_path,
    get_process_io_counters, kill_process, set_efficiency_mode, set_priority_class,
    set_process_affinity, ProcessInfo,
};

/// System metrics
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
    // Previous values for rate calculation
    prev_net_rx: u64,
    prev_net_tx: u64,
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
            prev_net_rx: 0,
            prev_net_tx: 0,
            prev_disk_read: 0,
            prev_disk_write: 0,
            prev_total_cpu_time: 0,
            last_native_refresh: std::time::Instant::now(),
        }
    }
}

/// Get system uptime in seconds using native Windows API
fn get_uptime() -> u64 {
    use windows::Win32::System::SystemInformation::GetTickCount64;
    unsafe { GetTickCount64() / 1000 }
}

/// Get hostname using native Windows API
fn get_hostname() -> String {
    use windows::Win32::System::SystemInformation::{GetComputerNameExW, ComputerNameDnsHostname};
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
        ).is_ok() {
            String::from_utf16_lossy(&buffer[..size as usize])
        } else {
            "unknown".to_string()
        }
    }
}

/// Network interface statistics
struct NetworkStats {
    rx_bytes: u64,
    tx_bytes: u64,
}

/// Get network I/O stats using native Windows IP Helper API
fn get_network_stats() -> NetworkStats {
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::NetworkManagement::IpHelper::{GetIfTable2, FreeMibTable, MIB_IF_TABLE2, IF_TYPE_SOFTWARE_LOOPBACK};
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
            self.net_rx_rate = net_stats.rx_bytes.saturating_sub(self.prev_net_rx);
            self.net_tx_rate = net_stats.tx_bytes.saturating_sub(self.prev_net_tx);
            self.prev_net_rx = net_stats.rx_bytes;
            self.prev_net_tx = net_stats.tx_bytes;
            self.net_rx_bytes = net_stats.rx_bytes;
            self.net_tx_bytes = net_stats.tx_bytes;
        }

        // Update battery status
        self.update_battery();
    }

    fn update_battery(&mut self) {
        // Use Windows API for battery status
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
    }

    /// Update existing processes using native NtQuerySystemInformation
    /// Reuse existing ProcessInfo structs to avoid memory allocation for strings
    pub fn update_processes_native(&mut self, processes: &mut Vec<ProcessInfo>) {
        use std::collections::{HashMap, HashSet};
        use self::native::{with_process_list, calculate_cpu_percentages_from_iter, filetime_to_unix};
        use self::cache::CACHE;

        // Periodically clean up stale PIDs from caches
        if CACHE.should_cleanup() {
            let current_pids: HashSet<u32> = processes.iter().map(|p| p.pid).collect();
            self::process::cleanup_stale_caches(&current_pids);
        }

        with_process_list(|proc_list| {
            // Update time tracking for CPU delta calculation
            let now = std::time::Instant::now();
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

            // Get CPU percentages based on time deltas
            let cpu_percentages = calculate_cpu_percentages_from_iter(&proc_list, cpu_delta);

            // Update global stats
            self.tasks_total = tasks_total;
            self.tasks_running = self.tasks_total.saturating_sub(1); // Exclude System Idle Process
            self.tasks_sleeping = 1;
            self.threads_total = threads_total;

            self.disk_read_rate = total_disk_read.saturating_sub(self.prev_disk_read);
            self.disk_write_rate = total_disk_write.saturating_sub(self.prev_disk_write);
            self.prev_disk_read = total_disk_read;
            self.prev_disk_write = total_disk_write;
            self.disk_read_bytes = total_disk_read;
            self.disk_write_bytes = total_disk_write;

            let total_mem = MemoryInfo::total_memory();

            // Track which processes we've seen in this update
            let mut seen_pids = HashSet::new();

            // Process list is sorted by existing order usually, but new list is by PID
            // We want to update existing entries and collect new ones
            
            // Build a map of existing processes index by PID for fast lookup
            let existing_map: HashMap<u32, usize> = processes
                .iter()
                .enumerate()
                .map(|(i, p)| (p.pid, i))
                .collect();

            let mut new_processes = Vec::new();

            // Iterate raw processes
            for raw_proc in proc_list.iter() {
                let pid = raw_proc.pid();
                seen_pids.insert(pid);
                
                if let Some(&idx) = existing_map.get(&pid) {
                    // Check if it's the same process instance (start time match)
                    let native_start = filetime_to_unix(raw_proc.create_time());
                    let existing_proc = &mut processes[idx];
                    
                    if (native_start as i64 - existing_proc.start_time as i64).abs() <= 1 {
                        // Update existing process
                        let cpu_pct = cpu_percentages.get(&pid).copied().unwrap_or(0.0);
                        existing_proc.update_from_raw(&raw_proc, cpu_pct, total_mem);
                    } else {
                        // PID reuse: replace existing process
                        let cpu_pct = cpu_percentages.get(&pid).copied().unwrap_or(0.0);
                        *existing_proc = ProcessInfo::from_raw(&raw_proc, cpu_pct, total_mem);
                    }
                } else {
                    // New process
                    let cpu_pct = cpu_percentages.get(&pid).copied().unwrap_or(0.0);
                    new_processes.push(ProcessInfo::from_raw(&raw_proc, cpu_pct, total_mem));
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
}
