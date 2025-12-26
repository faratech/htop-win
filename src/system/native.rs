//! Native Windows process enumeration using NtQuerySystemInformation
//! This is significantly faster than sysinfo as it gets all process info in a single syscall.

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Win32::Foundation::{HANDLE, UNICODE_STRING};

// Reusable buffer for NtQuerySystemInformation to avoid repeated allocations
thread_local! {
    static QUERY_BUFFER: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(Vec::with_capacity(1024 * 1024));
}

/// Wrapper around raw SystemProcessInfo to provide safe accessors
pub struct SystemProcess<'a> {
    info: &'a SystemProcessInfo,
}

impl<'a> SystemProcess<'a> {
    pub fn pid(&self) -> u32 {
        self.info.unique_process_id.0 as usize as u32
    }

    pub fn parent_pid(&self) -> u32 {
        self.info.inherited_from_unique_process_id.0 as usize as u32
    }

    pub fn thread_count(&self) -> u32 {
        self.info.number_of_threads
    }

    pub fn handle_count(&self) -> u32 {
        self.info.handle_count
    }

    pub fn base_priority(&self) -> i32 {
        self.info.base_priority
    }

    pub fn working_set(&self) -> u64 {
        self.info.working_set_size as u64
    }

    pub fn private_bytes(&self) -> u64 {
        self.info.private_page_count as u64
    }

    pub fn virtual_size(&self) -> u64 {
        // Use pagefile_usage (committed memory) for VIRT
        self.info.pagefile_usage as u64
    }

    pub fn kernel_time(&self) -> u64 {
        self.info.kernel_time as u64
    }

    pub fn user_time(&self) -> u64 {
        self.info.user_time as u64
    }

    pub fn create_time(&self) -> u64 {
        self.info.create_time as u64
    }

    pub fn read_bytes(&self) -> u64 {
        self.info.read_transfer_count as u64
    }

    pub fn write_bytes(&self) -> u64 {
        self.info.write_transfer_count as u64
    }

    /// Extract name - allocates a new String
    pub fn name(&self) -> String {
        if self.info.image_name.Length > 0 && !self.info.image_name.Buffer.is_null() {
            let slice = unsafe {
                std::slice::from_raw_parts(
                    self.info.image_name.Buffer.0,
                    (self.info.image_name.Length / 2) as usize,
                )
            };
            OsString::from_wide(slice).to_string_lossy().into_owned()
        } else if self.info.unique_process_id.0 as usize == 0 {
            "System Idle Process".to_string()
        } else {
            "System".to_string()
        }
    }
}

/// Iterator over system processes
pub struct SystemProcessIterator<'a> {
    buffer: &'a [u8],
    offset: usize,
    finished: bool,
}

impl<'a> Iterator for SystemProcessIterator<'a> {
    type Item = SystemProcess<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        if self.offset >= self.buffer.len() {
            return None;
        }

        let proc_info = unsafe { &*(self.buffer.as_ptr().add(self.offset) as *const SystemProcessInfo) };

        if proc_info.next_entry_offset == 0 {
            self.finished = true;
        } else {
            self.offset += proc_info.next_entry_offset as usize;
        }

        Some(SystemProcess { info: proc_info })
    }
}

/// Helper struct to access the process list
pub struct SystemProcessList<'a> {
    buffer: &'a [u8],
}

impl<'a> SystemProcessList<'a> {
    pub fn iter(&self) -> SystemProcessIterator<'a> {
        SystemProcessIterator {
            buffer: self.buffer,
            offset: 0,
            finished: false,
        }
    }
}

/// Execute a closure with access to the system process list
/// This handles buffer management and syscalls
pub fn with_process_list<F, R>(f: F) -> R
where
    F: FnOnce(SystemProcessList) -> R,
{
    QUERY_BUFFER.with(|buf| {
        let mut buffer = buf.borrow_mut();
        // Clear buffer but keep capacity
        buffer.clear();
        let cap = buffer.capacity();
        if cap < 1024 * 1024 {
            buffer.reserve(1024 * 1024 - cap);
        }
        
        // Ensure we have some zeroed space if resize is needed? 
        // Actually resize will zero initialize.
        // But we want to call with current capacity first? 
        // No, standard pattern is to resize to expected size.
        let new_cap = buffer.capacity();
        buffer.resize(new_cap, 0);

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

            // Other error - return empty result
            // We pass an empty slice in this case
            return f(SystemProcessList { buffer: &[] });
        }

        f(SystemProcessList { buffer: &buffer })
    })
}

// Keep NativeProcessInfo for compatibility if needed, or remove if unused.
// It was used in from_native, so we might need a version of it or update from_native.
// We'll update from_native to use SystemProcess.

// Helper for CPU calculation that works with iterator
pub fn calculate_cpu_percentages_from_iter(
    list: &SystemProcessList,
    total_cpu_delta: u64,
) -> HashMap<u32, f32> {
    use super::cache::CACHE;

    let now = std::time::Instant::now();
    let mut cpu_percentages = HashMap::with_capacity(500); // Estimate

    // Get snapshot of CPU times from unified cache
    let cache_snapshot = CACHE.snapshot();
    let mut updates = Vec::with_capacity(500);

    for proc in list.iter() {
        let pid = proc.pid();
        
        // System Idle Process (PID 0) represents idle CPU time, not actual work
        if pid == 0 {
            cpu_percentages.insert(0, 0.0);
            continue;
        }

        let total_time = proc.kernel_time() + proc.user_time();

        let cpu_percent = if let Some(entry) = cache_snapshot.get(&pid) {
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

        cpu_percentages.insert(pid, cpu_percent);
        
        updates.push((pid, proc.kernel_time(), proc.user_time(), proc.create_time()));
    }

    // Batch update cache
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

