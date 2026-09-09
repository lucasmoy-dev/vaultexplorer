//! Where a deleted file goes, and how it comes back.
//!
//! Syncing a deletion is the one thing that cannot be undone by syncing again,
//! so nothing here ever destroys a file: the engine hands each one over as it
//! is about to disappear, and it lands somewhere a person already knows how to
//! look — the desktop's own recycle bin.
//!
//! On a phone there is no such place. Android has no recycle bin an app may
//! write to in the background, so there the engine keeps deletions in the
//! hidden `.stversions` folder beside the files, and this module reads both.
//! To the person using it the two look the same: a list of what was deleted,
//! and a button to put it back.

use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Where the engine keeps deletions when there is no recycle bin to use.
pub const VERSIONS_DIR: &str = ".stversions";

/// One file that was deleted and can still be brought back.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedFile {
    /// Where the copy is now. Also what [`restore`] is given back.
    pub id: String,
    /// The name a person would recognise, e.g. "IMG_0421.jpg".
    pub name: String,
    /// Where it will go back to.
    pub original_path: String,
    /// When it was deleted, in seconds since the epoch.
    pub deleted_at: u64,
    pub bytes: u64,
    /// True when it is in the system's recycle bin, false when it is in the
    /// hidden folder the engine keeps. Only worth saying so the interface can
    /// explain where the file actually is.
    pub in_system_bin: bool,
}

/// Moves a file to the recycle bin, and says where it ended up.
///
/// This is the whole of what the engine calls when another device deletes
/// something: the file is handed over before it is destroyed, and putting it
/// in the same bin the file manager uses means "recuperar" is something the
/// user already knows how to do, with or without this app.
pub fn move_to_trash(path: &Path) -> Result<PathBuf> {
    let home = std::env::var("HOME").map(PathBuf::from).map_err(|_| {
        Error::Engine("there is no HOME, so there is no recycle bin to use".into())
    })?;
    move_into_bin(path, &home)
}

/// Everything deleted out of `folder` that can still be recovered, newest first.
pub fn deleted_in(folder: &Path) -> Vec<DeletedFile> {
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    let mut found = from_bins(folder, &home);
    found.extend(from_versions(folder));
    found.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at));
    found
}

