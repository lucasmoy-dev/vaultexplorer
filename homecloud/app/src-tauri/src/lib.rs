//! The desktop shell.
//!
//! Everything interesting lives in `homecore`; this file starts the engine when
//! the window opens, stops it when the window closes, and exposes the handful of
//! commands the interface calls. Errors come back as plain sentences, because
//! every one of them is going to be shown to a person.

use std::path::{Path, PathBuf};

use homecore::destination::{self, Pick};
use homecore::link::{link_binary, Links};
use homecore::model::{Invitation, Settings, SharedFolder, ThisDevice};
use homecore::supervisor::{engine_binary, Engine};
use homecore::PairingCode;
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, State};
use tokio::sync::RwLock;

struct AppState {
    engine: RwLock<Option<Engine>>,
    /// Why the engine is not running, already phrased for a person. Without
    /// this a failed launch is indistinguishable from a slow one, and the
    /// window sits on "Arrancando…" forever.
    startup_problem: RwLock<Option<String>>,
    /// Where folders joined from a code are put unless the user picks elsewhere.
    default_root: PathBuf,
    /// Kept so a retry can look for the engine again without rediscovering it.
    resource_dir: Option<PathBuf>,
    engine_home: PathBuf,
    /// This user's home, for guessing a destination that already exists.
    home_dir: PathBuf,
    /// Folders being served as public links right now.
    links: Links,
}

impl AppState {
    /// Where to put a folder called `label`, unless the user picks elsewhere.
    ///
    /// A folder arriving from another device usually has a counterpart here
    /// already — that is why the two are being paired. Proposing a fresh
    /// directory under `~/HomeCloud` when `~/Documents/cloud` is sitting right
    /// there is how someone ends up syncing an empty folder and believing it
    /// failed, so anything that exists wins over anything invented.
    fn suggest_for(&self, label: &str) -> PathBuf {
        let candidates = [
            self.home_dir.join(label),
            self.home_dir.join("Documents").join(label),
            self.home_dir.join("Documentos").join(label),
        ];
        candidates
            .into_iter()
            .find(|path| path.is_dir())
            .unwrap_or_else(|| self.default_root.join(label))
    }
}

/// Starts the engine and records either it or the reason it would not start.
async fn launch_engine(state: &AppState) {
    *state.startup_problem.write().await = None;

    let binary = match engine_binary(state.resource_dir.as_deref()) {
        Ok(path) => path,
        Err(e) => {
            *state.startup_problem.write().await = Some(plain(e));
            return;
        }
    };

    match Engine::start(&binary, &state.engine_home).await {
        Ok(engine) => {
            // A device with no name shows up on other people's screens as a
            // meaningless ID, so give it one on first run.
            let _ = engine.client.ensure_device_name(&default_device_name()).await;
            engine.client.set_auto_accept_root(state.default_root.clone());
            engine.client.set_preferences_path(state.engine_home.join("homecloud.json"));
            *state.engine.write().await = Some(engine);
        }
        Err(e) => *state.startup_problem.write().await = Some(plain(e)),
    }
}

/// Commands hand the interface a sentence, never a stack trace.
type UiResult<T> = Result<T, String>;

fn plain(err: impl std::fmt::Display) -> String {
    err.to_string()
}

