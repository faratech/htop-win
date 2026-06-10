//! GPU and NPU adapter monitoring via D3DKMT.
//!
//! dxgkrnl manages render-capable adapters (GPUs) and MCDM (Microsoft Compute
//! Driver Model) compute-only adapters (NPUs) through the same scheduling and
//! memory infrastructure. Task Manager's GPU/NPU columns read undocumented PDH
//! countersets, so we use the documented D3DKMT statistics DDI instead (the
//! same route SystemInformer takes): node running-time deltas for utilization,
//! segment commitments for memory. One enumeration pass tracks both adapter
//! classes so machines with a GPU and an NPU pay for a single state machine.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use windows::Wdk::Graphics::Direct3D::{
    D3DKMTCloseAdapter, D3DKMTEnumAdapters2, D3DKMTQueryAdapterInfo, D3DKMTQueryStatistics,
    D3DKMT_ADAPTERINFO, D3DKMT_ADAPTERREGISTRYINFO, D3DKMT_ADAPTERTYPE, D3DKMT_CLOSEADAPTER,
    D3DKMT_ENUMADAPTERS2, D3DKMT_QUERYADAPTERINFO, D3DKMT_QUERYSTATISTICS,
    D3DKMT_QUERYSTATISTICS_ADAPTER, D3DKMT_QUERYSTATISTICS_NODE,
    D3DKMT_QUERYSTATISTICS_PROCESS_NODE, D3DKMT_QUERYSTATISTICS_PROCESS_SEGMENT,
    D3DKMT_QUERYSTATISTICS_QUERY_NODE, D3DKMT_QUERYSTATISTICS_QUERY_SEGMENT,
    D3DKMT_QUERYSTATISTICS_SEGMENT, KMTQAITYPE_ADAPTERREGISTRYINFO, KMTQAITYPE_ADAPTERTYPE,
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
    /// pid -> previous RunningTime per node, flattened across adapters
    prev_proc_running: HashMap<u32, Vec<i64>>,
    /// (gpu, npu) gates last seen by process_stats; a change invalidates the
    /// flattened baselines (a disabled class keeps stale RunningTime values)
    prev_proc_gates: (bool, bool),
}

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

/// Enumerate dxgkrnl adapters and keep the GPUs and NPUs.
fn detect_adapters() -> Vec<TrackedAdapter> {
    unsafe {
        // First call with a null buffer returns the adapter count.
        let mut enum2 = D3DKMT_ENUMADAPTERS2::default();
        if D3DKMTEnumAdapters2(&mut enum2).is_err() || enum2.NumAdapters == 0 {
            return Vec::new();
        }
        let mut infos: Vec<D3DKMT_ADAPTERINFO> =
            vec![D3DKMT_ADAPTERINFO::default(); enum2.NumAdapters as usize];
        enum2.pAdapters = infos.as_mut_ptr();
        if D3DKMTEnumAdapters2(&mut enum2).is_err() {
            return Vec::new();
        }

        let mut adapters = Vec::new();
        for info in &infos[..enum2.NumAdapters as usize] {
            if info.hAdapter == 0 {
                continue;
            }
            if let Some(adapter) = build_adapter(info) {
                adapters.push(adapter);
            }
            // Statistics queries take the LUID, so no handle is retained.
            let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: info.hAdapter });
        }
        adapters
    }
}

