//! Everything the user can change, and where it lives on disk.
//!
//! One flat JSON file (`~/.config/livesubs/config.json`) rather than a
//! settings database: it's a handful of scalars, and a file the user can
//! read, diff and copy to another machine is worth more here than any
//! structure a heavier store would add.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

/// Where subtitles sit on screen. Deliberately anchors rather than raw
/// coordinates: the overlay has to land in the right place on whatever
/// monitor it opens on, and a saved pixel position from a 4K screen means
/// nothing on a laptop panel.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    // Bottom is where a subtitle belongs, and where every player puts it.
    #[default]
    Bottom,
    Center,
    Top,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // ---- what to listen to ----
    /// The microphone (your own voice).
    pub capture_mic: bool,
    /// What the machine plays -- the monitor of the default sink, so it
    /// follows whatever output you're actually using (speakers, then
    /// headphones, without touching this setting).
    pub capture_system: bool,

    // ---- recognition ----
    /// whisper.cpp model size: tiny | base | small | medium.
    pub model: String,
    /// "auto" lets whisper detect the spoken language per utterance;
    /// otherwise a fixed code ("en"/"es"/"fr") which is both faster and
    /// more accurate when you already know what's being spoken.
    pub source_language: String,
    /// "off", or the language every subtitle should end up in.
    pub target_language: String,
    /// Show the recognised original under the translation.
    pub show_original: bool,
    /// Speech/silence threshold multiplier. Higher = needs louder speech
    /// to trigger, which is what you want in a noisy room.
    pub sensitivity: f32,

    // ---- appearance ----
    pub anchor: Anchor,
    /// Distance from the anchored edge, in logical pixels.
    pub margin: u32,
    /// Overlay width as a percentage of the screen width.
    pub width_percent: u32,
    pub font_size: u32,
    pub background_color: String,
    pub background_opacity: f32,
    /// Distinct colours are how you tell your own voice from theirs.
    pub mic_color: String,
    pub system_color: String,
    pub max_lines: u32,
    /// Subtitles fade out this long after the last recognised speech, so
    /// a stale line doesn't sit over a video forever.
    pub hide_after_ms: u32,

    // ---- transcript ----
    pub log_enabled: bool,
    pub log_path: String,

    // ---- state ----
    /// Capture suspended from the tray. Persisted on purpose: if you quit
    /// while paused, it should come back paused rather than surprising you
    /// by listening.
    pub paused: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            capture_mic: true,
            capture_system: true,
            model: "base".to_string(),
            source_language: "auto".to_string(),
            target_language: "off".to_string(),
            show_original: false,
            sensitivity: 1.0,
            anchor: Anchor::Bottom,
            margin: 90,
            width_percent: 80,
            font_size: 34,
            background_color: "#000000".to_string(),
            background_opacity: 0.62,
            // Cyan-ish for you, plain white for everyone else: the two
            // most legible choices on a dark plate that still read as
            // clearly different at a glance.
            mic_color: "#7ad7ff".to_string(),
            system_color: "#ffffff".to_string(),
            max_lines: 2,
            hide_after_ms: 6000,
            log_enabled: false,
            log_path: default_log_path(),
            paused: false,
        }
    }
}

pub fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string())
}

fn default_log_path() -> String {
    format!("{}/Documents/livesubs-transcript.txt", home_dir())
}

pub fn config_dir() -> PathBuf {
    PathBuf::from(format!("{}/.config/livesubs", home_dir()))
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// Cache/model/venv home. Split from the config dir so wiping a broken
/// model or translation install never touches the user's settings.
pub fn data_dir() -> PathBuf {
    PathBuf::from(format!("{}/.local/share/livesubs", home_dir()))
}

pub fn cache_dir() -> PathBuf {
    PathBuf::from(format!("{}/.cache/livesubs", home_dir()))
}

pub fn load() -> Config {
    // A missing or corrupt file is not an error worth blocking startup
    // over -- an app whose whole UI is a tray icon has nowhere good to
    // report it, and defaults are a working configuration.
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(config: &Config) -> Result<(), String> {
    std::fs::create_dir_all(config_dir()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), json).map_err(|e| e.to_string())
}

/// The live config, shared by the tray, the pipeline and both windows.
pub struct SettingsState(pub Mutex<Config>);

impl SettingsState {
    pub fn get(&self) -> Config {
        self.0.lock().expect("settings mutex").clone()
    }
}

/// Fields that require tearing the capture/recognition threads down and
/// starting them again; everything else (colours, position, logging) is
/// picked up live.
pub fn needs_restart(old: &Config, new: &Config) -> bool {
    old.capture_mic != new.capture_mic
        || old.capture_system != new.capture_system
        || old.model != new.model
        || old.paused != new.paused
}
