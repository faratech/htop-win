use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crate::system::{ProcessInfo, SystemMetrics};

/// Snapshot of system state produced by the background data collector
pub struct SystemSnapshot {
    pub metrics: SystemMetrics,
    pub processes: Vec<ProcessInfo>,
    /// How long the refresh took (for benchmark stats)
    pub refresh_duration: Duration,
}

/// Handle for controlling the background data collector thread
pub struct DataCollector {
    /// Set to true to pause sending snapshots (collection continues for accurate deltas)
    pub paused: Arc<AtomicBool>,
    /// Refresh interval in milliseconds (read by collector each tick)
    pub tick_rate_ms: Arc<AtomicU64>,
    /// Send old process vecs back for reuse (avoids string re-allocation)
    pub recycle_tx: mpsc::Sender<Vec<ProcessInfo>>,
}

impl DataCollector {
    /// Spawn the background collection thread.
    /// Performs an initial refresh immediately so the caller can `recv()` the first snapshot.
    pub fn spawn(initial_tick_rate_ms: u64) -> (Self, mpsc::Receiver<SystemSnapshot>) {
        let paused = Arc::new(AtomicBool::new(false));
        let tick_rate_ms = Arc::new(AtomicU64::new(initial_tick_rate_ms));
        let (data_tx, data_rx) = mpsc::channel();
        let (recycle_tx, recycle_rx) = mpsc::channel();

        let handle = DataCollector {
            paused: Arc::clone(&paused),
            tick_rate_ms: Arc::clone(&tick_rate_ms),
            recycle_tx,
        };

        std::thread::Builder::new()
            .name("data-collector".into())
            .spawn({
                let paused = Arc::clone(&paused);
                let tick_rate_ms = Arc::clone(&tick_rate_ms);
                move || Self::run(data_tx, recycle_rx, paused, tick_rate_ms)
            })
            .expect("failed to spawn data collector thread");

        (handle, data_rx)
    }

    fn run(
        data_tx: mpsc::Sender<SystemSnapshot>,
        recycle_rx: mpsc::Receiver<Vec<ProcessInfo>>,
        paused: Arc<AtomicBool>,
        tick_rate_ms: Arc<AtomicU64>,
    ) {
        let mut metrics = SystemMetrics::default();
        let mut processes = Vec::new();

        // Initial refresh -- move the vec, no clone
        let start = Instant::now();
        metrics.refresh();
        metrics.update_processes_native(&mut processes);
        let _ = data_tx.send(SystemSnapshot {
            metrics: metrics.clone(),
            processes: std::mem::take(&mut processes),
            refresh_duration: start.elapsed(),
        });

        loop {
            let rate = tick_rate_ms.load(Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(rate));

            // Pick up recycled vec if available (reuses string allocations)
            // Drain to latest to avoid accumulation
            while let Ok(recycled) = recycle_rx.try_recv() {
                processes = recycled;
            }

            // Always collect (even when paused) to keep cache deltas accurate
            let start = Instant::now();
            metrics.refresh();
            metrics.update_processes_native(&mut processes);
            let duration = start.elapsed();

            if !paused.load(Ordering::Relaxed) {
                // Move the vec -- zero copy, no string allocations
                if data_tx
                    .send(SystemSnapshot {
                        metrics: metrics.clone(),
                        processes: std::mem::take(&mut processes),
                        refresh_duration: duration,
                    })
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}