/// Puts one file back where it was deleted from.
///
/// Restoring is deliberately a plain move back into the folder: the engine
/// then sees a file that is there again and sends it to the other devices, so
/// recovering on one device recovers it everywhere.
pub fn restore(id: &str) -> Result<PathBuf> {
    let copy = PathBuf::from(id);
    let target = destination_of(id)?;
    if target.exists() {
        return Err(Error::Engine(
            "ya hay un fichero con ese nombre en la carpeta".into(),
        ));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    move_file(&copy, &target)?;
    if let Some(info) = trash_info_for(&copy) {
        let _ = fs::remove_file(info);
    }
    Ok(target)
}

/// Where a copy would go back to, without moving anything.
///
/// Asked first by the caller, which checks the answer is inside the folder the
/// user is looking at: an id is a path, and a path from somewhere else would
/// otherwise write wherever it pleased.
pub fn destination_of(id: &str) -> Result<PathBuf> {
    let copy = PathBuf::from(id);
    if !copy.exists() {
        return Err(Error::Engine("esa copia ya no está donde estaba".into()));
    }
    match trash_info_for(&copy) {
        Some(info) => original_path_in(&info)
            .ok_or_else(|| Error::Engine("la papelera no dice de dónde salió".into())),
        None => version_origin(&copy)
            .ok_or_else(|| Error::Engine("no se sabe a qué carpeta pertenece".into())),
    }
}

// ---- the desktop's recycle bin ----------------------------------------

/// Moves one file into the right bin for the volume it lives on.
///
/// Split from [`move_to_trash`] so the tests can point it at a home of their
/// own instead of the one running the tests.
fn move_into_bin(path: &Path, home: &Path) -> Result<PathBuf> {
    let path = absolute(path);
    let bin = bin_for(&path, home)?;
    fs::create_dir_all(bin.join("files"))?;
    fs::create_dir_all(bin.join("info"))?;

    let wanted = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sin-nombre".into());
    let name = free_name(&bin, &wanted);

    // The record goes first: a copy in the bin that nothing describes is a
    // file nobody can put back.
    let mut info = fs::File::create(bin.join("info").join(format!("{name}.trashinfo")))?;
    writeln!(info, "[Trash Info]")?;
    writeln!(info, "Path={}", encode(&path.to_string_lossy()))?;
    writeln!(info, "DeletionDate={}", now_local())?;

    let landed = bin.join("files").join(&name);
    move_file(&path, &landed)?;
    Ok(landed)
}

/// The bin a file on this path belongs in.
///
/// A file on another disk cannot be moved into the home bin without copying
/// the whole thing across, so every volume is allowed its own `.Trash-<uid>` —
/// which is also where the file manager will look for it.
fn bin_for(path: &Path, home: &Path) -> Result<PathBuf> {
    let home_bin = home.join(".local/share/Trash");
    let parent = path.parent().unwrap_or(Path::new("/"));
    let same_disk = device_of(parent)
        .zip(device_of(home))
        .map(|(a, b)| a == b)
        .unwrap_or(false);
    if same_disk {
        return Ok(home_bin);
    }
    let root = mount_root(parent);
    Ok(root.join(format!(".Trash-{}", unsafe { libc::getuid() })))
}

/// Reads every bin that could hold something from this folder.
fn from_bins(folder: &Path, home: &Path) -> Vec<DeletedFile> {
    let folder = absolute(folder);
    let mut bins = vec![home.join(".local/share/Trash")];
    let volume = mount_root(&folder).join(format!(".Trash-{}", unsafe { libc::getuid() }));
    if !bins.contains(&volume) {
        bins.push(volume);
    }

    let mut found = Vec::new();
    for bin in bins {
        let Ok(entries) = fs::read_dir(bin.join("info")) else { continue };
        for entry in entries.flatten() {
            let info = entry.path();
            if info.extension().and_then(|e| e.to_str()) != Some("trashinfo") {
                continue;
            }
            let Some(original) = original_path_in(&info) else { continue };
            // Only what came out of this folder. Everything else in the bin is
            // the user's own business.
            if !original.starts_with(&folder) {
                continue;
            }
            let name = info.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let copy = bin.join("files").join(name.trim_end_matches(".trashinfo"));
            let Ok(meta) = fs::metadata(&copy) else { continue };
            found.push(DeletedFile {
                id: copy.to_string_lossy().into_owned(),
                name: original
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                original_path: original.to_string_lossy().into_owned(),
                deleted_at: changed_at(&meta),
                bytes: meta.len(),
                in_system_bin: true,
            });
        }
    }
    found
}

// ---- what the engine keeps when there is no bin -------------------------

/// Reads the hidden folder the engine fills on a phone.
fn from_versions(folder: &Path) -> Vec<DeletedFile> {
    let root = absolute(folder).join(VERSIONS_DIR);
    let mut found = Vec::new();
    walk(&root, &mut |file, meta| {
        let Ok(relative) = file.strip_prefix(&root) else { return };
        let original = absolute(folder).join(strip_stamp(relative));
        found.push(DeletedFile {
            id: file.to_string_lossy().into_owned(),
            name: original
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            original_path: original.to_string_lossy().into_owned(),
            // When it was deleted, which is in the stamp the engine puts in
            // the name — not the file's own time, which it keeps from the
            // original and which had the list saying a file deleted a minute
            // ago went "hace 3 h".
            deleted_at: stamp_seconds(file).unwrap_or_else(|| changed_at(meta)),
            bytes: meta.len(),
            in_system_bin: false,
        });
    });
    found
}

/// Where a copy in the hidden folder came from.
fn version_origin(copy: &Path) -> Option<PathBuf> {
    let text = copy.to_string_lossy();
    let (folder, relative) = text.split_once(&format!("/{VERSIONS_DIR}/"))?;
    Some(Path::new(folder).join(strip_stamp(Path::new(relative))))
}

/// The moment in `notas~20260909-213159.txt`, in seconds since the epoch.
/// The stamp is written in local time, so it is read back as local time.
fn stamp_seconds(file: &Path) -> Option<u64> {
    let name = file.file_name()?.to_str()?;
    let stamp: String = name.split_once('~')?.1.chars().take(15).collect();
    if stamp.len() != 15 || stamp.as_bytes()[8] != b'-' {
        return None;
    }
    let number = |from: usize, to: usize| stamp.get(from..to)?.parse::<i32>().ok();
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    parts.tm_year = number(0, 4)? - 1900;
    parts.tm_mon = number(4, 6)? - 1;
    parts.tm_mday = number(6, 8)?;
    parts.tm_hour = number(9, 11)?;
    parts.tm_min = number(11, 13)?;
    parts.tm_sec = number(13, 15)?;
    // Let the C library work out whether summer time was in force that day.
    parts.tm_isdst = -1;
    let seconds = unsafe { libc::mktime(&mut parts) };
    (seconds > 0).then_some(seconds as u64)
}

/// `notas~20260909-213159.txt` is how the engine stamps a kept copy. The name
/// a person is looking for is the one without it.
fn strip_stamp(relative: &Path) -> PathBuf {
    let Some(name) = relative.file_name().and_then(|n| n.to_str()) else {
        return relative.to_path_buf();
    };
    let Some((before, after)) = name.split_once('~') else {
        return relative.to_path_buf();
    };
    // Only a real stamp counts: eight digits, a dash, six digits.
    let stamp: String = after.chars().take(15).collect();
    let looks_like_a_stamp = stamp.len() == 15
        && stamp[..8].chars().all(|c| c.is_ascii_digit())
        && stamp[8..9] == *"-"
        && stamp[9..].chars().all(|c| c.is_ascii_digit());
    if !looks_like_a_stamp {
        return relative.to_path_buf();
    }
    let rest = &after[15..];
    let rebuilt = format!("{before}{rest}");
    match relative.parent() {
        Some(parent) => parent.join(rebuilt),
        None => PathBuf::from(rebuilt),
    }
}

// ---- odds and ends -----------------------------------------------------

fn trash_info_for(copy: &Path) -> Option<PathBuf> {
    let name = copy.file_name()?.to_str()?;
    let bin = copy.parent()?.parent()?;
    let info = bin.join("info").join(format!("{name}.trashinfo"));
    info.exists().then_some(info)
}

fn original_path_in(info: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(info).ok()?;
    let line = text.lines().find_map(|l| l.strip_prefix("Path="))?;
    Some(PathBuf::from(decode(line.trim())))
}

/// A name nothing else in the bin is using.
fn free_name(bin: &Path, wanted: &str) -> String {
    let mut name = wanted.to_string();
    let mut attempt = 2;
    while bin.join("files").join(&name).exists() || bin.join("info").join(format!("{name}.trashinfo")).exists() {
        let (stem, extension) = match wanted.split_once('.') {
            Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
            None => (wanted.to_string(), String::new()),
        };
        name = format!("{stem}.{attempt}{extension}");
        attempt += 1;
    }
    name
}

/// A rename where possible, a copy where the two ends are on different disks.
fn move_file(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to)?;
    fs::remove_file(from)?;
    Ok(())
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir().unwrap_or_default().join(path)
}

