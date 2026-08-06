//! "Free up space" (Favorites sidebar): recursively scans one or more real
//! filesystem roots for the largest files, streaming a running top-N
//! snapshot back to the frontend as the walk progresses instead of
//! blocking until an entire large tree (e.g. a whole home directory) has
//! been walked -- the sheet can start showing candidates immediately and
//! keeps refining the list as more of the tree is covered.

use crate::dirwalk::walk_entries;
use crate::ops::OpRegistry;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;

#[derive(Serialize, Clone)]
pub struct LargeFile {
    pub path: String,
    pub name: String,
    pub size: u64,
}

#[derive(Serialize, Clone)]
pub struct LargeFilesEvent {
    pub files: Vec<LargeFile>,
    pub scanned: u64,
    pub done: bool,
}

// Kept as the max-heap `BinaryHeap` needs, but reversed so the heap's own
// "biggest" (its peek/pop target) is actually the *smallest* size on file --
// letting a bounded top-N scan just `pop()` the smallest candidate whenever
// it grows past the cap, in O(log n), without ever sorting the whole set
// mid-walk.
struct Candidate {
    size: u64,
    path: String,
}
impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        other.size.cmp(&self.size)
    }
}

// Top-N cap kept in memory during the walk. Generous relative to the
// handful the sheet actually displays (RESULT_LIMIT in largefiles-sheet.tsx
// asks for far fewer) so that trimming this down to "the N largest" client
// side never has to worry about a genuinely-larger file having been
// evicted early by the bounded heap.
const MAX_CANDIDATES: usize = 500;
const SEND_INTERVAL: Duration = Duration::from_millis(200);

/// A favorite that's a subdirectory of another selected root (the default
/// favorites list is exactly this: `~/Downloads`, `~/Pictures`, etc. all
/// live inside `~` itself) would otherwise get walked twice, double-
/// counting every file under it. Sorted by path length so a shorter,
/// already-kept root is checked before any longer path that might nest
/// under it.
fn dedupe_nested_roots(mut roots: Vec<String>) -> Vec<String> {
    roots.sort_by_key(|r| r.len());
    let mut kept: Vec<String> = Vec::new();
    for root in roots {
        let nested = kept.iter().any(|k| {
            let kept_path = Path::new(k);
            Path::new(&root) == kept_path || Path::new(&root).starts_with(kept_path)
        });
        if !nested {
            kept.push(root);
        }
    }
    kept
}

fn snapshot(heap: &BinaryHeap<Candidate>, scanned: u64, done: bool) -> LargeFilesEvent {
    let mut files: Vec<LargeFile> = heap
        .iter()
        .map(|c| LargeFile {
            path: c.path.clone(),
            name: Path::new(&c.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| c.path.clone()),
            size: c.size,
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.size));
    LargeFilesEvent { files, scanned, done }
}

fn scan(roots: &[String], channel: &Channel<LargeFilesEvent>, cancel: &Arc<AtomicBool>) {
    let roots = dedupe_nested_roots(roots.to_vec());
    let mut heap: BinaryHeap<Candidate> = BinaryHeap::new();
    let mut scanned: u64 = 0;
    let mut last_sent = Instant::now();
    for root in &roots {
        let outcome = walk_entries(Path::new(root), &mut |entry| {
            if cancel.load(AtomicOrdering::Relaxed) {
                return ControlFlow::Break(());
            }
            let Ok(ft) = entry.file_type() else {
                return ControlFlow::Continue(false);
            };
            // Symlinks are neither sized nor descended into, same as
            // `fs_dir_size_recursive` -- following one into a loop would
            // hang the scan, and a symlink's "size" on disk isn't the
            // size of whatever it points at anyway.
            if ft.is_symlink() {
                return ControlFlow::Continue(false);
            }
            if ft.is_dir() {
                return ControlFlow::Continue(true);
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            scanned += 1;
            heap.push(Candidate {
                size,
                path: crate::path_to_string(entry.path()),
            });
            if heap.len() > MAX_CANDIDATES {
                heap.pop();
            }
            if last_sent.elapsed() >= SEND_INTERVAL {
                last_sent = Instant::now();
                let _ = channel.send(snapshot(&heap, scanned, false));
            }
            ControlFlow::Continue(false)
        });
        if outcome.is_break() {
            break;
        }
    }
    let _ = channel.send(snapshot(&heap, scanned, true));
}

/// Scan `roots` (real filesystem directories -- typically the user's
/// favorited folders, or a single folder they picked) for their largest
/// files, streaming periodic top-N snapshots over `channel` as the walk
/// progresses. Cancellable the same way every other channel-driven
/// operation here is: the frontend's "Stop" calls `cancel_operation` with
/// this channel's own id.
#[tauri::command]
pub async fn scan_large_files(
    roots: Vec<String>,
    channel: Channel<LargeFilesEvent>,
    registry: tauri::State<'_, OpRegistry>,
) -> Result<(), String> {
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    tauri::async_runtime::spawn_blocking(move || scan(&roots, &channel, &cancel))
        .await
        .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    Ok(())
}
