//! "Use Vault Explorer as the default file manager" -- the MIME-database
//! half of being the app other programs hand a folder to.
//!
//! Three things have to line up for that, and on a normal install none of
//! them do by themselves:
//!
//! 1. A `.desktop` entry that declares `inode/directory` and is findable
//!    by desktop-id. The deb/rpm bundle installs one, but the bundler
//!    names it after `productName` -- so the file on disk is
//!    `Vault Explorer.desktop`, space and all, and the obvious-looking
//!    `inode/directory=vaultexplorer.desktop` points at nothing. A
//!    default whose desktop-id doesn't resolve isn't an error anywhere:
//!    the lookup just falls through to the next candidate (Nautilus),
//!    which is exactly the "it never becomes the default" symptom.
//! 2. That id listed under `[Default Applications]` in the mimeapps.list
//!    the session actually reads -- which, for `XDG_CURRENT_DESKTOP=
//!    ubuntu:GNOME`, is `~/.config/gnome-mimeapps.list` whenever that
//!    file exists, and only otherwise `~/.config/mimeapps.list`. Writing
//!    the plain file while the desktop-prefixed one exists changes
//!    nothing at all.
//! 3. `org.freedesktop.FileManager1` on the session bus, because
//!    "Show in folder" (Chrome's Downloads panel, OBS's "Show
//!    Recordings") never consults the MIME default -- see
//!    `filemanager1.rs`. Enabling this claims that too, so both routes
//!    land here rather than one of them silently still opening Nautilus.
//!
//! Everything here is user-scoped (`~/.config`, `~/.local/share`); no
//! root, no pkexec.

use crate::app_icon::{desktop_dirs, desktop_entry_hidden, desktop_entry_value, find_desktop_file};
use crate::errmap::{LockExt, ToStringErr};
use std::path::{Path, PathBuf};

/// A folder is reported under both of these. `inode/directory` is the
/// modern name; `x-directory/normal` is the legacy alias plenty of
/// still-shipping GTK/Qt code queries instead, and owning only the first
/// leaves those apps going to Nautilus.
const DIR_MIMES: [&str; 2] = ["inode/directory", "x-directory/normal"];

fn config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{}/.config", crate::home_dir())))
}

fn user_applications_dir() -> PathBuf {
    PathBuf::from(format!("{}/.local/share/applications", crate::home_dir()))
}

/// Every mimeapps.list this session honours, most specific first (see the
/// module note: the desktop-prefixed file shadows the plain one). Only
/// desktop-prefixed files that already exist are included -- creating one
/// would newly shadow the plain file for every *other* app's defaults too.
fn mimeapps_paths() -> Vec<PathBuf> {
    let config = config_dir();
    let mut paths: Vec<PathBuf> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| config.join(format!("{}-mimeapps.list", d.to_lowercase())))
        .filter(|p| p.is_file())
        .collect();
    paths.push(config.join("mimeapps.list"));
    paths
}

/// The program an entry launches. `Exec=` is a command line with field
/// codes (`%U`, `%f`), and the binary may be quoted -- the first token,
/// unquoted, is all that's needed to tell "this is us" from "this is
/// some other app".
fn exec_binary(path: &Path) -> Option<PathBuf> {
    let exec = desktop_entry_value(path, "Exec")?;
    let first = exec.split_whitespace().next()?.trim_matches('"');
    (!first.is_empty()).then(|| PathBuf::from(first))
}

fn launches_this_binary(entry: &Path) -> bool {
    let Some(bin) = exec_binary(entry) else { return false };
    let Ok(exe) = std::env::current_exe() else { return false };
    // Path equality covers an absolute `Exec=`; the basename comparison
    // covers the bundle's bare `Exec=vaultexplorer %U`, which resolves off
    // PATH and so has no path to compare.
    bin == exe || (bin.file_name().is_some() && bin.file_name() == exe.file_name())
}

