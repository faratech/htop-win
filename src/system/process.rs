use std::collections::HashMap;
#[cfg(windows)]
use std::sync::Mutex;
use std::time::Duration;
use sysinfo::{ProcessStatus, System};

#[cfg(windows)]
use std::sync::LazyLock;

#[cfg(windows)]
use windows::core::PWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HLOCAL};
#[cfg(windows)]
use windows::Win32::Security::Authorization::ConvertStringSidToSidW;
#[cfg(windows)]
use windows::Win32::Security::{
    GetTokenInformation, LookupAccountSidW, TokenElevation, TokenUser, PSID, SID_NAME_USE,
    TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
};
#[cfg(windows)]
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
#[cfg(windows)]
use windows::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
#[cfg(windows)]
use windows::Win32::System::Threading::IO_COUNTERS;
#[cfg(windows)]
use windows::Win32::System::Threading::{
    GetPriorityClass, GetProcessHandleCount, GetProcessInformation, GetProcessIoCounters,
    GetProcessTimes, IsWow64Process2, OpenProcess, OpenProcessToken, ProcessPowerThrottling,
    SetPriorityClass, TerminateProcess, ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
    HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
    PROCESS_TERMINATE, REALTIME_PRIORITY_CLASS,
};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
};

// Cache for SID to username lookups
#[cfg(windows)]
static SID_CACHE: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    // Pre-populate well-known SIDs
    map.insert("S-1-5-18".to_string(), "SYSTEM".to_string());
    map.insert("S-1-5-19".to_string(), "LOCAL SERVICE".to_string());
    map.insert("S-1-5-20".to_string(), "NETWORK SERVICE".to_string());
    Mutex::new(map)
});

// Cache for PID to username lookups (persists across refreshes)
#[cfg(windows)]
static PID_USER_CACHE: LazyLock<Mutex<HashMap<u32, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// Cache for thread counts (expensive to compute - only refresh every 2 seconds)
#[cfg(windows)]
static THREAD_COUNT_CACHE: LazyLock<Mutex<(std::time::Instant, HashMap<u32, u32>)>> =
    LazyLock::new(|| Mutex::new((std::time::Instant::now(), HashMap::new())));

#[cfg(windows)]
const THREAD_CACHE_DURATION_MS: u128 = 2000; // Refresh thread counts every 2 seconds

/// Get the owner of a process by PID using Windows API
#[cfg(windows)]
fn get_process_owner(pid: u32) -> Option<String> {
    use windows::Win32::System::Threading::OpenProcessToken;

    // Special cases for system processes
    if pid == 0 {
        return Some("SYSTEM".to_string());
    }
    if pid == 4 {
        return Some("SYSTEM".to_string());
    }

    // Check PID cache first
    if let Ok(cache) = PID_USER_CACHE.lock() {
        if let Some(user) = cache.get(&pid) {
            return Some(user.clone());
        }
    }

    unsafe {
        // Try PROCESS_QUERY_INFORMATION first, fall back to LIMITED
        let process_handle = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid)
            .or_else(|_| OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid))
            .ok()
            .filter(|h| !h.is_invalid())?;

        // Open the process token
        let mut token_handle = HANDLE::default();
        let token_result = OpenProcessToken(process_handle, TOKEN_QUERY, &mut token_handle);

        if token_result.is_err() {
            let _ = CloseHandle(process_handle);
            return None;
        }

        // Get token user info - first call to get required size
        let mut token_info_len: u32 = 0;
        let _ = GetTokenInformation(token_handle, TokenUser, None, 0, &mut token_info_len);

        if token_info_len == 0 {
            let _ = CloseHandle(token_handle);
            let _ = CloseHandle(process_handle);
            return None;
        }

        let mut token_info: Vec<u8> = vec![0; token_info_len as usize];
        let get_info_result = GetTokenInformation(
            token_handle,
            TokenUser,
            Some(token_info.as_mut_ptr() as *mut _),
            token_info_len,
            &mut token_info_len,
        );

        let _ = CloseHandle(token_handle);
        let _ = CloseHandle(process_handle);

        if get_info_result.is_err() {
            return None;
        }

        // Cast to TOKEN_USER
        let token_user = &*(token_info.as_ptr() as *const TOKEN_USER);
        let sid = token_user.User.Sid;

        // Look up the account name
        let mut name_buf: Vec<u16> = vec![0; 256];
        let mut domain_buf: Vec<u16> = vec![0; 256];
        let mut name_len: u32 = name_buf.len() as u32;
        let mut domain_len: u32 = domain_buf.len() as u32;
        let mut sid_type = SID_NAME_USE::default();

        let lookup_result = LookupAccountSidW(
            None,
            sid,
            Some(PWSTR(name_buf.as_mut_ptr())),
            &mut name_len,
            Some(PWSTR(domain_buf.as_mut_ptr())),
            &mut domain_len,
            &mut sid_type,
        );

        if lookup_result.is_ok() && name_len > 0 {
            let username = String::from_utf16_lossy(&name_buf[..name_len as usize]);

            // Cache the result
            if let Ok(mut cache) = PID_USER_CACHE.lock() {
                cache.insert(pid, username.clone());
            }

            Some(username)
        } else {
            None
        }
    }
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

