//! Image thumbnail generation. Real (plain) files are cached to disk
//! keyed by path+mtime so re-listing a folder doesn't re-decode every
//! image every time. Vault-internal images are **never** disk-cached --
//! caching a decrypted thumbnail would leak vault image content onto
//! plaintext disk outside the vault's own ciphertext, defeating the
//! point. Every call for a vault image re-decrypts and re-thumbnails;
//! that's only ever for entries actually rendered on screen, so it's a
//! fine trade for keeping the security property simple and obviously
//! correct.

use crate::errmap::ToStringErr;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageEncoder};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn cache_dir() -> PathBuf {
    PathBuf::from(format!("{}/.cache/vaultexplorer/thumbnails", crate::home_dir()))
}

fn cache_key(path: &str, mtime: i64, max_size: u32) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    mtime.hash(&mut hasher);
    max_size.hash(&mut hasher);
    format!("{:x}.jpg", hasher.finish())
}

/// Decode `bytes`, shrink to fit within `max_size` x `max_size`
/// (preserving aspect ratio), and re-encode as a JPEG.
fn make_thumbnail(bytes: &[u8], max_size: u32) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(bytes).str_err()?;
    let thumb = img.thumbnail(max_size, max_size);
    let rgb = thumb.to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 80)
        .write_image(rgb.as_raw(), rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
        .str_err()?;
    Ok(out)
}

fn to_data_uri(jpeg_bytes: &[u8]) -> String {
    format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg_bytes))
}

/// Thumbnail for a real on-disk image file, cached by path+mtime.
pub fn thumbnail_for_path(path: &str, max_size: u32) -> Result<String, String> {
    let metadata = std::fs::metadata(path).str_err()?;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).str_err()?;
    let cache_path = dir.join(cache_key(path, mtime, max_size));

    if let Ok(cached) = std::fs::read(&cache_path) {
        return Ok(to_data_uri(&cached));
    }
    let bytes = std::fs::read(path).str_err()?;
    let jpeg = make_thumbnail(&bytes, max_size)?;
    let _ = std::fs::write(&cache_path, &jpeg);
    Ok(to_data_uri(&jpeg))
}

/// Thumbnail for already-decrypted vault-file bytes. See module docs for
/// why this path is never disk-cached.
pub fn thumbnail_for_bytes(bytes: &[u8], max_size: u32) -> Result<String, String> {
    let jpeg = make_thumbnail(bytes, max_size)?;
    Ok(to_data_uri(&jpeg))
}

/// Thumbnail for a real on-disk video file: grabs a single frame via
/// `ffmpeg` (1s in, or the very first frame for shorter clips) into a temp
/// JPEG, then runs it through the same resize/cache pipeline as a real
/// image. Real-fs only -- there's no vault-internal equivalent, since
/// ffmpeg needs a real path to read and decrypting a vault video to a
/// plaintext temp file for it would break the same invariant that scoped
/// vault-internal audio metadata clearing and media conversion out too.
pub fn thumbnail_for_video(path: &str, max_size: u32) -> Result<String, String> {
    let metadata = std::fs::metadata(path).str_err()?;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).str_err()?;
    let cache_path = dir.join(cache_key(&format!("video:{path}"), mtime, max_size));
    if let Ok(cached) = std::fs::read(&cache_path) {
        return Ok(to_data_uri(&cached));
    }

    // Per-call unique temp name (pid + atomic seq) -- a process-shared name
    // would collide once video thumbnails run concurrently on the blocking
    // threadpool.
    use std::sync::atomic::{AtomicU64, Ordering};
    static VTHUMB_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = VTHUMB_SEQ.fetch_add(1, Ordering::Relaxed);
    let frame_path =
        std::env::temp_dir().join(format!("vaultexplorer-vthumb-{}-{}.jpg", std::process::id(), seq));
    let grab = |seek: &str| -> bool {
        Command::new("ffmpeg")
            .args(["-y", "-ss", seek, "-i", path, "-frames:v", "1", "-vf", "scale=480:-1"])
            .arg(&frame_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
            && frame_path.exists()
    };
    // 1s in avoids all-black opening frames on most clips; fall back to
    // the very first frame for anything shorter than that.
    if !grab("00:00:01") && !grab("00:00:00") {
        return Err("could not extract a video frame".to_string());
    }
    let bytes = std::fs::read(&frame_path).str_err()?;
    let _ = std::fs::remove_file(&frame_path);
    let jpeg = make_thumbnail(&bytes, max_size)?;
    let _ = std::fs::write(&cache_path, &jpeg);
    Ok(to_data_uri(&jpeg))
}

// ---- Tauri commands ----

/// A small base64 JPEG data URI for a real image file, cached on disk by
/// path+mtime. `async` + `spawn_blocking` so the decode/resize/ffmpeg work
/// runs off the webview (main) thread on the blocking threadpool -- many
/// tiles opening at once then decode in parallel instead of freezing the
/// UI one image at a time.
#[tauri::command]
pub async fn fs_thumbnail(path: String, max_size: u32) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let ext = Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if matches!(ext.as_str(), "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v") {
            thumbnail_for_video(&path, max_size)
        } else {
            thumbnail_for_path(&path, max_size)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Same, for an image living inside the active vault -- decrypted only
/// in memory, never disk-cached (see the module doc comment). The decrypt
/// needs the vault state so it stays inline; only the CPU-bound
/// decode/resize is offloaded to the blocking pool.
#[tauri::command]
pub async fn vault_thumbnail(
    state: tauri::State<'_, crate::AppState>,
    rel_path: String,
    max_size: u32,
) -> Result<String, String> {
    let bytes = crate::with_vault(&state, |v| v.decrypt_file(&rel_path))?;
    tauri::async_runtime::spawn_blocking(move || thumbnail_for_bytes(&bytes, max_size))
        .await
        .map_err(|e| e.to_string())?
}