/// The desktop-id folders should be routed to. Prefers an installed entry
/// that already launches this binary and already claims folders (the
/// bundle's own `Vault Explorer.desktop`) so enabling this doesn't leave
/// two near-identical "Vault Explorer" rows in every "Open With" list.
/// Writes a user-level entry only when there's nothing to reuse, which is
/// the AppImage / `tauri dev` case.
fn resolve_desktop_id() -> Result<String, String> {
    for dir in desktop_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            // The deep-link plugin drops its own `vaultexplorer-handler.desktop`
            // for the `vaultexplorer://` scheme: same binary, but NoDisplay
            // and no folder MIME type, so it's no use as the folder handler.
            if desktop_entry_hidden(&path) || !launches_this_binary(&path) {
                continue;
            }
            if !desktop_entry_value(&path, "MimeType")
                .unwrap_or_default()
                .contains("inode/directory")
            {
                continue;
            }
            if let Some(id) = path.file_name().and_then(|n| n.to_str()) {
                return Ok(id.to_string());
            }
        }
    }
    write_user_entry()?;
    Ok("vaultexplorer.desktop".to_string())
}

fn write_user_entry() -> Result<(), String> {
    let exe = std::env::current_exe().str_err()?;
    let dir = user_applications_dir();
    std::fs::create_dir_all(&dir).str_err()?;
    std::fs::write(
        dir.join("vaultexplorer.desktop"),
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Vault Explorer\n\
             Comment=File manager with encrypted vaults\n\
             Categories=System;FileTools;FileManager;\n\
             Exec=\"{}\" %U\n\
             Icon=vaultexplorer\n\
             StartupWMClass=vaultexplorer\n\
             Terminal=false\n\
             MimeType=inode/directory;x-directory/normal;\n",
            exe.display()
        ),
    )
    .str_err()?;
    // Without this the entry exists but isn't in `mimeinfo.cache`, so
    // other apps' "Open With" lists -- which read the cache, not the
    // directory -- still won't offer it.
    let _ = std::process::Command::new("update-desktop-database")
        .arg(&dir)
        .status();
    Ok(())
}

/// Where "the user asked for this" is recorded, independently of whether
/// the MIME database currently reflects it.
///
/// The whole feature used to infer its own state from `mimeapps.list`,
/// which means anything that rewrote that file -- another file manager
/// claiming folders, a `xdg-mime default` run by some installer, a stale
/// desktop-id left behind by an older build of this app (the exact state
/// found on the reporting machine: `inode/directory=vaultexplorer.desktop`
/// with no such file anywhere) -- silently turned the feature off. Nothing
/// announced it; folders just started opening in Nautilus again. With the
/// intent stored separately, every launch can put the configuration back,
/// which is what makes this stick across reinstalls instead of needing the
/// toggle flipped again.
fn marker_path() -> PathBuf {
    PathBuf::from(format!("{}/.local/share/vaultexplorer", crate::home_dir()))
        .join("default-file-manager")
}

fn mark_enabled() {
    let path = marker_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, "1\n");
}

/// Whether the user ever turned this on -- the marker, or (for anyone who
/// enabled it before the marker existed) a folder default that names us,
/// including one whose desktop-id no longer resolves.
pub fn was_enabled() -> bool {
    marker_path().is_file() || current_default().is_some_and(|id| is_ours(&id))
}

/// Re-apply the MIME-database side of the setting: point the folder MIME
/// types at a desktop-id that actually resolves, and refresh the D-Bus
/// activation file. Idempotent, cheap (a couple of small file rewrites),
/// and safe to run on every launch -- it only ever touches lines that are
/// ours.
pub fn reassert() -> Result<(), String> {
    let id = resolve_desktop_id()?;
    for path in mimeapps_paths() {
        rewrite_defaults(&path, Action::Enable(&id))?;
    }
    if let Ok(exe) = std::env::current_exe() {
        crate::filemanager1::write_registration(&exe.to_string_lossy())?;
    }
    // Other apps' "Open With" lists read mimeinfo.cache, not the
    // directories, so a fresh entry is invisible to them until this runs.
    let _ = std::process::Command::new("update-desktop-database")
        .arg(user_applications_dir())
        .status();
    mark_enabled();
    Ok(())
}

