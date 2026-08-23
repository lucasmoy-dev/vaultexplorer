//! System file-picker integration: implements the
//! `org.freedesktop.impl.portal.FileChooser` D-Bus interface so
//! VaultExplorer can be set as the Open/Save dialog for portal-aware apps
//! (Flatpak/Snap, or native apps that opt into the desktop portal).
//!
//! **Real scope, stated plainly**: this only affects apps that actually
//! talk to `xdg-desktop-portal` for their file dialogs. Ordinary
//! desktop-installed GTK/Qt apps mostly use their own in-process native
//! file chooser and never touch the portal at all -- no amount of code
//! here changes that, it's a structural property of which apps opt into
//! the portal. Registration is scoped to *only* the FileChooser interface
//! preference, per-user (`~/.config/xdg-desktop-portal/portals.conf`), so
//! every other portal interface (screenshot, notifications, etc.) keeps
//! using the normal desktop backend untouched.
//!
//! One piece of this genuinely isn't per-user, though: the `.portal`
//! implementation file itself. Confirmed directly against this system's
//! `xdg-desktop-portal` (`--verbose` startup log, and by testing whether
//! overriding `XDG_DATA_DIRS` changed anything -- it didn't) that it only
//! ever scans the single hardcoded `/usr/share/xdg-desktop-portal/portals`
//! directory for portal implementations, full stop -- a per-user copy is
//! silently never loaded no matter how `portals.conf` is set up. So
//! enabling this needs one `pkexec`-elevated file copy (see
//! `install_system_portal_file`) -- a one-time graphical polkit prompt,
//! the same mechanism `machine.rs`'s "Update Drivers" already uses, not a
//! standing elevated process.
//!
//! Method signatures below were verified against the live system this was
//! built on via `busctl --user introspect org.freedesktop.impl.portal.desktop.gtk
//! /org/freedesktop/portal/desktop org.freedesktop.impl.portal.FileChooser`
//! (`osssa{sv} -> ua{sv}` for all three methods) -- not assumed from memory.
//! The backend method itself blocks (asynchronously) until the picker
//! window resolves or is cancelled, then returns the result as the direct
//! method reply; no separate `Request`/`Response` signal dance is needed
//! on our side for that. One spec detail this v1 does *not* implement:
//! registering a `org.freedesktop.impl.portal.Request` object at `handle`
//! so an external caller could cancel us via `Close()` -- the picker
//! window's own Cancel button is the only way to abort for now.

use crate::errmap::{LockExt, ToStringErr};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::oneshot;
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

pub type PickerMap = Arc<Mutex<HashMap<String, oneshot::Sender<Vec<String>>>>>;

#[derive(Default)]
pub struct PortalState {
    pub pending: PickerMap,
    /// Keeping the `Connection` alive for the app's lifetime is what keeps
    /// the bus name owned and the service serving requests -- dropping it
    /// would immediately stop both.
    pub connection: Mutex<Option<zbus::Connection>>,
}

/// Minimal percent-encoder for stuffing a filename into the picker window's
/// URL query string -- pulling in a whole crate for this one call site
/// isn't worth it.
pub(crate) fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[derive(serde::Serialize)]
struct FilterGroup {
    name: String,
    patterns: Vec<String>,
}

/// Pulls the portal spec's `filters` option apart: `a(sa(us))` -- a list of
/// (display name, [(rule type, glob pattern)]) pairs, e.g. the caller
/// offering `[("Rich Text Document", [(0, "*.rtf")]), ("Plain Text", [(0,
/// "*.txt")])]`. Rule type `0` is a glob pattern; `1` is a mimetype, which
/// we skip since resolving a mimetype back to a display extension would
/// need a whole mime database this app doesn't otherwise carry.
fn parse_filters(v: &OwnedValue) -> Vec<FilterGroup> {
    let mut out = Vec::new();
    let Value::Array(items) = &**v else { return out };
    for item in items.iter() {
        let Value::Structure(s) = item else { continue };
        let fields = s.fields();
        if fields.len() != 2 {
            continue;
        }
        let Ok(name) = String::try_from(fields[0].clone()) else { continue };
        let mut patterns = Vec::new();
        if let Value::Array(rules) = &fields[1] {
            for rule in rules.iter() {
                let Value::Structure(rs) = rule else { continue };
                let rf = rs.fields();
                if rf.len() != 2 {
                    continue;
                }
                let (Ok(kind), Ok(pattern)) = (u32::try_from(rf[0].clone()), String::try_from(rf[1].clone())) else {
                    continue;
                };
                if kind == 0 {
                    patterns.push(pattern);
                }
            }
        }
        if !patterns.is_empty() {
            out.push(FilterGroup { name, patterns });
        }
    }
    out
}