/// Extended process info from Windows API (combined for efficiency)
#[cfg(windows)]
struct WinProcessInfo {
    priority: i32,
    nice: i32,
    cpu_time: Duration,
    start_time: u64, // Unix timestamp
    handle_count: u32,
    io_read_bytes: u64,
    io_write_bytes: u64,
    shared_mem: u64,
    efficiency_mode: bool, // Windows 11 EcoQoS
    is_elevated: bool,     // Running as admin
    arch: ProcessArch,     // Process architecture
}

/// Get priority, nice, CPU time, handle count, and I/O counters with a single OpenProcess call
#[cfg(windows)]
fn get_win_process_info(pid: u32) -> WinProcessInfo {
    let default = WinProcessInfo {
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
    };

    unsafe {
        // Try full access first, fall back to limited
        let handle = match OpenProcess(PROCESS_QUERY_INFORMATION, false, pid) {
            Ok(h) if !h.is_invalid() => h,
            _ => match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(h) if !h.is_invalid() => h,
                _ => return default,
            },
        };

        // Get priority class
        let priority_class = GetPriorityClass(handle);
        let (priority, nice) = match priority_class {
            x if x == IDLE_PRIORITY_CLASS.0 => (39, 19),
            x if x == BELOW_NORMAL_PRIORITY_CLASS.0 => (30, 10),
            x if x == NORMAL_PRIORITY_CLASS.0 => (20, 0),
            x if x == ABOVE_NORMAL_PRIORITY_CLASS.0 => (10, -5),
            x if x == HIGH_PRIORITY_CLASS.0 => (5, -10),
            x if x == REALTIME_PRIORITY_CLASS.0 => (0, -20),
            _ => (20, 0),
        };

        // Get CPU time and start time
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();

        let (cpu_time, start_time) =
            if GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user).is_ok() {
                let kernel_100ns =
                    ((kernel.dwHighDateTime as u64) << 32) | kernel.dwLowDateTime as u64;
                let user_100ns = ((user.dwHighDateTime as u64) << 32) | user.dwLowDateTime as u64;
                let total_100ns = kernel_100ns + user_100ns;
                let secs = total_100ns / 10_000_000;
                let nanos = ((total_100ns % 10_000_000) * 100) as u32;

                // Convert creation time (FILETIME) to Unix timestamp
                // FILETIME is 100-nanosecond intervals since January 1, 1601
                // Unix epoch is January 1, 1970
                // Difference is 116444736000000000 100-ns intervals
                let creation_100ns =
                    ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
                let unix_time = if creation_100ns > 116444736000000000 {
                    (creation_100ns - 116444736000000000) / 10_000_000
                } else {
                    0
                };

                (Duration::new(secs, nanos), unix_time)
            } else {
                (Duration::ZERO, 0)
            };

        // Get handle count
        let mut handle_count: u32 = 0;
        let _ = GetProcessHandleCount(handle, &mut handle_count);

        // Get I/O counters
        let mut io_counters = IO_COUNTERS::default();
        let (io_read_bytes, io_write_bytes) =
            if GetProcessIoCounters(handle, &mut io_counters).is_ok() {
                (
                    io_counters.ReadTransferCount,
                    io_counters.WriteTransferCount,
                )
            } else {
                (0, 0)
            };

        // Get memory info for shared memory calculation
        // Use PROCESS_MEMORY_COUNTERS_EX to get PrivateUsage field
        // Shared memory = WorkingSetSize - PrivateUsage
        let mut mem_counters_ex = PROCESS_MEMORY_COUNTERS_EX::default();
        mem_counters_ex.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let shared_mem = if K32GetProcessMemoryInfo(
            handle,
            &mut mem_counters_ex as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )
        .as_bool()
        {
            // Shared = WorkingSetSize - PrivateUsage
            (mem_counters_ex.WorkingSetSize as u64)
                .saturating_sub(mem_counters_ex.PrivateUsage as u64)
        } else {
            0
        };

        // Check for Efficiency Mode (EcoQoS) - Windows 11+
        let efficiency_mode = {
            let mut throttle_state = PROCESS_POWER_THROTTLING_STATE::default();
            throttle_state.Version = 1; // PROCESS_POWER_THROTTLING_CURRENT_VERSION
            let result = GetProcessInformation(
                handle,
                ProcessPowerThrottling,
                &mut throttle_state as *mut _ as *mut _,
                std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            );
            if result.is_ok() {
                // Check if PROCESS_POWER_THROTTLING_EXECUTION_SPEED is set in StateMask
                (throttle_state.StateMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED) != 0
                    && (throttle_state.ControlMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED) != 0
            } else {
                false
            }
        };

        // Check if process is elevated (running as admin)
        let is_elevated = {
            let mut token_handle = HANDLE::default();
            if OpenProcessToken(handle, TOKEN_QUERY, &mut token_handle).is_ok() {
                let mut elevation = TOKEN_ELEVATION::default();
                let mut return_length: u32 = 0;
                let elevated = GetTokenInformation(
                    token_handle,
                    TokenElevation,
                    Some(&mut elevation as *mut _ as *mut _),
                    std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                    &mut return_length,
                )
                .is_ok()
                    && elevation.TokenIsElevated != 0;
                let _ = CloseHandle(token_handle);
                elevated
            } else {
                false
            }
        };

        // Detect process architecture (x86/x64/ARM64)
        let arch = {
            use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE;
            let mut process_machine = IMAGE_FILE_MACHINE::default();
            let mut native_machine = IMAGE_FILE_MACHINE::default();
            if IsWow64Process2(
                handle,
                &mut process_machine,
                Some(&mut native_machine),
            )
            .is_ok()
            {
                if process_machine.0 == 0 {
                    // Not running under WoW64, native process
                    ProcessArch::Native
                } else if process_machine == IMAGE_FILE_MACHINE_I386 {
                    ProcessArch::X86
                } else if process_machine == IMAGE_FILE_MACHINE_AMD64 {
                    // x64 process on ARM64 (via emulation)
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

        let _ = CloseHandle(handle);

        WinProcessInfo {
            priority,
            nice,
            cpu_time,
            start_time,
            handle_count,
            io_read_bytes,
            io_write_bytes,
            shared_mem,
            efficiency_mode,
            is_elevated,
            arch,
        }
    }
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
    }
}

