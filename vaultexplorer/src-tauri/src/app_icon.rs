//! Resolves the icon of whichever app the desktop has registered to *open*
//! a given file extension -- a `.docx` shows the Word/LibreOffice-Writer
//! icon, not a generic sheet, same idea Nautilus/Files use for file types
//! it has no bundled artwork for. Sourced live from the desktop's own
//! MIME/icon-theme databases (`xdg-mime` + the `.desktop` file's `Icon=`
//! + GTK icon-theme lookup) instead of a fixed bundled asset, so it always
//! matches whatever's actually installed (LibreOffice, OnlyOffice, WPS,
//! Word-via-something, ...) rather than one hardcoded guess.
//!
//! Also backs "Open With…" (`list_apps_for_path` / `open_with`) -- same
//! desktop-file/icon-theme machinery, just enumerating every registered
//! handler for a real file's MIME type instead of one default.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

fn cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Office-ish extensions `icons.tsx`'s `kindOf` otherwise buckets as
/// "generic" -- everything else (images, video, code, ...) already has a
/// bundled icon and never calls this.
fn mime_for_ext(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "odt" => "application/vnd.oasis.opendocument.text",
        "rtf" => "application/rtf",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        _ => return None,
    })
}

fn desktop_dirs() -> Vec<PathBuf> {
    let home = crate::home_dir();
    vec![
        PathBuf::from(format!("{home}/.local/share/applications")),
        PathBuf::from(format!("{home}/.local/share/flatpak/exports/share/applications")),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        // Snap-packaged apps (e.g. LibreOffice on Ubuntu, which is a snap
        // by default) register their `.desktop` files here, not under
        // /usr/share/applications -- confirmed missing this is exactly why
        // office-doc icons resolved to nothing on this machine: `xdg-mime
        // query default` for a .docx returns `libreoffice_writer.desktop`,
        // which only exists in this directory.
        PathBuf::from("/var/lib/snapd/desktop/applications"),
        PathBuf::from("/usr/local/share/applications"),
        PathBuf::from("/usr/share/applications"),
    ]
}

/// `xdg-mime query default` returns a desktop-id like
/// `libreoffice-writer.desktop`, but some installs (flatpak, some distro
/// packages) nest their `.desktop` file a level down (e.g. `kde4/`) --
/// one shallow subdir walk covers that without a whole crate for it.
fn find_desktop_file(desktop_id: &str) -> Option<PathBuf> {
    for dir in desktop_dirs() {
        let direct = dir.join(desktop_id);
        if direct.is_file() {
            return Some(direct);
        }
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let nested = entry.path().join(desktop_id);
                if nested.is_file() {
                    return Some(nested);
                }
            }
        }
    }
    None
}

/// Reads one `Key=value` line out of a `.desktop` file's `[Desktop Entry]`
/// section (the only section relevant here -- action-specific keys under
/// `[Desktop Action ...]` sections are never what these callers want).
fn desktop_entry_value(path: &Path, key: &str) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let prefix = format!("{key}=");
    let mut in_desktop_entry = false;
    for line in contents.lines() {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_desktop_entry = section == "Desktop Entry";
            continue;
        }
        if in_desktop_entry {
            if let Some(rest) = line.strip_prefix(&prefix) {
                return Some(rest.trim().to_string());
            }
        }
    }
    None
}

fn desktop_entry_hidden(path: &Path) -> bool {
    desktop_entry_value(path, "NoDisplay").as_deref() == Some("true")
        || desktop_entry_value(path, "Hidden").as_deref() == Some("true")
}

/// Turns a themed icon name (or an already-absolute path, which some
/// `.desktop` files use directly) into image bytes. Must run on the GTK
/// main thread -- `gtk::IconTheme` touches global GTK state that isn't
/// safe to call from a tokio worker.
fn resolve_icon_bytes(icon: &str) -> Option<(Vec<u8>, &'static str)> {
    let path: PathBuf = if icon.starts_with('/') {
        PathBuf::from(icon)
    } else {
        use gtk::prelude::IconThemeExt;
        let theme = gtk::IconTheme::default()?;
        let info = theme
            .lookup_icon(icon, 48, gtk::IconLookupFlags::FORCE_SIZE)
            .or_else(|| theme.lookup_icon(icon, 48, gtk::IconLookupFlags::empty()))?;
        info.filename()?
    };
    // Only the two formats the web view can render directly off a data:
    // URI without a re-encode step -- a raster icon theme lookup essentially
    // only ever resolves to one of these two anyway.
    let mime = match path.extension().and_then(|e| e.to_str()) {
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        _ => return None,
    };
    let bytes = std::fs::read(&path).ok()?;
    Some((bytes, mime))
}

fn icon_data_uri(icon: &str) -> Option<String> {
    resolve_icon_bytes(icon)
        .map(|(bytes, mime_type)| format!("data:{mime_type};base64,{}", STANDARD.encode(bytes)))
}

