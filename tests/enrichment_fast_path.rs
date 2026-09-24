//! Metadata enrichment applies cached facts without re-running the query pass,
//! and still queries whatever the cache lacks or has let expire. Its own test
//! binary: it reads and edits this process's entry in the global cache.
#![cfg(windows)]

use std::time::{Duration, Instant};

use htop_win::system::cache::CACHE;
use htop_win::system::{
    ProcessArch, ProcessEnrichmentRequirements, SystemMetrics, apply_cached_metadata,
    enrich_processes_for,
};

#[test]
fn cached_facts_reach_stale_rows_and_expired_facts_are_queried_again() {
    let pid = std::process::id();
    let mut metrics = SystemMetrics::default();
    let mut processes = Vec::new();
    metrics.update_processes_native(&mut processes);
    let raw = processes.iter().find(|p| p.pid == pid).unwrap().clone();
    let requirements = ProcessEnrichmentRequirements::visible(true);

    // First pass queries this process and caches what it learned.
    let mut queried = raw.clone();
    enrich_processes_for(std::slice::from_mut(&mut queried), requirements);
    assert_ne!(&*queried.user, "-", "the query pass resolved the owner");
    assert!(!queried.exe_path.is_empty());

    // A row cloned before that pass (e.g. from a snapshot collected earlier)
    // gets every cached fact, exactly as the query pass left them.
    let mut stale = raw.clone();
    assert_eq!(&*stale.user, "-");
    enrich_processes_for(std::slice::from_mut(&mut stale), requirements);
    assert_eq!(stale.user, queried.user);
    assert_eq!(stale.user_lower, queried.user_lower);
    assert_eq!(stale.exe_path, queried.exe_path);
    assert_eq!(stale.command, queried.command);
    assert_eq!(stale.arch, queried.arch);
    assert_eq!(stale.is_elevated, queried.is_elevated);
    assert_eq!(stale.efficiency_mode, queried.efficiency_mode);

    // An expired efficiency reading is not served from cache: it is queried
    // again and its timestamp refreshed.
    let expired = Instant::now() - Duration::from_secs(60);
    CACHE.update_batch(&[pid], |_, e| e.efficiency_updated = Some(expired));
    enrich_processes_for(std::slice::from_mut(&mut stale), requirements);
    let refreshed = CACHE.with_read(|cache| cache[&pid].efficiency_updated);
    assert!(refreshed.is_some_and(|at| at > expired), "{refreshed:?}");
}

#[test]
fn pseudo_processes_get_fixed_facts_without_a_query() {
    // The System Idle Process has a real create time but no cache entry (the
    // scan skips it), so its facts must never depend on the cache: a row
    // left "needing" a query re-arms the deferred pass on every snapshot.
    let mut metrics = SystemMetrics::default();
    let mut processes = Vec::new();
    metrics.update_processes_native(&mut processes);
    let rows: Vec<usize> = (processes.iter().enumerate())
        .filter(|(_, p)| p.pid == 0 || p.pid == 4)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(rows.len(), 2, "System Idle and System are always listed");
    let requirements = ProcessEnrichmentRequirements::visible(true);

    for _ in 0..2 {
        assert!(!apply_cached_metadata(&mut processes, &rows, requirements));
    }
    for &index in &rows {
        let p = &processes[index];
        assert_eq!(&*p.user, "SYSTEM", "pid {}", p.pid);
        assert_eq!(&*p.user_lower, "system");
        assert_eq!(p.is_elevated, p.pid == 4);
        assert_eq!(p.arch, ProcessArch::Native);
    }
}
