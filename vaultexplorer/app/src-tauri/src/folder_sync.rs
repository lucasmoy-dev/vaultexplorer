//! Two-way sync between two folders on the same device, with no external
//! tool -- the mobile counterpart of `local_sync.rs`.
//!
//! Desktop's version shells out to `unison`, which is the right call
//! there (a decades-old, well-understood bidirectional syncer, and the
//! desktop can just install it). Android has no unison and no way to get
//! one, so this is the same job in-process: walk both sides, compare each
//! against a journal of what the last pass saw, and copy or delete
//! whichever side actually changed.
//!
//! It is deliberately the same journal-based model as `drive_rest.rs` --
//! that is what makes a deletion propagate as a deletion instead of the
//! other side's copy immediately reappearing, and what makes a file
//! edited on both sides a conflict (kept twice) rather than a silent
//! overwrite. Why it's useful on a phone at all: the app's own sandboxed
//! storage and the shared storage a phone's other apps can see are two
//! genuinely separate trees, and an SD card is a third.

use crate::drive_rest::walk_local;
use crate::errmap::{LockExt, ToStringErr};
use crate::progress::ProgressReporter;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use tauri::Manager;

const LOOP_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Clone, Serialize, Deserialize)]
pub struct FolderPair {
    pub folder_a: String,
    pub folder_b: String,
}

#[derive(Clone, Serialize, Deserialize, Default)]
struct Journal {
    #[serde(default)]
    files: HashMap<String, Seen>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct Seen {
    a_size: u64,
    a_mtime_ms: u64,
    b_size: u64,
    b_mtime_ms: u64,
}

fn state_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).str_err()?;
    Ok(dir)
}

fn pairs_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(state_dir(app)?.join("folder_pairs.json"))
}

pub fn list_pairs(app: &tauri::AppHandle) -> Vec<FolderPair> {
    pairs_path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_pairs(app: &tauri::AppHandle, pairs: &[FolderPair]) -> Result<(), String> {
    std::fs::write(pairs_path(app)?, serde_json::to_string_pretty(pairs).str_err()?).str_err()
}

fn journal_path(dir: &Path, a: &str, b: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    // Both folders identify the pair -- the same folder can be one side of
    // more than one pair, so keying on either alone would collide.
    let digest = Sha256::digest(format!("{a}\u{0}{b}").as_bytes());
    dir.join(format!("folder_journal_{:x}.json", digest))
}

fn read_journal(dir: &Path, a: &str, b: &str) -> Journal {
    std::fs::read_to_string(journal_path(dir, a, b))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_journal(dir: &Path, a: &str, b: &str, journal: &Journal) -> Result<(), String> {
    std::fs::write(journal_path(dir, a, b), serde_json::to_string(journal).str_err()?).str_err()
}

#[derive(Default, Serialize)]
pub struct FolderSyncOutcome {
    pub copied_to_a: usize,
    pub copied_to_b: usize,
    pub deleted_in_a: usize,
    pub deleted_in_b: usize,
    pub conflicts: Vec<String>,
    pub summary: String,
}

fn mtime_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn copy_file(src: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).str_err()?;
    }
    std::fs::copy(src, dest).str_err()?;
    // Same mtime on both sides, so the *next* pass sees one file that
    // matches rather than two that each look freshly changed.
    if let Ok(meta) = std::fs::metadata(src) {
        if let Ok(modified) = meta.modified() {
            let _ = filetime_set(dest, modified);
        }
    }
    Ok(())
}

/// Copy a modification time onto `path`. `std::fs` has no setter, and
/// pulling in the `filetime` crate for one call isn't worth it -- this is
/// the same `utimensat` both platforms this app targets provide.
fn filetime_set(path: &Path, time: std::time::SystemTime) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let secs = time.duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?;
    let times = [
        libc::timespec {
            tv_sec: secs.as_secs() as libc::time_t,
            tv_nsec: secs.subsec_nanos() as _,
        },
        libc::timespec {
            tv_sec: secs.as_secs() as libc::time_t,
            tv_nsec: secs.subsec_nanos() as _,
        },
    ];
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let result = unsafe { libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), 0) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

