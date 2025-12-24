/// Per-core CPU time breakdown (in percentages)
#[derive(Default, Clone, Copy)]
#[allow(dead_code)]
pub struct CpuBreakdown {
    /// User mode CPU usage percentage
    pub user: f32,
    /// Kernel/system mode CPU usage percentage
    pub system: f32,
    /// Idle percentage (for reference, not displayed in bar)
    pub idle: f32,
}

/// CPU usage information
#[derive(Default, Clone)]
pub struct CpuInfo {
    /// Per-core CPU usage percentages
    pub core_usage: Vec<f32>,
    /// Per-core CPU breakdown (user/system/idle)
    pub core_breakdown: Vec<CpuBreakdown>,
}

impl CpuInfo {
    /// Create CpuInfo using Windows PDH (Performance Data Helper)
    /// This matches Task Manager's CPU usage calculation
    #[cfg(windows)]
    pub fn from_native() -> Self {
        let (core_usage, core_breakdown) = get_cpu_info_pdh();
        Self {
            core_usage,
            core_breakdown,
        }
    }

    #[cfg(not(windows))]
    pub fn from_native() -> Self {
        Self::default()
    }
}

/// PDH-based CPU info collection using Windows Performance Counters
/// This is the same method Task Manager uses
#[cfg(windows)]
fn get_cpu_info_pdh() -> (Vec<f32>, Vec<CpuBreakdown>) {
    use std::sync::Mutex;
    use windows::core::PCWSTR;
    use windows::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
        PdhGetFormattedCounterValue, PdhOpenQueryW, PDH_CSTATUS_VALID_DATA,
        PDH_FMT_DOUBLE, PDH_FMT_COUNTERVALUE, PDH_HCOUNTER, PDH_HQUERY,
    };
    use windows::Win32::System::SystemInformation::GetSystemInfo;

    /// Wrapper to make PDH handles Send (they're only accessed with mutex held)
    struct SendPtr(*mut std::ffi::c_void);
    unsafe impl Send for SendPtr {}
    impl SendPtr {
        fn as_query(&self) -> PDH_HQUERY { PDH_HQUERY(self.0) }
        fn as_counter(&self) -> PDH_HCOUNTER { PDH_HCOUNTER(self.0) }
    }

    /// Counter set for each CPU core (user time, privileged/system time)
    struct CoreCounters {
        user: SendPtr,
        privileged: SendPtr,
    }

    /// Static state for PDH query (persists across calls)
    struct PdhState {
        query: SendPtr,
        core_counters: Vec<CoreCounters>,
        initialized: bool,
        first_sample_done: bool,
    }

    impl Default for PdhState {
        fn default() -> Self {
            Self {
                query: SendPtr(std::ptr::null_mut()),
                core_counters: Vec::new(),
                initialized: false,
                first_sample_done: false,
            }
        }
    }

    // Drop implementation to close PDH query (won't be called in static, but good practice)
    impl Drop for PdhState {
        fn drop(&mut self) {
            if self.initialized {
                unsafe { let _ = PdhCloseQuery(self.query.as_query()); }
            }
        }
    }

    /// Helper to add a PDH counter
    unsafe fn add_counter(query: PDH_HQUERY, path: &str) -> Option<SendPtr> {
        let path_wide: Vec<u16> = format!("{}\0", path).encode_utf16().collect();
        let mut counter = PDH_HCOUNTER::default();
        let status = unsafe {
            PdhAddEnglishCounterW(
                query,
                PCWSTR(path_wide.as_ptr()),
                0,
                &mut counter,
            )
        };
        if status == 0 {
            Some(SendPtr(counter.0))
        } else {
            None
        }
    }

    /// Helper to get counter value as f32 percentage
    unsafe fn get_counter_value(counter: &SendPtr) -> f32 {
        let mut value = PDH_FMT_COUNTERVALUE::default();
        let status = unsafe {
            PdhGetFormattedCounterValue(
                counter.as_counter(),
                PDH_FMT_DOUBLE,
                None,
                &mut value,
            )
        };
        if status == 0 && value.CStatus == PDH_CSTATUS_VALID_DATA {
            unsafe { (value.Anonymous.doubleValue as f32).clamp(0.0, 100.0) }
        } else {
            0.0
        }
    }

    static PDH_STATE: Mutex<Option<PdhState>> = Mutex::new(None);

    let mut state_guard = PDH_STATE.lock().unwrap();
    let state = state_guard.get_or_insert_with(PdhState::default);

    // Get CPU count
    let cpu_count = unsafe {
        let mut sys_info = std::mem::zeroed();
        GetSystemInfo(&mut sys_info);
        sys_info.dwNumberOfProcessors as usize
    };

    // Initialize PDH query if needed
    if !state.initialized {
        unsafe {
            // Open a real-time query
            let mut query = PDH_HQUERY::default();
            let status = PdhOpenQueryW(PCWSTR::null(), 0, &mut query);
            if status != 0 {
                return fallback_cpu_info(cpu_count);
            }
            state.query = SendPtr(query.0);

            // Add counters for each processor:
            // - % User Time: time in user mode
            // - % Privileged Time: time in kernel mode (system)
            state.core_counters.reserve(cpu_count);
            for i in 0..cpu_count {
                let user = match add_counter(query, &format!("\\Processor({})\\% User Time", i)) {
                    Some(c) => c,
                    None => {
                        let _ = PdhCloseQuery(query);
                        return fallback_cpu_info(cpu_count);
                    }
                };
                let privileged = match add_counter(query, &format!("\\Processor({})\\% Privileged Time", i)) {
                    Some(c) => c,
                    None => {
                        let _ = PdhCloseQuery(query);
                        return fallback_cpu_info(cpu_count);
                    }
                };
                state.core_counters.push(CoreCounters { user, privileged });
            }

            state.initialized = true;
        }
    }

    // Collect query data
    unsafe {
        let status = PdhCollectQueryData(state.query.as_query());
        if status != 0 {
            return fallback_cpu_info(cpu_count);
        }
    }

    // First sample just initializes - PDH needs two samples for rate counters
    if !state.first_sample_done {
        state.first_sample_done = true;
        // Return zeros for first sample
        let core_usage = vec![0.0; cpu_count];
        let breakdowns = vec![CpuBreakdown { user: 0.0, system: 0.0, idle: 100.0 }; cpu_count];
        return (core_usage, breakdowns);
    }

    // Get formatted counter values
    let mut core_usage = Vec::with_capacity(cpu_count);
    let mut breakdowns = Vec::with_capacity(cpu_count);

    for counters in &state.core_counters {
        let user_pct = unsafe { get_counter_value(&counters.user) };
        let system_pct = unsafe { get_counter_value(&counters.privileged) };
        let total = (user_pct + system_pct).min(100.0);

        core_usage.push(total);
        breakdowns.push(CpuBreakdown {
            user: user_pct,
            system: system_pct,
            idle: (100.0 - total).max(0.0),
        });
    }

    (core_usage, breakdowns)
}

/// Fallback CPU info when PDH fails (returns zeros)
#[cfg(windows)]
fn fallback_cpu_info(cpu_count: usize) -> (Vec<f32>, Vec<CpuBreakdown>) {
    let core_usage = vec![0.0; cpu_count];
    let breakdowns = vec![CpuBreakdown { user: 0.0, system: 0.0, idle: 100.0 }; cpu_count];
    (core_usage, breakdowns)
}
