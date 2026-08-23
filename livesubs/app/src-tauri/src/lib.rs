//! LiveSubs: live subtitles for everything this machine hears.
//!
//! Shape of the thing: no main window, a tray icon, and two windows it
//! owns -- a click-through always-on-top `overlay` that draws the
//! subtitles, and a `settings` window opened from the tray. Audio capture
//! and recognition run on their own threads (see `pipeline`), so neither
//! window has to exist for the app to be doing its job.
//!
//! Everything heavy is local: whisper.cpp for speech (`stt`), Argos for
//! translation (`translate`). Nothing about a meeting leaves the machine.

mod audio;
mod logfile;
mod overlay;
mod pipeline;
mod settings;
mod stt;
mod translate;
mod tray;
mod update;

use pipeline::Pipeline;
use settings::{Config, SettingsState};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

// ---- windows ----------------------------------------------------------

pub fn show_settings_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

// ---- commands ---------------------------------------------------------

#[tauri::command]
fn get_config(state: tauri::State<'_, SettingsState>) -> Config {
    state.get()
}

/// Save a new configuration and apply it. Everything visual is applied
/// immediately; capture only restarts when it has to (see
/// `settings::needs_restart`), because restarting it costs a second of
/// deafness and the user is often mid-sentence when they drag a slider.
#[tauri::command]
fn set_config(
    app: AppHandle,
    config: Config,
    state: tauri::State<'_, SettingsState>,
    pipeline: tauri::State<'_, Pipeline>,
) -> Result<(), String> {
    let old = state.get();
    // `paused` is owned by the tray and the pause command, never by a
    // settings payload: the settings window holds a whole `Config` in
    // memory, and any stale copy of it (a window left open while the tray
    // toggled pause) would otherwise silently resurrect the old value the
    // next time a slider moved.
    let config = Config { paused: old.paused, ..config };
    {
        let mut guard = state.0.lock().map_err(|_| "settings mutex")?;
        *guard = config.clone();
    }
    settings::save(&config)?;
    overlay::apply(&app, &config)?;
    tray::refresh_pause_label(&app, config.paused);
    let _ = tauri::Emitter::emit(&app, "config-changed", &config);
    pipeline.reconfigure(&app, &old, &config)?;
    Ok(())
}

#[tauri::command]
fn toggle_pause(app: AppHandle) -> Result<bool, String> {
    toggle_paused(&app)
}

/// Flip listening on/off, persist it, and tell everyone. Shared by the
/// tray entry and the settings switch so the two can't disagree.
pub fn toggle_paused(app: &AppHandle) -> Result<bool, String> {
    let state = app.state::<SettingsState>();
    let mut config = state.get();
    config.paused = !config.paused;
    {
        let mut guard = state.0.lock().map_err(|_| "settings mutex")?;
        *guard = config.clone();
    }
    settings::save(&config)?;
    tray::refresh_pause_label(app, config.paused);
    let _ = tauri::Emitter::emit(app, "config-changed", &config);
    let pipeline = app.state::<Pipeline>();
    if config.paused {
        pipeline.stop();
        pipeline::emit_status(app, "idle", "Captura en pausa.");
    } else {
        pipeline.start(app, &config)?;
    }
    Ok(config.paused)
}

#[derive(serde::Serialize)]
struct EngineStatus {
    /// Model name -> already downloaded.
    models: Vec<(String, bool)>,
    model_ready: bool,
    translation_ready: bool,
    audio_ready: bool,
    parec_ready: bool,
    capturing: bool,
}

#[tauri::command]
fn engine_status(
    state: tauri::State<'_, SettingsState>,
    pipeline: tauri::State<'_, Pipeline>,
) -> EngineStatus {
    let config = state.get();
    EngineStatus {
        models: stt::MODELS
            .iter()
            .map(|(name, _)| (name.to_string(), stt::model_present(name)))
            .collect(),
        model_ready: stt::model_present(&config.model),
        translation_ready: translate::engine_installed(),
        audio_ready: audio::audio_server_available(),
        parec_ready: audio::parec_available(),
        capturing: pipeline.is_running(),
    }
}

