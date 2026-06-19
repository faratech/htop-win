//! Unified process cache module
//!
//! Consolidates all per-process caching into a single module with:
//! - Single lock for per-PID data (reduced contention)
//! - Unified cleanup mechanism
//! - Consistent TTL handling
//! - Centralized configuration

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Instant;

use super::process::ProcessArch;

/// Cache configuration constants
pub mod config {
    /// Clean caches every N refreshes
    pub const CLEANUP_INTERVAL: u32 = 10;
    /// Efficiency mode TTL in milliseconds
    pub const EFFICIENCY_TTL_MS: u128 = 30_000;
    /// Exe status check interval in seconds
    pub const EXE_STATUS_TTL_SECS: u64 = 10;
    /// Maximum exe status cache entries before clear
    pub const EXE_CACHE_MAX_SIZE: usize = 1000;
}

/// Per-PID cache entry containing all cached process data
#[derive(Clone)]
pub struct ProcessCacheEntry {
    // Process identity - used to detect PID reuse
    pub create_time: u64,

    // CPU time tracking (for CPU% delta calculation)
    pub kernel_time: u64,
    pub user_time: u64,
    pub cpu_time_updated: Instant,

    // I/O tracking (for rate delta calculation)
    pub prev_io_read: u64,
    pub prev_io_write: u64,
    pub io_updated: Instant,

    // User info (never changes for a PID). Arc<str> so common accounts
    // (SYSTEM, LOCAL SERVICE, ...) are shared across processes rather than
    // re-allocated per process per refresh.
    pub user: Option<Arc<str>>,

    // Static info (never changes for a PID)
    pub is_elevated: Option<bool>,
    pub arch: Option<ProcessArch>,
    pub exe_path: Option<String>,

    // Efficiency mode (TTL-based refresh)
    pub efficiency_mode: Option<bool>,
    pub efficiency_updated: Option<Instant>,
}

impl Default for ProcessCacheEntry {
    fn default() -> Self {
        Self {
            create_time: 0,
            kernel_time: 0,
            user_time: 0,
            cpu_time_updated: Instant::now(),
            prev_io_read: 0,
            prev_io_write: 0,
            io_updated: Instant::now(),
            user: None,
            is_elevated: None,
            arch: None,
            exe_path: None,
            efficiency_mode: None,
            efficiency_updated: None,
        }
    }
}

/// Exe status cache entry (keyed by path+process create FILETIME, not PID)
#[derive(Clone)]
pub struct ExeStatusEntry {
    pub updated: bool,
    pub deleted: bool,
    pub checked_at: u64,
}

/// Global process cache singleton
pub static CACHE: LazyLock<ProcessCache> = LazyLock::new(ProcessCache::new);

/// Unified process cache
pub struct ProcessCache {
    /// Per-PID cache entries
    entries: RwLock<HashMap<u32, ProcessCacheEntry>>,
    /// Exe status cache (keyed by path+process create FILETIME)
    exe_status: RwLock<HashMap<(String, u64), ExeStatusEntry>>,
    /// Cleanup counter for periodic maintenance
    cleanup_counter: AtomicU32,
}

