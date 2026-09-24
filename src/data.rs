use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use crate::system::{
    ProcessEnrichmentRequirements, ProcessInfo, SystemMetrics, enrich_processes_for,
    hydrate_processes_from_cache,
};

/// Apply cached metadata to a fresh process list, querying what `enrichment`
/// needs and the cache lacks (an enrichment pass hydrates as its first step).
fn hydrate_or_enrich(processes: &mut [ProcessInfo], enrichment: ProcessEnrichmentRequirements) {
    if enrichment.any() {
        enrich_processes_for(processes, enrichment);
    } else {
        hydrate_processes_from_cache(processes);
    }
}

/// Snapshot of system state produced by the background data collector
pub struct SystemSnapshot {
    pub metrics: SystemMetrics,
    pub processes: Vec<ProcessInfo>,
    /// How long the refresh took (for benchmark stats)
    pub refresh_duration: Duration,
    /// Canonical metadata dependencies queried for this exact process set.
    pub enrichment: ProcessEnrichmentRequirements,
    /// When the collector published this snapshot (display-lag measurement).
    pub published_at: Instant,
}

struct SnapshotSlotState {
    latest: Option<SystemSnapshot>,
    producer_connected: bool,
    receiver_connected: bool,
    /// Snapshots replaced before the receiver took them (never displayed).
    superseded: u64,
    /// Called after every publish so the UI loop wakes for the snapshot
    /// instead of polling for it (see `SnapshotReceiver::notify_on_publish`).
    notify: Option<Box<dyn Fn() + Send + Sync>>,
}

struct SnapshotSlot {
    state: Mutex<SnapshotSlotState>,
    ready: Condvar,
}

impl SnapshotSlot {
    fn new() -> Self {
        Self {
            state: Mutex::new(SnapshotSlotState {
                latest: None,
                producer_connected: true,
                receiver_connected: true,
                superseded: 0,
                notify: None,
            }),
            ready: Condvar::new(),
        }
    }
}

struct SnapshotPublisher {
    slot: Arc<SnapshotSlot>,
}

#[derive(Debug)]
struct SnapshotDisconnected;

impl SnapshotPublisher {
    /// Publish the newest snapshot and return the superseded process buffer for
    /// immediate reuse. There can never be more than one pending snapshot.
    fn publish(
        &self,
        snapshot: SystemSnapshot,
    ) -> Result<Option<Vec<ProcessInfo>>, SnapshotDisconnected> {
        let Ok(mut state) = self.slot.state.lock() else {
            return Err(SnapshotDisconnected);
        };
        if !state.receiver_connected {
            return Err(SnapshotDisconnected);
        }

        let superseded = state
            .latest
            .replace(snapshot)
            .map(|snapshot| snapshot.processes);
        if superseded.is_some() {
            state.superseded += 1;
        }
        if let Some(notify) = &state.notify {
            notify();
        }
        self.slot.ready.notify_one();
        Ok(superseded)
    }
}

impl Drop for SnapshotPublisher {
    fn drop(&mut self) {
        if let Ok(mut state) = self.slot.state.lock() {
            state.producer_connected = false;
            self.slot.ready.notify_all();
        }
    }
}

/// Capacity-one receiver that always yields the newest collector snapshot.
pub struct SnapshotReceiver {
    slot: Arc<SnapshotSlot>,
}

impl SnapshotReceiver {
    pub fn recv(&self) -> Result<SystemSnapshot, mpsc::RecvError> {
        let mut state = self.slot.state.lock().map_err(|_| mpsc::RecvError)?;
        loop {
            if let Some(snapshot) = state.latest.take() {
                return Ok(snapshot);
            }
            if !state.producer_connected {
                return Err(mpsc::RecvError);
            }
            state = self.slot.ready.wait(state).map_err(|_| mpsc::RecvError)?;
        }
    }

    /// Call `notify` after every publish (from the collector thread), so the
    /// UI can block on its own wait object and still pick snapshots up
    /// immediately. Fires right away if a snapshot is already pending.
    pub fn notify_on_publish(&self, notify: impl Fn() + Send + Sync + 'static) {
        let Ok(mut state) = self.slot.state.lock() else {
            return;
        };
        if state.latest.is_some() {
            notify();
        }
        state.notify = Some(Box::new(notify));
    }

    /// How many snapshots were replaced before the receiver took them.
    pub fn superseded_count(&self) -> u64 {
        self.slot.state.lock().map_or(0, |state| state.superseded)
    }

    pub fn try_recv(&self) -> Result<SystemSnapshot, mpsc::TryRecvError> {
        let mut state = self
            .slot
            .state
            .lock()
            .map_err(|_| mpsc::TryRecvError::Disconnected)?;
        if let Some(snapshot) = state.latest.take() {
            Ok(snapshot)
        } else if state.producer_connected {
            Err(mpsc::TryRecvError::Empty)
        } else {
            Err(mpsc::TryRecvError::Disconnected)
        }
    }
}