/// The portal spec's `current_folder` option is a NUL-terminated byte
/// string (`ay`), not UTF-8 text -- callers that set it (mostly non-browser
/// apps; browsers usually just rely on the chooser's own last-used folder)
/// hand us a real filesystem path this way instead of through `Value::Str`.
fn parse_current_folder(v: &OwnedValue) -> Option<String> {
    let Value::Array(bytes) = &**v else { return None };
    let mut buf = Vec::with_capacity(bytes.len());
    for b in bytes.iter() {
        buf.push(u8::try_from(b.clone()).ok()?);
    }
    while buf.last() == Some(&0) {
        buf.pop();
    }
    String::from_utf8(buf).ok().filter(|s| !s.is_empty())
}

fn new_request_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Center of the window that asked for the picker, in root-window (physical)
/// pixels. `parent_window` is the portal spec's handle for the requesting
/// toplevel: `"x11:0x<xid>"` on X11, and an opaque compositor token on
/// Wayland (where no client may position its own windows anyway, so there's
/// nothing to do with it). Read straight off the X server -- GDK's
/// foreign-window API would mean pulling in the gdk-x11 backend crate, and
/// this is two round-trips of plain protocol.
fn x11_parent_center(parent_window: &str) -> Option<(f64, f64)> {
    use x11rb::protocol::xproto::ConnectionExt;
    let hex = parent_window.trim().strip_prefix("x11:")?.trim_start_matches("0x");
    let win = u32::from_str_radix(hex, 16).ok()?;
    let (conn, _screen) = x11rb::connect(None).ok()?;
    let geom = conn.get_geometry(win).ok()?.reply().ok()?;
    // A reparented toplevel's own x/y are relative to the frame the window
    // manager wrapped it in, not the root, so translate its origin instead
    // of trusting them.
    let origin = conn.translate_coordinates(win, geom.root, 0, 0).ok()?.reply().ok()?;
    Some((
        f64::from(origin.dst_x) + f64::from(geom.width) / 2.0,
        f64::from(origin.dst_y) + f64::from(geom.height) / 2.0,
    ))
}

/// Top-left (logical, matching the builder's units) that puts a `w`x`h`
/// picker in the middle of the monitor the *requesting app* is on.
///
/// `.center()` can't do this: it centers on whichever monitor the new window
/// happened to be created on -- the primary, in practice -- which is why a
/// Save As triggered from a browser on the second screen opened the panel
/// over on the first one. Applies to Open/Open Folder too; they're all this
/// one code path.
///
/// Must not be called from the main thread: the monitor/cursor queries below
/// are message-passed to the event loop and block on its reply.
fn picker_position(app: &AppHandle, parent_window: &str, w: f64, h: f64) -> Option<(f64, f64)> {
    // Prefer the requesting window's own center. It's the only signal that
    // stays right when the picker is triggered from the keyboard (Ctrl+S)
    // with the pointer parked on another screen. The cursor is the fallback:
    // still correct for the click-driven case, and it's all there is when the
    // caller sends no usable parent handle (an empty string is legal, and
    // some apps do exactly that).
    let (px, py) = x11_parent_center(parent_window)
        .or_else(|| app.cursor_position().ok().map(|p| (p.x, p.y)))?;
    let monitor = app
        .monitor_from_point(px, py)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())?;
    // Monitor geometry is physical; the window builder's position is logical.
    let scale = monitor.scale_factor();
    let pos = monitor.position();
    let size = monitor.size();
    let x = f64::from(pos.x) / scale + (f64::from(size.width) / scale - w) / 2.0;
    let y = f64::from(pos.y) / scale + (f64::from(size.height) / scale - h) / 2.0;
    Some((x, y))
}

