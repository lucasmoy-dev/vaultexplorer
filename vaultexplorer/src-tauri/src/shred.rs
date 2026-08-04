use crate::dirwalk::count_files_recursive;
use crate::errmap::ToStringErr;
use crate::progress::{ProgressEvent, ProgressReporter};
use std::path::Path;
use tauri::ipc::Channel;

const SHRED_PASSES: u32 = 3;

/// Overwrite a single file's content with random bytes `SHRED_PASSES`
/// times, syncing each pass, before unlinking it. NOTE: on SSDs/flash
/// storage with wear-leveling this does not guarantee the original data
/// is unrecoverable -- the drive's controller may write the "overwrite"
/// to different physical cells than the original. Real deletion
/// guarantees there require full-disk encryption or drive-level secure
/// erase, not a userspace overwrite -- the confirm dialog in the
/// frontend states this plainly rather than overselling it.
fn shred_file(path: &Path) -> std::io::Result<()> {
    use rand::RngCore;
    let len = std::fs::metadata(path)?.len();
    let mut buf = vec![0u8; len.min(1024 * 1024).max(1) as usize];
    let mut file = std::fs::OpenOptions::new().write(true).open(path)?;
    for _ in 0..SHRED_PASSES {
        use std::io::{Seek, SeekFrom, Write};
        file.seek(SeekFrom::Start(0))?;
        let mut remaining = len;
        while remaining > 0 {
            let chunk = remaining.min(buf.len() as u64) as usize;
            rand::thread_rng().fill_bytes(&mut buf[..chunk]);
            file.write_all(&buf[..chunk])?;
            remaining -= chunk as u64;
        }
        file.sync_all()?;
    }
    drop(file);
    std::fs::remove_file(path)
}

/// Remove `dir` and everything under it, overwriting file contents first.
/// Best-effort and silent -- for cleanup paths that must not fail loudly or
/// block on a progress channel (see `MountHandle::drop`, which uses it on a
/// just-unmounted FUSE mountpoint: anything left inside is plaintext that
/// landed on the bare directory instead of going through the vault).
pub(crate) fn purge_dir_securely(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        // Already gone, or unreadable -- either way there's nothing to do.
        let _ = std::fs::remove_dir(dir);
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            purge_dir_securely(&path);
        } else if shred_file(&path).is_err() {
            // A file we couldn't overwrite (permissions, a live handle) is
            // still better unlinked than left sitting there.
            let _ = std::fs::remove_file(&path);
        }
    }
    let _ = std::fs::remove_dir(dir);
}

fn shred_recursive(
    path: &Path,
    done: &std::cell::Cell<u64>,
    progress: &ProgressReporter,
) -> std::io::Result<()> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            shred_recursive(&entry?.path(), done, progress)?;
        }
        std::fs::remove_dir(path)?;
    } else {
        shred_file(path)?;
        done.set(done.get() + 1);
        progress.report(done.get());
    }
    Ok(())
}

/// Securely delete real filesystem paths (multi-pass overwrite before
/// unlink, see `shred_file`'s doc comment for the SSD caveat). Not
/// offered for vault-internal content -- ciphertext there gains nothing
/// from shredding over the vault's own remove.
#[tauri::command]
pub(crate) async fn fs_secure_delete(
    paths: Vec<String>,
    channel: Channel<ProgressEvent>,
    registry: tauri::State<'_, crate::ops::OpRegistry>,
) -> Result<(), String> {
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let out = tauri::async_runtime::spawn_blocking(move || {
        let total: u64 = paths.iter().map(|p| count_files_recursive(Path::new(p))).sum();
        let reporter = ProgressReporter::new_cancellable(channel, total.max(1), cancel);
        let done = std::cell::Cell::new(0u64);
        for p in &paths {
            if reporter.is_cancelled() {
                return Err("Cancelled".to_string());
            }
            shred_recursive(Path::new(p), &done, &reporter).str_err()?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    out
}
