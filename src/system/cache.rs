//! Unified process cache module
//!
//! Consolidates all per-process caching into a single module with:
//! - Single lock for per-PID data (reduced contention)
//! - Unified cleanup mechanism
//! - Consistent TTL handling
//! - Centralized configuration

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
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
    /// Minimum exe status cache capacity before eviction. The effective cap
    /// scales with the live process count (entries are one per process), see
    /// `ProcessCache::begin_exe_tick`.
    pub const EXE_CACHE_MAX_SIZE: usize = 1000;
    /// Per-tick cap on exe-status filesystem stats. Entries past their
    /// (jittered) deadline are restated across following ticks in whatever
    /// order they come due, so the whole set never stats in one tick.
    pub const EXE_STATS_PER_TICK: u32 = 96;
}

/// FILETIME of the Unix epoch (1601-01-01 → 1970-01-01) in 100ns ticks, used
/// to express mtimes and process create times in the same units.
const UNIX_EPOCH_FILETIME_100NS: u64 = 116444736000000000;

/// How long an exe-status entry stays fresh before its file is re-stat'd.
const EXE_STATUS_TTL: Duration = Duration::from_secs(config::EXE_STATUS_TTL_SECS);

/// Upper bound of the per-entry freshness jitter. Spreading deadlines keeps
/// same-tick entries from expiring (and re-stat'ing) together.
const EXE_STATUS_MAX_JITTER: Duration = Duration::from_secs(config::EXE_STATUS_TTL_SECS / 2);

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
    /// Freshness deadline: `checked_at + TTL + jitter`. Kept separate from
    /// `checked_at` so per-entry jitter can de-synchronize re-stat bursts
    /// while eviction (which graces on `checked_at`) still runs on schedule.
    next_check: Instant,
}

/// Exe-status cache layout: `path -> (process create FILETIME -> entry)`.
///
/// Nested instead of a single `HashMap<(String, u64), _>` composite key so the
/// hot hit-path lookup can borrow the caller's `&str` (`Box<str>: Borrow<str>`
/// hashes and compares through `str`) instead of allocating a `String` per call.
type ExeStatusMap = HashMap<Box<str>, HashMap<u64, ExeStatusEntry>>;

/// [`ExeStatusMap`] plus its total entry count, kept in step with every
/// insert/eviction so the per-insert cap check is O(1) instead of a walk over
/// every path.
#[derive(Default)]
struct ExeStatusCache {
    map: ExeStatusMap,
    len: usize,
}

/// An entry is fresh while `now` is before its (jittered) deadline.
fn exe_entry_is_fresh(entry: &ExeStatusEntry, now: Instant) -> bool {
    now < entry.next_check
}

/// An entry is retained until twice the TTL has passed since its stat. The
/// grace covers the jitter window plus ticks deferred by the per-tick budget,
/// so entries waiting to be restated keep their last verdict.
fn exe_entry_retained(entry: &ExeStatusEntry, now: Instant) -> bool {
    now.saturating_duration_since(entry.checked_at) < EXE_STATUS_TTL + EXE_STATUS_TTL
}

/// Deterministic per-entry jitter in `[0, EXE_STATUS_MAX_JITTER)`, hashed from
/// path and create time so entries created in the same tick get different
/// deadlines without touching a wall clock or RNG.
fn exe_status_jitter(exe_path: &str, start_time_100ns: u64) -> Duration {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    for byte in exe_path.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3); // FNV-1a prime
    }
    hash ^= start_time_100ns;
    hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
    hash ^= hash >> 33;
    let nanos = hash % EXE_STATUS_MAX_JITTER.as_nanos().max(1) as u64;
    Duration::from_nanos(nanos)
}

/// Total number of cached entries across every path (test cross-check for
/// the running count).
#[cfg(test)]
fn exe_status_len(map: &ExeStatusMap) -> usize {
    map.values().map(HashMap::len).sum()
}

/// Drop entries past the retention grace, pruning paths left without any
/// entry. Returns the number of entries removed.
fn evict_expired_entries(map: &mut ExeStatusMap, now: Instant) -> usize {
    let mut removed = 0usize;
    map.retain(|_, by_start| {
        by_start.retain(|_, entry| {
            let retained = exe_entry_retained(entry, now);
            if !retained {
                removed += 1;
            }
            retained
        });
        !by_start.is_empty()
    });
    removed
}

