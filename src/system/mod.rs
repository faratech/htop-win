mod cpu;
mod memory;
#[cfg(windows)]
mod native;
mod process;

pub use cpu::CpuInfo;
pub use memory::{format_bytes, MemoryInfo};
pub use process::{
    enable_debug_privilege, enrich_processes, get_process_affinity, get_process_io_counters,
    kill_process, set_priority, set_process_affinity, ProcessInfo,
};
#[cfg(windows)]
pub use native::{query_all_processes, calculate_cpu_percentages, cleanup_cpu_time_cache};

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
    #[cfg(windows)]
    prev_total_cpu_time: u64,
    #[cfg(windows)]
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
            #[cfg(windows)]
            prev_total_cpu_time: 0,
            #[cfg(windows)]
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

#[cfg(not(windows))]
fn get_hostname() -> String {
    "unknown".to_string()
}

/// Network interface statistics
#[cfg(windows)]
struct NetworkStats {
    rx_bytes: u64,
    tx_bytes: u64,
}

/// Get network I/O stats using native Windows IP Helper API
#[cfg(windows)]
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

#[cfg(not(windows))]
fn get_network_stats() -> (u64, u64) {
    (0, 0)
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
        #[cfg(windows)]
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

    /// Get processes using native NtQuerySystemInformation (single syscall, much faster)
    /// Returns processes with CPU percentages calculated from time deltas
    #[cfg(windows)]
    pub fn get_processes_native(&mut self) -> Vec<ProcessInfo> {
        // Query all processes in a single syscall
        let mut native_procs = query_all_processes();

        // Update time tracking for CPU delta calculation
        let now = std::time::Instant::now();
        self.last_native_refresh = now;

        // Calculate total CPU time for all processes
        let total_cpu_time: u64 = native_procs.iter()
            .map(|p| p.kernel_time + p.user_time)
            .sum();

        // Calculate delta (100-nanosecond units)
        let cpu_delta = total_cpu_time.saturating_sub(self.prev_total_cpu_time);
        self.prev_total_cpu_time = total_cpu_time;

        // Get CPU percentages based on time deltas
        let cpu_percentages = calculate_cpu_percentages(&mut native_procs, cpu_delta);

        // Update task counts from native data
        // On Windows, most processes are in "running/ready" state (no real sleep distinction like Linux)
        self.tasks_total = native_procs.len();
        self.tasks_running = self.tasks_total.saturating_sub(1); // Exclude System Idle Process
        self.tasks_sleeping = 1; // Just the System Idle Process
        self.threads_total = native_procs.iter().map(|p| p.thread_count as usize).sum();

        // Update disk I/O from native data
        let total_disk_read: u64 = native_procs.iter().map(|p| p.read_bytes).sum();
        let total_disk_write: u64 = native_procs.iter().map(|p| p.write_bytes).sum();

        self.disk_read_rate = total_disk_read.saturating_sub(self.prev_disk_read);
        self.disk_write_rate = total_disk_write.saturating_sub(self.prev_disk_write);
        self.prev_disk_read = total_disk_read;
        self.prev_disk_write = total_disk_write;
        self.disk_read_bytes = total_disk_read;
        self.disk_write_bytes = total_disk_write;

        // Convert to ProcessInfo using native memory info for total_mem
        let total_mem = MemoryInfo::total_memory();
        ProcessInfo::from_native(&native_procs, &cpu_percentages, total_mem)
    }

    #[cfg(not(windows))]
    pub fn get_processes_native(&mut self) -> Vec<ProcessInfo> {
        Vec::new()
    }
}