macro_rules! with_engine {
    ($state:expr, |$client:ident| $body:expr) => {{
        let guard = $state.engine.read().await;
        let engine = guard
            .as_ref()
            .ok_or_else(|| "the sync engine is still starting up".to_string())?;
        let $client = &engine.client;
        $body.await.map_err(plain)
    }};
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Readiness {
    ready: bool,
    device: Option<ThisDevice>,
    /// Set when startup failed, already phrased for the user.
    problem: Option<String>,
}

#[tauri::command]
async fn readiness(state: State<'_, AppState>) -> UiResult<Readiness> {
    let guard = state.engine.read().await;
    let Some(engine) = guard.as_ref() else {
        return Ok(Readiness {
            ready: false,
            device: None,
            problem: state.startup_problem.read().await.clone(),
        });
    };
    match engine.client.this_device().await {
        Ok(device) => Ok(Readiness { ready: true, device: Some(device), problem: None }),
        Err(e) => Ok(Readiness { ready: false, device: None, problem: Some(plain(e)) }),
    }
}

/// Tries again after a failed launch, so a transient cause (another copy that
/// has since closed, a machine still waking up) does not need the app restarted.
#[tauri::command]
async fn retry_engine(state: State<'_, AppState>) -> UiResult<()> {
    if let Some(mut engine) = state.engine.write().await.take() {
        let _ = engine.stop().await;
    }
    launch_engine(&state).await;
    Ok(())
}

#[tauri::command]
async fn list_folders(state: State<'_, AppState>) -> UiResult<Vec<SharedFolder>> {
    with_engine!(state, |client| client.folders())
}

#[tauri::command]
async fn list_invitations(state: State<'_, AppState>) -> UiResult<Vec<Invitation>> {
    with_engine!(state, |client| client.invitations())
}

#[tauri::command]
async fn share_folder(state: State<'_, AppState>, path: String, label: String) -> UiResult<String> {
    let guard = state.engine.read().await;
    let engine = guard.as_ref().ok_or("the sync engine is still starting up")?;
    let code = engine.client.share_folder(&path, &label).await.map_err(plain)?;
    code.encode().map_err(plain)
}

#[tauri::command]
async fn code_for(state: State<'_, AppState>, folder_id: String) -> UiResult<String> {
    with_engine!(state, |client| client.code_text_for(&folder_id))
}

// ---- public links ---------------------------------------------------------

/// Whether this device has joined a zrok account, which is what a link needs.
#[tauri::command]
async fn link_ready(state: State<'_, AppState>) -> UiResult<bool> {
    Ok(state.links.is_ready().await)
}

/// Joins the zrok account a token belongs to. Once per device.
#[tauri::command]
async fn link_join(state: State<'_, AppState>, token: String) -> UiResult<()> {
    state.links.join(&token).await.map_err(plain)
}

/// Starts serving a folder and returns the address to hand out. `password`
/// empty means a link with no password at all, which is said on screen.
#[tauri::command]
async fn link_start(
    state: State<'_, AppState>,
    folder_id: String,
    path: String,
    password: String,
) -> UiResult<String> {
    let auth = if password.trim().is_empty() {
        String::new()
    } else {
        format!("familia:{}", password.trim())
    };
    state
        .links
        .start(&folder_id, Path::new(&path), &auth)
        .await
        .map_err(plain)
}

#[tauri::command]
async fn link_stop(state: State<'_, AppState>, folder_id: String) -> UiResult<()> {
    state.links.stop(&folder_id).await.map_err(plain)
}

/// The address a folder is being served at, or nothing when it is not.
#[tauri::command]
async fn link_for(state: State<'_, AppState>, folder_id: String) -> UiResult<Option<String>> {
    Ok(state.links.url_for(&folder_id))
}

/// Puts a password on a folder, or takes it off when `password` is empty.
#[tauri::command]
async fn set_folder_password(
    state: State<'_, AppState>,
    folder_id: String,
    password: String,
) -> UiResult<()> {
    let guard = state.engine.read().await;
    let engine = guard.as_ref().ok_or("the sync engine is still starting up")?;
    if password.trim().is_empty() {
        engine.client.clear_folder_password(&folder_id).await.map_err(plain)
    } else {
        engine.client.set_folder_password(&folder_id, &password).await.map_err(plain)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CodePreview {
    device_name: String,
    folder_label: String,
    /// Where this device would put the folder, unless the user says otherwise.
    suggested_path: String,
    /// What the folder holds on the other device. `None` from an older one.
    bytes: Option<u64>,
}

/// Reads a pasted code without acting on it, so the interface can ask "accept
/// Fotos from Portátil de Lucas?" before anything is written to disk.
#[tauri::command]
async fn preview_code(
    state: State<'_, AppState>,
    code: String,
    password: Option<String>,
) -> UiResult<CodePreview> {
    let guard = state.engine.read().await;
    let engine = guard.as_ref().ok_or("the sync engine is still starting up")?;
    let parsed = engine
        .client
        .read_code(&code, password.as_deref())
        .await
        .map_err(plain)?;
    Ok(CodePreview {
        suggested_path: state.suggest_for(&parsed.folder_label).to_string_lossy().into_owned(),
        device_name: parsed.device_name,
        folder_label: parsed.folder_label,
        bytes: parsed.bytes,
    })
}

/// What choosing `chosen` would actually do, so the interface can say it before
/// the user commits. `pick` is `"inside"`, `"itself"`, or absent for the default.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Destination {
    path: String,
    pick: String,
    explanation: String,
    /// Room left on the disk this path lives on.
    free_bytes: Option<u64>,
}

#[tauri::command]
async fn resolve_destination(chosen: String, label: String, pick: Option<String>) -> UiResult<Destination> {
    let chosen = PathBuf::from(chosen);
    let pick = match pick.as_deref() {
        Some("itself") => Pick::Itself,
        Some("inside") => Pick::Inside,
        _ => destination::default_pick(&chosen, &label),
    };
    let path = destination::resolve(&chosen, &label, pick);
    Ok(Destination {
        free_bytes: homecore::disk::free_bytes(&path),
        path: path.to_string_lossy().into_owned(),
        pick: match pick {
            Pick::Itself => "itself".into(),
            Pick::Inside => "inside".into(),
        },
        explanation: destination::describe(&chosen, &label, pick),
    })
}

/// Holds a folder back while the connection is metered. Desktop machines are
/// rarely on a metered link, but the setting is shared so a folder configured
/// on the phone reads the same here.
#[tauri::command]
async fn set_folder_wifi_only(state: State<'_, AppState>, folder_id: String, wifi_only: bool) -> UiResult<()> {
    with_engine!(state, |client| client.set_folder_wifi_only(&folder_id, wifi_only))
}

/// Tears every connection down and dials again.
#[tauri::command]
async fn reconnect_all(state: State<'_, AppState>) -> UiResult<()> {
    with_engine!(state, |client| client.reconnect_all())
}

/// Makes a folder one that receives but never sends, or two-way again.
#[tauri::command]
async fn set_folder_read_only(state: State<'_, AppState>, folder_id: String, read_only: bool) -> UiResult<()> {
    with_engine!(state, |client| client.set_folder_read_only(&folder_id, read_only))
}

/// Drops devices that share nothing here any more — the identities left behind
/// by reinstalling the app on a phone.
#[tauri::command]
async fn forget_unused_devices(state: State<'_, AppState>) -> UiResult<Vec<String>> {
    with_engine!(state, |client| client.forget_unused_devices())
}

#[tauri::command]
async fn redeem_code(
    state: State<'_, AppState>,
    code: String,
    local_path: String,
    password: Option<String>,
) -> UiResult<()> {
    std::fs::create_dir_all(&local_path).map_err(|e| format!("could not create {local_path}: {e}"))?;
    let guard = state.engine.read().await;
    let engine = guard.as_ref().ok_or("the sync engine is still starting up")?;
    let parsed = engine
        .client
        .read_code(&code, password.as_deref())
        .await
        .map_err(plain)?;
    engine.client.redeem(&parsed, &local_path).await.map_err(plain)
}

#[tauri::command]
async fn suggested_path(state: State<'_, AppState>, label: String) -> UiResult<String> {
    Ok(state.suggest_for(&label).to_string_lossy().into_owned())
}

#[tauri::command]
async fn accept_invitation(
    state: State<'_, AppState>,
    invitation: Invitation,
    local_path: Option<String>,
) -> UiResult<()> {
    if let Some(path) = &local_path {
        std::fs::create_dir_all(path).map_err(|e| format!("could not create {path}: {e}"))?;
    }
    let guard = state.engine.read().await;
    let engine = guard.as_ref().ok_or("the sync engine is still starting up")?;
    engine.client.accept(&invitation, local_path.as_deref()).await.map_err(plain)
}

#[tauri::command]
async fn decline_invitation(state: State<'_, AppState>, invitation: Invitation) -> UiResult<()> {
    let guard = state.engine.read().await;
    let engine = guard.as_ref().ok_or("the sync engine is still starting up")?;
    engine.client.decline(&invitation).await.map_err(plain)
}

#[tauri::command]
async fn set_folder_paused(state: State<'_, AppState>, folder_id: String, paused: bool) -> UiResult<()> {
    with_engine!(state, |client| client.set_folder_paused(&folder_id, paused))
}

#[tauri::command]
async fn stop_sharing(state: State<'_, AppState>, folder_id: String) -> UiResult<()> {
    with_engine!(state, |client| client.stop_sharing(&folder_id))
}

#[tauri::command]
async fn settings(state: State<'_, AppState>) -> UiResult<Settings> {
    with_engine!(state, |client| client.settings())
}

#[tauri::command]
async fn save_settings(state: State<'_, AppState>, settings: Settings) -> UiResult<()> {
    with_engine!(state, |client| client.save_settings(&settings))
}

/// What this machine calls itself before the user renames it.
fn default_device_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "Mi ordenador".to_string())
}

