//! Event-driven wait for the UI loop: console input OR a published snapshot.
//!
//! crossterm's `event::poll` waits on the console input handle alone (its
//! waker exists only with the `event-stream` feature), so a snapshot the
//! collector thread publishes cannot wake it; the loop would have to poll for
//! snapshots on the ~15.6 ms Windows timer tick. Instead the loop waits here on
//! both the console input buffer and an auto-reset event the collector raises
//! on every publish, so pickup takes a thread wake rather than a timer tick.
//!
//! Input itself is still read through crossterm. Callers must drain it with
//! `event::poll(Duration::ZERO)` before waiting here, so no event crossterm
//! already buffered is left behind the handle wait.

use std::time::Duration;

/// Why [`EventWait::wait`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// The console input buffer has unread records.
    Input,
    /// The collector published a snapshot.
    Snapshot,
    /// The timeout elapsed.
    Timeout,
}

/// Whole milliseconds for a Win32 wait, rounded up so a sub-millisecond
/// remainder waits 1 ms instead of spinning on 0 ms waits, and clamped below
/// `INFINITE` (`u32::MAX`).
pub fn timeout_millis(timeout: Duration) -> u32 {
    timeout
        .as_nanos()
        .div_ceil(1_000_000)
        .min(u128::from(u32::MAX - 1)) as u32
}

#[cfg(windows)]
mod imp {
    use super::{Wake, timeout_millis};
    use std::io;
    use std::sync::Arc;
    use std::time::Duration;

    use windows::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects};
    use windows::core::w;

    /// Kernel handle closed on drop.
    struct OwnedHandle(HANDLE);

    // SAFETY: kernel handles are process-wide values; these are only used
    // through thread-safe Win32 calls (SetEvent, WaitForMultipleObjects).
    unsafe impl Send for OwnedHandle {}
    unsafe impl Sync for OwnedHandle {}

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// Waits on the console input buffer and the snapshot event together.
    pub struct EventWait {
        console_input: OwnedHandle,
        snapshot: Arc<OwnedHandle>,
    }

    /// Cheap, thread-safe handle the collector side uses to wake the UI.
    #[derive(Clone)]
    pub struct SnapshotSignal(Arc<OwnedHandle>);

    impl SnapshotSignal {
        /// Wake the UI loop (a no-op if it is already signaled).
        pub fn raise(&self) {
            unsafe {
                let _ = SetEvent(self.0.0);
            }
        }
    }

    impl EventWait {
        pub fn new() -> io::Result<Self> {
            // Our own handle to the console input buffer crossterm reads (same
            // CONIN$ open it performs), so it is signaled exactly while unread
            // input records exist.
            let console_input = unsafe {
                CreateFileW(
                    w!("CONIN$"),
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAGS_AND_ATTRIBUTES(0),
                    None,
                )
            }
            .map_err(|error| io::Error::other(format!("cannot open CONIN$: {error}")))?;
            let console_input = OwnedHandle(console_input);
            // Auto-reset: one wake per raise, cleared by the wait itself.
            let event = unsafe { CreateEventW(None, false, false, None) }
                .map_err(|error| io::Error::other(format!("CreateEventW failed: {error}")))?;
            Ok(Self {
                console_input,
                snapshot: Arc::new(OwnedHandle(event)),
            })
        }

        pub fn signal_handle(&self) -> SnapshotSignal {
            SnapshotSignal(Arc::clone(&self.snapshot))
        }

        /// Block until console input is pending, a snapshot is signaled, or
        /// `timeout` elapses. Input wins when both are ready; the snapshot
        /// event stays signaled for the next wait.
        pub fn wait(&self, timeout: Duration) -> io::Result<Wake> {
            let handles = [self.console_input.0, self.snapshot.0];
            let result =
                unsafe { WaitForMultipleObjects(&handles, false, timeout_millis(timeout)) };
            if result == WAIT_OBJECT_0 {
                Ok(Wake::Input)
            } else if result.0 == WAIT_OBJECT_0.0 + 1 {
                Ok(Wake::Snapshot)
            } else if result == WAIT_TIMEOUT {
                Ok(Wake::Timeout)
            } else if result == WAIT_FAILED {
                Err(io::Error::last_os_error())
            } else {
                Err(io::Error::other(format!(
                    "unexpected wait result: {:#x}",
                    result.0
                )))
            }
        }
    }
}

/// Non-Windows fallback (the crate targets Windows): plain crossterm polling,
/// so snapshots are picked up at the next input or housekeeping wake.
#[cfg(not(windows))]
mod imp {
    use super::Wake;
    use std::io;
    use std::time::Duration;

    pub struct EventWait;

    #[derive(Clone)]
    pub struct SnapshotSignal;

    impl SnapshotSignal {
        pub fn raise(&self) {}
    }

    impl EventWait {
        pub fn new() -> io::Result<Self> {
            Ok(Self)
        }

        pub fn signal_handle(&self) -> SnapshotSignal {
            SnapshotSignal
        }

        pub fn wait(&self, timeout: Duration) -> io::Result<Wake> {
            Ok(if crossterm::event::poll(timeout)? {
                Wake::Input
            } else {
                Wake::Timeout
            })
        }
    }
}

pub use imp::{EventWait, SnapshotSignal};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_rounds_up_to_whole_milliseconds() {
        assert_eq!(timeout_millis(Duration::ZERO), 0);
        assert_eq!(timeout_millis(Duration::from_micros(1)), 1);
        assert_eq!(timeout_millis(Duration::from_micros(1_500)), 2);
        assert_eq!(timeout_millis(Duration::from_millis(250)), 250);
        // Never INFINITE, however long the request.
        assert_eq!(timeout_millis(Duration::from_secs(u64::MAX)), u32::MAX - 1);
    }

    /// A wait object for the test, or `None` (test skipped) when the harness
    /// has no console or unread console input would mask the snapshot event.
    #[cfg(windows)]
    fn test_waiter() -> Option<EventWait> {
        let Ok(waiter) = EventWait::new() else {
            eprintln!("skipped: no console input buffer in this test harness");
            return None;
        };
        if waiter.wait(Duration::ZERO).ok() == Some(Wake::Input) {
            eprintln!("skipped: console input pending in this test harness");
            return None;
        }
        Some(waiter)
    }

    #[cfg(windows)]
    #[test]
    fn wait_times_out_when_nothing_is_pending() {
        let Some(waiter) = test_waiter() else { return };
        assert_eq!(
            waiter.wait(Duration::from_millis(30)).unwrap(),
            Wake::Timeout
        );
    }

    #[cfg(windows)]
    #[test]
    fn raise_wakes_the_wait_immediately_and_auto_resets() {
        let Some(waiter) = test_waiter() else { return };
        let signal = waiter.signal_handle();
        let (tx, rx) = std::sync::mpsc::channel();
        let raiser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            let raised_at = std::time::Instant::now();
            signal.raise();
            tx.send(raised_at).unwrap();
        });

        let woke = waiter.wait(Duration::from_secs(10)).unwrap();
        let woke_at = std::time::Instant::now();
        raiser.join().unwrap();
        let latency = woke_at.saturating_duration_since(rx.recv().unwrap());
        eprintln!("snapshot signal wake latency: {latency:?}");
        assert_eq!(woke, Wake::Snapshot);
        // A thread wake, not a timer tick; loose bound for loaded CI hosts.
        assert!(latency < Duration::from_millis(500), "{latency:?}");

        // Auto-reset: the wake consumed the signal.
        assert_eq!(
            waiter.wait(Duration::from_millis(20)).unwrap(),
            Wake::Timeout
        );
    }
}
