//! Search-as-you-type, streamed and cancellable.
//!
//! What this replaces were two plain synchronous commands, `fs_search` and
//! `search_vault`. Tauri runs a non-async command on the main thread, so
//! each one froze the whole window (no repaint, no input) for as long as
//! its walk took -- seconds for a home folder -- and with search firing
//! 250ms after every keystroke, the walks queued up one behind another.
//! Nothing could stop an outdated one, and nothing showed until the whole
//! walk was over.
//!
//! Now a search runs on its own thread, sends its hits back in small
//! batches as it finds them (with the metadata a row needs, so the results
//! don't then have to list every parent folder), stops the moment a newer
//! search starts or the user leaves, and stops by itself after
//! `MAX_RESULTS`.

use crate::AppState;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::State;

/// The id of the one search allowed to keep running. Starting a search or
/// cancelling bumps it, and every running walk checks it per entry.
static CURRENT: AtomicU64 = AtomicU64::new(0);

pub const MAX_RESULTS: usize = 1000;
const FLUSH_EVERY: Duration = Duration::from_millis(80);
const FLUSH_AT: usize = 64;

#[derive(Serialize, Clone, Debug)]
pub struct SearchHit {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub is_vault: bool,
    pub size: u64,
    pub mtime: i64,
    /// False for vault hits, whose size/mtime aren't filled in here.
    pub has_meta: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct SearchBatch {
    pub id: u64,
    pub hits: Vec<SearchHit>,
    pub done: bool,
    /// Stopped at `MAX_RESULTS`, not at the end of the tree.
    pub truncated: bool,
}

/// Collects hits and hands them to `send` in batches -- often enough that
/// results appear while the walk is still going, rarely enough that a
/// folder with thousands of matches isn't one IPC message per hit.
struct Batcher<F: FnMut(SearchBatch)> {
    id: u64,
    pending: Vec<SearchHit>,
    last: Instant,
    total: usize,
    send: F,
}

impl<F: FnMut(SearchBatch)> Batcher<F> {
    fn new(id: u64, send: F) -> Self {
        Self { id, pending: Vec::new(), last: Instant::now(), total: 0, send }
    }
    fn live(&self) -> bool {
        CURRENT.load(Ordering::Relaxed) == self.id
    }
    /// Returns false once the search should stop (cap reached).
    fn push(&mut self, hit: SearchHit) -> bool {
        self.pending.push(hit);
        self.total += 1;
        if self.pending.len() >= FLUSH_AT || self.last.elapsed() >= FLUSH_EVERY {
            self.flush(false, false);
        }
        self.total < MAX_RESULTS
    }
    fn tick(&mut self) {
        if !self.pending.is_empty() && self.last.elapsed() >= FLUSH_EVERY {
            self.flush(false, false);
        }
    }
    fn flush(&mut self, done: bool, truncated: bool) {
        self.last = Instant::now();
        if !self.live() {
            return;
        }
        let hits = std::mem::take(&mut self.pending);
        (self.send)(SearchBatch { id: self.id, hits, done, truncated });
    }
    fn finish(mut self) {
        let truncated = self.total >= MAX_RESULTS;
        self.flush(true, truncated);
    }
}

fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Kernel/virtual trees a search from "/" must not wander into: /proc
/// alone is hundreds of thousands of entries that are not files anyone is
/// looking for, and /run holds this app's own FUSE vault mounts.
fn skip_dir(path: &Path) -> bool {
    matches!(path.to_str(), Some("/proc" | "/sys" | "/dev" | "/run" | "/snap"))
}

/// Breadth-first, so what's near the folder being searched shows up
/// first; hidden entries skipped and symlinked directories not followed,
/// like the walk this replaces.
fn walk_fs<F: FnMut(SearchBatch)>(root: &Path, needle: &str, b: &mut Batcher<F>) {
    let mut queue = VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        if !b.live() {
            return;
        }
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            if !b.live() {
                return;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if name.to_lowercase().contains(needle) {
                // Stat through a symlink, so a link to a folder shows as one.
                let meta = std::fs::metadata(&path).ok();
                let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(ft.is_dir());
                let hit = SearchHit {
                    path: crate::path_to_string(path.clone()),
                    name: name.clone(),
                    is_dir,
                    is_vault: is_dir && vaultcore::vault_exists(&path),
                    size: if is_dir { 0 } else { meta.as_ref().map(|m| m.len()).unwrap_or(0) },
                    mtime: meta.as_ref().map(mtime_secs).unwrap_or(0),
                    has_meta: meta.is_some(),
                };
                if !b.push(hit) {
                    return;
                }
            }
            if ft.is_dir() && !ft.is_symlink() && !skip_dir(&path) {
                queue.push_back(path);
            }
        }
        b.tick();
    }
}

