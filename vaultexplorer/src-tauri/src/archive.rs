use crate::errmap::ToStringErr;
use crate::progress::{ProgressEvent, ProgressReporter};
use crate::{with_vault, AppState};
use std::io::Write;
use std::path::Path;
use tauri::ipc::Channel;
use tauri::State;

#[tauri::command]
pub(crate) fn compress_entries(
    state: State<AppState>,
    dir: String,
    names: Vec<String>,
    dest_name: String,
    password: Option<String>,
    level: Option<i64>,
    readme: Option<String>,
) -> Result<(), String> {
    let dest_rel = Path::new(&dir).join(&dest_name);
    let opts = vaultcore::CompressOptions {
        password,
        level,
        readme,
    };
    with_vault(&state, |v| v.compress_paths(&dir, &names, &dest_rel, &opts))
}

#[tauri::command]
pub(crate) fn decompress_entry(
    state: State<AppState>,
    zip_rel_path: String,
    dest_dir_rel_path: String,
    password: Option<String>,
) -> Result<(), String> {
    with_vault(&state, |v| {
        v.decompress_zip(&zip_rel_path, &dest_dir_rel_path, password.as_deref())
    })
}

fn zip_add_recursive(
    zw: &mut zip::ZipWriter<std::fs::File>,
    base: &Path,
    path: &Path,
    options: zip::write::FileOptions<'_, ()>,
    done: &std::cell::Cell<u64>,
    progress: &ProgressReporter,
) -> std::io::Result<()> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            zip_add_recursive(zw, base, &entry?.path(), options, done, progress)?;
        }
    } else {
        let rel = path.strip_prefix(base).unwrap_or(path);
        let name = rel.to_string_lossy().replace('\\', "/");
        zw.start_file(name, options)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        std::io::copy(&mut std::fs::File::open(path)?, zw)?;
        done.set(done.get() + 1);
        progress.report(done.get());
    }
    Ok(())
}

/// Compress `names` (children of `dir`) into a zip archive at
/// `dir/dest_name`, entries named relative to `dir` -- same behavior as
/// Finder's "Compress" context menu action. `password` (if set) turns on
/// AES-256 zip encryption; `level` overrides the Deflate level; `readme`
/// (if set) bundles a README.txt alongside the compressed entries.
#[tauri::command]
pub(crate) async fn fs_compress(
    dir: String,
    names: Vec<String>,
    dest_name: String,
    password: Option<String>,
    level: Option<i64>,
    readme: Option<String>,
    channel: Channel<ProgressEvent>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
    let dir = Path::new(&dir);
    let total: u64 = names.iter().map(|n| crate::dirwalk::count_files_recursive(&dir.join(n))).sum();
    let reporter = ProgressReporter::new(channel, total.max(1));
    let done = std::cell::Cell::new(0u64);

    let file = std::fs::File::create(dir.join(&dest_name)).str_err()?;
    let mut zw = zip::ZipWriter::new(file);
    let mut options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    if let Some(level) = level {
        options = options.compression_level(Some(level));
    }
    if let Some(pw) = &password {
        options = options.with_aes_encryption(zip::AesMode::Aes256, pw);
    }
    for name in &names {
        zip_add_recursive(&mut zw, dir, &dir.join(name), options, &done, &reporter)
            .str_err()?;
    }
    if let Some(readme) = &readme {
        zw.start_file("README.txt", options)
            .str_err()?;
        zw.write_all(readme.as_bytes()).str_err()?;
    }
    zw.finish().str_err()?;
    Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The tar.gz alternative to `fs_compress` -- real filesystem only (a
/// vault's contents can't be piped through the system `tar` binary
/// without writing decrypted bytes to disk first, which defeats the
/// point of a vault, so vault compression stays zip-only, done in
/// process by the same `zip` crate as `compress_entries`). No password
/// support: unlike zip, .tar.gz has no standard, portable encryption --
/// anyone wanting a protected archive should pick zip instead.
#[tauri::command]
pub(crate) async fn fs_compress_targz(
    dir: String,
    names: Vec<String>,
    dest_name: String,
    channel: Channel<ProgressEvent>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
    let total: u64 = names.iter().map(|n| crate::dirwalk::count_files_recursive(&Path::new(&dir).join(n))).sum();
    let reporter = ProgressReporter::new(channel, total.max(1));
    let output = std::process::Command::new("tar")
        .current_dir(&dir)
        .arg("czf")
        .arg(&dest_name)
        .arg("--")
        .args(&names)
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    reporter.report(total.max(1));
    Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Extract every entry of the zip archive at `zip_path` into `dest_dir`.
/// `password` unlocks an AES-encrypted archive; pass `None` for a plain one.
#[tauri::command]
pub(crate) async fn fs_decompress(
    zip_path: String,
    dest_dir: String,
    password: Option<String>,
    channel: Channel<ProgressEvent>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
    let file = std::fs::File::open(&zip_path).str_err()?;
    let mut archive = zip::ZipArchive::new(file).str_err()?;
    let reporter = ProgressReporter::new(channel, (archive.len() as u64).max(1));
    for i in 0..archive.len() {
        let mut entry = match &password {
            Some(pw) => archive
                .by_index_decrypt(i, pw.as_bytes())
                .str_err()?,
            None => archive.by_index(i).str_err()?,
        };
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let dest_path = Path::new(&dest_dir).join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest_path).str_err()?;
        } else {
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent).str_err()?;
            }
            let mut out = std::fs::File::create(&dest_path).str_err()?;
            std::io::copy(&mut entry, &mut out).str_err()?;
        }
        reporter.report((i + 1) as u64);
    }
    Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
