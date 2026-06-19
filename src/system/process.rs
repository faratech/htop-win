#[cfg(windows)]
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

#[cfg(windows)]
use super::native::{SystemProcess, filetime_to_unix};

#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, SetLastError, WIN32_ERROR};
#[cfg(windows)]
use windows::Win32::Security::{
    AdjustTokenPrivileges, GetTokenInformation, LUID_AND_ATTRIBUTES, LookupAccountSidW,
    LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, SID_NAME_USE, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
};
#[cfg(windows)]
use windows::Win32::System::Threading::IO_COUNTERS;
#[cfg(windows)]
use windows::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess,
    GetProcessInformation, GetProcessIoCounters, HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS,
    IsWow64Process2, NORMAL_PRIORITY_CLASS, OpenProcess, OpenProcessToken,
    PROCESS_MACHINE_INFORMATION, PROCESS_NAME_WIN32, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SET_INFORMATION, PROCESS_TERMINATE, ProcessMachineTypeInfo, ProcessPowerThrottling,
    QueryFullProcessImageNameW, REALTIME_PRIORITY_CLASS, SetPriorityClass, TerminateProcess,
};
#[cfg(windows)]
use windows::core::PWSTR;

/// Enable SeDebugPrivilege to access process information for service accounts
/// This allows reading tokens for NETWORK SERVICE, LOCAL SERVICE, etc.
/// Only succeeds if running as Administrator
#[cfg(windows)]
pub fn enable_debug_privilege() -> bool {
    use windows::core::w;
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .is_err()
        {
            return false;
        }

        let mut luid = windows::Win32::Foundation::LUID::default();
        // SE_DEBUG_NAME = "SeDebugPrivilege"
        if LookupPrivilegeValueW(None, w!("SeDebugPrivilege"), &mut luid).is_err() {
            let _ = CloseHandle(token);
            return false;
        }

        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };

        SetLastError(WIN32_ERROR(0));
        let result = AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None).is_ok()
            && GetLastError().0 != 1300; // ERROR_NOT_ALL_ASSIGNED
        let _ = CloseHandle(token);
        result
    }
}

#[cfg(not(windows))]
pub fn enable_debug_privilege() -> bool {
    false
}

/// Check if an executable has been modified or deleted since a process started.
/// Returns (exe_updated, exe_deleted) like htop's red basename highlighting.
/// Delegates to unified cache module.
#[cfg(windows)]
#[inline]
fn check_exe_status(exe_path: &str, start_time_100ns: u64) -> (bool, bool) {
    super::cache::CACHE.check_exe_status(exe_path, start_time_100ns)
}

#[cfg(not(windows))]
fn check_exe_status(_exe_path: &str, _start_time_100ns: u64) -> (bool, bool) {
    (false, false)
}

// Common usernames as UTF-16 for fast comparison (avoids UTF-16 to UTF-8 conversion)
#[cfg(windows)]
const SYSTEM_UTF16: [u16; 6] = [0x53, 0x59, 0x53, 0x54, 0x45, 0x4D]; // "SYSTEM"
#[cfg(windows)]
const LOCAL_SERVICE_UTF16: [u16; 13] = [
    0x4C, 0x4F, 0x43, 0x41, 0x4C, 0x20, 0x53, 0x45, 0x52, 0x56, 0x49, 0x43, 0x45,
]; // "LOCAL SERVICE"
#[cfg(windows)]
const NETWORK_SERVICE_UTF16: [u16; 15] = [
    0x4E, 0x45, 0x54, 0x57, 0x4F, 0x52, 0x4B, 0x20, 0x53, 0x45, 0x52, 0x56, 0x49, 0x43, 0x45,
]; // "NETWORK SERVICE"

// Pre-allocated static strings for common usernames
#[cfg(windows)]
static SYSTEM_STR: &str = "SYSTEM";
#[cfg(windows)]
static LOCAL_SERVICE_STR: &str = "LOCAL SERVICE";
#[cfg(windows)]
static NETWORK_SERVICE_STR: &str = "NETWORK SERVICE";

/// Intern a username from UTF-16, avoiding conversion for common names
#[cfg(windows)]
#[inline]
fn intern_username_utf16(name: &[u16]) -> Arc<str> {
    // Fast path: check against known common usernames (avoids UTF-16 conversion)
    if name == SYSTEM_UTF16 {
        return Arc::from(SYSTEM_STR);
    }
    if name == LOCAL_SERVICE_UTF16 {
        return Arc::from(LOCAL_SERVICE_STR);
    }
    if name == NETWORK_SERVICE_UTF16 {
        return Arc::from(NETWORK_SERVICE_STR);
    }
    // Fallback: convert from UTF-16
    Arc::from(String::from_utf16_lossy(name))
}