async fn run_picker(
    app: &AppHandle,
    pending: &PickerMap,
    parent_window: &str,
    mode: &'static str,
    multiple: bool,
    suggested_name: Option<&str>,
    filters: &[FilterGroup],
    initial_folder: Option<&str>,
    directory: bool,
) -> Vec<String> {
    let id = new_request_id();
    let (tx, rx) = oneshot::channel();
    pending.lock_safe().insert(id.clone(), tx);

    let label = format!("picker-{id}");
    let mut url = format!("index.html?picker={mode}&reqid={id}&multiple={multiple}");
    if directory {
        url.push_str("&directory=true");
    }
    if let Some(name) = suggested_name {
        url.push_str("&name=");
        url.push_str(&url_encode(name));
    }
    if !filters.is_empty() {
        if let Ok(json) = serde_json::to_string(filters) {
            url.push_str("&filters=");
            url.push_str(&url_encode(&json));
        }
    }
    if let Some(folder) = initial_folder {
        url.push_str("&folder=");
        url.push_str(&url_encode(folder));
    }
    let app = app.clone();
    let label_for_build = label.clone();
    let parent_window = parent_window.to_string();
    let title = if mode == "save" { "Save" } else { "Open" };
    // The Save panel starts collapsed (PickerView's own `expanded` state
    // defaults to false for "save"), so it should *start* at that small
    // size directly -- creating it at the expanded size and having React
    // shrink it down a moment after mount was the "opens big, then
    // suddenly shrinks" flash. Open has no collapsed state, always expanded.
    let (w, h) = if mode == "save" { (420.0, 230.0) } else { (760.0, 560.0) };
    // WebviewWindowBuilder must run on the main event loop; AppHandle-based
    // window creation is Tauri's documented thread-safe entry point for
    // exactly this (called here from a tokio task driven by the D-Bus
    // dispatch, not the main thread).
    let build_result = tauri::async_runtime::spawn_blocking(move || {
        let mut builder = WebviewWindowBuilder::new(&app, &label_for_build, WebviewUrl::App(url.into()))
            .title(title)
            .inner_size(w, h);
        // Falls back to `.center()` when the requesting monitor can't be
        // worked out at all (no parent handle, no cursor, no monitor list) --
        // same behaviour as before, just no longer the only behaviour.
        builder = match picker_position(&app, &parent_window, w, h) {
            Some((x, y)) => builder.position(x, y),
            None => builder.center(),
        };
        let result = builder
            // Matches the main window's own decorations:false + transparent:true
            // (see tauri.conf.json) -- without this the picker got a plain
            // opaque-white GTK surface: a visible white flash before the
            // webview's first paint, and a washed-out backdrop behind the
            // frosted-glass `.context-menu` (its `backdrop-filter` blur had
            // nothing translucent to blur, so it just rendered flat and
            // light regardless of the app's actual color scheme). The CSS
            // side (`html, body, #root { background: transparent }`) already
            // assumed this and was silently depending on it everywhere else
            // in the app -- this window just never got the matching config.
            .decorations(false)
            .transparent(true)
            .background_color(tauri::window::Color(0, 0, 0, 0))
            .focused(true)
            .build();
        if let Ok(window) = &result {
            // `.focused(true)` above only sets the *initial* GTK show hint --
            // the window manager still applies its own focus-stealing
            // prevention since this fires from a background D-Bus dispatch,
            // not a direct user click, which is why it was landing as a
            // "wants attention" notification instead of actually raising.
            // An explicit present/activate call after the window exists
            // (gtk_window_present under the hood) asks again as a real
            // window operation, which the WM is far more willing to honor.
            let _ = window.set_focus();
            let _ = window.request_user_attention(Some(tauri::UserAttentionType::Critical));
            // Some window managers still leave it behind the requesting
            // app ("sometimes it doesn't focus"). Briefly pinning it on
            // top forces the raise, and it is dropped again a moment later
            // so the picker doesn't sit over everything while in use.
            let _ = window.set_always_on_top(true);
            let top_window = window.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(700));
                let _ = top_window.set_always_on_top(false);
            });
        }
        result
    })
    .await;
    match build_result {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            eprintln!("portal: picker window build failed: {e}");
            pending.lock_safe().remove(&id);
            return Vec::new();
        }
        Err(e) => {
            eprintln!("portal: picker window build task panicked/cancelled: {e}");
            pending.lock_safe().remove(&id);
            return Vec::new();
        }
    }

    rx.await.unwrap_or_default()
}

