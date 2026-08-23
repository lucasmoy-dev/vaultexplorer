//! Audio in, captions out.
//!
//! One thread per capture source, each running the same short loop:
//! `parec` frames -> VAD -> whisper -> (optional) translation -> a
//! `caption` event for the overlay and a line in the transcript. Two
//! independent threads rather than one mixed stream, because keeping the
//! microphone and the system output apart *is* the feature that lets the
//! overlay colour "you" differently from "them" -- mixing them would make
//! that impossible to recover.
//!
//! Restarts are explicit: changing the model, or which sources to listen
//! to, tears the threads down and starts new ones (see
//! `settings::needs_restart`). Colours, position, target language and
//! logging are read per utterance instead, so tweaking them mid-meeting
//! costs nothing.

use crate::audio::{Recorder, Source};
use crate::settings::{Config, SettingsState};
use crate::stt::{Engine, Session};
use crate::translate::Translator;
use livesubs_core::vad::{Vad, FRAME};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tauri::{AppHandle, Emitter, Manager};

/// What the overlay draws.
#[derive(Clone, Serialize)]
pub struct Caption {
    pub source: Source,
    /// What ends up on screen (translated when translation is on).
    pub text: String,
    /// The recognised original, when it differs from `text`.
    pub original: Option<String>,
    /// Detected (or forced) language of the speech.
    pub language: String,
}

/// Anything the settings window should show about the engine's health:
/// download progress, a dead `parec`, a translation worker that won't
/// start. An app with no main window has no other way to tell the user
/// why it went quiet.
#[derive(Clone, Serialize)]
pub struct Status {
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
}

pub fn emit_status(app: &AppHandle, kind: &str, message: impl Into<String>) {
    let _ = app.emit(
        "status",
        Status { kind: kind.to_string(), message: message.into(), progress: None },
    );
}

pub fn emit_progress(app: &AppHandle, message: impl Into<String>, progress: f64) {
    let _ = app.emit(
        "status",
        Status { kind: "progress".into(), message: message.into(), progress: Some(progress) },
    );
}

struct Running {
    stop: Arc<AtomicBool>,
    /// PIDs of the `parec` children, so stopping doesn't have to wait for
    /// a blocking read to return on its own (which, on a silent monitor
    /// source, can be a long time).
    pids: Vec<u32>,
    threads: Vec<JoinHandle<()>>,
    model: String,
}

#[derive(Default)]
pub struct Pipeline {
    running: Mutex<Option<Running>>,
    /// Kept across restarts when the model didn't change: loading `base`
    /// takes a second or two, and toggling one source shouldn't pay it.
    engine: Mutex<Option<Arc<Engine>>>,
    translator: Arc<Translator>,
}

impl Pipeline {
    pub fn translator(&self) -> Arc<Translator> {
        self.translator.clone()
    }

