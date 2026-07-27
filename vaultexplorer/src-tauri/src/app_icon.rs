//! Resolves the icon of whichever app the desktop has registered to *open*
//! a given file extension -- a `.docx` shows the Word/LibreOffice-Writer
//! icon, not a generic sheet, same idea Nautilus/Files use for file types
//! it has no bundled artwork for. Sourced live from the desktop's own
//! MIME/icon-theme databases (`xdg-mime` + the `.desktop` file's `Icon=`
//! + GTK icon-theme lookup) instead of a fixed bundled asset, so it always
//! matches whatever's actually installed (LibreOffice, OnlyOffice, WPS,
//! Word-via-something, ...) rather than one hardcoded guess.

use base64::{engine::general_purpose::STANDARD, Engine as _};
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

fn icon_key_from_desktop_file(path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let mut in_desktop_entry = false;
    for line in contents.lines() {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_desktop_entry = section == "Desktop Entry";
            continue;
        }
        if in_desktop_entry {
            if let Some(rest) = line.strip_prefix("Icon=") {
                return Some(rest.trim().to_string());
            }
        }
    }
    None
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
        Command::new("xdg-mime")
            .args(["query", "default", &mime_owned])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
    .await
    .ok()
    .flatten();

    let icon_name = match desktop_id {
        Some(id) => {
            tauri::async_runtime::spawn_blocking(move || {
                find_desktop_file(&id).and_then(|p| icon_key_from_desktop_file(&p))
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
        let _ = tx.send(resolve_icon_bytes(&icon_name));
    });
    let resolved = if hop.is_ok() { rx.await.ok().flatten() } else { None };

    let data_uri = resolved.map(|(bytes, mime_type)| {
        format!("data:{mime_type};base64,{}", STANDARD.encode(bytes))
    });
    cache().lock().unwrap().insert(mime.to_string(), data_uri.clone());
    data_uri
}
