//! NPU (Neural Processing Unit) monitoring via D3DKMT.
//!
//! NPUs are exposed as MCDM (Microsoft Compute Driver Model) compute-only
//! adapters managed by dxgkrnl, sharing the GPU scheduling and memory
//! infrastructure. Task Manager's NPU columns (Windows 11 KB5094126) read an
//! undocumented PDH counterset, so we use the documented D3DKMT statistics
//! DDI instead (the same route SystemInformer takes): node running-time
//! deltas for utilization, segment commitments for memory.

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

/// System-wide NPU snapshot stored in `SystemMetrics` (cheap to clone).
#[derive(Clone, Default)]
pub struct NpuInfo {
    /// Adapter name from the driver, e.g. "Intel(R) AI Boost"
    pub name: String,
    /// 0-100, max across all engine nodes (Task Manager semantics)
    pub utilization: f32,
    /// Dedicated + shared bytes in use
    pub mem_used: u64,
    /// Sum of segment commit limits (0 if the driver reports none)
    pub mem_total: u64,
    pub dedicated_used: u64,
    pub shared_used: u64,
}

/// Per-process NPU usage row.
#[derive(Clone, Copy, Default)]
pub struct ProcNpu {
    /// 0-100, max across NPU engine nodes
    pub percent: f32,
    /// Dedicated + shared committed bytes
    pub memory: u64,
}

/// One detected MCDM compute-only adapter.
struct NpuAdapter {
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
struct NpuState {
    adapters: Vec<NpuAdapter>,
    detected: bool,
    /// Set when a statistics query fails (driver reset); triggers re-detection
    needs_reenumeration: bool,
    last_sample: Option<Instant>,
    last_proc_sample: Option<Instant>,
    /// pid -> previous RunningTime per node, flattened across adapters
    prev_proc_running: HashMap<u32, Vec<i64>>,
}

static NPU_STATE: Mutex<Option<NpuState>> = Mutex::new(None);

/// Per-process collection gate, set from the UI thread (NPU column visible or
/// sorted), read by the data-collector thread. Per-process stats cost a handle
/// open plus a few syscalls per process per tick, so they're off unless shown.
static PROCESS_STATS_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_process_stats_enabled(enabled: bool) {
    PROCESS_STATS_ENABLED.store(enabled, Ordering::Relaxed);
}

/// MCDM adapters report ComputeOnly (bit 11); exclude software renderers (bit 2).
#[inline]
fn is_compute_only_adapter(adapter_type_value: u32) -> bool {
    const SOFTWARE_DEVICE: u32 = 1 << 2;
    const COMPUTE_ONLY: u32 = 1 << 11;
    adapter_type_value & COMPUTE_ONLY != 0 && adapter_type_value & SOFTWARE_DEVICE == 0
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

/// Enumerate dxgkrnl adapters and keep the MCDM compute-only ones (NPUs).
fn detect_adapters() -> Vec<NpuAdapter> {
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
            if let Some(adapter) = build_npu_adapter(info) {
                adapters.push(adapter);
            }
            // Statistics queries take the LUID, so no handle is retained.
            let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: info.hAdapter });
        }
        adapters
    }
}

/// Classify one enumerated adapter; returns Some only for NPUs.
fn build_npu_adapter(info: &D3DKMT_ADAPTERINFO) -> Option<NpuAdapter> {
    unsafe {
        // Adapter type: skip everything that isn't an MCDM compute-only device.
        let mut adapter_type = D3DKMT_ADAPTERTYPE::default();
        let mut query = D3DKMT_QUERYADAPTERINFO {
            hAdapter: info.hAdapter,
            Type: KMTQAITYPE_ADAPTERTYPE,
            pPrivateDriverData: &mut adapter_type as *mut _ as *mut std::ffi::c_void,
            PrivateDriverDataSize: std::mem::size_of::<D3DKMT_ADAPTERTYPE>() as u32,
        };
        if D3DKMTQueryAdapterInfo(&mut query).is_err()
            || !is_compute_only_adapter(adapter_type.Anonymous.Value)
        {
            return None;
        }

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
            if s.is_empty() { "NPU".to_string() } else { s }
        } else {
            "NPU".to_string()
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

        Some(NpuAdapter {
            luid: info.AdapterLuid,
            name,
            node_count,
            segment_count,
            aperture_segment,
            prev_node_running: vec![0; node_count as usize],
        })
    }
}

