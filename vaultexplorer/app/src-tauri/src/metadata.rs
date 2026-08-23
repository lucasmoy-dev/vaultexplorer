//! Clear Metadata: strip EXIF/GPS from images (decode+re-encode is itself a
//! strip -- `image` never round-trips metadata through `DynamicImage`) and
//! tags from audio (via `lofty`, whole-tag clear). Animated GIFs are
//! explicitly rejected rather than run through the image path: `image`'s
//! GIF decode via `DynamicImage` only keeps a single frame, so "clearing
//! metadata" on one would silently collapse the animation -- a
//! data-loss bug, not a metadata fix.
//!
//! Audio clearing only ever operates on a real path (vault-internal audio
//! is out of scope for now -- `lofty`'s file-level API would need a
//! plaintext temp file to work on in-memory vault bytes, which would break
//! the "decrypted content never touches plaintext disk" invariant the rest
//! of the vault-facing code holds to; image clearing stays fully in-memory
//! and has no such trade-off, so it's supported for vault files too).

use crate::errmap::ToStringErr;
use crate::progress::{ProgressEvent, ProgressReporter};
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{BoundTaggedFile, TaggedFileExt};
use serde::Serialize;
use std::fs::OpenOptions;
use std::path::Path;

#[derive(Serialize, Clone)]
pub struct ClearResult {
    pub name: String,
    pub cleared: bool,
    pub reason: Option<String>,
}

fn ext_of(name: &str) -> String {
    name.rsplit('.').next().unwrap_or("").to_lowercase()
}

const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "m4a", "mp4", "ogg", "opus", "wav", "aac", "ape", "wv", "mpc", "aiff", "aif",
];

/// Strip metadata from already-decoded image bytes, re-encoding to the same
/// format. Returns `Err` (not cleared) for unsupported/animated formats.
pub fn clear_image_metadata(bytes: &[u8], ext: &str) -> Result<Vec<u8>, String> {
    if ext == "gif" {
        return Err("animated formats not supported".to_string());
    }
    let format = image::ImageFormat::from_extension(ext)
        .ok_or_else(|| "unsupported file type".to_string())?;
    let img = image::load_from_memory(bytes).str_err()?;
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), format)
        .str_err()?;
    Ok(out)
}

/// Clear every tag (ID3v2, ID3v1, Vorbis Comments, etc.) on an audio file
/// in place.
pub fn clear_audio_tags(path: &str) -> Result<(), String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .str_err()?;
    let mut tagged =
        BoundTaggedFile::read_from(file, ParseOptions::new()).str_err()?;
    tagged.clear();
    tagged.save(WriteOptions::default()).str_err()
}

/// Clear metadata on a real filesystem file, dispatching by extension.
pub fn clear_fs_file(path: &str, name: &str) -> ClearResult {
    let ext = ext_of(name);
    if image::ImageFormat::from_extension(&ext).is_some() || ext == "gif" {
        let result = std::fs::read(path)
            .str_err()
            .and_then(|bytes| clear_image_metadata(&bytes, &ext))
            .and_then(|out| std::fs::write(path, out).str_err());
        return match result {
            Ok(()) => ClearResult { name: name.to_string(), cleared: true, reason: None },
            Err(reason) => ClearResult { name: name.to_string(), cleared: false, reason: Some(reason) },
        };
    }
    if AUDIO_EXTS.contains(&ext.as_str()) {
        return match clear_audio_tags(path) {
            Ok(()) => ClearResult { name: name.to_string(), cleared: true, reason: None },
            Err(reason) => ClearResult { name: name.to_string(), cleared: false, reason: Some(reason) },
        };
    }
    ClearResult {
        name: name.to_string(),
        cleared: false,
        reason: Some("unsupported file type".to_string()),
    }
}

/// Clear metadata on already-decrypted vault file bytes -- images only, see
/// module docs. Returns the new bytes to be re-encrypted and written back
/// by the caller, or a skip reason.
pub fn clear_vault_bytes(bytes: &[u8], name: &str) -> Result<Vec<u8>, ClearResult> {
    let ext = ext_of(name);
    if image::ImageFormat::from_extension(&ext).is_some() || ext == "gif" {
        return clear_image_metadata(bytes, &ext).map_err(|reason| ClearResult {
            name: name.to_string(),
            cleared: false,
            reason: Some(reason),
        });
    }
    Err(ClearResult {
        name: name.to_string(),
        cleared: false,
        reason: Some(if AUDIO_EXTS.contains(&ext.as_str()) {
            "audio metadata clearing isn't supported for vault-internal files yet".to_string()
        } else {
            "unsupported file type".to_string()
        }),
    })
}

// ---- Tauri commands ----

/// Strip EXIF/GPS from images and tags from audio files, in place. See
/// the module doc comment for the per-format dispatch and its documented
/// limits (animated GIFs and vault-internal audio are explicitly
/// skipped).
#[tauri::command]
pub async fn fs_clear_metadata(
    paths: Vec<String>,
    channel: tauri::ipc::Channel<ProgressEvent>,
    registry: tauri::State<'_, crate::ops::OpRegistry>,
) -> Result<Vec<ClearResult>, String> {
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let out = tauri::async_runtime::spawn_blocking(move || {
        let reporter = ProgressReporter::new_cancellable(channel, (paths.len() as u64).max(1), cancel);
        let mut results = Vec::with_capacity(paths.len());
        for (i, path) in paths.iter().enumerate() {
            if reporter.is_cancelled() {
                return Err("Cancelled".to_string());
            }
            let name = Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            results.push(clear_fs_file(path, &name));
            reporter.report((i + 1) as u64);
        }
        Ok(results)
    })
    .await
    .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    out
}

#[tauri::command]
pub fn vault_clear_metadata(
    state: tauri::State<crate::AppState>,
    rel_paths: Vec<String>,
    channel: tauri::ipc::Channel<ProgressEvent>,
) -> Result<Vec<ClearResult>, String> {
    let reporter = ProgressReporter::new(channel, (rel_paths.len() as u64).max(1));
    let mut results = Vec::with_capacity(rel_paths.len());
    for (i, rel_path) in rel_paths.iter().enumerate() {
        let name = Path::new(rel_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| rel_path.clone());
        let bytes = crate::with_vault(&state, |v| v.decrypt_file(rel_path))?;
        match clear_vault_bytes(&bytes, &name) {
            Ok(cleared) => {
                crate::with_vault(&state, |v| v.write_file(rel_path, &cleared))?;
                results.push(ClearResult { name, cleared: true, reason: None });
            }
            Err(skip) => results.push(skip),
        }
        reporter.report((i + 1) as u64);
    }
    Ok(results)
}
