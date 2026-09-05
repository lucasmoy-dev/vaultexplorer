//! The desktop shell.
//!
//! Everything interesting lives in `homecore`; this file starts the engine when
//! the window opens, stops it when the window closes, and exposes the handful of
//! commands the interface calls. Errors come back as plain sentences, because
//! every one of them is going to be shown to a person.

use std::path::PathBuf;

use homecore::model::{Invitation, Settings, SharedFolder, ThisDevice};
use homecore::supervisor::{engine_binary, Engine};
use homecore::PairingCode;
use serde::Serialize;
use tauri::{Manager, State};
use tokio::sync::RwLock;

struct AppState {
    engine: RwLock<Option<Engine>>,
    /// Where folders joined from a code are put unless the user picks elsewhere.
    default_root: PathBuf,
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
        return Ok(Readiness { ready: false, device: None, problem: None });
    };
    match engine.client.this_device().await {
        Ok(device) => Ok(Readiness { ready: true, device: Some(device), problem: None }),
        Err(e) => Ok(Readiness { ready: false, device: None, problem: Some(plain(e)) }),
    }
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
        suggested_path: state.default_root.join(&parsed.folder_label).to_string_lossy().into_owned(),
        device_name: parsed.device_name,
        folder_label: parsed.folder_label,
    })
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
    Ok(state.default_root.join(label).to_string_lossy().into_owned())
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

pub fn run() {
    tauri::Builder::default()
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
                default_root: home_dir.join("HomeCloud"),
            });

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let binary = match engine_binary(resource_dir.as_deref()) {
                    Ok(path) => path,
                    Err(e) => {
                        eprintln!("homecloud: {e}");
                        return;
                    }
                };
                match Engine::start(&binary, &data_dir.join("engine")).await {
                    Ok(engine) => {
                        // A device with no name shows up on other people's
                        // screens as a meaningless ID, so give it one on first run.
                        if let Ok(me) = engine.client.this_device().await {
                            if me.name.is_empty() {
                                let _ = engine.client.set_this_device_name(&default_device_name()).await;
                            }
                        }
                        let state = handle.state::<AppState>();
                        *state.engine.write().await = Some(engine);
                    }
                    Err(e) => eprintln!("homecloud: {e}"),
                }
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
            suggested_path,
            accept_invitation,
            decline_invitation,
            set_folder_paused,
            stop_sharing,
            settings,
            save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running HomeCloud");
}
