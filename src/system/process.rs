use std::collections::HashMap;
#[cfg(windows)]
use std::sync::RwLock;
use std::time::Duration;
use rayon::prelude::*;

#[cfg(windows)]
use super::native::{NativeProcessInfo, filetime_to_unix, priority_to_nice};

#[cfg(windows)]
use std::sync::LazyLock;

#[cfg(windows)]
use windows::core::PWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
#[cfg(windows)]
use windows::Win32::Security::{
    AdjustTokenPrivileges, GetTokenInformation, LookupAccountSidW, LookupPrivilegeValueW,
    TokenElevation, TokenUser, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, SID_NAME_USE,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY, TOKEN_USER,
};
#[cfg(windows)]
use windows::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
#[cfg(windows)]
use windows::Win32::System::Threading::IO_COUNTERS;
#[cfg(windows)]
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessInformation, GetProcessIoCounters, GetProcessTimes,
    IsWow64Process2, OpenProcess, OpenProcessToken, ProcessPowerThrottling,
    QueryFullProcessImageNameW, SetPriorityClass, TerminateProcess,
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, HIGH_PRIORITY_CLASS,
    IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_NAME_WIN32,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
    PROCESS_TERMINATE, REALTIME_PRIORITY_CLASS,
};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
};

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
        ).is_err() {
            return false;
        }

        let mut luid = windows::Win32::Foundation::LUID::default();
        // SE_DEBUG_NAME = "SeDebugPrivilege"
        if LookupPrivilegeValueW(None, w!("SeDebugPrivilege"), &mut luid).is_err() {
            let _ = CloseHandle(token);
            return false;
        }

        let mut tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };

        let result = AdjustTokenPrivileges(token, false, Some(&mut tp), 0, None, None).is_ok();
        let _ = CloseHandle(token);
        result
    }
}

#[cfg(not(windows))]
pub fn enable_debug_privilege() -> bool {
    false
}

// Cache for PID to username lookups (persists across refreshes)
#[cfg(windows)]
static PID_USER_CACHE: LazyLock<RwLock<HashMap<u32, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

// Common usernames as UTF-16 for fast comparison (avoids UTF-16 to UTF-8 conversion)
#[cfg(windows)]
const SYSTEM_UTF16: [u16; 6] = [0x53, 0x59, 0x53, 0x54, 0x45, 0x4D]; // "SYSTEM"
#[cfg(windows)]
const LOCAL_SERVICE_UTF16: [u16; 13] = [0x4C, 0x4F, 0x43, 0x41, 0x4C, 0x20, 0x53, 0x45, 0x52, 0x56, 0x49, 0x43, 0x45]; // "LOCAL SERVICE"
#[cfg(windows)]
const NETWORK_SERVICE_UTF16: [u16; 15] = [0x4E, 0x45, 0x54, 0x57, 0x4F, 0x52, 0x4B, 0x20, 0x53, 0x45, 0x52, 0x56, 0x49, 0x43, 0x45]; // "NETWORK SERVICE"

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
fn intern_username_utf16(name: &[u16]) -> String {
    // Fast path: check against known common usernames (avoids UTF-16 conversion)
    if name == SYSTEM_UTF16 {
        return SYSTEM_STR.to_string();
    }
    if name == LOCAL_SERVICE_UTF16 {
        return LOCAL_SERVICE_STR.to_string();
    }
    if name == NETWORK_SERVICE_UTF16 {
        return NETWORK_SERVICE_STR.to_string();
    }
    // Fallback: convert from UTF-16
    String::from_utf16_lossy(name)
}

// Cache for static process info (elevation, architecture, exe_path) - these don't change during process lifetime
#[cfg(windows)]
static STATIC_PROCESS_INFO_CACHE: LazyLock<RwLock<HashMap<u32, (bool, ProcessArch, String)>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

// Cache for efficiency_mode (requires Windows API call, not available from NtQuerySystemInformation)
#[cfg(windows)]
struct EfficiencyModeCache {
    efficiency_mode: bool,
    last_update: std::time::Instant,
}

#[cfg(windows)]
static EFFICIENCY_MODE_CACHE: LazyLock<RwLock<HashMap<u32, EfficiencyModeCache>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

#[cfg(windows)]
const EFFICIENCY_CACHE_TTL_MS: u128 = 30000; // Refresh efficiency mode every 30 seconds