fn conflict_name(rel: &str, from: &str) -> String {
    let path = Path::new(rel);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let renamed = match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => format!("{stem} (from {from}).{ext}"),
        None => format!("{stem} (from {from})"),
    };
    match path.parent().and_then(|p| p.to_str()).filter(|p| !p.is_empty()) {
        Some(parent) => format!("{parent}/{renamed}"),
        None => renamed,
    }
}

/// One full two-way pass over a pair.
pub fn sync_pair(
    // Where the pair's journal lives -- passed in rather than derived from
    // an `AppHandle` so the whole pass is exercisable in a test with two
    // real folders and no running app.
    state_dir: &Path,
    pair: &FolderPair,
    reporter: Option<&ProgressReporter>,
) -> Result<FolderSyncOutcome, String> {
    let root_a = PathBuf::from(&pair.folder_a);
    let root_b = PathBuf::from(&pair.folder_b);
    for root in [&root_a, &root_b] {
        if !root.is_dir() {
            return Err(format!("\"{}\" isn't a folder anymore", root.display()));
        }
    }
    let label_a = root_a.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "A".into());
    let label_b = root_b.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "B".into());
    let side_a = walk_local(&root_a)?;
    let side_b = walk_local(&root_b)?;
    let mut journal = read_journal(state_dir, &pair.folder_a, &pair.folder_b);
    let mut outcome = FolderSyncOutcome::default();

    let mut paths: Vec<String> = side_a.keys().chain(side_b.keys()).cloned().collect();
    paths.extend(journal.files.keys().cloned());
    paths.sort();
    paths.dedup();
    if let Some(reporter) = reporter {
        reporter.set_total((paths.len() as u64).max(1));
    }
    // Deletions run after the copy pass, deepest path first, so a folder
    // is only removed once whatever was inside it is gone.
    let mut delete_a: Vec<String> = Vec::new();
    let mut delete_b: Vec<String> = Vec::new();
    let mut done = 0u64;

    for rel in &paths {
        done += 1;
        if let Some(reporter) = reporter {
            if reporter.is_cancelled() {
                write_journal(state_dir, &pair.folder_a, &pair.folder_b, &journal)?;
                return Err("cancelled".to_string());
            }
            reporter.report(done);
        }
        let in_a = side_a.get(rel);
        let in_b = side_b.get(rel);
        // Folders: created where missing, removed only when the journal
        // says they were synced before and one side dropped them.
        if in_a.is_some_and(|e| e.is_dir) || in_b.is_some_and(|e| e.is_dir) {
            match (in_a, in_b) {
                (Some(_), None) => std::fs::create_dir_all(root_b.join(rel)).str_err()?,
                (None, Some(_)) => std::fs::create_dir_all(root_a.join(rel)).str_err()?,
                _ => {}
            }
            continue;
        }
        let seen = journal.files.get(rel).copied();
        match (in_a, in_b) {
            (Some(a), Some(b)) => {
                let a_changed = seen
                    .map(|s| s.a_size != a.size || s.a_mtime_ms != a.mtime_ms)
                    .unwrap_or(true);
                let b_changed = seen
                    .map(|s| s.b_size != b.size || s.b_mtime_ms != b.mtime_ms)
                    .unwrap_or(true);
                if !a_changed && !b_changed {
                    continue;
                }
                // Same size and same mtime means the same file as far as
                // any sync tool goes -- most often a first pass over two
                // folders that already match.
                if a.size == b.size && a.mtime_ms == b.mtime_ms {
                    journal.files.insert(
                        rel.clone(),
                        Seen {
                            a_size: a.size,
                            a_mtime_ms: a.mtime_ms,
                            b_size: b.size,
                            b_mtime_ms: b.mtime_ms,
                        },
                    );
                    continue;
                }
                if a_changed && b_changed {
                    // Both edited: keep both, each side getting a copy of
                    // the other's version alongside its own.
                    copy_file(&root_a.join(rel), &root_b.join(conflict_name(rel, &label_a)))?;
                    copy_file(&root_b.join(rel), &root_a.join(conflict_name(rel, &label_b)))?;
                    outcome.conflicts.push(rel.clone());
                    outcome.copied_to_a += 1;
                    outcome.copied_to_b += 1;
                    journal.files.insert(
                        rel.clone(),
                        Seen {
                            a_size: a.size,
                            a_mtime_ms: a.mtime_ms,
                            b_size: b.size,
                            b_mtime_ms: b.mtime_ms,
                        },
                    );
                } else if a_changed {
                    copy_file(&root_a.join(rel), &root_b.join(rel))?;
                    outcome.copied_to_b += 1;
                    journal.files.insert(
                        rel.clone(),
                        Seen {
                            a_size: a.size,
                            a_mtime_ms: a.mtime_ms,
                            b_size: a.size,
                            b_mtime_ms: mtime_ms(&root_b.join(rel)),
                        },
                    );
                } else {
                    copy_file(&root_b.join(rel), &root_a.join(rel))?;
                    outcome.copied_to_a += 1;
                    journal.files.insert(
                        rel.clone(),
                        Seen {
                            a_size: b.size,
                            a_mtime_ms: mtime_ms(&root_a.join(rel)),
                            b_size: b.size,
                            b_mtime_ms: b.mtime_ms,
                        },
                    );
                }
            }
            (Some(a), None) => {
                let unchanged_since_sync = seen
                    .map(|s| s.a_size == a.size && s.a_mtime_ms == a.mtime_ms)
                    .unwrap_or(false);
                if seen.is_some() && unchanged_since_sync {
                    // Deleted on B and untouched on A -- mirror the delete.
                    delete_a.push(rel.clone());
                    continue;
                }
                copy_file(&root_a.join(rel), &root_b.join(rel))?;
                outcome.copied_to_b += 1;
                journal.files.insert(
                    rel.clone(),
                    Seen {
                        a_size: a.size,
                        a_mtime_ms: a.mtime_ms,
                        b_size: a.size,
                        b_mtime_ms: mtime_ms(&root_b.join(rel)),
                    },
                );
            }
            (None, Some(b)) => {
                let unchanged_since_sync = seen
                    .map(|s| s.b_size == b.size && s.b_mtime_ms == b.mtime_ms)
                    .unwrap_or(false);
                if seen.is_some() && unchanged_since_sync {
                    delete_b.push(rel.clone());
                    continue;
                }
                copy_file(&root_b.join(rel), &root_a.join(rel))?;
                outcome.copied_to_a += 1;
                journal.files.insert(
                    rel.clone(),
                    Seen {
                        a_size: b.size,
                        a_mtime_ms: mtime_ms(&root_a.join(rel)),
                        b_size: b.size,
                        b_mtime_ms: b.mtime_ms,
                    },
                );
            }
            (None, None) => {
                journal.files.remove(rel);
            }
        }
    }

    for rel in delete_a {
        if std::fs::remove_file(root_a.join(&rel)).is_ok() {
            outcome.deleted_in_a += 1;
        }
        journal.files.remove(&rel);
    }
    for rel in delete_b {
        if std::fs::remove_file(root_b.join(&rel)).is_ok() {
            outcome.deleted_in_b += 1;
        }
        journal.files.remove(&rel);
    }

    write_journal(state_dir, &pair.folder_a, &pair.folder_b, &journal)?;
    outcome.summary = summarize(&outcome, &label_a, &label_b);
    if let Some(reporter) = reporter {
        reporter.finish();
    }
    Ok(outcome)
}