/// Clean up caches by removing entries for PIDs that no longer exist
/// Delegates to unified cache module
#[cfg(windows)]
pub fn cleanup_stale_caches(current_pids: &std::collections::HashSet<u32>) {
    super::cache::CACHE.cleanup(current_pids);
}

/// Process architecture
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProcessArch {
    #[default]
    Native, // Native architecture (matches OS)
    X86,   // 32-bit x86 (WoW64 on x64/ARM64)
    X64,   // x64 running on ARM64 via emulation
    ARM64, // Native ARM64
}

impl ProcessArch {
    /// Short display string for the architecture
    pub fn as_str(&self) -> &'static str {
        match self {
            ProcessArch::Native => "",
            ProcessArch::X86 => "x86",
            ProcessArch::X64 => "x64",
            ProcessArch::ARM64 => "ARM",
        }
    }
}

// ============================================================================
// Helper functions to reduce code duplication
// ============================================================================

/// Open a process handle with fallback from full to limited query access
#[cfg(windows)]
#[inline]
pub(crate) fn open_process_query(pid: u32) -> Option<HANDLE> {
    unsafe {
        match OpenProcess(PROCESS_QUERY_INFORMATION, false, pid) {
            Ok(h) if !h.is_invalid() => Some(h),
            _ => match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(h) if !h.is_invalid() => Some(h),
                _ => None,
            },
        }
    }
}

/// Query the full executable path from a process handle
#[cfg(windows)]
#[inline]
fn query_exe_path(handle: HANDLE) -> String {
    unsafe {
        let mut capacity = 1024usize;
        loop {
            let mut buffer = vec![0u16; capacity];
            let mut size = buffer.len() as u32;
            if QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut size,
            )
            .is_ok()
            {
                return String::from_utf16_lossy(&buffer[..size as usize]);
            }
            if capacity >= 32768 {
                return String::new();
            }
            capacity *= 2;
        }
    }
}

/// Extract username from an already-opened token handle (avoids duplicate OpenProcess)
#[cfg(windows)]
fn get_user_from_token(token_handle: HANDLE, pid: u32) -> Option<Arc<str>> {
    unsafe {
        // Get token user info - first call to get required size
        let mut token_info_len: u32 = 0;
        let _ = GetTokenInformation(token_handle, TokenUser, None, 0, &mut token_info_len);

        if token_info_len == 0 {
            return None;
        }

        // Allocate buffer and get token info
        let mut token_info: Vec<usize> =
            vec![0; (token_info_len as usize).div_ceil(std::mem::size_of::<usize>())];
        if GetTokenInformation(
            token_handle,
            TokenUser,
            Some(token_info.as_mut_ptr() as *mut _),
            token_info_len,
            &mut token_info_len,
        )
        .is_err()
        {
            return None;
        }

        let token_user = &*(token_info.as_ptr() as *const TOKEN_USER);

        // Look up the account name from the SID
        let mut name_len: u32 = 256;
        let mut domain_len: u32 = 256;
        let mut name: Vec<u16> = vec![0; name_len as usize];
        let mut domain: Vec<u16> = vec![0; domain_len as usize];
        let mut sid_type = SID_NAME_USE::default();

        if LookupAccountSidW(
            None,
            token_user.User.Sid,
            Some(PWSTR(name.as_mut_ptr())),
            &mut name_len,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut domain_len,
            &mut sid_type,
        )
        .is_ok()
        {
            // Use interning to avoid UTF-16 conversion for common usernames
            let username = intern_username_utf16(&name[..name_len as usize]);

            // Cache the result using unified cache
            super::cache::CACHE.set_user(pid, username.clone());

            Some(username)
        } else {
            None
        }
    }
}

/// Get I/O counters for a specific process (on-demand, for ProcessInfo dialog)
#[cfg(windows)]
pub fn get_process_io_counters(pid: u32) -> (u64, u64) {
    let handle = match open_process_query(pid) {
        Some(h) => h,
        None => return (0, 0),
    };
    unsafe {
        let mut io = IO_COUNTERS::default();
        let result = if GetProcessIoCounters(handle, &mut io).is_ok() {
            (io.ReadTransferCount, io.WriteTransferCount)
        } else {
            (0, 0)
        };
        let _ = CloseHandle(handle);
        result
    }
}

#[cfg(not(windows))]
pub fn get_process_io_counters(_pid: u32) -> (u64, u64) {
    (0, 0)
}

/// Get executable path for a specific process (on-demand, for ProcessInfo dialog)
#[cfg(windows)]
pub fn get_process_exe_path(pid: u32) -> String {
    let handle = match open_process_query(pid) {
        Some(h) => h,
        None => return String::new(),
    };
    let result = query_exe_path(handle);
    unsafe {
        let _ = CloseHandle(handle);
    }
    result
}

#[cfg(not(windows))]
pub fn get_process_exe_path(_pid: u32) -> String {
    String::new()
}

