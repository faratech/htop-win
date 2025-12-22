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

/// Per-core previous CPU times for delta calculation
#[derive(Default, Clone, Copy)]
struct PrevCpuTimes {
    user: i64,
    kernel: i64,
    idle: i64,
}

/// CPU usage information
#[derive(Default, Clone)]
pub struct CpuInfo {
    /// Per-core CPU usage percentages
    pub core_usage: Vec<f32>,
    /// Per-core CPU breakdown (user/system/idle)
    pub core_breakdown: Vec<CpuBreakdown>,
}

/// Thread-safe storage for previous CPU times (needed for delta calculation)
#[cfg(windows)]
use std::sync::Mutex;

#[cfg(windows)]
static PREV_CPU_TIMES: Mutex<Option<Vec<PrevCpuTimes>>> = Mutex::new(None);

impl CpuInfo {
    /// Create CpuInfo using native Windows API (NtQuerySystemInformation)
    #[cfg(windows)]
    pub fn from_native() -> Self {
        let (core_usage, core_breakdown) = get_cpu_info();
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

/// Get CPU count and per-core breakdown using NtQuerySystemInformation
#[cfg(windows)]
fn get_cpu_info() -> (Vec<f32>, Vec<CpuBreakdown>) {
    use std::mem::size_of;
    use windows::Wdk::System::SystemInformation::{
        NtQuerySystemInformation, SYSTEM_INFORMATION_CLASS,
    };
    use windows::Win32::System::WindowsProgramming::SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION;

    // SystemProcessorPerformanceInformation = 8
    const SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION_CLASS: SYSTEM_INFORMATION_CLASS =
        SYSTEM_INFORMATION_CLASS(8);

    // Query with a large enough buffer for up to 256 cores
    const MAX_CORES: usize = 256;
    let mut buffer: Vec<SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION> =
        vec![SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION::default(); MAX_CORES];
    let buffer_size = (MAX_CORES * size_of::<SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION>()) as u32;
    let mut return_length: u32 = 0;

    let result = unsafe {
        NtQuerySystemInformation(
            SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION_CLASS,
            buffer.as_mut_ptr() as *mut _,
            buffer_size,
            &mut return_length,
        )
    };

    if result.is_err() {
        return (Vec::new(), Vec::new());
    }

    let cpu_count =
        return_length as usize / size_of::<SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION>();

    // Get or initialize previous times
    let mut prev_guard = PREV_CPU_TIMES.lock().unwrap();
    if prev_guard.is_none() || prev_guard.as_ref().unwrap().len() != cpu_count {
        *prev_guard = Some(vec![PrevCpuTimes::default(); cpu_count]);
    }
    let prev_times = prev_guard.as_mut().unwrap();

    let mut core_usage = Vec::with_capacity(cpu_count);
    let mut breakdowns = Vec::with_capacity(cpu_count);

    for (i, info) in buffer.iter().take(cpu_count).enumerate() {
        // Note: KernelTime includes IdleTime on Windows
        let user = info.UserTime;
        let kernel = info.KernelTime;
        let idle = info.IdleTime;

        let prev = &prev_times[i];

        // Calculate deltas
        let user_delta = (user - prev.user).max(0);
        let kernel_delta = (kernel - prev.kernel).max(0);
        let idle_delta = (idle - prev.idle).max(0);

        // Total time = user + kernel (kernel already includes idle)
        // Active kernel time = kernel - idle
        let total = user_delta + kernel_delta;
        let system_delta = kernel_delta - idle_delta;

        let (usage, breakdown) = if total > 0 {
            let user_pct = (user_delta as f64 / total as f64 * 100.0) as f32;
            let system_pct = (system_delta as f64 / total as f64 * 100.0).max(0.0) as f32;
            let idle_pct = (idle_delta as f64 / total as f64 * 100.0) as f32;
            (
                user_pct + system_pct, // Total usage = user + system
                CpuBreakdown {
                    user: user_pct,
                    system: system_pct,
                    idle: idle_pct,
                },
            )
        } else {
            (
                0.0,
                CpuBreakdown {
                    user: 0.0,
                    system: 0.0,
                    idle: 100.0,
                },
            )
        };

        core_usage.push(usage);
        breakdowns.push(breakdown);

        // Store current values for next iteration
        prev_times[i] = PrevCpuTimes {
            user,
            kernel,
            idle,
        };
    }

    (core_usage, breakdowns)
}
