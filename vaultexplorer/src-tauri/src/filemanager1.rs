//! Implements `org.freedesktop.FileManager1` so VaultExplorer can be *the*
//! file manager for "Show in folder"-type actions (Chrome's Downloads
//! panel, OBS's "Show Recordings", etc.).
//!
//! Unlike `portal.rs`'s FileChooser integration, this isn't routed through
//! `xdg-desktop-portal` at all -- `org.freedesktop.FileManager1` is a plain
//! session-bus well-known name with no preference/voting mechanism:
//! whichever process currently owns the name wins, full stop. Nautilus
//! normally holds it forever (it runs a persistent
//! `nautilus --gapplication-service` from login), so becoming the default
//! here means requesting the name with `ReplaceExisting` to take it away
//! from Nautilus for as long as VaultExplorer is running. When VaultExplorer
//! quits, the name is simply released (Nautilus doesn't proactively
//! reclaim it) -- for this to stick with nothing else running, VaultExplorer
//! would need to run persistently in the background too, which this alone
//! doesn't set up.

use crate::errmap::ToStringErr;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use zbus::interface;

const FM_BUS_NAME: &str = "org.freedesktop.FileManager1";

#[derive(Default)]
pub struct FileManagerState {
    /// Same reasoning as `portal::PortalState::connection`: holding this
    /// alive is what keeps the bus name owned.
    pub connection: Mutex<Option<zbus::Connection>>,
}

#[derive(Serialize, Clone)]
pub struct ShowInFolderPayload {
    pub path: String,
    pub select: Option<String>,
}

/// Minimal percent-decoder for the `file://` URIs callers hand us (a
/// filename with a space becomes `%20`, etc.) -- pulling in a whole crate
/// for this one call site isn't worth it, matching `portal.rs`'s own
/// `url_encode` reasoning in the other direction.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn uri_to_path(uri: &str) -> Option<String> {
    uri.strip_prefix("file://").map(percent_decode)
}

pub struct FileManager1Iface {
    pub app: AppHandle,
}

impl FileManager1Iface {
    /// Brings the main window to the front and asks the frontend (via a
    /// plain `emit`, not a command return value -- nothing is waiting on
    /// this call's result) to navigate to `path`, optionally selecting
    /// `select` once the listing loads.
    fn reveal(&self, path: String, select: Option<String>) {
        if let Some(main) = self.app.get_webview_window("main") {
            let _ = main.show();
            let _ = main.unminimize();
            let _ = main.set_focus();
        }
        let _ = self.app.emit("show-in-folder", ShowInFolderPayload { path, select });
    }
}

#[interface(name = "org.freedesktop.FileManager1")]
impl FileManager1Iface {
    async fn show_folders(&self, uris: Vec<String>, _startup_id: String) {
        if let Some(path) = uris.first().and_then(|u| uri_to_path(u)) {
            self.reveal(path, None);
        }
    }

    async fn show_items(&self, uris: Vec<String>, _startup_id: String) {
        if let Some(full) = uris.first().and_then(|u| uri_to_path(u)) {
            let path = std::path::Path::new(&full);
            let parent = path.parent().map(|p| p.to_string_lossy().into_owned());
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            if let Some(parent) = parent {
                self.reveal(parent, name);
            }
        }
    }

    // No dedicated "properties" panel distinct from just revealing the
    // item -- same behavior as ShowItems.
    async fn show_item_properties(&self, uris: Vec<String>, startup_id: String) {
        self.show_items(uris, startup_id).await;
    }
}

fn service_file_path() -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}/.local/share/dbus-1/services", crate::home_dir()))
        .join(format!("{FM_BUS_NAME}.service"))
}

/// Writes the D-Bus service-activation file for `org.freedesktop.FileManager1`
/// -- a plain session-bus name, so unlike `portal.rs`'s FileChooser
/// integration this needs no `.portal`/`portals.conf`/pkexec dance at all,
/// just this one file. Nautilus's own persistent `--gapplication-service`
/// normally holds the name from login onward so this activation path won't
/// often fire in practice (a name already owned never triggers activation)
/// -- it's a fallback for whenever nobody currently owns it (Nautilus not
/// running) rather than the primary way VaultExplorer takes over; the
/// primary way is `start_service` below, called whenever VaultExplorer
/// itself is already running with the toggle on. Reuses the same
/// `--portal-activated` marker as the FileChooser service file: either one
/// D-Bus-activating VaultExplorer should skip showing the main window
/// until something actually needs it.
pub fn write_registration(exe_path: &str) -> Result<(), String> {
    let svc_dir = service_file_path();
    std::fs::create_dir_all(svc_dir.parent().unwrap()).str_err()?;
    std::fs::write(
        &svc_dir,
        format!("[D-BUS Service]\nName={FM_BUS_NAME}\nExec={exe_path} --portal-activated\n"),
    )
    .str_err()
}

pub fn remove_registration() {
    let _ = std::fs::remove_file(service_file_path());
}

pub async fn start_service(app: AppHandle) -> Result<zbus::Connection, String> {
    let iface = FileManager1Iface { app };
    zbus::connection::Builder::session()
        .str_err()?
        .name(FM_BUS_NAME)
        .str_err()?
        .replace_existing_names(true)
        .serve_at("/org/freedesktop/FileManager1", iface)
        .str_err()?
        .build()
        .await
        .str_err()
}