fn device_of(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|m| m.dev())
}

/// Walks up until the disk changes: that is where the volume begins.
fn mount_root(path: &Path) -> PathBuf {
    let Some(device) = device_of(path) else { return PathBuf::from("/") };
    let mut root = absolute(path);
    while let Some(parent) = root.parent() {
        if device_of(parent) != Some(device) {
            break;
        }
        root = parent.to_path_buf();
    }
    root
}

fn changed_at(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        })
}

fn walk(dir: &Path, found: &mut impl FnMut(&Path, &fs::Metadata)) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            walk(&path, found);
        } else {
            found(&path, &meta);
        }
    }
}

/// The local time, in the shape the recycle bin's own records use.
fn now_local() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as libc::time_t;
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    let mut buffer = [0u8; 32];
    unsafe {
        libc::localtime_r(&seconds, &mut parts);
        let format = b"%Y-%m-%dT%H:%M:%S\0";
        libc::strftime(
            buffer.as_mut_ptr() as *mut libc::c_char,
            buffer.len(),
            format.as_ptr() as *const libc::c_char,
            &parts,
        );
    }
    String::from_utf8_lossy(&buffer)
        .trim_end_matches('\0')
        .to_string()
}

/// Percent-encoding, as the recycle bin's records are written. The separators
/// stay readable; everything else that is not plain is escaped.
fn encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        let plain = byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~');
        if plain {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&text[index + 1..index + 3], 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder and a home of their own, so the tests never touch the real bin.
    fn scratch(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("homecloud-trash-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        let folder = root.join("home/Fotos");
        fs::create_dir_all(&folder).unwrap();
        (folder, home)
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn a_deleted_file_goes_to_the_bin_and_says_where_it_came_from() {
        let (folder, home) = scratch("basics");
        let file = folder.join("una foto.jpg");
        write(&file, "unos bytes");

        let landed = move_into_bin(&file, &home).expect("must reach the bin");
        assert!(landed.exists(), "the copy must be in the bin");
        assert!(!file.exists(), "and gone from the folder");

        let deleted = from_bins(&folder, &home);
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].name, "una foto.jpg");
        assert_eq!(deleted[0].original_path, file.to_string_lossy());
        assert!(deleted[0].in_system_bin);
    }

    #[test]
    fn restoring_puts_it_back_where_it_was() {
        let (folder, home) = scratch("restore");
        let file = folder.join("cuentas/enero.txt");
        write(&file, "hola");

        move_into_bin(&file, &home).unwrap();
        let deleted = from_bins(&folder, &home);
        let back = restore(&deleted[0].id).expect("must come back");

        assert_eq!(back, file);
        assert_eq!(fs::read_to_string(&file).unwrap(), "hola");
        assert!(from_bins(&folder, &home).is_empty(), "and leave the bin empty");
    }

    #[test]
    fn two_files_with_the_same_name_both_survive() {
        let (folder, home) = scratch("collision");
        let file = folder.join("nota.txt");
        write(&file, "primera");
        move_into_bin(&file, &home).unwrap();
        write(&file, "segunda");
        move_into_bin(&file, &home).unwrap();

        let deleted = from_bins(&folder, &home);
        assert_eq!(deleted.len(), 2, "the second must not overwrite the first");
        let kept: Vec<String> = deleted
            .iter()
            .map(|d| fs::read_to_string(&d.id).unwrap())
            .collect();
        assert!(kept.contains(&"primera".to_string()));
        assert!(kept.contains(&"segunda".to_string()));
    }

    #[test]
    fn other_peoples_rubbish_is_not_listed_as_this_folders() {
        let (folder, home) = scratch("filter");
        let elsewhere = home.join("Descargas/factura.pdf");
        write(&elsewhere, "no es de la carpeta");
        move_into_bin(&elsewhere, &home).unwrap();

        assert!(
            from_bins(&folder, &home).is_empty(),
            "only what came out of this folder belongs in its list"
        );
    }

    #[test]
    fn the_phones_hidden_copies_are_listed_and_restored_too() {
        let (folder, _home) = scratch("versions");
        // What the engine writes on a phone: the same path under .stversions,
        // with a stamp in the name.
        let kept = folder.join(VERSIONS_DIR).join("viaje/playa~20260909-213159.jpg");
        write(&kept, "una foto");

        let deleted = from_versions(&folder);
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].name, "playa.jpg");
        assert!(!deleted[0].in_system_bin);

        let back = restore(&deleted[0].id).unwrap();
        assert_eq!(back, folder.join("viaje/playa.jpg"));
        assert_eq!(fs::read_to_string(back).unwrap(), "una foto");
    }

    #[test]
    fn a_kept_copy_is_dated_when_it_was_deleted_not_when_it_was_written() {
        let (folder, _home) = scratch("stamp");
        let kept = folder.join(VERSIONS_DIR).join("f1~20260909-215830.bin");
        write(&kept, "unos bytes");
        // The engine keeps the original file's own time on the copy, so
        // reading that instead of the stamp had a file deleted a minute ago
        // showing as deleted hours earlier.
        let listed = &from_versions(&folder)[0];
        let stamp = stamp_seconds(Path::new("f1~20260909-215830.bin")).expect("must parse");
        assert_eq!(listed.deleted_at, stamp);
        assert_ne!(listed.deleted_at, changed_at(&fs::metadata(&kept).unwrap()));
    }

    #[test]
    fn a_name_with_a_tilde_in_it_is_not_mistaken_for_a_stamp() {
        assert_eq!(
            strip_stamp(Path::new("copia~final.txt")),
            PathBuf::from("copia~final.txt")
        );
        assert_eq!(
            strip_stamp(Path::new("copia~20260909-213159.txt")),
            PathBuf::from("copia.txt")
        );
    }

    #[test]
    fn paths_survive_being_written_into_the_bins_records() {
        let path = "/home/lucas/Fotos/año nuevo & fiesta/ñandú.jpg";
        assert_eq!(decode(&encode(path)), path);
        assert!(encode(path).contains("%20"), "spaces must be escaped");
    }

    #[test]
    fn restoring_over_something_that_is_back_already_is_refused() {
        let (folder, home) = scratch("conflict");
        let file = folder.join("nota.txt");
        write(&file, "vieja");
        move_into_bin(&file, &home).unwrap();
        write(&file, "una nueva con el mismo nombre");

        let deleted = from_bins(&folder, &home);
        assert!(
            restore(&deleted[0].id).is_err(),
            "bringing one back must never overwrite the file that is there"
        );
    }
}
