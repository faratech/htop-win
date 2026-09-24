use std::collections::HashMap;
use std::time::{Duration, Instant};

mod cpu;
#[cfg(windows)]
mod d3dkmt;
#[cfg(not(windows))]
#[allow(dead_code)]
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
    pub fn set_gpu_selection(_name: Option<&str>) {}
    pub fn gpu_names() -> Vec<String> {
        Vec::new()
    }
    pub fn process_stats_enabled() -> bool {
        false
    }
    pub fn has_tracked_adapters() -> bool {
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
    GpuInfo, NpuInfo, debug_dump as gpu_debug_dump, gpu_names, has_tracked_adapters,
    set_gpu_process_stats_enabled, set_gpu_selection, set_npu_process_stats_enabled,
};
pub use memory::{MemoryInfo, format_bytes, push_bytes};
// Part of the library API (used by tests/visual_test.rs); the binary target
// compiles this module tree too but never references the re-export, so the
// unused-import lint must be silenced for that target.
#[allow(unused_imports)]
pub use process::ProcessArch;
pub use process::{
    ProcessEnrichmentRequirements, ProcessIdentity, ProcessInfo, apply_cached_metadata,
    enable_debug_privilege, enrich_processes_at,
    enrich_processes, enrich_processes_for, get_process_affinity, get_process_exe_path,
    get_process_io_counters, hydrate_processes_from_cache, kill_process, set_efficiency_mode,
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
    /// Consecutive refreshes since a battery was last reported; drives the
    /// absent-battery repoll latch in `update_battery`.
    #[cfg_attr(not(windows), allow(dead_code))]
    battery_absent_ticks: u32,
    /// Gates active on the previous `refresh` (transition detection).
    #[cfg_attr(not(windows), allow(dead_code))]
    last_collect_gates: u8,
    // GPU (None when no render-capable hardware adapter exists)
    pub gpu: Option<GpuInfo>,
    // NPU (None when no MCDM compute-only adapter exists)
    pub npu: Option<NpuInfo>,
    // Previous values for rate calculation
    prev_network: Option<HashMap<u64, NetworkCounters>>,
    prev_net_sample: Option<Instant>,
    // Native process enumeration state
    /// Capacity baseline for process CPU%; follows the cached topology (30 s
    /// TTL) so hot-add or processor-group changes move the denominator with
    /// the layout instead of staying frozen at the value captured at startup.
    #[cfg_attr(not(windows), allow(dead_code))]
    logical_processor_count: usize,
    #[cfg_attr(not(windows), allow(dead_code))]
    last_native_refresh: Instant,
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
            battery_absent_ticks: 0,
            last_collect_gates: 0, // != ALL so the first refresh re-primes baselines
            gpu: None,
            npu: None,
            prev_network: None,
            prev_net_sample: None,
            logical_processor_count: active_logical_processor_count(),
            last_native_refresh: Instant::now(),
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

/// Cumulative counters for one network interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct NetworkCounters {
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

/// Convert a wall-clock sample interval into total CPU capacity in the same
/// 100-nanosecond units used by SYSTEM_PROCESS_INFORMATION.
#[inline]
#[cfg_attr(not(windows), allow(dead_code))]
fn cpu_capacity_100ns(elapsed: Duration, logical_processor_count: usize) -> u64 {
    let capacity = elapsed
        .as_nanos()
        .saturating_mul(logical_processor_count.max(1) as u128)
        / 100;
    capacity.min(u64::MAX as u128) as u64
}

/// Capacity denominator for this tick: the count reported by the live
/// processor enumeration when it yields one, otherwise the last known count
/// (never zero, so the interval can still be divided).
#[inline]
#[cfg_attr(not(windows), allow(dead_code))]
fn tick_processor_count(previous: usize, enumerated: usize) -> usize {
    if enumerated > 0 {
        enumerated
    } else {
        previous.max(1)
    }
}

#[inline]
#[cfg_attr(not(windows), allow(dead_code))]
fn process_cpu_percentage(time_delta: u64, capacity_delta: u64) -> f32 {
    if capacity_delta == 0 {
        0.0
    } else {
        ((time_delta as f64 / capacity_delta as f64 * 100.0) as f32).clamp(0.0, 100.0)
    }
}

#[cfg(windows)]
fn active_logical_processor_count() -> usize {
    // Cached topology (30 s TTL): the per-tick capacity denominator and the
    // CPU refresh share one enumeration instead of querying every group twice
    // per tick. Hot-add still propagates via `tick_processor_count`.
    cpu::cached_processor_layout_len().max(1)
}

#[cfg(not(windows))]
fn active_logical_processor_count() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
}

