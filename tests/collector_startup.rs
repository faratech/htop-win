//! The collector's startup sequence: the first snapshot must not wait for the
//! CPU counters to be set up (it carries the zero readings their first sample
//! would), and the second arrives early instead of a full interval later.
//! Its own test binary: the collector uses process-wide counter state.
#![cfg(windows)]

use std::time::Duration;

use htop_win::data::DataCollector;

#[test]
fn first_snapshot_skips_counter_setup_and_the_second_comes_early() {
    // An interval far longer than the early second sample.
    let (_collector, snapshots) = DataCollector::spawn(20_000);

    let first = snapshots.recv().unwrap();
    let cpu = &first.metrics.cpu;
    assert!(
        !cpu.core_usage.is_empty(),
        "every logical processor is listed"
    );
    assert!(cpu.core_usage.iter().all(|&usage| usage == 0.0));
    assert_eq!(cpu.core_breakdown.len(), cpu.core_usage.len());
    assert!(!first.processes.is_empty());

    let second = snapshots.recv().unwrap();
    let gap = second
        .published_at
        .saturating_duration_since(first.published_at);
    assert!(gap < Duration::from_secs(10), "second sample after {gap:?}");
    assert_eq!(second.metrics.cpu.core_usage.len(), cpu.core_usage.len());
}
