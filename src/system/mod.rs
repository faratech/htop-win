mod cpu;
mod memory;
mod process;

pub use cpu::CpuInfo;
pub use memory::{format_bytes, MemoryInfo};
pub use process::{
    get_process_affinity, kill_process, set_priority, set_process_affinity, ProcessInfo,
};

use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Networks, ProcessRefreshKind, RefreshKind, System,
    UpdateKind,
};

/// System metrics
pub struct SystemMetrics {
    sys: System,
    networks: Networks,
    networks_initialized: bool,
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
}

/// Create optimized ProcessRefreshKind - only what we actually need
fn process_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cpu()                                    // CPU usage
        .with_memory()                                 // Memory usage
        .with_disk_usage()                             // Disk I/O
        .with_user(UpdateKind::OnlyIfNotSet)           // User (cached after first lookup)
        .with_exe(UpdateKind::OnlyIfNotSet)            // Exe path (doesn't change)
        .with_cmd(UpdateKind::OnlyIfNotSet)            // Command line (doesn't change)
}

impl Default for SystemMetrics {
    fn default() -> Self {
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything())
                .with_processes(process_refresh_kind()),
        );
        sys.refresh_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything())
                .with_processes(process_refresh_kind()),
        );

        Self {
            sys,
            networks: Networks::new(),
            networks_initialized: false,
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
        }
    }
}

impl SystemMetrics {
    pub fn refresh(&mut self) {
        self.sys.refresh_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything())
                .with_processes(process_refresh_kind()),
        );

        // Refresh existing network interfaces without re-scanning the system each tick
        if self.networks_initialized {
            self.networks.refresh(false);
        } else {
            self.networks.refresh(true);
            self.networks_initialized = true;
        }

        // Update CPU info
        self.cpu = CpuInfo::from_sysinfo(&self.sys);

        // Update memory info
        self.memory = MemoryInfo::from_sysinfo(&self.sys);

        // Update uptime
        self.uptime = System::uptime();

        // Hostname rarely changes – compute once to avoid repeated allocations
        if self.hostname.is_empty() {
            self.hostname = System::host_name().unwrap_or_else(|| "unknown".to_string());
        }

        // Count tasks
        self.tasks_total = self.sys.processes().len();
        self.tasks_running = 0;
        self.tasks_sleeping = 0;
        self.threads_total = 0;

        // Aggregate disk I/O from all processes
        let mut total_disk_read: u64 = 0;
        let mut total_disk_write: u64 = 0;

        for proc in self.sys.processes().values() {
            match proc.status() {
                sysinfo::ProcessStatus::Run => self.tasks_running += 1,
                _ => self.tasks_sleeping += 1,
            }
            self.threads_total += 1;

            // Aggregate disk I/O
            let disk_usage = proc.disk_usage();
            total_disk_read += disk_usage.read_bytes;
            total_disk_write += disk_usage.written_bytes;
        }

        // Update disk I/O rates
        self.disk_read_rate = total_disk_read.saturating_sub(self.prev_disk_read);
        self.disk_write_rate = total_disk_write.saturating_sub(self.prev_disk_write);
        self.prev_disk_read = total_disk_read;
        self.prev_disk_write = total_disk_write;
        self.disk_read_bytes = total_disk_read;
        self.disk_write_bytes = total_disk_write;

        // Update network I/O
        let mut total_rx: u64 = 0;
        let mut total_tx: u64 = 0;
        for (_name, data) in self.networks.iter() {
            total_rx += data.total_received();
            total_tx += data.total_transmitted();
        }

        self.net_rx_rate = total_rx.saturating_sub(self.prev_net_rx);
        self.net_tx_rate = total_tx.saturating_sub(self.prev_net_tx);
        self.prev_net_rx = total_rx;
        self.prev_net_tx = total_tx;
        self.net_rx_bytes = total_rx;
        self.net_tx_bytes = total_tx;

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

    pub fn get_processes(&self) -> Vec<ProcessInfo> {
        ProcessInfo::from_sysinfo(&self.sys)
    }
}
