mod android;
#[cfg(desktop)]
mod app_icon;
mod archive;
#[cfg(desktop)]
mod autostart;
mod archive_browse;
mod clipboard;
mod convert;
mod dirwalk;
mod errmap;
// Desktop-only: see the mobile-scoping note by this Cargo.toml section of
// the same name for why these specifically can't come along to
// Android/iOS (not a choice -- there's nothing on those platforms for
// them to shell out to or hook into).
#[cfg(desktop)]
mod filemanager1;
#[cfg(desktop)]
mod freeze;
mod git;
mod git_sync;
#[cfg(desktop)]
mod fs_watch;
#[cfg(desktop)]
mod local_sync;
mod info;
mod largefiles;
mod machine;
mod metadata;
mod montage;
mod ops;
#[cfg(desktop)]
mod portal;
mod progress;
mod rclone;
mod recovery;
mod reorganize;
mod share;
mod shred;
mod sync;
mod syncthing;
mod verify;
mod terminal;
mod cast;
mod mediaserver;
mod musicorg;
mod ytstreams;
mod webfind;
#[cfg(desktop)]
mod ytdl;
// Not desktop-only: the same loopback embed page is what lets mobile play
// a YouTube result inside the app instead of handing it to the YouTube
// app, so the module has to exist on Android too (its command is
// registered unconditionally).
mod ytembed;
mod thumbnail;
#[cfg(desktop)]
mod transcribe;

use errmap::{LockExt, ToStringErr};
use progress::{ProgressEvent, ProgressReporter};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::ipc::Channel;
use tauri::Manager;
use tauri::State;
use vaultcore::Vault;

// No FUSE on Android/iOS -- there's no OS mechanism to expose the vault's
// decrypted contents as a real mountpoint another app could open a path
// into, so `mount`/`MountHandle` degrades to "nothing to hold onto" there.
// Every *other* vault operation (browse, decrypt, view, edit) goes
// through the `Vault` object directly and never needed FUSE at all --
// only `open_path` (handing a vault file to an external app) does, and
// that's simply unavailable on mobile (see its own cfg-gate below).
#[cfg(desktop)]
struct MountHandle {
    mountpoint: PathBuf,
    /// `Option` only so `Drop` below can drop the session -- unmounting the
    /// FUSE filesystem -- *before* trying to remove the (now plain, empty)
    /// mountpoint directory. Field drop order would run after our own
    /// `drop()` body, and `remove_dir` on a still-mounted path fails.
    session: Option<fuser::BackgroundSession>,
}

/// Whether `path` is currently a mount point, read from the kernel's own
/// view rather than inferred -- `fuser`'s unmount-on-drop is best-effort and
/// silently leaves the mount up if anything holds it busy (a shell sitting in
/// `cd`, an editor with a file open), and "did it actually come down" is the
/// difference between a locked vault and a still-readable one.
#[cfg(desktop)]
fn is_mount_point(path: &Path) -> bool {
    let Ok(mountinfo) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    let target = path.to_string_lossy();
    mountinfo.lines().any(|line| {
        // mountinfo field 5 is the mount point, space-escaped as \040 etc.
        line.split(' ')
            .nth(4)
            .map(|f| f.replace("\\040", " ") == target)
            .unwrap_or(false)
    })
}

/// Locking a vault drops its session, which is supposed to unmount. Two
/// things went wrong with just doing that:
///
/// 1. The mountpoint directory stayed behind -- an empty
///    `vaultexplorer-mnt-<pid>-<hash>` per vault ever opened, surviving both
///    the lock and the app exit.
/// 2. Worse: if anything held the mount busy, the unmount silently failed and
///    the vault's *decrypted view stayed readable through that path after the
///    vault was locked*. Locking has to actually revoke access, so a busy
///    mount is force-detached (`fusermount -uz`, a lazy unmount: the path
///    stops resolving immediately, the mount is torn down once the last
///    holder lets go).
///
/// Anything still inside afterwards is a real file on the bare directory --
/// a write that raced the unmount, or a mount that never came up -- i.e. the
/// one way plaintext can end up at rest here. Those get shredded, not just
/// unlinked. (The mountpoint now lives on tmpfs, so in practice they were
/// never on a disk to begin with; see `mount_base_dir`.)
#[cfg(desktop)]
impl Drop for MountHandle {
    fn drop(&mut self) {
        drop(self.session.take());
        let mountpoint = self.mountpoint.clone();
        // Unmount + cleanup can block (a lazy unmount shells out; shredding
        // leftovers does real I/O) and `drop` runs on whatever thread locked
        // the vault, including the UI's command thread -- so hand it off.
        // Nothing waits on the result; a next-launch sweep is the backstop.
        std::thread::spawn(move || {
            for attempt in 0..5 {
                if !is_mount_point(&mountpoint) {
                    break;
                }
                if attempt == 0 {
                    // Give fuser's own unmount a moment before forcing it.
                    std::thread::sleep(std::time::Duration::from_millis(60));
                    continue;
                }
                let forced = std::process::Command::new("fusermount")
                    .args(["-u", "-z", &mountpoint.to_string_lossy()])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if !forced {
                    let _ = std::process::Command::new("umount")
                        .args(["-l", &mountpoint.to_string_lossy()])
                        .status();
                }
                std::thread::sleep(std::time::Duration::from_millis(60));
            }
            if is_mount_point(&mountpoint) {
                eprintln!(
                    "vault: mountpoint {} could not be detached; leaving it rather than touching a live mount",
                    mountpoint.display()
                );
                return;
            }
            shred::purge_dir_securely(&mountpoint);
        });
    }
}
#[cfg(mobile)]
struct MountHandle {
    mountpoint: PathBuf,
}

/// One unlocked vault. Nested vaults (a vault whose root lives inside
/// another vault's decrypted FUSE view) are just another entry here,
/// keyed by their own root path -- there's nothing hierarchical about the
/// storage itself, only about how a root path happens to be a subpath of
/// another session's mountpoint.
struct VaultSession {
    vault: Vault,
    mount: Mutex<Option<MountHandle>>,
}

#[derive(Default)]
pub(crate) struct AppState {
    vaults: Mutex<HashMap<String, VaultSession>>,
    /// Which vault root the plain (no-root-argument) vault commands
    /// operate on -- set whenever the frontend navigates into a vault
    /// context, so list_dir/search_vault/etc. don't need a root param
    /// threaded through every call site.
    active: Mutex<Option<String>>,
}

#[cfg(desktop)]
#[derive(Default)]
pub(crate) struct FreezeState {
    /// Live FUSE mounts, keyed by original folder path. Removing an entry
    /// drops its `BackgroundSession`, which unmounts (fuser's own Drop
    /// impl) -- same convention as `VaultSession`'s mount handle.
    pub(crate) mounts: Mutex<HashMap<String, fuser::BackgroundSession>>,
}

#[derive(Serialize)]
struct EntryDto {
    name: String,
    is_dir: bool,
    is_vault: bool,
    size: u64,
    mtime: i64,
}

#[derive(Serialize)]
struct FsEntryDto {
    name: String,
    is_dir: bool,
    is_vault: bool,
    is_hidden: bool,
    size: u64,
    mtime: i64,
    created: i64,
}

fn copy_recursive(
    src: &Path,
    dst: &Path,
    done: &std::cell::Cell<u64>,
    progress: &ProgressReporter,
) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()), done, progress)?;
        }
    } else {
        std::fs::copy(src, dst)?;
        done.set(done.get() + std::fs::metadata(src).map(|m| m.len()).unwrap_or(0));
        progress.report(done.get());
    }
    Ok(())
}

pub(crate) fn with_vault<T>(
    state: &State<AppState>,
    f: impl FnOnce(&Vault) -> vaultcore::Result<T>,
) -> Result<T, String> {
    let root = state.active.lock_safe().clone().ok_or("no vault unlocked")?;
    let map = state.vaults.lock_safe();
    let session = map.get(&root).ok_or("no vault unlocked")?;
    f(&session.vault).str_err()
}

#[tauri::command]
fn vault_exists(path: String) -> bool {
    vaultcore::vault_exists(path)
}

#[tauri::command]
fn create_vault(state: State<AppState>, path: String, password: String) -> Result<(), String> {
    let vault = Vault::create(&path, password.as_bytes()).str_err()?;
    state.vaults.lock_safe().insert(
        path.clone(),
        VaultSession {
            vault,
            mount: Mutex::new(None),
        },
    );
    *state.active.lock_safe() = Some(path);
    Ok(())
}

