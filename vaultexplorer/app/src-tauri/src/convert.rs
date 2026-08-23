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

/// Run one ffmpeg job, streaming coarse progress (via ffmpeg's
/// `-progress pipe:1`, parsed for `out_time_us=`) against `duration` from
/// `ffprobe`. Falls back to a single 0%->100% jump if duration can't be
/// determined (e.g. a format ffprobe can't read) rather than failing the
/// job over it. Cancelling the Actions row kills the ffmpeg process.
fn run_ffmpeg(
    input: &str,
    output: &str,
    extra_args: &[String],
    duration: Option<f64>,
    progress: &ProgressReporter,
) -> Result<(), String> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-y", "-i", input]);
    cmd.args(extra_args);
    cmd.args(["-progress", "pipe:1", "-nostats", "-loglevel", "error"]);
    cmd.arg(output);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().str_err()?;
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    for line in reader.lines().map_while(Result::ok) {
        if progress.is_cancelled() {
            let _ = child.kill();
            // The half-written output is not a file the user asked for.
            let _ = std::fs::remove_file(output);
            return Err("Cancelled".to_string());
        }
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
        let _ = std::fs::remove_file(output);
        return Err(if stderr.trim().is_empty() { "ffmpeg failed".to_string() } else { stderr });
    }
    progress.finish();
    Ok(())
}

/// Convert (or extract audio from) `input` into a new file at `output`.
/// `duration` comes from the caller (which needs it anyway, to size the
/// progress total) rather than being probed again here.
pub fn convert_media(
    input: &str,
    output: &str,
    target_ext: &str,
    quality: Quality,
    duration: Option<f64>,
    progress: &ProgressReporter,
) -> Result<(), String> {
    let extra_args = ffmpeg_args_for(target_ext, quality)?;
    run_ffmpeg(input, output, &extra_args, duration, progress)
}

/// Whether this ffmpeg build has a given encoder compiled in -- HEVC
/// (libx265) is what makes the shrink below actually shrink, but it's a
/// separate GPL-licensed library that some distro builds leave out, and
/// falling back to H.264 is much better than failing.
fn ffmpeg_has_encoder(name: &str) -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(name))
        .unwrap_or(false)
}

/// How hard to squeeze, for `shrink_video`. Named for the trade the user is
/// making, not for a codec setting -- the CRF numbers below are what those
/// names mean per encoder.
#[derive(Clone, Copy)]
pub enum ShrinkLevel {
    Light,
    Balanced,
    Small,
}

impl ShrinkLevel {
    pub fn from_str(s: &str) -> ShrinkLevel {
        match s {
            "light" => ShrinkLevel::Light,
            "small" => ShrinkLevel::Small,
            _ => ShrinkLevel::Balanced,
        }
    }
    /// HEVC CRFs. 28 is x265's own default -- roughly "looks the same at
    /// normal viewing distance" -- so Balanced sits there, Light backs off
    /// to visually transparent, and Small accepts softness in fine detail.
    fn hevc_crf(self) -> &'static str {
        match self {
            ShrinkLevel::Light => "24",
            ShrinkLevel::Balanced => "28",
            ShrinkLevel::Small => "32",
        }
    }
    /// The H.264 equivalents: x264 needs a CRF about 4-5 lower than x265
    /// for comparable quality, and produces a bigger file for it.
    fn h264_crf(self) -> &'static str {
        match self {
            ShrinkLevel::Light => "20",
            ShrinkLevel::Balanced => "24",
            ShrinkLevel::Small => "28",
        }
    }
    fn audio_bitrate(self) -> &'static str {
        match self {
            ShrinkLevel::Light => "160k",
            ShrinkLevel::Balanced => "128k",
            ShrinkLevel::Small => "96k",
        }
    }
}