impl ProcessCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            exe_status: RwLock::new(HashMap::new()),
            cleanup_counter: AtomicU32::new(0),
        }
    }

    /// Batch update CPU times and I/O bytes for multiple PIDs (single lock acquisition)
    /// Tuple: (pid, kernel_time, user_time, create_time, io_read, io_write)
    /// Returns a map of PID → (io_read_rate, io_write_rate) computed from cache deltas
    pub fn update_times_batch(
        &self,
        updates: &[(u32, u64, u64, u64, u64, u64)],
    ) -> HashMap<u32, (u64, u64)> {
        let mut io_rates = HashMap::with_capacity(updates.len());
        if let Ok(mut cache) = self.entries.write() {
            let now = Instant::now();
            for &(pid, kernel_time, user_time, create_time, io_read, io_write) in updates {
                let entry = cache.entry(pid).or_default();
                // Detect PID reuse: if create_time changed, invalidate static fields
                if entry.create_time != 0 && entry.create_time != create_time {
                    entry.user = None;
                    entry.is_elevated = None;
                    entry.arch = None;
                    entry.exe_path = None;
                    entry.efficiency_mode = None;
                    entry.efficiency_updated = None;
                    entry.prev_io_read = 0;
                    entry.prev_io_write = 0;
                }
                // First appearance (new PID or PID reuse): rate = 0, not a delta
                let is_first = entry.create_time == 0 || entry.create_time != create_time;
                let elapsed = now.duration_since(entry.io_updated).as_secs_f64();
                let read_rate = if is_first || elapsed <= 0.0 {
                    0
                } else {
                    (io_read.saturating_sub(entry.prev_io_read) as f64 / elapsed) as u64
                };
                let write_rate = if is_first || elapsed <= 0.0 {
                    0
                } else {
                    (io_write.saturating_sub(entry.prev_io_write) as f64 / elapsed) as u64
                };
                io_rates.insert(pid, (read_rate, write_rate));

                entry.create_time = create_time;
                entry.kernel_time = kernel_time;
                entry.user_time = user_time;
                entry.cpu_time_updated = now;
                entry.prev_io_read = io_read;
                entry.prev_io_write = io_write;
                entry.io_updated = now;
            }
        }
        io_rates
    }

    /// Cache username for a PID
    pub fn set_user(&self, pid: u32, user: Arc<str>) {
        if let Ok(mut cache) = self.entries.write() {
            let entry = cache.entry(pid).or_default();
            entry.user = Some(user);
        }
    }

    /// Cache efficiency mode for a PID
    pub fn set_efficiency_mode(&self, pid: u32, mode: bool) {
        if let Ok(mut cache) = self.entries.write() {
            let entry = cache.entry(pid).or_default();
            entry.efficiency_mode = Some(mode);
            entry.efficiency_updated = Some(Instant::now());
        }
    }

    // ========== Exe Status Methods ==========

    /// Check exe status with caching
    /// Returns (exe_updated, exe_deleted)
    pub fn check_exe_status(&self, exe_path: &str, start_time_100ns: u64) -> (bool, bool) {
        use std::fs;
        use std::time::UNIX_EPOCH;
        const UNIX_EPOCH_FILETIME_100NS: u64 = 116444736000000000;

        if exe_path.is_empty() {
            return (false, false);
        }

        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let cache_key = (exe_path.to_string(), start_time_100ns);

        // Check cache first
        if let Ok(cache) = self.exe_status.read()
            && let Some(entry) = cache.get(&cache_key)
            && now.saturating_sub(entry.checked_at) < config::EXE_STATUS_TTL_SECS
        {
            return (entry.updated, entry.deleted);
        }

        // Cache miss or stale - do filesystem check
        let result = match fs::metadata(exe_path) {
            Ok(metadata) => {
                let exe_updated = metadata
                    .modified()
                    .ok()
                    .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
                    .map(|mtime_unix| {
                        let mtime_100ns = UNIX_EPOCH_FILETIME_100NS
                            .saturating_add(mtime_unix.as_secs().saturating_mul(10_000_000))
                            .saturating_add((mtime_unix.subsec_nanos() / 100) as u64);
                        mtime_100ns > start_time_100ns
                    })
                    .unwrap_or(false);
                (exe_updated, false)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, true),
            Err(_) => (false, false),
        };

        // Update cache (with size limit)
        if let Ok(mut cache) = self.exe_status.write() {
            if cache.len() > config::EXE_CACHE_MAX_SIZE {
                cache.clear();
            }
            cache.insert(
                cache_key,
                ExeStatusEntry {
                    updated: result.0,
                    deleted: result.1,
                    checked_at: now,
                },
            );
        }

        result
    }

    // ========== Read Access ==========

    /// Execute a closure with read access to the cache, avoiding a full clone.
    /// The lock is held for the duration of the callback.
    pub fn with_read<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&HashMap<u32, ProcessCacheEntry>) -> R,
    {
        static EMPTY: std::sync::LazyLock<HashMap<u32, ProcessCacheEntry>> =
            std::sync::LazyLock::new(HashMap::new);
        let guard = self.entries.read();
        match guard {
            Ok(cache) => f(&cache),
            Err(_) => f(&EMPTY),
        }
    }

    // ========== Cleanup Methods ==========

    /// Check if cleanup should run (every CLEANUP_INTERVAL refreshes)
    pub fn should_cleanup(&self) -> bool {
        self.cleanup_counter
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(config::CLEANUP_INTERVAL)
    }

    /// Remove entries for PIDs that no longer exist
    pub fn cleanup(&self, current_pids: &HashSet<u32>) {
        // Clean per-PID entries
        if let Ok(mut cache) = self.entries.write() {
            cache.retain(|pid, _| current_pids.contains(pid));
        }

        // Exe status cache uses size-based cleanup (in check_exe_status).
        // No PID-based cleanup needed since keys are (path, create FILETIME).
    }

    // ========== Batch Update Methods ==========

    /// Batch update multiple entries (single lock acquisition)
    pub fn update_batch<F>(&self, pids: &[u32], mut updater: F)
    where
        F: FnMut(u32, &mut ProcessCacheEntry),
    {
        if let Ok(mut cache) = self.entries.write() {
            for &pid in pids {
                let entry = cache.entry(pid).or_default();
                updater(pid, entry);
            }
        }
    }
}

