//! The desktop shell.
//!
//! Everything interesting lives in `homecore`; this file starts the engine when
//! the window opens, stops it when the window closes, and exposes the handful of
//! commands the interface calls. Errors come back as plain sentences, because
//! every one of them is going to be shown to a person.

use std::path::PathBuf;

use homecore::destination::{self, Pick};
use homecore::model::{Invitation, Settings, SharedFolder, ThisDevice};
use homecore::supervisor::{engine_binary, Engine};
use homecore::PairingCode;
use serde::Serialize;
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
    let guard = state.engine.read().await;
    let engine = guard.as_ref().ok_or("the sync engine is still starting up")?;
    let code = engine.client.code_for(&folder_id).await.map_err(plain)?;
    code.encode().map_err(plain)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CodePreview {
    device_name: String,
    folder_label: String,
    /// Where this device would put the folder, unless the user says otherwise.
    suggested_path: String,
}

/// Reads a pasted code without acting on it, so the interface can ask "accept
/// Fotos from Portátil de Lucas?" before anything is written to disk.
#[tauri::command]
async fn preview_code(state: State<'_, AppState>, code: String) -> UiResult<CodePreview> {
    let parsed = PairingCode::decode(&code).map_err(plain)?;
    Ok(CodePreview {
        suggested_path: state.suggest_for(&parsed.folder_label).to_string_lossy().into_owned(),
        device_name: parsed.device_name,
        folder_label: parsed.folder_label,
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
}

#[tauri::command]
async fn resolve_destination(chosen: String, label: String, pick: Option<String>) -> UiResult<Destination> {
    let chosen = PathBuf::from(chosen);
    let pick = match pick.as_deref() {
        Some("itself") => Pick::Itself,
        Some("inside") => Pick::Inside,
        _ => destination::default_pick(&chosen, &label),
    };
    Ok(Destination {
        path: destination::resolve(&chosen, &label, pick).to_string_lossy().into_owned(),
        pick: match pick {
            Pick::Itself => "itself".into(),
            Pick::Inside => "inside".into(),
        },
        explanation: destination::describe(&chosen, &label, pick),
    })
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
async fn redeem_code(state: State<'_, AppState>, code: String, local_path: String) -> UiResult<()> {
    let parsed = PairingCode::decode(&code).map_err(plain)?;
    std::fs::create_dir_all(&local_path).map_err(|e| format!("could not create {local_path}: {e}"))?;
    let guard = state.engine.read().await;
    let engine = guard.as_ref().ok_or("the sync engine is still starting up")?;
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
                resource_dir,
                engine_home: data_dir.join("engine"),
            });

            if let Some(window) = app.get_webview_window("main") {
                enable_camera(&window);
            }

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                launch_engine(&handle.state::<AppState>()).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // The engine is a child process; leaving it running after the last
            // window closes would strand it with no way to reach it.
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.state::<AppState>();
                tauri::async_runtime::block_on(async {
                    if let Some(mut engine) = state.engine.write().await.take() {
                        let _ = engine.stop().await;
                    }
                });
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
