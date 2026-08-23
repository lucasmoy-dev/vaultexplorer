//! Speech to text, locally, via whisper.cpp (`whisper-rs`).
//!
//! Nothing leaves the machine: the audio of every call you sit in is not
//! something to post to a cloud API for convenience. The model is a GGML
//! file downloaded once from whisper.cpp's own distribution and cached
//! under `~/.cache/livesubs/whisper/`.
//!
//! Sizes are a real trade-off on this hardware (Intel iGPU, so CPU-only):
//! `tiny` keeps up with anything but mangles Spanish and French, `base` is
//! the sensible default with two live streams, `small` is noticeably
//! better on accents at roughly 2-3x the cost, `medium` only makes sense
//! with a single stream and patience.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

pub const MODELS: &[(&str, &str)] = &[
    ("tiny", "ggml-tiny.bin"),
    ("base", "ggml-base.bin"),
    ("small", "ggml-small.bin"),
    ("medium", "ggml-medium.bin"),
];

fn model_file(model: &str) -> Result<&'static str, String> {
    MODELS
        .iter()
        .find(|(name, _)| *name == model)
        .map(|(_, file)| *file)
        .ok_or_else(|| format!("unknown model \"{model}\""))
}

/// The model file's name inside whichever directory the caller keeps its
/// models in.
pub fn model_file_name(model: &str) -> Result<&'static str, String> {
    model_file(model)
}

pub fn model_path(dir: &Path, model: &str) -> Result<PathBuf, String> {
    Ok(dir.join(model_file(model)?))
}

pub fn model_present(dir: &Path, model: &str) -> bool {
    model_path(dir, model).map(|p| p.is_file()).unwrap_or(false)
}

/// Where a GGML model is published. Exposed so the Android side can
/// download it with the platform's own HTTP stack instead of linking a
/// second one into the shared library.
pub fn model_url(model: &str) -> Result<String, String> {
    Ok(format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        model_file(model)?
    ))
}

/// Download the model into `dir` if it isn't there yet, reporting `(done,
/// total)` bytes as it goes. Downloads to a `.part` file and renames on
/// success, so an interrupted download can't leave a truncated model that
/// then fails to load with something cryptic from ggml.
#[cfg(feature = "download")]
pub fn ensure_model(dir: &Path, model: &str, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    let path = model_path(dir, model)?;
    if path.is_file() {
        return Ok(path);
    }
    let file_name = model_file(model)?;
    let url = model_url(model)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let response = reqwest::blocking::Client::builder()
        .timeout(None) // a 1.5GB model on a slow line is not a hung request
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .send()
        .map_err(|e| format!("couldn't reach huggingface.co: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("model download failed: HTTP {}", response.status()));
    }
    let total = response.content_length().unwrap_or(0);
    let part = dir.join(format!("{file_name}.part"));
    let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut reader = response;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let read = std::io::Read::read(&mut reader, &mut buf).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        std::io::Write::write_all(&mut out, &buf[..read]).map_err(|e| e.to_string())?;
        done += read as u64;
        progress(done, total);
    }
    drop(out);
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The loaded model. One per app: the weights are shared, and each capture
/// stream gets its own decoding state off the same context (whisper.cpp's
/// intended way to run two streams without loading the model twice).
pub struct Engine {
    ctx: Arc<WhisperContext>,
    pub model: String,
}

impl Engine {
    /// Load a model by name from `dir`.
    pub fn load(dir: &Path, model: &str) -> Result<Engine, String> {
        let path = model_path(dir, model)?;
        if !path.is_file() {
            return Err(format!("the {model} model isn't downloaded yet"));
        }
        Self::load_file(&path, model)
    }

    /// Load a model from an exact path -- what the Android side uses,
    /// since it hands the file down from Kotlin.
    pub fn load_file(path: &Path, model: &str) -> Result<Engine, String> {
        let ctx = WhisperContext::new_with_params(
            path.to_str().ok_or("model path isn't valid UTF-8")?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| format!("couldn't load the {model} model: {e}"))?;
        Ok(Engine { ctx: Arc::new(ctx), model: model.to_string() })
    }

    pub fn session(&self) -> Result<Session, String> {
        let state = self.ctx.create_state().map_err(|e| e.to_string())?;
        Ok(Session { state, threads: threads_per_stream() })
    }
}

/// Leave the machine usable. Two streams transcribing at once on a 16-core
/// laptop should not saturate it -- whisper.cpp scales poorly past a
/// handful of threads anyway, and the browser in the video call needs CPU
/// too.
fn threads_per_stream() -> i32 {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) as i32;
    (cores / 4).clamp(2, 6)
}

pub struct Recognition {
    pub text: String,
    /// Whisper's own guess (or the language it was told to assume).
    pub language: String,
}

/// Whisper reports, per segment, how likely it thinks that stretch was
/// *not* speech. Above this it is dropped: on a monitor stream that
/// carries music, keyboard noise or two overlapping sources, whisper will
/// happily invent a fluent sentence out of nothing, and a hallucinated
/// subtitle is worse than no subtitle -- it is indistinguishable from a
/// real one. 0.6 is whisper.cpp's own default threshold.
const NO_SPEECH_MAX: f32 = 0.6;

pub struct Session {
    state: WhisperState,
    threads: i32,
}