/// Lets the pairing screen open the camera to read a QR.
///
/// WebKitGTK says no twice: media streams are off in its settings, and every
/// request after that is refused unless the embedder answers it. Tauri does not,
/// so `getUserMedia` fails with a permission error and the scan button would be
/// a button that never works.
///
/// Only camera requests are granted. Anything else — a microphone, the screen —
/// is left to the default refusal, because nothing in this app has any business
/// asking for them.
#[cfg(target_os = "linux")]
fn enable_camera(window: &tauri::WebviewWindow) {
    use webkit2gtk::glib::Cast;
    use webkit2gtk::{
        PermissionRequestExt, SettingsExt, UserMediaPermissionRequest,
        UserMediaPermissionRequestExt, WebViewExt,
    };

    let applied = window.with_webview(|webview| {
        let view = webview.inner();
        match WebViewExt::settings(&view) {
            Some(settings) => {
                settings.set_enable_media_stream(true);
                eprintln!("homecloud: camera enabled in the webview settings");
            }
            None => eprintln!("homecloud: the webview exposed no settings; the camera will not open"),
        }
        view.connect_permission_request(|_, request| {
            let is_camera = request
                .downcast_ref::<UserMediaPermissionRequest>()
                .is_some_and(|media| media.is_for_video_device());
            if is_camera {
                request.allow();
            } else {
                request.deny();
            }
            true
        });
    });
    if let Err(e) = applied {
        eprintln!("homecloud: could not reach the webview to enable the camera: {e}");
    }
}