/// Enforce the size cap once it is exceeded: shed expired entries first and
/// wipe the cache wholesale only when everything left is still fresh, i.e.
/// when eviction cannot free enough room. `len` is the running entry count.
fn enforce_exe_cache_limit(map: &mut ExeStatusMap, len: &mut usize, now: Instant, cap: usize) {
    if *len <= cap {
        return;
    }
    *len = len.saturating_sub(evict_expired_entries(map, now));
    if *len > cap {
        map.clear();
        *len = 0;
    }
}

/// Global process cache singleton
pub static CACHE: LazyLock<ProcessCache> = LazyLock::new(ProcessCache::new);

/// Unified process cache
pub struct ProcessCache {
    /// Per-PID cache entries
    entries: RwLock<HashMap<u32, ProcessCacheEntry>>,
    /// Exe status cache (keyed by path+process create FILETIME)
    exe_status: RwLock<ExeStatusCache>,
    /// Cleanup counter for periodic maintenance
    cleanup_counter: AtomicU32,
    /// Remaining exe-status stats allowed this tick (see `begin_exe_tick`)
    exe_budget: AtomicU32,
    /// Exe-status entry cap for the current process population (see
    /// `begin_exe_tick`); never below `config::EXE_CACHE_MAX_SIZE`.
    exe_cap: AtomicUsize,
}