fn uris_result(uris: Vec<String>) -> (u32, HashMap<String, OwnedValue>) {
    if uris.is_empty() {
        return (1, HashMap::new()); // 1 == cancelled, per the portal spec
    }
    let mut results = HashMap::new();
    if let Ok(v) = OwnedValue::try_from(Value::new(uris)) {
        results.insert("uris".to_string(), v);
    }
    (0, results)
}

pub struct FileChooserIface {
    pub app: AppHandle,
    pub pending: PickerMap,
}

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooserIface {
    async fn open_file(
        &self,
        _handle: ObjectPath<'_>,
        _app_id: String,
        parent_window: String,
        _title: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let multiple = options
            .get("multiple")
            .and_then(|v| bool::try_from(v.clone()).ok())
            .unwrap_or(false);
        let folder = options.get("current_folder").and_then(parse_current_folder);
        // The FileChooser "Open Folder" request is a plain OpenFile with the
        // `directory` boolean option set -- without reading it here every
        // folder-open arrives indistinguishable from a file-open, so the
        // picker rendered folders uncapped and its confirm handler stripped
        // directories out of the result (a dead "Open" button).
        let directory = options
            .get("directory")
            .and_then(|v| bool::try_from(v.clone()).ok())
            .unwrap_or(false);
        let uris = run_picker(&self.app, &self.pending, &parent_window, "open", multiple, None, &[], folder.as_deref(), directory).await;
        uris_result(uris)
    }

    async fn save_file(
        &self,
        _handle: ObjectPath<'_>,
        _app_id: String,
        parent_window: String,
        _title: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        // `current_name` is the portal spec's hint for the caller's suggested
        // filename (e.g. a browser's real download name) -- without reading
        // it the picker had no way to default to anything but a hardcoded
        // placeholder.
        let name = options
            .get("current_name")
            .and_then(|v| String::try_from(v.clone()).ok());
        let filters = options.get("filters").map(parse_filters).unwrap_or_default();
        let folder = options.get("current_folder").and_then(parse_current_folder);
        let uris = run_picker(&self.app, &self.pending, &parent_window, "save", false, name.as_deref(), &filters, folder.as_deref(), false).await;
        uris_result(uris)
    }

    async fn save_files(
        &self,
        _handle: ObjectPath<'_>,
        _app_id: String,
        parent_window: String,
        _title: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let name = options
            .get("current_name")
            .and_then(|v| String::try_from(v.clone()).ok());
        let filters = options.get("filters").map(parse_filters).unwrap_or_default();
        let folder = options.get("current_folder").and_then(parse_current_folder);
        let uris = run_picker(&self.app, &self.pending, &parent_window, "save", true, name.as_deref(), &filters, folder.as_deref(), false).await;
        uris_result(uris)
    }
}

const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.vaultexplorer";

fn service_file_path() -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}/.local/share/dbus-1/services", crate::home_dir()))
        .join(format!("{BUS_NAME}.service"))
}
fn portal_file_path() -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "{}/.local/share/xdg-desktop-portal/portals",
        crate::home_dir()
    ))
    .join("vaultexplorer.portal")
}
fn portals_conf_path() -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}/.config/xdg-desktop-portal", crate::home_dir()))
        .join("portals.conf")
}

const PREFERRED_LINE: &str = "org.freedesktop.impl.portal.FileChooser=vaultexplorer";

const SYSTEM_PORTAL_FILE: &str = "/usr/share/xdg-desktop-portal/portals/vaultexplorer.portal";

