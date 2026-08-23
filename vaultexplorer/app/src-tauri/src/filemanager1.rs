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

/// Last reveal request, kept so a *cold start* isn't lost. `reveal()` emits
/// an event, and an event has no listeners when D-Bus activation is what
/// started the process: the JS side hasn't mounted (let alone registered its
/// listener) by the time ShowItems is serviced, so the emit goes nowhere and
/// the app just opens on its default folder. The frontend drains this on
/// mount instead; the emit stays as the path for an already-running app.
fn pending_reveal() -> &'static Mutex<Option<ShowInFolderPayload>> {
    static PENDING: std::sync::OnceLock<Mutex<Option<ShowInFolderPayload>>> =
        std::sync::OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(None))
}

/// Drained once by the frontend as it mounts. Returns `None` for an app that
/// was already running when the request arrived (the event handled it).
#[tauri::command]
pub fn take_pending_reveal() -> Option<ShowInFolderPayload> {
    pending_reveal().lock().ok().and_then(|mut slot| slot.take())
}

/// Queue a reveal for the next window that mounts (same slot ShowItems
/// uses for cold starts) -- lets a directory CLI argument reuse the whole
/// drain path.
pub fn set_pending_reveal(path: String, select: Option<String>) {
    if let Ok(mut slot) = pending_reveal().lock() {
        *slot = Some(ShowInFolderPayload { path, select });
    }
}

/// Resolve a CLI argument to an existing directory, accepting both a plain
/// path and the `file://` URI a `%U` Exec field code delivers. This is how
/// "open a folder with VaultExplorer" arrives when the caller went through
/// xdg-open/gio (OBS's "Show Recordings", a double-clicked folder once
/// VaultExplorer is the inode/directory default) rather than through the
/// org.freedesktop.FileManager1 service above (Chrome-style "Show in
/// folder", which only works while a VaultExplorer process owns the name).
pub fn cli_dir_arg(arg: &str) -> Option<String> {
    if arg.starts_with('-') || (arg.contains("://") && !arg.starts_with("file://")) {
        return None;
    }
    let path = uri_to_path(arg).unwrap_or_else(|| arg.to_string());
    std::path::Path::new(&path).is_dir().then_some(path)
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
            // Most WMs (KDE/GNOME, X11 or Wayland) refuse to hand focus to
            // a window that wasn't already focused -- ICCCM/EWMH focus-
            // stealing prevention, same restriction noted for the portal
            // picker. set_focus() above is a no-op there. Urgency hint
            // (taskbar/dock flash) isn't blocked the same way, so it's the
            // fallback that actually gets noticed.
            let _ = main.request_user_attention(Some(tauri::UserAttentionType::Critical));
        }
        let payload = ShowInFolderPayload { path, select };
        if let Ok(mut slot) = pending_reveal().lock() {
            *slot = Some(payload.clone());
        }
        let _ = self.app.emit("show-in-folder", payload);
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

/// `replace_existing_names` only works if the current owner *allowed*
/// replacement, and Nautilus (running as `nautilus --gapplication-service`
/// from login) does not -- so requesting the name quietly loses and every
/// "Show in Files" keeps landing in Nautilus, which is exactly the
/// reported behaviour. Asking that instance to quit frees the name; it is
/// a plain `nautilus -q` (the documented way to end the service, not a
/// kill) and it only happens when the current owner really is Nautilus,
/// so nothing else on the bus is ever touched.
async fn yield_name_from_nautilus(conn: &zbus::Connection) {
    let dbus = zbus::fdo::DBusProxy::new(conn).await;
    let Ok(dbus) = dbus else { return };
    let name = match zbus::names::BusName::try_from(FM_BUS_NAME) {
        Ok(n) => n,
        Err(_) => return,
    };
    let Ok(pid) = dbus.get_connection_unix_process_id(name).await else { return };
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
    if comm.trim() != "nautilus" {
        return;
    }
    let _ = std::process::Command::new("nautilus").arg("-q").status();
}

/// Re-request the name for this connection, replacing whoever holds it.
async fn request_fm_name(conn: &zbus::Connection) -> bool {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else { return false };
    let Ok(name) = zbus::names::WellKnownName::try_from(FM_BUS_NAME) else { return false };
    dbus.request_name(name, Default::default()).await.is_ok()
}

/// Take the name back when something else grabs it mid-session.
///
/// The claim below is made with D-Bus's default flags, which include
/// `AllowReplacement` -- so Nautilus starting up *after* VaultExplorer
/// (the user opens Files once, or any app hands GNOME a folder) silently
/// takes `org.freedesktop.FileManager1` away again, and every later "Show
/// in folder" lands in Nautilus even though VaultExplorer is still
/// running and still configured as the file manager. That is the
/// "sometimes it works, sometimes it doesn't" half of this feature: the
/// configuration was never the thing that changed, ownership was.
///
/// Bounded on purpose. If some other program is *also* re-claiming on
/// loss, two processes trading the name forever is worse than losing it
/// once, so this gives up after a handful of rounds.
const RECLAIM_LIMIT: usize = 5;

async fn watch_for_name_loss(conn: zbus::Connection) {
    use futures_util::StreamExt;
    let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else { return };
    let Ok(mut lost) = dbus.receive_name_lost().await else { return };
    let mut reclaims = 0usize;
    while let Some(signal) = lost.next().await {
        let Ok(args) = signal.args() else { continue };
        if args.name.as_str() != FM_BUS_NAME {
            continue;
        }
        reclaims += 1;
        if reclaims > RECLAIM_LIMIT {
            eprintln!("filemanager1: gave up reclaiming {FM_BUS_NAME} after {RECLAIM_LIMIT} rounds");
            break;
        }
        yield_name_from_nautilus(&conn).await;
        if !request_fm_name(&conn).await {
            eprintln!("filemanager1: failed to reclaim {FM_BUS_NAME}");
        }
    }
}

pub async fn start_service(app: AppHandle) -> Result<zbus::Connection, String> {
    // A first, name-less connection just to see who holds it. Doing this
    // before the real request keeps the common case (nobody else has it)
    // to a single round-trip.
    if let Ok(probe) = zbus::Connection::session().await {
        yield_name_from_nautilus(&probe).await;
    }
    let iface = FileManager1Iface { app };
    let conn = zbus::connection::Builder::session()
        .str_err()?
        .name(FM_BUS_NAME)
        .str_err()?
        .replace_existing_names(true)
        .serve_at("/org/freedesktop/FileManager1", iface)
        .str_err()?
        .build()
        .await
        .str_err()?;
    let watcher = conn.clone();
    tauri::async_runtime::spawn(async move { watch_for_name_loss(watcher).await });
    Ok(conn)
}