enum Action<'a> {
    Enable(&'a str),
    Disable,
}

/// Whether a desktop-id currently registered for folders is one turning
/// this feature off should take back. An id that resolves to no file at
/// all is only claimed when it names us -- that's the broken
/// `vaultexplorer.desktop` line this feature exists to replace; anyone
/// else's dangling entry stays their problem.
fn is_ours(desktop_id: &str) -> bool {
    match find_desktop_file(desktop_id) {
        Some(path) => launches_this_binary(&path),
        None => desktop_id.contains("vaultexplorer"),
    }
}

/// Points `DIR_MIMES` at a desktop-id under `[Default Applications]`, or
/// on `Disable` drops the entries that are ours. Every other line in the
/// file -- other sections, other MIME types, comments, ordering -- is kept
/// verbatim: this file holds every app's defaults, not just ours, and a
/// folder default some third app owns is not ours to clear.
fn rewrite_defaults(path: &Path, action: Action) -> Result<(), String> {
    if matches!(action, Action::Disable) && !path.exists() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut section = String::new();
    // Where a new entry belongs: just past the last line of
    // `[Default Applications]`, wherever that section happens to end.
    let mut insert_at: Option<usize> = None;
    for line in existing.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if section == "Default Applications" {
                insert_at = Some(lines.len());
            }
            section = name.to_string();
        } else if section == "Default Applications" {
            if let Some((mime, value)) = trimmed.split_once('=') {
                if DIR_MIMES.contains(&mime.trim()) {
                    let drop = match action {
                        // Dropped here and re-added below, so a stale id
                        // (the `vaultexplorer.desktop` that resolved to no
                        // file) is replaced rather than left sitting above
                        // the new line.
                        Action::Enable(_) => true,
                        Action::Disable => {
                            value.split(';').next().is_some_and(|v| is_ours(v.trim()))
                        }
                    };
                    if drop {
                        continue;
                    }
                }
            }
        }
        lines.push(line.to_string());
    }
    if section == "Default Applications" {
        insert_at = Some(lines.len());
    }
    if let Action::Enable(id) = action {
        let new: Vec<String> = DIR_MIMES.iter().map(|m| format!("{m}={id}")).collect();
        match insert_at {
            Some(at) => {
                for (i, line) in new.into_iter().enumerate() {
                    lines.insert(at + i, line);
                }
            }
            None => {
                let mut head = vec!["[Default Applications]".to_string()];
                head.extend(new);
                if !lines.is_empty() {
                    head.push(String::new());
                }
                head.extend(lines);
                lines = head;
            }
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).str_err()?;
    }
    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(path, out).str_err()
}

/// The desktop-id currently handling folders, read the way the desktop
/// reads it: first file that mentions `inode/directory` wins, and a
/// `;`-separated value is a preference list whose head is the default.
fn current_default() -> Option<String> {
    for path in mimeapps_paths() {
        let Ok(contents) = std::fs::read_to_string(&path) else { continue };
        let mut section = String::new();
        for line in contents.lines() {
            let trimmed = line.trim();
            if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section = name.to_string();
                continue;
            }
            if section != "Default Applications" {
                continue;
            }
            if let Some((mime, value)) = trimmed.split_once('=') {
                if mime.trim() == "inode/directory" {
                    return value.split(';').next().map(|v| v.trim().to_string());
                }
            }
        }
    }
    None
}

/// Deliberately resolves the id to a real file instead of just string-
/// matching it: an entry naming a `.desktop` that doesn't exist is
/// precisely the broken state this feature exists to fix, and reporting
/// that as "on" would leave the toggle checked while folders keep opening
/// in Nautilus.
#[tauri::command]
pub fn default_file_manager_enabled() -> bool {
    current_default()
        .and_then(|id| find_desktop_file(&id))
        .is_some_and(|path| launches_this_binary(&path))
}

