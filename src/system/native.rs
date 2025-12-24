//! Native Windows process enumeration using NtQuerySystemInformation
//! This is significantly faster than sysinfo as it gets all process info in a single syscall.

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Win32::Foundation::{HANDLE, UNICODE_STRING};

/// Raw process information from NtQuerySystemInformation
#[derive(Clone, Debug)]
pub struct NativeProcessInfo {
    pub pid: u32,
    pub parent_pid: u32,
    pub name: String,
    pub thread_count: u32,
    pub handle_count: u32,
    pub base_priority: i32,
    pub working_set: u64,
    pub private_bytes: u64,
    pub virtual_size: u64,
    pub kernel_time: u64,   // 100-nanosecond intervals
    pub user_time: u64,     // 100-nanosecond intervals
    pub create_time: u64,   // FILETIME as u64
    pub read_bytes: u64,
    pub write_bytes: u64,
}

// SYSTEM_PROCESS_INFORMATION with actual field layout
// Reference: https://www.geoffchappell.com/studies/windows/km/ntoskrnl/api/ex/sysinfo/process.htm
// Note: #[repr(C)] handles alignment padding automatically
#[repr(C)]
struct SystemProcessInfo {
    next_entry_offset: u32,
    number_of_threads: u32,
    working_set_private_size: i64,
    hard_fault_count: u32,
    number_of_threads_high_watermark: u32,
    cycle_time: u64,
    create_time: i64,
    user_time: i64,
    kernel_time: i64,
    image_name: UNICODE_STRING,
    base_priority: i32,
    unique_process_id: HANDLE,
    inherited_from_unique_process_id: HANDLE,
    handle_count: u32,
    session_id: u32,
    unique_process_key: usize,
    peak_virtual_size: usize,
    virtual_size: usize,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_page_count: usize,
    read_operation_count: i64,
    write_operation_count: i64,
    other_operation_count: i64,
    read_transfer_count: i64,
    write_transfer_count: i64,
    other_transfer_count: i64,
}

// CPU time cache is now in cache.rs module

// Reusable buffer for NtQuerySystemInformation to avoid repeated allocations
thread_local! {
    static QUERY_BUFFER: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(Vec::with_capacity(1024 * 1024));
}

/// Query all processes using NtQuerySystemInformation (single syscall)
pub fn query_all_processes() -> Vec<NativeProcessInfo> {
    QUERY_BUFFER.with(|buf| {
        let mut buffer = buf.borrow_mut();
        // Clear the buffer to ensure fresh data on each call
        // This is critical - without clearing, stale data from previous calls may persist
        buffer.clear();
        let cap = buffer.capacity();
        if cap < 1024 * 1024 {
            buffer.reserve(1024 * 1024 - cap);
        }
        let new_cap = buffer.capacity();
        buffer.resize(new_cap, 0);
        query_all_processes_with_buffer(&mut buffer)
    })
}

fn query_all_processes_with_buffer(buffer: &mut Vec<u8>) -> Vec<NativeProcessInfo> {
    let mut return_length: u32 = 0;

    // Query system process information
    loop {
        let status = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                buffer.as_mut_ptr() as *mut _,
                buffer.len() as u32,
                &mut return_length,
            )
        };

        if status.is_ok() {
            break;
        }

        // STATUS_INFO_LENGTH_MISMATCH - need bigger buffer
        if status.0 as u32 == 0xC0000004 {
            buffer.resize(return_length as usize + 65536, 0);
            continue;
        }

        // Other error
        return Vec::new();
    }

    let mut processes = Vec::with_capacity(500);
    let mut offset: usize = 0;

    loop {
        let proc_info = unsafe { &*(buffer.as_ptr().add(offset) as *const SystemProcessInfo) };

        // Extract process name from UNICODE_STRING
        let name = if proc_info.image_name.Length > 0 && !proc_info.image_name.Buffer.is_null() {
            let slice = unsafe {
                std::slice::from_raw_parts(
                    proc_info.image_name.Buffer.0,
                    (proc_info.image_name.Length / 2) as usize,
                )
            };
            OsString::from_wide(slice).to_string_lossy().into_owned()
        } else if proc_info.unique_process_id.0 as usize == 0 {
            "System Idle Process".to_string()
        } else {
            "System".to_string()
        };

        let pid = proc_info.unique_process_id.0 as usize as u32;
        let parent_pid = proc_info.inherited_from_unique_process_id.0 as usize as u32;

        let kernel_time = proc_info.kernel_time as u64;
        let user_time = proc_info.user_time as u64;

        processes.push(NativeProcessInfo {
            pid,
            parent_pid,
            name,
            thread_count: proc_info.number_of_threads,
            handle_count: proc_info.handle_count,
            base_priority: proc_info.base_priority,
            working_set: proc_info.working_set_size as u64,
            private_bytes: proc_info.private_page_count as u64,
            // Use pagefile_usage (committed memory) for VIRT, not virtual_size (total address space)
            // virtual_size can be 2TB+ on 64-bit which confuses users
            // pagefile_usage matches Task Manager's "Commit size"
            virtual_size: proc_info.pagefile_usage as u64,
            kernel_time,
            user_time,
            create_time: proc_info.create_time as u64,
            read_bytes: proc_info.read_transfer_count as u64,
            write_bytes: proc_info.write_transfer_count as u64,
        });

        if proc_info.next_entry_offset == 0 {
            break;
        }
        offset += proc_info.next_entry_offset as usize;
    }

    processes
}

/// Calculate CPU percentage using delta from previous measurement
pub fn calculate_cpu_percentages(
    processes: &mut [NativeProcessInfo],
    total_cpu_delta: u64,
) -> HashMap<u32, f32> {
    use super::cache::CACHE;

    let now = std::time::Instant::now();
    let mut cpu_percentages = HashMap::with_capacity(processes.len());

    // Get snapshot of CPU times from unified cache
    let cache_snapshot = CACHE.snapshot();

    for proc in processes.iter() {
        let total_time = proc.kernel_time + proc.user_time;

        let cpu_percent = if let Some(entry) = cache_snapshot.get(&proc.pid) {
            let prev_total = entry.kernel_time + entry.user_time;
            let time_delta = total_time.saturating_sub(prev_total);
            let elapsed = now.duration_since(entry.cpu_time_updated).as_nanos() as u64;

            if elapsed > 0 && total_cpu_delta > 0 {
                // CPU percentage relative to total system CPU time
                (time_delta as f64 / total_cpu_delta as f64 * 100.0) as f32
            } else {
                0.0
            }
        } else {
            0.0
        };

        cpu_percentages.insert(proc.pid, cpu_percent);
    }

    // Batch update cache with current times (single lock acquisition)
    // Include create_time to detect PID reuse
    let updates: Vec<(u32, u64, u64, u64)> = processes
        .iter()
        .map(|p| (p.pid, p.kernel_time, p.user_time, p.create_time))
        .collect();
    CACHE.update_cpu_times_batch(&updates);

    cpu_percentages
}


/// Convert FILETIME (100-ns intervals since 1601) to Unix timestamp
#[inline]
pub fn filetime_to_unix(filetime: u64) -> u64 {
    // FILETIME epoch: January 1, 1601
    // Unix epoch: January 1, 1970
    // Difference: 116444736000000000 100-nanosecond intervals
    filetime.saturating_sub(116444736000000000) / 10_000_000
}

