//! Lets a zip/tar.gz be double-clicked and browsed exactly like a real
//! folder. Rather than a bespoke virtual filesystem, this just extracts
//! the archive into a scratch directory under the cache dir and hands
//! that directory back to the frontend as an ordinary `fs` location --
//! every existing folder feature (create, rename, copy, paste, New File,
//! etc.) already works against a real directory, so nothing else needs a
//! separate code path. `unmount` re-packs whatever's in the scratch
//! directory back over the original archive file when the frontend
//! navigates away from it (or any ancestor of it).
//!
//! Real-filesystem archives only -- same reasoning as `fs_compress_targz`:
//! doing this against a vault-sourced archive would mean writing decrypted
//! bytes out to a real scratch directory on disk, which defeats the point
//! of a vault.

use crate::errmap::{LockExt, ToStringErr};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Copy, PartialEq)]
enum ArchiveFormat {
    Zip,
    TarGz,
}

fn format_of(path: &str) -> Option<ArchiveFormat> {
    let lower = path.to_lowercase();
    if lower.ends_with(".zip") {
        Some(ArchiveFormat::Zip)
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some(ArchiveFormat::TarGz)
    } else {
        None
    }
}

struct Mount {
    archive_path: PathBuf,
    format: ArchiveFormat,
    password: Option<String>,
}

#[derive(Default)]
pub struct ArchiveMountState {
    mounts: Mutex<HashMap<String, Mount>>,
}

fn scratch_root() -> PathBuf {
    PathBuf::from(format!("{}/.cache/vaultexplorer/archive-mounts", crate::home_dir()))
}

fn rand_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    format!("{:x}-{:x}", std::process::id(), nanos)
}

/// Extract `archive_path` into a fresh scratch directory and return it.
/// `password` is only meaningful for an AES-encrypted zip; pass `None`
/// otherwise. On an encrypted zip opened without a password, the error
/// returned contains "password" (bubbled straight up from the `zip`
/// crate), matching the existing convention `decompressEntry` already
/// relies on to trigger a password prompt.
pub fn mount(state: &ArchiveMountState, archive_path: &str, password: Option<String>) -> Result<String, String> {
    let format = format_of(archive_path).ok_or_else(|| "not a supported archive".to_string())?;
    let root = scratch_root();
    std::fs::create_dir_all(&root).str_err()?;
    let dir = root.join(rand_id());
    std::fs::create_dir_all(&dir).str_err()?;

    let result = match format {
        ArchiveFormat::Zip => extract_zip(archive_path, &dir, password.as_deref()),
        ArchiveFormat::TarGz => extract_targz(archive_path, &dir),
    };
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }

    let mountpoint = dir.to_string_lossy().to_string();
    state.mounts.lock_safe().insert(
        mountpoint.clone(),
        Mount {
            archive_path: PathBuf::from(archive_path),
            format,
            password,
        },
    );
    Ok(mountpoint)
}

/// Re-pack the scratch directory back over the original archive, then
/// remove the scratch directory. A no-op (not an error) if `mountpoint`
/// isn't a live mount -- lets the frontend call this defensively.
pub fn unmount(state: &ArchiveMountState, mountpoint: &str) -> Result<(), String> {
    let Some(mount) = state.mounts.lock_safe().remove(mountpoint) else {
        return Ok(());
    };
    let dir = Path::new(mountpoint);
    let result = match mount.format {
        ArchiveFormat::Zip => repack_zip(dir, &mount.archive_path, mount.password.as_deref()),
        ArchiveFormat::TarGz => repack_targz(dir, &mount.archive_path),
    };
    let _ = std::fs::remove_dir_all(dir);
    result
}

/// Every currently active mount whose scratch directory is not `new_path`
/// or an ancestor of it -- these have just been navigated away from and
/// need flushing.
pub fn mounts_outside(state: &ArchiveMountState, new_path: &str) -> Vec<String> {
    state
        .mounts
        .lock()
        .unwrap()
        .keys()
        .filter(|mp| mp.as_str() != new_path && !new_path.starts_with(mp.as_str()))
        .cloned()
        .collect()
}

/// Every currently active mount, regardless of location -- used to flush
/// everything on app quit.
pub fn all_mounts(state: &ArchiveMountState) -> Vec<String> {
    state.mounts.lock_safe().keys().cloned().collect()
}

// ---- Tauri commands ----

/// Extract `path` into a scratch directory and return it -- the frontend
/// then just navigates into it like any other folder. `password` unlocks
/// an AES-encrypted zip; omit for a plain archive or a tar.gz.
#[tauri::command]
pub fn archive_mount(
    state: tauri::State<ArchiveMountState>,
    path: String,
    password: Option<String>,
) -> Result<String, String> {
    mount(&state, &path, password)
}