/// Enriched data from Windows API for visible processes
#[cfg(windows)]
struct EnrichedProcessData {
    pid: u32,
    shared_mem: u64,
    efficiency_mode: bool,
    /// True only when efficiency_mode was freshly queried this pass (handle opened
    /// and GetProcessInformation ran). When false the value came from cache, so the
    /// cache's TTL timestamp must NOT be bumped — otherwise it never expires and the
    /// indicator freezes at its first-seen value for any continuously-visible process.
    efficiency_fresh: bool,
    /// True only when architecture was actually queried or is a known synthetic
    /// system-process value.
    arch_fresh: bool,
    /// True only when a non-empty executable path was actually queried or is a
    /// known synthetic system-process value.
    exe_path_fresh: bool,
    /// True only when TokenElevation returned an authoritative elevation result.
    elevation_fresh: bool,
    is_elevated: bool,
    arch: ProcessArch,
    user: Option<Arc<str>>,
    exe_path: String,
}

/// Enrich processes with data not available from NtQuerySystemInformation
/// (shared_mem, efficiency_mode, is_elevated, arch, user, exe_path)
/// Note: cpu_time and start_time come from NtQuerySystemInformation (in from_native) for ALL processes,
/// so we don't query them here to maintain consistency between visible and non-visible processes.
/// Call this for visible processes only to minimize Windows API calls
/// Set fetch_exe_path=true only when show_program_path setting is enabled
#[cfg(windows)]
pub fn enrich_processes(processes: &mut [ProcessInfo], fetch_exe_path: bool) {
    use super::cache::{CACHE, config};
    use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE;

    // Clone only the visible PIDs' cache entries under a short read lock, instead
    // of cloning the entire cache map (`snapshot()`) every refresh. The lock is
    // released before the per-process syscalls below — those must NOT run while
    // holding it, or they would stall the collector thread's cache write lock.
    let cache_snapshot: HashMap<u32, super::cache::ProcessCacheEntry> = CACHE.with_read(|cache| {
        processes
            .iter()
            .filter_map(|p| cache.get(&p.pid).map(|e| (p.pid, e.clone())))
            .collect()
    });
    let now = std::time::Instant::now();

    // Query data sequentially - parallel overhead exceeds benefit for this workload
    let enriched_data: Vec<EnrichedProcessData> = processes
        .iter()
        .map(|p| {
            let pid = p.pid;
            if pid == 0 || pid == 4 {
                return EnrichedProcessData {
                    pid,
                    shared_mem: 0,
                    efficiency_mode: false,
                    efficiency_fresh: false,
                    arch_fresh: true,
                    exe_path_fresh: true,
                    elevation_fresh: true,
                    is_elevated: pid == 4, // System process is elevated
                    arch: ProcessArch::Native,
                    user: Some(Arc::from(SYSTEM_STR)),
                    exe_path: String::new(),
                };
            }

            // Get cached entry from unified snapshot
            let cached_entry = cache_snapshot.get(&pid);

            // Check each cached fact independently. A failed query for one field
            // must not cause unrelated cached facts to be ignored or overwritten
            // with fabricated defaults.
            let cached_elevation = cached_entry.and_then(|e| e.is_elevated);
            let cached_arch = cached_entry.and_then(|e| e.arch);
            let cached_exe_path = cached_entry.and_then(|e| e.exe_path.clone());
            let cached_user = cached_entry.and_then(|e| e.user.clone());

            // Check if efficiency cache is still valid (within TTL)
            let efficiency_valid = cached_entry
                .and_then(|e| e.efficiency_updated)
                .map(|updated| now.duration_since(updated).as_millis() < config::EFFICIENCY_TTL_MS)
                .unwrap_or(false);
            let cached_efficiency_mode = if efficiency_valid {
                cached_entry.and_then(|e| e.efficiency_mode)
            } else {
                None
            };

            // Determine what we need to query
            let need_arch = cached_arch.is_none();
            let need_elevation = cached_elevation.is_none();
            let need_user = cached_user.is_none();
            let need_efficiency = !efficiency_valid;
            let need_exe_path = fetch_exe_path
                && cached_exe_path
                    .as_ref()
                    .map(|p| p.is_empty())
                    .unwrap_or(true);

            // Skip OpenProcess entirely if we have all cached data and don't need times
            let need_handle =
                need_arch || need_elevation || need_user || need_efficiency || need_exe_path;

            let handle = if need_handle {
                open_process_query(pid)
            } else {
                None
            };

            // If we couldn't get a handle but need one, use cached data if available
            if need_handle && handle.is_none() {
                let user = cached_user;
                let efficiency_mode = cached_efficiency_mode.unwrap_or(false);

                return EnrichedProcessData {
                    pid,
                    shared_mem: 0,
                    efficiency_mode,
                    efficiency_fresh: false, // served from cache (no handle)
                    arch_fresh: false,
                    exe_path_fresh: false,
                    elevation_fresh: false,
                    is_elevated: cached_elevation.unwrap_or(false),
                    arch: cached_arch.unwrap_or(ProcessArch::Native),
                    user,
                    exe_path: cached_exe_path.unwrap_or_default(),
                };
            }

            // Shared memory comes from the native private working-set field.
            // PROCESS_MEMORY_COUNTERS_EX::PrivateUsage is private commit, not
            // private resident working set, so handle-based fallback would be
            // semantically wrong.
            let shared_mem = p.shared_mem;

            // Use cached exe_path or query if needed.
            let mut exe_path_fresh = false;
            let exe_path = if fetch_exe_path {
                if let Some(path) = cached_exe_path.as_ref().filter(|p| !p.is_empty()) {
                    path.clone()
                } else if let Some(h) = handle {
                    let path = query_exe_path(h);
                    exe_path_fresh = !path.is_empty();
                    path
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            // Use cached efficiency mode or query if stale
            let efficiency_mode = if let Some(mode) = cached_efficiency_mode {
                mode
            } else if let Some(h) = handle {
                unsafe {
                    let mut throttle_state = PROCESS_POWER_THROTTLING_STATE {
                        Version: 1,
                        ..Default::default()
                    };
                    let result = GetProcessInformation(
                        h,
                        ProcessPowerThrottling,
                        &mut throttle_state as *mut _ as *mut _,
                        std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
                    );
                    result.is_ok()
                        && (throttle_state.StateMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED)
                            != 0
                        && (throttle_state.ControlMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED)
                            != 0
                }
            } else {
                false
            };

            // Use cached architecture if available, otherwise query it. Query
            // failures return Native for the visible row but are not cached.
            let (arch, arch_fresh) = if let Some(arch) = cached_arch {
                (arch, false)
            } else if let Some(h) = handle {
                unsafe {
                    let mut process_machine = IMAGE_FILE_MACHINE::default();
                    let mut native_machine = IMAGE_FILE_MACHINE::default();
                    if IsWow64Process2(h, &mut process_machine, Some(&mut native_machine)).is_ok() {
                        if process_machine == IMAGE_FILE_MACHINE_I386 {
                            (ProcessArch::X86, true)
                        } else if process_machine == IMAGE_FILE_MACHINE_AMD64 {
                            (ProcessArch::X64, true)
                        } else if process_machine == IMAGE_FILE_MACHINE_ARM64 {
                            (ProcessArch::ARM64, true)
                        } else if process_machine.0 == 0 {
                            if native_machine == IMAGE_FILE_MACHINE_ARM64 {
                                let mut machine_info = PROCESS_MACHINE_INFORMATION::default();
                                if GetProcessInformation(
                                    h,
                                    ProcessMachineTypeInfo,
                                    &mut machine_info as *mut _ as *mut _,
                                    std::mem::size_of::<PROCESS_MACHINE_INFORMATION>() as u32,
                                )
                                .is_ok()
                                {
                                    if machine_info.ProcessMachine == IMAGE_FILE_MACHINE_AMD64 {
                                        (ProcessArch::X64, true)
                                    } else {
                                        (ProcessArch::Native, true)
                                    }
                                } else {
                                    (ProcessArch::Native, false)
                                }
                            } else {
                                (ProcessArch::Native, true)
                            }
                        } else {
                            (ProcessArch::Native, true)
                        }
                    } else {
                        (ProcessArch::Native, false)
                    }
                }
            } else {
                (ProcessArch::Native, false)
            };

            // Open token once for both elevation and user queries. Elevation is
            // cached only when TokenElevation itself succeeds.
            let (is_elevated, user, elevation_fresh) = if cached_elevation.is_some()
                && cached_user.is_some()
            {
                (
                    cached_elevation.unwrap_or(false),
                    cached_user.clone(),
                    false,
                )
            } else if let Some(h) = handle {
                unsafe {
                    let mut token_handle = HANDLE::default();
                    if OpenProcessToken(h, TOKEN_QUERY, &mut token_handle).is_ok() {
                        let (elevated, elevation_fresh) = if let Some(elevated) = cached_elevation {
                            (elevated, false)
                        } else {
                            let mut elevation = TOKEN_ELEVATION::default();
                            let mut return_length: u32 = 0;
                            let elev_result = GetTokenInformation(
                                token_handle,
                                TokenElevation,
                                Some(&mut elevation as *mut _ as *mut _),
                                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                                &mut return_length,
                            )
                            .is_ok();
                            (elev_result && elevation.TokenIsElevated != 0, elev_result)
                        };

                        let user = if cached_user.is_some() {
                            cached_user.clone()
                        } else {
                            get_user_from_token(token_handle, pid)
                        };

                        let _ = CloseHandle(token_handle);
                        (elevated, user, elevation_fresh)
                    } else {
                        (
                            cached_elevation.unwrap_or(false),
                            cached_user.clone(),
                            false,
                        )
                    }
                }
            } else {
                (
                    cached_elevation.unwrap_or(false),
                    cached_user.clone(),
                    false,
                )
            };

            if let Some(h) = handle {
                unsafe {
                    let _ = CloseHandle(h);
                }
            }

            EnrichedProcessData {
                pid,
                shared_mem,
                efficiency_mode,
                // Fresh only when we actually opened a handle and queried it.
                efficiency_fresh: need_efficiency && handle.is_some(),
                arch_fresh,
                exe_path_fresh,
                elevation_fresh,
                is_elevated,
                arch,
                user,
                exe_path,
            }
        })
        .collect();

    // Build lookup map once (used for both the cache update and the struct update
    // below). Previously the cache-update closure did a linear `find` per PID,
    // which was O(n^2) over the visible slice.
    let data_map: HashMap<u32, &EnrichedProcessData> =
        enriched_data.iter().map(|d| (d.pid, d)).collect();

    // Update unified cache with newly queried data (single lock acquisition)
    {
        let pids: Vec<u32> = enriched_data.iter().map(|d| d.pid).collect();
        CACHE.update_batch(&pids, |pid, entry| {
            if let Some(data) = data_map.get(&pid) {
                if data.arch_fresh {
                    entry.arch = Some(data.arch);
                }
                if data.exe_path_fresh {
                    entry.exe_path = Some(data.exe_path.clone());
                }
                if data.elevation_fresh {
                    entry.is_elevated = Some(data.is_elevated);
                }
                // Only refresh the efficiency TTL when it was actually re-queried;
                // bumping it on cache-served values would keep the 30s TTL from ever
                // expiring, freezing the indicator for continuously-visible rows.
                if data.efficiency_fresh {
                    entry.efficiency_mode = Some(data.efficiency_mode);
                    entry.efficiency_updated = Some(std::time::Instant::now());
                }
            }
        });
    }

    // Update process structs
    // IMPORTANT: cpu_time and start_time are already populated from NtQuerySystemInformation in from_native.
    // Do NOT overwrite them here - NtQuerySystemInformation provides authoritative data for ALL processes,
    // while enrich_processes only runs on VISIBLE processes. Overwriting would create inconsistency
    // where visible processes have different (GetProcessTimes) values than non-visible ones (NtQuerySystemInformation).
    // This caused a bug where rows appeared to "stop updating" when sorting changed which processes were visible.
    for proc in processes.iter_mut() {
        if let Some(data) = data_map.get(&proc.pid) {
            // Only update shared_mem if we got valid data. The raw process pass
            // already computes this from private working set; this fallback is
            // for entries that still have zero because the kernel value was not
            // available.
            if data.shared_mem != 0 {
                proc.shared_mem = data.shared_mem;
            }
            proc.efficiency_mode = data.efficiency_mode;
            proc.is_elevated = data.is_elevated;
            proc.arch = data.arch;
            if let Some(ref user) = data.user
                && *user != proc.user
            {
                proc.user = user.clone();
                proc.user_lower = user.to_lowercase().into();
            }
            // Update exe_path and command if we got a valid path
            if !data.exe_path.is_empty() && data.exe_path != *proc.exe_path {
                // Share one allocation between exe_path and command (refcounted).
                let path: Arc<str> = Arc::from(data.exe_path.clone());
                proc.exe_path = path.clone();
                proc.command = path;
                proc.command_lower = data.exe_path.to_lowercase().into();
            }
        }
    }
}

#[cfg(not(windows))]
pub fn enrich_processes(_processes: &mut [ProcessInfo], _fetch_exe_path: bool) {
    // No-op on non-Windows
}

#[cfg(not(windows))]
struct WinProcessInfo {
    priority: i32,
    cpu_time: Duration,
    start_time: u64,
    handle_count: u32,
    io_read_bytes: u64,
    io_write_bytes: u64,
    shared_mem: u64,
    efficiency_mode: bool,
    is_elevated: bool,
    arch: ProcessArch,
    user: Option<String>,
}

#[cfg(not(windows))]
fn get_win_process_info(_pid: u32) -> WinProcessInfo {
    WinProcessInfo {
        priority: 20,
        cpu_time: Duration::ZERO,
        start_time: 0,
        handle_count: 0,
        io_read_bytes: 0,
        io_write_bytes: 0,
        shared_mem: 0,
        efficiency_mode: false,
        is_elevated: false,
        arch: ProcessArch::Native,
        user: None,
    }
}

/// Process information
#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: u32,
    pub name: Arc<str>,
    pub exe_path: Arc<str>, // Full executable path
    pub command: Arc<str>,  // Full command line with arguments
    pub user: Arc<str>,
    pub status: char,
    pub cpu_percent: f32,
    pub mem_percent: f32,
    pub virtual_mem: u64,
    pub resident_mem: u64,
    pub shared_mem: u64,
    pub priority: i32,
    pub cpu_time: Duration,
    pub tree_depth: usize,
    pub tree_prefix: String, // Tree display prefix (├─, └─, │, etc.)
    // New fields for extended features
    pub has_children: bool,     // Has child processes (for tree view)
    pub is_collapsed: bool,     // Is collapsed in tree view
    pub thread_count: u32,      // Number of threads
    pub start_time: u64,        // Process start time (Unix timestamp)
    pub create_time_100ns: u64, // Raw Windows FILETIME process creation timestamp
    pub handle_count: u32,      // Number of handles (Windows)
    pub io_read_bytes: u64,     // I/O bytes read (cumulative)
    pub io_write_bytes: u64,    // I/O bytes written (cumulative)
    pub io_read_rate: u64,      // I/O read bytes per second
    pub io_write_rate: u64,     // I/O write bytes per second
    pub gpu_percent: f32,       // GPU utilization (max across all GPU engine nodes)
    pub gpu_memory: u64,        // GPU committed bytes across all GPU adapters
    pub npu_percent: f32,       // NPU utilization (max across NPU engine nodes)
    pub npu_memory: u64,        // NPU dedicated + shared committed bytes
    // Pre-computed lowercase strings for efficient filtering (avoid per-filter allocations)
    pub name_lower: Arc<str>,
    pub command_lower: Arc<str>,
    pub user_lower: Arc<str>,
    // Pre-computed search match flag (set during filtering, used in rendering)
    pub matches_search: bool,
    // Windows 11 Efficiency Mode (EcoQoS)
    pub efficiency_mode: bool,
    // Running as administrator
    pub is_elevated: bool,
    // Process architecture (x86/x64/ARM64)
    pub arch: ProcessArch,
    // Executable was modified after process started (like htop's red basename)
    pub exe_updated: bool,
    // Executable no longer exists at the original path
    pub exe_deleted: bool,
}

impl ProcessInfo {
    /// Format CPU time as HH:MM:SS or MM:SS.ms
    pub fn format_cpu_time(&self) -> String {
        let secs = self.cpu_time.as_secs();
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        let secs = secs % 60;
        let centis = self.cpu_time.subsec_millis() / 10;

        if hours > 0 {
            format!("{:02}:{:02}:{:02}", hours, mins, secs)
        } else {
            format!("{:02}:{:02}.{:02}", mins, secs, centis)
        }
    }

    /// Update existing ProcessInfo from raw SystemProcess (avoids reallocation)
    #[cfg(windows)]
    pub fn update_from_raw(&mut self, proc: &SystemProcess, cpu_percent: f32, total_mem: u64) {
        self.cpu_percent = cpu_percent;
        self.mem_percent = if total_mem > 0 {
            (proc.working_set() as f64 / total_mem as f64 * 100.0) as f32
        } else {
            0.0
        };

        self.virtual_mem = proc.virtual_size();
        self.resident_mem = proc.working_set();
        self.shared_mem = proc
            .working_set()
            .saturating_sub(proc.private_working_set());
        self.priority = proc.base_priority();
        self.thread_count = proc.thread_count();
        self.handle_count = proc.handle_count();
        self.io_read_bytes = proc.read_bytes();
        self.io_write_bytes = proc.write_bytes();

        self.parent_pid = proc.parent_pid();

        let total_100ns = proc.kernel_time() + proc.user_time();
        self.cpu_time = Duration::new(
            total_100ns / 10_000_000,
            ((total_100ns % 10_000_000) * 100) as u32,
        );

        self.create_time_100ns = proc.create_time();
        self.start_time = filetime_to_unix(self.create_time_100ns);

        let (exe_updated, exe_deleted) = check_exe_status(&self.exe_path, self.create_time_100ns);
        self.exe_updated = exe_updated;
        self.exe_deleted = exe_deleted;
    }

    /// Create ProcessInfo from raw SystemProcess
    #[cfg(windows)]
    pub fn from_raw(proc: &SystemProcess, cpu_percent: f32, total_mem: u64) -> Self {
        use super::cache::{CACHE, config};

        let pid = proc.pid();
        let now = std::time::Instant::now();

        // Read only this PID's cached fields under a short read lock, instead of
        // cloning the entire cache map (`snapshot()`) just to look up one entry.
        // from_raw runs once per *new* process, so the old full-map clone was
        // O(processes^2) string-cloning on boot / mass-spawn.
        let (is_elevated, arch, cached_exe_path, efficiency_mode, user) =
            CACHE.with_read(|cache| {
                let cached_entry = cache.get(&pid);

                let is_elevated = cached_entry.and_then(|e| e.is_elevated).unwrap_or(false);
                let arch = cached_entry
                    .and_then(|e| e.arch)
                    .unwrap_or(ProcessArch::Native);
                let cached_exe_path = cached_entry
                    .and_then(|e| e.exe_path.clone())
                    .unwrap_or_default();

                let efficiency_mode = cached_entry
                    .and_then(|e| {
                        if let (Some(mode), Some(updated)) =
                            (e.efficiency_mode, e.efficiency_updated)
                            && now.duration_since(updated).as_millis() < config::EFFICIENCY_TTL_MS
                        {
                            return Some(mode);
                        }
                        None
                    })
                    .unwrap_or(false);

                let user: Arc<str> =
                    cached_entry
                        .and_then(|e| e.user.clone())
                        .unwrap_or_else(|| {
                            if pid == 0 || pid == 4 {
                                Arc::from(SYSTEM_STR)
                            } else {
                                Arc::from("-")
                            }
                        });

                (is_elevated, arch, cached_exe_path, efficiency_mode, user)
            });

        let mem_percent = if total_mem > 0 {
            (proc.working_set() as f64 / total_mem as f64 * 100.0) as f32
        } else {
            0.0
        };

        let total_100ns = proc.kernel_time() + proc.user_time();
        let cpu_time = Duration::new(
            total_100ns / 10_000_000,
            ((total_100ns % 10_000_000) * 100) as u32,
        );

        let create_time_100ns = proc.create_time();
        let start_time = filetime_to_unix(create_time_100ns);

        // Parse name only here (allocation)
        let name: Arc<str> = proc.name().into();
        let name_lower: Arc<str> = name.to_lowercase().into();

        let (exe_path, command, command_lower): (Arc<str>, Arc<str>, Arc<str>) =
            if !cached_exe_path.is_empty() {
                let lower: Arc<str> = cached_exe_path.to_lowercase().into();
                // Single allocation shared between exe_path and command (refcounted).
                let path: Arc<str> = Arc::from(cached_exe_path);
                (path.clone(), path, lower)
            } else {
                // No exe path: command falls back to the (already-owned) name.
                (Arc::from(""), name.clone(), name_lower.clone())
            };

        let (exe_updated, exe_deleted) = check_exe_status(&exe_path, create_time_100ns);

        let user_lower: Arc<str> = user.to_lowercase().into();

        ProcessInfo {
            pid,
            parent_pid: proc.parent_pid(),
            name,
            exe_path,
            command,
            user,
            status: '?',
            cpu_percent,
            mem_percent,
            virtual_mem: proc.virtual_size(),
            resident_mem: proc.working_set(),
            shared_mem: proc
                .working_set()
                .saturating_sub(proc.private_working_set()),
            priority: proc.base_priority(),
            cpu_time,
            tree_depth: 0,
            tree_prefix: String::new(),
            has_children: false,
            is_collapsed: false,
            thread_count: proc.thread_count(),
            start_time,
            create_time_100ns,
            handle_count: proc.handle_count(),
            io_read_bytes: proc.read_bytes(),
            io_write_bytes: proc.write_bytes(),
            io_read_rate: 0,
            io_write_rate: 0,
            gpu_percent: 0.0,
            gpu_memory: 0,
            npu_percent: 0.0,
            npu_memory: 0,
            name_lower,
            command_lower,
            user_lower,
            matches_search: false,
            efficiency_mode,
            is_elevated,
            arch,
            exe_updated,
            exe_deleted,
        }
    }
}

/// Kill a process by PID
#[cfg(windows)]
pub fn kill_process(pid: u32, _signal: u32) -> Result<(), String> {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, false, pid)
            .map_err(|e| format!("Cannot open process: {}", e))?;

        if handle.is_invalid() {
            return Err(format!(
                "Cannot open process {} (access denied or not found)",
                pid
            ));
        }

        let result = TerminateProcess(handle, 1);
        let _ = CloseHandle(handle);

        result.map_err(|e| format!("Cannot terminate: {}", e))?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn kill_process(pid: u32, signal: u32) -> Result<(), String> {
    use std::process::Command;
    let sig = match signal {
        9 => "KILL",
        15 => "TERM",
        _ => "TERM",
    };
    Command::new("kill")
        .args(["-s", sig, &pid.to_string()])
        .output()
        .map_err(|e| format!("Failed to kill process: {}", e))?;
    Ok(())
}

/// Set process priority class directly
#[cfg(windows)]
pub fn set_priority_class(
    pid: u32,
    priority: crate::app::WindowsPriorityClass,
) -> Result<(), String> {
    use crate::app::WindowsPriorityClass;

    unsafe {
        let handle = OpenProcess(PROCESS_SET_INFORMATION, false, pid)
            .map_err(|e| format!("Cannot open process: {}", e))?;

        if handle.is_invalid() {
            return Err(format!("Cannot open process {} (access denied)", pid));
        }

        // Map WindowsPriorityClass to Windows API constant
        let priority_class = match priority {
            WindowsPriorityClass::Idle => IDLE_PRIORITY_CLASS,
            WindowsPriorityClass::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
            WindowsPriorityClass::Normal => NORMAL_PRIORITY_CLASS,
            WindowsPriorityClass::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
            WindowsPriorityClass::High => HIGH_PRIORITY_CLASS,
            WindowsPriorityClass::Realtime => REALTIME_PRIORITY_CLASS,
        };

        let result = SetPriorityClass(handle, priority_class);
        let _ = CloseHandle(handle);

        result.map_err(|e| format!("Cannot set priority: {}", e))?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn set_priority_class(
    pid: u32,
    priority: crate::app::WindowsPriorityClass,
) -> Result<(), String> {
    use crate::app::WindowsPriorityClass;
    use std::process::Command;

    // Map to nice value for Unix
    let nice = match priority {
        WindowsPriorityClass::Idle => 19,
        WindowsPriorityClass::BelowNormal => 10,
        WindowsPriorityClass::Normal => 0,
        WindowsPriorityClass::AboveNormal => -5,
        WindowsPriorityClass::High => -10,
        WindowsPriorityClass::Realtime => -20,
    };

    Command::new("renice")
        .args([&nice.to_string(), "-p", &pid.to_string()])
        .output()
        .map_err(|e| format!("Failed to set priority: {}", e))?;
    Ok(())
}

/// Set process efficiency mode (EcoQoS power throttling)
#[cfg(windows)]
pub fn set_efficiency_mode(pid: u32, enabled: bool) -> Result<(), String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
        PROCESS_SET_INFORMATION, ProcessPowerThrottling, SetProcessInformation,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_SET_INFORMATION, false, pid)
            .map_err(|e| format!("Cannot open process: {}", e))?;

        if handle.is_invalid() {
            return Err(format!("Cannot open process {} (access denied)", pid));
        }

        // Use both throttling flags like the working implementation in main.rs
        let control_mask = PROCESS_POWER_THROTTLING_EXECUTION_SPEED
            | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION;

        // StateMask: same as ControlMask to enable, default (0) to disable
        let mut throttle_state: PROCESS_POWER_THROTTLING_STATE = std::mem::zeroed();
        throttle_state.Version = 1;
        throttle_state.ControlMask = control_mask;
        if enabled {
            throttle_state.StateMask = control_mask;
        }

        let result = SetProcessInformation(
            handle,
            ProcessPowerThrottling,
            &mut throttle_state as *mut _ as *mut _,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );

        let _ = CloseHandle(handle);
        result.map_err(|e| format!("Cannot set efficiency mode: {}", e))?;
    }

    // Update the cache immediately so UI reflects the change
    update_efficiency_mode_cache(pid, enabled);

    Ok(())
}