/// Installs the `.portal` file where `xdg-desktop-portal` actually looks
/// for it. Confirmed via its own `--verbose` startup log ("load portals
/// from /usr/share/xdg-desktop-portal/portals") that on this system/
/// version it only scans that single, hardcoded system directory --
/// `XDG_DATA_HOME`/`XDG_DATA_DIRS` are ignored entirely for this specific
/// purpose (tested directly: setting `XDG_DATA_DIRS` to include this
/// user's own data dir before starting it made no difference). So the
/// per-user copy `write_registration` also writes is otherwise silently
/// never loaded, no matter how `portals.conf` is configured -- this is
/// the one piece of this feature that genuinely needs elevated
/// privileges. `pkexec` pops the same graphical polkit prompt already
/// used by "Update Drivers": a one-time admin approval, not a standing
/// elevated process.
fn install_system_portal_file(contents: &str) -> Result<(), String> {
    // Skip the pkexec prompt entirely when the system file already has
    // this exact content -- without this check, the self-heal-on-every-
    // launch logic in lib.rs (and every D-Bus-activated Save/Open dialog,
    // since that's a fresh launch too) re-triggered the graphical polkit
    // prompt every single time, not just the one time it's actually
    // needed (first install, or content drift).
    if std::fs::read_to_string(SYSTEM_PORTAL_FILE).ok().as_deref() == Some(contents) {
        return Ok(());
    }
    let tmp = std::env::temp_dir().join("vaultexplorer-system.portal");
    std::fs::write(&tmp, contents).str_err()?;
    let output = std::process::Command::new("pkexec")
        .args(["install", "-m", "644", tmp.to_str().unwrap(), SYSTEM_PORTAL_FILE])
        .output()
        .str_err();
    let _ = std::fs::remove_file(&tmp);
    let output = output?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

fn uninstall_system_portal_file() -> Result<(), String> {
    if !std::path::Path::new(SYSTEM_PORTAL_FILE).exists() {
        return Ok(());
    }
    let output = std::process::Command::new("pkexec")
        .args(["rm", "-f", SYSTEM_PORTAL_FILE])
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

/// Whether we're currently registered as the preferred FileChooser
/// backend -- just checks for our line in the user's `portals.conf`.
pub fn is_enabled() -> bool {
    std::fs::read_to_string(portals_conf_path())
        .map(|s| s.contains(PREFERRED_LINE))
        .unwrap_or(false)
}

/// Write the registration files (service activation, portal declaration,
/// and a `[preferred]` line scoped to *only* the FileChooser interface --
/// every other interface is left to whatever the user already had
/// configured, or the desktop default). Everything except the `.portal`
/// file itself lives under the user's own `~/.local/share`/`~/.config` --
/// that one needs a one-time `pkexec`-elevated copy into
/// `/usr/share/xdg-desktop-portal/portals/` too, since that's the only
/// place this system's `xdg-desktop-portal` actually looks for portal
/// implementations (see `install_system_portal_file`'s doc comment).
pub fn write_registration(exe_path: &str) -> Result<(), String> {
    let svc_dir = service_file_path();
    std::fs::create_dir_all(svc_dir.parent().unwrap()).str_err()?;
    // D-Bus activation just launches the normal app binary -- there's no
    // separate headless mode; `run()`'s own startup checks `is_enabled()`
    // and starts serving if so, so an activation-triggered launch ends up
    // registering the service the same way an already-running instance
    // that gets toggled on does.
    // `--portal-activated` is our own marker, not a real Tauri/CLI flag --
    // `run()` in lib.rs checks argv for it on startup to tell "D-Bus spun me
    // up solely to service this FileChooser request" apart from "the user
    // actually launched the app", and skips showing the main Explorer
    // window in the former case. Without it, a Save/Open dialog triggered
    // while VaultExplorer wasn't already running popped the *entire* app
    // window alongside the picker.
    std::fs::write(
        &svc_dir,
        format!("[D-BUS Service]\nName={BUS_NAME}\nExec={exe_path} --portal-activated\n"),
    )
    .str_err()?;

    let portal_contents =
        format!("[portal]\nDBusName={BUS_NAME}\nInterfaces=org.freedesktop.impl.portal.FileChooser\nUseIn=gnome;\n");
    let portal_dir = portal_file_path();
    std::fs::create_dir_all(portal_dir.parent().unwrap()).str_err()?;
    std::fs::write(&portal_dir, &portal_contents).str_err()?;
    install_system_portal_file(&portal_contents)?;

    let conf_path = portals_conf_path();
    std::fs::create_dir_all(conf_path.parent().unwrap()).str_err()?;
    let existing = std::fs::read_to_string(&conf_path).unwrap_or_default();
    if !existing.contains(PREFERRED_LINE) {
        let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
        if let Some(idx) = lines.iter().position(|l| l.trim() == "[preferred]") {
            lines.insert(idx + 1, PREFERRED_LINE.to_string());
        } else {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push("[preferred]".to_string());
            lines.push(PREFERRED_LINE.to_string());
        }
        std::fs::write(&conf_path, lines.join("\n") + "\n").str_err()?;
    }
    Ok(())
}

/// Remove only our own lines/files -- never touches any other section or
/// backend's registration. The system-wide `.portal` copy also needs
/// another one-time `pkexec` prompt to remove (uid-mismatched with this
/// process, so a plain unprivileged `remove_file` can't touch it);
/// that failing shouldn't block cleaning up everything else this app
/// actually owns without elevated rights.
pub fn remove_registration() -> Result<(), String> {
    let _ = std::fs::remove_file(service_file_path());
    let _ = std::fs::remove_file(portal_file_path());
    if let Err(e) = uninstall_system_portal_file() {
        eprintln!("portal: couldn't remove the system-wide .portal file: {e}");
    }
    let conf_path = portals_conf_path();
    if let Ok(existing) = std::fs::read_to_string(&conf_path) {
        let cleaned: Vec<&str> = existing.lines().filter(|l| l.trim() != PREFERRED_LINE).collect();
        std::fs::write(&conf_path, cleaned.join("\n") + "\n").str_err()?;
    }
    Ok(())
}

/// Start owning the FileChooser backend bus name and return the live
/// `Connection` -- the caller must keep it (see `PortalState::connection`)
/// for as long as the service should keep serving; dropping it stops it.
/// Disabling later (`remove_registration`) does not stop this connection
/// -- it only removes the preference, so nothing routes to us anymore;
/// the name is simply dropped on app exit.
pub async fn start_service(app: AppHandle, pending: PickerMap) -> Result<zbus::Connection, String> {
    let iface = FileChooserIface { app, pending };
    zbus::connection::Builder::session()
        .str_err()?
        .name(BUS_NAME)
        .str_err()?
        .serve_at("/org/freedesktop/portal/desktop", iface)
        .str_err()?
        .build()
        .await
        .str_err()
}

// ---- Tauri commands ----

#[tauri::command]
pub fn portal_is_enabled() -> bool {
    is_enabled()
}

/// Turn on the "system file picker" integration: write the (per-user,
/// no-root) registration files and start serving the FileChooser backend
/// for the rest of this session. See the module doc comment for the real,
/// narrower scope of what this actually affects.
///
/// The registration files alone aren't enough for this to take effect:
/// `xdg-desktop-portal` (the actual dispatcher every app's file dialog
/// talks to) only reads `portals.conf` and enumerates `.portal` files at
/// its *own* startup -- writing them after it's already been running
/// (e.g. since login) leaves it oblivious until it happens to restart on
/// its own. Restarting the user-scoped systemd unit here is what makes
/// toggling this on actually take effect immediately, instead of
/// silently doing nothing until the next logout/login -- confirmed via
/// `systemctl --user status xdg-desktop-portal.service` showing it can
/// run for days uninterrupted otherwise.
/// Registering the backend only decides *which* portal serves a request --
/// it does not make apps ask a portal in the first place. Outside a
/// sandbox, GTK and Qt show their own built-in dialogs unless told
/// otherwise, which is why "Save as" in ordinary apps kept bypassing this
/// one. These two variables are the documented switches for that, and
/// `environment.d` is the session-wide place for them.
///
/// Written only by the explicit "use VaultExplorer as the system file
/// picker" toggle, and removed when it is turned off. Takes effect on the
/// next login, since that is when the session reads environment.d.
fn env_file_path() -> std::path::PathBuf {
    std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".config/environment.d/95-vaultexplorer-portal.conf")
}

/// Pushes the same variables into the *running* session, so apps launched
/// from now on pick them up without logging out. environment.d is only
/// read at login, which is why enabling the setting appeared to do nothing
/// until the next session -- the complaint that other programs still
/// ignored this picker.
fn apply_env_to_session() {
    for pair in ["GTK_USE_PORTAL=1", "QT_QPA_PLATFORMTHEME=xdgdesktopportal"] {
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "set-environment", pair])
            .status();
        // systemd's copy covers units; D-Bus activation has its own,
        // and that is the one desktop apps are usually started from.
        let _ = std::process::Command::new("dbus-update-activation-environment")
            .args(["--systemd", pair])
            .status();
    }
}

