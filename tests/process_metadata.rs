//! Isolated Windows process-metadata regression fixture. No process actions run.
use htop_win::system::cache::CACHE;
use htop_win::system::{ProcessEnrichmentRequirements, SystemMetrics, enrich_processes_for};
use std::time::{Duration, Instant};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualAlloc(
        address: *mut core::ffi::c_void,
        size: usize,
        allocation: u32,
        protect: u32,
    ) -> *mut core::ffi::c_void;
    fn VirtualFree(address: *mut core::ffi::c_void, size: usize, free_type: u32) -> i32;
}
struct Reservation(*mut core::ffi::c_void);
impl Drop for Reservation {
    fn drop(&mut self) {
        unsafe {
            VirtualFree(self.0, 0, 0x8000);
        }
    }
}

#[test]
fn metadata_backoff_and_virtual_reservations_use_authoritative_process_data() {
    let pid = std::process::id();
    let mut metrics = SystemMetrics::default();
    let mut processes = Vec::new();
    metrics.update_processes_native(&mut processes);
    let mut process = processes.iter().find(|p| p.pid == pid).unwrap().clone();
    let failed_at = Instant::now();
    CACHE.update_batch(&[pid], |_, e| {
        e.arch = None;
        e.query_failed_at = Some(failed_at);
    });
    let requirements = ProcessEnrichmentRequirements {
        arch: true,
        ..Default::default()
    };
    for _ in 0..3 {
        enrich_processes_for(std::slice::from_mut(&mut process), requirements);
        CACHE.with_read(|cache| {
            assert_eq!(cache[&pid].query_failed_at, Some(failed_at));
            assert!(cache[&pid].arch.is_none());
        });
    }
    CACHE.update_batch(&[pid], |_, e| {
        e.query_failed_at = Some(failed_at - Duration::from_secs(16))
    });
    enrich_processes_for(std::slice::from_mut(&mut process), requirements);
    CACHE.with_read(|cache| {
        assert!(cache[&pid].query_failed_at.is_none());
        assert!(cache[&pid].arch.is_some());
    });
    // An impossible creation time forces a verified-handle failure without
    // changing any process. The next suppressed pass must retain that failure.
    process.create_time_100ns += 1;
    CACHE.update_times_batch(&[(pid, 0, 0, process.create_time_100ns, 0, 0)]);
    CACHE.with_read(|cache| assert!(cache[&pid].query_failed_at.is_none()));
    enrich_processes_for(std::slice::from_mut(&mut process), requirements);
    let failure = CACHE.with_read(|cache| cache[&pid].query_failed_at);
    assert!(failure.is_some());
    enrich_processes_for(std::slice::from_mut(&mut process), requirements);
    CACHE.with_read(|cache| assert_eq!(cache[&pid].query_failed_at, failure));
    metrics.update_processes_native(&mut processes);
    let before = processes.iter().find(|p| p.pid == pid).unwrap().virtual_mem;
    let size = 1usize << 30;
    // Reserve address space only: MEM_RESERVE, PAGE_NOACCESS (no physical commit).
    let reservation =
        Reservation(unsafe { VirtualAlloc(std::ptr::null_mut(), size, 0x2000, 0x01) });
    assert!(!reservation.0.is_null());
    metrics.update_processes_native(&mut processes);
    let after = processes.iter().find(|p| p.pid == pid).unwrap().virtual_mem;
    // Allow small unrelated collector/runtime allocations to be freed meanwhile.
    assert!(after.saturating_sub(before) >= size as u64 - (16 << 20));
}