/// Stop whatever search is running (the search box was cleared, or the
/// user navigated away).
#[tauri::command]
pub fn search_cancel() {
    CURRENT.fetch_add(1, Ordering::Relaxed);
}

/// Start a search and return its id at once; hits arrive on `channel`.
/// `kind` is "fs" (filenames under `root`) or "vault" (names and text
/// content in the active vault). Starting one stops the previous one.
#[tauri::command]
pub async fn search_start(
    state: State<'_, AppState>,
    kind: String,
    root: String,
    query: String,
    channel: Channel<SearchBatch>,
) -> Result<u64, String> {
    let id = CURRENT.fetch_add(1, Ordering::Relaxed) + 1;
    let needle = query.trim().to_lowercase();
    let send = move |batch: SearchBatch| {
        let _ = channel.send(batch);
    };
    if needle.is_empty() {
        Batcher::new(id, send).finish();
        return Ok(id);
    }
    if kind == "vault" {
        let vault = crate::active_vault(&state)?;
        std::thread::spawn(move || {
            let mut b = Batcher::new(id, send);
            let _ = vault.search_streaming(&needle, &mut |rel, is_dir| {
                if !b.live() {
                    return false;
                }
                let path = rel.to_string_lossy().into_owned();
                let name = rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                b.push(SearchHit { path, name, is_dir, is_vault: false, size: 0, mtime: 0, has_meta: false })
            });
            b.finish();
        });
    } else {
        let root = PathBuf::from(root);
        std::thread::spawn(move || {
            let mut b = Batcher::new(id, send);
            walk_fs(&root, &needle, &mut b);
            b.finish();
        });
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The tests share CURRENT (a cancel in one would silence another).
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn tree() -> PathBuf {
        let root = std::env::temp_dir().join(format!("vx-search-test-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b/c")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("Report.txt"), b"1").unwrap();
        std::fs::write(root.join("a/b/c/deep-report.pdf"), b"22").unwrap();
        std::fs::write(root.join(".hidden/report.txt"), b"3").unwrap();
        std::fs::write(root.join("a/other.txt"), b"4").unwrap();
        root
    }

    fn run(root: &Path, needle: &str) -> Vec<SearchBatch> {
        let id = CURRENT.fetch_add(1, Ordering::Relaxed) + 1;
        let mut out = Vec::new();
        {
            let mut b = Batcher::new(id, |batch| out.push(batch));
            walk_fs(root, needle, &mut b);
            b.finish();
        }
        out
    }

    #[test]
    fn finds_nearest_first_with_metadata_and_skips_hidden() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = tree();
        let batches = run(&root, "report");
        let hits: Vec<_> = batches.iter().flat_map(|b| b.hits.clone()).collect();
        let names: Vec<_> = hits.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["Report.txt", "deep-report.pdf"], "breadth-first, hidden skipped");
        assert_eq!(hits[1].size, 2);
        assert!(hits[1].mtime > 0 && hits[1].has_meta);
        let last = batches.last().unwrap();
        assert!(last.done && !last.truncated);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_newer_search_silences_the_older_one() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = tree();
        let id = CURRENT.fetch_add(1, Ordering::Relaxed) + 1;
        let mut out: Vec<SearchBatch> = Vec::new();
        {
            let mut b = Batcher::new(id, |batch| out.push(batch));
            // Someone starts another search before this walk begins.
            search_cancel();
            walk_fs(&root, "report", &mut b);
            b.finish();
        }
        assert!(out.is_empty(), "a cancelled search must not send anything: {out:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stops_at_the_cap() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("vx-search-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..(MAX_RESULTS + 50) {
            std::fs::write(root.join(format!("hit-{i}")), b"").unwrap();
        }
        let batches = run(&root, "hit");
        let total: usize = batches.iter().map(|b| b.hits.len()).sum();
        assert_eq!(total, MAX_RESULTS);
        assert!(batches.last().unwrap().truncated);
        assert!(batches.len() > 1, "streamed in batches, not one message");
        let _ = std::fs::remove_dir_all(&root);
    }
}