impl Drop for SnapshotReceiver {
    fn drop(&mut self) {
        if let Ok(mut state) = self.slot.state.lock() {
            state.receiver_connected = false;
            state.latest = None;
            self.slot.ready.notify_all();
        }
    }
}

/// Early-wake request for the collector's inter-tick sleep. Prefers a
/// high-resolution timer wait: a condition-variable timeout rounds up to the
/// ~15.6 ms Windows timer tick, which started every collection up to a tick
/// late (and capped short refresh intervals at ~64 Hz).
enum CollectorWake {
    Timer(crate::event_wait::DeadlineWait),
    Condvar {
        pending: Mutex<bool>,
        signal: Condvar,
    },
}

impl Default for CollectorWake {
    fn default() -> Self {
        match crate::event_wait::DeadlineWait::new() {
            Ok(wait) => Self::Timer(wait),
            Err(_) => Self::Condvar {
                pending: Mutex::new(false),
                signal: Condvar::new(),
            },
        }
    }
}

impl CollectorWake {
    fn notify(&self) {
        match self {
            Self::Timer(wait) => wait.raise(),
            Self::Condvar { pending, signal } => {
                *pending.lock().unwrap_or_else(PoisonError::into_inner) = true;
                signal.notify_one();
            }
        }
    }

    /// Sleep until `deadline` unless woken first. Returns true when woken
    /// early (the request is consumed).
    fn wait_until(&self, deadline: Instant) -> bool {
        let (pending, signal) = match self {
            Self::Timer(wait) => return wait.wait_until(deadline),
            Self::Condvar { pending, signal } => (pending, signal),
        };
        let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if std::mem::take(&mut *pending) {
                return true;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            if left.is_zero() {
                return false;
            }
            pending = signal
                .wait_timeout(pending, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// Handle for controlling the background data collector thread
pub struct DataCollector {
    /// While true, collection and publishing are skipped (see `run`)
    pub paused: Arc<AtomicBool>,
    /// Refresh interval in milliseconds (read by collector each tick)
    pub tick_rate_ms: Arc<AtomicU64>,
    /// Canonical metadata required by active filters/sorts.
    enrichment_requirements: Arc<AtomicU8>,
    /// Send old process vecs back for reuse (avoids string re-allocation)
    pub recycle_tx: mpsc::Sender<Vec<ProcessInfo>>,
    /// Cuts the collector's sleep short (see `wake`).
    wake: Arc<CollectorWake>,
}

impl DataCollector {
    /// Spawn the background collection thread.
    /// Performs an initial refresh immediately so the caller can `recv()` the first snapshot.
    pub fn spawn(initial_tick_rate_ms: u64) -> (Self, SnapshotReceiver) {
        Self::spawn_with_enrichment(
            initial_tick_rate_ms,
            ProcessEnrichmentRequirements::default(),
        )
    }

    /// Spawn with canonical metadata needed by first-snapshot CLI filters.
    pub fn spawn_with_enrichment(
        initial_tick_rate_ms: u64,
        initial_enrichment: ProcessEnrichmentRequirements,
    ) -> (Self, SnapshotReceiver) {
        let paused = Arc::new(AtomicBool::new(false));
        let tick_rate_ms = Arc::new(AtomicU64::new(initial_tick_rate_ms));
        let enrichment_requirements = Arc::new(AtomicU8::new(initial_enrichment.bits()));
        let snapshot_slot = Arc::new(SnapshotSlot::new());
        let data_tx = SnapshotPublisher {
            slot: Arc::clone(&snapshot_slot),
        };
        let data_rx = SnapshotReceiver {
            slot: snapshot_slot,
        };
        let (recycle_tx, recycle_rx) = mpsc::channel();
        let wake = Arc::new(CollectorWake::default());

        let handle = DataCollector {
            paused: Arc::clone(&paused),
            tick_rate_ms: Arc::clone(&tick_rate_ms),
            enrichment_requirements: Arc::clone(&enrichment_requirements),
            recycle_tx,
            wake: Arc::clone(&wake),
        };

        std::thread::Builder::new()
            .name("data-collector".into())
            .spawn({
                let paused = Arc::clone(&paused);
                let tick_rate_ms = Arc::clone(&tick_rate_ms);
                move || {
                    Self::run(
                        data_tx,
                        recycle_rx,
                        paused,
                        tick_rate_ms,
                        enrichment_requirements,
                        wake,
                    )
                }
            })
            .expect("failed to spawn data collector thread");

        (handle, data_rx)
    }

    pub fn set_enrichment_requirements(&self, requirements: ProcessEnrichmentRequirements) {
        self.enrichment_requirements
            .store(requirements.bits(), Ordering::Release);
    }

    /// Wake the collector from its inter-tick sleep after changing `paused`,
    /// `tick_rate_ms` or the enrichment requirements. It collects immediately
    /// when resuming from pause or when the requirements need metadata the
    /// last snapshot lacks; otherwise it only re-reads its schedule.
    pub fn wake(&self) {
        self.wake.notify();
    }

    fn run(
        data_tx: SnapshotPublisher,
        recycle_rx: mpsc::Receiver<Vec<ProcessInfo>>,
        paused: Arc<AtomicBool>,
        tick_rate_ms: Arc<AtomicU64>,
        enrichment_requirements: Arc<AtomicU8>,
        wake: Arc<CollectorWake>,
    ) {
        let mut metrics = SystemMetrics::default();
        let mut processes = Vec::new();

        // Initial refresh -- move the vec, no clone
        let start = Instant::now();
        metrics.refresh();
        metrics.update_processes_native(&mut processes);
        let enrichment = ProcessEnrichmentRequirements::from_bits(
            enrichment_requirements.load(Ordering::Acquire),
        );
        hydrate_or_enrich(&mut processes, enrichment);

        // Fixed-schedule pacing: sleep until a deadline that advances by the
        // tick rate, so the real period is exactly `rate` instead of
        // `rate + collect time` (sleep-based pacing drifts by the work done).
        let mut last_rate = tick_rate_ms.load(Ordering::Relaxed);
        let mut next_tick = Instant::now() + Duration::from_millis(last_rate.max(1));
        if data_tx
            .publish(SystemSnapshot {
                metrics: metrics.clone(),
                processes: std::mem::take(&mut processes),
                refresh_duration: start.elapsed(),
                enrichment,
                published_at: Instant::now(),
            })
            .is_err()
        {
            return;
        }
        let mut published_enrichment = enrichment;
        // While paused, collection is skipped entirely; the first tick after
        // resume clears stale gap-averaged rates (see below).
        let mut was_paused = false;

        loop {
            let rate = tick_rate_ms.load(Ordering::Relaxed);
            if rate != last_rate {
                // Rate changed (config edit / benchmark mode): re-derive the
                // schedule from now.
                last_rate = rate;
                next_tick = Instant::now() + Duration::from_millis(rate.max(1));
            }
            let now = Instant::now();
            if next_tick > now {
                if wake.wait_until(next_tick) {
                    // Woken early by the UI. Collect now only when it is
                    // waiting on a snapshot: resuming from pause, or metadata
                    // the published snapshot lacks. Otherwise (e.g. a rate
                    // change) re-derive the schedule at the top of the loop.
                    let wanted = ProcessEnrichmentRequirements::from_bits(
                        enrichment_requirements.load(Ordering::Acquire),
                    );
                    let resuming = was_paused && !paused.load(Ordering::Relaxed);
                    if !resuming && published_enrichment.contains(wanted) {
                        continue;
                    }
                    next_tick = Instant::now();
                }
            } else {
                // Ran past the deadline (collect > rate): don't accumulate
                // debt, just run again immediately.
                next_tick = now;
            }
            next_tick += Duration::from_millis(last_rate.max(1));

            let paused_now = paused.load(Ordering::Relaxed);

            // Pick up recycled vec if available (reuses string allocations)
            // Drain to latest to avoid accumulation
            while let Ok(recycled) = recycle_rx.try_recv() {
                processes = recycled;
            }

            // Paused: skip collection entirely (idle, near-zero cost). A
            // dialog that needs metadata not yet published still forces one
            // collect+publish so it can fill its fields.
            if paused_now {
                was_paused = true;
                let enrichment = ProcessEnrichmentRequirements::from_bits(
                    enrichment_requirements.load(Ordering::Acquire),
                );
                if !published_enrichment.contains(enrichment) {
                    let start = Instant::now();
                    metrics.refresh();
                    metrics.update_processes_native(&mut processes);
                    hydrate_or_enrich(&mut processes, enrichment);
                    let duration = start.elapsed();
                    match data_tx.publish(SystemSnapshot {
                        metrics: metrics.clone(),
                        processes: std::mem::take(&mut processes),
                        refresh_duration: duration,
                        enrichment,
                        published_at: Instant::now(),
                    }) {
                        Ok(Some(recycled)) => processes = recycled,
                        Ok(None) => {}
                        Err(_) => break,
                    }
                    published_enrichment = enrichment;
                }
                continue;
            }

            let start = Instant::now();
            metrics.refresh();
            metrics.update_processes_native(&mut processes);
            let enrichment = ProcessEnrichmentRequirements::from_bits(
                enrichment_requirements.load(Ordering::Acquire),
            );
            hydrate_or_enrich(&mut processes, enrichment);
            let duration = start.elapsed();

            if was_paused {
                was_paused = false;
                // Rates on the first tick after a pause would be averages
                // over the whole paused span (every rate denominator uses
                // real elapsed time). Show a clean zero-rate tick instead;
                // baselines were refreshed by this collection, so the next
                // tick shows normal values.
                for process in processes.iter_mut() {
                    process.cpu_percent = 0.0;
                    process.io_read_rate = 0;
                    process.io_write_rate = 0;
                }
                metrics.cpu.core_usage.fill(0.0);
                metrics.net_rx_rate = 0;
                metrics.net_tx_rate = 0;
                metrics.disk_read_rate = 0;
                metrics.disk_write_rate = 0;
            }

            let expands_metadata_coverage = !published_enrichment.contains(enrichment);
            if !paused.load(Ordering::Relaxed) || expands_metadata_coverage {
                // Move the vec without cloning. If the UI has not consumed the
                // previous snapshot, replace it and reuse that older buffer.
                match data_tx.publish(SystemSnapshot {
                    metrics: metrics.clone(),
                    processes: std::mem::take(&mut processes),
                    refresh_duration: duration,
                    enrichment,
                    published_at: Instant::now(),
                }) {
                    Ok(Some(recycled)) => {
                        processes = recycled;
                        published_enrichment = enrichment;
                    }
                    Ok(None) => published_enrichment = enrichment,
                    Err(_) => break,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(marker: u64) -> SystemSnapshot {
        SystemSnapshot {
            metrics: SystemMetrics::default(),
            processes: Vec::with_capacity(marker as usize),
            refresh_duration: Duration::from_millis(marker),
            enrichment: ProcessEnrichmentRequirements::default(),
            published_at: Instant::now(),
        }
    }

    #[test]
    fn pending_snapshot_is_bounded_and_latest_wins() {
        let slot = Arc::new(SnapshotSlot::new());
        let publisher = SnapshotPublisher {
            slot: Arc::clone(&slot),
        };
        let receiver = SnapshotReceiver { slot };

        let first = publisher
            .publish(snapshot(1))
            .unwrap_or_else(|_| panic!("receiver unexpectedly disconnected"));
        assert!(first.is_none());
        let recycled = publisher
            .publish(snapshot(2))
            .unwrap_or_else(|_| panic!("receiver unexpectedly disconnected"))
            .expect("the pending snapshot should be recycled");
        assert_eq!(recycled.capacity(), 1);

        let received = receiver.try_recv().unwrap();
        assert_eq!(received.refresh_duration, Duration::from_millis(2));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(receiver.superseded_count(), 1);
    }

    #[test]
    fn receiver_observes_producer_disconnect() {
        let slot = Arc::new(SnapshotSlot::new());
        let publisher = SnapshotPublisher {
            slot: Arc::clone(&slot),
        };
        let receiver = SnapshotReceiver { slot };

        drop(publisher);
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        assert!(receiver.recv().is_err());
    }

    #[test]
    fn collector_wake_cuts_the_sleep_short_once() {
        let wake = CollectorWake::default();
        wake.notify();
        let far = Instant::now() + Duration::from_secs(60);
        assert!(wake.wait_until(far), "a pending wake returns immediately");
        // The request was consumed: the next sleep runs to its deadline.
        let soon = Instant::now() + Duration::from_millis(5);
        assert!(!wake.wait_until(soon));
        assert!(Instant::now() >= soon);
    }

    #[test]
    fn publish_notifies_the_ui_once_per_snapshot() {
        use std::sync::atomic::AtomicUsize;

        let slot = Arc::new(SnapshotSlot::new());
        let publisher = SnapshotPublisher {
            slot: Arc::clone(&slot),
        };
        let receiver = SnapshotReceiver { slot };
        let wakes = Arc::new(AtomicUsize::new(0));
        receiver.notify_on_publish({
            let wakes = Arc::clone(&wakes);
            move || {
                wakes.fetch_add(1, Ordering::SeqCst);
            }
        });
        assert_eq!(wakes.load(Ordering::SeqCst), 0, "nothing pending yet");

        let _ = publisher.publish(snapshot(1));
        let _ = publisher.publish(snapshot(2));
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn notify_fires_immediately_for_an_already_pending_snapshot() {
        use std::sync::atomic::AtomicUsize;

        let slot = Arc::new(SnapshotSlot::new());
        let publisher = SnapshotPublisher {
            slot: Arc::clone(&slot),
        };
        let receiver = SnapshotReceiver { slot };
        // Published before the UI installed its hook: must not be missed.
        let _ = publisher.publish(snapshot(1));
        let wakes = Arc::new(AtomicUsize::new(0));
        receiver.notify_on_publish({
            let wakes = Arc::clone(&wakes);
            move || {
                wakes.fetch_add(1, Ordering::SeqCst);
            }
        });
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
    }
}