/// Turn an existing (populated) folder into a vault, encrypting its current
/// contents in place -- "Convert to Vault". Snapshots the folder's entries
/// first, writes the vault meta, then absorbs each pre-existing entry into
/// the vault (encrypting + removing the plaintext original).
#[tauri::command]
fn convert_folder_to_vault(state: State<AppState>, path: String, password: String) -> Result<(), String> {
    let root = std::path::PathBuf::from(&path);
    let names: Vec<std::ffi::OsString> = std::fs::read_dir(&root)
        .str_err()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .collect();
    let vault = Vault::create(&path, password.as_bytes()).str_err()?;
    for name in names {
        vault
            .absorb(&root.join(&name), std::path::Path::new(&name))
            .str_err()?;
    }
    state.vaults.lock_safe().insert(
        path.clone(),
        VaultSession {
            vault,
            mount: Mutex::new(None),
        },
    );
    *state.active.lock_safe() = Some(path);
    Ok(())
}

#[tauri::command]
fn unlock_vault(state: State<AppState>, path: String, password: String) -> Result<(), String> {
    let vault = Vault::unlock(&path, password.as_bytes()).str_err()?;
    state.vaults.lock_safe().insert(
        path.clone(),
        VaultSession {
            vault,
            mount: Mutex::new(None),
        },
    );
    *state.active.lock_safe() = Some(path);
    Ok(())
}

/// Make `root` the target of the plain vault commands (list_dir,
/// search_vault, etc.) -- called by the frontend whenever it navigates
/// into a different vault's context, including a nested one.
#[tauri::command]
fn set_active_vault(state: State<AppState>, root: String) {
    *state.active.lock_safe() = Some(root);
}

/// Lock `root` and, cascading, every vault nested inside it (any vault
/// whose root path starts with `root/`) -- dropping each `VaultSession`
/// unmounts its FUSE session first and then zeroizes its keys.
#[tauri::command]
fn lock_vault(state: State<AppState>, root: String) {
    let prefix = format!("{root}/");
    let mut map = state.vaults.lock_safe();
    map.retain(|k, _| *k != root && !k.starts_with(&prefix));
    let mut active = state.active.lock_safe();
    if let Some(a) = active.as_ref() {
        if *a == root || a.starts_with(&prefix) {
            *active = None;
        }
    }
}

#[tauri::command]
fn list_dir(state: State<AppState>, rel_path: String) -> Result<Vec<EntryDto>, String> {
    with_vault(&state, |v| v.list_dir(&rel_path)).map(|entries| {
        entries
            .into_iter()
            .map(|e| EntryDto {
                name: e.name,
                is_dir: e.is_dir,
                is_vault: e.is_vault,
                size: e.size,
                mtime: e.mtime,
            })
            .collect()
    })
}

/// Listing for a specific unlocked vault, not necessarily the active one
/// -- lets the preview pane show a vault's decrypted contents when the
/// vault folder is selected from the real fs (instead of the raw
/// ciphertext names `fs_list` would return). Errs while the vault is
/// locked; the caller renders that as "locked", never falls back to
/// ciphertext.
#[tauri::command]
fn vault_list_dir_at(
    state: State<AppState>,
    root: String,
    rel_path: String,
) -> Result<Vec<EntryDto>, String> {
    let map = state.vaults.lock_safe();
    let session = map.get(&root).ok_or("vault is locked")?;
    session.vault.list_dir(&rel_path).str_err().map(|entries| {
        entries
            .into_iter()
            .map(|e| EntryDto {
                name: e.name,
                is_dir: e.is_dir,
                is_vault: e.is_vault,
                size: e.size,
                mtime: e.mtime,
            })
            .collect()
    })
}

#[tauri::command]
fn search_vault(state: State<AppState>, query: String) -> Result<Vec<String>, String> {
    with_vault(&state, |v| v.search(&query))
        .map(|paths| paths.into_iter().map(path_to_string).collect())
}

#[tauri::command]
fn move_entry(state: State<AppState>, src: String, dest: String) -> Result<(), String> {
    with_vault(&state, |v| v.move_path(&src, &dest))
}

#[tauri::command]
fn copy_entry(state: State<AppState>, src: String, dest: String) -> Result<(), String> {
    with_vault(&state, |v| v.copy_path(&src, &dest))
}

/// Copy a file between two *different* unlocked vaults directly (decrypt
/// under the source vault's key, re-encrypt under the destination's) --
/// `move_entry`/`copy_entry` above only make sense within a single vault,
/// since both go through `with_vault`, which always operates on whichever
/// root is currently "active" (i.e. wherever the frontend is currently
/// browsing). Pasting something copied from vault A while browsing vault B
/// otherwise silently ran the paste against B using a rel path that was
/// only ever valid in A. Files only, same restriction `import_file`/
/// `export_file` already have for the fs<->vault boundary.
#[tauri::command]
fn vault_to_vault_copy(
    state: State<AppState>,
    src_root: String,
    src_rel: String,
    dest_root: String,
    dest_rel: String,
) -> Result<(), String> {
    let plaintext = {
        let map = state.vaults.lock_safe();
        let session = map.get(&src_root).ok_or("Source vault isn't unlocked")?;
        session.vault.decrypt_file(&src_rel).str_err()?
    };
    let map = state.vaults.lock_safe();
    let session = map.get(&dest_root).ok_or("Destination vault isn't unlocked")?;
    session.vault.write_file(&dest_rel, &plaintext).str_err()
}

/// Same as `vault_to_vault_copy`, then removes the source -- the "cut"
/// side of a cross-vault paste.
#[tauri::command]
fn vault_to_vault_move(
    state: State<AppState>,
    src_root: String,
    src_rel: String,
    dest_root: String,
    dest_rel: String,
) -> Result<(), String> {
    vault_to_vault_copy(state.clone(), src_root.clone(), src_rel.clone(), dest_root, dest_rel)?;
    let map = state.vaults.lock_safe();
    let session = map.get(&src_root).ok_or("Source vault isn't unlocked")?;
    session.vault.remove_file(&src_rel).str_err()
}

#[tauri::command]
fn delete_file(state: State<AppState>, rel_path: String) -> Result<(), String> {
    with_vault(&state, |v| v.remove_file(&rel_path))
}

#[tauri::command]
fn delete_dir(state: State<AppState>, rel_path: String) -> Result<(), String> {
    with_vault(&state, |v| v.remove_dir(&rel_path))
}

#[tauri::command]
fn make_dir(state: State<AppState>, rel_path: String) -> Result<(), String> {
    with_vault(&state, |v| v.create_dir(&rel_path))
}

#[tauri::command]
fn new_file(state: State<AppState>, rel_path: String) -> Result<(), String> {
    with_vault(&state, |v| v.write_file(&rel_path, b""))
}

#[tauri::command]
fn import_file(state: State<AppState>, src_path: String, dest_rel: String) -> Result<(), String> {
    with_vault(&state, |v| v.encrypt_file(&src_path, &dest_rel))
}

/// The other direction of `import_file` -- decrypts a vault file out to a
/// real filesystem path, for cut/copy+paste across the vault boundary
/// (files only; a folder needs Encrypt.../Decrypt... from the context
/// menu instead of plain paste, since that's a recursive operation this
/// single-file command doesn't attempt).
#[tauri::command]
fn export_file(state: State<AppState>, rel_path: String, dest_fs_path: String) -> Result<(), String> {
    let bytes = with_vault(&state, |v| v.decrypt_file(&rel_path))?;
    std::fs::write(&dest_fs_path, bytes).str_err()
}

/// Checks `password` against `path`'s on-disk vault metadata without
/// touching any unlocked-vault state -- used before writing a password to
/// the OS keyring (enabling "Unlock automatically" on an already-existing
/// vault via its Vault Settings sheet) so a typo doesn't get stored as an
/// auto-unlock password that will just silently fail every future launch
/// (see `auto_unlock_vaults`'s own doc comment on that failure mode).
#[tauri::command]
fn verify_vault_password(path: String, password: String) -> Result<(), String> {
    Vault::unlock(&path, password.as_bytes()).map(|_| ()).str_err()
}

/// Store `password` in the OS keyring so `root` can be unlocked
/// automatically on the next app launch ("Unlock automatically" advanced
/// vault option). This is a deliberate, opted-in trade-off: it makes the
/// vault open itself on this machine without prompting, at the cost of
/// the password living in the OS's own credential store rather than only
/// in the user's head.
#[tauri::command]
fn set_vault_auto_unlock(root: String, password: String) -> Result<(), String> {
    keyring::Entry::new("vaultexplorer-auto-unlock", &root)
        .and_then(|e| e.set_password(&password))
        .str_err()
}

#[tauri::command]
fn clear_vault_auto_unlock(root: String) -> Result<(), String> {
    if let Ok(e) = keyring::Entry::new("vaultexplorer-auto-unlock", &root) {
        let _ = e.delete_credential();
    }
    Ok(())
}