fn xdg_mime_query(args: &[&str]) -> Option<String> {
    Command::new("xdg-mime")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A `data:` URI for the icon of whatever app is registered to open `ext`,
/// cached in memory per-MIME-type for the process lifetime. `None` for any
/// extension with no office-document mapping, no registered default app,
/// or no resolvable icon -- callers keep their bundled generic icon then.
#[tauri::command]
pub async fn app_icon_for_ext(app: tauri::AppHandle, ext: String) -> Option<String> {
    let mime = mime_for_ext(&ext.to_lowercase())?;
    if let Some(hit) = cache().lock().unwrap().get(mime) {
        return hit.clone();
    }

    let mime_owned = mime.to_string();
    let desktop_id = tauri::async_runtime::spawn_blocking(move || {
        xdg_mime_query(&["query", "default", &mime_owned])
    })
    .await
    .ok()
    .flatten();

    let icon_name = match desktop_id {
        Some(id) => {
            tauri::async_runtime::spawn_blocking(move || {
                find_desktop_file(&id).and_then(|p| desktop_entry_value(&p, "Icon"))
            })
            .await
            .ok()
            .flatten()
        }
        None => None,
    };

    let Some(icon_name) = icon_name else {
        cache().lock().unwrap().insert(mime.to_string(), None);
        return None;
    };

    // GTK calls must happen on the main thread; hop over via a oneshot and
    // await the result back here.
    let (tx, rx) = tokio::sync::oneshot::channel();
    let hop = app.run_on_main_thread(move || {
        let _ = tx.send(icon_data_uri(&icon_name));
    });
    let data_uri = if hop.is_ok() { rx.await.ok().flatten() } else { None };

    cache().lock().unwrap().insert(mime.to_string(), data_uri.clone());
    data_uri
}

#[derive(Serialize, Clone)]
pub struct AppInfo {
    id: String,
    name: String,
    icon: Option<String>,
    is_default: bool,
}

/// Every desktop-id `<dir>/mimeinfo.cache`'s `[MIME Cache]` section lists
/// for `mime`, merged across every dir in `desktop_dirs()` (a file's
/// registered handlers are split across the user, system, and snap
/// directories -- e.g. Chrome only shows up in the user one, LibreOffice's
/// alternatives only in the snap one). Order isn't meaningful here; the
/// caller sorts.
fn registered_desktop_ids(mime: &str) -> Vec<String> {
    let prefix = format!("{mime}=");
    let mut ids = Vec::new();
    for dir in desktop_dirs() {
        let Ok(contents) = std::fs::read_to_string(dir.join("mimeinfo.cache")) else {
            continue;
        };
        for line in contents.lines() {
            let Some(rest) = line.strip_prefix(&prefix) else { continue };
            for id in rest.split(';') {
                if !id.is_empty() && !ids.iter().any(|existing: &String| existing == id) {
                    ids.push(id.to_string());
                }
            }
        }
    }
    ids
}

/// Every app registered to open `path` (by its real, detected MIME type --
/// `xdg-mime query filetype`, not just the extension), for "Open With…".
/// Icons resolved the same way as `app_icon_for_ext`, batched into one
/// GTK-main-thread hop rather than one per app.
#[tauri::command]
pub async fn list_apps_for_path(app: tauri::AppHandle, path: String) -> Vec<AppInfo> {
    let Some(mime) = tauri::async_runtime::spawn_blocking(move || {
        xdg_mime_query(&["query", "filetype", &path])
    })
    .await
    .ok()
    .flatten() else {
        return Vec::new();
    };

    let mime_for_default = mime.clone();
    let (default_id, entries) = tauri::async_runtime::spawn_blocking(move || {
        let default_id = xdg_mime_query(&["query", "default", &mime_for_default]);
        let mut ids = registered_desktop_ids(&mime_for_default);
        if let Some(d) = &default_id {
            if !ids.iter().any(|existing| existing == d) {
                ids.insert(0, d.clone());
            }
        }
        let entries: Vec<(String, String, Option<String>)> = ids
            .into_iter()
            .filter_map(|id| {
                let desktop_path = find_desktop_file(&id)?;
                if desktop_entry_hidden(&desktop_path) {
                    return None;
                }
                // No Exec= means nothing to actually launch (e.g. a pure
                // MIME-type-association placeholder) -- not a usable choice.
                desktop_entry_value(&desktop_path, "Exec")?;
                let name = desktop_entry_value(&desktop_path, "Name").unwrap_or_else(|| id.clone());
                let icon = desktop_entry_value(&desktop_path, "Icon");
                Some((id, name, icon))
            })
            .collect();
        (default_id, entries)
    })
    .await
    .unwrap_or((None, Vec::new()));

    let icon_names: Vec<Option<String>> = entries.iter().map(|(_, _, icon)| icon.clone()).collect();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let hop = app.run_on_main_thread(move || {
        let resolved: Vec<Option<String>> =
            icon_names.into_iter().map(|icon| icon.and_then(|i| icon_data_uri(&i))).collect();
        let _ = tx.send(resolved);
    });
    let icons = if hop.is_ok() { rx.await.unwrap_or_default() } else { Vec::new() };

    let mut apps: Vec<AppInfo> = entries
        .into_iter()
        .zip(icons.into_iter().chain(std::iter::repeat(None)))
        .map(|((id, name, _), icon)| {
            let is_default = default_id.as_deref() == Some(id.as_str());
            AppInfo { id, name, icon, is_default }
        })
        .collect();
    apps.sort_by(|a, b| b.is_default.cmp(&a.is_default).then_with(|| a.name.cmp(&b.name)));
    apps
}

#[derive(Serialize, Clone)]
pub struct AppChoice {
    id: String,
    name: String,
    comment: Option<String>,
    icon_name: Option<String>,
}

/// Every installed, launchable app -- the full list behind "Open With… >
/// Other Application…", where the point is to *find an app that isn't
/// registered for this MIME type* (the Windows "choose another app" case),
/// so this deliberately ignores MIME registration entirely.
///
/// Icons come back as theme *names*, not data URIs: a machine has a few
/// hundred `.desktop` files, and resolving every one through the GTK icon
/// theme would mean a few hundred main-thread file reads + base64 encodes
/// for rows that are mostly filtered out or scrolled away. The picker asks
/// for the handful it actually renders via `app_icons`.
#[tauri::command]
pub async fn list_all_apps() -> Vec<AppChoice> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut by_id: HashMap<String, AppChoice> = HashMap::new();
        for dir in desktop_dirs() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for entry in rd.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                let Some(id) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
                    continue;
                };
                // First dir wins -- `desktop_dirs()` is ordered user-first,
                // so a `~/.local/share/applications` override shadows the
                // system copy of the same desktop-id, which is what the
                // desktop itself would launch.
                if by_id.contains_key(&id) {
                    continue;
                }
                if desktop_entry_hidden(&path) {
                    continue;
                }
                // Link/Directory entries aren't apps, and no Exec= means
                // there's nothing to launch.
                if desktop_entry_value(&path, "Type").as_deref().unwrap_or("Application")
                    != "Application"
                {
                    continue;
                }
                if desktop_entry_value(&path, "Exec").is_none() {
                    continue;
                }
                let name = desktop_entry_value(&path, "Name").unwrap_or_else(|| id.clone());
                let comment = desktop_entry_value(&path, "Comment");
                let icon_name = desktop_entry_value(&path, "Icon");
                by_id.insert(id.clone(), AppChoice { id, name, comment, icon_name });
            }
        }
        let mut apps: Vec<AppChoice> = by_id.into_values().collect();
        apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        apps
    })
    .await
    .unwrap_or_default()
}

