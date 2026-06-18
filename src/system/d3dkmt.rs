//! GPU and NPU adapter monitoring via D3DKMT.
//!
//! dxgkrnl manages render-capable adapters (GPUs) and MCDM (Microsoft Compute
//! Driver Model) compute-only adapters (NPUs) through the same scheduling and
//! memory infrastructure. Task Manager's GPU/NPU columns read undocumented PDH
//! countersets, so we use the documented D3DKMT statistics DDI instead (the
//! same route SystemInformer takes): node running-time deltas for utilization,
//! segment commitments for memory. One enumeration pass tracks both adapter
//! classes so machines with a GPU and an NPU pay for a single state machine.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Wdk::Graphics::Direct3D::{
    D3DKMT_ADAPTERINFO, D3DKMT_ADAPTERREGISTRYINFO, D3DKMT_ADAPTERTYPE, D3DKMT_CLOSEADAPTER,
    D3DKMT_ENUMADAPTERS2, D3DKMT_QUERYADAPTERINFO, D3DKMT_QUERYSTATISTICS,
    D3DKMT_QUERYSTATISTICS_ADAPTER, D3DKMT_QUERYSTATISTICS_NODE,
    D3DKMT_QUERYSTATISTICS_PROCESS_NODE, D3DKMT_QUERYSTATISTICS_PROCESS_SEGMENT,
    D3DKMT_QUERYSTATISTICS_QUERY_NODE, D3DKMT_QUERYSTATISTICS_QUERY_SEGMENT,
    D3DKMT_QUERYSTATISTICS_SEGMENT, D3DKMTCloseAdapter, D3DKMTEnumAdapters2,
    D3DKMTQueryAdapterInfo, D3DKMTQueryStatistics, KMTQAITYPE_ADAPTERREGISTRYINFO,
    KMTQAITYPE_ADAPTERTYPE,
};
use windows::Win32::Foundation::{CloseHandle, LUID};

/// System-wide adapter snapshot stored in `SystemMetrics` (cheap to clone).
#[derive(Clone, Default)]
pub struct AdapterMetrics {
    /// Adapter name from the driver, e.g. "Intel(R) AI Boost"
    pub name: String,
    /// 0-100, max across all engine nodes (Task Manager semantics)
    pub utilization: f32,
    /// Dedicated + shared bytes in use
    pub mem_used: u64,
    /// Sum of segment commit limits (0 if the driver reports none)
    pub mem_total: u64,
    pub dedicated_used: u64,
    /// Sum of non-aperture segment commit limits (VRAM size on a discrete GPU)
    pub dedicated_total: u64,
    pub shared_used: u64,
}

impl AdapterMetrics {
    /// Memory pool the meter should display, as (used, total).
    ///
    /// A discrete GPU exposes its VRAM as a large dedicated pool, so we show
    /// that. An integrated GPU exposes only a small dedicated carveout (often
    /// 128 MB, ~0 used) and does its real work in shared system memory — so for
    /// those we show the combined pool instead, which is where the usage lives.
    /// The 1 GiB threshold cleanly separates an iGPU carveout from real VRAM.
    pub fn meter_memory(&self) -> (u64, u64) {
        const DISCRETE_VRAM_MIN: u64 = 1 << 30; // 1 GiB
        if self.dedicated_total >= DISCRETE_VRAM_MIN {
            (self.dedicated_used, self.dedicated_total)
        } else {
            (self.mem_used, self.mem_total)
        }
    }
}

pub type NpuInfo = AdapterMetrics;
pub type GpuInfo = AdapterMetrics;

/// Per-class results of one refresh pass over all tracked adapters.
#[derive(Clone, Default)]
pub struct AdapterSnapshot {
    pub gpu: Option<GpuInfo>,
    pub npu: Option<NpuInfo>,
}