impl ProcessCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            exe_status: RwLock::new(ExeStatusCache::default()),
            cleanup_counter: AtomicU32::new(0),
            exe_budget: AtomicU32::new(0),
            exe_cap: AtomicUsize::new(config::EXE_CACHE_MAX_SIZE),
        }
    }

    /// Open a new collection tick with a fresh exe-stat budget. Entries past
    /// their jittered deadline are restated across ticks under this cap so a
    /// large process set never stats in a single tick.
    ///
    /// `live_processes` sizes the cache: entries are one per process, so a
    /// fixed cap below the process count would wipe fresh verdicts every few
    /// ticks and keep the stat budget saturated (issue #102). Twice the live
    /// count leaves room for exited processes awaiting eviction.
    pub fn begin_exe_tick(&self, budget: u32, live_processes: usize) {
        self.exe_budget.store(budget, Ordering::Relaxed);
        self.exe_cap.store(
            live_processes
                .saturating_mul(2)
                .max(config::EXE_CACHE_MAX_SIZE),
            Ordering::Relaxed,
        );
    }

    /// Take one stat from the budget, if any remain.
    fn take_exe_budget(&self) -> bool {
        self.exe_budget
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |b| b.checked_sub(1))
            .is_ok()
    }

    /// Batch update CPU times and I/O bytes for multiple PIDs (single lock acquisition)
    /// Tuple: (pid, kernel_time, user_time, create_time, io_read, io_write)
    /// Returns a map of PID → (io_read_rate, io_write_rate) computed from cache deltas
    pub fn update_times_batch(
        &self,
        updates: &[(u32, u64, u64, u64, u64, u64)],
    ) -> HashMap<u32, (u64, u64)> {
        let mut io_rates = HashMap::with_capacity(updates.len());
        self.update_times_batch_into(updates, &mut io_rates);
        io_rates
    }

    /// [`ProcessCache::update_times_batch`] filling a caller-owned map so the
    /// per-tick rate map's capacity can be reused across ticks.
    pub fn update_times_batch_into(
        &self,
        updates: &[(u32, u64, u64, u64, u64, u64)],
        io_rates: &mut HashMap<u32, (u64, u64)>,
    ) {
        self.update_times_batch_at_into(updates, Instant::now(), io_rates)
    }

    /// Explicit-instant variant for TTL tests.
    #[cfg(test)]
    fn update_times_batch_at(
        &self,
        updates: &[(u32, u64, u64, u64, u64, u64)],
        now: Instant,
    ) -> HashMap<u32, (u64, u64)> {
        let mut io_rates = HashMap::with_capacity(updates.len());
        self.update_times_batch_at_into(updates, now, &mut io_rates);
        io_rates
    }

    fn update_times_batch_at_into(
        &self,
        updates: &[(u32, u64, u64, u64, u64, u64)],
        now: Instant,
        io_rates: &mut HashMap<u32, (u64, u64)>,
    ) {
        io_rates.clear();
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
        self.check_exe_status_impl(exe_path, start_time_100ns, Instant::now(), false, false)
    }

    /// [`ProcessCache::check_exe_status`] under the collector's staggered
    /// regime: jittered per-entry deadlines spread re-stats out, and the
    /// per-tick budget (see [`ProcessCache::begin_exe_tick`]) defers entries
    /// whose deadline passed until a later tick, keeping their last verdict.
    pub fn check_exe_status_staggered(
        &self,
        exe_path: &str,
        start_time_100ns: u64,
    ) -> (bool, bool) {
        self.check_exe_status_impl(exe_path, start_time_100ns, Instant::now(), true, true)
    }

    /// [`ProcessCache::check_exe_status`] evaluated at an explicit instant,
    /// mirroring `update_times_batch_at` so TTL behavior is testable.
    #[cfg(test)]
    fn check_exe_status_at(
        &self,
        exe_path: &str,
        start_time_100ns: u64,
        now: Instant,
    ) -> (bool, bool) {
        self.check_exe_status_impl(exe_path, start_time_100ns, now, false, false)
    }

    fn check_exe_status_impl(
        &self,
        exe_path: &str,
        start_time_100ns: u64,
        now: Instant,
        jitter: bool,
        budgeted: bool,
    ) -> (bool, bool) {
        use std::fs;

        if exe_path.is_empty() {
            return (false, false);
        }

        // Hot path: borrowed lookup through `Box<str>: Borrow<str>`, so the hit
        // case allocates nothing and reads no wall clock. Only the monotonic
        // `now` decides freshness. Verdicts are copied out so the lock guard
        // can drop before any filesystem work.
        let existing: Option<(bool, bool, bool)> = if let Ok(cache) = self.exe_status.read() {
            cache
                .map
                .get(exe_path)
                .and_then(|by_start| by_start.get(&start_time_100ns))
                .map(|e| (e.updated, e.deleted, exe_entry_is_fresh(e, now)))
        } else {
            None
        };
        if let Some((updated, deleted, fresh)) = existing
            && fresh
        {
            return (updated, deleted);
        }

        // The entry's deadline passed (or it does not exist yet). Under the
        // budget, defer the restat to a later tick: report the last known
        // verdict unchanged, leaving the entry due where it is. First-seen
        // entries report the neutral verdict uncached.
        if budgeted && !self.take_exe_budget() {
            return existing.map_or((false, false), |(updated, deleted, _)| (updated, deleted));
        }

        // Cache miss or due - do filesystem check. The comparison only needs
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
        if let Ok(mut guard) = self.exe_status.write() {
            let cache = &mut *guard;
            let cap = self.exe_cap.load(Ordering::Relaxed);
            enforce_exe_cache_limit(&mut cache.map, &mut cache.len, now, cap);
            let jitter = if jitter {
                exe_status_jitter(exe_path, start_time_100ns)
            } else {
                Duration::ZERO
            };
            let replaced = cache
                .map
                .entry(Box::from(exe_path))
                .or_default()
                .insert(
                    start_time_100ns,
                    ExeStatusEntry {
                        updated: result.0,
                        deleted: result.1,
                        checked_at: now,
                        next_check: now + EXE_STATUS_TTL + jitter,
                    },
                );
            if replaced.is_none() {
                cache.len += 1;
            }
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

    impl ProcessCache {
        fn with_exe_status<R>(&self, f: impl FnOnce(&ExeStatusMap) -> R) -> R {
            let cache = self.exe_status.read().expect("exe_status lock");
            // The running count must always match the map it summarizes.
            assert_eq!(cache.len, exe_status_len(&cache.map));
            f(&cache.map)
        }
    }

    /// `enforce_exe_cache_limit` at the base cap with a freshly counted `len`,
    /// returning the updated running count.
    fn enforce_base_cap(map: &mut ExeStatusMap, now: Instant) -> usize {
        let mut len = exe_status_len(map);
        enforce_exe_cache_limit(map, &mut len, now, config::EXE_CACHE_MAX_SIZE);
        assert_eq!(len, exe_status_len(map), "running count drifted");
        len
    }

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
                    next_check: checked_at + EXE_STATUS_TTL,
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
        // `later` is past the retention grace (2x TTL: TTL plus the jitter +
        // deferral window), so entries stamped at t0 are shed while entries
        // stamped at `later` are still retained.
        let later = t0 + EXE_STATUS_TTL + EXE_STATUS_TTL + Duration::from_secs(1);

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
                        next_check: t0 + EXE_STATUS_TTL,
                    },
                ),
                (
                    8u64,
                    ExeStatusEntry {
                        updated: false,
                        deleted: false,
                        checked_at: later,
                        next_check: later + EXE_STATUS_TTL,
                    },
                ),
            ]),
        );
        assert!(exe_status_len(&map) > config::EXE_CACHE_MAX_SIZE);

        enforce_base_cap(&mut map, later);

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

        assert_eq!(enforce_base_cap(&mut map, t0 + Duration::from_secs(1)), 0);

        assert!(map.is_empty());
    }

    #[test]
    fn caches_under_the_cap_are_left_alone() {
        let t0 = Instant::now();
        let mut map = ExeStatusMap::new();
        add_synthetic_entries(&mut map, 0..64, t0);

        assert_eq!(enforce_base_cap(&mut map, t0 + Duration::from_secs(1)), 64);

        assert_eq!(exe_status_len(&map), 64);
        assert_eq!(map.len(), 64);
    }

    #[test]
    fn expired_only_paths_are_pruned_from_the_outer_map() {
        let t0 = Instant::now();
        let later = t0 + EXE_STATUS_TTL + EXE_STATUS_TTL + Duration::from_secs(1);
        let mut map = ExeStatusMap::new();
        add_synthetic_entries(&mut map, 0..4, t0);
        add_synthetic_entries(&mut map, 100..102, later);

        assert_eq!(evict_expired_entries(&mut map, later), 4);

        assert_eq!(map.len(), 2);
        assert!(contains_path(&map, "C:\\apps\\app100.exe"));
        assert!(contains_path(&map, "C:\\apps\\app101.exe"));
    }

    #[test]
    fn staggered_checks_defer_when_budget_is_exhausted() {
        let cache = ProcessCache::new();
        let t0 = Instant::now();
        // No budget opened: nothing gets stat'd or cached.
        let path = "C:\\defer\\app.exe";
        assert_eq!(
            cache.check_exe_status_impl(path, 1, t0, true, true),
            (false, false)
        );
        assert!(cache.with_exe_status(|m| m.is_empty()));

        // Open a 1-stat budget: the first due entry stats and caches, the
        // second defers with the neutral verdict and stays uncached.
        cache.begin_exe_tick(1, 0);
        assert_eq!(
            cache.check_exe_status_impl(path, 1, t0, true, true),
            (false, true) // nonexistent path -> deleted
        );
        let path2 = "C:\\defer\\app2.exe";
        assert_eq!(
            cache.check_exe_status_impl(path2, 2, t0, true, true),
            (false, false)
        );
        assert!(cache.with_exe_status(|m| !m.contains_key(path2)));
    }

    #[test]
    fn staggered_checks_keep_last_verdict_while_deferred() {
        let cache = ProcessCache::new();
        let t0 = Instant::now();
        cache.begin_exe_tick(1, 0);
        // A deleted exe is stat'd once and cached as deleted.
        let path = "C:\\gone\\app.exe";
        assert_eq!(
            cache.check_exe_status_impl(path, 1, t0, true, true),
            (false, true)
        );
        // Past its deadline with the budget drained: the cached (deleted)
        // verdict holds until a later tick restats - no flicker to (f,t).
        cache.begin_exe_tick(0, 0);
        let later = t0 + EXE_STATUS_TTL + Duration::from_secs(60);
        assert_eq!(
            cache.check_exe_status_impl(path, 1, later, true, true),
            (false, true)
        );
    }

    #[test]
    fn staggered_jitter_spreads_deadlines_across_the_window() {
        // Same-tick entries must not share a deadline: the jitter over 200
        // synthetic keys should cover most of [0, MAX_JITTER).
        let mut deadlines: Vec<u128> = (0..200)
            .map(|i| {
                let path = format!("C:\\apps\\spread{}.exe", i);
                (EXE_STATUS_TTL + exe_status_jitter(&path, 10_000 + i)).as_millis()
            })
            .collect();
        deadlines.sort_unstable();
        let min = deadlines[0];
        let max = deadlines[deadlines.len() - 1];
        let ttl = EXE_STATUS_TTL.as_millis();
        assert!(min < ttl + EXE_STATUS_MAX_JITTER.as_millis() / 4);
        assert!(max > ttl + EXE_STATUS_MAX_JITTER.as_millis() * 3 / 4);
        // Deterministic: same inputs, same deadline.
        assert_eq!(
            exe_status_jitter("C:\\a.exe", 7),
            exe_status_jitter("C:\\a.exe", 7)
        );
    }

    #[test]
    fn staggered_entries_converge_within_a_few_ticks() {
        let cache = ProcessCache::new();
        let t0 = Instant::now();
        // 50 entries come due in the same instant (as a real tick produces);
        // the budget paces their initial caching, and everything is cached
        // after enough ticks.
        cache.begin_exe_tick(config::EXE_STATS_PER_TICK, 0);
        for i in 0..50u64 {
            let path = format!("C:\\conv\\app{}.exe", i);
            cache.check_exe_status_impl(&path, 1_000 + i, t0, true, true);
        }
        let cached = cache.with_exe_status(|m| m.values().map(|b| b.len()).sum::<usize>());
        assert_eq!(cached, 50, "budget of {} covers 50 entries", config::EXE_STATS_PER_TICK);
    }

    #[test]
    fn cap_scales_with_live_processes_so_fresh_entries_survive() {
        // Issue #102: with more live processes than the base cap, every entry
        // is fresh, so a fixed 1000-entry cap wiped the whole cache repeatedly.
        let t0 = Instant::now();
        let mut map = ExeStatusMap::new();
        let entries = config::EXE_CACHE_MAX_SIZE + 200;
        add_synthetic_entries(&mut map, 0..entries, t0);
        let mut len = exe_status_len(&map);

        let cache = ProcessCache::new();
        cache.begin_exe_tick(0, entries);
        let cap = cache.exe_cap.load(Ordering::Relaxed);
        assert_eq!(cap, entries * 2);

        enforce_exe_cache_limit(&mut map, &mut len, t0 + Duration::from_secs(1), cap);
        assert_eq!(len, entries);
        assert_eq!(exe_status_len(&map), entries);

        // A small population never lowers the cap below the base size.
        cache.begin_exe_tick(0, 10);
        assert_eq!(
            cache.exe_cap.load(Ordering::Relaxed),
            config::EXE_CACHE_MAX_SIZE
        );
    }

    #[test]
    fn running_count_tracks_inserts_restats_and_evictions() {
        let cache = ProcessCache::new();
        let t0 = Instant::now();
        cache.begin_exe_tick(u32::MAX, 0);
        for i in 0..20u64 {
            let path = format!("C:\\count\\app{}.exe", i % 5);
            cache.check_exe_status_impl(&path, i, t0, false, true);
        }
        // Re-stat the same keys after the TTL: replaced, not double-counted.
        let later = t0 + EXE_STATUS_TTL + Duration::from_secs(1);
        for i in 0..20u64 {
            let path = format!("C:\\count\\app{}.exe", i % 5);
            cache.check_exe_status_impl(&path, i, later, false, true);
        }
        assert_eq!(cache.with_exe_status(exe_status_len), 20);

        // Evictions keep the count in step (checked inside with_exe_status).
        let mut guard = cache.exe_status.write().unwrap();
        let state = &mut *guard;
        let past_grace = later + EXE_STATUS_TTL + EXE_STATUS_TTL + Duration::from_secs(1);
        enforce_exe_cache_limit(&mut state.map, &mut state.len, past_grace, 0);
        assert_eq!(state.len, 0);
        assert!(state.map.is_empty());
    }
}
