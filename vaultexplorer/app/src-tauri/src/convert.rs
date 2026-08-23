//! "Convert To": native image format conversion (via `image`, same
//! decode/re-encode shape as `metadata.rs`) plus video/audio conversion by
//! shelling out to `ffmpeg` (confirmed present on this machine at
//! `/usr/bin/ffmpeg`, GPL build -- shelling out to the already-installed
//! binary is not a licensing concern, same as invoking `git`).
//!
//! Video/audio conversion is real-fs only for now: ffmpeg needs a real
//! path to read from, and decrypting a vault file to a plaintext temp
//! file for it would break the "decrypted content never touches
//! plaintext disk" invariant the rest of the vault-facing code holds to
//! -- same call already made for vault-internal audio metadata clearing.

use crate::errmap::ToStringErr;
use crate::progress::{ProgressEvent, ProgressReporter};
use crate::{with_vault, AppState};
use image::ImageEncoder;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use tauri::ipc::Channel;
use tauri::State;

/// Quality tiers shared across every lossy video/audio target -- mapped
/// per-codec below rather than exposing raw CRF/bitrate numbers in the UI.
#[derive(Clone, Copy)]
pub enum Quality {
    High,
    Medium,
    Low,
}

impl Quality {
    pub fn from_str(s: &str) -> Quality {
        match s {
            "high" => Quality::High,
            "low" => Quality::Low,
            _ => Quality::Medium,
        }
    }
    pub(crate) fn h264_crf(self) -> &'static str {
        match self {
            Quality::High => "18",
            Quality::Medium => "23",
            Quality::Low => "28",
        }
    }
    fn vp9_crf(self) -> &'static str {
        match self {
            Quality::High => "24",
            Quality::Medium => "32",
            Quality::Low => "40",
        }
    }
    fn audio_bitrate(self) -> &'static str {
        match self {
            Quality::High => "320k",
            Quality::Medium => "192k",
            Quality::Low => "128k",
        }
    }
}

// ---------------- images (native) ----------------

pub fn convert_image_bytes(bytes: &[u8], target_ext: &str, quality: Option<u8>) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(bytes).str_err()?;
    let mut out = Vec::new();
    match target_ext {
        "jpg" | "jpeg" => {
            let rgb = img.to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.unwrap_or(85))
                .write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
                .str_err()?;
        }
        "webp" => {
            // The `image` crate's WebP encoder is lossless-only pre-0.26;
            // `quality` is accepted for API symmetry with jpeg but has no
            // effect yet -- noted rather than silently ignored.
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::WebP)
                .str_err()?;
        }
        _ => {
            let format = image::ImageFormat::from_extension(target_ext)
                .ok_or_else(|| format!("unsupported target format: {target_ext}"))?;
            img.write_to(&mut std::io::Cursor::new(&mut out), format)
                .str_err()?;
        }
    }
    Ok(out)
}

/// Resize to fit within `width` x `height` (aspect-ratio preserved,
/// Lanczos3 filter), re-encoded in the same format the image was
/// decoded as.
pub fn resize_image_bytes(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let format = image::guess_format(bytes).str_err()?;
    let img = image::load_from_memory(bytes).str_err()?;
    let resized = img.resize(width, height, image::imageops::FilterType::Lanczos3);
    let mut out = Vec::new();
    resized.write_to(&mut std::io::Cursor::new(&mut out), format).str_err()?;
    Ok(out)
}

// ---------------- video/audio (ffmpeg) ----------------

pub fn ffmpeg_available() -> bool {
    Command::new("which").arg("ffmpeg").output().map(|o| o.status.success()).unwrap_or(false)
}

pub(crate) fn probe_duration_secs(path: &str) -> Option<f64> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().ok()
}