/// Download a whisper model, streaming progress as `status` events (the
/// settings window may be closed and reopened mid-download; events survive
/// that, a command's return value wouldn't).
#[tauri::command]
async fn download_model(app: AppHandle, model: String) -> Result<(), String> {
    let handle = app.clone();
    let name = model.clone();
    tauri::async_runtime::spawn_blocking(move || {
        stt::ensure_model(&name, |done, total| {
            let fraction = if total > 0 { done as f64 / total as f64 } else { 0.0 };
            pipeline::emit_progress(
                &handle,
                format!(
                    "Descargando el modelo {name} ({} MB de {} MB)",
                    done / 1_048_576,
                    total / 1_048_576
                ),
                fraction,
            );
        })
        .map(|_| ())
    })
    .await
    .map_err(|e| e.to_string())??;

    pipeline::emit_status(&app, "info", format!("Modelo {model} listo."));
    // A download is usually what was blocking capture: start now instead
    // of making the user toggle something.
    let config = app.state::<SettingsState>().get();
    if config.model == model && !config.paused {
        app.state::<Pipeline>().start(&app, &config)?;
    }
    Ok(())
}

/// Create the translation venv and fetch the language packages. Minutes
/// long and hundreds of MB, so it reports each step.
#[tauri::command]
async fn install_translation(app: AppHandle) -> Result<(), String> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        translate::install(|step| pipeline::emit_status(&handle, "info", step))
    })
    .await
    .map_err(|e| e.to_string())??;
    pipeline::emit_status(&app, "info", "Motor de traducción instalado.");
    Ok(())
}

/// Translate one line on demand -- the settings window's "probar" button,
/// so the user can tell a working install from a silent one without
/// waiting for someone to speak.
#[tauri::command]
async fn test_translation(
    app: AppHandle,
    text: String,
    from: String,
    to: String,
) -> Result<String, String> {
    let translator = app.state::<Pipeline>().translator();
    tauri::async_runtime::spawn_blocking(move || translator.translate(&text, &from, &to))
        .await
        .map_err(|e| e.to_string())?
}

/// A caption pushed from the settings window, to preview colours and
/// position without speaking. Goes through the same event the real
/// pipeline uses, so what you see is exactly what a caption looks like.
#[tauri::command]
fn preview_caption(app: AppHandle, source: String) -> Result<(), String> {
    let source = if source == "mic" { audio::Source::Mic } else { audio::Source::System };
    let text = match source {
        audio::Source::Mic => "Así se verán tus propias palabras.",
        audio::Source::System => "This is how the other side of the call will look.",
    };
    tauri::Emitter::emit(
        &app,
        "caption",
        pipeline::Caption {
            source,
            text: text.to_string(),
            original: None,
            language: "es".to_string(),
        },
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_log_file(app: AppHandle, path: String) -> Result<(), String> {
    let _ = app;
    // xdg-open, not a bundled opener plugin: one call, no extra
    // dependency, and the desktop's own file association is exactly the
    // behaviour wanted here.
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = settings::load();
    // Both the setup closure and the run loop need the configuration the
    // app started with (one to start capture, the other to place the
    // overlay once GTK is up), and each takes it by move.
    let startup_config = config.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(SettingsState(Mutex::new(config.clone())))
        .manage(Pipeline::default())
        .invoke_handler(tauri::generate_handler![
            get_config,
            set_config,
            toggle_pause,
            engine_status,
            download_model,
            install_translation,
            test_translation,
            preview_caption,
            open_log_file,
            update::app_version,
            update::check_update,
            update::open_releases_page,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            tray::build(&handle)?;

            // Closing the settings window must not close the app -- it's a
            // tray app, and the window is a panel you open when you need
            // it. Hide instead.
            if let Some(window) = app.get_webview_window("settings") {
                let hidden = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = hidden.hide();
                    }
                });
            }

            // A way to check placement, colours and click-through without
            // waiting for someone to speak: `LIVESUBS_DEMO=1 livesubs`
            // draws one caption of each kind a couple of seconds after
            // start. Documented in the README; harmless when unset.
            if std::env::var("LIVESUBS_DEMO").is_ok() {
                let demo = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    let _ = preview_caption(demo.clone(), "system".to_string());
                    std::thread::sleep(std::time::Duration::from_millis(700));
                    let _ = preview_caption(demo, "mic".to_string());
                });
            }

            // Capture starts on its own: the app is useless until it is
            // listening, and the user already said so by launching it.
            let start_handle = handle.clone();
            let start_config = config.clone();
            std::thread::spawn(move || {
                if let Err(error) = start_handle.state::<Pipeline>().start(&start_handle, &start_config) {
                    pipeline::emit_status(&start_handle, "error", error);
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building LiveSubs")
        .run(move |app, event| {
            // The overlay is placed once the event loop is actually
            // running, not in `setup`: sizing and click-through are
            // window-manager round trips, and GTK has to have realised the
            // window before it can take an input shape.
            if let tauri::RunEvent::Ready = event {
                overlay::apply_settling(app, &startup_config);
            }
            // Without this, hiding the last window would exit the process
            // and take the tray icon with it.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            if let tauri::RunEvent::Exit = event {
                app.state::<Pipeline>().stop();
            }
        });
}
