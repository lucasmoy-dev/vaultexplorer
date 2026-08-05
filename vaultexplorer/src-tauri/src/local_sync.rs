//! Bidirectional sync between two arbitrary local folders (two genuinely
//! separate directories -- not a symlink or bind mount -- whose changes
//! get reflected into each other), via the `unison` CLI, triggered by real
//! filesystem events rather than a blind poll.
//!
//! Unison does have its own `-repeat watch` mode, but it depends on a
//! separate `unison-fsmonitor` helper binary that isn't packaged for this
//! distro at all (confirmed: `-repeat watch` fails outright with "Fatal
//! error: No file monitoring helper program found" without it) -- so this
//! watches both folders itself (via `notify`/`notify-debouncer-mini`) and
//! calls `sync_once` when something changes, debounced so a burst of
//! writes collapses into one sync instead of one per file. A much longer
//! interval still runs as a safety net underneath the watcher, for
//! filesystems/setups where inotify-style watching doesn't work (some
//! network mounts, for instance).
//!
//! Unison's own `-batch` mode already does exactly the right thing for an
//! unattended loop (confirmed against real disposable folders, see the
//! tests below): it propagates every non-conflicting change automatically
//! in both directions, and for a genuine conflict (both sides edited the
//! same file differently since the last sync) it leaves *both* sides
//! untouched and just reports it, rather than guessing which one wins.