/// `data:` URIs for a batch of themed icon names (as returned by
/// `list_all_apps`), resolved in one GTK-main-thread hop instead of one per
/// icon. Same length and order as `icons`; `None` where nothing resolved.
#[tauri::command]
pub async fn app_icons(app: tauri::AppHandle, icons: Vec<String>) -> Vec<Option<String>> {
    let count = icons.len();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let hop = app.run_on_main_thread(move || {
        let resolved: Vec<Option<String>> = icons.iter().map(|i| icon_data_uri(i)).collect();
        let _ = tx.send(resolved);
    });
    if hop.is_err() {
        return vec![None; count];
    }
    rx.await.unwrap_or_else(|_| vec![None; count])
}

/// Expands a `.desktop` file's `Exec=` value for launching one specific
/// `path`: substitutes the single-file field codes (`%f`/`%F`/`%u`/`%U`)
/// with `path` and `%%` with a literal `%`, and drops the ones this caller
/// has nothing meaningful to put in their place (`%i`/`%c`/`%k`/`%v`/`%m`
/// -- icon/name/desktop-file-path/device/mimetype, all fine to just omit).
/// Only handles double-quoted tokens (the one quoting form the Desktop
/// Entry spec actually expects `Exec=` to use) -- not a full shell lexer,
/// but every real `.desktop` file seen in practice needs nothing more.
fn parse_exec(exec: &str, path: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in exec.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            ' ' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
        .into_iter()
        .filter_map(|t| match t.as_str() {
            "%f" | "%F" | "%u" | "%U" => Some(path.to_string()),
            "%i" | "%c" | "%k" | "%v" | "%m" => None,
            "%%" => Some("%".to_string()),
            other => Some(other.to_string()),
        })
        .collect()
}

/// Launches `path` with the app named by `desktop_id` (one of the ids
/// `list_apps_for_path` returned) -- the "Open With…" action itself.
#[tauri::command]
pub fn open_with(path: String, desktop_id: String) -> Result<(), String> {
    let desktop_path =
        find_desktop_file(&desktop_id).ok_or_else(|| format!("{desktop_id} not found"))?;
    let exec = desktop_entry_value(&desktop_path, "Exec")
        .ok_or_else(|| format!("{desktop_id} has no Exec= entry"))?;
    let argv = parse_exec(&exec, &path);
    let Some((bin, args)) = argv.split_first() else {
        return Err(format!("{desktop_id} has an empty Exec="));
    };
    std::process::Command::new(bin)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't launch \"{bin}\": {e}"))
}
