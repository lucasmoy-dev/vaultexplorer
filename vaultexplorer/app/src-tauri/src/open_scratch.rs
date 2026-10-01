//! Bridges the gap `vault_decrypt_to_temp` opens on desktop: handing an
//! external app (VLC, an editor, anything sandboxed enough that the FUSE
//! mountpoint isn't visible to it) a throwaway plaintext copy means
//! whatever it saves lands on that copy, not on the vault -- without this,
//! the edit is real but invisible to VaultExplorer and gone the moment the
//! scratch dir gets swept. This watches that one file and re-encrypts it
//! back into the vault on every save.
//!
//! Unlike `fs_watch` (one directory watched at a time, replaced on
//! navigate), more than one file can be open externally at once, so each
//! call to `watch` gets its own thread with no shared registry -- there's
//! nothing to coordinate between them, and each one already knows how to
//! stop itself (the vault it's writing back to gets locked, or the temp
//! file disappears).

use crate::AppState;
use notify_debouncer_mini::new_debouncer;
use notify_debouncer_mini::notify::RecursiveMode;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(500);

#[derive(Clone, serde::Serialize)]
struct VaultChanged {
    root: String,
    /// The changed file's *parent* directory, vault-relative -- what the
    /// frontend's current listing is keyed on, not the file itself.
    dir: String,
}

/// Watch `temp_path` (a decrypted scratch copy of `root`'s `rel_path`) and,
/// on every debounced change, re-encrypt its current contents back into
/// the vault at `rel_path`. Stops itself the first time that write fails
/// (most likely because the vault has since been locked) rather than
/// retrying forever -- this is best-effort background sync, not something
/// with its own error UI to report to.
pub fn watch(app: AppHandle, root: String, rel_path: String, temp_path: PathBuf) {
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(mut debouncer) = new_debouncer(DEBOUNCE_WINDOW, tx) else {
            return;
        };
        if debouncer
            .watcher()
            .watch(&temp_path, RecursiveMode::NonRecursive)
            .is_err()
        {
            return;
        }
        // Leaked deliberately, same reason as fs_watch.rs: this thread runs
        // until it decides to stop, and the debouncer must outlive it.
        std::mem::forget(debouncer);
        loop {
            match rx.recv_timeout(Duration::from_millis(300)) {
                Ok(_) => {
                    let Ok(bytes) = std::fs::read(&temp_path) else {
                        // Swept out from under the watch (app restarted, or
                        // the external app deleted-and-recreated it in a
                        // way the debouncer didn't coalesce) -- nothing
                        // left to read back.
                        break;
                    };
                    let state = app.state::<AppState>();
                    let write = crate::with_vault_at(&state, &root, |v| v.write_file(&rel_path, &bytes));
                    if write.is_err() {
                        break;
                    }
                    let dir = Path::new(&rel_path)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let _ = app.emit("vault-changed", VaultChanged { root: root.clone(), dir });
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

/// Removes every `open-<pid>` scratch dir under the app's cache dir except
/// this process's own -- whatever a previous run left behind by not
/// exiting cleanly (a crash, a force-kill), since nothing else will ever
/// clean those up otherwise. Called once at startup, mirroring
/// `sweep_stale_mountpoints`'s same idea for the FUSE side.
pub fn sweep_stale(app: &AppHandle) {
    let Ok(base) = app.path().app_cache_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&base) else {
        return;
    };
    let mine = format!("open-{}", std::process::id());
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("open-") && name != mine.as_str() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}