    pub fn is_running(&self) -> bool {
        self.running.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// Start capture for `config`. A missing model is not an error: it
    /// reports itself and leaves the app idle, because the fix (a
    /// download) belongs to the user.
    pub fn start(&self, app: &AppHandle, config: &Config) -> Result<(), String> {
        self.stop();
        if config.paused {
            emit_status(app, "idle", "Captura en pausa.");
            return Ok(());
        }
        if !config.capture_mic && !config.capture_system {
            emit_status(app, "idle", "No hay ninguna fuente de audio activada.");
            return Ok(());
        }
        if !crate::audio::parec_available() {
            return Err("falta parec. Instálalo con: sudo apt install pulseaudio-utils".to_string());
        }
        if !crate::stt::model_present(&config.model) {
            emit_status(
                app,
                "model-missing",
                format!("El modelo \"{}\" no está descargado.", config.model),
            );
            return Ok(());
        }

        let engine = self.load_engine(&config.model)?;
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        let mut pids = Vec::new();
        let mut sources = Vec::new();
        if config.capture_mic {
            sources.push(Source::Mic);
        }
        if config.capture_system {
            sources.push(Source::System);
        }

        for source in sources {
            let mut recorder = Recorder::start(source)?;
            pids.push(recorder.pid());
            let session = engine.session()?;
            let app = app.clone();
            let stop = stop.clone();
            let translator = self.translator.clone();
            threads.push(std::thread::spawn(move || {
                run_source(&app, source, &mut recorder, session, translator, stop);
            }));
        }

        emit_status(app, "running", "Escuchando.");
        *self.running.lock().map_err(|_| "pipeline mutex")? =
            Some(Running { stop, pids, threads, model: config.model.clone() });
        Ok(())
    }

    fn load_engine(&self, model: &str) -> Result<Arc<Engine>, String> {
        let mut guard = self.engine.lock().map_err(|_| "engine mutex")?;
        if let Some(engine) = guard.as_ref() {
            if engine.model == model {
                return Ok(engine.clone());
            }
        }
        let engine = Arc::new(crate::stt::load_engine(model)?);
        *guard = Some(engine.clone());
        Ok(engine)
    }

    pub fn stop(&self) {
        let Ok(mut guard) = self.running.lock() else { return };
        let Some(running) = guard.take() else { return };
        running.stop.store(true, Ordering::Relaxed);
        // SIGTERM the recorders so the threads' blocking reads return now
        // rather than whenever the next audio frame happens to arrive.
        for pid in &running.pids {
            // SAFETY: a plain kill(2) on a pid this process spawned.
            unsafe { libc::kill(*pid as libc::pid_t, libc::SIGTERM) };
        }
        for thread in running.threads {
            let _ = thread.join();
        }
        let _ = running.model;
    }

    /// Apply a new configuration, restarting only if something structural
    /// changed.
    pub fn reconfigure(&self, app: &AppHandle, old: &Config, new: &Config) -> Result<(), String> {
        if new.target_language == "off" {
            // Don't keep ~500MB of translation models resident for a
            // feature that's off.
            self.translator.shutdown();
        }
        if crate::settings::needs_restart(old, new) || !self.is_running() && !new.paused {
            self.start(app, new)?;
        }
        Ok(())
    }
}

/// One capture source's whole life. Returns when `stop` is set or the
/// recorder dies (audio server restart, device unplugged) -- the latter is
/// reported, since silent subtitles with no explanation is the worst
/// failure this app has.
fn run_source(
    app: &AppHandle,
    source: Source,
    recorder: &mut Recorder,
    mut session: Session,
    translator: Arc<Translator>,
    stop: Arc<AtomicBool>,
) {
    let settings = app.state::<SettingsState>();
    let mut vad = Vad::new(settings.get().sensitivity);
    let mut frame = vec![0f32; FRAME];
    let mut last_sensitivity = settings.get().sensitivity;

    while !stop.load(Ordering::Relaxed) {
        if !recorder.read_exact_samples(&mut frame) {
            if !stop.load(Ordering::Relaxed) {
                let detail = recorder.stderr_tail();
                emit_status(
                    app,
                    "error",
                    if detail.is_empty() {
                        format!("La captura de {} se detuvo.", source.label())
                    } else {
                        format!("La captura de {} se detuvo: {detail}", source.label())
                    },
                );
            }
            return;
        }
        // Sensitivity is a live knob: someone fiddling with it mid-meeting
        // shouldn't have to restart capture to hear the difference.
        let config = settings.get();
        if (config.sensitivity - last_sensitivity).abs() > f32::EPSILON {
            vad.set_sensitivity(config.sensitivity);
            last_sensitivity = config.sensitivity;
        }
        let Some(utterance) = vad.push(&frame) else { continue };
        handle_utterance(app, source, &mut session, &translator, &config, &utterance);
    }

    // Whatever was mid-sentence when capture stopped still deserves to be
    // transcribed -- into the transcript at least.
    if let Some(tail) = vad.flush() {
        let config = settings.get();
        handle_utterance(app, source, &mut session, &translator, &config, &tail);
    }
}

fn handle_utterance(
    app: &AppHandle,
    source: Source,
    session: &mut Session,
    translator: &Translator,
    config: &Config,
    samples: &[f32],
) {
    let forced = (config.source_language != "auto").then_some(config.source_language.as_str());
    let recognition = match session.transcribe(samples, forced) {
        Ok(recognition) => recognition,
        Err(error) => {
            emit_status(app, "error", error);
            return;
        }
    };
    if recognition.text.is_empty() {
        return; // silence, music, a door -- nothing to show
    }

    let detected = if recognition.language.is_empty() {
        config.source_language.clone()
    } else {
        recognition.language.clone()
    };
    let mut shown = recognition.text.clone();
    let mut original = None;
    if config.target_language != "off" && detected != config.target_language {
        match translator.translate(&recognition.text, &detected, &config.target_language) {
            Ok(translated) => {
                if translated.trim() != recognition.text.trim() {
                    original = Some(recognition.text.clone());
                    shown = translated;
                }
            }
            Err(error) => emit_status(app, "error", format!("Traducción: {error}")),
        }
    }

    let _ = app.emit(
        "caption",
        Caption {
            source,
            text: shown.clone(),
            original: config.show_original.then(|| original.clone()).flatten(),
            language: detected.clone(),
        },
    );

    if config.log_enabled {
        let translated = original.as_ref().map(|_| shown.as_str());
        let raw = original.clone().unwrap_or_else(|| shown.clone());
        if let Err(error) = crate::logfile::append(&config.log_path, source.label(), &detected, &raw, translated) {
            emit_status(app, "error", format!("Transcripción: {error}"));
        }
    }
}