// Counter for periodic cache cleanup (every N refreshes)
#[cfg(windows)]
static CLEANUP_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
#[cfg(windows)]
const CLEANUP_INTERVAL: u32 = 10; // Clean caches every 10 refreshes

/// Clean up caches by removing entries for PIDs that no longer exist
#[cfg(windows)]
fn cleanup_stale_caches(current_pids: &std::collections::HashSet<u32>) {
    if let Ok(mut cache) = PID_USER_CACHE.write() {
        cache.retain(|pid, _| current_pids.contains(pid));
    }
    if let Ok(mut cache) = STATIC_PROCESS_INFO_CACHE.write() {
        cache.retain(|pid, _| current_pids.contains(pid));
    }
    if let Ok(mut cache) = EFFICIENCY_MODE_CACHE.write() {
        cache.retain(|pid, _| current_pids.contains(pid));
    }
    // Also clean up CPU time cache in native module
    super::cleanup_cpu_time_cache(current_pids);
}

/// Process architecture
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProcessArch {
    #[default]
    Native,   // Native architecture (matches OS)
    X86,      // 32-bit x86 (WoW64 on x64/ARM64)
    X64,      // x64 running on ARM64 via emulation
    ARM64,    // Native ARM64
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
fn open_process_query(pid: u32) -> Option<HANDLE> {
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

/// Query CPU time and start time from a process handle
#[cfg(windows)]
#[inline]
fn query_process_times(handle: HANDLE) -> (Duration, u64) {
    unsafe {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();

        if GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user).is_ok() {
            let kernel_100ns = ((kernel.dwHighDateTime as u64) << 32) | kernel.dwLowDateTime as u64;
            let user_100ns = ((user.dwHighDateTime as u64) << 32) | user.dwLowDateTime as u64;
            let total_100ns = kernel_100ns + user_100ns;
            let cpu_time = Duration::new(total_100ns / 10_000_000, ((total_100ns % 10_000_000) * 100) as u32);

            // Convert FILETIME to Unix timestamp (116444736000000000 = difference between 1601 and 1970)
            let creation_100ns = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
            let start_time = creation_100ns.saturating_sub(116444736000000000) / 10_000_000;

            (cpu_time, start_time)
        } else {
            (Duration::ZERO, 0)
        }
    }
}

/// Query shared memory (WorkingSetSize - PrivateUsage) from a process handle
#[cfg(windows)]
#[inline]
fn query_shared_mem(handle: HANDLE) -> u64 {
    unsafe {
        let mut mem = PROCESS_MEMORY_COUNTERS_EX::default();
        mem.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        if K32GetProcessMemoryInfo(handle, &mut mem as *mut _ as *mut PROCESS_MEMORY_COUNTERS, mem.cb).as_bool() {
            (mem.WorkingSetSize as u64).saturating_sub(mem.PrivateUsage as u64)
        } else {
            0
        }
    }
}

/// Query the full executable path from a process handle
#[cfg(windows)]
#[inline]
fn query_exe_path(handle: HANDLE) -> String {
    unsafe {
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        if QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size).is_ok() {
            String::from_utf16_lossy(&buffer[..size as usize])
        } else {
            String::new()
        }
    }
}

/// Extract username from an already-opened token handle (avoids duplicate OpenProcess)
#[cfg(windows)]
fn get_user_from_token(token_handle: HANDLE, pid: u32) -> Option<String> {
    unsafe {
        // Get token user info - first call to get required size
        let mut token_info_len: u32 = 0;
        let _ = GetTokenInformation(token_handle, TokenUser, None, 0, &mut token_info_len);

        if token_info_len == 0 {
            return None;
        }

        // Allocate buffer and get token info
        let mut token_info: Vec<u8> = vec![0; token_info_len as usize];
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

            // Cache the result
            if let Ok(mut cache) = PID_USER_CACHE.write() {
                cache.insert(pid, username.clone());
            }

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
        } else { (0, 0) };
        let _ = CloseHandle(handle);
        result
    }
}

#[cfg(not(windows))]
pub fn get_process_io_counters(_pid: u32) -> (u64, u64) {
    (0, 0)
}

/// Enriched data from Windows API for visible processes
#[cfg(windows)]
struct EnrichedProcessData {
    pid: u32,
    cpu_time: Duration,
    start_time: u64,
    shared_mem: u64,
    efficiency_mode: bool,
    is_elevated: bool,
    arch: ProcessArch,
    user: Option<String>,
    exe_path: String,
}