#[tauri::command]
pub async fn set_default_file_manager(
    app: tauri::AppHandle,
    fm_state: tauri::State<'_, crate::filemanager1::FileManagerState>,
    enabled: bool,
) -> Result<(), String> {
    if !enabled {
        let _ = std::fs::remove_file(marker_path());
        for path in mimeapps_paths() {
            rewrite_defaults(&path, Action::Disable)?;
        }
        // The running FileManager1 service is left alone (dropping the bus
        // name mid-session would only strand in-flight callers); only the
        // activation registration is withdrawn, and only if the system-
        // file-picker toggle isn't relying on it too.
        if !crate::portal::is_enabled() {
            crate::filemanager1::remove_registration();
        }
        return Ok(());
    }
    reassert()?;
    let already_running = fm_state.connection.lock_safe().is_some();
    if !already_running {
        let conn = crate::filemanager1::start_service(app).await?;
        *fm_state.connection.lock_safe() = Some(conn);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = PathBuf::from(format!("/tmp/ve-mimeapps-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    /// The exact shape found on the machine that reported this: a stale
    /// `inode/directory` line naming a desktop-id with no file behind it,
    /// sitting in a file shared with every other app's defaults.
    #[test]
    fn enable_replaces_stale_entry_and_keeps_everything_else() {
        let path = scratch("stale");
        std::fs::write(
            &path,
            "[Default Applications]\n\
             text/html=brave_brave.desktop\n\
             inode/directory=vaultexplorer.desktop\n\
             \n\
             [Added Associations]\n\
             text/plain=sublime-text_subl.desktop;\n",
        )
        .unwrap();

        rewrite_defaults(&path, Action::Enable("Vault Explorer.desktop")).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();

        assert!(!out.contains("inode/directory=vaultexplorer.desktop"));
        assert!(out.contains("inode/directory=Vault Explorer.desktop"));
        assert!(out.contains("x-directory/normal=Vault Explorer.desktop"));
        assert!(out.contains("text/html=brave_brave.desktop"));
        assert!(out.contains("[Added Associations]"));
        assert!(out.contains("text/plain=sublime-text_subl.desktop;"));
        // The new lines belong to [Default Applications], not to the
        // section that follows it.
        let defaults = out.find("[Default Applications]").unwrap();
        let added = out.find("[Added Associations]").unwrap();
        let dir_line = out.find("inode/directory=").unwrap();
        assert!(dir_line > defaults && dir_line < added);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn enable_creates_the_section_when_the_file_has_none() {
        let path = scratch("nosection");
        std::fs::write(&path, "[Added Associations]\ntext/plain=foo.desktop;\n").unwrap();

        rewrite_defaults(&path, Action::Enable("vaultexplorer.desktop")).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();

        assert!(out.starts_with("[Default Applications]\ninode/directory=vaultexplorer.desktop\n"));
        assert!(out.contains("[Added Associations]"));
        assert!(out.contains("text/plain=foo.desktop;"));
        let _ = std::fs::remove_file(&path);
    }

    /// Turning the setting off reverts *our* claim only. Another app's
    /// folder default is a choice the user made elsewhere.
    #[test]
    fn disable_leaves_another_apps_default_alone() {
        let path = scratch("disable");
        std::fs::write(
            &path,
            "[Default Applications]\n\
             inode/directory=vaultexplorer-not-a-real-file.desktop\n\
             x-directory/normal=definitely-not-installed-xyz.desktop\n",
        )
        .unwrap();

        rewrite_defaults(&path, Action::Disable).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();

        assert!(!out.contains("vaultexplorer-not-a-real-file.desktop"));
        assert!(out.contains("x-directory/normal=definitely-not-installed-xyz.desktop"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn disable_does_not_create_a_missing_file() {
        let path = scratch("absent");
        rewrite_defaults(&path, Action::Disable).unwrap();
        assert!(!path.exists());
    }
}
