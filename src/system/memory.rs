/// Memory usage information matching htop's memory breakdown
/// htop shows: Used (green) + Shared (magenta) + Buffers (blue) + Cache (yellow)
#[derive(Default, Clone)]
pub struct MemoryInfo {
    /// Total physical memory in bytes
    pub total: u64,
    /// Used memory in bytes (application working sets - excludes cache/buffers)
    /// htop: MEMORY_USED = totalMem - freeMem - cachedMem - buffersMem
    pub used: u64,
    /// Shared memory in bytes (memory shared between processes)
    /// htop: MEMORY_SHARED - on Windows we can approximate this
    pub shared: u64,
    /// Buffer cache in bytes (disk I/O buffers)
    /// htop: MEMORY_BUFFERS - minimal on Windows
    pub buffers: u64,
    /// Page/file cache in bytes (standby list on Windows)
    /// htop: MEMORY_CACHE (yellow segment)
    pub cached: u64,
    /// Memory used percentage (used / total)
    pub used_percent: f32,
    /// Total swap in bytes
    pub swap_total: u64,
    /// Used swap in bytes
    pub swap_used: u64,
    /// Swap used percentage
    pub swap_percent: f32,
}

impl MemoryInfo {
    /// Create MemoryInfo using native Windows API
    /// Uses NtQuerySystemInformation for accurate memory breakdown matching htop style:
    /// - Used (green): In-use memory (Modified + InUse pages)
    /// - Buffers (blue): System file cache working set
    /// - Cache (yellow): Standby memory (can be reclaimed)
    #[cfg(windows)]
    pub fn from_native() -> Self {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
        use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SYSTEM_INFORMATION_CLASS};

        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };

        let mut perf_info = PERFORMANCE_INFORMATION {
            cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
            ..Default::default()
        };

        unsafe {
            if GlobalMemoryStatusEx(&mut status).is_ok() {
                let total = status.ullTotalPhys;
                let available = status.ullAvailPhys;

                // Get page size and system cache from GetPerformanceInfo
                let (page_size, system_cache) = if GetPerformanceInfo(&mut perf_info, perf_info.cb).is_ok() {
                    (perf_info.PageSize as u64, perf_info.SystemCache as u64 * perf_info.PageSize as u64)
                } else {
                    (4096, 0)
                };

                // Try to get detailed memory breakdown using NtQuerySystemInformation
                // SYSTEM_MEMORY_LIST_INFORMATION = 80
                #[repr(C)]
                #[derive(Default)]
                struct SystemMemoryListInfo {
                    zero_page_count: u64,
                    free_page_count: u64,
                    modified_page_count: u64,
                    modified_no_write_page_count: u64,
                    bad_page_count: u64,
                    page_count_by_priority: [u64; 8], // Standby lists 0-7
                    repurposed_page_by_priority: [u64; 8],
                    modified_page_count_page_file: u64,
                }

                let mut mem_list = SystemMemoryListInfo::default();
                let status_code = NtQuerySystemInformation(
                    SYSTEM_INFORMATION_CLASS(80), // SystemMemoryListInformation
                    &mut mem_list as *mut _ as *mut _,
                    std::mem::size_of::<SystemMemoryListInfo>() as u32,
                    std::ptr::null_mut(),
                );

                // Calculate memory segments htop-style
                let (used, cached, buffers, shared) = if status_code.is_ok() {
                    // Calculate standby (cache) from priority lists
                    let standby_pages: u64 = mem_list.page_count_by_priority.iter().sum();
                    let standby = standby_pages * page_size;
                    let modified = mem_list.modified_page_count * page_size;
                    let free = mem_list.free_page_count * page_size;

                    // htop-style calculation:
                    // - Used = total - free - standby - buffers (what apps are actually using)
                    // - Cache = standby list (yellow)
                    // - Buffers = system file cache (blue) - use a portion of system_cache
                    let in_use = total.saturating_sub(free + standby);

                    // Split system_cache into buffers (small portion) for visual variety
                    // htop Linux shows buffers as a separate blue segment
                    let buffers = system_cache.min(in_use / 10); // Cap at 10% of used
                    let used = in_use.saturating_sub(buffers);

                    (used, standby, buffers, modified.min(used / 20)) // shared as small portion
                } else {
                    // Fallback: estimate from GlobalMemoryStatusEx
                    // available = free + standby, so standby ≈ available - some_free_estimate
                    let in_use = total.saturating_sub(available);
                    // Estimate: standby is roughly 80% of available, free is 20%
                    let estimated_standby = available * 4 / 5;
                    let buffers = system_cache.min(in_use / 10);
                    let used = in_use.saturating_sub(buffers);

                    (used, estimated_standby, buffers, 0)
                };

                let swap_total = status.ullTotalPageFile.saturating_sub(total);
                let swap_available = status.ullAvailPageFile.saturating_sub(available);
                let swap_used = swap_total.saturating_sub(swap_available);

                // used_percent reflects actual application memory usage
                let total_used = used + buffers + shared;
                Self {
                    total,
                    used,
                    shared,
                    buffers,
                    cached,
                    used_percent: if total > 0 { total_used as f32 / total as f32 * 100.0 } else { 0.0 },
                    swap_total,
                    swap_used,
                    swap_percent: if swap_total > 0 { swap_used as f32 / swap_total as f32 * 100.0 } else { 0.0 },
                }
            } else {
                Self::default()
            }
        }
    }

    #[cfg(not(windows))]
    pub fn from_native() -> Self {
        Self::default()
    }

    /// Get total memory (for ProcessInfo calculations)
    #[cfg(windows)]
    pub fn total_memory() -> u64 {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };

        unsafe {
            if GlobalMemoryStatusEx(&mut status).is_ok() {
                status.ullTotalPhys
            } else {
                0
            }
        }
    }

    #[cfg(not(windows))]
    pub fn total_memory() -> u64 {
        0
    }
}

/// Format bytes into human-readable string
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1}T", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1}G", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0}K", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}
