//! Live-updates the file list when something changes the currently
//! browsed real-fs folder from outside the app -- a terminal `rm`, a
//! browser download landing in it, another program writing there.
//! Without this the UI only reflects reality again after a manual
//! refresh. Vault-internal browsing doesn't need this: nothing writes
//! into a vault's encrypted storage except this app itself.
//!
//! Only one folder is ever watched at a time (whatever the frontend is
//! currently showing) -- `fs_watch_set` tears down the previous watch
//! before starting the next one, same one-at-a-time model as the
//! frontend's own single current directory.

use notify_debouncer_mini::new_debouncer;
use notify_debouncer_mini::notify::RecursiveMode;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(400);

#[derive(Default)]
pub struct FsWatchState {
    active: Mutex<Option<(String, Arc<AtomicBool>)>>,
}

/// Start watching `path` (non-recursive -- only direct children, matching
/// exactly what the file list shows), replacing whatever was previously
/// watched. Pass `None` to just stop watching (e.g. the frontend
/// navigated into a vault, or to "My Computer").
#[tauri::command]
pub fn fs_watch_set(app: AppHandle, state: State<FsWatchState>, path: Option<String>) {
    let mut active = state.active.lock().unwrap();
    if let Some((_, stop)) = active.take() {
        stop.store(true, Ordering::Relaxed);
    }
    let Some(path) = path else { return };
    let stop = Arc::new(AtomicBool::new(false));
    *active = Some((path.clone(), stop.clone()));
    drop(active);

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(mut debouncer) = new_debouncer(DEBOUNCE_WINDOW, tx) else { return };
        if debouncer
            .watcher()
            .watch(Path::new(&path), RecursiveMode::NonRecursive)
            .is_err()
        {
            return;
        }
        // Leaked deliberately: this thread runs until `fs_watch_set`
        // flips its stop flag, and the debouncer must outlive it or the
        // watch stops.
        std::mem::forget(debouncer);
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(300)) {
                Ok(_) => {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let _ = app.emit("fs-changed", &path);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}