/// Classify one enumerated adapter; returns Some for GPUs and NPUs.
fn build_adapter(info: &D3DKMT_ADAPTERINFO) -> Option<TrackedAdapter> {
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
            return None;
        }
        let class = classify_adapter(adapter_type.Anonymous.Value)?;
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
            if s.is_empty() { fallback_name.to_string() } else { s }
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
            return None;
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

        Some(TrackedAdapter {
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
            stats.QueryResult.NodeInformation.GlobalInformation.RunningTime
        };
        if let Some(secs) = elapsed {
            let pct = running_time_to_percent(
                adapter.prev_node_running[node_id as usize],
                running,
                secs,
            );
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
        let used = if segment.BytesResident > 0 {
            segment.BytesResident
        } else {
            segment.BytesCommitted
        };
        if adapter.aperture_segment.get(segment_id as usize).copied().unwrap_or(false) {
            metrics.shared_used += used;
        } else {
            metrics.dedicated_used += used;
            metrics.dedicated_total += segment.CommitLimit;
        }
        metrics.mem_total += segment.CommitLimit;
    }

    metrics.mem_used = metrics.dedicated_used + metrics.shared_used;
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

/// Pick the primary GPU: largest dedicated commit limit selects the discrete
/// card over an iGPU; ties and all-zero fall back to enumeration order. The
/// meter shows only the primary adapter — summing one card's VRAM with
/// another adapter's aperture limit would produce nonsense totals.
fn aggregate_gpus(per_adapter: &[(AdapterClass, AdapterMetrics)]) -> Option<AdapterMetrics> {
    let mut primary: Option<&AdapterMetrics> = None;
    let mut count = 0;
    for (class, metrics) in per_adapter {
        if *class != AdapterClass::Gpu {
            continue;
        }
        count += 1;
        if primary.is_none_or(|p| metrics.dedicated_total > p.dedicated_total) {
            primary = Some(metrics);
        }
    }
    let mut info = primary?.clone();
    if count > 1 {
        info.name.push_str(&format!(" (+{})", count - 1));
    }
    Some(info)
}

/// Refresh system-wide GPU/NPU metrics. Both fields are None when no tracked
/// adapter exists; after the one-time detection that path is just a mutex
/// lock and an empty check.
pub fn refresh() -> AdapterSnapshot {
    let mut guard = ADAPTER_STATE.lock().unwrap();
    let state = guard.get_or_insert_with(AdapterState::default);

    if !state.detected || state.needs_reenumeration {
        state.adapters = detect_adapters();
        state.detected = true;
        state.needs_reenumeration = false;
        state.last_sample = None;
        state.last_proc_sample = None;
        state.prev_proc_running.clear();
    }
    if state.adapters.is_empty() {
        return AdapterSnapshot::default();
    }

    let now = Instant::now();
    // First sample after (re-)detection only sets baselines and reports 0%.
    let elapsed = state.last_sample.map(|t| now.duration_since(t).as_secs_f64());
    state.last_sample = Some(now);

    let mut per_adapter = Vec::with_capacity(state.adapters.len());
    for adapter in &mut state.adapters {
        let Some(metrics) = query_adapter_metrics(adapter, elapsed) else {
            // Driver reset or adapter removal: re-enumerate on the next tick.
            state.needs_reenumeration = true;
            return AdapterSnapshot::default();
        };
        per_adapter.push((adapter.class, metrics));
    }

    AdapterSnapshot {
        gpu: aggregate_gpus(&per_adapter),
        npu: aggregate_npus(&per_adapter),
    }
}

/// Per-process GPU/NPU stats for the given PIDs. A class is only queried when
/// its adapters exist and its `set_*_process_stats_enabled(true)` gate is on;
/// with both gates off this returns an empty map without opening any handles.
pub fn process_stats(pids: &[u32]) -> HashMap<u32, ProcAdapterStats> {
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
    let elapsed = state.last_proc_sample.map(|t| now.duration_since(t).as_secs_f64());
    state.last_proc_sample = Some(now);

    // Split borrows so adapters and the baseline map can be used together.
    let AdapterState { adapters, prev_proc_running, .. } = state;
    // Baseline slots cover every node of every adapter so the flattened
    // indices stay stable regardless of which classes are queried.
    let total_nodes: usize = adapters.iter().map(|a| a.node_count as usize).sum();

    let mut result = HashMap::with_capacity(pids.len());
    for &pid in pids {
        // Idle and System pseudo-processes can't be opened.
        if pid == 0 || pid == 4 {
            continue;
        }
        let Some(handle) = super::process::open_process_query(pid) else {
            continue;
        };

        // A PID seen for the first time only sets baselines (reports 0%).
        let fresh = !prev_proc_running.contains_key(&pid);
        let prev = prev_proc_running
            .entry(pid)
            .or_insert_with(|| vec![0; total_nodes]);

        let mut entry = ProcAdapterStats::default();
        let mut node_index = 0;
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
                    let running =
                        unsafe { stats.QueryResult.ProcessNodeInformation.RunningTime };
                    if !fresh && let Some(secs) = elapsed {
                        let pct = running_time_to_percent(prev[node_index], running, secs);
                        match adapter.class {
                            AdapterClass::Gpu => {
                                entry.gpu_percent = entry.gpu_percent.max(pct)
                            }
                            AdapterClass::Npu => {
                                entry.npu_percent = entry.npu_percent.max(pct)
                            }
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
                    let committed = unsafe {
                        stats.QueryResult.ProcessSegmentInformation.BytesCommitted
                    };
                    match adapter.class {
                        AdapterClass::Gpu => entry.gpu_memory += committed,
                        AdapterClass::Npu => entry.npu_memory += committed,
                    }
                }
            }
        }
        unsafe {
            let _ = CloseHandle(handle);
        }
        result.insert(pid, entry);
    }

    // Prune baselines for PIDs that died or could no longer be opened.
    prev_proc_running.retain(|pid, _| result.contains_key(pid));

    result
}

#[cfg(test)]
mod tests {
    use super::{
        aggregate_gpus, aggregate_npus, classify_adapter, running_time_to_percent,
        utf16_to_string, AdapterClass, AdapterMetrics,
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
        let buf: Vec<u16> = "Intel(R) AI Boost\0\0extra"
            .encode_utf16()
            .collect();
        assert_eq!(utf16_to_string(&buf), "Intel(R) AI Boost");
        let no_nul: Vec<u16> = "NPU".encode_utf16().collect();
        assert_eq!(utf16_to_string(&no_nul), "NPU");
    }
}
