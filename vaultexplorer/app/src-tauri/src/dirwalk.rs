//! Shared recursive core for this codebase's several "walk a real
//! directory tree, doing something per entry" helpers. Pulled out because
//! `dir_total_size`, `fs_dir_size_recursive`, `count_files_recursive` and
//! `fs_search_recursive` all repeated the same `read_dir(..).flatten()` +
//! recurse shape -- but they genuinely differ in per-entry semantics
//! (whether a symlink is followed or skipped, whether hidden entries are
//! skipped, whether there's an early-exit budget), so `walk_entries`
//! itself stays behavior-neutral and every one of those differences lives
//! in each wrapper's own visitor closure instead, preserving each
//! function's exact original behavior.

use std::ops::ControlFlow;
use std::path::Path;

/// Walk every entry directly and recursively under `dir`, in `read_dir`'s
/// own (unspecified, OS-dependent) order, calling `visit` once per entry.
/// If `dir` itself can't be read (missing, not a directory, permission
/// denied, ...) or an individual `read_dir` entry errors, that
/// directory/entry is silently skipped -- the same `if let Ok(..) =
/// read_dir(..)` / `.flatten()` error-swallowing every caller here
/// already relied on before this was pulled out into one place.
///
/// `visit` decides, per entry:
/// - `ControlFlow::Continue(true)` -- recurse into this entry too (a
///   no-op if it isn't actually a readable directory: recursing just
///   means calling `read_dir` on it, which will fail and get silently
///   skipped exactly like any other unreadable directory).
/// - `ControlFlow::Continue(false)` -- don't recurse into this entry,
///   move on to `dir`'s next entry.
/// - `ControlFlow::Break(())` -- stop the *entire* walk immediately,
///   unwinding out of every recursion level in one shot (used by the
///   filename search's result-budget early exit).
pub(crate) fn walk_entries(
    dir: &Path,
    visit: &mut impl FnMut(&std::fs::DirEntry) -> ControlFlow<(), bool>,
) -> ControlFlow<()> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return ControlFlow::Continue(());
    };
    for entry in read.flatten() {
        let descend = match visit(&entry) {
            ControlFlow::Break(()) => return ControlFlow::Break(()),
            ControlFlow::Continue(descend) => descend,
        };
        if descend {
            match walk_entries(&entry.path(), visit) {
                ControlFlow::Break(()) => return ControlFlow::Break(()),
                ControlFlow::Continue(()) => {}
            }
        }
    }
    ControlFlow::Continue(())
}

/// Recursive plaintext size of a real filesystem path -- if `path` itself
/// isn't a directory, its own metadata size (0 on error). A directory
/// symlink encountered while descending is followed (via `Path::is_dir`,
/// which stats through symlinks), matching the original recursive
/// `if path.is_dir() { .. } else { metadata size }` shape exactly.
pub(crate) fn dir_total_size(path: &Path) -> u64 {
    if path.is_dir() {
        let mut total = 0u64;
        let _ = walk_entries(path, &mut |entry| {
            let p = entry.path();
            if p.is_dir() {
                ControlFlow::Continue(true)
            } else {
                total += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                ControlFlow::Continue(false)
            }
        });
        total
    } else {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }
}

/// Recursive size of a real-fs directory for the "Get Info" panel.
/// Unlike `dir_total_size`, this uses `entry.file_type()` (lstat, doesn't
/// follow symlinks) and explicitly skips any symlink entirely -- neither
/// sized nor descended into -- to avoid the symlink-loop risk
/// `dir_total_size`'s stat-following recursion doesn't guard against.
pub(crate) fn fs_dir_size_recursive(path: &Path) -> u64 {
    let mut total = 0u64;
    let _ = walk_entries(path, &mut |entry| {
        let Ok(ft) = entry.file_type() else {
            return ControlFlow::Continue(false);
        };
        if ft.is_symlink() {
            return ControlFlow::Continue(false);
        }
        if ft.is_dir() {
            ControlFlow::Continue(true)
        } else {
            if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
            ControlFlow::Continue(false)
        }
    });
    total
}

/// Recursive file count under a real filesystem path, for the shred/
/// secure-delete and compress progress totals -- if `path` itself isn't a
/// directory, it counts as the single file it is. Same stat-following
/// (not lstat) directory check as `dir_total_size`, for the same reason:
/// this is the older of the pair, and changing its symlink handling now
/// would change how many files a shred/compress progress bar counts.
pub(crate) fn count_files_recursive(path: &Path) -> u64 {
    if path.is_dir() {
        let mut total = 0u64;
        let _ = walk_entries(path, &mut |entry| {
            if entry.path().is_dir() {
                ControlFlow::Continue(true)
            } else {
                total += 1;
                ControlFlow::Continue(false)
            }
        });
        total
    } else {
        1
    }
}

/// Case-insensitive filename search under `dir`, appending matches to
/// `out` and decrementing the shared `budget` for each one, stopping the
/// entire walk the moment `budget` hits 0. Hidden (dotfile) entries are
/// skipped outright -- neither matched nor descended into. A directory
/// entry is only descended into if it's a plain directory (via
/// `file_type()`, lstat) and not itself a symlink, to avoid following a
/// symlink into a loop -- independent of that: a symlink whose *name*
/// matches the query still gets pushed to `out`, only its own recursion
/// is what's skipped.
pub(crate) fn fs_search_recursive(dir: &Path, query: &str, out: &mut Vec<String>, budget: &mut usize) {
    let _ = walk_entries(dir, &mut |entry| {
        if *budget == 0 {
            return ControlFlow::Break(());
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            return ControlFlow::Continue(false);
        }
        let path = entry.path();
        if name.to_lowercase().contains(query) {
            out.push(crate::path_to_string(path.clone()));
            *budget -= 1;
        }
        match entry.file_type() {
            Ok(ft) if ft.is_dir() && !ft.is_symlink() => ControlFlow::Continue(true),
            _ => ControlFlow::Continue(false),
        }
    });
}
