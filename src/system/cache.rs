//! Unified process cache module
//!
//! Consolidates all per-process caching into a single module with:
//! - Single lock for per-PID data (reduced contention)
//! - Unified cleanup mechanism
//! - Consistent TTL handling
//! - Centralized configuration

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, RwLock};
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

    // User info (never changes for a PID)
    pub user: Option<String>,

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
            user: None,
            is_elevated: None,
            arch: None,
            exe_path: None,
            efficiency_mode: None,
            efficiency_updated: None,
        }
    }
}

/// Exe status cache entry (keyed by path+start_time, not PID)
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
    /// Exe status cache (keyed by path+start_time)
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

    // ========== CPU Time Methods ==========

    /// Get CPU times for a PID (for delta calculation)
    /// Returns (kernel_time, user_time, last_update_instant)
    #[allow(dead_code)]
    pub fn get_cpu_times(&self, pid: u32) -> Option<(u64, u64, Instant)> {
        self.entries
            .read()
            .ok()
            .and_then(|cache| {
                cache.get(&pid).map(|e| (e.kernel_time, e.user_time, e.cpu_time_updated))
            })
    }

    /// Update CPU times for a PID
    #[allow(dead_code)]
    pub fn update_cpu_times(&self, pid: u32, kernel_time: u64, user_time: u64) {
        if let Ok(mut cache) = self.entries.write() {
            let entry = cache.entry(pid).or_default();
            entry.kernel_time = kernel_time;
            entry.user_time = user_time;
            entry.cpu_time_updated = Instant::now();
        }
    }

    /// Batch update CPU times for multiple PIDs (single lock acquisition)
    /// Tuple: (pid, kernel_time, user_time, create_time)
    pub fn update_cpu_times_batch(&self, updates: &[(u32, u64, u64, u64)]) {
        if let Ok(mut cache) = self.entries.write() {
            let now = Instant::now();
            for &(pid, kernel_time, user_time, create_time) in updates {
                let entry = cache.entry(pid).or_default();
                // Detect PID reuse: if create_time changed, invalidate static fields
                if entry.create_time != 0 && entry.create_time != create_time {
                    entry.user = None;
                    entry.is_elevated = None;
                    entry.arch = None;
                    entry.exe_path = None;
                    entry.efficiency_mode = None;
                    entry.efficiency_updated = None;
                }
                entry.create_time = create_time;
                entry.kernel_time = kernel_time;
                entry.user_time = user_time;
                entry.cpu_time_updated = now;
            }
        }
    }

    // ========== User Cache Methods ==========

    /// Get cached username for a PID
    #[allow(dead_code)]
    pub fn get_user(&self, pid: u32) -> Option<String> {
        self.entries
            .read()
            .ok()
            .and_then(|cache| cache.get(&pid).and_then(|e| e.user.clone()))
    }

    /// Cache username for a PID
    pub fn set_user(&self, pid: u32, user: String) {
        if let Ok(mut cache) = self.entries.write() {
            let entry = cache.entry(pid).or_default();
            entry.user = Some(user);
        }
    }

    // ========== Static Info Methods ==========

    /// Get cached static info (is_elevated, arch, exe_path)
    #[allow(dead_code)]
    pub fn get_static_info(&self, pid: u32) -> Option<(bool, ProcessArch, String)> {
        self.entries
            .read()
            .ok()
            .and_then(|cache| {
                cache.get(&pid).and_then(|e| {
                    match (&e.is_elevated, &e.arch, &e.exe_path) {
                        (Some(elev), Some(arch), Some(path)) => Some((*elev, *arch, path.clone())),
                        _ => None,
                    }
                })
            })
    }

    /// Cache static info for a PID
    #[allow(dead_code)]
    pub fn set_static_info(&self, pid: u32, is_elevated: bool, arch: ProcessArch, exe_path: String) {
        if let Ok(mut cache) = self.entries.write() {
            let entry = cache.entry(pid).or_default();
            entry.is_elevated = Some(is_elevated);
            entry.arch = Some(arch);
            entry.exe_path = Some(exe_path);
        }
    }

    // ========== Efficiency Mode Methods ==========

    /// Get cached efficiency mode if still valid (within TTL)
    #[allow(dead_code)]
    pub fn get_efficiency_mode(&self, pid: u32) -> Option<bool> {
        let now = Instant::now();
        self.entries
            .read()
            .ok()
            .and_then(|cache| {
                cache.get(&pid).and_then(|e| {
                    if let (Some(mode), Some(updated)) = (e.efficiency_mode, e.efficiency_updated)
                        && now.duration_since(updated).as_millis() < config::EFFICIENCY_TTL_MS {
                            return Some(mode);
                        }
                    None
                })
            })
    }

    /// Check if efficiency mode cache is stale
    #[allow(dead_code)]
    pub fn is_efficiency_stale(&self, pid: u32) -> bool {
        let now = Instant::now();
        self.entries
            .read()
            .ok()
            .map(|cache| {
                cache.get(&pid).is_none_or(|e| {
                    e.efficiency_updated.is_none_or(|updated| {
                        now.duration_since(updated).as_millis() >= config::EFFICIENCY_TTL_MS
                    })
                })
            })
            .unwrap_or(true)
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
    pub fn check_exe_status(&self, exe_path: &str, start_time: u64) -> (bool, bool) {
        use std::fs;
        use std::time::UNIX_EPOCH;

        if exe_path.is_empty() {
            return (false, false);
        }

        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let cache_key = (exe_path.to_string(), start_time);

        // Check cache first
        if let Ok(cache) = self.exe_status.read()
            && let Some(entry) = cache.get(&cache_key)
                && now.saturating_sub(entry.checked_at) < config::EXE_STATUS_TTL_SECS {
                    return (entry.updated, entry.deleted);
                }

        // Cache miss or stale - do filesystem check
        let result = match fs::metadata(exe_path) {
            Ok(metadata) => {
                let exe_updated = metadata
                    .modified()
                    .ok()
                    .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
                    .map(|mtime_unix| mtime_unix.as_secs() > start_time)
                    .unwrap_or(false);
                (exe_updated, false)
            }
            Err(_) => (false, true),
        };

        // Update cache (with size limit)
        if let Ok(mut cache) = self.exe_status.write() {
            if cache.len() > config::EXE_CACHE_MAX_SIZE {
                cache.clear();
            }
            cache.insert(cache_key, ExeStatusEntry {
                updated: result.0,
                deleted: result.1,
                checked_at: now,
            });
        }

        result
    }

    // ========== Snapshot Methods ==========

    /// Get a snapshot of all cached data (single lock acquisition)
    /// Returns cloned data to minimize lock hold time
    pub fn snapshot(&self) -> HashMap<u32, ProcessCacheEntry> {
        self.entries
            .read()
            .map(|cache| cache.clone())
            .unwrap_or_default()
    }

    /// Get snapshot of specific fields for specific PIDs
    #[allow(dead_code)]
    pub fn snapshot_for_pids(&self, pids: &[u32]) -> HashMap<u32, ProcessCacheEntry> {
        self.entries
            .read()
            .map(|cache| {
                pids.iter()
                    .filter_map(|pid| cache.get(pid).map(|e| (*pid, e.clone())))
                    .collect()
            })
            .unwrap_or_default()
    }

    // ========== Cleanup Methods ==========

    /// Check if cleanup should run (every CLEANUP_INTERVAL refreshes)
    pub fn should_cleanup(&self) -> bool {
        self.cleanup_counter.fetch_add(1, Ordering::Relaxed) % config::CLEANUP_INTERVAL == 0
    }

    /// Remove entries for PIDs that no longer exist
    pub fn cleanup(&self, current_pids: &HashSet<u32>) {
        // Clean per-PID entries
        if let Ok(mut cache) = self.entries.write() {
            cache.retain(|pid, _| current_pids.contains(pid));
        }

        // Exe status cache uses size-based cleanup (in check_exe_status)
        // No PID-based cleanup needed since keys are (path, start_time)
    }

    /// Force clear all caches (for testing or reset)
    #[allow(dead_code)]
    pub fn clear(&self) {
        if let Ok(mut cache) = self.entries.write() {
            cache.clear();
        }
        if let Ok(mut cache) = self.exe_status.write() {
            cache.clear();
        }
        self.cleanup_counter.store(0, Ordering::Relaxed);
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
    fn test_cpu_times() {
        let cache = ProcessCache::new();
        assert!(cache.get_cpu_times(123).is_none());

        cache.update_cpu_times(123, 1000, 2000);
        let (k, u, _) = cache.get_cpu_times(123).unwrap();
        assert_eq!(k, 1000);
        assert_eq!(u, 2000);
    }

    #[test]
    fn test_user_cache() {
        let cache = ProcessCache::new();
        assert!(cache.get_user(123).is_none());

        cache.set_user(123, "testuser".to_string());
        assert_eq!(cache.get_user(123), Some("testuser".to_string()));
    }

    #[test]
    fn test_cleanup() {
        let cache = ProcessCache::new();
        cache.update_cpu_times(1, 100, 200);
        cache.update_cpu_times(2, 100, 200);
        cache.update_cpu_times(3, 100, 200);

        let current_pids: HashSet<u32> = [1, 3].into_iter().collect();
        cache.cleanup(&current_pids);

        assert!(cache.get_cpu_times(1).is_some());
        assert!(cache.get_cpu_times(2).is_none()); // Cleaned up
        assert!(cache.get_cpu_times(3).is_some());
    }

    #[test]
    fn test_snapshot() {
        let cache = ProcessCache::new();
        cache.update_cpu_times(1, 100, 200);
        cache.set_user(1, "user1".to_string());
        cache.update_cpu_times(2, 300, 400);

        let snapshot = cache.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.contains_key(&1));
        assert!(snapshot.contains_key(&2));
    }
}
