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

    /// In-place refresh reusing `core_usage`/`core_breakdown` capacity across
    /// ticks (the vecs only resize on hot-add, where PDH re-registers anyway).
    #[cfg(windows)]
    pub fn refresh_in_place(&mut self) {
        get_cpu_info_pdh_into(&mut self.core_usage, &mut self.core_breakdown);
    }

    #[cfg(not(windows))]
    pub fn refresh_in_place(&mut self) {
        *self = Self::default();
    }
}

/// Re-prime flag for [`reset_first_sample`]: the PDH state lives inside
/// `get_cpu_info_pdh_into`, so the reset is requested via this flag and
/// applied on the next collection.
#[cfg(windows)]
static PDH_REPRIME: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Re-prime PDH so the first collection after the CPU gate is re-enabled
/// re-baselines (reports 0%) instead of a gap-averaged rate.
#[cfg(windows)]
pub(crate) fn reset_first_sample() {
    PDH_REPRIME.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(not(windows))]
pub(crate) fn reset_first_sample() {}

/// How often the processor topology is re-enumerated. Hot-add and processor-group
/// changes are rare, but per-tick enumeration costs a syscall per group on
/// every refresh *and* on every CPU% capacity-denominator update. Thirty
/// seconds matches the D3DKMT adapter topology check interval.
#[cfg(windows)]
const LAYOUT_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Cached processor topology shared by the per-tick CPU refresh and the CPU%
/// capacity denominator so neither enumerates processor groups every tick.
#[cfg(windows)]
static LAYOUT_CACHE: std::sync::Mutex<Option<CachedLayout>> = std::sync::Mutex::new(None);

/// The processor layout and when it was read.
#[cfg(windows)]
type CachedLayout = (Vec<(u16, u32)>, std::time::Instant);

/// Runs `f` against the cached layout, refreshing it under the lock when the
/// TTL has lapsed. Shared by the full-clone accessor and the clone-free len
/// accessor used on the per-tick hot paths.
#[cfg(windows)]
fn with_layout<R>(f: impl FnOnce(&[(u16, u32)]) -> R) -> R {
    let mut guard = LAYOUT_CACHE.lock().unwrap();
    let stale = guard
        .as_ref()
        .is_none_or(|(_, sampled_at)| sampled_at.elapsed() >= LAYOUT_TTL);
    if stale {
        *guard = Some((processor_layout(), std::time::Instant::now()));
    }
    f(guard
        .as_ref()
        .map(|(layout, _)| layout.as_slice())
        .unwrap_or(&[]))
}

/// Cached processor layout (see [`LAYOUT_TTL`]). The vector is tiny (one entry
/// per logical processor); cold paths (counter registration, debug dumps) may
/// as well clone it — hot paths should prefer [`cached_processor_layout_len`].
#[cfg(windows)]
pub(crate) fn cached_processor_layout() -> Vec<(u16, u32)> {
    with_layout(<[(u16, u32)]>::to_vec)
}

/// Logical-processor count from the cached topology, without cloning the
/// layout vector (the per-tick callers only need the count).
#[cfg(windows)]
pub(crate) fn cached_processor_layout_len() -> usize {
    with_layout(<[(u16, u32)]>::len)
}

/// Flat list of (group, processor-in-group) for every active logical processor
/// across all Windows processor groups, in group-then-index order.
///
/// Windows splits systems with >64 logical processors into groups of up to 64.
/// `GetSystemInfo`/`dwNumberOfProcessors` and the legacy `\Processor(N)` PDH
/// counterset only ever see the *current* group (<=64), so high-core-count
/// servers/HEDT would show only the first 64 CPUs. Enumerating every group and
/// using the group-aware `\Processor Information(group,n)` counters makes them
/// all visible. Falls back to a single GetSystemInfo-sized group if the group
/// APIs report nothing.
#[cfg(windows)]
fn processor_layout() -> Vec<(u16, u32)> {
    use windows::Win32::System::Threading::{
        GetActiveProcessorCount, GetActiveProcessorGroupCount,
    };
    let mut layout = Vec::new();
    unsafe {
        let groups = GetActiveProcessorGroupCount();
        for group in 0..groups {
            let count = GetActiveProcessorCount(group);
            for n in 0..count {
                layout.push((group, n));
            }
        }
    }
    if layout.is_empty() {
        use windows::Win32::System::SystemInformation::GetSystemInfo;
        let count = unsafe {
            let mut si = std::mem::zeroed();
            GetSystemInfo(&mut si);
            si.dwNumberOfProcessors
        };
        for n in 0..count {
            layout.push((0, n));
        }
    }
    layout
}

