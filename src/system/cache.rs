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
use std::time::{Duration, Instant, UNIX_EPOCH};

use super::process::ProcessArch;

/// Cache configuration constants
pub mod config {
    /// Clean caches every N refreshes
    pub const CLEANUP_INTERVAL: u32 = 10;
    /// Efficiency mode TTL in milliseconds
    pub const EFFICIENCY_TTL_MS: u128 = 30_000;
    /// How long a failed metadata query suppresses retries (negative cache TTL)
    pub const QUERY_FAILURE_TTL_MS: u128 = 15_000;
    /// Exe status check interval in seconds
    pub const EXE_STATUS_TTL_SECS: u64 = 10;
    /// Maximum exe status cache entries before eviction
    pub const EXE_CACHE_MAX_SIZE: usize = 1000;
}

/// FILETIME of the Unix epoch (1601-01-01 → 1970-01-01) in 100ns ticks, used
/// to express mtimes and process create times in the same units.
const UNIX_EPOCH_FILETIME_100NS: u64 = 116444736000000000;

/// How long an exe-status entry stays fresh before its file is re-stat'd.
const EXE_STATUS_TTL: Duration = Duration::from_secs(config::EXE_STATUS_TTL_SECS);

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

    // Negative cache: when a metadata query (OpenProcess or a per-fact query
    // behind it) failed, the fact is "unknown", not authoritative — so it is
    // retried only after QUERY_FAILURE_TTL_MS instead of every refresh.
    pub query_failed_at: Option<Instant>,
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
            query_failed_at: None,
        }
    }
}

/// Exe status cache entry (keyed by path+process create FILETIME, not PID)
#[derive(Clone)]
pub struct ExeStatusEntry {
    pub updated: bool,
    pub deleted: bool,
    /// Monotonic instant of the filesystem stat backing this entry. `Instant`
    /// (not wall-clock) so TTL freshness is immune to clock changes.
    pub checked_at: Instant,
}

/// Exe-status cache layout: `path -> (process create FILETIME -> entry)`.
///
/// Nested instead of a single `HashMap<(String, u64), _>` composite key so the
/// hot hit-path lookup can borrow the caller's `&str` (`Box<str>: Borrow<str>`
/// hashes and compares through `str`) instead of allocating a `String` per call.
type ExeStatusMap = HashMap<Box<str>, HashMap<u64, ExeStatusEntry>>;

/// An entry is fresh while its last stat is younger than the TTL.
fn exe_entry_is_fresh(entry: &ExeStatusEntry, now: Instant) -> bool {
    now.saturating_duration_since(entry.checked_at) < EXE_STATUS_TTL
}

/// Total number of cached entries across every path.
fn exe_status_len(map: &ExeStatusMap) -> usize {
    map.values().map(HashMap::len).sum()
}

/// Drop entries whose TTL has lapsed, pruning paths left without any entry.
/// Returns the number of entries removed.
fn evict_expired_entries(map: &mut ExeStatusMap, now: Instant) -> usize {
    let mut removed = 0usize;
    map.retain(|_, by_start| {
        by_start.retain(|_, entry| {
            let fresh = exe_entry_is_fresh(entry, now);
            if !fresh {
                removed += 1;
            }
            fresh
        });
        !by_start.is_empty()
    });
    removed
}

/// Enforce the size cap once it is exceeded: shed expired entries first and
/// wipe the cache wholesale only when everything left is still fresh, i.e.
/// when eviction cannot free enough room.
fn enforce_exe_cache_limit(map: &mut ExeStatusMap, now: Instant) {
    if exe_status_len(map) <= config::EXE_CACHE_MAX_SIZE {
        return;
    }
    evict_expired_entries(map, now);
    if exe_status_len(map) > config::EXE_CACHE_MAX_SIZE {
        map.clear();
    }
}

/// Global process cache singleton
pub static CACHE: LazyLock<ProcessCache> = LazyLock::new(ProcessCache::new);