/// Enrich processes with data not available from NtQuerySystemInformation
/// (cpu_time, start_time, shared_mem, efficiency_mode, is_elevated, arch, user, exe_path)
/// Call this for visible processes only to minimize Windows API calls
/// Set fetch_exe_path=true only when show_program_path setting is enabled
#[cfg(windows)]
pub fn enrich_processes(processes: &mut [ProcessInfo], fetch_exe_path: bool) {
    use rayon::prelude::*;
    use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE;

    // Pre-read caches to avoid lock contention in parallel loop
    let static_cache_snapshot: HashMap<u32, (bool, ProcessArch, String)> = STATIC_PROCESS_INFO_CACHE
        .read()
        .map(|c| c.clone())
        .unwrap_or_default();
    let user_cache_snapshot: HashMap<u32, String> = PID_USER_CACHE
        .read()
        .map(|c| c.clone())
        .unwrap_or_default();
    let efficiency_cache_snapshot: HashMap<u32, EfficiencyModeCache> = EFFICIENCY_MODE_CACHE
        .read()
        .map(|c| c.iter().map(|(&k, v)| (k, EfficiencyModeCache {
            efficiency_mode: v.efficiency_mode,
            last_update: v.last_update,
        })).collect())
        .unwrap_or_default();
    let now = std::time::Instant::now();

    // Query data in parallel
    let enriched_data: Vec<EnrichedProcessData> = processes
        .par_iter()
        .map(|p| {
            let pid = p.pid;
            if pid == 0 || pid == 4 {
                return EnrichedProcessData {
                    pid,
                    cpu_time: Duration::ZERO,
                    start_time: 0,
                    shared_mem: 0,
                    efficiency_mode: false,
                    is_elevated: pid == 4,  // System process is elevated
                    arch: ProcessArch::Native,
                    user: Some(SYSTEM_STR.to_string()),
                    exe_path: String::new(),
                };
            }

            // Check static cache for elevation, arch, exe_path (these never change)
            let cached_static = static_cache_snapshot.get(&pid);
            let cached_user = user_cache_snapshot.get(&pid);
            let cached_efficiency = efficiency_cache_snapshot.get(&pid);

            // Check if efficiency cache is still valid (within TTL)
            let efficiency_valid = cached_efficiency
                .map(|c| now.duration_since(c.last_update).as_millis() < EFFICIENCY_CACHE_TTL_MS)
                .unwrap_or(false);

            // Determine what we need to query
            let need_static = cached_static.is_none();
            let need_user = cached_user.is_none();
            let need_efficiency = !efficiency_valid;
            let need_exe_path = fetch_exe_path && cached_static.map(|(_, _, p)| p.is_empty()).unwrap_or(true);

            // Skip OpenProcess entirely if we have all cached data and don't need times
            let need_handle = need_static || need_user || need_efficiency || need_exe_path;

            let handle = if need_handle {
                open_process_query(pid)
            } else {
                None
            };

            // If we couldn't get a handle but need one, use cached data if available
            if need_handle && handle.is_none() {
                let (is_elevated, arch, exe_path) = cached_static
                    .map(|(e, a, p)| (*e, *a, p.clone()))
                    .unwrap_or((false, ProcessArch::Native, String::new()));
                let user = cached_user.cloned();
                let efficiency_mode = cached_efficiency.map(|c| c.efficiency_mode).unwrap_or(false);

                return EnrichedProcessData {
                    pid,
                    cpu_time: Duration::ZERO,
                    start_time: 0,
                    shared_mem: 0,
                    efficiency_mode,
                    is_elevated,
                    arch,
                    user,
                    exe_path,
                };
            }

            // Query times and memory (always needed for TIME+ accuracy)
            let (cpu_time, start_time, shared_mem) = if let Some(h) = handle {
                let (ct, st) = query_process_times(h);
                let sm = query_shared_mem(h);
                (ct, st, sm)
            } else {
                (Duration::ZERO, 0, 0)
            };

            // Use cached exe_path or query if needed
            let exe_path = if fetch_exe_path {
                if let Some((_, _, path)) = cached_static {
                    if !path.is_empty() {
                        path.clone()
                    } else if let Some(h) = handle {
                        query_exe_path(h)
                    } else {
                        String::new()
                    }
                } else if let Some(h) = handle {
                    query_exe_path(h)
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            // Use cached efficiency mode or query if stale
            let efficiency_mode = if efficiency_valid {
                cached_efficiency.unwrap().efficiency_mode
            } else if let Some(h) = handle {
                unsafe {
                    let mut throttle_state = PROCESS_POWER_THROTTLING_STATE::default();
                    throttle_state.Version = 1;
                    let result = GetProcessInformation(
                        h, ProcessPowerThrottling,
                        &mut throttle_state as *mut _ as *mut _,
                        std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
                    );
                    result.is_ok()
                        && (throttle_state.StateMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED) != 0
                        && (throttle_state.ControlMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED) != 0
                }
            } else {
                false
            };

            // Use cached elevation/arch if available, otherwise query
            let (is_elevated, arch) = if let Some(&(elevated, arch, _)) = cached_static {
                (elevated, arch)
            } else if let Some(h) = handle {
                // Query elevation from token
                let elevated = unsafe {
                    let mut token_handle = HANDLE::default();
                    if OpenProcessToken(h, TOKEN_QUERY, &mut token_handle).is_ok() {
                        let mut elevation = TOKEN_ELEVATION::default();
                        let mut return_length: u32 = 0;
                        let elev = GetTokenInformation(
                            token_handle,
                            TokenElevation,
                            Some(&mut elevation as *mut _ as *mut _),
                            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                            &mut return_length,
                        ).is_ok() && elevation.TokenIsElevated != 0;
                        let _ = CloseHandle(token_handle);
                        elev
                    } else {
                        false
                    }
                };

                // Query architecture
                let arch = unsafe {
                    let mut process_machine = IMAGE_FILE_MACHINE::default();
                    let mut native_machine = IMAGE_FILE_MACHINE::default();
                    if IsWow64Process2(h, &mut process_machine, Some(&mut native_machine)).is_ok() {
                        if process_machine.0 == 0 {
                            ProcessArch::Native
                        } else if process_machine == IMAGE_FILE_MACHINE_I386 {
                            ProcessArch::X86
                        } else if process_machine == IMAGE_FILE_MACHINE_AMD64 {
                            ProcessArch::X64
                        } else if process_machine == IMAGE_FILE_MACHINE_ARM64 {
                            ProcessArch::ARM64
                        } else {
                            ProcessArch::Native
                        }
                    } else {
                        ProcessArch::Native
                    }
                };

                (elevated, arch)
            } else {
                (false, ProcessArch::Native)
            };

            // Get user from cache or query from token (separate from elevation/arch caching)
            let user = if let Some(u) = cached_user {
                Some(u.clone())
            } else if let Some(h) = handle {
                unsafe {
                    let mut token_handle = HANDLE::default();
                    if OpenProcessToken(h, TOKEN_QUERY, &mut token_handle).is_ok() {
                        let u = get_user_from_token(token_handle, pid);
                        let _ = CloseHandle(token_handle);
                        u
                    } else {
                        None
                    }
                }
            } else {
                None
            };

            if let Some(h) = handle {
                unsafe { let _ = CloseHandle(h); }
            }

            EnrichedProcessData {
                pid,
                cpu_time,
                start_time,
                shared_mem,
                efficiency_mode,
                is_elevated,
                arch,
                user,
                exe_path,
            }
        })
        .collect();

    // Update caches with newly queried data
    if let Ok(mut cache) = STATIC_PROCESS_INFO_CACHE.write() {
        for data in &enriched_data {
            cache.insert(data.pid, (data.is_elevated, data.arch, data.exe_path.clone()));
        }
    }

    // Cache efficiency_mode (requires Windows API call, not available from NtQuerySystemInformation)
    if let Ok(mut cache) = EFFICIENCY_MODE_CACHE.write() {
        let now = std::time::Instant::now();
        for data in &enriched_data {
            cache.insert(data.pid, EfficiencyModeCache {
                efficiency_mode: data.efficiency_mode,
                last_update: now,
            });
        }
    }

    // Build lookup map
    let data_map: HashMap<u32, &EnrichedProcessData> = enriched_data
        .iter()
        .map(|d| (d.pid, d))
        .collect();

    // Update process structs
    // IMPORTANT: Only overwrite fields if enrichment got valid data.
    // cpu_time and start_time are already populated from NtQuerySystemInformation in from_native.
    // Don't overwrite them with zeros when OpenProcess fails (access denied).
    for proc in processes.iter_mut() {
        if let Some(data) = data_map.get(&proc.pid) {
            // Only update cpu_time/start_time if we got actual data (not defaults from failed OpenProcess)
            if !data.cpu_time.is_zero() {
                proc.cpu_time = data.cpu_time;
            }
            if data.start_time != 0 {
                proc.start_time = data.start_time;
            }
            if data.shared_mem != 0 {
                proc.shared_mem = data.shared_mem;
            }
            proc.efficiency_mode = data.efficiency_mode;
            proc.is_elevated = data.is_elevated;
            proc.arch = data.arch;
            if let Some(ref user) = data.user {
                proc.user = user.clone();
                proc.user_lower = user.to_lowercase();
            }
            // Update exe_path and command if we got a valid path
            if !data.exe_path.is_empty() {
                proc.exe_path = data.exe_path.clone();
                proc.command = data.exe_path.clone();
                proc.command_lower = data.exe_path.to_lowercase();
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
    nice: i32,
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
        nice: 0,
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
    pub name: String,
    pub exe_path: String, // Full executable path
    pub command: String,  // Full command line with arguments
    pub user: String,
    pub status: char,
    pub cpu_percent: f32,
    pub mem_percent: f32,
    pub virtual_mem: u64,
    pub resident_mem: u64,
    pub shared_mem: u64,
    pub priority: i32,
    pub nice: i32,
    pub cpu_time: Duration,
    pub tree_depth: usize,
    pub tree_prefix: String, // Tree display prefix (├─, └─, │, etc.)
    // New fields for extended features
    pub has_children: bool,  // Has child processes (for tree view)
    pub is_collapsed: bool,  // Is collapsed in tree view
    pub thread_count: u32,   // Number of threads
    pub start_time: u64,     // Process start time (Unix timestamp)
    pub handle_count: u32,   // Number of handles (Windows)
    pub io_read_bytes: u64,  // I/O bytes read
    pub io_write_bytes: u64, // I/O bytes written
    // Pre-computed lowercase strings for efficient filtering (avoid per-filter allocations)
    pub name_lower: String,
    pub command_lower: String,
    pub user_lower: String,
    // Pre-computed search match flag (set during filtering, used in rendering)
    pub matches_search: bool,
    // Windows 11 Efficiency Mode (EcoQoS)
    pub efficiency_mode: bool,
    // Running as administrator
    pub is_elevated: bool,
    // Process architecture (x86/x64/ARM64)
    pub arch: ProcessArch,
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

    /// Create ProcessInfo from native NtQuerySystemInformation data
    /// This is significantly faster than sysinfo as it uses a single syscall for all processes.
    /// Fields not available from NT API (exe_path, command, user, efficiency_mode, is_elevated, arch)
    /// use cached values or defaults - call enrich_processes() for visible processes to fill them.
    #[cfg(windows)]
    pub fn from_native(
        native_procs: &[NativeProcessInfo],
        cpu_percentages: &HashMap<u32, f32>,
        total_mem: u64,
    ) -> Vec<ProcessInfo> {
        // Periodically clean up stale PIDs from caches to prevent unbounded growth
        {
            use std::sync::atomic::Ordering;
            if CLEANUP_COUNTER.fetch_add(1, Ordering::Relaxed) % CLEANUP_INTERVAL == 0 {
                let current_pids: std::collections::HashSet<u32> =
                    native_procs.iter().map(|p| p.pid).collect();
                cleanup_stale_caches(&current_pids);
            }
        }

        native_procs
            .par_iter()
            .map(|proc| {
                let pid = proc.pid;

                // Get cached static info (is_elevated, arch, exe_path) if available
                let (is_elevated, arch, cached_exe_path) = STATIC_PROCESS_INFO_CACHE
                    .read()
                    .ok()
                    .and_then(|cache| cache.get(&pid).cloned())
                    .unwrap_or((false, ProcessArch::Native, String::new()));

                // Always use native data for priority/nice/handle_count
                // These come directly from NtQuerySystemInformation
                let nice = priority_to_nice(proc.base_priority);
                let priority = match proc.base_priority {
                    0..=4 => 0,      // Realtime
                    5..=8 => 5,      // High
                    9..=12 => 20,    // Normal
                    13..=15 => 30,   // Below normal
                    _ => 39,         // Idle
                };
                let handle_count = proc.handle_count;

                // Get cached efficiency_mode if available (requires Windows API call to query)
                let now = std::time::Instant::now();
                let efficiency_mode = EFFICIENCY_MODE_CACHE
                    .read()
                    .ok()
                    .and_then(|cache| {
                        cache.get(&pid).and_then(|info| {
                            if now.duration_since(info.last_update).as_millis() < EFFICIENCY_CACHE_TTL_MS {
                                Some(info.efficiency_mode)
                            } else {
                                None
                            }
                        })
                    })
                    .unwrap_or(false);

                // Get cached user if available
                let user = PID_USER_CACHE
                    .read()
                    .ok()
                    .and_then(|cache| cache.get(&pid).cloned())
                    .unwrap_or_else(|| {
                        // Special cases - use interned string
                        if pid == 0 || pid == 4 {
                            SYSTEM_STR.to_string()
                        } else {
                            "-".to_string()
                        }
                    });

                // CPU percentage from pre-calculated delta
                let cpu_percent = cpu_percentages.get(&pid).copied().unwrap_or(0.0);

                // Memory percentage
                let mem_percent = if total_mem > 0 {
                    (proc.working_set as f64 / total_mem as f64 * 100.0) as f32
                } else {
                    0.0
                };

                // Convert kernel+user time to Duration
                let total_100ns = proc.kernel_time + proc.user_time;
                let cpu_time = Duration::new(
                    total_100ns / 10_000_000,
                    ((total_100ns % 10_000_000) * 100) as u32,
                );

                // Convert create_time to Unix timestamp
                let start_time = filetime_to_unix(proc.create_time);

                // Use cached exe_path if available, otherwise fall back to name
                let (exe_path, command, command_lower) = if !cached_exe_path.is_empty() {
                    let lower = cached_exe_path.to_lowercase();
                    (cached_exe_path.clone(), cached_exe_path, lower)
                } else {
                    (String::new(), proc.name.clone(), proc.name.to_lowercase())
                };

                // Pre-compute lowercase strings for filtering
                let name_lower = proc.name.to_lowercase();
                let user_lower = user.to_lowercase();

                ProcessInfo {
                    pid,
                    parent_pid: proc.parent_pid,
                    name: proc.name.clone(),
                    exe_path,
                    command,
                    user,
                    status: 'R', // NT API doesn't give us detailed status
                    cpu_percent,
                    mem_percent,
                    virtual_mem: proc.virtual_size,
                    resident_mem: proc.working_set,
                    shared_mem: proc.working_set.saturating_sub(proc.private_bytes),
                    priority,
                    nice,
                    cpu_time,
                    tree_depth: 0,
                    tree_prefix: String::new(),
                    has_children: false,
                    is_collapsed: false,
                    thread_count: proc.thread_count,
                    start_time,
                    handle_count,
                    io_read_bytes: proc.read_bytes,
                    io_write_bytes: proc.write_bytes,
                    name_lower,
                    command_lower,
                    user_lower,
                    matches_search: false,
                    efficiency_mode,
                    is_elevated,
                    arch,
                }
            })
            .collect()
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

/// Set process priority (nice value)
#[cfg(windows)]
pub fn set_priority(pid: u32, nice: i32) -> Result<(), String> {
    unsafe {
        let handle = OpenProcess(PROCESS_SET_INFORMATION, false, pid)
            .map_err(|e| format!("Cannot open process: {}", e))?;

        if handle.is_invalid() {
            return Err(format!("Cannot open process {} (access denied)", pid));
        }

        // Map nice value to Windows priority class
        let priority_class = if nice <= -15 {
            REALTIME_PRIORITY_CLASS
        } else if nice <= -10 {
            HIGH_PRIORITY_CLASS
        } else if nice <= -5 {
            ABOVE_NORMAL_PRIORITY_CLASS
        } else if nice <= 5 {
            NORMAL_PRIORITY_CLASS
        } else if nice <= 10 {
            BELOW_NORMAL_PRIORITY_CLASS
        } else {
            IDLE_PRIORITY_CLASS
        };

        let result = SetPriorityClass(handle, priority_class);
        let _ = CloseHandle(handle);

        result.map_err(|e| format!("Cannot set priority: {}", e))?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn set_priority(pid: u32, nice: i32) -> Result<(), String> {
    use std::process::Command;
    Command::new("renice")
        .args([&nice.to_string(), "-p", &pid.to_string()])
        .output()
        .map_err(|e| format!("Failed to set priority: {}", e))?;
    Ok(())
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
        OpenProcess, SetProcessAffinityMask, PROCESS_SET_INFORMATION,
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