fn ffmpeg_args_for(target_ext: &str, quality: Quality) -> Result<Vec<String>, String> {
    let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    Ok(match target_ext {
        "mp4" | "mkv" | "mov" | "m4v" => a(&[
            "-c:v", "libx264", "-crf", quality.h264_crf(), "-preset", "medium", "-c:a", "aac", "-b:a", "192k",
        ]),
        "webm" => a(&["-c:v", "libvpx-vp9", "-crf", quality.vp9_crf(), "-b:v", "0", "-c:a", "libopus"]),
        "mp3" => a(&["-vn", "-c:a", "libmp3lame", "-b:a", quality.audio_bitrate()]),
        "wav" => a(&["-vn", "-c:a", "pcm_s16le"]),
        "flac" => a(&["-vn", "-c:a", "flac"]),
        "ogg" => a(&["-vn", "-c:a", "libvorbis", "-b:a", quality.audio_bitrate()]),
        "m4a" | "aac" => a(&["-vn", "-c:a", "aac", "-b:a", quality.audio_bitrate()]),
        _ => return Err(format!("unsupported target format: {target_ext}")),
    })
}

/// Convert (or extract audio from) `input` into a new file at `output`,
/// streaming coarse progress (via ffmpeg's `-progress pipe:1`, parsed for
/// `out_time_us=`) against the input's duration from `ffprobe`. Falls
/// back to a single 0%->100% jump if duration can't be determined (e.g. a
/// format ffprobe can't read) rather than failing the conversion over it.
pub fn convert_media(input: &str, output: &str, target_ext: &str, quality: Quality, progress: &ProgressReporter) -> Result<(), String> {
    let extra_args = ffmpeg_args_for(target_ext, quality)?;
    let duration = probe_duration_secs(input);

    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-y", "-i", input]);
    cmd.args(&extra_args);
    cmd.args(["-progress", "pipe:1", "-nostats", "-loglevel", "error"]);
    cmd.arg(output);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().str_err()?;
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    for line in reader.lines().map_while(Result::ok) {
        if let Some(us) = line.strip_prefix("out_time_us=") {
            if let (Ok(us), Some(dur)) = (us.parse::<f64>(), duration) {
                let done_ms = (us / 1000.0) as u64;
                let total_ms = (dur * 1000.0) as u64;
                progress.report(done_ms.min(total_ms));
            }
        }
    }
    let status = child.wait().str_err()?;
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut s) = child.stderr.take() {
            use std::io::Read;
            let _ = s.read_to_string(&mut stderr);
        }
        return Err(if stderr.trim().is_empty() { "ffmpeg failed".to_string() } else { stderr });
    }
    Ok(())
}

// ---------------- PDF / office (LibreOffice + poppler + ImageMagick) ----------------

pub fn libreoffice_available() -> bool {
    Command::new("which").arg("libreoffice").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Convert between PDF and DOC/DOCX (either direction) via LibreOffice
/// headless. `target_ext` is "pdf" or "docx". Real-fs only -- LibreOffice
/// needs a real path to read and write.
pub fn convert_office(input: &str, dest_dir: &str, target_ext: &str) -> Result<String, String> {
    let output = Command::new("libreoffice")
        .args(["--headless", "--convert-to", target_ext, "--outdir", dest_dir, input])
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let stem = Path::new(input).file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let dest = Path::new(dest_dir).join(format!("{stem}.{target_ext}"));
    if !dest.exists() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(if stderr.trim().is_empty() {
            "LibreOffice produced no output file".to_string()
        } else {
            stderr.trim().to_string()
        });
    }
    Ok(dest.to_string_lossy().to_string())
}

/// One JPEG per page, via `pdftoppm` (poppler-utils). Returns the
/// produced file paths, in page order.
pub fn pdf_to_images(input: &str, dest_dir: &str, dest_stem: &str) -> Result<Vec<String>, String> {
    let prefix = Path::new(dest_dir).join(dest_stem);
    let output = Command::new("pdftoppm")
        .args(["-jpeg", "-r", "150", input, prefix.to_str().unwrap()])
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let mut out = Vec::new();
    if let Ok(read) = std::fs::read_dir(dest_dir) {
        let want_prefix = format!("{dest_stem}-");
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&want_prefix) && name.ends_with(".jpg") {
                out.push(entry.path().to_string_lossy().to_string());
            }
        }
    }
    out.sort();
    Ok(out)
}