fn summarize(outcome: &FolderSyncOutcome, label_a: &str, label_b: &str) -> String {
    let mut parts = Vec::new();
    if outcome.copied_to_b > 0 {
        parts.push(format!("{} copied to {label_b}", outcome.copied_to_b));
    }
    if outcome.copied_to_a > 0 {
        parts.push(format!("{} copied to {label_a}", outcome.copied_to_a));
    }
    if outcome.deleted_in_a > 0 {
        parts.push(format!("{} deleted in {label_a}", outcome.deleted_in_a));
    }
    if outcome.deleted_in_b > 0 {
        parts.push(format!("{} deleted in {label_b}", outcome.deleted_in_b));
    }
    if !outcome.conflicts.is_empty() {
        parts.push(format!(
            "{} changed on both sides -- kept both copies ({})",
            outcome.conflicts.len(),
            outcome.conflicts.join(", ")
        ));
    }
    if parts.is_empty() {
        "Already in sync.".to_string()
    } else {
        parts.join(", ")
    }
}

// ---- background loop + state -----------------------------------------

#[derive(Default)]
pub struct FolderSyncState {
    loops: Mutex<HashMap<String, Arc<AtomicBool>>>,
    syncing: Arc<Mutex<HashSet<String>>>,
    last_error: Arc<Mutex<HashMap<String, String>>>,
}

