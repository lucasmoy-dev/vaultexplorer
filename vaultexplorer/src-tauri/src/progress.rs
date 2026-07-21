//! Throttled progress reporting for long-running commands. The frontend
//! creates a `Channel` per operation and passes it in as a command
//! argument (Tauri 2's recommended way to stream ordered updates, as
//! opposed to a plain named event); we throttle sends here so a tight
//! byte-copy loop doesn't flood the IPC channel with thousands of
//! messages per second.

use crate::errmap::LockExt;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;

#[derive(Clone, Serialize)]
pub struct ProgressEvent {
    pub done: u64,
    pub total: u64,
}

const MIN_INTERVAL: Duration = Duration::from_millis(100);

pub struct ProgressReporter {
    channel: Channel<ProgressEvent>,
    total: u64,
    last_sent: Mutex<Instant>,
    // Set true (via the operation registry, see ops.rs) when the user hits
    // the cancel X. Long loops check `is_cancelled()` between iterations and
    // bail; shell-out helpers additionally kill their child.
    cancel: Arc<AtomicBool>,
}

impl ProgressReporter {
    pub fn new(channel: Channel<ProgressEvent>, total: u64) -> Self {
        ProgressReporter {
            channel,
            total,
            last_sent: Mutex::new(Instant::now() - MIN_INTERVAL),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Like `new`, but wired to a shared cancel flag from the op registry.
    pub fn new_cancellable(channel: Channel<ProgressEvent>, total: u64, cancel: Arc<AtomicBool>) -> Self {
        ProgressReporter {
            channel,
            total,
            last_sent: Mutex::new(Instant::now() - MIN_INTERVAL),
            cancel,
        }
    }

    /// Whether the user requested cancellation of this operation.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Report `done` out of the total this reporter was created with.
    /// Throttled to roughly 10/sec, except the final (done >= total)
    /// call always goes through so the UI reliably sees 100%.
    pub fn report(&self, done: u64) {
        let is_final = done >= self.total;
        let mut last = self.last_sent.lock_safe();
        if !is_final && last.elapsed() < MIN_INTERVAL {
            return;
        }
        *last = Instant::now();
        let _ = self.channel.send(ProgressEvent {
            done,
            total: self.total,
        });
    }
}