use crate::errmap::{LockExt, ToStringErr};
use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(800);
// Only reached if the watcher itself never fires (or the last real event
// was long ago) -- a backstop, not the primary mechanism anymore.
const SAFETY_NET_INTERVAL: Duration = Duration::from_secs(60);

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}
fn config_dir() -> PathBuf {
    Path::new(&home_dir()).join(".config/vaultexplorer")
}
fn pairs_path() -> PathBuf {
    config_dir().join("local_sync_pairs.json")
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LocalSyncPair {
    pub folder_a: String,
    pub folder_b: String,
}

pub fn list_pairs() -> Vec<LocalSyncPair> {
    fs::read_to_string(pairs_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Deleting either side of a pair outside `local_sync_remove` (the normal
/// file-manager delete/trash flow has no idea a folder is one half of a
/// sync pair) used to leave that pair in `local_sync_pairs.json` forever
/// -- every app start restarted its loop, which then failed the same
/// "folder doesn't exist" `sync_once` every safety-net interval,
/// indefinitely, with no UI path left to remove it (the sheet that manages
/// a pair is reached by right-clicking the folder, which is gone). Called
/// once at startup instead of `list_pairs()` so a pair missing either side
/// is dropped for good rather than resurrected on every launch.
pub fn list_pairs_pruning_missing() -> Vec<LocalSyncPair> {
    let pairs = list_pairs();
    let (kept, dropped): (Vec<_>, Vec<_>) =
        pairs.into_iter().partition(|p| Path::new(&p.folder_a).exists() && Path::new(&p.folder_b).exists());
    if !dropped.is_empty() {
        let _ = save_pairs(&kept);
    }
    kept
}

pub fn save_pairs(pairs: &[LocalSyncPair]) -> Result<(), String> {
    fs::create_dir_all(config_dir()).str_err()?;
    let json = serde_json::to_string_pretty(pairs).str_err()?;
    fs::write(pairs_path(), json).str_err()
}

pub fn unison_available() -> bool {
    Command::new("which").arg("unison").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Run one bisync pass between `a` and `b`. `Ok(conflicts)` (empty if
/// none) on a clean or conflicts-only run; `Err` only for a genuine
/// failure (bad path, unison itself erroring out).
///
/// Unison's exit codes, confirmed against real disposable folders: `0` =
/// everything synced; `1` = synced except for the reported conflicts,
/// each left untouched on both sides rather than guessing which wins --
/// not a failure, just something the user needs to resolve by hand; `2`+
/// = a real error (bad arguments, permission problems, etc).
pub fn sync_once(a: &str, b: &str) -> Result<Vec<String>, String> {
    let output = Command::new("unison")
        .arg(a)
        .arg(b)
        .args(["-batch", "-silent"])
        .output()
        .str_err()?;
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if code == 0 {
        return Ok(Vec::new());
    }
    if code == 1 {
        let conflicts: Vec<String> = stdout
            .lines()
            .filter(|l| l.contains("<-?->"))
            .filter_map(|l| l.split_whitespace().last())
            .map(str::to_string)
            .collect();
        return Ok(conflicts);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(if stderr.trim().is_empty() { stdout.trim().to_string() } else { stderr.trim().to_string() })
}

/// `stop` is set from outside (`stop_loop`, a real user-requested "stop
/// syncing"); `alive` is cleared by the loop itself when it self-
/// terminates (a folder it's syncing no longer exists) -- distinct flags
/// because `start_loop`'s "already running, don't spawn a second thread"
/// check needs to tell those two apart. A dead-but-still-in-the-map entry
/// must NOT block a fresh loop from starting if the same pair gets added
/// again later.
#[derive(Default)]
struct LoopHandle {
    stop: AtomicBool,
    alive: AtomicBool,
}

#[derive(Default)]
pub struct LocalSyncState {
    active: Mutex<HashMap<String, Arc<LoopHandle>>>,
    /// Which pairs are mid-`sync_once` *right now* -- lets the frontend
    /// show a real "syncing…" indicator for this unattended background
    /// loop instead of a permanently-static one. An `Arc` of its own
    /// since the spawned thread below needs to share it and can't hold a
    /// borrowed `&LocalSyncState`.
    syncing: Arc<Mutex<HashSet<String>>>,
}

// NUL can't appear in a real path, so it's a safe separator for a
// composite key -- a pair is identified by *both* folders together, not
// just one (the same folder could plausibly be one side of more than one
// pair).
fn pair_key(a: &str, b: &str) -> String {
    format!("{a}\u{0}{b}")
}

pub fn start_loop(state: &LocalSyncState, a: String, b: String) {
    let key = pair_key(&a, &b);
    let mut active = state.active.lock_safe();
    if active.get(&key).is_some_and(|h| h.alive.load(Ordering::Relaxed)) {
        return;
    }
    let handle = Arc::new(LoopHandle { stop: AtomicBool::new(false), alive: AtomicBool::new(true) });
    active.insert(key.clone(), handle.clone());
    drop(active);
    let syncing = state.syncing.clone();

    // A plain OS thread, not a tokio task -- everything this does (the
    // debouncer's blocking recv, unison's own blocking process spawn) is
    // synchronous, so there's no async runtime to gain anything from here.
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        // Falling back to poll-only if the watcher itself can't be set up
        // (e.g. inotify instance limits exhausted) rather than not syncing
        // at all -- degraded, but still correct.
        if let Ok(mut debouncer) = new_debouncer(DEBOUNCE_WINDOW, tx.clone()) {
            let _ = debouncer.watcher().watch(Path::new(&a), RecursiveMode::Recursive);
            let _ = debouncer.watcher().watch(Path::new(&b), RecursiveMode::Recursive);
            // Leaked deliberately: this thread runs for the lifetime of
            // the pair (which can span the whole app session), and the
            // debouncer must outlive it or the watch stops.
            std::mem::forget(debouncer);
        }
        loop {
            if handle.stop.load(Ordering::Relaxed) {
                break;
            }
            // Either a real filesystem-change event arrives (near-
            // instant), or this safety-net timeout elapses first.
            let _ = rx.recv_timeout(SAFETY_NET_INTERVAL);
            if handle.stop.load(Ordering::Relaxed) {
                break;
            }
            // Deleting either folder outside `local_sync_remove` (the
            // normal file-manager delete has no idea it's one half of a
            // sync pair) used to leave this loop retrying a doomed
            // `sync_once` every SAFETY_NET_INTERVAL forever, with no UI
            // path left to stop it (the sheet that manages a pair is
            // reached by right-clicking the folder, which is gone).
            // Self-terminating and pruning the persisted pair is what
            // actually recovers from that instead of just failing quietly
            // and endlessly.
            if !Path::new(&a).exists() || !Path::new(&b).exists() {
                let mut pairs = list_pairs();
                pairs.retain(|p| !(p.folder_a == a && p.folder_b == b));
                let _ = save_pairs(&pairs);
                break;
            }
            syncing.lock_safe().insert(key.clone());
            let _ = sync_once(&a, &b);
            syncing.lock_safe().remove(&key);
        }
        handle.alive.store(false, Ordering::Relaxed);
    });
}

pub fn stop_loop(state: &LocalSyncState, a: &str, b: &str) {
    if let Some(handle) = state.active.lock_safe().remove(&pair_key(a, b)) {
        handle.stop.store(true, Ordering::Relaxed);
    }
}

pub fn is_active(state: &LocalSyncState, a: &str, b: &str) -> bool {
    state.active.lock_safe().get(&pair_key(a, b)).is_some_and(|h| h.alive.load(Ordering::Relaxed))
}

/// Every path (either side of any pair, however the frontend happens to
/// be badging it) that's mid-`sync_once` right now.
pub fn syncing_now(state: &LocalSyncState) -> Vec<String> {
    state
        .syncing
        .lock()
        .unwrap()
        .iter()
        .flat_map(|key| key.split('\u{0}').map(str::to_string).collect::<Vec<_>>())
        .collect()
}

// ---- Tauri commands ----

#[tauri::command]
pub fn local_sync_available() -> bool {
    unison_available()
}

#[tauri::command]
pub fn local_sync_list_pairs() -> Vec<LocalSyncPair> {
    list_pairs()
}

#[tauri::command]
pub fn local_sync_is_active(state: tauri::State<LocalSyncState>, folder_a: String, folder_b: String) -> bool {
    is_active(&state, &folder_a, &folder_b)
}

#[tauri::command]
pub fn local_sync_syncing_now(state: tauri::State<LocalSyncState>) -> Vec<String> {
    syncing_now(&state)
}

#[tauri::command]
pub async fn local_sync_add(
    state: tauri::State<'_, LocalSyncState>,
    folder_a: String,
    folder_b: String,
) -> Result<Vec<String>, String> {
    let (a, b) = (folder_a.clone(), folder_b.clone());
    let conflicts = tokio::task::spawn_blocking(move || sync_once(&a, &b))
        .await
        .str_err()??;

    let mut pairs = list_pairs();
    pairs.retain(|p| !(p.folder_a == folder_a && p.folder_b == folder_b));
    pairs.push(LocalSyncPair {
        folder_a: folder_a.clone(),
        folder_b: folder_b.clone(),
    });
    save_pairs(&pairs)?;
    start_loop(&state, folder_a, folder_b);
    Ok(conflicts)
}

#[tauri::command]
pub fn local_sync_remove(
    state: tauri::State<LocalSyncState>,
    folder_a: String,
    folder_b: String,
) -> Result<(), String> {
    stop_loop(&state, &folder_a, &folder_b);
    let mut pairs = list_pairs();
    pairs.retain(|p| !(p.folder_a == folder_a && p.folder_b == folder_b));
    save_pairs(&pairs)
}

#[tauri::command]
pub async fn local_sync_now(folder_a: String, folder_b: String) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || sync_once(&folder_a, &folder_b))
        .await
        .str_err()?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// First sync propagates a file that only exists on one side; a
    /// second sync propagates a new file added on the *other* side plus
    /// an edit made on the *first* side -- confirming both directions
    /// really do get reconciled by a single `sync_once` call each time,
    /// not just "whichever side unison happens to prefer".
    #[test]
    fn sync_once_propagates_changes_both_directions() {
        let pid = std::process::id();
        let a = format!("/tmp/ve-local-sync-test-a-{pid}");
        let b = format!("/tmp/ve-local-sync-test-b-{pid}");
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(format!("{a}/file1.txt"), "hello").unwrap();

        let conflicts = sync_once(&a, &b).expect("first sync failed");
        assert!(conflicts.is_empty());
        assert_eq!(fs::read_to_string(format!("{b}/file1.txt")).unwrap(), "hello");

        fs::write(format!("{b}/file2.txt"), "world").unwrap();
        fs::write(format!("{a}/file1.txt"), "hello-modified").unwrap();

        let conflicts = sync_once(&a, &b).expect("second sync failed");
        assert!(conflicts.is_empty());
        assert_eq!(fs::read_to_string(format!("{a}/file2.txt")).unwrap(), "world");
        assert_eq!(fs::read_to_string(format!("{b}/file1.txt")).unwrap(), "hello-modified");

        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    /// A real conflict (both sides edited the same file differently since
    /// the last sync) must be reported, and -- critically -- must leave
    /// *both* sides exactly as they were, rather than unison guessing a
    /// winner and silently discarding one side's edit.
    #[test]
    fn sync_once_reports_conflicts_without_touching_either_side() {
        let pid = std::process::id();
        let a = format!("/tmp/ve-local-sync-test-conflict-a-{pid}");
        let b = format!("/tmp/ve-local-sync-test-conflict-b-{pid}");
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(format!("{a}/shared.txt"), "base").unwrap();
        sync_once(&a, &b).expect("baseline sync failed");

        fs::write(format!("{a}/shared.txt"), "edited-in-a").unwrap();
        fs::write(format!("{b}/shared.txt"), "edited-in-b").unwrap();

        let conflicts = sync_once(&a, &b).expect("conflicting sync must report, not error");
        assert_eq!(conflicts, vec!["shared.txt".to_string()]);
        assert_eq!(fs::read_to_string(format!("{a}/shared.txt")).unwrap(), "edited-in-a");
        assert_eq!(fs::read_to_string(format!("{b}/shared.txt")).unwrap(), "edited-in-b");

        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    /// The actual point of the event-driven rewrite: a file dropped into
    /// one folder must reach the other within a couple of seconds, not
    /// only after some fixed poll interval elapses. Runs the real
    /// `start_loop` (a real background thread with a real `notify`
    /// watcher, not a mock) against a disposable pair, well under the old
    /// 25s poll interval this replaced.
    #[test]
    fn start_loop_syncs_promptly_on_a_real_file_change() {
        let pid = std::process::id();
        let a = format!("/tmp/ve-local-sync-test-event-a-{pid}");
        let b = format!("/tmp/ve-local-sync-test-event-b-{pid}");
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();

        let state = LocalSyncState::default();
        start_loop(&state, a.clone(), b.clone());

        // Give the watcher a moment to actually attach before writing --
        // otherwise the write could race its own watch registration.
        std::thread::sleep(Duration::from_millis(300));
        fs::write(format!("{a}/new-file.txt"), "hot off the press").unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut arrived = false;
        while std::time::Instant::now() < deadline {
            if fs::read_to_string(format!("{b}/new-file.txt")).map(|s| s == "hot off the press").unwrap_or(false)
            {
                arrived = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        stop_loop(&state, &a, &b);
        assert!(arrived, "file never reached the other side within 5s of an event-driven loop");

        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }
}