/// Try to unlock every vault the user has opted into "unlock
/// automatically", called once at app startup. Returns the roots that
/// were actually unlocked (a stored password that no longer matches --
/// e.g. after a manual password change -- is silently skipped rather
/// than treated as a hard error).
#[tauri::command]
fn auto_unlock_vaults(state: State<AppState>, roots: Vec<String>) -> Vec<String> {
    let mut unlocked = Vec::new();
    for root in roots {
        let Ok(entry) = keyring::Entry::new("vaultexplorer-auto-unlock", &root) else {
            continue;
        };
        let Ok(password) = entry.get_password() else {
            continue;
        };
        if let Ok(vault) = Vault::unlock(&root, password.as_bytes()) {
            state.vaults.lock_safe().insert(
                root.clone(),
                VaultSession {
                    vault,
                    mount: Mutex::new(None),
                },
            );
            unlocked.push(root);
        }
    }
    unlocked
}

/// Recursive plaintext size of `rel_path` inside the vault, for the
/// "Get Info" panel -- can be slower than the shallow size shown from
/// [`list_dir`] since it opens every file under a directory.
#[tauri::command]
fn dir_size(state: State<AppState>, rel_path: String) -> Result<u64, String> {
    with_vault(&state, |v| v.dir_size(&rel_path))
}

/// Recursive size of a real-fs directory, for the "Get Info" panel.
#[tauri::command]
fn fs_dir_size(path: String) -> u64 {
    dirwalk::fs_dir_size_recursive(Path::new(&path))
}

/// Move a file or folder to the OS trash/recycle bin (reversible), rather
/// than deleting it outright -- the default "Delete" action. Permanent
/// removal (Ctrl+Delete in the UI) still goes through `fs_delete`.
#[cfg(desktop)]
#[tauri::command]
fn fs_trash(path: String) -> Result<(), String> {
    trash::delete(&path).str_err()
}

/// Trash a whole selection in one call.
///
/// Deleting thousands of files used to be one IPC round-trip *per file*,
/// each running `trash::delete` synchronously on the main thread -- so the
/// window froze for as long as it took and nothing showed the progress.
/// `trash::delete_all` hands the whole list to the platform's trash
/// implementation at once (a single D-Bus/portal call rather than N of
/// them, which is where nearly all of the time went), and it runs off the
/// UI thread with a cancellable Actions row.
///
/// Chunked rather than one giant call so progress moves and a cancel is
/// honoured partway through; the chunk size is a balance between those two
/// and the per-call overhead that made this slow to begin with.
#[cfg(desktop)]
#[tauri::command]
async fn fs_trash_many(
    paths: Vec<String>,
    channel: tauri::ipc::Channel<progress::ProgressEvent>,
    registry: tauri::State<'_, ops::OpRegistry>,
) -> Result<(), String> {
    const CHUNK: usize = 64;
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let total = paths.len() as u64;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let reporter = progress::ProgressReporter::new_cancellable(channel, total.max(1), cancel);
        let mut done = 0u64;
        for chunk in paths.chunks(CHUNK) {
            if reporter.is_cancelled() {
                return Err("Cancelled".to_string());
            }
            trash::delete_all(chunk).str_err()?;
            done += chunk.len() as u64;
            reporter.report(done);
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    result
}

/// No mobile equivalent of the desktop version above: the `trash` crate has
/// no Android/iOS backend, and there's no universal "OS trash can" concept
/// for an arbitrary file path there -- MediaStore's own IS_TRASHED purgatory
/// only covers items already indexed *through* MediaStore, not the direct
/// filesystem paths this app deletes by. Falling through to a real
/// permanent delete is the honest behavior: every call site here already
/// treats this as "the default Delete action", and silently having that
/// action fail outright on mobile (this command simply not existing) is
/// worse than it not being reversible.
#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
async fn fs_trash(path: String) -> Result<(), String> {
    fs_delete(path).await
}

/// The real filesystem path of the OS trash's contents, so it can be
/// browsed like any other folder from the sidebar. Created on first use --
/// nothing's been trashed yet on a fresh system.
#[cfg(desktop)]
#[tauri::command]
fn trash_dir() -> Result<String, String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    let path = format!("{home}/.local/share/Trash/files");
    std::fs::create_dir_all(&path).str_err()?;
    Ok(path)
}

/// Permanently purge everything in the trash, via the same freedesktop
/// trash index `trash::os_limited` reads/writes elsewhere -- rather than
/// just `rm -rf`-ing the files dir, this also clears the paired
/// `.local/share/Trash/info/*.trashinfo` metadata so nothing's left
/// orphaned.
#[cfg(desktop)]
#[tauri::command]
fn empty_trash() -> Result<(), String> {
    let items = trash::os_limited::list().str_err()?;
    trash::os_limited::purge_all(items).str_err()
}

/// Put everything back where it came from.
#[cfg(desktop)]
#[tauri::command]
fn trash_restore_all() -> Result<(), String> {
    let items = trash::os_limited::list().str_err()?;
    trash::os_limited::restore_all(items).str_err()
}

/// Match trash items by their current filename under
/// `~/.local/share/Trash/files` (what the UI shows when browsing the
/// trash folder) -- that's the trashinfo filename's stem, not the
/// original name, since freedesktop appends a numeric suffix (`foo.2`)
/// on name collisions and this must match the actual on-disk entry.
#[cfg(desktop)]
fn match_trash_items(names: Vec<String>) -> Result<Vec<trash::TrashItem>, String> {
    let wanted: std::collections::HashSet<String> = names.into_iter().collect();
    let items = trash::os_limited::list().str_err()?;
    Ok(items
        .into_iter()
        .filter(|item| {
            Path::new(&item.id)
                .file_stem()
                .map(|stem| wanted.contains(&stem.to_string_lossy().into_owned()))
                .unwrap_or(false)
        })
        .collect())
}

/// Restore specific items out of the trash back to their original location.
#[cfg(desktop)]
#[tauri::command]
fn trash_restore(names: Vec<String>) -> Result<(), String> {
    trash::os_limited::restore_all(match_trash_items(names)?).str_err()
}

/// Permanently purge specific items out of the trash (rather than the
/// whole trash via `empty_trash`).
#[cfg(desktop)]
#[tauri::command]
fn trash_purge(names: Vec<String>) -> Result<(), String> {
    trash::os_limited::purge_all(match_trash_items(names)?).str_err()
}

/// Where "Use as Template" stashes a copy of the source file, and where
/// "New From Template" copies back from. A stash rather than a path
/// reference so renaming/deleting the original later can't break a
/// template.
#[tauri::command]
fn templates_dir(app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(mobile)]
    let base = app.path().app_data_dir().str_err()?;
    #[cfg(desktop)]
    let base = {
        let _ = &app;
        PathBuf::from(format!("{}/.local/share/vaultexplorer", std::env::var("HOME").unwrap_or_else(|_| "/".to_string())))
    };
    let path = base.join("templates");
    std::fs::create_dir_all(&path).str_err()?;
    Ok(path_to_string(path))
}

const ENCRYPTED_EXT: &str = "vlt";

/// Encrypt the plain file at `path` with `password` into a standalone
/// `.vlt` file next to it (vaultcore's own format, unrelated to any
/// vault), then delete the plaintext original. Returns the new file's
/// full path.
#[tauri::command]
fn fs_encrypt_file(path: String, password: String) -> Result<String, String> {
    let src = Path::new(&path);
    let plaintext = std::fs::read(src).str_err()?;
    let ciphertext = vaultcore::encrypt_file_with_password(&plaintext, password.as_bytes())
        .str_err()?;
    let dest = PathBuf::from(format!("{}.{}", path, ENCRYPTED_EXT));
    std::fs::write(&dest, ciphertext).str_err()?;
    std::fs::remove_file(src).str_err()?;
    Ok(dest.to_string_lossy().to_string())
}

/// Where a decrypted-for-the-OS-opener file gets scratched to. On desktop
/// this is `std::env::temp_dir()` (tmpfs on this app's other RAM-backed
/// scratch dirs, see the FUSE mountpoint doc comment below) -- unchanged
/// from before. On Android, `temp_dir()` (both the std one and Tauri's own
/// `path().temp_dir()`, which just wraps it) resolves to `/tmp`, which
/// doesn't exist in the app's sandboxed filesystem view: `create_dir_all`
/// fails outright, so this whole decrypt-and-open flow was silently broken
/// there. `app_cache_dir()` is the one path Android actually grants this app
/// write access to *and* the one already declared as shareable in
/// `file_paths.xml`'s `cache-path` entry -- required for the OS opener to
/// receive a `content://` URI for it at all (a raw path outside that list
/// is not grantable via `FileProvider`).
fn scratch_open_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    #[cfg(mobile)]
    {
        Ok(app.path().app_cache_dir().str_err()?.join(format!("open-{}", std::process::id())))
    }
    #[cfg(not(mobile))]
    {
        let _ = app;
        Ok(std::env::temp_dir().join(format!("vaultexplorer-open-{}", std::process::id())))
    }
}