/// Per-process GPU/NPU usage row.
#[derive(Clone, Copy, Default)]
pub struct ProcAdapterStats {
    /// 0-100, max across all GPU engine nodes of all GPU adapters
    pub gpu_percent: f32,
    /// Committed bytes across all GPU adapters
    pub gpu_memory: u64,
    /// 0-100, max across NPU engine nodes
    pub npu_percent: f32,
    /// Dedicated + shared committed bytes
    pub npu_memory: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AdapterClass {
    Gpu,
    Npu,
}

/// Adapter LUID flattened to a comparable/sortable key: (LowPart, HighPart).
type LuidKey = (u32, i32);

/// One detected dxgkrnl adapter (render-capable or MCDM compute-only).
struct TrackedAdapter {
    class: AdapterClass,
    luid: LUID,
    name: String,
    node_count: u32,
    segment_count: u32,
    /// Per segment: true = aperture (shared system memory)
    aperture_segment: Vec<bool>,
    /// Per node: previous global RunningTime (100ns units)
    prev_node_running: Vec<i64>,
}

/// Delta-tracking state. Lives in a static (not SystemMetrics) because the
/// metrics struct is cloned for every snapshot sent to the UI thread.
#[derive(Default)]
struct AdapterState {
    adapters: Vec<TrackedAdapter>,
    detected: bool,
    /// Set when a statistics query fails (driver reset); triggers re-detection
    needs_reenumeration: bool,
    last_sample: Option<Instant>,
    last_proc_sample: Option<Instant>,
    /// (pid, process create time) -> previous RunningTime per node, flattened across adapters
    prev_proc_running: HashMap<(u32, u64), Vec<i64>>,
    /// (gpu, npu) gates last seen by process_stats; a change invalidates the
    /// flattened baselines (a disabled class keeps stale RunningTime values)
    prev_proc_gates: (bool, bool),
    /// Sorted LUIDs of every adapter seen at the last (re-)detection; compared
    /// against fresh enumerations to spot topology changes (driver installs,
    /// removals) that never fail an existing statistics query
    enumerated_luids: Vec<LuidKey>,
    /// LUIDs of render/compute adapters that enumerated but whose statistics
    /// were unavailable at detection (commonly a discrete GPU in a low-power
    /// state). Non-empty forces a retry on the next topology check so the
    /// adapter appears once it powers up.
    pending_luids: Vec<LuidKey>,
    last_topology_check: Option<Instant>,
    /// Last successful snapshot, served during the one-tick window after a
    /// statistics failure so adapter presence (meters, hardware-aware default
    /// columns) doesn't flicker off while re-enumeration happens
    last_snapshot: AdapterSnapshot,
}

/// Re-check adapter topology this often: catches drivers installed or removed
/// while running. One enumeration syscall when nothing changed.
const TOPOLOGY_CHECK_INTERVAL: Duration = Duration::from_secs(30);

static ADAPTER_STATE: Mutex<Option<AdapterState>> = Mutex::new(None);

/// Per-process collection gates, set from the UI thread (a GPU/NPU column is
/// visible or sorted), read by the data-collector thread. Per-process stats
/// cost a handle open plus a few syscalls per process per tick, so each class
/// is off unless shown.
static GPU_PROCESS_STATS_ENABLED: AtomicBool = AtomicBool::new(false);
static NPU_PROCESS_STATS_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_gpu_process_stats_enabled(enabled: bool) {
    GPU_PROCESS_STATS_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn set_npu_process_stats_enabled(enabled: bool) {
    NPU_PROCESS_STATS_ENABLED.store(enabled, Ordering::Relaxed);
}

/// User's pinned GPU adapter (by name). None = auto-select the card with the
/// most dedicated VRAM. Lets multi-GPU machines choose which card the meter
/// and per-process columns follow.
static GPU_SELECTION: Mutex<Option<String>> = Mutex::new(None);

pub fn set_gpu_selection(name: Option<String>) {
    *GPU_SELECTION.lock().unwrap() = name;
}

/// Names of the GPU adapters currently tracked, in enumeration order, for the
/// Setup selector. Empty when no GPU is present or detection hasn't run yet.
pub fn gpu_names() -> Vec<String> {
    ADAPTER_STATE
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| {
            s.adapters
                .iter()
                .filter(|a| a.class == AdapterClass::Gpu)
                .map(|a| a.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Cheap (lock-free) pre-check for whether per-process GPU/NPU stats might be
/// wanted. When this is false the caller can skip building the all-PIDs vector
/// and pass an empty slice to `process_stats` (which still runs its gate-change
/// bookkeeping). `process_stats` remains the authoritative gate — it also
/// requires the matching adapter class to actually exist.
pub fn process_stats_enabled() -> bool {
    GPU_PROCESS_STATS_ENABLED.load(Ordering::Relaxed)
        || NPU_PROCESS_STATS_ENABLED.load(Ordering::Relaxed)
}

/// Adapter classification from D3DKMT_ADAPTERTYPE flags. Software renderers
/// (bit 2, e.g. Microsoft Basic Render Driver) are excluded outright; MCDM
/// compute-only adapters (bit 11) are NPUs; anything else render-capable
/// (bit 0) is a GPU.
#[inline]
fn classify_adapter(adapter_type_value: u32) -> Option<AdapterClass> {
    const RENDER_SUPPORTED: u32 = 1 << 0;
    const SOFTWARE_DEVICE: u32 = 1 << 2;
    const COMPUTE_ONLY: u32 = 1 << 11;
    if adapter_type_value & SOFTWARE_DEVICE != 0 {
        return None;
    }
    if adapter_type_value & COMPUTE_ONLY != 0 {
        return Some(AdapterClass::Npu);
    }
    if adapter_type_value & RENDER_SUPPORTED != 0 {
        return Some(AdapterClass::Gpu);
    }
    None
}

/// Δ RunningTime (100ns units) over Δ wall-clock, as a clamped percentage.
/// Negative deltas (driver reset, PID reuse) clamp to zero.
#[inline]
fn running_time_to_percent(prev: i64, cur: i64, wall_elapsed_secs: f64) -> f32 {
    if wall_elapsed_secs <= 0.0 {
        return 0.0;
    }
    let delta = (cur - prev).max(0) as f64;
    ((delta / 10_000_000.0) / wall_elapsed_secs * 100.0).clamp(0.0, 100.0) as f32
}

/// Decode a NUL-terminated UTF-16 buffer.
fn utf16_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// D3DKMTQueryStatistics takes `*const` (mirroring d3dkmthk.h) but writes the
/// result through it; route the pointer through `&mut` so the mutation is sound.
#[inline]
unsafe fn query_statistics(query: &mut D3DKMT_QUERYSTATISTICS) -> bool {
    unsafe { D3DKMTQueryStatistics(&raw const *query).is_ok() }
}

/// Run D3DKMTEnumAdapters2 and hand each enumerated adapter to `f`, closing
/// the adapter handles afterwards (statistics queries take the LUID, so no
/// handle is retained).
fn for_each_enumerated_adapter(mut f: impl FnMut(&D3DKMT_ADAPTERINFO)) {
    unsafe {
        // First call with a null buffer returns the adapter count.
        let mut enum2 = D3DKMT_ENUMADAPTERS2::default();
        if D3DKMTEnumAdapters2(&mut enum2).is_err() || enum2.NumAdapters == 0 {
            return;
        }
        let mut infos: Vec<D3DKMT_ADAPTERINFO> =
            vec![D3DKMT_ADAPTERINFO::default(); enum2.NumAdapters as usize];
        enum2.pAdapters = infos.as_mut_ptr();
        if D3DKMTEnumAdapters2(&mut enum2).is_err() {
            return;
        }

        for info in &infos[..enum2.NumAdapters as usize] {
            if info.hAdapter == 0 {
                continue;
            }
            f(info);
            let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER {
                hAdapter: info.hAdapter,
            });
        }
    }
}

/// Result of probing one enumerated adapter.
enum AdapterProbe {
    /// A GPU/NPU we can monitor right now.
    Tracked(TrackedAdapter),
    /// A render/compute adapter whose statistics are currently unavailable
    /// (commonly a discrete GPU parked in a low-power D3 state on a hybrid
    /// laptop). We retry these so they appear once powered up.
    Pending,
    /// Not a GPU/NPU (software/WARP, display-only) or unidentifiable; ignored.
    Untrackable,
}

/// Enumerate dxgkrnl adapters; returns the GPUs/NPUs we can track now, the
/// sorted LUID set of everything enumerated (for the periodic topology check),
/// and the sorted LUIDs of trackable-but-currently-unavailable adapters to retry.
fn detect_adapters() -> (Vec<TrackedAdapter>, Vec<LuidKey>, Vec<LuidKey>) {
    let mut adapters = Vec::new();
    let mut luids = Vec::new();
    let mut pending = Vec::new();
    for_each_enumerated_adapter(|info| {
        let luid = (info.AdapterLuid.LowPart, info.AdapterLuid.HighPart);
        luids.push(luid);
        match build_adapter(info) {
            AdapterProbe::Tracked(adapter) => adapters.push(adapter),
            AdapterProbe::Pending => pending.push(luid),
            AdapterProbe::Untrackable => {}
        }
    });
    luids.sort_unstable();
    pending.sort_unstable();
    (adapters, luids, pending)
}

/// Sorted LUIDs of every adapter dxgkrnl currently exposes — the cheap probe
/// behind the periodic topology check.
fn enumerate_luids() -> Vec<LuidKey> {
    let mut luids = Vec::new();
    for_each_enumerated_adapter(|info| {
        luids.push((info.AdapterLuid.LowPart, info.AdapterLuid.HighPart));
    });
    luids.sort_unstable();
    luids
}

/// Classify and probe one enumerated adapter.
fn build_adapter(info: &D3DKMT_ADAPTERINFO) -> AdapterProbe {
    unsafe {
        // Adapter type: skip software devices and anything we don't track.
        let mut adapter_type = D3DKMT_ADAPTERTYPE::default();
        let mut query = D3DKMT_QUERYADAPTERINFO {
            hAdapter: info.hAdapter,
            Type: KMTQAITYPE_ADAPTERTYPE,
            pPrivateDriverData: &mut adapter_type as *mut _ as *mut std::ffi::c_void,
            PrivateDriverDataSize: std::mem::size_of::<D3DKMT_ADAPTERTYPE>() as u32,
        };
        if D3DKMTQueryAdapterInfo(&mut query).is_err() {
            // Can't even read the type (deep power-down / driver hiccup). We
            // don't know if it's a GPU, so don't retry it as pending.
            return AdapterProbe::Untrackable;
        }
        let Some(class) = classify_adapter(adapter_type.Anonymous.Value) else {
            return AdapterProbe::Untrackable;
        };
        let fallback_name = match class {
            AdapterClass::Gpu => "GPU",
            AdapterClass::Npu => "NPU",
        };

        // Friendly name from the driver registry info (best effort).
        let mut registry_info = D3DKMT_ADAPTERREGISTRYINFO::default();
        let mut query = D3DKMT_QUERYADAPTERINFO {
            hAdapter: info.hAdapter,
            Type: KMTQAITYPE_ADAPTERREGISTRYINFO,
            pPrivateDriverData: &mut registry_info as *mut _ as *mut std::ffi::c_void,
            PrivateDriverDataSize: std::mem::size_of::<D3DKMT_ADAPTERREGISTRYINFO>() as u32,
        };
        let name = if D3DKMTQueryAdapterInfo(&mut query).is_ok() {
            let s = utf16_to_string(&registry_info.AdapterString);
            if s.is_empty() {
                fallback_name.to_string()
            } else {
                s
            }
        } else {
            fallback_name.to_string()
        };

        // Node/segment counts for the statistics loops.
        let mut stats = D3DKMT_QUERYSTATISTICS {
            Type: D3DKMT_QUERYSTATISTICS_ADAPTER,
            AdapterLuid: info.AdapterLuid,
            ..Default::default()
        };
        if !query_statistics(&mut stats) {
            // It's a GPU/NPU class but statistics aren't available yet (likely
            // powered down). Mark pending so refresh() retries it.
            return AdapterProbe::Pending;
        }
        let node_count = stats.QueryResult.AdapterInformation.NodeCount;
        let segment_count = stats.QueryResult.AdapterInformation.NbSegments;

        // Aperture flag per segment (distinguishes shared vs dedicated memory).
        let mut aperture_segment = Vec::with_capacity(segment_count as usize);
        for segment_id in 0..segment_count {
            let mut stats = D3DKMT_QUERYSTATISTICS {
                Type: D3DKMT_QUERYSTATISTICS_SEGMENT,
                AdapterLuid: info.AdapterLuid,
                ..Default::default()
            };
            stats.Anonymous.QuerySegment = D3DKMT_QUERYSTATISTICS_QUERY_SEGMENT {
                SegmentId: segment_id,
            };
            let aperture =
                query_statistics(&mut stats) && stats.QueryResult.SegmentInformation.Aperture != 0;
            aperture_segment.push(aperture);
        }

        AdapterProbe::Tracked(TrackedAdapter {
            class,
            luid: info.AdapterLuid,
            name,
            node_count,
            segment_count,
            aperture_segment,
            prev_node_running: vec![0; node_count as usize],
        })
    }
}

/// Query utilization and memory for one adapter. Returns None on a statistics
/// failure (driver reset / adapter removal), which triggers re-enumeration.
fn query_adapter_metrics(
    adapter: &mut TrackedAdapter,
    elapsed: Option<f64>,
) -> Option<AdapterMetrics> {
    let mut metrics = AdapterMetrics {
        name: adapter.name.clone(),
        ..Default::default()
    };

    // Utilization: max across engine nodes of the running-time deltas.
    for node_id in 0..adapter.node_count {
        let mut stats = D3DKMT_QUERYSTATISTICS {
            Type: D3DKMT_QUERYSTATISTICS_NODE,
            AdapterLuid: adapter.luid,
            ..Default::default()
        };
        stats.Anonymous.QueryNode = D3DKMT_QUERYSTATISTICS_QUERY_NODE { NodeId: node_id };
        if !unsafe { query_statistics(&mut stats) } {
            return None;
        }
        let running = unsafe {
            stats
                .QueryResult
                .NodeInformation
                .GlobalInformation
                .RunningTime
        };
        if let Some(secs) = elapsed {
            let pct =
                running_time_to_percent(adapter.prev_node_running[node_id as usize], running, secs);
            metrics.utilization = metrics.utilization.max(pct);
        }
        adapter.prev_node_running[node_id as usize] = running;
    }

    // Memory: resident bytes per segment, split by aperture (shared) flag.
    for segment_id in 0..adapter.segment_count {
        let mut stats = D3DKMT_QUERYSTATISTICS {
            Type: D3DKMT_QUERYSTATISTICS_SEGMENT,
            AdapterLuid: adapter.luid,
            ..Default::default()
        };
        stats.Anonymous.QuerySegment = D3DKMT_QUERYSTATISTICS_QUERY_SEGMENT {
            SegmentId: segment_id,
        };
        if !unsafe { query_statistics(&mut stats) } {
            return None;
        }
        let segment = unsafe { stats.QueryResult.SegmentInformation };
        // Some drivers (e.g. Adreno) report a u64::MAX "no limit" CommitLimit on
        // aperture/shared segments. Treat it as 0 so it neither overflows the
        // total (panic in debug, garbage wrap in release) nor reports a nonsense
        // multi-exabyte capacity. saturating_add guards the remaining sums too.
        let limit = if segment.CommitLimit == u64::MAX {
            0
        } else {
            segment.CommitLimit
        };
        let used = if segment.BytesResident > 0 {
            segment.BytesResident
        } else {
            segment.BytesCommitted
        };
        if adapter
            .aperture_segment
            .get(segment_id as usize)
            .copied()
            .unwrap_or(false)
        {
            metrics.shared_used = metrics.shared_used.saturating_add(used);
        } else {
            metrics.dedicated_used = metrics.dedicated_used.saturating_add(used);
            metrics.dedicated_total = metrics.dedicated_total.saturating_add(limit);
        }
        metrics.mem_total = metrics.mem_total.saturating_add(limit);
    }

    metrics.mem_used = metrics.dedicated_used.saturating_add(metrics.shared_used);
    Some(metrics)
}

/// Aggregate NPU adapters: max utilization, summed memory, first adapter's
/// name with a "(+N)" suffix when more exist.
fn aggregate_npus(per_adapter: &[(AdapterClass, AdapterMetrics)]) -> Option<AdapterMetrics> {
    let mut npus = per_adapter
        .iter()
        .filter(|(class, _)| *class == AdapterClass::Npu)
        .map(|(_, metrics)| metrics);
    let mut info = npus.next()?.clone();
    let mut extra = 0;
    for m in npus {
        info.utilization = info.utilization.max(m.utilization);
        info.mem_used += m.mem_used;
        info.mem_total += m.mem_total;
        info.dedicated_used += m.dedicated_used;
        info.dedicated_total += m.dedicated_total;
        info.shared_used += m.shared_used;
        extra += 1;
    }
    if extra > 0 {
        info.name.push_str(&format!(" (+{})", extra));
    }
    Some(info)
}

/// Pick the primary GPU. If the user pinned an adapter by name (Setup), use the
/// first GPU matching it; otherwise auto-select the largest dedicated commit
/// limit (the discrete card over an iGPU), with ties and all-zero falling back
/// to enumeration order. The meter shows only the primary adapter — summing one
/// card's VRAM with another adapter's aperture limit would produce nonsense.
fn aggregate_gpus(per_adapter: &[(AdapterClass, AdapterMetrics)]) -> Option<AdapterMetrics> {
    let selection = GPU_SELECTION.lock().unwrap().clone();
    let mut primary: Option<&AdapterMetrics> = None;
    let mut selected: Option<&AdapterMetrics> = None;
    let mut count = 0;
    for (class, metrics) in per_adapter {
        if *class != AdapterClass::Gpu {
            continue;
        }
        count += 1;
        if let Some(ref sel) = selection
            && selected.is_none()
            && metrics.name == *sel
        {
            selected = Some(metrics);
        }
        if primary.is_none_or(|p| metrics.dedicated_total > p.dedicated_total) {
            primary = Some(metrics);
        }
    }
    // A pinned-but-absent adapter falls back to auto so the meter never blanks.
    let mut info = selected.or(primary)?.clone();
    if count > 1 {
        info.name.push_str(&format!(" (+{})", count - 1));
    }
    Some(info)
}

/// Refresh system-wide GPU/NPU metrics. Both fields are None when no tracked
/// adapter exists. Steady state is a mutex lock plus the statistics queries;
/// every TOPOLOGY_CHECK_INTERVAL one extra enumeration syscall compares the
/// adapter LUID set so drivers installed or removed while running are picked
/// up without a restart.
pub fn refresh() -> AdapterSnapshot {
    let mut guard = ADAPTER_STATE.lock().unwrap();
    let state = guard.get_or_insert_with(AdapterState::default);
    let now = Instant::now();

    // Periodic topology check. A changed LUID set (new NPU driver, removed
    // eGPU, ...) forces full re-detection below; an unchanged set costs one
    // syscall and leaves all delta baselines untouched.
    if state.detected
        && !state.needs_reenumeration
        && state
            .last_topology_check
            .is_none_or(|t| now.duration_since(t) >= TOPOLOGY_CHECK_INTERVAL)
    {
        state.last_topology_check = Some(now);
        // Re-detect when the adapter set changed, or when a previously-down
        // adapter is pending — retry it in case it has since powered up.
        if enumerate_luids() != state.enumerated_luids || !state.pending_luids.is_empty() {
            state.needs_reenumeration = true;
        }
    }

    if !state.detected || state.needs_reenumeration {
        let old = std::mem::take(&mut state.adapters);
        let (mut adapters, luids, pending) = detect_adapters();
        // Preserve per-adapter utilization baselines for adapters that survived
        // the re-detection (matched by LUID), so a re-enumeration triggered by a
        // pending/transient adapter doesn't reset the working adapters' meters.
        // Seed baselines for freshly-appeared adapters so their first sample
        // reads ~0% instead of spiking to 100%.
        for a in &mut adapters {
            match old
                .iter()
                .find(|o| o.luid.LowPart == a.luid.LowPart && o.luid.HighPart == a.luid.HighPart)
            {
                Some(o) if o.prev_node_running.len() == a.prev_node_running.len() => {
                    a.prev_node_running.clone_from(&o.prev_node_running);
                }
                _ => {
                    let _ = query_adapter_metrics(a, None);
                }
            }
        }
        state.adapters = adapters;
        state.enumerated_luids = luids;
        state.pending_luids = pending;
        state.detected = true;
        state.needs_reenumeration = false;
        state.last_topology_check = Some(now);
        // Keep last_sample so surviving adapters retain utilization continuity
        // (a counter reset clamps to 0% for one tick via running_time_to_percent).
        // The per-process node layout may have shifted, so reset those baselines.
        state.last_proc_sample = None;
        state.prev_proc_running.clear();
    }
    if state.adapters.is_empty() {
        state.last_snapshot = AdapterSnapshot::default();
        return AdapterSnapshot::default();
    }

    // First sample after (re-)detection only sets baselines and reports 0%.
    let elapsed = state
        .last_sample
        .map(|t| now.duration_since(t).as_secs_f64());
    state.last_sample = Some(now);

    let mut per_adapter = Vec::with_capacity(state.adapters.len());
    for adapter in &mut state.adapters {
        let Some(metrics) = query_adapter_metrics(adapter, elapsed) else {
            // Driver reset or adapter removal: re-enumerate on the next tick.
            // Serve the last good snapshot meanwhile so adapter presence
            // (meters, hardware-aware default columns) doesn't blink off for
            // one tick; re-detection decides whether it's really gone.
            state.needs_reenumeration = true;
            return state.last_snapshot.clone();
        };
        per_adapter.push((adapter.class, metrics));
    }

    let snapshot = AdapterSnapshot {
        gpu: aggregate_gpus(&per_adapter),
        npu: aggregate_npus(&per_adapter),
    };
    state.last_snapshot = snapshot.clone();
    snapshot
}

/// Per-process GPU/NPU stats for the given PIDs. A class is only queried when
/// its adapters exist and its `set_*_process_stats_enabled(true)` gate is on;
/// with both gates off this returns an empty map without opening any handles.
pub fn process_stats(processes: &[(u32, u64)]) -> HashMap<u32, ProcAdapterStats> {
    let mut guard = ADAPTER_STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else {
        return HashMap::new();
    };

    let gpu_enabled = GPU_PROCESS_STATS_ENABLED.load(Ordering::Relaxed)
        && state.adapters.iter().any(|a| a.class == AdapterClass::Gpu);
    let npu_enabled = NPU_PROCESS_STATS_ENABLED.load(Ordering::Relaxed)
        && state.adapters.iter().any(|a| a.class == AdapterClass::Npu);

    // Nodes of a disabled class keep stale RunningTime baselines, so any gate
    // change restarts delta tracking from a clean first sample.
    if (gpu_enabled, npu_enabled) != state.prev_proc_gates {
        state.prev_proc_gates = (gpu_enabled, npu_enabled);
        state.prev_proc_running.clear();
        state.last_proc_sample = None;
    }
    if !gpu_enabled && !npu_enabled {
        return HashMap::new();
    }

    let now = Instant::now();
    let elapsed = state
        .last_proc_sample
        .map(|t| now.duration_since(t).as_secs_f64());
    state.last_proc_sample = Some(now);

    // Split borrows so adapters and the baseline map can be used together.
    let AdapterState {
        adapters,
        prev_proc_running,
        ..
    } = state;
    // Baseline slots cover every node of every adapter so the flattened
    // indices stay stable regardless of which classes are queried.
    let total_nodes: usize = adapters.iter().map(|a| a.node_count as usize).sum();

    let mut result = HashMap::with_capacity(processes.len());
    let mut current_keys = HashSet::with_capacity(processes.len());
    for &(pid, create_time) in processes {
        let key = (pid, create_time);
        // Idle and System pseudo-processes can't be opened.
        if pid == 0 || pid == 4 {
            continue;
        }
        let Some(handle) = super::process::open_process_query(pid) else {
            continue;
        };

        // A PID seen for the first time only sets baselines (reports 0%).
        let fresh = !prev_proc_running.contains_key(&key);
        let prev = prev_proc_running
            .entry(key)
            .or_insert_with(|| vec![0; total_nodes]);

        let mut entry = ProcAdapterStats::default();
        let mut node_index = 0;
        let mut sampled_running_time = false;
        for adapter in adapters.iter() {
            let class_enabled = match adapter.class {
                AdapterClass::Gpu => gpu_enabled,
                AdapterClass::Npu => npu_enabled,
            };
            if !class_enabled {
                node_index += adapter.node_count as usize;
                continue;
            }
            for node_id in 0..adapter.node_count {
                let mut stats = D3DKMT_QUERYSTATISTICS {
                    Type: D3DKMT_QUERYSTATISTICS_PROCESS_NODE,
                    AdapterLuid: adapter.luid,
                    hProcess: handle,
                    ..Default::default()
                };
                stats.Anonymous.QueryProcessNode =
                    D3DKMT_QUERYSTATISTICS_QUERY_NODE { NodeId: node_id };
                // Failures here are per-process (exited, no adapter reference);
                // leave zeros rather than tearing down the adapter state.
                if unsafe { query_statistics(&mut stats) } {
                    sampled_running_time = true;
                    let running = unsafe { stats.QueryResult.ProcessNodeInformation.RunningTime };
                    if !fresh && let Some(secs) = elapsed {
                        let pct = running_time_to_percent(prev[node_index], running, secs);
                        match adapter.class {
                            AdapterClass::Gpu => entry.gpu_percent = entry.gpu_percent.max(pct),
                            AdapterClass::Npu => entry.npu_percent = entry.npu_percent.max(pct),
                        }
                    }
                    prev[node_index] = running;
                }
                node_index += 1;
            }
            for segment_id in 0..adapter.segment_count {
                let mut stats = D3DKMT_QUERYSTATISTICS {
                    Type: D3DKMT_QUERYSTATISTICS_PROCESS_SEGMENT,
                    AdapterLuid: adapter.luid,
                    hProcess: handle,
                    ..Default::default()
                };
                stats.Anonymous.QueryProcessSegment = D3DKMT_QUERYSTATISTICS_QUERY_SEGMENT {
                    SegmentId: segment_id,
                };
                if unsafe { query_statistics(&mut stats) } {
                    let committed =
                        unsafe { stats.QueryResult.ProcessSegmentInformation.BytesCommitted };
                    // Treat the u64::MAX "no limit" sentinel as 0 and saturate so
                    // a quirky driver can't overflow the per-process total.
                    let committed = if committed == u64::MAX { 0 } else { committed };
                    match adapter.class {
                        AdapterClass::Gpu => {
                            entry.gpu_memory = entry.gpu_memory.saturating_add(committed)
                        }
                        AdapterClass::Npu => {
                            entry.npu_memory = entry.npu_memory.saturating_add(committed)
                        }
                    }
                }
            }
        }
        unsafe {
            let _ = CloseHandle(handle);
        }
        if sampled_running_time {
            current_keys.insert(key);
        }
        result.insert(pid, entry);
    }

    // Prune baselines for PIDs that died or could no longer be opened.
    prev_proc_running.retain(|key, _| current_keys.contains(key));

    result
}

/// Human-readable dump of every dxgkrnl adapter and its raw segment statistics,
/// for diagnosing GPU/NPU memory reporting (`--gpu-debug`). Enumerates ALL
/// adapters, including ones we don't track, so a dropped/powered-down discrete
/// GPU is visible.
pub fn debug_dump() -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let mut idx = 0usize;
    for_each_enumerated_adapter(|info| {
        let _ = writeln!(
            out,
            "Adapter #{idx}  LUID {:#010x}:{}",
            info.AdapterLuid.LowPart, info.AdapterLuid.HighPart
        );
        idx += 1;
        unsafe {
            // Adapter type + classification.
            let mut atype = D3DKMT_ADAPTERTYPE::default();
            let mut q = D3DKMT_QUERYADAPTERINFO {
                hAdapter: info.hAdapter,
                Type: KMTQAITYPE_ADAPTERTYPE,
                pPrivateDriverData: &mut atype as *mut _ as *mut std::ffi::c_void,
                PrivateDriverDataSize: std::mem::size_of::<D3DKMT_ADAPTERTYPE>() as u32,
            };
            let type_ok = D3DKMTQueryAdapterInfo(&mut q).is_ok();
            let tval = if type_ok { atype.Anonymous.Value } else { 0 };
            let class = match classify_adapter(tval) {
                Some(AdapterClass::Gpu) => "GPU",
                Some(AdapterClass::Npu) => "NPU",
                None => "(untracked)",
            };

            // Friendly name.
            let mut reg = D3DKMT_ADAPTERREGISTRYINFO::default();
            let mut q2 = D3DKMT_QUERYADAPTERINFO {
                hAdapter: info.hAdapter,
                Type: KMTQAITYPE_ADAPTERREGISTRYINFO,
                pPrivateDriverData: &mut reg as *mut _ as *mut std::ffi::c_void,
                PrivateDriverDataSize: std::mem::size_of::<D3DKMT_ADAPTERREGISTRYINFO>() as u32,
            };
            let name = if D3DKMTQueryAdapterInfo(&mut q2).is_ok() {
                utf16_to_string(&reg.AdapterString)
            } else {
                "?".to_string()
            };
            let _ = writeln!(out, "  name : {name}");
            let _ = writeln!(
                out,
                "  type : {tval:#06x} (queryOk={type_ok}) class={class}"
            );

            // Node/segment counts.
            let mut stats = D3DKMT_QUERYSTATISTICS {
                Type: D3DKMT_QUERYSTATISTICS_ADAPTER,
                AdapterLuid: info.AdapterLuid,
                ..Default::default()
            };
            if !query_statistics(&mut stats) {
                let _ = writeln!(
                    out,
                    "  ADAPTER stats query FAILED (adapter likely powered down)"
                );
                return;
            }
            let node_count = stats.QueryResult.AdapterInformation.NodeCount;
            let seg_count = stats.QueryResult.AdapterInformation.NbSegments;
            let _ = writeln!(out, "  nodes={node_count} segments={seg_count}");

            let (mut ded_used, mut ded_total, mut shared_used, mut mem_total) =
                (0u64, 0u64, 0u64, 0u64);
            for sid in 0..seg_count {
                let mut s = D3DKMT_QUERYSTATISTICS {
                    Type: D3DKMT_QUERYSTATISTICS_SEGMENT,
                    AdapterLuid: info.AdapterLuid,
                    ..Default::default()
                };
                s.Anonymous.QuerySegment = D3DKMT_QUERYSTATISTICS_QUERY_SEGMENT { SegmentId: sid };
                if !query_statistics(&mut s) {
                    let _ = writeln!(out, "    seg {sid}: query FAILED");
                    continue;
                }
                let seg = s.QueryResult.SegmentInformation;
                let ap = seg.Aperture != 0;
                let used = if seg.BytesResident > 0 {
                    seg.BytesResident
                } else {
                    seg.BytesCommitted
                };
                let _ = writeln!(
                    out,
                    "    seg {sid}: aperture={ap} commitLimit={cl:#x} ({cl}) committed={cm:#x} resident={rs:#x}",
                    cl = seg.CommitLimit,
                    cm = seg.BytesCommitted,
                    rs = seg.BytesResident
                );
                if ap {
                    shared_used = shared_used.saturating_add(used);
                } else {
                    ded_used = ded_used.saturating_add(used);
                    ded_total = ded_total.saturating_add(seg.CommitLimit);
                }
                mem_total = mem_total.saturating_add(seg.CommitLimit);
            }
            let _ = writeln!(
                out,
                "  => dedicated {ded_used}/{ded_total}  shared_used={shared_used}  mem_total={mem_total}"
            );
        }
    });
    if idx == 0 {
        out.push_str("No dxgkrnl adapters enumerated.\n");
    }

    // End-to-end: what the meter would show via the real refresh()+aggregation.
    let snap = refresh();
    let _ = writeln!(out, "\n--- meter (via refresh + aggregation) ---");
    if let Some(g) = &snap.gpu {
        let (u, t) = g.meter_memory();
        let _ = writeln!(
            out,
            "GPU: {} util={:.1}% mem={u}/{t} (dedicated {}/{}, shared_used {}, mem_total {})",
            g.name, g.utilization, g.dedicated_used, g.dedicated_total, g.shared_used, g.mem_total
        );
    } else {
        let _ = writeln!(out, "GPU: none");
    }
    if let Some(n) = &snap.npu {
        let (u, t) = n.meter_memory();
        let _ = writeln!(
            out,
            "NPU: {} util={:.1}% mem={u}/{t}",
            n.name, n.utilization
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterClass, AdapterMetrics, aggregate_gpus, aggregate_npus, classify_adapter,
        running_time_to_percent, utf16_to_string,
    };

    #[test]
    fn test_classify_adapter() {
        // Render-capable (bit 0) -> GPU
        assert!(matches!(classify_adapter(1 << 0), Some(AdapterClass::Gpu)));
        // ComputeOnly (bit 11) alone -> NPU
        assert!(matches!(classify_adapter(1 << 11), Some(AdapterClass::Npu)));
        // SoftwareDevice (bit 2) excludes regardless of other capabilities
        // (WARP / Microsoft Basic Render Driver)
        assert!(classify_adapter((1 << 0) | (1 << 2)).is_none());
        assert!(classify_adapter((1 << 11) | (1 << 2)).is_none());
        // ComputeOnly wins over render-capable
        assert!(matches!(
            classify_adapter((1 << 0) | (1 << 11)),
            Some(AdapterClass::Npu)
        ));
        // Neither render nor compute (e.g. display-only) -> untracked
        assert!(classify_adapter(0).is_none());
        // Other flags alongside ComputeOnly don't disqualify
        assert!(matches!(
            classify_adapter((1 << 11) | (1 << 13)),
            Some(AdapterClass::Npu)
        ));
    }

    fn metrics(name: &str, utilization: f32, dedicated_total: u64) -> AdapterMetrics {
        AdapterMetrics {
            name: name.to_string(),
            utilization,
            mem_used: 100,
            mem_total: 200,
            dedicated_used: 50,
            dedicated_total,
            shared_used: 50,
        }
    }

    #[test]
    fn test_meter_memory_picks_pool_by_vram_size() {
        // Discrete card: large dedicated VRAM -> show the dedicated pool.
        let discrete = AdapterMetrics {
            dedicated_used: 2 << 30,
            dedicated_total: 8 << 30,
            mem_used: 3 << 30,
            mem_total: 24 << 30,
            ..Default::default()
        };
        assert_eq!(discrete.meter_memory(), (2 << 30, 8 << 30));

        // Integrated GPU (Adreno-shaped): tiny unused 128 MB dedicated carveout,
        // real usage in shared -> show the combined pool, not "0/128M".
        let integrated = AdapterMetrics {
            dedicated_used: 0,
            dedicated_total: 128 << 20,
            shared_used: 1_739_767_808,
            mem_used: 1_739_767_808,
            mem_total: 8_722_055_168,
            ..Default::default()
        };
        assert_eq!(integrated.meter_memory(), (1_739_767_808, 8_722_055_168));
    }

    #[test]
    fn test_aggregate_gpus_picks_largest_dedicated() {
        // dGPU (large VRAM) wins over iGPU even when enumerated second
        let per_adapter = vec![
            (AdapterClass::Gpu, metrics("iGPU", 80.0, 128 << 20)),
            (AdapterClass::Gpu, metrics("dGPU", 10.0, 8 << 30)),
            (AdapterClass::Npu, metrics("NPU", 99.0, 0)),
        ];
        let gpu = aggregate_gpus(&per_adapter).unwrap();
        assert_eq!(gpu.name, "dGPU (+1)");
        assert_eq!(gpu.utilization, 10.0);
        assert_eq!(gpu.dedicated_total, 8 << 30);
    }

    #[test]
    fn test_aggregate_gpus_tie_prefers_enumeration_order() {
        let per_adapter = vec![
            (AdapterClass::Gpu, metrics("first", 1.0, 0)),
            (AdapterClass::Gpu, metrics("second", 2.0, 0)),
        ];
        let gpu = aggregate_gpus(&per_adapter).unwrap();
        assert_eq!(gpu.name, "first (+1)");
    }

    #[test]
    fn test_aggregate_gpus_none_without_gpus() {
        let per_adapter = vec![(AdapterClass::Npu, metrics("NPU", 1.0, 0))];
        assert!(aggregate_gpus(&per_adapter).is_none());
        assert!(aggregate_gpus(&[]).is_none());
    }

    #[test]
    fn test_aggregate_npus_max_util_summed_memory() {
        let per_adapter = vec![
            (AdapterClass::Gpu, metrics("GPU", 90.0, 8 << 30)),
            (AdapterClass::Npu, metrics("NPU A", 20.0, 0)),
            (AdapterClass::Npu, metrics("NPU B", 60.0, 0)),
        ];
        let npu = aggregate_npus(&per_adapter).unwrap();
        assert_eq!(npu.name, "NPU A (+1)");
        assert_eq!(npu.utilization, 60.0);
        assert_eq!(npu.mem_used, 200);
        assert_eq!(npu.mem_total, 400);
    }

    #[test]
    fn test_running_time_to_percent() {
        // 0.5s of running time over 1s wall clock = 50%
        let half_sec_100ns = 5_000_000;
        assert_eq!(running_time_to_percent(0, half_sec_100ns, 1.0), 50.0);
        // Negative delta (driver reset / PID reuse) clamps to 0
        assert_eq!(running_time_to_percent(half_sec_100ns, 0, 1.0), 0.0);
        // Over-100% (multi-context overlap) clamps to 100
        assert_eq!(running_time_to_percent(0, 30_000_000, 1.0), 100.0);
        // Zero or negative wall clock yields 0
        assert_eq!(running_time_to_percent(0, half_sec_100ns, 0.0), 0.0);
    }

    #[test]
    fn test_utf16_to_string() {
        let buf: Vec<u16> = "Intel(R) AI Boost\0\0extra".encode_utf16().collect();
        assert_eq!(utf16_to_string(&buf), "Intel(R) AI Boost");
        let no_nul: Vec<u16> = "NPU".encode_utf16().collect();
        assert_eq!(utf16_to_string(&no_nul), "NPU");
    }
}
