//! Local, offline audio/video transcription via `whisper-rs` (vendors
//! whisper.cpp, CPU-only on this machine -- no NVIDIA GPU, confirmed
//! earlier this session). Real-fs only: audio is extracted via `ffmpeg`,
//! which needs a real path (same constraint as every other ffmpeg-backed
//! feature here).
//!
//! The GGML model isn't bundled (it's tens of MB) -- it's downloaded once
//! from the official whisper.cpp Hugging Face distribution on first use
//! and cached under `~/.cache/vaultexplorer/whisper/`. "tiny" (multilingual,
//! ~75MB) is the only size offered for now: smallest/fastest, reasonable
//! accuracy for a first cut, easy to add a size picker later.

use crate::errmap::ToStringErr;
use crate::progress::ProgressReporter;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin";

fn model_dir() -> PathBuf {
    PathBuf::from(format!("{}/.cache/vaultexplorer/whisper", crate::home_dir()))
}
fn model_path() -> PathBuf {
    model_dir().join("ggml-tiny.bin")
}

pub fn model_downloaded() -> bool {
    model_path().is_file()
}

/// Download the model, reporting progress by bytes. The real total (from
/// `Content-Length`) is only known after the response headers arrive, so
/// the `ProgressReporter` is built here rather than passed in -- building
/// it upfront with a placeholder total would mean the "always send the
/// final update unthrottled" guarantee never triggers (done never
/// reaches a placeholder like `u64::MAX`), and the UI could get stuck
/// just short of 100%.
pub fn download_model(channel: tauri::ipc::Channel<crate::progress::ProgressEvent>) -> Result<(), String> {
    std::fs::create_dir_all(model_dir()).str_err()?;
    let resp = reqwest::blocking::get(MODEL_URL).str_err()?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0).max(1);
    let progress = ProgressReporter::new(channel, total);
    let mut reader = resp;
    let tmp_path = model_dir().join("ggml-tiny.bin.part");
    let mut file = std::fs::File::create(&tmp_path).str_err()?;
    let mut buf = [0u8; 65536];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).str_err()?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n]).str_err()?;
        done += n as u64;
        if total > 0 {
            progress.report(done.min(total));
        }
    }
    drop(file);
    std::fs::rename(&tmp_path, model_path()).str_err()?;
    Ok(())
}

/// Decode `input` (audio or video) to mono 16kHz 32-bit float PCM samples
/// via `ffmpeg`, entirely in memory (no temp file).
fn decode_to_f32_mono16k(input: &str) -> Result<Vec<f32>, String> {
    let output = Command::new("ffmpeg")
        .args(["-y", "-i", input, "-ar", "16000", "-ac", "1", "-f", "f32le", "-loglevel", "error", "pipe:1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let bytes = output.stdout;
    let samples = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    Ok(samples)
}

/// Transcribe `input` (audio or video), writing the transcript to
/// `dest_txt_path`. `progress` receives 0..=100 for the actual
/// transcription pass (decoding is comparatively instant and isn't
/// separately reported).
pub fn transcribe(input: &str, dest_txt_path: &str, progress: Arc<ProgressReporter>) -> Result<(), String> {
    let samples = decode_to_f32_mono16k(input)?;

    let ctx = WhisperContext::new_with_params(model_path().to_str().unwrap(), WhisperContextParameters::default())
        .str_err()?;
    let mut state = ctx.create_state().str_err()?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    let progress_for_cb = progress.clone();
    params.set_progress_callback_safe(move |p: i32| {
        progress_for_cb.report(p.clamp(0, 100) as u64);
    });

    state.full(params, &samples).str_err()?;

    let n = state.full_n_segments();
    let mut text = String::new();
    for i in 0..n {
        if let Some(seg) = state.get_segment(i) {
            if let Ok(s) = seg.to_str_lossy() {
                text.push_str(&s);
            }
        }
    }
    std::fs::write(dest_txt_path, text.trim()).str_err()?;
    progress.report(100);
    Ok(())
}

// ---- Tauri commands ----

#[tauri::command]
pub fn transcribe_model_downloaded() -> bool {
    model_downloaded()
}

#[tauri::command]
pub async fn transcribe_download_model(
    channel: tauri::ipc::Channel<crate::progress::ProgressEvent>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || download_model(channel))
        .await
        .map_err(|e| e.to_string())?
}

/// Local, offline transcription (video or audio -> .txt) via whisper.cpp.
/// Real-fs only -- see the module doc comment for why (ffmpeg needs a real
/// path). `async` + `spawn_blocking` so the (very heavy) ffmpeg + whisper.cpp
/// pass runs off the webview thread and the UI stays responsive (progress
/// streams live instead of the whole app freezing until it finishes).
#[tauri::command]
pub async fn transcribe_run(
    path: String,
    dest_txt_path: String,
    channel: tauri::ipc::Channel<crate::progress::ProgressEvent>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let reporter = Arc::new(ProgressReporter::new(channel, 100));
        transcribe(&path, &dest_txt_path, reporter)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::ProgressEvent;

    /// Real end-to-end smoke test: downloads the actual model (skipped if
    /// already cached from a prior run) and transcribes a synthetic
    /// spoken-silence clip via a real whisper.cpp pass, verifying the
    /// whole pipeline (ffmpeg decode -> whisper.cpp -> .txt file) runs to
    /// completion without crashing. Not asserting on transcript content
    /// (a synthesized tone has no real speech) -- this is a plumbing
    /// test, not an accuracy test.
    #[test]
    fn transcribe_pipeline_runs() {
        if !model_downloaded() {
            let channel = tauri::ipc::Channel::<ProgressEvent>::new(|_| Ok(()));
            download_model(channel).expect("model download failed");
        }
        assert!(model_downloaded());

        let clip = "/tmp/ve-transcribe-test.wav";
        let status = std::process::Command::new("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", clip])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("failed to run ffmpeg");
        assert!(status.success());

        let dest_txt = "/tmp/ve-transcribe-test.txt";
        let channel = tauri::ipc::Channel::<ProgressEvent>::new(|_| Ok(()));
        let reporter = Arc::new(ProgressReporter::new(channel, 100));
        transcribe(clip, dest_txt, reporter).expect("transcription failed");
        assert!(std::path::Path::new(dest_txt).exists());

        let _ = std::fs::remove_file(clip);
        let _ = std::fs::remove_file(dest_txt);
    }
}