impl Session {
    /// Transcribe one utterance. `language` is `None` for auto-detect.
    pub fn transcribe(&mut self, samples: &[f32], language: Option<&str>) -> Result<Recognition, String> {
        // Greedy sampling: beam search costs several times more for a
        // difference that doesn't survive being read off a screen for two
        // seconds.
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(self.threads);
        params.set_print_progress(false);
        params.set_print_special(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        // Each utterance is judged on its own audio. Carrying decoder
        // context across chunks is what makes whisper loop, repeating the
        // previous sentence forever once it gets one wrong -- very visible
        // in live subtitles.
        params.set_no_context(true);
        params.set_single_segment(false);
        params.set_suppress_blank(true);
        // Suppress the non-speech tokens whisper uses for music and sound
        // effects; on this input they only ever become "[Music]"-style
        // captions that the cleaner would strip anyway.
        params.set_suppress_nst(true);
        params.set_language(language);

        self.state.full(params, samples).map_err(|e| format!("recognition failed: {e}"))?;

        let mut text = String::new();
        for i in 0..self.state.full_n_segments() {
            if let Some(segment) = self.state.get_segment(i) {
                if segment.no_speech_probability() > NO_SPEECH_MAX {
                    continue;
                }
                if let Ok(chunk) = segment.to_str_lossy() {
                    text.push_str(&chunk);
                }
            }
        }
        let detected = whisper_rs::get_lang_str(self.state.full_lang_id_from_state())
            .unwrap_or("")
            .to_string();
        Ok(Recognition { text: clean(&text), language: detected })
    }
}

/// Whisper emits its non-speech guesses as bracketed tags ("[BLANK_AUDIO]",
/// "(music)", "[ Silence ]") and pads with spaces. None of that belongs on
/// screen, and an utterance that reduces to nothing shouldn't produce a
/// caption at all.
pub fn clean(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut depth_square = 0usize;
    let mut depth_round = 0usize;
    let mut depth_music = 0usize;
    for ch in raw.chars() {
        match ch {
            '[' => depth_square += 1,
            ']' => depth_square = depth_square.saturating_sub(1),
            '(' => depth_round += 1,
            ')' => depth_round = depth_round.saturating_sub(1),
            '\u{266a}' | '\u{266b}' | '\u{1d160}' => depth_music += 1,
            _ if depth_square == 0 && depth_round == 0 => out.push(ch),
            _ => {}
        }
    }
    let _ = depth_music;
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim().to_string()
}

/// Minimal 16-bit PCM WAV reader, for test fixtures only: find the `data`
/// chunk and scale the samples into the -1..1 floats whisper takes. A crate
/// for this would only be pulled in for tests, and the app's real input
/// never touches a file (it comes from `parec` as floats already).
#[cfg(test)]
fn wav_to_f32_mono(bytes: &[u8]) -> Vec<f32> {
    let mut offset = 12; // past "RIFF....WAVE"
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body = offset + 8;
        if id == b"data" {
            let end = (body + size).min(bytes.len());
            return bytes[body..end]
                .chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0)
                .collect();
        }
        offset = body + size + (size & 1);
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_padding_are_stripped() {
        assert_eq!(clean("  [BLANK_AUDIO] "), "");
        assert_eq!(clean(" (music) hello   there "), "hello there");
        assert_eq!(clean("[ Silence ]"), "");
        assert_eq!(clean(" Bonjour à tous "), "Bonjour à tous");
    }

    #[test]
    fn real_speech_survives_untouched() {
        assert_eq!(clean("The meeting starts at ten."), "The meeting starts at ten.");
    }

    #[test]
    fn every_model_name_maps_to_a_file() {
        let dir = Path::new("/tmp/whatever");
        for (name, _) in MODELS {
            assert!(model_path(dir, name).is_ok(), "{name} has no file mapping");
            assert!(model_url(name).unwrap().ends_with(".bin"), "{name} has no url");
        }
        assert!(model_path(dir, "enormous").is_err());
    }

    /// The real pipeline on real speech: whisper.cpp transcribes a known
    /// clip and reports the right language.
    ///
    /// `#[ignore]`d: it downloads the model (~148MB) and a sample clip.
    /// Run it on purpose:
    ///
    /// ```text
    /// cargo test --release -p livesubs-core -- --ignored --nocapture transcribes
    /// ```
    #[test]
    #[ignore]
    fn transcribes_real_speech() {
        let dir = std::env::temp_dir().join("livesubs-core-models");
        ensure_model(&dir, "base", |_, _| {}).expect("model download failed");
        // JFK's inaugural line -- whisper.cpp's own sample, so a failure
        // here is this code, not the audio.
        let clip = std::env::temp_dir().join("livesubs-jfk.wav");
        if !clip.is_file() {
            let url = "https://raw.githubusercontent.com/ggerganov/whisper.cpp/master/samples/jfk.wav";
            let bytes = reqwest::blocking::get(url)
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.bytes())
                .expect("couldn't fetch the sample clip");
            std::fs::write(&clip, &bytes).expect("couldn't write the sample clip");
        }
        let samples = wav_to_f32_mono(&std::fs::read(&clip).expect("read clip"));
        assert!(samples.len() > 16_000, "clip looks empty: {} samples", samples.len());

        let engine = Engine::load(&dir, "base").expect("load base");
        let mut session = engine.session().expect("session");

        // Auto-detect, the app's default: the language is part of what is
        // being verified, not an input.
        let recognition = session.transcribe(&samples, None).expect("transcribe");
        println!("[{}] {}", recognition.language, recognition.text);
        assert_eq!(recognition.language, "en", "detected the wrong language");
        let lowered = recognition.text.to_lowercase();
        assert!(
            lowered.contains("fellow americans") && lowered.contains("country"),
            "unexpected transcript: {}",
            recognition.text
        );

    }

    #[test]
    fn thread_split_leaves_headroom() {
        let threads = threads_per_stream();
        assert!((2..=6).contains(&threads), "unreasonable thread count: {threads}");
    }
}
