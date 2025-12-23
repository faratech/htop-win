use std::sync::OnceLock;

/// Cached page size - never changes at runtime
static PAGE_SIZE: OnceLock<u64> = OnceLock::new();

/// Get the system page size (cached after first call)
#[cfg(windows)]
fn get_page_size() -> u64 {
    *PAGE_SIZE.get_or_init(|| {
        use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
        let mut sys_info = SYSTEM_INFO::default();
        unsafe { GetSystemInfo(&mut sys_info) };
        sys_info.dwPageSize as u64
    })
}

#[cfg(not(windows))]
fn get_page_size() -> u64 {
    4096
}

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

                // "In Use" as shown by Task Manager = Total - Available
                // This is the most reliable calculation that matches Windows Task Manager
                let in_use = total.saturating_sub(available);

                // Use cached page size (never changes at runtime)
                let page_size = get_page_size();

                // Get system cache size from GetPerformanceInfo
                let system_cache = if GetPerformanceInfo(&mut perf_info, perf_info.cb).is_ok() {
                    perf_info.SystemCache as u64 * page_size
                } else {
                    0
                };

                // Try to get detailed memory breakdown for cache visualization
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

                // Calculate cache breakdown for visualization
                // "In Use" is always from GlobalMemoryStatusEx to match Task Manager
                let (used, cached, buffers, shared) = if status_code.is_ok() {
                    // Calculate standby (cache) from priority lists
                    let standby_pages: u64 = mem_list.page_count_by_priority.iter().sum();
                    let standby = standby_pages * page_size;
                    let modified = mem_list.modified_page_count * page_size;

                    // Cache = standby (clean cached) + modified (dirty cached)
                    // This is what Windows considers "Available" minus truly free pages
                    let cache = standby + modified;

                    // Buffers: portion of system file cache (for htop-style display)
                    let buffers = system_cache.min(in_use / 10); // Cap at 10% of used
                    let used = in_use.saturating_sub(buffers);

                    (used, cache, buffers, 0)
                } else {
                    // Fallback without detailed breakdown
                    // Estimate cache as the difference between available and a small free estimate
                    let estimated_cache = available.saturating_sub(available / 10);
                    let buffers = system_cache.min(in_use / 10);
                    let used = in_use.saturating_sub(buffers);

                    (used, estimated_cache, buffers, 0)
                };

                // Get actual page file usage using NtQuerySystemInformation
                // SystemPageFileInformation = 18
                #[repr(C)]
                struct SystemPageFileInfo {
                    next_entry_offset: u32,
                    total_size: u32,      // Total size in pages
                    total_in_use: u32,    // Currently in use in pages
                    peak_usage: u32,
                    page_file_name: [u16; 260], // UNICODE_STRING follows but we don't need it
                }

                let mut pagefile_info: [u8; 512] = [0; 512];
                let mut return_length: u32 = 0;
                let pf_status = NtQuerySystemInformation(
                    SYSTEM_INFORMATION_CLASS(18), // SystemPageFileInformation
                    pagefile_info.as_mut_ptr() as *mut _,
                    pagefile_info.len() as u32,
                    &mut return_length,
                );

                let (swap_total, swap_used) = if pf_status.is_ok() && return_length >= 16 {
                    // Parse the page file info - may have multiple page files
                    let mut total_size: u64 = 0;
                    let mut total_in_use: u64 = 0;
                    let mut offset = 0usize;

                    loop {
                        if offset + 16 > pagefile_info.len() {
                            break;
                        }
                        let info = &*(pagefile_info.as_ptr().add(offset) as *const SystemPageFileInfo);
                        total_size += info.total_size as u64 * page_size;
                        total_in_use += info.total_in_use as u64 * page_size;

                        if info.next_entry_offset == 0 {
                            break;
                        }
                        offset += info.next_entry_offset as usize;
                    }
                    (total_size, total_in_use)
                } else {
                    // Fallback: estimate from GetPerformanceInfo
                    // Page file size ≈ commit limit - physical memory
                    let pf_total = (perf_info.CommitLimit as u64)
                        .saturating_sub(perf_info.PhysicalTotal as u64) * page_size;
                    // Usage estimate: committed that exceeds physical
                    let pf_used = (perf_info.CommitTotal as u64 * page_size)
                        .saturating_sub(total);
                    (pf_total, pf_used.min(pf_total))
                };

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