#[cfg(not(windows))]
fn get_process_owner(_pid: u32) -> Option<String> {
    None
}

/// Get thread counts for all processes in one efficient call (cached)
#[cfg(windows)]
fn get_all_thread_counts() -> HashMap<u32, u32> {
    // Check if we can use cached thread counts
    if let Ok(mut cache) = THREAD_COUNT_CACHE.lock() {
        let elapsed = cache.0.elapsed().as_millis();
        if elapsed < THREAD_CACHE_DURATION_MS && !cache.1.is_empty() {
            return cache.1.clone();
        }

        // Need to refresh - compute new counts
        let mut counts: HashMap<u32, u32> = HashMap::new();

        unsafe {
            // Create a snapshot of all threads in the system
            if let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) {
                if !snapshot.is_invalid() {
                    let mut entry = THREADENTRY32 {
                        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                        ..Default::default()
                    };

                    // Get the first thread
                    if Thread32First(snapshot, &mut entry).is_ok() {
                        loop {
                            // Increment count for this process
                            *counts.entry(entry.th32OwnerProcessID).or_insert(0) += 1;

                            // Reset dwSize before next call
                            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;

                            if Thread32Next(snapshot, &mut entry).is_err() {
                                break;
                            }
                        }
                    }

                    let _ = CloseHandle(snapshot);
                }
            }
        }

        // Update cache
        cache.0 = std::time::Instant::now();
        cache.1 = counts.clone();
        counts
    } else {
        // Fallback if lock fails
        HashMap::new()
    }
}

#[cfg(not(windows))]
fn get_all_thread_counts() -> HashMap<u32, u32> {
    HashMap::new()
}

/// Convert a Windows SID string to a username
#[cfg(windows)]
fn sid_to_username(sid_str: &str) -> String {
    // Check cache first
    if let Ok(cache) = SID_CACHE.lock() {
        if let Some(cached) = cache.get(sid_str) {
            return cached.clone();
        }
    }

    // Try to look up the SID using Windows API
    let result = unsafe {
        // Convert string SID to binary SID
        let sid_wide: Vec<u16> = sid_str.encode_utf16().chain(std::iter::once(0)).collect();
        let mut psid: PSID = PSID::default();

        if ConvertStringSidToSidW(windows::core::PCWSTR(sid_wide.as_ptr()), &mut psid).is_err() {
            return truncate_sid(sid_str);
        }

        // Buffers for account name and domain
        let mut name_buf: Vec<u16> = vec![0; 256];
        let mut domain_buf: Vec<u16> = vec![0; 256];
        let mut name_len: u32 = name_buf.len() as u32;
        let mut domain_len: u32 = domain_buf.len() as u32;
        let mut sid_type = SID_NAME_USE::default();

        let lookup_result = LookupAccountSidW(
            None,
            psid,
            Some(PWSTR(name_buf.as_mut_ptr())),
            &mut name_len,
            Some(PWSTR(domain_buf.as_mut_ptr())),
            &mut domain_len,
            &mut sid_type,
        );

        // Free the SID
        let _ = windows::Win32::Foundation::LocalFree(Some(HLOCAL(psid.0)));

        if lookup_result.is_ok() && name_len > 0 {
            let name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
            Some(name)
        } else {
            None
        }
    };

    let username = result.unwrap_or_else(|| truncate_sid(sid_str));

    // Cache the result
    if let Ok(mut cache) = SID_CACHE.lock() {
        cache.insert(sid_str.to_string(), username.clone());
    }

    username
}