/// Unified process cache
pub struct ProcessCache {
    /// Per-PID cache entries
    entries: RwLock<HashMap<u32, ProcessCacheEntry>>,
    /// Exe status cache (keyed by path+process create FILETIME)
    exe_status: RwLock<ExeStatusMap>,
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
        self.update_times_batch_at(updates, Instant::now())
    }

    fn update_times_batch_at(
        &self,
        updates: &[(u32, u64, u64, u64, u64, u64)],
        now: Instant,
    ) -> HashMap<u32, (u64, u64)> {
        let mut io_rates = HashMap::with_capacity(updates.len());
        if let Ok(mut cache) = self.entries.write() {
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
                    entry.query_failed_at = None;
                    entry.prev_io_read = 0;
                    entry.prev_io_write = 0;
                }
                // First appearance (new PID or PID reuse): rate = 0, not a delta
                let is_first = entry.create_time == 0 || entry.create_time != create_time;
                let elapsed = now
                    .saturating_duration_since(entry.io_updated)
                    .as_secs_f64();
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
        self.check_exe_status_at(exe_path, start_time_100ns, Instant::now())
    }

    /// [`ProcessCache::check_exe_status`] evaluated at an explicit instant,
    /// mirroring `update_times_batch_at` so TTL behavior is testable.
    fn check_exe_status_at(
        &self,
        exe_path: &str,
        start_time_100ns: u64,
        now: Instant,
    ) -> (bool, bool) {
        use std::fs;

        if exe_path.is_empty() {
            return (false, false);
        }

        // Hot path: borrowed lookup through `Box<str>: Borrow<str>`, so the hit
        // case allocates nothing and reads no wall clock. Only the monotonic
        // `now` decides freshness.
        if let Ok(cache) = self.exe_status.read()
            && let Some(by_start) = cache.get(exe_path)
            && let Some(entry) = by_start.get(&start_time_100ns)
            && exe_entry_is_fresh(entry, now)
        {
            return (entry.updated, entry.deleted);
        }

        // Cache miss or stale - do filesystem check. The comparison only needs
        // the file's own mtime converted into FILETIME units; no wall-clock
        // read is required here.
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

        // Update cache (size-capped; shed expired entries before clearing)
        if let Ok(mut cache) = self.exe_status.write() {
            enforce_exe_cache_limit(&mut cache, now);
            cache
                .entry(Box::from(exe_path))
                .or_default()
                .insert(
                    start_time_100ns,
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

        // Exe status cache uses TTL eviction and a size cap (in
        // check_exe_status). No PID-based cleanup needed since keys are
        // (path, create FILETIME), not PIDs.
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
    use std::time::SystemTime;

    #[test]
    fn test_batch_update_and_io_rates() {
        let cache = ProcessCache::new();
        let start = Instant::now();

        // First tick: new process, rate should be 0
        let rates = cache.update_times_batch_at(&[(100, 500, 600, 9999, 1000, 2000)], start);
        assert_eq!(rates[&100], (0, 0)); // First appearance → zero rate

        // Second tick: delta-based rate
        let rates = cache.update_times_batch_at(
            &[(100, 700, 800, 9999, 1500, 2800)],
            start + std::time::Duration::from_secs(2),
        );
        assert_eq!(rates[&100], (250, 400));

        // PID reuse (different create_time): rate resets to 0
        let rates = cache.update_times_batch_at(
            &[(100, 10, 20, 5555, 300, 400)],
            start + std::time::Duration::from_secs(3),
        );
        assert_eq!(rates[&100], (0, 0));
    }

    #[test]
    fn process_membership_changes_do_not_reset_survivor_io_rates() {
        let cache = ProcessCache::new();
        let start = Instant::now();
        let first = [
            (100, 10, 20, 1_000, 1_000, 2_000),
            (200, 30, 40, 2_000, 9_000_000, 8_000_000),
        ];
        let rates = cache.update_times_batch_at(&first, start);
        assert_eq!(rates[&100], (0, 0));
        assert_eq!(rates[&200], (0, 0));

        // PID 200 exits. PID 100's delta is still computed from its own identity.
        let rates = cache.update_times_batch_at(
            &[(100, 15, 25, 1_000, 1_300, 2_500)],
            start + std::time::Duration::from_secs(1),
        );
        assert_eq!(rates[&100], (300, 500));
    }

    #[test]
    fn io_counter_reset_rebaselines_the_affected_direction() {
        let cache = ProcessCache::new();
        let start = Instant::now();
        cache.update_times_batch_at(&[(100, 10, 20, 1_000, 1_000, 2_000)], start);

        let rates = cache.update_times_batch_at(
            &[(100, 15, 25, 1_000, 100, 2_200)],
            start + std::time::Duration::from_secs(1),
        );
        assert_eq!(rates[&100], (0, 200));

        let rates = cache.update_times_batch_at(
            &[(100, 20, 30, 1_000, 150, 2_300)],
            start + std::time::Duration::from_secs(2),
        );
        assert_eq!(rates[&100], (50, 100));
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

    // ===== Exe status cache =====

    fn filetime_from_unix_secs(secs: u64) -> u64 {
        UNIX_EPOCH_FILETIME_100NS.saturating_add(secs.saturating_mul(10_000_000))
    }

    fn temp_exe_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("htop-win-exe-cache-{}-{tag}", std::process::id()))
    }

    /// Create/rewrite a temp file with an explicit mtime (std-only, no
    /// external test dependencies).
    fn write_file_with_mtime(path: &std::path::Path, mtime: SystemTime) {
        use std::io::Write as _;
        let mut f = std::fs::File::create(path).expect("create temp file");
        f.write_all(b"htop-win exe-status test").expect("write temp file");
        f.set_times(std::fs::FileTimes::new().set_modified(mtime))
            .expect("set mtime");
    }

    fn remove_file(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
    }

    fn path_str(path: &std::path::Path) -> &str {
        path.to_str().expect("utf-8 temp path")
    }

    #[test]
    fn hit_path_serves_cached_verdict_without_restating() {
        let cache = ProcessCache::new();
        let path = temp_exe_path("hit");
        let t0 = Instant::now();
        // Process started at t=1000s; file written at t=2000s -> "updated".
        let start_time = filetime_from_unix_secs(1_000);

        write_file_with_mtime(&path, UNIX_EPOCH + Duration::from_secs(2_000));
        assert_eq!(
            cache.check_exe_status_at(path_str(&path), start_time, t0),
            (true, false)
        );

        // Rewrite with an mtime that would flip the verdict to "not updated";
        // within the TTL the cached answer must win (no restat).
        write_file_with_mtime(&path, UNIX_EPOCH + Duration::from_secs(500));
        assert_eq!(
            cache.check_exe_status_at(
                path_str(&path),
                start_time,
                t0 + EXE_STATUS_TTL - Duration::from_secs(1)
            ),
            (true, false)
        );

        // One tick past the TTL the entry is stale and gets restated.
        assert_eq!(
            cache.check_exe_status_at(
                path_str(&path),
                start_time,
                t0 + EXE_STATUS_TTL + Duration::from_secs(1)
            ),
            (false, false)
        );

        remove_file(&path);
    }

    #[test]
    fn deletion_is_negative_cached_until_ttl_lapses() {
        let cache = ProcessCache::new();
        let path = temp_exe_path("deleted");
        let t0 = Instant::now();
        let start_time = filetime_from_unix_secs(1_000);

        write_file_with_mtime(&path, UNIX_EPOCH + Duration::from_secs(2_000));
        assert_eq!(
            cache.check_exe_status_at(path_str(&path), start_time, t0),
            (true, false)
        );

        remove_file(&path);
        // Still fresh: cached verdict even though the file is gone.
        assert_eq!(
            cache.check_exe_status_at(path_str(&path), start_time, t0 + Duration::from_secs(5)),
            (true, false)
        );
        // Stale: restat reports the deletion...
        assert_eq!(
            cache.check_exe_status_at(path_str(&path), start_time, t0 + EXE_STATUS_TTL * 2),
            (false, true)
        );
        // ...and that verdict is cached for its own TTL without the file.
        assert_eq!(
            cache.check_exe_status_at(path_str(&path), start_time, t0 + EXE_STATUS_TTL * 3),
            (false, true)
        );

        remove_file(&path);
    }

    /// Insert synthetic entries (no filesystem involved), one path each.
    fn add_synthetic_entries(
        map: &mut ExeStatusMap,
        range: std::ops::Range<usize>,
        checked_at: Instant,
    ) {
        for i in range {
            let key: Box<str> = format!("C:\\apps\\app{i}.exe").into_boxed_str();
            map.entry(key).or_default().insert(
                i as u64,
                ExeStatusEntry {
                    updated: false,
                    deleted: false,
                    checked_at,
                },
            );
        }
    }

    fn contains_path(map: &ExeStatusMap, name: &str) -> bool {
        map.contains_key(name)
    }

    #[test]
    fn cap_eviction_sheds_expired_entries_and_keeps_fresh_ones() {
        let t0 = Instant::now();
        // `later` is one tick past the TTL, so entries stamped at t0 expire
        // while entries stamped at `later` are still fresh.
        let later = t0 + EXE_STATUS_TTL + Duration::from_secs(1);

        let mut map = ExeStatusMap::new();
        add_synthetic_entries(&mut map, 0..600, t0); // will be expired
        add_synthetic_entries(&mut map, 1_000..1_600, later); // still fresh
        // One path holding two start-times of mixed freshness.
        map.insert(
            Box::from("C:\\apps\\shared.exe"),
            HashMap::from([
                (
                    7u64,
                    ExeStatusEntry {
                        updated: true,
                        deleted: false,
                        checked_at: t0,
                    },
                ),
                (
                    8u64,
                    ExeStatusEntry {
                        updated: false,
                        deleted: false,
                        checked_at: later,
                    },
                ),
            ]),
        );
        assert!(exe_status_len(&map) > config::EXE_CACHE_MAX_SIZE);

        enforce_exe_cache_limit(&mut map, later);

        // Expired entries shed, every fresh entry intact: no wholesale clear.
        assert!(exe_status_len(&map) <= config::EXE_CACHE_MAX_SIZE);
        assert_eq!(map.len(), 601);
        for i in 1_000..1_600 {
            assert!(
                contains_path(&map, &format!("C:\\apps\\app{i}.exe")),
                "fresh entry {i} was discarded"
            );
        }
        for i in 0..600 {
            assert!(
                !contains_path(&map, &format!("C:\\apps\\app{i}.exe")),
                "expired entry {i} survived"
            );
        }
        // Mixed-freshness path survives with only its fresh start-time.
        let shared = &map["C:\\apps\\shared.exe"];
        assert!(!shared.contains_key(&7));
        assert!(shared.contains_key(&8));
    }

    #[test]
    fn wholesale_clear_only_when_nothing_is_expired() {
        let t0 = Instant::now();
        let mut map = ExeStatusMap::new();
        // All entries fresh and over the cap: eviction cannot free anything,
        // so clearing the whole map is the only way back under the limit.
        add_synthetic_entries(&mut map, 0..config::EXE_CACHE_MAX_SIZE + 200, t0);
        assert!(exe_status_len(&map) > config::EXE_CACHE_MAX_SIZE);

        enforce_exe_cache_limit(&mut map, t0 + Duration::from_secs(1));

        assert!(map.is_empty());
    }

    #[test]
    fn caches_under_the_cap_are_left_alone() {
        let t0 = Instant::now();
        let mut map = ExeStatusMap::new();
        add_synthetic_entries(&mut map, 0..64, t0);

        enforce_exe_cache_limit(&mut map, t0 + Duration::from_secs(1));

        assert_eq!(exe_status_len(&map), 64);
        assert_eq!(map.len(), 64);
    }

    #[test]
    fn expired_only_paths_are_pruned_from_the_outer_map() {
        let t0 = Instant::now();
        let later = t0 + EXE_STATUS_TTL + Duration::from_secs(1);
        let mut map = ExeStatusMap::new();
        add_synthetic_entries(&mut map, 0..4, t0);
        add_synthetic_entries(&mut map, 100..102, later);

        assert_eq!(evict_expired_entries(&mut map, later), 4);

        assert_eq!(map.len(), 2);
        assert!(contains_path(&map, "C:\\apps\\app100.exe"));
        assert!(contains_path(&map, "C:\\apps\\app101.exe"));
    }
}