/// PDH-based CPU info collection using Windows Performance Counters
/// This is the same method Task Manager uses
#[cfg(windows)]
fn get_cpu_info_pdh() -> (Vec<f32>, Vec<CpuBreakdown>) {
    let mut core_usage = Vec::new();
    let mut core_breakdown = Vec::new();
    get_cpu_info_pdh_into(&mut core_usage, &mut core_breakdown);
    (core_usage, core_breakdown)
}

/// [`get_cpu_info_pdh`] writing into caller-owned vecs so per-tick capacity
/// is reused (core count changes re-register the PDH state anyway).
#[cfg(windows)]
fn get_cpu_info_pdh_into(core_usage: &mut Vec<f32>, breakdowns: &mut Vec<CpuBreakdown>) {
    use std::sync::Mutex;
    use windows::Win32::System::Performance::{
        PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterValue,
        PdhOpenQueryW,
    };
    use windows::core::PCWSTR;

    /// Wrapper to make PDH handles Send (they're only accessed with mutex held)
    struct SendPtr(*mut std::ffi::c_void);
    unsafe impl Send for SendPtr {}
    impl SendPtr {
        fn as_query(&self) -> PDH_HQUERY {
            PDH_HQUERY(self.0)
        }
        fn as_counter(&self) -> PDH_HCOUNTER {
            PDH_HCOUNTER(self.0)
        }
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
                unsafe {
                    let _ = PdhCloseQuery(self.query.as_query());
                }
            }
        }
    }

    /// Helper to add a PDH counter
    unsafe fn add_counter(query: PDH_HQUERY, path: &str) -> Option<SendPtr> {
        let path_wide: Vec<u16> = format!("{}\0", path).encode_utf16().collect();
        let mut counter = PDH_HCOUNTER::default();
        let status =
            unsafe { PdhAddEnglishCounterW(query, PCWSTR(path_wide.as_ptr()), 0, &mut counter) };
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
            PdhGetFormattedCounterValue(counter.as_counter(), PDH_FMT_DOUBLE, None, &mut value)
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

    if PDH_REPRIME.swap(false, std::sync::atomic::Ordering::Relaxed) {
        state.first_sample_done = false;
    }

    // Every logical processor across all processor groups (handles >64-CPU
    // systems, which Windows splits into groups of up to 64). Cached: topology
    // only changes on hot-add or processor-group reassignment.
    let cpu_count = cached_processor_layout_len();

    // Re-register counters when the topology changed since init (hot-add or a
    // new processor group); otherwise new cores stay invisible until restart.
    // The replaced state's Drop closes the old query, so no manual close here
    // (that would double-close the handle).
    if state.initialized && state.core_counters.len() != cpu_count {
        *state = PdhState::default();
    }

    // Initialize PDH query if needed
    if !state.initialized {
        unsafe {
            // Open a real-time query
            let mut query = PDH_HQUERY::default();
            let status = PdhOpenQueryW(PCWSTR::null(), 0, &mut query);
            if status != 0 {
                let (usage, breakdown) = fallback_cpu_info(cpu_count);
                *core_usage = usage;
                *breakdowns = breakdown;
                return;
            }
            // Add per-processor counters using the group-aware "Processor
            // Information(group,n)" counterset so every group is covered:
            // - % User Time: time in user mode
            // - % Privileged Time: time in kernel mode (system)
            let mut core_counters = Vec::with_capacity(cpu_count);
            for &(group, n) in cached_processor_layout().iter() {
                let user_path = format!("\\Processor Information({group},{n})\\% User Time");
                let priv_path = format!("\\Processor Information({group},{n})\\% Privileged Time");
                let user = match add_counter(query, &user_path) {
                    Some(c) => c,
                    None => {
                        let _ = PdhCloseQuery(query);
                        let (usage, breakdown) = fallback_cpu_info(cpu_count);
                        *core_usage = usage;
                        *breakdowns = breakdown;
                        return;
                    }
                };
                let privileged = match add_counter(query, &priv_path) {
                    Some(c) => c,
                    None => {
                        let _ = PdhCloseQuery(query);
                        let (usage, breakdown) = fallback_cpu_info(cpu_count);
                        *core_usage = usage;
                        *breakdowns = breakdown;
                        return;
                    }
                };
                core_counters.push(CoreCounters { user, privileged });
            }

            state.query = SendPtr(query.0);
            state.core_counters = core_counters;
            state.initialized = true;
        }
    }

    // Collect query data
    unsafe {
        let status = PdhCollectQueryData(state.query.as_query());
        if status != 0 {
            let (usage, breakdown) = fallback_cpu_info(cpu_count);
            *core_usage = usage;
            *breakdowns = breakdown;
            return;
        }
    }

    // First sample just initializes - PDH needs two samples for rate counters
    if !state.first_sample_done {
        state.first_sample_done = true;
        // Zeros for the first sample
        core_usage.clear();
        core_usage.resize(cpu_count, 0.0);
        breakdowns.clear();
        breakdowns.resize(
            cpu_count,
            CpuBreakdown {
                user: 0.0,
                system: 0.0,
                idle: 100.0,
            },
        );
        return;
    }

    // Get formatted counter values in place
    core_usage.clear();
    core_usage.resize(cpu_count, 0.0);
    breakdowns.clear();
    breakdowns.resize(
        cpu_count,
        CpuBreakdown {
            user: 0.0,
            system: 0.0,
            idle: 100.0,
        },
    );

    for (idx, counters) in state.core_counters.iter().enumerate() {
        let user_pct = unsafe { get_counter_value(&counters.user) };
        let system_pct = unsafe { get_counter_value(&counters.privileged) };
        let total = (user_pct + system_pct).min(100.0);

        core_usage[idx] = total;
        breakdowns[idx] = CpuBreakdown {
            user: user_pct,
            system: system_pct,
            idle: (100.0 - total).max(0.0),
        };
    }
}

