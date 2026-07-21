//! Registry of in-flight cancellable operations. The frontend assigns each
//! long task an `op_id` (the same id its progress row uses) and passes it
//! into the command. The command registers an `OpToken` here; the "cancel"
//! X in the UI calls `cancel_operation(op_id)`, which flips the token's
//! cancel flag and kills any registered child process (ffmpeg/whisper/etc.).
//! Pure-Rust loops poll the flag (via `ProgressReporter::is_cancelled`) and
//! bail between iterations.

use crate::errmap::LockExt;
use std::collections::HashMap;
use std::process::Child;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct OpRegistry {
    ops: Mutex<HashMap<u32, OpEntry>>,
}

struct OpEntry {
    cancel: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

/// Handed to the worker: a shared cancel flag plus a slot to stash a child
/// process so `cancel_operation` can kill it mid-run.
pub struct OpToken {
    pub cancel: Arc<AtomicBool>,
    pub child: Arc<Mutex<Option<Child>>>,
}

impl OpRegistry {
    pub fn register(&self, id: u32) -> OpToken {
        let cancel = Arc::new(AtomicBool::new(false));
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        self.ops.lock_safe().insert(
            id,
            OpEntry {
                cancel: cancel.clone(),
                child: child.clone(),
            },
        );
        OpToken { cancel, child }
    }

    pub fn finish(&self, id: u32) {
        self.ops.lock_safe().remove(&id);
    }

    pub fn cancel(&self, id: u32) {
        if let Some(entry) = self.ops.lock_safe().get(&id) {
            entry.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(mut child) = entry.child.lock_safe().take() {
                let _ = child.kill();
            }
        }
    }
}

#[tauri::command]
pub fn cancel_operation(op_id: u32, registry: tauri::State<OpRegistry>) {
    registry.cancel(op_id);
}