/// Re-pack a mounted archive's scratch directory back over the original
/// file and remove the scratch directory.
#[tauri::command]
pub fn archive_unmount(state: tauri::State<ArchiveMountState>, mountpoint: String) -> Result<(), String> {
    unmount(&state, &mountpoint)
}

/// Every currently mounted archive's scratch directory that `new_path`
/// isn't inside (or equal to) -- the frontend calls this on every
/// navigation and unmounts whatever comes back.
#[tauri::command]
pub fn archive_mounts_left_behind(
    state: tauri::State<ArchiveMountState>,
    new_path: String,
) -> Vec<String> {
    mounts_outside(&state, &new_path)
}

/// Every currently mounted archive's scratch directory, regardless of
/// location -- used to flush everything on app quit.
#[tauri::command]
pub fn archive_all_mounts(state: tauri::State<ArchiveMountState>) -> Vec<String> {
    all_mounts(&state)
}

fn extract_zip(archive_path: &str, dest_dir: &Path, password: Option<&str>) -> Result<(), String> {
    let file = std::fs::File::open(archive_path).str_err()?;
    let mut archive = zip::ZipArchive::new(file).str_err()?;
    for i in 0..archive.len() {
        let mut entry = match password {
            Some(pw) => archive.by_index_decrypt(i, pw.as_bytes()).str_err()?,
            None => archive.by_index(i).str_err()?,
        };
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let dest_path = dest_dir.join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest_path).str_err()?;
        } else {
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent).str_err()?;
            }
            let mut out = std::fs::File::create(&dest_path).str_err()?;
            std::io::copy(&mut entry, &mut out).str_err()?;
        }
    }
    Ok(())
}

fn zip_add_dir_recursive(
    zw: &mut zip::ZipWriter<std::fs::File>,
    base: &Path,
    path: &Path,
    options: zip::write::FileOptions<'_, ()>,
) -> std::io::Result<()> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            zip_add_dir_recursive(zw, base, &entry?.path(), options)?;
        }
    } else {
        let rel = path.strip_prefix(base).unwrap_or(path);
        let name = rel.to_string_lossy().replace('\\', "/");
        zw.start_file(name, options).map_err(std::io::Error::other)?;
        std::io::copy(&mut std::fs::File::open(path)?, zw)?;
    }
    Ok(())
}

fn repack_zip(src_dir: &Path, archive_path: &Path, password: Option<&str>) -> Result<(), String> {
    let tmp_path = PathBuf::from(format!("{}.tmp-repack", archive_path.display()));
    {
        let file = std::fs::File::create(&tmp_path).str_err()?;
        let mut zw = zip::ZipWriter::new(file);
        let mut options =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        if let Some(pw) = password {
            options = options.with_aes_encryption(zip::AesMode::Aes256, pw);
        }
        for entry in std::fs::read_dir(src_dir).str_err()? {
            let entry = entry.str_err()?;
            zip_add_dir_recursive(&mut zw, src_dir, &entry.path(), options).str_err()?;
        }
        zw.finish().str_err()?;
    }
    std::fs::rename(&tmp_path, archive_path).str_err()?;
    Ok(())
}

