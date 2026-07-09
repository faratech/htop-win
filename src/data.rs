use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::system::{
    ProcessEnrichmentRequirements, ProcessInfo, SystemMetrics, enrich_processes_for,
    hydrate_processes_from_cache,
};

/// Snapshot of system state produced by the background data collector
pub struct SystemSnapshot {
    pub metrics: SystemMetrics,
    pub processes: Vec<ProcessInfo>,
    /// How long the refresh took (for benchmark stats)
    pub refresh_duration: Duration,
    /// Canonical metadata dependencies queried for this exact process set.
    pub enrichment: ProcessEnrichmentRequirements,
}

struct SnapshotSlotState {
    latest: Option<SystemSnapshot>,
    producer_connected: bool,
    receiver_connected: bool,
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

/// Handle for controlling the background data collector thread
pub struct DataCollector {
    /// Set to true to pause sending snapshots (collection continues for accurate deltas)
    pub paused: Arc<AtomicBool>,
    /// Refresh interval in milliseconds (read by collector each tick)
    pub tick_rate_ms: Arc<AtomicU64>,
    /// Canonical metadata required by active filters/sorts.
    enrichment_requirements: Arc<AtomicU8>,
    /// Send old process vecs back for reuse (avoids string re-allocation)
    pub recycle_tx: mpsc::Sender<Vec<ProcessInfo>>,
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

        let handle = DataCollector {
            paused: Arc::clone(&paused),
            tick_rate_ms: Arc::clone(&tick_rate_ms),
            enrichment_requirements: Arc::clone(&enrichment_requirements),
            recycle_tx,
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

    fn run(
        data_tx: SnapshotPublisher,
        recycle_rx: mpsc::Receiver<Vec<ProcessInfo>>,
        paused: Arc<AtomicBool>,
        tick_rate_ms: Arc<AtomicU64>,
        enrichment_requirements: Arc<AtomicU8>,
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
        hydrate_processes_from_cache(&mut processes);
        if enrichment.any() {
            enrich_processes_for(&mut processes, enrichment);
        }
        if data_tx
            .publish(SystemSnapshot {
                metrics: metrics.clone(),
                processes: std::mem::take(&mut processes),
                refresh_duration: start.elapsed(),
                enrichment,
            })
            .is_err()
        {
            return;
        }
        let mut published_enrichment = enrichment;

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
            let enrichment = ProcessEnrichmentRequirements::from_bits(
                enrichment_requirements.load(Ordering::Acquire),
            );
            hydrate_processes_from_cache(&mut processes);
            if enrichment.any() {
                enrich_processes_for(&mut processes, enrichment);
            }
            let duration = start.elapsed();

            let expands_metadata_coverage = !published_enrichment.contains(enrichment);
            if !paused.load(Ordering::Relaxed) || expands_metadata_coverage {
                // Move the vec without cloning. If the UI has not consumed the
                // previous snapshot, replace it and reuse that older buffer.
                match data_tx.publish(SystemSnapshot {
                    metrics: metrics.clone(),
                    processes: std::mem::take(&mut processes),
                    refresh_duration: duration,
                    enrichment,
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
}