fn write_env_file() -> Result<(), String> {
    let path = env_file_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).str_err()?;
    }
    std::fs::write(
        &path,
        "# Written by VaultExplorer's \"system file picker\" setting.\n\
         # Makes GTK and Qt apps ask the desktop portal for file dialogs\n\
         # instead of drawing their own. Delete this file (or turn the\n\
         # setting off) to go back to the toolkit dialogs.\n\
         GTK_USE_PORTAL=1\n\
         QT_QPA_PLATFORMTHEME=xdgdesktopportal\n",
    )
    .str_err()
}

#[tauri::command]
pub async fn portal_enable(
    app: tauri::AppHandle,
    state: tauri::State<'_, PortalState>,
    fm_state: tauri::State<'_, crate::filemanager1::FileManagerState>,
) -> Result<(), String> {
    let exe = std::env::current_exe()
        .str_err()?
        .to_string_lossy()
        .to_string();
    write_registration(&exe)?;
    let already_running = state.connection.lock_safe().is_some();
    if !already_running {
        let pending = state.pending.clone();
        let conn = start_service(app.clone(), pending).await?;
        *state.connection.lock_safe() = Some(conn);
    }
    // Same toggle also claims org.freedesktop.FileManager1 (see
    // filemanager1.rs) -- one "make VaultExplorer the system's file
    // integration point" switch, not two.
    crate::filemanager1::write_registration(&exe)?;
    let fm_already_running = fm_state.connection.lock_safe().is_some();
    if !fm_already_running {
        let conn = crate::filemanager1::start_service(app).await?;
        *fm_state.connection.lock_safe() = Some(conn);
    }
    // Best-effort: a failure here only means GTK/Qt apps keep using their
    // own dialogs, which is exactly the state this toggle was already in.
    let _ = write_env_file();
    apply_env_to_session();
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "restart", "xdg-desktop-portal.service"])
        .status();
    Ok(())
}