/// Records what the camera did, or would not do.
///
/// A failure inside the webview reaches nobody: the app is normally launched
/// from a menu, so stderr goes nowhere a person will ever look. Writing it
/// beside the app's own data means the answer survives closing the window and
/// can be read afterwards.
#[tauri::command]
fn report_camera_problem(state: State<'_, AppState>, detail: String) {
    eprintln!("homecloud: camera: {detail}");
    let line = format!(
        "{}  {detail}\n",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default()
    );
    if let Some(parent) = state.engine_home.parent() {
        let _ = std::fs::create_dir_all(parent);
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(parent.join("camera.log"))
            .map(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()));
    }
}

#[cfg(not(target_os = "linux"))]
fn enable_camera(_window: &tauri::WebviewWindow) {}

/// Puts HomeCloud in the status bar and keeps it reachable from there.
///
/// Left click shows the window, right click opens a menu whose only
/// destructive item is Quit — so closing for good is deliberate, and closing
/// the window is not.
fn install_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Abrir HomeCloud", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Cerrar HomeCloud", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let Some(icon) = app.default_window_icon().cloned() else {
        // Without an icon there is nothing to click, and a tray entry nobody
        // can see would leave no way to reopen a hidden window.
        eprintln!("homecloud: no window icon in this build, so no tray icon");
        return Ok(());
    };

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("HomeCloud")
        .menu(&menu)
        // The menu must not also open on a left click, or showing the window
        // would need two gestures instead of one.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => reveal(app),
            "quit" => {
                // Closing the window is what stops the engine, and it is also
                // what lets the app exit: destroying it runs the same shutdown
                // as before the tray existed.
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.destroy();
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Turns starting with the session on, once.
///
/// On by default, because a sync app that waits to be opened is not syncing.
/// Only ever set the first time: after that it is the user's switch, and
/// re-enabling it behind their back on every launch would make the setting a
/// lie.
fn arrange_autostart(app: &tauri::AppHandle, data_dir: &Path) {
    use tauri_plugin_autostart::ManagerExt;

    let marker = data_dir.join("autostart-decided");
    if marker.exists() {
        return;
    }
    match app.autolaunch().enable() {
        Ok(()) => {
            let _ = std::fs::create_dir_all(data_dir);
            let _ = std::fs::write(&marker, "1");
        }
        Err(e) => eprintln!("homecloud: could not turn on start-with-session: {e}"),
    }
}

/// Whether HomeCloud starts with the session.
#[tauri::command]
fn autostart_enabled(app: tauri::AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> UiResult<()> {
    use tauri_plugin_autostart::ManagerExt;
    let outcome = if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    outcome.map_err(plain)
}

/// Downloads an update and hands back where it landed.
///
/// Installing it is left to the system's package installer, which is the only
/// thing that can ask for the password a `.deb` needs. Doing it here would mean
/// this app holding a root prompt, which it has no business doing.
#[tauri::command]
async fn download_update(app: tauri::AppHandle, url: String) -> UiResult<String> {
    // Only ever our own releases. A URL arriving from anywhere else would make
    // this a general-purpose downloader pointed at whatever asked.
    if !url.starts_with("https://github.com/lucasmoy-dev/") {
        return Err("esa descarga no viene de HomeCloud".into());
    }

    let name = url.rsplit('/').next().unwrap_or("homecloud.deb").to_string();
    let target = app
        .path()
        .download_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(name);

    let bytes = reqwest::get(&url)
        .await
        .map_err(|e| format!("no se pudo descargar: {e}"))?
        .error_for_status()
        .map_err(|e| format!("no se pudo descargar: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("la descarga se cortó: {e}"))?;

    std::fs::write(&target, &bytes)
        .map_err(|e| format!("no se pudo guardar en {}: {e}", target.display()))?;
    Ok(target.to_string_lossy().into_owned())
}

/// Asks the engine to look at a folder again, which is the way out of an index
/// that has drifted from what is on disk.
#[tauri::command]
async fn rescan(state: State<'_, AppState>, folder_id: String) -> UiResult<()> {
    with_engine!(state, |client| client.rescan(&folder_id))
}

/// Brings the window back from the tray, wherever it was left.
fn reveal(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        // Must be registered first. Two copies of the app would each start an
        // engine against the same database; the second one loses, dies, and
        // leaves the user watching a window that never finishes starting.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // Starts with the session, minimised to the tray. A folder that only
        // syncs while someone remembers to open a window is not synced.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            // Resolved here rather than in the core: only the toolkit knows
            // where this build's resources actually landed.
            let resource_dir = app.path().resource_dir().ok();
            let home_dir = app.path().home_dir().unwrap_or_else(|_| PathBuf::from("."));

            app.manage(AppState {
                engine: RwLock::new(None),
                startup_problem: RwLock::new(None),
                default_root: home_dir.join("HomeCloud"),
                home_dir: home_dir.clone(),
                links: Links::new(
                    link_binary(resource_dir.as_deref()).unwrap_or_else(|_| PathBuf::from("hcshare")),
                    data_dir.join("zrok"),
                ),
                resource_dir,
                engine_home: data_dir.join("engine"),
            });

            if let Some(window) = app.get_webview_window("main") {
                enable_camera(&window);
            }
            install_tray(app.handle())?;
            arrange_autostart(app.handle(), &data_dir);

            // The window is configured hidden so a login launch can stay in the
            // tray; every other launch is someone asking to see it.
            if !std::env::args().any(|arg| arg == "--hidden") {
                reveal(app.handle());
            }

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                launch_engine(&handle.state::<AppState>()).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                // Closing the window means "get out of my way", not "stop
                // syncing": a sync app that only works while a window is open
                // is one that quietly stops doing its job. The tray icon is
                // where it goes, and its menu is the only way out.
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // Reached only through the tray's Quit, which destroys the
                // window on purpose. The engine is a child process, so leaving
                // it running would strand it with nothing able to reach it.
                tauri::WindowEvent::Destroyed => {
                    let state = window.state::<AppState>();
                    tauri::async_runtime::block_on(async {
                        // Public links go down with the app. One left running
                        // after the window is gone is a tunnel nobody can see
                        // and nobody can close.
                        state.links.stop_all().await;
                        if let Some(mut engine) = state.engine.write().await.take() {
                            let _ = engine.stop().await;
                        }
                    });
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            readiness,
            list_folders,
            list_invitations,
            share_folder,
            code_for,
            preview_code,
            redeem_code,
            resolve_destination,
            set_folder_read_only,
            set_folder_wifi_only,
            set_folder_password,
            link_ready,
            link_join,
            link_start,
            link_stop,
            link_for,
            reconnect_all,
            rescan,
            download_update,
            autostart_enabled,
            set_autostart,
            forget_unused_devices,
            report_camera_problem,
            suggested_path,
            accept_invitation,
            decline_invitation,
            set_folder_paused,
            stop_sharing,
            settings,
            save_settings,
            retry_engine,
        ])
        .run(tauri::generate_context!())
        .expect("error while running HomeCloud");
}