impl Default for ProcessCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_update_and_io_rates() {
        let cache = ProcessCache::new();

        // First tick: new process, rate should be 0
        let rates = cache.update_times_batch(&[(100, 500, 600, 9999, 1000, 2000)]);
        assert_eq!(rates[&100], (0, 0)); // First appearance → zero rate

        // Second tick: delta-based rate
        std::thread::sleep(std::time::Duration::from_millis(10));
        let rates = cache.update_times_batch(&[(100, 700, 800, 9999, 1500, 2800)]);
        assert!(rates[&100].0 > 0); // bytes/sec from 1500-1000 over elapsed time
        assert!(rates[&100].1 > 0); // bytes/sec from 2800-2000 over elapsed time

        // PID reuse (different create_time): rate resets to 0
        let rates = cache.update_times_batch(&[(100, 10, 20, 5555, 300, 400)]);
        assert_eq!(rates[&100], (0, 0));
    }

    #[test]
    fn test_user_cache() {
        let cache = ProcessCache::new();
        cache.set_user(123, Arc::from("testuser"));
        let user = cache.with_read(|c| c[&123].user.clone());
        assert_eq!(user, Some(Arc::from("testuser")));
    }

    #[test]
    fn test_cleanup() {
        let cache = ProcessCache::new();
        cache.update_times_batch(&[
            (1, 100, 200, 1, 0, 0),
            (2, 100, 200, 2, 0, 0),
            (3, 100, 200, 3, 0, 0),
        ]);

        let current_pids: HashSet<u32> = [1, 3].into_iter().collect();
        cache.cleanup(&current_pids);

        cache.with_read(|c| {
            assert!(c.contains_key(&1));
            assert!(!c.contains_key(&2)); // Cleaned up
            assert!(c.contains_key(&3));
        });
    }

    #[test]
    fn test_with_read() {
        let cache = ProcessCache::new();
        cache.update_times_batch(&[(1, 100, 200, 1, 0, 0), (2, 300, 400, 2, 0, 0)]);
        cache.set_user(1, Arc::from("user1"));

        cache.with_read(|c| {
            assert_eq!(c.len(), 2);
            assert!(c.contains_key(&1));
            assert!(c.contains_key(&2));
        });
    }
}