/// Decrypt the `.vlt` file at `path` with `password` into a scratch temp
/// file (named after the original, extension restored) and return its
/// path so the caller can hand it to the OS opener. The plaintext never
/// touches the original directory.
#[tauri::command]
fn fs_decrypt_file(app: tauri::AppHandle, path: String, password: String) -> Result<String, String> {
    let plaintext = vaultcore::decrypt_file_with_password(&path, password.as_bytes())
        .str_err()?;
    let stem = Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let dir = scratch_open_dir(&app)?;
    std::fs::create_dir_all(&dir).str_err()?;
    let dest = dir.join(stem);
    std::fs::write(&dest, plaintext).str_err()?;
    Ok(dest.to_string_lossy().to_string())
}

/// Same idea as `fs_encrypt_file`, but for a file *inside* an unlocked
/// vault: the vault decrypts it as usual, we wrap that plaintext in an
/// extra standalone password layer, and write the result back as the
/// vault entry's new (`.vlt`-suffixed) content -- a legitimate double
/// encryption, not a bug. Returns the new rel path.
#[tauri::command]
fn encrypt_file_in_vault(
    state: State<AppState>,
    rel_path: String,
    password: String,
) -> Result<String, String> {
    with_vault(&state, |v| {
        let plaintext = v.decrypt_file(&rel_path)?;
        let wrapped = vaultcore::encrypt_file_with_password(&plaintext, password.as_bytes())?;
        let dest_rel = format!("{rel_path}.{ENCRYPTED_EXT}");
        v.write_file(&dest_rel, &wrapped)?;
        v.remove_file(&rel_path)?;
        Ok(dest_rel)
    })
}

/// Decrypt a `.vlt`-inside-a-vault entry into a scratch temp file for the
/// OS opener, mirroring `fs_decrypt_file`.
#[tauri::command]
fn decrypt_file_in_vault(
    app: tauri::AppHandle,
    state: State<AppState>,
    rel_path: String,
    password: String,
) -> Result<String, String> {
    let dir = scratch_open_dir(&app)?;
    with_vault(&state, |v| {
        let wrapped = v.decrypt_file(&rel_path)?;
        let plaintext = vaultcore::decrypt_bytes_with_password(&wrapped, password.as_bytes())?;
        let stem = Path::new(&rel_path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join(stem);
        std::fs::write(&dest, plaintext)?;
        Ok(dest.to_string_lossy().to_string())
    })
}

/// Mobile-only alternative to `open_path`: decrypts a *regular* vault entry
/// (not `.vlt`-wrapped -- see `decrypt_file_in_vault` for that layer) into
/// the same scratch dir, keeping its original filename so the receiving
/// app can infer its type from the extension. No FUSE/DocumentsProvider
/// needed: the plaintext exists on disk only transiently, exactly like the
/// `.vlt` standalone-decrypt flow this mirrors -- registered unconditionally
/// (harmless on desktop, just unused there since `open_path` already covers
/// it via FUSE without a throwaway copy).
#[tauri::command]
fn vault_decrypt_to_temp(app: tauri::AppHandle, state: State<AppState>, rel_path: String) -> Result<String, String> {
    let dir = scratch_open_dir(&app)?;
    with_vault(&state, |v| v.decrypt_file(&rel_path)).and_then(|plaintext| {
        let name = Path::new(&rel_path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        std::fs::create_dir_all(&dir).str_err()?;
        let dest = dir.join(name);
        std::fs::write(&dest, plaintext).str_err()?;
        Ok(dest.to_string_lossy().to_string())
    })
}

const TAGS_FILENAME: &str = ".ve-tags.json";

fn read_tags(dir: &Path) -> std::collections::HashMap<String, String> {
    std::fs::read_to_string(dir.join(TAGS_FILENAME))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Color labels (Finder-style tags) for entries in `dir`, keyed by
/// filename. Stored in a plain JSON sidecar next to the files -- there's
/// no cross-platform xattr story here, and a sidecar is trivially portable.
#[tauri::command]
fn fs_get_tags(dir: String) -> std::collections::HashMap<String, String> {
    read_tags(Path::new(&dir))
}

/// Set (or, if `color` is null, clear) the color tag for `name` inside `dir`.
#[tauri::command]
fn fs_set_tag(dir: String, name: String, color: Option<String>) -> Result<(), String> {
    let dir = Path::new(&dir);
    let mut tags = read_tags(dir);
    match color {
        Some(c) => {
            tags.insert(name, c);
        }
        None => {
            tags.remove(&name);
        }
    }
    let path = dir.join(TAGS_FILENAME);
    if tags.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let json = serde_json::to_string(&tags).str_err()?;
    std::fs::write(&path, json).str_err()
}

// ---- Sensitive files (per-file / per-folder re-auth gate) ----

#[tauri::command]
fn vault_set_sensitive(state: State<AppState>, rel_path: String, sensitive: bool) -> Result<(), String> {
    with_vault(&state, |v| v.set_sensitive(&rel_path, sensitive))
}

#[tauri::command]
fn vault_is_sensitive(state: State<AppState>, rel_path: String) -> Result<bool, String> {
    with_vault(&state, |v| Ok(v.is_sensitive(&rel_path)))
}

#[tauri::command]
fn vault_list_sensitive(state: State<AppState>) -> Result<Vec<String>, String> {
    with_vault(&state, |v| v.list_sensitive())
}

#[tauri::command]
fn vault_unlock_sensitive(
    state: State<AppState>,
    password: String,
    timeout_secs: Option<u64>,
) -> Result<(), String> {
    with_vault(&state, |v| {
        v.unlock_sensitive(
            password.as_bytes(),
            timeout_secs.map(std::time::Duration::from_secs),
        )
    })
}

#[tauri::command]
fn vault_sensitive_unlocked(state: State<AppState>) -> Result<bool, String> {
    with_vault(&state, |v| Ok(v.sensitive_unlocked()))
}

#[tauri::command]
fn vault_lock_sensitive(state: State<AppState>) -> Result<(), String> {
    with_vault(&state, |v| {
        v.lock_sensitive();
        Ok(())
    })
}

/// Change a vault's password (O(1) re-wrap of the master key; no file is
/// re-encrypted). Verifies `old_password` first. The in-memory unlocked
/// session stays valid since the master key is unchanged.
#[tauri::command]
fn change_vault_password(root: String, old_password: String, new_password: String) -> Result<(), String> {
    vaultcore::change_password(&root, old_password.as_bytes(), new_password.as_bytes()).str_err()
}

/// Ensure the vault is mounted via FUSE, then return the *absolute*
/// mountpoint path for `rel_path` -- the frontend passes that straight to
/// the opener plugin, so the OS launches whatever program it would
/// normally use, pointed at plaintext that only ever exists on this
/// virtual filesystem, never written unencrypted to real disk. No mobile
/// equivalent: handing a vault file to some *other* app via a real path
/// needs FUSE (desktop) or a whole DocumentsProvider integration
/// (Android) this doesn't attempt -- every other vault operation (view,
/// edit, thumbnail) already stays in-process and doesn't need this at all.
#[cfg(desktop)]
#[tauri::command]
fn open_path(state: State<AppState>, rel_path: String) -> Result<String, String> {
    let mountpoint = ensure_mounted(&state)?;
    Ok(mountpoint.join(&rel_path).to_string_lossy().to_string())
}

/// Where FUSE mountpoints go. `$XDG_RUNTIME_DIR` first, then `/dev/shm`: both
/// are tmpfs, i.e. RAM-backed and never written to a disk, and the runtime dir
/// is additionally per-user 0700 and wiped at logout.
///
/// This matters because of what a mountpoint is: while the mount is up, every
/// byte read under it is served from memory by `fuse_mount.rs` and the only
/// thing on disk is the vault's ciphertext. But the *directory* is a real
/// directory, and anything that writes to that path while the mount ISN'T up
/// (a write racing the unmount, a mount that failed to start) writes
/// plaintext straight to whatever filesystem it lives on. `/tmp` is not a
/// tmpfs on every system -- on this one it's the root filesystem -- so the old
/// `std::env::temp_dir()` location meant that accident put plaintext on a
/// disk. On tmpfs the same accident stays in RAM and dies with the machine.
///
/// Falls back to the temp dir if neither tmpfs path is usable, since a
/// working mount beats no mount at all -- `MountHandle::drop` shreds leftovers
/// either way.
#[cfg(desktop)]
fn mount_base_dir() -> PathBuf {
    let candidates = [
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        Some(PathBuf::from("/dev/shm")),
    ];
    for base in candidates.into_iter().flatten() {
        if !base.is_dir() {
            continue;
        }
        let dir = base.join("vaultexplorer");
        if std::fs::create_dir_all(&dir).is_ok() {
            // 0700 rather than whatever the umask gives (0775 here): /dev/shm
            // is world-readable, so on that fallback the umask default would
            // leave the directory holding mountpoints listable by other users.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
            }
            return dir;
        }
    }
    std::env::temp_dir()
}

/// Removes `vaultexplorer-mnt-<pid>-<hash>` directories left behind by earlier
/// runs. `MountHandle::drop` handles the normal case, but a `kill -9` (or any
/// build predating that destructor) can't run one, so old ones accumulate.
/// Scans both the tmpfs base and the legacy temp-dir location. Runs once at
/// startup.
#[cfg(desktop)]
fn sweep_stale_mountpoints() {
    let mut bases = vec![mount_base_dir()];
    let temp = std::env::temp_dir();
    if !bases.contains(&temp) {
        bases.push(temp);
    }
    for base in bases {
        let Ok(read_dir) = std::fs::read_dir(&base) else { continue };
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let Some(rest) = name.to_str().and_then(|n| n.strip_prefix("vaultexplorer-mnt-")) else {
                continue;
            };
            let Some(pid) = rest.split('-').next().and_then(|p| p.parse::<u32>().ok()) else {
                continue;
            };
            // Another *live* instance owns its own mountpoints -- two copies
            // of this app can run at once (including the portal-activated
            // one), and one must never pull the rug out from under the other.
            if std::path::Path::new(&format!("/proc/{pid}")).exists() {
                continue;
            }
            let path = entry.path();
            // A dead process can leave its mount still attached. Detaching it
            // is the whole point (that mount is a readable decrypted view
            // nobody is watching any more), but only ever lazily, and only
            // for a pid that's gone.
            if is_mount_point(&path) {
                let _ = std::process::Command::new("fusermount")
                    .args(["-u", "-z", &path.to_string_lossy()])
                    .status();
                if is_mount_point(&path) {
                    continue; // still live -- leave it strictly alone
                }
            }
            shred::purge_dir_securely(&path);
        }
    }
}

#[cfg(desktop)]
fn short_hash(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Ensure the *active* vault (see `AppState::active`) is mounted via FUSE,
/// then return its absolute mountpoint. Each vault session gets its own
/// mountpoint (named from a hash of its root, so nested vaults mounted
/// concurrently don't collide) and its own FUSE session, independent of
/// any other currently-unlocked vault.
#[cfg(desktop)]
fn ensure_mounted(state: &State<AppState>) -> Result<PathBuf, String> {
    let root = state.active.lock_safe().clone().ok_or("no vault unlocked")?;
    let map = state.vaults.lock_safe();
    let session = map.get(&root).ok_or("no vault unlocked")?;

    let mut mount_guard = session.mount.lock_safe();
    if let Some(m) = mount_guard.as_ref() {
        return Ok(m.mountpoint.clone());
    }

    let mountpoint = mount_base_dir().join(format!(
        "vaultexplorer-mnt-{}-{}",
        std::process::id(),
        short_hash(&root)
    ));
    std::fs::create_dir_all(&mountpoint).str_err()?;
    // 0700: the decrypted view is this user's business only. FUSE mounts
    // default to owner-only access anyway, but the *directory* under it (what
    // a stray write would land in) inherits the umask otherwise.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&mountpoint, std::fs::Permissions::from_mode(0o700));
    }
    let fuse_session =
        vaultcore::fuse_mount::spawn(session.vault.clone(), &mountpoint).str_err()?;
    let mountpoint_clone = mountpoint.clone();
    *mount_guard = Some(MountHandle {
        mountpoint,
        session: Some(fuse_session),
    });
    Ok(mountpoint_clone)
}

fn path_to_string(p: PathBuf) -> String {
    p.to_string_lossy().replace('\\', "/")
}

// ---- real (plaintext) filesystem browsing ----

/// The starting point for "browse the real filesystem" (the "All My
/// Files" favorite, vault creation's default location, etc.). On
/// desktop that's just `$HOME`, same as always. Android has no
/// equivalent -- apps are sandboxed to their own storage rather than
/// having a personal home directory other apps can also see -- so this
/// resolves to this app's own external-files directory instead (a real
/// `std::fs`-usable absolute path Android grants without any extra
/// permission, just private to this app rather than shared).
/// Plain, unconditional `$HOME` -- used internally for XDG-style cache/
/// config paths (thumbnail cache, archive-mount scratch dir, and a few
/// fully desktop-only modules' own paths) that stay desktop-shaped
/// regardless of platform. Distinct from the `browse_root_dir` *command*
/// below, which is what "browse the real filesystem" actually starts
/// from and needs to be mobile-aware. On Android this resolves to an
/// unwritable path (`HOME` is unset there) -- thumbnail caching/archive-
/// mount-as-folder degrade gracefully rather than working fully there
/// for now; see the mobile-scoping notes elsewhere in this file.
pub(crate) fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}