/// Fallback CPU info when PDH fails (returns zeros)
#[cfg(windows)]
fn fallback_cpu_info(cpu_count: usize) -> (Vec<f32>, Vec<CpuBreakdown>) {
    let core_usage = vec![0.0; cpu_count];
    let breakdowns = vec![
        CpuBreakdown {
            user: 0.0,
            system: 0.0,
            idle: 100.0
        };
        cpu_count
    ];
    (core_usage, breakdowns)
}

/// Diagnostics for CPU/processor-group enumeration (`--cpu-debug`): samples
/// per-core usage twice (PDH needs two samples) and reports the group layout.
#[cfg(windows)]
pub fn debug_dump() -> String {
    use std::fmt::Write as _;
    let layout = processor_layout();
    let groups = layout
        .iter()
        .map(|(g, _)| *g)
        .max()
        .map(|g| g + 1)
        .unwrap_or(0);
    let _ = get_cpu_info_pdh(); // first sample primes the counters (returns zeros)
    std::thread::sleep(std::time::Duration::from_millis(300));
    let (usage, breakdown) = get_cpu_info_pdh();

    let mut out = String::new();
    let _ = writeln!(out, "Logical processors: {}", layout.len());
    let _ = writeln!(out, "Processor groups:   {groups}");
    let per_group: std::collections::BTreeMap<u16, usize> =
        layout
            .iter()
            .fold(std::collections::BTreeMap::new(), |mut m, (g, _)| {
                *m.entry(*g).or_default() += 1;
                m
            });
    for (g, n) in &per_group {
        let _ = writeln!(out, "  group {g}: {n} processors");
    }
    let _ = writeln!(out, "\nper-core usage (after 300ms):");
    for (i, (u, bd)) in usage.iter().zip(breakdown.iter()).enumerate() {
        let _ = writeln!(
            out,
            "  CPU {i:>3}: {}%  (user {} sys {})",
            crate::numfmt::tenths_str(*u, 5),
            crate::numfmt::tenths_str(bd.user, 0),
            crate::numfmt::tenths_str(bd.system, 0)
        );
    }
    out
}

#[cfg(not(windows))]
pub fn debug_dump() -> String {
    "CPU / processor-group diagnostics are only available on Windows".to_string()
}