/// Calculate totals and rates without allowing new, removed, or reset
/// interfaces to contribute lifetime counters to the interval rate.
fn network_totals_and_rates(
    current: &HashMap<u64, NetworkCounters>,
    previous: Option<&HashMap<u64, NetworkCounters>>,
    elapsed_secs: f64,
) -> (u64, u64, u64, u64) {
    let mut total_rx = 0u64;
    let mut total_tx = 0u64;
    let mut delta_rx = 0u64;
    let mut delta_tx = 0u64;

    for (identity, counters) in current {
        total_rx = total_rx.saturating_add(counters.rx_bytes);
        total_tx = total_tx.saturating_add(counters.tx_bytes);

        if let Some(old) = previous.and_then(|interfaces| interfaces.get(identity)) {
            if counters.rx_bytes >= old.rx_bytes {
                delta_rx = delta_rx.saturating_add(counters.rx_bytes - old.rx_bytes);
            }
            if counters.tx_bytes >= old.tx_bytes {
                delta_tx = delta_tx.saturating_add(counters.tx_bytes - old.tx_bytes);
            }
        }
    }

    (
        total_rx,
        total_tx,
        bytes_per_second(delta_rx, elapsed_secs),
        bytes_per_second(delta_tx, elapsed_secs),
    )
}

#[inline]
#[cfg_attr(not(windows), allow(dead_code))]
fn aggregate_io_rates(io_rates: &HashMap<u32, (u64, u64)>) -> (u64, u64) {
    io_rates.values().fold((0u64, 0u64), |totals, rates| {
        (
            totals.0.saturating_add(rates.0),
            totals.1.saturating_add(rates.1),
        )
    })
}

#[inline]
#[cfg_attr(not(windows), allow(dead_code))]
fn is_displayed_task(pid: u32) -> bool {
    pid != 0
}

thread_local! {
    /// Current interface counters, cleared and refilled each tick; swapped
    /// into `SystemMetrics::prev_network` afterwards so both maps keep their
    /// capacity instead of one side being rebuilt per tick.
    static NETWORK_INTERFACES: std::cell::RefCell<HashMap<u64, NetworkCounters>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Fill [`NETWORK_INTERFACES`] with the up (non-loopback) interface counters.
/// Returns false when the IP Helper query failed (callers keep last values).
#[cfg(windows)]
fn get_network_stats() -> bool {
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, IF_TYPE_SOFTWARE_LOOPBACK, MIB_IF_TABLE2,
    };
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table) != WIN32_ERROR(0) || table.is_null() {
            return false;
        }

        let num_entries = (*table).NumEntries as usize;
        let entries = std::slice::from_raw_parts((*table).Table.as_ptr(), num_entries);

        NETWORK_INTERFACES.with(|cell| {
            let mut interfaces = cell.borrow_mut();
            interfaces.clear();
            for entry in entries {
                // Skip loopback and non-operational interfaces. Omitting a down
                // interface also discards its baseline so reconnecting starts at 0.
                if entry.OperStatus == IfOperStatusUp
                    && entry.Type != IF_TYPE_SOFTWARE_LOOPBACK
                {
                    interfaces.insert(
                        entry.InterfaceLuid.Value,
                        NetworkCounters {
                            rx_bytes: entry.InOctets,
                            tx_bytes: entry.OutOctets,
                        },
                    );
                }
            }
        });

        FreeMibTable(table as *const _);
    }

    true
}

#[cfg(not(windows))]
fn get_network_stats() -> bool {
    NETWORK_INTERFACES.with(|cell| cell.borrow_mut().clear());
    false
}

