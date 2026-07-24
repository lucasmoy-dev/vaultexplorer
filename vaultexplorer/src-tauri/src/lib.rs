mod android;
mod archive;
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
mod machine;
mod metadata;
mod montage;
mod ops;
#[cfg(desktop)]
mod portal;
mod progress;
mod rclone;
mod recovery;
mod share;
mod shred;
mod sync;
mod syncthing;
mod terminal;
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
    _session: fuser::BackgroundSession,
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
/// removal (Ctrl+Delete in the UI) still goes through `fs_delete`. No
/// mobile equivalent: the `trash` crate has no Android/iOS backend at all
/// (there's no universal "OS trash can" concept to hook into there).
#[cfg(desktop)]
#[tauri::command]
fn fs_trash(path: String) -> Result<(), String> {
    trash::delete(&path).str_err()
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

/// Decrypt the `.vlt` file at `path` with `password` into a scratch temp
/// file (named after the original, extension restored) and return its
/// path so the caller can hand it to the OS opener. The plaintext never
/// touches the original directory.
#[tauri::command]
fn fs_decrypt_file(path: String, password: String) -> Result<String, String> {
    let plaintext = vaultcore::decrypt_file_with_password(&path, password.as_bytes())
        .str_err()?;
    let stem = Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let dir = std::env::temp_dir().join(format!("vaultexplorer-open-{}", std::process::id()));
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
    state: State<AppState>,
    rel_path: String,
    password: String,
) -> Result<String, String> {
    with_vault(&state, |v| {
        let wrapped = v.decrypt_file(&rel_path)?;
        let plaintext = vaultcore::decrypt_bytes_with_password(&wrapped, password.as_bytes())?;
        let stem = Path::new(&rel_path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        let dir = std::env::temp_dir().join(format!("vaultexplorer-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join(stem);
        std::fs::write(&dest, plaintext)?;
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

    let mountpoint = std::env::temp_dir().join(format!(
        "vaultexplorer-mnt-{}-{}",
        std::process::id(),
        short_hash(&root)
    ));
    std::fs::create_dir_all(&mountpoint).str_err()?;
    let fuse_session =
        vaultcore::fuse_mount::spawn(session.vault.clone(), &mountpoint).str_err()?;
    let mountpoint_clone = mountpoint.clone();
    *mount_guard = Some(MountHandle {
        mountpoint,
        _session: fuse_session,
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
    tauri::async_runtime::spawn_blocking(move || std::fs::rename(&src, &dest).str_err())
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
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
            // Best-effort, idempotent: registers this app as the handler
            // for `vaultexplorer://` links (used by the P2P sync "share a
            // link" flow) every launch, the same defensive-registration
            // pattern as the portal service above -- harmless if it's
            // already registered, and covers e.g. an AppImage that wasn't
            // registered by its launcher. Mobile doesn't need (or expose)
            // this at runtime at all -- the scheme is registered via the
            // Android manifest/iOS Info.plist at build time instead.
            #[cfg(desktop)]
            if let Err(e) = tauri_plugin_deep_link::DeepLinkExt::deep_link(app).register_all() {
                eprintln!("deep-link: failed to register vaultexplorer:// scheme: {e}");
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
                    // Re-write the D-Bus service file on a normal launch (not
                    // an activation), not just when the user first flips the
                    // toggle -- it hardcodes this binary's own Exec line
                    // (including the `--portal-activated` marker), so if that
                    // ever drifts (a rebuild, a reinstalled binary path) this
                    // self-heals it instead of quietly D-Bus-activating a
                    // stale Exec forever.
                    if !portal_activated {
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
                // are the real launch's job.
                if !portal_activated {
                // Re-mount every frozen folder fresh, from a discarded shadow
                // -- this *is* the "back to how it was after a restart"
                // guarantee (see freeze.rs): only holds while VaultExplorer's
                // mount is alive, so each launch re-establishes it from
                // scratch rather than resuming whatever was left dangling.
                let freeze_state = app.state::<FreezeState>();
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
                // ...and every local-folder pair's event-driven watch loop --
                // without this, a pair only actually kept syncing until the
                // app was next closed, since nothing else ever calls
                // `local_sync::start_loop` again after the one time it's
                // created.
                let local_sync_state = app.state::<local_sync::LocalSyncState>();
                for pair in local_sync::list_pairs() {
                    local_sync::start_loop(&local_sync_state, pair.folder_a, pair.folder_b);
                }
                // ...and every Drive pair's own watch loop, same reason:
                // otherwise a pair only auto-synced until the app was next
                // closed.
                let drive_sync_state = app.state::<sync::DriveSyncState>();
                for pair in sync::list_pairs() {
                    sync::start_loop(&drive_sync_state, pair);
                }
                } // end if !portal_activated
            }
            // Resume every git-synced folder's background poll loop too --
            // git itself works fine cross-platform (just `std::process::
            // Command`), so this one isn't desktop-gated. Still skipped for a
            // portal-activated picker server -- same reason as the loops above.
            if !portal_activated {
                let git_sync_state = app.state::<git_sync::GitSyncState>();
                for pair in git_sync::list_pairs() {
                    git_sync::start_loop(&git_sync_state, pair.local_path);
                }
            }
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
            lock_vault,
            set_active_vault,
            set_vault_auto_unlock,
            clear_vault_auto_unlock,
            auto_unlock_vaults,
            list_dir,
            search_vault,
            move_entry,
            copy_entry,
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
            #[cfg(desktop)]
            terminal::open_terminal,
            #[cfg(desktop)]
            terminal::run_shell_script,
            terminal::open_in_editor,
            is_mobile_platform,
            browse_root_dir,
            #[cfg(target_os = "android")]
            android::android_storage_access_granted,
            #[cfg(target_os = "android")]
            android::android_request_storage_access,
            fs_list,
            fs_is_vault,
            fs_set_readonly,
            fs_is_readonly,
            fs_search,
            fs_mkdir,
            fs_new_file,
            fs_read_text,
            fs_write_text,
            fs_save_pasted_image,
            vault_read_text,
            vault_write_text,
            share::fs_share_file,
            share::vault_share_file,
            fs_delete,
            shred::fs_secure_delete,
            #[cfg(desktop)]
            fs_trash,
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
            rclone::rclone_installed,
            rclone::rclone_providers,
            rclone::rclone_is_connected,
            rclone::rclone_connect,
            rclone::rclone_disconnect,
            sync::drive_list_pairs,
            sync::drive_add_pair,
            sync::drive_remove_pair,
            sync::drive_sync_now,
            sync::drive_syncing_now,
            sync::drive_sync_last_error,
            #[cfg(desktop)]
            sync::drive_sync_is_active,
            #[cfg(desktop)]
            fs_watch::fs_watch_set,
            git_sync::git_sync_list_pairs,
            git_sync::git_sync_is_active,
            git_sync::git_sync_syncing_now,
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