/// Update the efficiency mode cache for a specific PID
#[cfg(windows)]
fn update_efficiency_mode_cache(pid: u32, enabled: bool) {
    super::cache::CACHE.set_efficiency_mode(pid, enabled);
}

#[cfg(not(windows))]
pub fn set_efficiency_mode(_pid: u32, _enabled: bool) -> Result<(), String> {
    Err("Efficiency mode is only available on Windows".to_string())
}

/// Get process CPU affinity mask
#[cfg(windows)]
pub fn get_process_affinity(pid: u32) -> Result<u64, String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        GetProcessAffinityMask, OpenProcess, PROCESS_QUERY_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid)
            .map_err(|e| format!("Cannot open process: {}", e))?;
        let mut process_mask: usize = 0;
        let mut system_mask: usize = 0;
        let result = GetProcessAffinityMask(handle, &mut process_mask, &mut system_mask);
        let _ = CloseHandle(handle);
        result.map_err(|e| format!("Cannot get affinity: {}", e))?;
        Ok(process_mask as u64)
    }
}

#[cfg(not(windows))]
pub fn get_process_affinity(_pid: u32) -> Result<u64, String> {
    // Not implemented for non-Windows
    Ok(u64::MAX)
}

/// Set process CPU affinity mask
#[cfg(windows)]
pub fn set_process_affinity(pid: u32, mask: u64) -> Result<(), String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_INFORMATION, SetProcessAffinityMask,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_SET_INFORMATION, false, pid)
            .map_err(|e| format!("Cannot open process: {}", e))?;
        let result = SetProcessAffinityMask(handle, mask as usize);
        let _ = CloseHandle(handle);
        result.map_err(|e| format!("Cannot set affinity: {}", e))?;
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn set_process_affinity(_pid: u32, _mask: u64) -> Result<(), String> {
    // Not implemented for non-Windows
    Ok(())
}