/// A single image into a single-page PDF, via ImageMagick's `convert`
/// (confirmed present and working on this machine, unlike this
/// environment's LibreOffice snap install -- see the code comment on the
/// `fs_convert_office`/`fs_image_to_pdf` commands in lib.rs for why the
/// two PDF paths use different tools).
pub fn image_to_pdf(input: &str, dest_path: &str) -> Result<(), String> {
    let output = Command::new("convert").args([input, dest_path]).output().str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

fn resized_dest_name(dir: &Path, stem: &str, ext: &str) -> String {
    let mut candidate = format!("{stem}-resized.{ext}");
    let mut i = 2;
    while dir.join(&candidate).exists() {
        candidate = format!("{stem}-resized-{i}.{ext}");
        i += 1;
    }
    candidate
}

// ---- Tauri commands ----

#[tauri::command]
pub fn convert_ffmpeg_available() -> bool {
    ffmpeg_available()
}

#[tauri::command]
pub async fn fs_convert_image(path: String, dest_path: String, target_ext: String, quality: Option<u8>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).str_err()?;
        let out = convert_image_bytes(&bytes, &target_ext, quality)?;
        std::fs::write(&dest_path, out).str_err()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn vault_convert_image(
    state: State<AppState>,
    rel_path: String,
    dest_rel_path: String,
    target_ext: String,
    quality: Option<u8>,
) -> Result<(), String> {
    let bytes = with_vault(&state, |v| v.decrypt_file(&rel_path))?;
    let out = convert_image_bytes(&bytes, &target_ext, quality)?;
    with_vault(&state, |v| v.write_file(&dest_rel_path, &out))
}

#[tauri::command]
pub async fn fs_resize_images(
    paths: Vec<String>,
    width: u32,
    height: u32,
    channel: Channel<ProgressEvent>,
    registry: tauri::State<'_, crate::ops::OpRegistry>,
) -> Result<(), String> {
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let out = tauri::async_runtime::spawn_blocking(move || {
    let reporter = ProgressReporter::new_cancellable(channel, (paths.len() as u64).max(1), cancel);
    for (i, path) in paths.iter().enumerate() {
        if reporter.is_cancelled() {
            return Err("Cancelled".to_string());
        }
        let p = Path::new(path);
        let bytes = std::fs::read(p).str_err()?;
        let out = resize_image_bytes(&bytes, width, height)?;
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
        let dest = p.parent().unwrap_or(Path::new(".")).join(resized_dest_name(p.parent().unwrap_or(Path::new(".")), stem, ext));
        std::fs::write(dest, out).str_err()?;
        reporter.report((i + 1) as u64);
    }
    Ok(())
    })
    .await
    .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    out
}

#[tauri::command]
pub fn vault_resize_images(
    state: State<AppState>,
    rel_paths: Vec<String>,
    width: u32,
    height: u32,
    channel: Channel<ProgressEvent>,
) -> Result<(), String> {
    let reporter = ProgressReporter::new(channel, (rel_paths.len() as u64).max(1));
    for (i, rel_path) in rel_paths.iter().enumerate() {
        let bytes = with_vault(&state, |v| v.decrypt_file(rel_path))?;
        let out = resize_image_bytes(&bytes, width, height)?;
        let rp = Path::new(rel_path);
        let stem = rp.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
        let ext = rp.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
        let dir = rp.parent().unwrap_or(Path::new(""));
        let dest_rel = dir.join(format!("{stem}-resized.{ext}"));
        with_vault(&state, |v| v.write_file(&dest_rel, &out))?;
        reporter.report((i + 1) as u64);
    }
    Ok(())
}

#[tauri::command]
pub fn convert_libreoffice_available() -> bool {
    libreoffice_available()
}

#[tauri::command]
pub async fn fs_convert_office(path: String, dest_dir: String, target_ext: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || convert_office(&path, &dest_dir, &target_ext))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn fs_pdf_to_images(path: String, dest_dir: String, dest_stem: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || pdf_to_images(&path, &dest_dir, &dest_stem))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn fs_image_to_pdf(path: String, dest_path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || image_to_pdf(&path, &dest_path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn fs_convert_media(
    path: String,
    dest_path: String,
    target_ext: String,
    quality: String,
    channel: Channel<ProgressEvent>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let reporter = ProgressReporter::new(channel, 1000);
        convert_media(&path, &dest_path, &target_ext, Quality::from_str(&quality), &reporter)
    })
    .await
    .map_err(|e| e.to_string())?
}