/// Truncate long SID strings for display
#[cfg(windows)]
fn truncate_sid(sid: &str) -> String {
    if sid.len() > 12 {
        if let Some(last_dash) = sid.rfind('-') {
            let suffix = &sid[last_dash..];
            if suffix.len() < 8 {
                return format!("S-1-..{}", suffix);
            }
        }
        format!("{}...", &sid[..9])
    } else {
        sid.to_string()
    }
}

#[cfg(not(windows))]
fn sid_to_username(sid_str: &str) -> String {
    sid_str.to_string()
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
    pub fn from_sysinfo(sys: &System) -> Vec<ProcessInfo> {
        let total_mem = sys.total_memory();

        // Note: We intentionally don't clear PID_USER_CACHE every refresh.
        // The cache grows but user lookups are expensive Windows API calls.
        // Stale entries (from reused PIDs) are rare and only affect display.

        // Get thread counts for all processes in one efficient call
        let thread_counts = get_all_thread_counts();

        sys.processes()
            .iter()
            .map(|(pid, proc)| {
                let pid_u32 = pid.as_u32();
                let thread_count = thread_counts.get(&pid_u32).copied().unwrap_or(0);
                let memory = proc.memory();
                let mem_percent = if total_mem > 0 {
                    (memory as f64 / total_mem as f64 * 100.0) as f32
                } else {
                    0.0
                };

                let status = match proc.status() {
                    ProcessStatus::Run => 'R',
                    ProcessStatus::Sleep => 'S',
                    ProcessStatus::Idle => 'I',
                    ProcessStatus::Zombie => 'Z',
                    ProcessStatus::Stop => 'T',
                    _ => '?',
                };

                // Get executable path
                let exe_path = proc
                    .exe()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();

                // Get command line arguments
                let cmd = proc.cmd();
                let command = if cmd.is_empty() {
                    // If no command line, use exe path or name
                    if !exe_path.is_empty() {
                        exe_path.clone()
                    } else {
                        proc.name().to_string_lossy().to_string()
                    }
                } else {
                    // Join command line arguments
                    cmd.iter()
                        .map(|s| s.to_string_lossy().to_string())
                        .collect::<Vec<_>>()
                        .join(" ")
                };

                // Get user - try Windows API first, then fall back to sysinfo
                let user = get_process_owner(pid_u32)
                    .or_else(|| proc.user_id().map(|u| sid_to_username(&u.to_string())))
                    .unwrap_or_else(|| "-".to_string());

                // Get parent PID
                let parent_pid = proc.parent().map(|p| p.as_u32()).unwrap_or(0);

                // Get priority, nice, and CPU time in one optimized call
                let win_info = get_win_process_info(pid_u32);

                let name = proc.name().to_string_lossy().to_string();

                // Pre-compute lowercase strings once at creation time
                let name_lower = name.to_lowercase();
                let command_lower = command.to_lowercase();
                let user_lower = user.to_lowercase();

                ProcessInfo {
                    pid: pid_u32,
                    parent_pid,
                    name,
                    exe_path,
                    command,
                    user,
                    status,
                    cpu_percent: proc.cpu_usage(),
                    mem_percent,
                    virtual_mem: proc.virtual_memory(),
                    resident_mem: memory,
                    shared_mem: win_info.shared_mem,
                    priority: win_info.priority,
                    nice: win_info.nice,
                    cpu_time: win_info.cpu_time,
                    tree_depth: 0,
                    tree_prefix: String::new(), // Set by tree builder
                    // New fields
                    has_children: false, // Set by tree builder
                    is_collapsed: false, // Set by tree builder
                    thread_count,
                    start_time: win_info.start_time,
                    handle_count: win_info.handle_count,
                    io_read_bytes: win_info.io_read_bytes,
                    io_write_bytes: win_info.io_write_bytes,
                    // Pre-computed for efficient filtering
                    name_lower,
                    command_lower,
                    user_lower,
                    matches_search: false, // Set during filtering
                    efficiency_mode: win_info.efficiency_mode,
                    is_elevated: win_info.is_elevated,
                    arch: win_info.arch,
                }
            })
            .collect()
    }

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
