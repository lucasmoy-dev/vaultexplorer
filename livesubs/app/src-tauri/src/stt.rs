//! Where *this* app keeps its models, and nothing else.
//!
//! The recognition itself (whisper.cpp, the VAD, the "is this even
//! speech" rules) is in `livesubs-core`, shared with the Android app. All
//! that differs here is the cache directory and the fact that the desktop
//! downloads its own models.

use std::path::PathBuf;

pub use livesubs_core::stt::{Engine, Session, MODELS};

pub fn model_dir() -> PathBuf {
    crate::settings::cache_dir().join("whisper")
}

pub fn model_present(model: &str) -> bool {
    livesubs_core::stt::model_present(&model_dir(), model)
}

/// Download the model if it isn't cached yet, reporting `(done, total)`.
pub fn ensure_model(model: &str, progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    livesubs_core::stt::ensure_model(&model_dir(), model, progress)
}

pub fn load_engine(model: &str) -> Result<Engine, String> {
    Engine::load(&model_dir(), model)
}