/// Re-encode `input` to a much smaller file at `output`, same resolution
/// and duration, by switching codec (HEVC where available, H.264
/// otherwise) and giving up a little quality -- the "this video is 4GB and
/// it doesn't need to be" case, as opposed to `convert_media`'s "I need
/// this in a different container/format".
///
/// Constant-quality (CRF) rather than a target bitrate or a target size:
/// the saving then follows how compressible the footage actually is
/// (phone video off a modern camera routinely lands at a fifth of its
/// original size) instead of degrading a hard-to-encode clip to hit a
/// number. `-vf scale` only enters at the smallest level, where capping to
/// 1080p is most of where the saving comes from.
pub fn shrink_video(
    input: &str,
    output: &str,
    level: ShrinkLevel,
    duration: Option<f64>,
    progress: &ProgressReporter,
) -> Result<(), String> {
    let hevc = ffmpeg_has_encoder("libx265");
    let mut args: Vec<String> = Vec::new();
    let push = |args: &mut Vec<String>, vals: &[&str]| args.extend(vals.iter().map(|s| s.to_string()));
    if hevc {
        push(&mut args, &["-c:v", "libx265", "-crf", level.hevc_crf(), "-preset", "medium"]);
        // Without hvc1, QuickTime/iOS/Photos see an unplayable file even
        // though the stream itself is fine.
        push(&mut args, &["-tag:v", "hvc1"]);
    } else {
        push(&mut args, &["-c:v", "libx264", "-crf", level.h264_crf(), "-preset", "medium"]);
    }
    // 10-bit or 4:2:2 source re-encoded as-is stays unplayable on plenty of
    // hardware decoders; 8-bit 4:2:0 is the format everything can play.
    push(&mut args, &["-pix_fmt", "yuv420p"]);
    if matches!(level, ShrinkLevel::Small) {
        // Width-capped, height rounded to an even number (encoders reject
        // odd dimensions), and never upscaled -- `min(1920,iw)` leaves
        // anything already smaller alone.
        push(&mut args, &["-vf", "scale='min(1920,iw)':-2"]);
    }
    push(&mut args, &["-c:a", "aac", "-b:a", level.audio_bitrate()]);
    // Metadata up front, so the result starts playing while it streams.
    push(&mut args, &["-movflags", "+faststart"]);
    run_ffmpeg(input, output, &args, duration, progress)
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
    registry: tauri::State<'_, crate::ops::OpRegistry>,
) -> Result<(), String> {
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let out = tauri::async_runtime::spawn_blocking(move || {
        let duration = probe_duration_secs(&path);
        let reporter = ProgressReporter::new_cancellable(channel, progress_total_ms(duration), cancel);
        convert_media(&path, &dest_path, &target_ext, Quality::from_str(&quality), duration, &reporter)
    })
    .await
    .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    out
}

/// ffmpeg reports progress as elapsed output time in milliseconds, so the
/// progress total has to be the clip's own duration in ms. It used to be a
/// flat 1000 for every conversion, which anything longer than a second
/// overshot on its very first update -- and since the frontend treats
/// `done >= total` as "finished", the Actions row vanished a second in and
/// the rest of the conversion ran with nothing on screen.
fn progress_total_ms(duration: Option<f64>) -> u64 {
    duration.map(|d| (d * 1000.0) as u64).filter(|ms| *ms > 0).unwrap_or(1)
}

/// Re-encode a video into a much smaller file (see `shrink_video`).
#[tauri::command]
pub async fn fs_shrink_video(
    path: String,
    dest_path: String,
    level: String,
    channel: Channel<ProgressEvent>,
    registry: tauri::State<'_, crate::ops::OpRegistry>,
) -> Result<(), String> {
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let out = tauri::async_runtime::spawn_blocking(move || {
        let duration = probe_duration_secs(&path);
        let reporter = ProgressReporter::new_cancellable(channel, progress_total_ms(duration), cancel);
        shrink_video(&path, &dest_path, ShrinkLevel::from_str(&level), duration, &reporter)
    })
    .await
    .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    out
}