/// Remove the registration files. Does not stop an already-started
/// connection this session (see the module doc comment) -- nothing routes
/// to us anymore once the preference is gone, so that's harmless.
/// Called at startup when the portal backend is already registered: the
/// env file was previously written only by the toggle, so anyone who had
/// enabled the picker *before* that existed never got it -- their file
/// simply wasn't there, and every other app kept using its own dialog.
pub fn ensure_env_registered() {
    if !service_file_path().exists() {
        return;
    }
    if !env_file_path().exists() {
        let _ = write_env_file();
    }
    apply_env_to_session();
}

#[tauri::command]
pub fn portal_disable() -> Result<(), String> {
    crate::filemanager1::remove_registration();
    let _ = std::fs::remove_file(env_file_path());
    remove_registration()
}

#[tauri::command]
pub fn portal_resolve(state: tauri::State<PortalState>, request_id: String, uris: Vec<String>) -> Result<(), String> {
    if let Some(tx) = state.pending.lock_safe().remove(&request_id) {
        let _ = tx.send(uris);
    }
    Ok(())
}

#[tauri::command]
pub fn portal_cancel(state: tauri::State<PortalState>, request_id: String) -> Result<(), String> {
    if let Some(tx) = state.pending.lock_safe().remove(&request_id) {
        let _ = tx.send(Vec::new());
    }
    Ok(())
}