/// Lets the frontend adapt its own UI (hide the custom desktop titlebar/
/// resize handles, skip menu entries for desktop-only integrations like
/// git/cloud/P2P sync, freeze, machine tools, terminal) without needing a
/// separate mobile build of the JS bundle -- same binary, one runtime
/// check.
#[tauri::command]
fn is_mobile_platform() -> bool {
    cfg!(mobile)
}

#[tauri::command]
fn browse_root_dir(app: tauri::AppHandle) -> String {
    #[cfg(mobile)]
    {
        if let Ok(dir) = app.path().app_data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            // The default favorites point at these -- create them so
            // they're real, listable folders instead of 404ing on first
            // launch (there's no ~/Documents-equivalent auto-created in
            // an Android app's sandboxed data dir).
            for sub in ["Documents", "Pictures", "Downloads"] {
                let _ = std::fs::create_dir_all(dir.join(sub));
            }
            return path_to_string(dir);
        }
    }
    #[cfg(desktop)]
    {
        let _ = &app;
        return std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    }
    #[allow(unreachable_code)]
    "/".to_string()
}

/// Whether `path` is itself a vault root, for callers that only have the
/// path in hand (e.g. opening a Favorites entry) rather than a `fs_list`
/// row with `is_vault` already attached.
#[tauri::command]
fn fs_is_vault(path: String) -> bool {
    Path::new(&path).join(".vault.meta").exists()
}

/// List a real OS directory. Hidden entries (dotfiles) are skipped. A
/// subdirectory that contains a `.vault.meta` is flagged `is_vault` so the
/// UI can render it as a lockable vault-folder.
#[tauri::command]
fn fs_list(path: String, show_hidden: bool) -> Result<Vec<FsEntryDto>, String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&path).str_err()? {
        let entry = entry.str_err()?;
        let name = entry.file_name().to_string_lossy().to_string();
        let is_hidden = name.starts_with('.');
        if is_hidden && !show_hidden {
            continue;
        }
        let p = entry.path();
        let is_dir = p.is_dir();
        let is_vault = is_dir && p.join(".vault.meta").exists();
        let metadata = entry.metadata().str_err()?;
        let size = if is_dir { 0 } else { metadata.len() };
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let created = metadata
            .created()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        out.push(FsEntryDto {
            name,
            is_dir,
            is_vault,
            is_hidden,
            size,
            mtime,
            created,
        });
    }
    Ok(out)
}

/// Toggle the write bit(s) on a real file/folder -- the "Read-only"
/// checkbox in Get Info.
#[tauri::command]
fn fs_set_readonly(path: String, readonly: bool) -> Result<(), String> {
    let metadata = std::fs::metadata(&path).str_err()?;
    let mut perms = metadata.permissions();
    perms.set_readonly(readonly);
    std::fs::set_permissions(&path, perms).str_err()
}

#[tauri::command]
fn fs_is_readonly(path: String) -> Result<bool, String> {
    let metadata = std::fs::metadata(&path).str_err()?;
    Ok(metadata.permissions().readonly())
}

/// Case-insensitive filename search under `root`, capped at 500 results so
/// a search on a huge tree stays snappy.
#[tauri::command]
fn fs_search(root: String, query: String) -> Result<Vec<String>, String> {
    let query = query.to_lowercase();
    let mut out = Vec::new();
    let mut budget = 500usize;
    dirwalk::fs_search_recursive(Path::new(&root), &query, &mut out, &mut budget);
    Ok(out)
}

#[tauri::command]
fn fs_mkdir(path: String) -> Result<(), String> {
    std::fs::create_dir(&path).str_err()
}

#[tauri::command]
fn fs_new_file(path: String) -> Result<(), String> {
    if Path::new(&path).exists() {
        return Err("a file with that name already exists".to_string());
    }
    std::fs::write(&path, b"").str_err()
}

/// Backing the list-with-preview view's live text/markdown editor pane --
/// plain UTF-8 read/write, no encoding detection. Good enough for the
/// .txt/.md files that pane targets; anything else fails cleanly rather
/// than mangling bytes.
#[tauri::command]
fn fs_read_text(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).str_err()
}

#[tauri::command]
fn fs_write_text(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).str_err()
}