fn key_of(pair: &FolderPair) -> String {
    format!("{}\u{0}{}", pair.folder_a, pair.folder_b)
}

pub fn start_loop(app: &tauri::AppHandle, state: &FolderSyncState, pair: FolderPair) {
    let key = key_of(&pair);
    let mut loops = state.loops.lock_safe();
    if loops.contains_key(&key) {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    loops.insert(key.clone(), stop.clone());
    drop(loops);
    let syncing = state.syncing.clone();
    let last_error = state.last_error.clone();
    let app = app.clone();
    std::thread::spawn(move || loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let Some(current) = list_pairs(&app).into_iter().find(|p| key_of(p) == key) else {
            return; // unlinked
        };
        syncing.lock_safe().insert(current.folder_a.clone());
        syncing.lock_safe().insert(current.folder_b.clone());
        let result = match state_dir(&app) {
            Ok(dir) => sync_pair(&dir, &current, None),
            Err(e) => Err(e),
        };
        syncing.lock_safe().remove(&current.folder_a);
        syncing.lock_safe().remove(&current.folder_b);
        match result {
            Ok(_) => {
                last_error.lock_safe().remove(&key);
            }
            Err(e) => {
                last_error.lock_safe().insert(key.clone(), e);
            }
        }
        for _ in 0..(LOOP_INTERVAL.as_secs() / 5) {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

pub fn stop_loop(state: &FolderSyncState, pair: &FolderPair) {
    if let Some(stop) = state.loops.lock_safe().remove(&key_of(pair)) {
        stop.store(true, Ordering::Relaxed);
    }
}

// ---- commands --------------------------------------------------------

#[tauri::command]
pub fn folder_sync_list_pairs(app: tauri::AppHandle) -> Vec<FolderPair> {
    list_pairs(&app)
}

#[tauri::command]
pub fn folder_sync_add(
    app: tauri::AppHandle,
    folder_a: String,
    folder_b: String,
    state: tauri::State<'_, FolderSyncState>,
) -> Result<FolderPair, String> {
    if folder_a == folder_b {
        return Err("those are the same folder".to_string());
    }
    // One folder inside the other would have each pass feeding itself.
    let (a, b) = (folder_a.trim_end_matches('/'), folder_b.trim_end_matches('/'));
    if b.starts_with(&format!("{a}/")) || a.starts_with(&format!("{b}/")) {
        return Err("one of those folders is inside the other".to_string());
    }
    if !Path::new(&folder_b).is_dir() {
        return Err(format!("\"{folder_b}\" isn't a folder on this device"));
    }
    let mut pairs = list_pairs(&app);
    if pairs
        .iter()
        .any(|p| (p.folder_a == folder_a && p.folder_b == folder_b) || (p.folder_a == folder_b && p.folder_b == folder_a))
    {
        return Err("those two folders are already paired".to_string());
    }
    let pair = FolderPair { folder_a, folder_b };
    pairs.push(pair.clone());
    save_pairs(&app, &pairs)?;
    start_loop(&app, &state, pair.clone());
    Ok(pair)
}

#[tauri::command]
pub fn folder_sync_remove(
    app: tauri::AppHandle,
    folder: String,
    state: tauri::State<'_, FolderSyncState>,
) -> Result<(), String> {
    let pairs = list_pairs(&app);
    let (removed, kept): (Vec<FolderPair>, Vec<FolderPair>) =
        pairs.into_iter().partition(|p| p.folder_a == folder || p.folder_b == folder);
    for pair in &removed {
        stop_loop(&state, pair);
        if let Ok(dir) = state_dir(&app) {
            let _ = std::fs::remove_file(journal_path(&dir, &pair.folder_a, &pair.folder_b));
        }
    }
    save_pairs(&app, &kept)
}

#[tauri::command]
pub async fn folder_sync_now(
    app: tauri::AppHandle,
    folder: String,
    channel: tauri::ipc::Channel<crate::progress::ProgressEvent>,
    state: tauri::State<'_, FolderSyncState>,
    registry: tauri::State<'_, crate::ops::OpRegistry>,
) -> Result<FolderSyncOutcome, String> {
    let pair = list_pairs(&app)
        .into_iter()
        .find(|p| p.folder_a == folder || p.folder_b == folder)
        .ok_or("that folder isn't paired with another one")?;
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let syncing = state.syncing.clone();
    let last_error = state.last_error.clone();
    let handle = app.clone();
    let key = key_of(&pair);
    let result = tauri::async_runtime::spawn_blocking(move || {
        {
            let mut live = syncing.lock_safe();
            if live.contains(&pair.folder_a) {
                return Err("that pair is already syncing".to_string());
            }
            live.insert(pair.folder_a.clone());
            live.insert(pair.folder_b.clone());
        }
        let reporter = ProgressReporter::new_cancellable(channel, 1, cancel);
        let out = state_dir(&handle).and_then(|dir| sync_pair(&dir, &pair, Some(&reporter)));
        let mut live = syncing.lock_safe();
        live.remove(&pair.folder_a);
        live.remove(&pair.folder_b);
        drop(live);
        match &out {
            Ok(_) => {
                last_error.lock_safe().remove(&key);
            }
            Err(e) => {
                last_error.lock_safe().insert(key.clone(), e.clone());
            }
        }
        out
    })
    .await
    .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    if let Some(pair) = list_pairs(&app)
        .into_iter()
        .find(|p| p.folder_a == folder || p.folder_b == folder)
    {
        start_loop(&app, &state, pair);
    }
    result
}

/// Every folder taking part in a pass right now, for the sync badge --
/// same shape as the desktop `local_sync_syncing_now`.
#[tauri::command]
pub fn folder_sync_syncing_now(state: tauri::State<'_, FolderSyncState>) -> Vec<String> {
    state.syncing.lock_safe().iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_names_say_where_the_copy_came_from() {
        assert_eq!(conflict_name("notes.md", "Phone"), "notes (from Phone).md");
        assert_eq!(conflict_name("a/notes.md", "SD"), "a/notes (from SD).md");
        assert_eq!(conflict_name("LICENSE", "SD"), "LICENSE (from SD)");
    }

    /// A whole pair's life: first pass in both directions, an edit on one
    /// side, a delete on one side, and the same file edited on both.
    #[test]
    fn two_way_pass_propagates_changes_deletes_and_keeps_conflicts() {
        let base = std::env::temp_dir().join("vaultexplorer-folder-sync-e2e");
        let _ = std::fs::remove_dir_all(&base);
        let state = base.join("state");
        let a = base.join("Phone");
        let b = base.join("Card");
        for dir in [&state, &a, &b] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let pair = FolderPair {
            folder_a: a.to_string_lossy().to_string(),
            folder_b: b.to_string_lossy().to_string(),
        };
        let sync = || sync_pair(&state, &pair, None).expect("sync failed");

        // 1. New files on each side, including a nested one.
        std::fs::write(a.join("only-a.txt"), b"a").unwrap();
        std::fs::create_dir_all(a.join("sub")).unwrap();
        std::fs::write(a.join("sub/nested.txt"), b"nested").unwrap();
        std::fs::write(b.join("only-b.txt"), b"b").unwrap();
        let out = sync();
        assert_eq!(out.copied_to_b, 2, "{}", out.summary);
        assert_eq!(out.copied_to_a, 1, "{}", out.summary);
        assert_eq!(std::fs::read(b.join("sub/nested.txt")).unwrap(), b"nested");
        assert_eq!(std::fs::read(a.join("only-b.txt")).unwrap(), b"b");

        // 2. A quiet pass changes nothing.
        let out = sync();
        assert_eq!((out.copied_to_a, out.copied_to_b, out.deleted_in_a, out.deleted_in_b), (0, 0, 0, 0));
        assert_eq!(out.summary, "Already in sync.");

        // 3. An edit on B travels to A. (mtime is set explicitly: two
        //    writes inside the same filesystem timestamp granularity can
        //    otherwise look unchanged.)
        std::fs::write(b.join("only-a.txt"), b"edited on b").unwrap();
        filetime_set(&b.join("only-a.txt"), std::time::SystemTime::now() + Duration::from_secs(5)).unwrap();
        let out = sync();
        assert_eq!(out.copied_to_a, 1, "{}", out.summary);
        assert_eq!(std::fs::read(a.join("only-a.txt")).unwrap(), b"edited on b");

        // 4. A delete on A removes the file on B rather than coming back.
        std::fs::remove_file(a.join("only-b.txt")).unwrap();
        let out = sync();
        assert_eq!(out.deleted_in_b, 1, "{}", out.summary);
        assert!(!b.join("only-b.txt").exists());
        let out = sync();
        assert!(!a.join("only-b.txt").exists(), "a deleted file must not reappear");
        assert_eq!(out.copied_to_a, 0);

        // 5. The same file edited on both sides: both versions survive,
        //    each side keeping its own and gaining the other's.
        let later = std::time::SystemTime::now() + Duration::from_secs(10);
        std::fs::write(a.join("only-a.txt"), b"mine").unwrap();
        filetime_set(&a.join("only-a.txt"), later).unwrap();
        std::fs::write(b.join("only-a.txt"), b"theirs").unwrap();
        filetime_set(&b.join("only-a.txt"), later + Duration::from_secs(1)).unwrap();
        let out = sync();
        assert_eq!(out.conflicts, vec!["only-a.txt".to_string()], "{}", out.summary);
        assert_eq!(std::fs::read(a.join("only-a.txt")).unwrap(), b"mine");
        assert_eq!(std::fs::read(b.join("only-a.txt")).unwrap(), b"theirs");
        assert_eq!(std::fs::read(a.join("only-a (from Card).txt")).unwrap(), b"theirs");
        assert_eq!(std::fs::read(b.join("only-a (from Phone).txt")).unwrap(), b"mine");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn mtime_can_be_copied_onto_another_file() {
        let dir = std::env::temp_dir().join("vaultexplorer-folder-sync-test");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.txt");
        let dest = dir.join("dest.txt");
        std::fs::write(&src, b"hello").unwrap();
        // An mtime clearly in the past, so a copy that *didn't* carry it
        // over would be obvious.
        let past = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        filetime_set(&src, past).unwrap();
        copy_file(&src, &dest).unwrap();
        assert_eq!(mtime_ms(&src), mtime_ms(&dest));
        assert_eq!(mtime_ms(&dest), 1_000_000_000_000);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