// Scratch PID→slot index reused across refresh ticks. Thread-local storage
// needs no synchronization, keeps `SystemMetrics` cheap to clone for every
// snapshot sent to the UI thread, and stays correct if another thread ever
// runs this (each thread simply reuses its own instance).
#[cfg(windows)]
thread_local! {
    static PID_INDEX_SCRATCH: std::cell::RefCell<HashMap<u32, usize>> =
        std::cell::RefCell::new(HashMap::new());
    /// Survivor-liveness flags, one per entry of the incoming process vec.
    static ALIVE_SCRATCH: std::cell::RefCell<Vec<bool>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Processes that appeared in this tick's raw list but not in the vec.
    static NEW_PROCESSES: std::cell::RefCell<Vec<ProcessInfo>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Which subsystems `SystemMetrics::refresh` collects this tick. Gates mirror
/// the UI's meter visibility (see `App::canonical_collect_requirements`); a
/// gated-off subsystem keeps its last collected values. All bits set = collect
/// everything (the default, so headless runs and tests see full collection).
pub mod collect_gates {
    pub const NET: u8 = 1 << 0;
    pub const BATTERY: u8 = 1 << 1;
    pub const CPU: u8 = 1 << 2;
    pub const GPU: u8 = 1 << 3;
    pub const NPU: u8 = 1 << 4;
    pub const ALL: u8 = NET | BATTERY | CPU | GPU | NPU;
}

static COLLECT_GATES: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(collect_gates::ALL);

/// Absent-battery latch: confirm absence for this many ticks, then repoll
/// only every [`BATTERY_ABSENT_REPOLL_TICKS`] (≈30 s at the default rate).
const BATTERY_ABSENT_CONFIRM_TICKS: u32 = 8;
const BATTERY_ABSENT_REPOLL_TICKS: u32 = 20;

/// Update the per-tick collection gates (called from the UI thread when meter
/// visibility changes).
pub fn set_collect_gates(bits: u8) {
    COLLECT_GATES.store(bits, std::sync::atomic::Ordering::Relaxed);
}

impl SystemMetrics {
    /// Refresh system metrics (CPU, memory, uptime, hostname, battery, network)
    /// Does NOT refresh processes - use get_processes_native() for that
    pub fn refresh(&mut self) {
        // Gate transitions re-prime rate baselines so the first tick after a
        // subsystem is re-enabled reports a clean zero rate instead of a
        // gap-averaged value.
        let gates = COLLECT_GATES.load(std::sync::atomic::Ordering::Relaxed);
        let newly_enabled = gates & !self.last_collect_gates;
        self.last_collect_gates = gates;
        if newly_enabled & collect_gates::NET != 0 {
            self.prev_net_sample = None;
        }
        if newly_enabled & collect_gates::CPU != 0 {
            cpu::reset_first_sample();
        }

        // Update CPU info using native API
        // In-place refresh keeps the per-core vecs' capacity across ticks.
        if gates & collect_gates::CPU != 0 {
            self.cpu.refresh_in_place();
        }

        // Update memory info using native API
        self.memory = MemoryInfo::from_native();

        // Update uptime
        self.uptime = get_uptime();

        // Hostname rarely changes – compute once to avoid repeated allocations
        if self.hostname.is_empty() {
            self.hostname = get_hostname();
        }

        // Update network I/O using native API
        if gates & collect_gates::NET != 0 && get_network_stats() {
            let now = Instant::now();
            let elapsed = self
                .prev_net_sample
                .map(|sample| now.duration_since(sample).as_secs_f64())
                .unwrap_or(0.0);
            NETWORK_INTERFACES.with(|cell| {
                let mut interfaces = cell.borrow_mut();
                let (rx_bytes, tx_bytes, rx_rate, tx_rate) = network_totals_and_rates(
                    &interfaces,
                    self.prev_network.as_ref(),
                    elapsed,
                );
                self.net_rx_bytes = rx_bytes;
                self.net_tx_bytes = tx_bytes;
                self.net_rx_rate = rx_rate;
                self.net_tx_rate = tx_rate;
                // Swap so the old baseline map becomes next tick's scratch and
                // both sides keep their capacity.
                match self.prev_network.as_mut() {
                    Some(prev) => std::mem::swap(prev, &mut interfaces),
                    None => self.prev_network = Some(std::mem::take(&mut interfaces)),
                }
            });
            self.prev_net_sample = Some(now);
        }

        // Update battery status
        if gates & collect_gates::BATTERY != 0 {
            self.update_battery();
        }

        // Update GPU/NPU metrics (no-op on machines without tracked adapters)
        if gates & (collect_gates::GPU | collect_gates::NPU) != 0 {
            let adapters = d3dkmt::refresh();
            self.gpu = adapters.gpu;
            self.npu = adapters.npu;
        }
    }

    fn update_battery(&mut self) {
        // Absent-battery latch: on desktops (no battery) this would otherwise
        // poll GetSystemPowerStatus every tick forever. After a few absent
        // reads, repoll only every ~30 s so hot-docking still recovers.
        self.battery_absent_ticks = self.battery_absent_ticks.saturating_add(1);
        if self.battery_percent.is_none()
            && self.battery_absent_ticks > BATTERY_ABSENT_CONFIRM_TICKS
            && !self.battery_absent_ticks.is_multiple_of(BATTERY_ABSENT_REPOLL_TICKS)
        {
            return;
        }
        // Use Windows API for battery status
        #[cfg(windows)]
        {
            use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
            let mut status = SYSTEM_POWER_STATUS::default();
            unsafe {
                if GetSystemPowerStatus(&mut status).is_ok() {
                    if status.BatteryLifePercent <= 100 {
                        self.battery_percent = Some(status.BatteryLifePercent as f32);
                        self.battery_absent_ticks = 0;
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
        use self::native::{calculate_process_rates, with_process_list, with_process_rates};

        // On query failure (None), keep the previous process list and baselines
        // untouched rather than blanking the table for a frame.
        let _ = with_process_list(|proc_list| {
            // Update time tracking for CPU delta calculation. The capacity
            // denominator follows the cached topology (30 s TTL): a hot-add
            // or processor-group change propagates at the next re-enumeration
            // instead of skewing every percentage until restart.
            let now = Instant::now();
            self.logical_processor_count = tick_processor_count(
                self.logical_processor_count,
                active_logical_processor_count(),
            );
            let cpu_capacity = cpu_capacity_100ns(
                now.duration_since(self.last_native_refresh),
                self.logical_processor_count,
            );
            self.last_native_refresh = now;

            // First pass: calculate process and cumulative I/O totals.
            let mut tasks_total = 0;
            let mut threads_total = 0;
            let mut total_disk_read: u64 = 0;
            let mut total_disk_write: u64 = 0;

            for proc in proc_list.iter() {
                tasks_total += usize::from(is_displayed_task(proc.pid()));
                threads_total += proc.thread_count() as usize;
                total_disk_read = total_disk_read.saturating_add(proc.read_bytes());
                total_disk_write = total_disk_write.saturating_add(proc.write_bytes());
            }

            // Open the tick's exe-stat budget so due re-stats spread across
            // ticks instead of stat-bombing the filesystem in one synchronized
            // burst, and size the exe-status cache to this process count.
            CACHE.begin_exe_tick(cache::config::EXE_STATS_PER_TICK, tasks_total);

            // Get CPU percentages and I/O rates based on cache deltas
            // (fills thread-local rate tables; read back via with_process_rates)
            calculate_process_rates(&proc_list, cpu_capacity);

            // Update global stats
            self.tasks_total = tasks_total;
            // Windows doesn't expose per-process running/sleeping state like Linux.
            // Exclude System Idle Process (PID 0) from task count; don't fabricate sleep counts.
            self.tasks_running = 0;
            self.tasks_sleeping = 0;
            self.threads_total = threads_total;

            (self.disk_read_rate, self.disk_write_rate) =
                with_process_rates(|rates| aggregate_io_rates(&rates.io_rates));
            self.disk_read_bytes = total_disk_read;
            self.disk_write_bytes = total_disk_write;

            let total_mem = MemoryInfo::total_memory();

            // Index of existing processes by PID for fast lookup. The map
            // lives in thread-local scratch so the per-tick table allocation
            // disappears; `clear` keeps the capacity for the next tick.
            // Liveness of survivors is tracked with one flag per existing
            // slot instead of a second hashed set: the merge loop below is
            // the only writer, so the dead-removal pass needs no hashing.
            // (Sound because this function maintains 1:1 PID uniqueness:
            // survivors are a subset of the previous output and new entries
            // are disjoint from them, starting from an empty vector.)
            // Liveness flags and the new-processes vec also live in
            // thread-local scratch so a steady-state tick allocates nothing.
            ALIVE_SCRATCH.with(|alive_cell| {
                let mut alive = alive_cell.borrow_mut();
                alive.clear();
                alive.resize(processes.len(), false);

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
            PID_INDEX_SCRATCH.with(|scratch| {
                let mut existing_map = scratch.borrow_mut();
                existing_map.clear();
                existing_map.reserve(processes.len());
                for (i, p) in processes.iter().enumerate() {
                    existing_map.insert(p.pid, i);
                }
                NEW_PROCESSES.with(|new_cell| {
                    let mut new_processes = new_cell.borrow_mut();
                    new_processes.clear();
                with_process_rates(|rates| {
                    for raw_proc in proc_list.iter() {
                        let pid = raw_proc.pid();
                        let cpu_pct = rates.cpu_percentages.get(&pid).copied().unwrap_or(0.0);
                        let (io_read_rate, io_write_rate) =
                            rates.io_rates.get(&pid).copied().unwrap_or((0, 0));
                        let proc_adapter = adapter_stats.get(&pid).copied().unwrap_or_default();

                        if let Some(&idx) = existing_map.get(&pid) {
                            alive[idx] = true;
                            let native_start = raw_proc.create_time();
                            let existing_proc = &mut processes[idx];

                            if native_start == existing_proc.create_time_100ns {
                                // Update existing process (reuses string allocations)
                                existing_proc.update_from_raw(&raw_proc, cpu_pct, total_mem);
                            } else {
                                // PID reuse: replace entirely
                                *existing_proc =
                                    ProcessInfo::from_raw(&raw_proc, cpu_pct, total_mem);
                            }
                            existing_proc.io_read_rate = io_read_rate;
                            existing_proc.io_write_rate = io_write_rate;
                            existing_proc.gpu_percent = proc_adapter.gpu_percent;
                            existing_proc.gpu_memory = proc_adapter.gpu_memory;
                            existing_proc.npu_percent = proc_adapter.npu_percent;
                            existing_proc.npu_memory = proc_adapter.npu_memory;
                        } else {
                            let mut proc_info =
                                ProcessInfo::from_raw(&raw_proc, cpu_pct, total_mem);
                            proc_info.io_read_rate = io_read_rate;
                            proc_info.io_write_rate = io_write_rate;
                            proc_info.gpu_percent = proc_adapter.gpu_percent;
                            proc_info.gpu_memory = proc_adapter.gpu_memory;
                            proc_info.npu_percent = proc_adapter.npu_percent;
                            proc_info.npu_memory = proc_adapter.npu_memory;
                            new_processes.push(proc_info);
                        }
                    }
                });
                });
            });

                // Remove dead processes. Order-preserving and hash-free: `alive`
                // was sized to `processes` before the merge above, and appending
                // happens after, so indices still line up.
                let mut slot = 0;
                processes.retain(|_| {
                    let keep = alive[slot];
                    slot += 1;
                    keep
                });
            });

            // Append new processes (append drains the vec but keeps its
            // capacity in the thread-local for the next tick)
            NEW_PROCESSES.with(|new_cell| {
                let mut new_processes = new_cell.borrow_mut();
                if !new_processes.is_empty() {
                    processes.append(&mut new_processes);
                }
            });

            // Only a successful kernel query may evict cached identities. The
            // input vector can legitimately be empty while awaiting UI reuse.
            // The live set is built only on cleanup ticks (every
            // CLEANUP_INTERVAL refreshes) instead of on every tick.
            if CACHE.should_cleanup() {
                let live_pids: std::collections::HashSet<u32> =
                    processes.iter().map(|p| p.pid).collect();
                self::process::cleanup_stale_caches(&live_pids);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn counters(rx_bytes: u64, tx_bytes: u64) -> NetworkCounters {
        NetworkCounters { rx_bytes, tx_bytes }
    }

    #[test]
    fn cpu_capacity_uses_all_logical_processors() {
        assert_eq!(
            cpu_capacity_100ns(Duration::from_secs(1), 128),
            1_280_000_000
        );
        assert_eq!(cpu_capacity_100ns(Duration::ZERO, 128), 0);
        assert_eq!(
            cpu_capacity_100ns(Duration::from_secs(u64::MAX), 128),
            u64::MAX
        );
    }

    #[test]
    fn process_cpu_percentage_is_bounded_and_zero_safe() {
        assert_eq!(process_cpu_percentage(250, 1_000), 25.0);
        assert_eq!(process_cpu_percentage(1, 0), 0.0);
        assert_eq!(process_cpu_percentage(2_000, 1_000), 100.0);
    }

    #[test]
    fn capacity_denominator_follows_core_count_changes() {
        // Constant layout: the tick denominator is unchanged, so the math
        // stays exactly as before.
        let steady = tick_processor_count(8, 8);
        assert_eq!(
            cpu_capacity_100ns(Duration::from_millis(500), steady),
            cpu_capacity_100ns(Duration::from_millis(500), 8)
        );

        // Hot-add / new processor group: the denominator grows with the
        // current enumeration instead of staying at the startup count.
        let grown = tick_processor_count(8, 16);
        assert_eq!(grown, 16);
        assert_eq!(
            cpu_capacity_100ns(Duration::from_millis(500), grown),
            cpu_capacity_100ns(Duration::from_millis(500), 8) * 2
        );

        // A shrinking layout is picked up as well.
        assert_eq!(tick_processor_count(16, 8), 8);

        // An empty enumeration (failed query) keeps the last known count.
        assert_eq!(tick_processor_count(8, 0), 8);
        // ...and never lets it drop to zero.
        assert_eq!(tick_processor_count(0, 0), 1);
    }

    #[test]
    fn network_first_sample_and_new_interfaces_start_at_zero() {
        let first = HashMap::from([(1, counters(10_000, 20_000))]);
        assert_eq!(
            network_totals_and_rates(&first, None, 1.0),
            (10_000, 20_000, 0, 0)
        );

        let second = HashMap::from([(1, counters(10_500, 20_250)), (2, counters(99_000, 88_000))]);
        assert_eq!(
            network_totals_and_rates(&second, Some(&first), 0.5),
            (109_500, 108_250, 1_000, 500)
        );
    }

    #[test]
    fn network_interface_churn_and_counter_resets_do_not_spike() {
        let before_disconnect =
            HashMap::from([(1, counters(1_000, 2_000)), (2, counters(5_000, 6_000))]);
        let disconnected = HashMap::from([(2, counters(5_100, 6_200))]);
        assert_eq!(
            network_totals_and_rates(&disconnected, Some(&before_disconnect), 1.0),
            (5_100, 6_200, 100, 200)
        );

        let reconnected =
            HashMap::from([(1, counters(50_000, 60_000)), (2, counters(5_200, 6_300))]);
        assert_eq!(
            network_totals_and_rates(&reconnected, Some(&disconnected), 1.0),
            (55_200, 66_300, 100, 100)
        );

        let reset = HashMap::from([(1, counters(10, 20)), (2, counters(5_300, 6_400))]);
        assert_eq!(
            network_totals_and_rates(&reset, Some(&reconnected), 1.0),
            (5_310, 6_420, 100, 100)
        );
    }

    #[test]
    fn disk_aggregate_saturates_and_excludes_no_current_rate() {
        let rates = HashMap::from([(10, (100, 200)), (20, (300, 400))]);
        assert_eq!(aggregate_io_rates(&rates), (400, 600));

        let overflowing = HashMap::from([(10, (u64::MAX, u64::MAX)), (20, (1, 1))]);
        assert_eq!(aggregate_io_rates(&overflowing), (u64::MAX, u64::MAX));
    }

    #[test]
    fn system_idle_process_is_not_a_displayed_task() {
        assert!(!is_displayed_task(0));
        assert!(is_displayed_task(4));
    }

    #[test]
    fn enrichment_requirement_bits_round_trip_and_compare_coverage() {
        let required = ProcessEnrichmentRequirements {
            user: true,
            exe_path: true,
            ..Default::default()
        };
        assert_eq!(
            ProcessEnrichmentRequirements::from_bits(required.bits()),
            required
        );
        assert!(ProcessEnrichmentRequirements::visible(true).contains(required));
        assert!(!ProcessEnrichmentRequirements::default().contains(required));
    }

    #[test]
    fn gated_subsystems_keep_last_values() {
        use super::collect_gates;
        use super::set_collect_gates;

        let mut metrics = SystemMetrics {
            net_rx_bytes: 42,
            net_tx_bytes: 43,
            cpu: super::CpuInfo {
                core_usage: vec![7.5],
                ..Default::default()
            },
            battery_percent: Some(88.0),
            ..Default::default()
        };

        // All gates off: refresh leaves every gated value untouched.
        set_collect_gates(0);
        metrics.refresh();
        assert_eq!(metrics.net_rx_bytes, 42);
        assert_eq!(metrics.net_tx_bytes, 43);
        assert_eq!(metrics.cpu.core_usage, vec![7.5]);
        assert_eq!(metrics.battery_percent, Some(88.0));

        // Restored gates: refresh runs normally (values may change, but the
        // collection path must execute without panicking on any platform).
        set_collect_gates(collect_gates::ALL);
        metrics.refresh();
        set_collect_gates(collect_gates::ALL);
    }
}