/// Raw-bytes counterpart to `fs_write_text`, for callers (e.g. the image
/// editor's canvas export) that produce binary content rather than UTF-8
/// text.
#[tauri::command]
fn fs_write_bytes(path: String, bytes: Vec<u8>) -> Result<(), String> {
    std::fs::write(&path, bytes).str_err()
}

/// Saves an image pasted into the markdown editor as a real sibling file
/// next to the note (real fs only -- a vault file has no "sibling path"
/// to write plaintext bytes into without breaking the vault's own
/// invariant). Returns just the filename, since that's what the markdown
/// source stores as a relative reference.
#[tauri::command]
fn fs_save_pasted_image(dir: String, bytes: Vec<u8>) -> Result<String, String> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = format!("pasted-image-{millis}.png");
    std::fs::write(Path::new(&dir).join(&name), &bytes).str_err()?;
    Ok(name)
}

#[tauri::command]
fn vault_read_text(state: State<AppState>, rel_path: String) -> Result<String, String> {
    let bytes = with_vault(&state, |v| v.decrypt_file(&rel_path))?;
    String::from_utf8(bytes).str_err()
}

#[tauri::command]
fn vault_write_text(state: State<AppState>, rel_path: String, content: String) -> Result<(), String> {
    with_vault(&state, |v| v.write_file(&rel_path, content.as_bytes()))
}

/// Raw-bytes counterpart to `vault_write_text`, for binary content (e.g.
/// an edited image) written back into the vault.
#[tauri::command]
fn vault_write_bytes(state: State<AppState>, rel_path: String, bytes: Vec<u8>) -> Result<(), String> {
    with_vault(&state, |v| v.write_file(&rel_path, &bytes))
}

#[tauri::command]
async fn fs_delete(path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = Path::new(&path);
        if p.is_dir() {
            std::fs::remove_dir_all(p).str_err()
        } else {
            std::fs::remove_file(p).str_err()
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Rename or move a real file/dir (dest is the full destination path).
#[tauri::command]
async fn fs_rename(src: String, dest: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        // rename(2) with an existing dest either silently clobbers a file
        // or fails with the kernel's raw "Directory not empty (os error
        // 39)" for a non-empty directory -- neither is what a user
        // renaming in the UI should see. Refuse up front with a clear
        // message instead. (`src != dest` keeps a same-path no-op safe.)
        if src != dest && Path::new(&dest).exists() {
            let name = Path::new(&dest)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| dest.clone());
            return Err(format!("\"{name}\" already exists here"));
        }
        std::fs::rename(&src, &dest).str_err()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn fs_copy(
    src: String,
    dest: String,
    channel: Channel<ProgressEvent>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let src_path = Path::new(&src);
        let reporter = ProgressReporter::new(channel, dirwalk::dir_total_size(src_path).max(1));
        let done = std::cell::Cell::new(0u64);
        copy_recursive(src_path, Path::new(&dest), &done, &reporter).str_err()
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Create a symlink at `dest` pointing at `target` -- the closest real-fs
/// equivalent of Finder's "Make Alias".
#[tauri::command]
fn fs_create_shortcut(target: String, dest: String) -> Result<(), String> {
    std::os::unix::fs::symlink(&target, &dest).str_err()
}

/// Frozen-folder remounts plus every configured sync watch loop --
/// everything a real user launch resumes in the background. Split out of
/// `setup()` so (a) it runs off the main thread (freeze remounts do
/// blocking FUSE mounts, which used to stall the first paint), and (b) a
/// formerly portal-activated primary instance can start it lazily when a
/// real launch gets forwarded to it by the single-instance plugin. Runs at
/// most once per process -- the loops themselves also no-op when already
/// active, this guard just avoids re-walking the freeze remounts.
#[cfg(desktop)]
fn start_background_loops(handle: tauri::AppHandle) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        // Re-mount every frozen folder fresh, from a discarded shadow --
        // this *is* the "back to how it was after a restart" guarantee
        // (see freeze.rs): only holds while VaultExplorer's mount is
        // alive, so each launch re-establishes it from scratch rather
        // than resuming whatever was left dangling.
        let freeze_state = handle.state::<FreezeState>();
        for meta in freeze::list_frozen() {
            if let Err(e) = freeze::discard_shadow(&meta.original_path) {
                eprintln!("freeze: failed to discard shadow for {}: {e}", meta.original_path);
                continue;
            }
            if std::fs::create_dir_all(&meta.original_path).is_err() {
                continue;
            }
            match freeze::spawn(&meta.original_path, Path::new(&meta.original_path)) {
                Ok(session) => {
                    freeze_state.mounts.lock_safe().insert(meta.original_path, session);
                }
                Err(e) => eprintln!("freeze: failed to remount {}: {e}", meta.original_path),
            }
        }
        // ...and every sync pair's event-driven watch loop -- without
        // this, a pair only actually kept syncing until the app was next
        // closed, since nothing else ever calls the start_loops again.
        let local_sync_state = handle.state::<local_sync::LocalSyncState>();
        for pair in local_sync::list_pairs_pruning_missing() {
            local_sync::start_loop(&local_sync_state, pair.folder_a, pair.folder_b);
        }
        let drive_sync_state = handle.state::<sync::DriveSyncState>();
        for pair in sync::list_pairs() {
            sync::start_loop(&drive_sync_state, pair);
        }
        let git_sync_state = handle.state::<git_sync::GitSyncState>();
        for pair in git_sync::list_pairs() {
            git_sync::start_loop(&git_sync_state, pair.local_path);
        }
    });
}

/// A second launch of the binary, forwarded here by the single-instance
/// plugin: open another Explorer window in this already-running process
/// (near-instant) instead of letting a whole second app boot (seconds).
#[cfg(desktop)]
fn open_extra_explorer_window(app: &tauri::AppHandle) {
    // If this primary instance was a portal-activated picker server, its
    // main window exists but was never shown (and no sync loops run) -- a
    // real launch forwarded here means the user wants the full app now.
    if let Some(main) = app.get_webview_window("main") {
        if !main.is_visible().unwrap_or(true) {
            let _ = main.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 0)));
            let _ = main.show();
            let _ = main.set_focus();
            start_background_loops(app.clone());
            return;
        }
    }
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(2);
    let label = format!("main-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    // Mirrors the main window's tauri.conf.json declaration (minus
    // visible:false -- there's no pre-show work to hide here).
    match tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::App("index.html".into()))
        .title("VaultExplorer")
        .inner_size(1040.0, 680.0)
        .min_inner_size(720.0, 480.0)
        .decorations(false)
        .transparent(true)
        .build()
    {
        Ok(w) => {
            // Same WebKitGTK opaque-background workaround as the main
            // window (see setup()).
            let _ = w.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 0)));
            let _ = w.set_focus();
        }
        Err(e) => eprintln!("single-instance: failed to open a new window: {e}"),
    }
}

/// The standalone "just the video, nothing else" player window Internet's
/// video results open into -- same `index.html?<mode>=...` query-string
/// routing PickerView already uses (see portal.rs's `run_picker`), just a
/// different mode. `items` is a pre-serialized JSON array (built on the JS
/// side, since it's already holding the search results) rather than a Rust
/// struct here -- this command's only job is getting that string safely
/// into a URL and opening the window, not understanding its shape.
#[cfg(desktop)]
#[tauri::command]
fn open_player_window(app: tauri::AppHandle, kind: String, items: String, index: usize) -> Result<(), String> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let label = format!("player-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let url = format!(
        "index.html?player=1&kind={}&items={}&index={index}",
        portal::url_encode(&kind),
        portal::url_encode(&items)
    );
    let w = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(url.into()))
        .title("Player")
        .inner_size(960.0, 620.0)
        .min_inner_size(360.0, 240.0)
        .decorations(false)
        .transparent(true)
        .build()
        .str_err()?;
    let _ = w.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 255)));
    let _ = w.set_focus();
    Ok(())
}

/// The local-media counterpart of `open_player_window`: photos, video and
/// audio files open in their own window rather than as an overlay stacked
/// on top of the file grid, so the grid stays usable behind them and each
/// piece of media gets real window chrome (traffic lights, fullscreen,
/// its own place in the window switcher) -- the way a media file behaves
/// everywhere else on the desktop.
/// The real folder behind the Internet section.
///
/// Internet's "Videos"/"Images"/"Books" are live searches with no path
/// behind them, but a saved search or a kept `.youtube.url` link is an
/// ordinary file and wants an ordinary place to live -- and folders to
/// organise into. That place is here, created on first use, so the section
/// behaves like the rest of the file manager instead of being a dead end.
#[tauri::command]
fn internet_root() -> Result<String, String> {
    let home = std::env::var("HOME").str_err()?;
    let dir = std::path::Path::new(&home).join(".local/share/vaultexplorer/internet");
    std::fs::create_dir_all(&dir).str_err()?;
    Ok(dir.to_string_lossy().to_string())
}

