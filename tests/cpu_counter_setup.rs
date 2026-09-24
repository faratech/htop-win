//! The first collection must not pay for setting up the CPU counters: that
//! happens in `prime_cpu`, after the first frame is published. Its own test
//! binary, so the counters start out unset.
#![cfg(windows)]

use std::time::{Duration, Instant};

use htop_win::system::SystemMetrics;

#[test]
fn counter_setup_happens_in_priming_not_the_first_collection() {
    let mut metrics = SystemMetrics::default();

    let started = Instant::now();
    metrics.refresh_initial();
    let initial = started.elapsed();

    let started = Instant::now();
    metrics.prime_cpu();
    let priming = started.elapsed();

    eprintln!("first collection {initial:?}, counter setup {priming:?}");
    // Setup is only observable where it is expensive (hundreds of ms on
    // typical Windows); there it must land in the priming call.
    if initial.max(priming) > Duration::from_millis(50) {
        assert!(
            priming > initial,
            "counter setup ran in the first collection ({initial:?} vs {priming:?})"
        );
    }
    assert!(metrics.cpu.core_usage.iter().all(|&usage| usage == 0.0));
}