fn extract_targz(archive_path: &str, dest_dir: &Path) -> Result<(), String> {
    let output = std::process::Command::new("tar")
        .arg("xzf")
        .arg(archive_path)
        .arg("-C")
        .arg(dest_dir)
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

fn repack_targz(src_dir: &Path, archive_path: &Path) -> Result<(), String> {
    let tmp_path = PathBuf::from(format!("{}.tmp-repack", archive_path.display()));
    let entries: Vec<String> = std::fs::read_dir(src_dir)
        .str_err()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    let mut cmd = std::process::Command::new("tar");
    cmd.current_dir(src_dir).arg("czf").arg(&tmp_path);
    if entries.is_empty() {
        // tar refuses "-- " with zero file operands ("Cowardly refusing
        // to create an empty archive") -- every dir/file was deleted
        // while browsing, so hand it an explicitly empty file list.
        cmd.arg("--files-from=/dev/null");
    } else {
        cmd.arg("--").args(&entries);
    }
    let output = cmd.output().str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    std::fs::rename(&tmp_path, archive_path).str_err()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Round-trips a plain zip through mount -> edit -> unmount -> re-open,
    /// confirming edits made in the scratch directory (add, delete, modify)
    /// really do land back in the original archive file.
    #[test]
    fn zip_mount_edit_unmount_roundtrip() {
        let zip_path = format!("/tmp/ve-archive-test-{}.zip", std::process::id());
        let _ = std::fs::remove_file(&zip_path);
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zw.start_file("a.txt", options).unwrap();
            zw.write_all(b"hello").unwrap();
            zw.start_file("sub/b.txt", options).unwrap();
            zw.write_all(b"world").unwrap();
            zw.finish().unwrap();
        }

        let state = ArchiveMountState::default();
        let mountpoint = mount(&state, &zip_path, None).expect("mount failed");
        assert_eq!(std::fs::read_to_string(format!("{mountpoint}/a.txt")).unwrap(), "hello");
        assert_eq!(std::fs::read_to_string(format!("{mountpoint}/sub/b.txt")).unwrap(), "world");

        assert!(mounts_outside(&state, "/some/unrelated/path").contains(&mountpoint));
        assert!(!mounts_outside(&state, &mountpoint).contains(&mountpoint));
        assert!(!mounts_outside(&state, &format!("{mountpoint}/sub")).contains(&mountpoint));

        std::fs::remove_file(format!("{mountpoint}/a.txt")).unwrap();
        std::fs::write(format!("{mountpoint}/sub/b.txt"), b"WORLD").unwrap();
        std::fs::write(format!("{mountpoint}/c.txt"), b"brand new").unwrap();

        unmount(&state, &mountpoint).expect("unmount failed");
        assert!(!Path::new(&mountpoint).exists(), "scratch dir must be cleaned up");
        assert!(state.mounts.lock_safe().is_empty());

        let file = std::fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(!names.iter().any(|n| n == "a.txt"), "deleted file must not survive repack");
        assert!(names.iter().any(|n| n == "c.txt"), "new file must survive repack");
        let mut b = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("sub/b.txt").unwrap(), &mut b).unwrap();
        assert_eq!(b, "WORLD", "edited file must survive repack");

        let _ = std::fs::remove_file(&zip_path);
    }

    /// Same round-trip, but for an AES-password-protected zip -- also
    /// confirms mounting with no/wrong password surfaces a "password"
    /// error (the signal `decompressEntry`'s existing frontend flow keys
    /// off of to show a password prompt).
    #[test]
    fn zip_mount_with_password_roundtrip() {
        let zip_path = format!("/tmp/ve-archive-test-pw-{}.zip", std::process::id());
        let _ = std::fs::remove_file(&zip_path);
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .with_aes_encryption(zip::AesMode::Aes256, "s3cret");
            zw.start_file("secret.txt", options).unwrap();
            zw.write_all(b"top secret").unwrap();
            zw.finish().unwrap();
        }

        let state = ArchiveMountState::default();
        let err = mount(&state, &zip_path, None).expect_err("should require a password");
        assert!(err.to_lowercase().contains("password"), "error was: {err}");

        let mountpoint = mount(&state, &zip_path, Some("s3cret".to_string())).expect("mount failed");
        assert_eq!(std::fs::read_to_string(format!("{mountpoint}/secret.txt")).unwrap(), "top secret");
        std::fs::write(format!("{mountpoint}/secret.txt"), b"even more secret").unwrap();
        unmount(&state, &mountpoint).expect("unmount failed");

        let file = std::fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut out = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name_decrypt("secret.txt", b"s3cret").unwrap(),
            &mut out,
        )
        .unwrap();
        assert_eq!(out, "even more secret");

        let _ = std::fs::remove_file(&zip_path);
    }

    /// Same round-trip for tar.gz, including the empty-archive edge case
    /// (every file deleted while browsing) that plain `tar czf` refuses.
    #[test]
    fn targz_mount_edit_unmount_roundtrip() {
        let archive_path = format!("/tmp/ve-archive-test-{}.tar.gz", std::process::id());
        let src_dir = format!("/tmp/ve-archive-test-{}-src", std::process::id());
        let _ = std::fs::remove_file(&archive_path);
        let _ = std::fs::remove_dir_all(&src_dir);
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(format!("{src_dir}/only.txt"), b"only file").unwrap();
        let status = std::process::Command::new("tar")
            .current_dir(&src_dir)
            .arg("czf")
            .arg(&archive_path)
            .arg("only.txt")
            .status()
            .unwrap();
        assert!(status.success());

        let state = ArchiveMountState::default();
        let mountpoint = mount(&state, &archive_path, None).expect("mount failed");
        assert_eq!(std::fs::read_to_string(format!("{mountpoint}/only.txt")).unwrap(), "only file");

        // Delete the only file -- exercises the empty-archive edge case.
        std::fs::remove_file(format!("{mountpoint}/only.txt")).unwrap();
        unmount(&state, &mountpoint).expect("unmount of now-empty archive failed");

        let mountpoint2 = mount(&state, &archive_path, None).expect("re-mount of empty archive failed");
        let remaining: Vec<_> = std::fs::read_dir(&mountpoint2).unwrap().collect();
        assert!(remaining.is_empty(), "repacked archive should now be empty");

        let _ = std::fs::remove_dir_all(&mountpoint2);
        let _ = std::fs::remove_file(&archive_path);
        let _ = std::fs::remove_dir_all(&src_dir);
    }
}