#[cfg(desktop)]
#[tauri::command]
fn open_media_window(app: tauri::AppHandle, items: String, index: usize) -> Result<(), String> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let label = format!("media-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let url = format!("index.html?media=1&items={}&index={index}", portal::url_encode(&items));
    let w = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(url.into()))
        .title("Media")
        .inner_size(1000.0, 680.0)
        .min_inner_size(420.0, 300.0)
        .decorations(false)
        .transparent(true)
        .build()
        .str_err()?;
    let _ = w.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 255)));
    let _ = w.set_focus();
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // Must be the first plugin registered (its own documented requirement)
    // so a second launch is intercepted before anything else initializes.
    // A `--portal-activated` D-Bus activation is never forwarded into a
    // window: if one races in while this instance runs, the portal service
    // here already owns the bus name and will serve the request.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
        if argv.iter().any(|a| a == "--portal-activated") {
            return;
        }
        // A forwarded launch carrying a directory (OBS "Show Recordings" →
        // xdg-open → this binary as the inode/directory handler): open the
        // new window right there -- pending-reveal is drained by whichever
        // window mounts next, which is the one created just below.
        if let Some(dir) = argv.iter().skip(1).find_map(|a| filemanager1::cli_dir_arg(a)) {
            filemanager1::set_pending_reveal(dir, None);
        }
        open_extra_explorer_window(app);
    }));
    let builder = builder
        .manage(AppState::default())
        .manage(ops::OpRegistry::default())
        .manage(git_sync::GitSyncState::default())
        .manage(archive_browse::ArchiveMountState::default());
    // Desktop-only managed state -- see the mobile-scoping note by the
    // `mod` declarations at the top of this file.
    #[cfg(desktop)]
    let builder = builder
        .manage(portal::PortalState::default())
        .manage(filemanager1::FileManagerState::default())
        .manage(FreezeState::default())
        .manage(local_sync::LocalSyncState::default())
        .manage(sync::DriveSyncState::default())
        .manage(fs_watch::FsWatchState::default())
        .plugin(tauri_plugin_drag::init());

    builder
        .setup(|app| {
            // `--portal-activated` is our own marker (see portal.rs): xdg-desktop-
            // portal D-Bus-activates this binary purely to service one Save/Open
            // dialog for some *other* app. Such a launch must stay a lean picker
            // server -- it must NOT show the main window, re-run pkexec-gated
            // registration self-heal, or (crucially) start the background sync
            // daemons below. A lingering activated instance running the Drive
            // watch loop alongside the user's real instance meant two concurrent
            // `rclone bisync` runs on the same remote -> the "prior lock file
            // found" bisync failure. Hoisted here so every startup side-effect
            // can gate on it, on both desktop and mobile (always false on mobile,
            // which has no portal).
            let portal_activated = std::env::args().any(|a| a == "--portal-activated");
            // Cold start with a directory argument (xdg-open/gio launching
            // this binary as the inode/directory handler, or a plain
            // `vaultexplorer ~/some/dir`): queue it so the main window
            // opens there once the frontend mounts and drains the slot.
            #[cfg(desktop)]
            if let Some(dir) = std::env::args().skip(1).find_map(|a| filemanager1::cli_dir_arg(&a)) {
                filemanager1::set_pending_reveal(dir, None);
            }
            // Best-effort, idempotent housekeeping that nothing in this
            // launch depends on -- moved off the main thread because it
            // used to run (blocking) before the first paint:
            // - deep-link registration: registers this app as the handler
            //   for `vaultexplorer://` links every launch (harmless if
            //   already registered; covers e.g. an unregistered AppImage).
            //   The plugin shells out to `update-desktop-database` +
            //   `xdg-mime` unconditionally, often the single most
            //   expensive startup item. Mobile registers the scheme via
            //   the Android manifest/iOS Info.plist at build time instead.
            // - stale-mountpoint sweep: only touches `vaultexplorer-mnt-
            //   <pid>-*` dirs of dead pids, so this process's own mounts
            //   (all created later, under its own live pid) can't race it.
            // - portal/filemanager1 registration self-heal: only rewrites
            //   on drift, and its worst case (`pkexec` prompting for the
            //   admin password after a rebuild changed the binary path)
            //   now pops over a usable app instead of blocking the launch.
            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    if let Err(e) = tauri_plugin_deep_link::DeepLinkExt::deep_link(&handle).register_all() {
                        eprintln!("deep-link: failed to register vaultexplorer:// scheme: {e}");
                    }
                    sweep_stale_mountpoints();
                    if portal::is_enabled() && !portal_activated {
                        if let Ok(exe) = std::env::current_exe() {
                            let exe = exe.to_string_lossy();
                            if let Err(e) = portal::write_registration(&exe) {
                                eprintln!("portal: failed to refresh service registration: {e}");
                            }
                            if let Err(e) = filemanager1::write_registration(&exe) {
                                eprintln!("filemanager1: failed to refresh service registration: {e}");
                            }
                        }
                    }
                });
            }
            #[cfg(desktop)]
            {
                // Resume the system-file-picker service across restarts if
                // the user had previously enabled it (see portal.rs) --
                // registering the files persists, so the service should too.
                // A D-Bus-activated launch (the process xdg-desktop-portal
                // spins up for *every* Save/Open dialog in *any* app) carries
                // our own `--portal-activated` marker. Such a launch must
                // never re-run the registration self-heal below: that path
                // reaches `install_system_portal_file`, whose `pkexec` copy
                // into /usr/share pops a graphical root-password prompt --
                // which is exactly the "save-as asks for the admin password
                // every time" bug. Registration is a user-initiated,
                // real-launch concern; the activated process only needs to
                // serve the request (start_service, below). `portal_activated`
                // is computed once at the top of this setup closure.
                if portal::is_enabled() {
                    // (The service-file registration self-heal that used to
                    // sit here runs in the housekeeping thread above -- the
                    // D-Bus name claims below don't read those files, only
                    // future D-Bus *activations* do.)
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app_handle.state::<portal::PortalState>();
                        let pending = state.pending.clone();
                        match portal::start_service(app_handle.clone(), pending).await {
                            Ok(conn) => *state.connection.lock_safe() = Some(conn),
                            Err(e) => eprintln!("portal service failed to start: {e}"),
                        }
                    });
                    // Same "system integration" toggle also claims
                    // org.freedesktop.FileManager1 (see filemanager1.rs) --
                    // "Show in folder"-type actions from other apps
                    // (Chrome's Downloads panel, OBS's "Show Recordings")
                    // then open in VaultExplorer instead of Nautilus, for
                    // as long as this process is running.
                    let fm_app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let state = fm_app_handle.state::<filemanager1::FileManagerState>();
                        match filemanager1::start_service(fm_app_handle.clone()).await {
                            Ok(conn) => *state.connection.lock_safe() = Some(conn),
                            Err(e) => eprintln!("filemanager1: failed to start: {e}"),
                        }
                    });
                }
                // The main window starts invisible (see tauri.conf.json's
                // "visible": false on it) specifically so this decision can
                // be made cleanly here instead of racing a `.hide()` against
                // Tauri's own window creation -- Tauri's internal setup()
                // builds every config-declared window (which maps/shows it,
                // if visible:true) *before* this closure ever runs, so a
                // `.hide()` here was sometimes too late to preempt that
                // first paint. D-Bus activated us solely to service a
                // FileChooser request (see the `--portal-activated` marker
                // `portal.rs` writes into the service file's Exec line) --
                // the picker window portal.rs is about to open is the only
                // UI that launch should show, so only a normal user launch
                // shows the main Explorer window at all.
                if !portal_activated {
                    if let Some(main) = app.get_webview_window("main") {
                        // WebKitGTK paints an opaque webview background by
                        // default even when the tao window is `transparent:
                        // true` -- so the translucent chrome only flashed
                        // see-through during a resize repaint, then got
                        // painted back over. Explicitly clearing the
                        // background color (fully transparent) makes the
                        // liquid-glass translucency persist.
                        let _ = main.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 0)));
                        let _ = main.show();
                    }
                }
                // A `--portal-activated` launch is a transient picker server for
                // another app's dialog; it must not resurrect the user's frozen
                // mounts or start any background sync watcher (see the hoisted
                // `portal_activated` comment -- double Drive loops = double
                // `rclone bisync` = the "prior lock file found" failure). Those
                // are the real launch's job -- started off the main thread so
                // the blocking freeze remounts never stall the first paint.
                if !portal_activated {
                    start_background_loops(app.handle().clone());
                }
            }
            // NOTE: no mobile equivalent of the above -- `git_sync`'s loop
            // shells out to the `git` binary (see git_sync.rs), which
            // doesn't exist on Android/iOS. An earlier version of this
            // resumed it there anyway on the mistaken premise that "git is
            // cross-platform" (true of the Rust code, not of a `git`
            // binary to exec); it started a loop that could only ever fail
            // every tick. Desktop's copy lives inside start_background_loops.
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![
            ops::cancel_operation,
            vault_exists,
            create_vault,
            convert_folder_to_vault,
            unlock_vault,
            verify_vault_password,
            lock_vault,
            set_active_vault,
            set_vault_auto_unlock,
            clear_vault_auto_unlock,
            auto_unlock_vaults,
            list_dir,
            vault_list_dir_at,
            search_vault,
            move_entry,
            copy_entry,
            vault_to_vault_copy,
            vault_to_vault_move,
            delete_file,
            delete_dir,
            make_dir,
            new_file,
            import_file,
            export_file,
            archive::compress_entries,
            archive::decompress_entry,
            dir_size,
            fs_dir_size,
            clipboard::fs_copy_image_to_clipboard,
            clipboard::vault_copy_image_to_clipboard,
            thumbnail::fs_thumbnail,
            thumbnail::vault_thumbnail,
            #[cfg(desktop)]
            app_icon::app_icon_for_ext,
            #[cfg(desktop)]
            app_icon::list_apps_for_path,
            #[cfg(desktop)]
            filemanager1::take_pending_reveal,
            #[cfg(desktop)]
            app_icon::list_all_apps,
            #[cfg(desktop)]
            app_icon::app_icons,
            #[cfg(desktop)]
            app_icon::open_with,
            #[cfg(desktop)]
            autostart::autostart_enabled,
            #[cfg(desktop)]
            autostart::set_autostart,
            metadata::fs_clear_metadata,
            metadata::vault_clear_metadata,
            info::fs_file_info,
            info::vault_file_info,
            convert::convert_ffmpeg_available,
            convert::fs_convert_image,
            convert::vault_convert_image,
            convert::fs_convert_media,
            montage::fs_build_montage,
            convert::convert_libreoffice_available,
            convert::fs_convert_office,
            convert::fs_pdf_to_images,
            convert::fs_image_to_pdf,
            convert::fs_resize_images,
            convert::vault_resize_images,
            #[cfg(desktop)]
            transcribe::transcribe_model_downloaded,
            #[cfg(desktop)]
            transcribe::transcribe_download_model,
            #[cfg(desktop)]
            transcribe::transcribe_run,
            vault_set_sensitive,
            vault_is_sensitive,
            vault_list_sensitive,
            vault_unlock_sensitive,
            vault_sensitive_unlocked,
            vault_lock_sensitive,
            change_vault_password,
            #[cfg(desktop)]
            open_path,
            webfind::search_youtube,
            webfind::search_images,
            webfind::search_books,
            webfind::list_video_providers,
            webfind::search_provider_videos,
            webfind::resolve_provider_playable,
            webfind::list_animeflv_episodes,
            webfind::download_web_result,
            mediaserver::media_url,
            musicorg::organize_music,
            ytstreams::youtube_streams,
            ytstreams::download_stream,
            cast::cast_discover,
            cast::cast_play_youtube,
            #[cfg(target_os = "android")]
            android::android_mux_video,
            internet_root,
            #[cfg(desktop)]
            ytdl::download_video,
            #[cfg(desktop)]
            ytdl::resolve_stream_url,
            #[cfg(desktop)]
            open_player_window,
            #[cfg(desktop)]
            open_media_window,
            ytembed::youtube_embed_url,
            #[cfg(desktop)]
            terminal::open_terminal,
            #[cfg(desktop)]
            terminal::run_shell_script,
            #[cfg(desktop)]
            terminal::open_in_editor,
            is_mobile_platform,
            browse_root_dir,
            #[cfg(target_os = "android")]
            android::android_storage_access_granted,
            #[cfg(target_os = "android")]
            android::android_request_storage_access,
            #[cfg(target_os = "android")]
            android::android_pin_folder_shortcut,
            #[cfg(target_os = "android")]
            android::android_contacts_permission_granted,
            #[cfg(target_os = "android")]
            android::android_request_contacts_permission,
            #[cfg(target_os = "android")]
            android::android_export_contacts,
            #[cfg(target_os = "android")]
            android::android_import_contacts,
            #[cfg(target_os = "android")]
            android::android_open_path,
            #[cfg(target_os = "android")]
            android::android_share_path,
            #[cfg(target_os = "android")]
            android::android_download_and_install_apk,
            #[cfg(target_os = "android")]
            android::android_can_install_packages,
            #[cfg(target_os = "android")]
            android::android_request_install_packages_access,
            fs_list,
            fs_is_vault,
            fs_set_readonly,
            fs_is_readonly,
            fs_search,
            fs_mkdir,
            fs_new_file,
            fs_read_text,
            fs_write_text,
            fs_write_bytes,
            fs_save_pasted_image,
            vault_read_text,
            vault_write_text,
            vault_write_bytes,
            share::fs_share_file,
            share::vault_share_file,
            fs_delete,
            shred::fs_secure_delete,
            fs_trash,
            #[cfg(desktop)]
            fs_trash_many,
            largefiles::scan_large_files,
            #[cfg(desktop)]
            trash_dir,
            #[cfg(desktop)]
            empty_trash,
            #[cfg(desktop)]
            trash_restore_all,
            #[cfg(desktop)]
            trash_restore,
            #[cfg(desktop)]
            trash_purge,
            templates_dir,
            git::git_repo_root,
            git::git_status,
            git::git_pull,
            git::git_push,
            git::git_commit_all,
            git::git_stage,
            git::git_unstage,
            git::git_discard,
            #[cfg(desktop)]
            portal::portal_is_enabled,
            #[cfg(desktop)]
            portal::portal_enable,
            #[cfg(desktop)]
            portal::portal_disable,
            #[cfg(desktop)]
            portal::portal_resolve,
            #[cfg(desktop)]
            portal::portal_cancel,
            machine::machine_list_drives,
            machine::machine_summary,
            #[cfg(desktop)]
            machine::machine_format_drive,
            machine::machine_advanced_info,
            machine::machine_update_drivers,
            recovery::recovery_tool_available,
            recovery::recovery_list_disks,
            recovery::recovery_same_disk,
            recovery::recovery_run,
            #[cfg(desktop)]
            reorganize::claude_reorganize_folder,
            #[cfg(desktop)]
            freeze::freeze_folder,
            #[cfg(desktop)]
            freeze::list_frozen_folders,
            #[cfg(desktop)]
            freeze::unfreeze_folder,
            fs_rename,
            fs_copy,
            archive::fs_compress,
            archive::fs_compress_targz,
            archive::fs_decompress,
            archive_browse::archive_mount,
            archive_browse::archive_unmount,
            archive_browse::archive_mounts_left_behind,
            archive_browse::archive_all_mounts,
            fs_create_shortcut,
            fs_get_tags,
            fs_set_tag,
            fs_encrypt_file,
            fs_decrypt_file,
            encrypt_file_in_vault,
            decrypt_file_in_vault,
            vault_decrypt_to_temp,
            rclone::rclone_installed,
            rclone::rclone_providers,
            rclone::rclone_is_connected,
            rclone::rclone_connect,
            rclone::rclone_disconnect,
            #[cfg(desktop)]
            rclone::rclone_read_conf_raw,
            #[cfg(desktop)]
            rclone::rclone_merge_conf_raw,
            sync::drive_list_pairs,
            sync::drive_add_pair,
            sync::drive_remove_pair,
            sync::drive_sync_now,
            sync::drive_syncing_now,
            sync::drive_sync_activity,
            sync::drive_sync_last_error,
            verify::drive_verifying_now,
            verify::sync_verify_states,
            #[cfg(desktop)]
            sync::drive_sync_is_active,
            #[cfg(desktop)]
            fs_watch::fs_watch_set,
            git_sync::git_sync_list_pairs,
            git_sync::git_sync_is_active,
            git_sync::git_sync_syncing_now,
            git_sync::git_sync_last_error,
            git_sync::git_sync_add,
            git_sync::git_sync_remove,
            #[cfg(desktop)]
            local_sync::local_sync_available,
            #[cfg(desktop)]
            local_sync::local_sync_list_pairs,
            #[cfg(desktop)]
            local_sync::local_sync_is_active,
            #[cfg(desktop)]
            local_sync::local_sync_syncing_now,
            #[cfg(desktop)]
            local_sync::local_sync_add,
            #[cfg(desktop)]
            local_sync::local_sync_remove,
            #[cfg(desktop)]
            local_sync::local_sync_now,
            syncthing::syncthing_installed,
            syncthing::syncthing_syncing_now,
            syncthing::syncthing_qr_svg,
            syncthing::syncthing_my_device_id,
            syncthing::syncthing_list_devices,
            syncthing::syncthing_add_device,
            syncthing::syncthing_remove_device,
            syncthing::syncthing_list_folders,
            syncthing::syncthing_share_folder,
            syncthing::syncthing_remove_folder,
            syncthing::syncthing_pending_devices,
            syncthing::syncthing_pending_folders,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