/// Refresh system-wide NPU metrics. Returns None when no NPU exists; after the
/// one-time detection that path is just a mutex lock and an empty check.
pub fn refresh() -> Option<NpuInfo> {
    let mut guard = NPU_STATE.lock().unwrap();
    let state = guard.get_or_insert_with(NpuState::default);

    if !state.detected || state.needs_reenumeration {
        state.adapters = detect_adapters();
        state.detected = true;
        state.needs_reenumeration = false;
        state.last_sample = None;
        state.last_proc_sample = None;
        state.prev_proc_running.clear();
    }
    if state.adapters.is_empty() {
        return None;
    }

    let now = Instant::now();
    // First sample after (re-)detection only sets baselines and reports 0%.
    let elapsed = state.last_sample.map(|t| now.duration_since(t).as_secs_f64());
    state.last_sample = Some(now);

    let mut info = NpuInfo {
        name: state.adapters[0].name.clone(),
        ..Default::default()
    };
    if state.adapters.len() > 1 {
        info.name.push_str(&format!(" (+{})", state.adapters.len() - 1));
    }

    let mut failed = false;
    for adapter in &mut state.adapters {
        // Utilization: max across engine nodes of the running-time deltas.
        for node_id in 0..adapter.node_count {
            let mut stats = D3DKMT_QUERYSTATISTICS {
                Type: D3DKMT_QUERYSTATISTICS_NODE,
                AdapterLuid: adapter.luid,
                ..Default::default()
            };
            stats.Anonymous.QueryNode = D3DKMT_QUERYSTATISTICS_QUERY_NODE { NodeId: node_id };
            if !unsafe { query_statistics(&mut stats) } {
                failed = true;
                break;
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
                info.utilization = info.utilization.max(pct);
            }
            adapter.prev_node_running[node_id as usize] = running;
        }
        if failed {
            break;
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
                failed = true;
                break;
            }
            let segment = unsafe { stats.QueryResult.SegmentInformation };
            let used = if segment.BytesResident > 0 {
                segment.BytesResident
            } else {
                segment.BytesCommitted
            };
            if adapter.aperture_segment.get(segment_id as usize).copied().unwrap_or(false) {
                info.shared_used += used;
            } else {
                info.dedicated_used += used;
            }
            info.mem_total += segment.CommitLimit;
        }
        if failed {
            break;
        }
    }

    if failed {
        // Driver reset or adapter removal: re-enumerate on the next tick.
        state.needs_reenumeration = true;
        return None;
    }

    info.mem_used = info.dedicated_used + info.shared_used;
    Some(info)
}

/// Per-process NPU stats for the given PIDs. Returns an empty map unless an
/// NPU exists and `set_process_stats_enabled(true)` was called.
pub fn process_stats(pids: &[u32]) -> HashMap<u32, ProcNpu> {
    let mut guard = NPU_STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else {
        return HashMap::new();
    };
    if state.adapters.is_empty() {
        return HashMap::new();
    }
    if !PROCESS_STATS_ENABLED.load(Ordering::Relaxed) {
        // Drop stale baselines so re-enabling starts from a clean first sample.
        if !state.prev_proc_running.is_empty() {
            state.prev_proc_running.clear();
        }
        state.last_proc_sample = None;
        return HashMap::new();
    }

    let now = Instant::now();
    let elapsed = state.last_proc_sample.map(|t| now.duration_since(t).as_secs_f64());
    state.last_proc_sample = Some(now);

    // Split borrows so adapters and the baseline map can be used together.
    let NpuState { adapters, prev_proc_running, .. } = state;
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

        let mut entry = ProcNpu::default();
        let mut node_index = 0;
        for adapter in adapters.iter() {
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
                        entry.percent = entry.percent.max(pct);
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
                    entry.memory += unsafe {
                        stats.QueryResult.ProcessSegmentInformation.BytesCommitted
                    };
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
    use super::{is_compute_only_adapter, running_time_to_percent, utf16_to_string};

    #[test]
    fn test_compute_only_detection() {
        // ComputeOnly (bit 11) alone -> NPU
        assert!(is_compute_only_adapter(1 << 11));
        // ComputeOnly + SoftwareDevice (bit 2) -> software renderer, not NPU
        assert!(!is_compute_only_adapter((1 << 11) | (1 << 2)));
        // Render-capable GPU (bit 0) without ComputeOnly -> not NPU
        assert!(!is_compute_only_adapter(1 << 0));
        assert!(!is_compute_only_adapter(0));
        // Other flags alongside ComputeOnly don't disqualify
        assert!(is_compute_only_adapter((1 << 11) | (1 << 13)));
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
