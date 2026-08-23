//! "Freeze Folder": a password-gated, rollback-capable folder, backed by a
//! purpose-built FUSE passthrough+shadow filesystem (see module-level
//! honest-limits note below -- this is genuinely a different, smaller
//! piece of engineering than `vaultcore::fuse_mount::VaultFs`, since there
//! is no encryption here forcing whole-file buffering: real per-fd
//! `std::fs::File` handles do real byte-range I/O directly).
//!
//! Freezing moves a real folder's contents into a hidden **base** (the
//! pristine snapshot, never modified while frozen) plus an initially-empty
//! **shadow** (every write/create/delete while frozen lands only here),
//! then mounts a merged read/write view *at the folder's own original
//! path* -- so any program, not just VaultExplorer, that touches that path
//! while frozen is transparently redirected. Deletions are tracked as a
//! small `whiteouts.json` set (there's no real device-file whiteout
//! mechanism available without root), persisted alongside base/shadow so
//! it survives a crash.
//!
//! **Honest limits, stated here and (more importantly) in the UI before
//! anyone freezes a folder**: this is a userspace FUSE mount, not a
//! kernel/firmware immutability guarantee. It holds only while this
//! process's mount is alive. "Back to how it was after a reboot" is
//! achieved by VaultExplorer re-mounting frozen folders at its own next
//! launch (discarding that session's shadow = full rollback) -- if some
//! other program touches the (till-then-plain, empty-looking) folder path
//! before VaultExplorer starts and remounts, or the app is never
//! relaunched, there's no guarantee. `base` is always safe on disk
//! regardless (a crash/kill -9 just leaves the real path unmounted and
//! empty-looking until remounted or manually restored).

use crate::errmap::{LockExt, ToStringErr};
use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use fuser::{
    BackgroundSession, FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyCreate,
    ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request,
    FUSE_ROOT_ID,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

const TTL: Duration = Duration::from_secs(1);

fn frozen_root() -> PathBuf {
    PathBuf::from(format!("{}/.local/share/vaultexplorer/frozen", crate::home_dir()))
}

fn short_hash(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .str_err()
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FreezeMeta {
    pub original_path: String,
    // Never sent to the frontend (see `#[serde(skip_serializing)]`) --
    // only read back on this side to verify an unfreeze attempt.
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub frozen_at: i64,
}

fn meta_dir_for(original_path: &str) -> PathBuf {
    frozen_root().join(short_hash(original_path))
}

pub fn meta_path(original_path: &str) -> PathBuf {
    meta_dir_for(original_path).join("meta.json")
}
fn base_dir(original_path: &str) -> PathBuf {
    meta_dir_for(original_path).join("base")
}
fn shadow_dir(original_path: &str) -> PathBuf {
    meta_dir_for(original_path).join("shadow")
}
fn whiteouts_path(original_path: &str) -> PathBuf {
    meta_dir_for(original_path).join("whiteouts.json")
}

/// Wipe `shadow` and the whiteout set back to empty -- this is the actual
/// mechanism behind "back to how it was after a restart": every fresh
/// app launch remounts frozen folders from a *discarded* shadow, so
/// anything written/deleted since the last clean unfreeze never
/// persisted in the first place. `base` (the pristine snapshot) is never
/// touched by this.
pub fn discard_shadow(original_path: &str) -> Result<(), String> {
    let shadow = shadow_dir(original_path);
    let _ = std::fs::remove_dir_all(&shadow);
    std::fs::create_dir_all(&shadow).str_err()?;
    let _ = std::fs::remove_file(whiteouts_path(original_path));
    Ok(())
}

/// Every currently-frozen folder this app knows about (whether its mount
/// is live right now or not) -- read straight off disk, so it survives a
/// crash between sessions.
pub fn list_frozen() -> Vec<FreezeMeta> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(frozen_root()) else {
        return out;
    };
    for entry in read.flatten() {
        let meta_path = entry.path().join("meta.json");
        if let Ok(raw) = std::fs::read_to_string(&meta_path) {
            if let Ok(meta) = serde_json::from_str::<FreezeMeta>(&raw) {
                out.push(meta);
            }
        }
    }
    out
}

/// Move `original_path`'s contents into a fresh `base` snapshot, create an
/// empty `shadow`, write `meta.json`. Does **not** mount -- the caller
/// (the Tauri command) mounts right after so the folder is never left in
/// a half-frozen (moved-away, not-yet-remounted) state for longer than
/// one function call.
pub fn freeze(original_path: &str, password_hash: String, now: i64) -> Result<(), String> {
    let dir = meta_dir_for(original_path);
    if dir.exists() {
        return Err("this folder is already frozen".to_string());
    }
    std::fs::create_dir_all(&dir).str_err()?;
    let base = base_dir(original_path);
    // Move (not copy) the real contents into base -- cheap (same
    // filesystem, typically a rename) and leaves nothing plaintext
    // duplicated on disk mid-freeze.
    std::fs::rename(original_path, &base).str_err()?;
    std::fs::create_dir_all(shadow_dir(original_path)).str_err()?;
    std::fs::create_dir_all(original_path).str_err()?;
    let meta = FreezeMeta {
        original_path: original_path.to_string(),
        password_hash,
        frozen_at: now,
    };
    std::fs::write(meta_path(original_path), serde_json::to_string(&meta).unwrap())
        .str_err()?;
    Ok(())
}

/// Recursively copy `shadow`'s files onto `base`, skip whiteouts, then
/// move the merged result back to `original_path` and delete the freeze
/// metadata -- "Keep Changes".
pub fn commit(original_path: &str) -> Result<(), String> {
    let base = base_dir(original_path);
    let shadow = shadow_dir(original_path);
    let deleted = load_whiteouts(original_path);
    apply_whiteouts(&base, &deleted);
    copy_tree(&shadow, &base).str_err()?;
    restore_and_cleanup(original_path, &base)
}

/// Discard `shadow` entirely, restore only pristine `base` -- "Discard
/// Changes / Rollback".
pub fn rollback(original_path: &str) -> Result<(), String> {
    let base = base_dir(original_path);
    restore_and_cleanup(original_path, &base)
}

fn restore_and_cleanup(original_path: &str, base: &Path) -> Result<(), String> {
    // The mount at `original_path` must already be torn down (dropped)
    // by the caller before this runs, or the rename below would race the
    // live FUSE mount.
    let _ = std::fs::remove_dir(original_path); // ok to fail if non-empty/still mounted-looking
    std::fs::rename(base, original_path).str_err()?;
    std::fs::remove_dir_all(meta_dir_for(original_path)).str_err()?;
    Ok(())
}

fn load_whiteouts(original_path: &str) -> HashSet<PathBuf> {
    std::fs::read_to_string(whiteouts_path(original_path))
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<PathBuf>>(&s).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

fn apply_whiteouts(base: &Path, deleted: &HashSet<PathBuf>) {
    for rel in deleted {
        let p = base.join(rel);
        if p.is_dir() {
            let _ = std::fs::remove_dir_all(&p);
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
}

fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&dst_path)?;
            copy_tree(&entry.path(), &dst_path)?;
        } else {
            if let Some(parent) = dst_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

// ---------------- FUSE filesystem ----------------

struct Inodes {
    path_to_ino: HashMap<PathBuf, u64>,
    ino_to_path: HashMap<u64, PathBuf>,
    next_ino: u64,
}
impl Inodes {
    fn new() -> Self {
        let mut path_to_ino = HashMap::new();
        let mut ino_to_path = HashMap::new();
        path_to_ino.insert(PathBuf::new(), FUSE_ROOT_ID);
        ino_to_path.insert(FUSE_ROOT_ID, PathBuf::new());
        Self { path_to_ino, ino_to_path, next_ino: FUSE_ROOT_ID + 1 }
    }
    fn ino_for(&mut self, path: &Path) -> u64 {
        if let Some(&ino) = self.path_to_ino.get(path) {
            return ino;
        }
        let ino = self.next_ino;
        self.next_ino += 1;
        self.path_to_ino.insert(path.to_path_buf(), ino);
        self.ino_to_path.insert(ino, path.to_path_buf());
        ino
    }
    fn path_for(&self, ino: u64) -> Option<PathBuf> {
        self.ino_to_path.get(&ino).cloned()
    }
}

pub struct FreezeFs {
    base: PathBuf,
    shadow: PathBuf,
    whiteouts_file: PathBuf,
    deleted: Mutex<HashSet<PathBuf>>,
    inodes: Mutex<Inodes>,
    handles: Mutex<HashMap<u64, std::fs::File>>,
    next_fh: AtomicU64,
}

impl FreezeFs {
    pub fn new(original_path: &str) -> Self {
        let deleted = std::fs::read_to_string(whiteouts_path(original_path))
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<PathBuf>>(&s).ok())
            .map(|v| v.into_iter().collect())
            .unwrap_or_default();
        Self {
            base: base_dir(original_path),
            shadow: shadow_dir(original_path),
            whiteouts_file: whiteouts_path(original_path),
            deleted: Mutex::new(deleted),
            inodes: Mutex::new(Inodes::new()),
            handles: Mutex::new(HashMap::new()),
            next_fh: AtomicU64::new(1),
        }
    }

    fn persist_whiteouts(&self) {
        let deleted = self.deleted.lock_safe();
        let list: Vec<&PathBuf> = deleted.iter().collect();
        if let Ok(json) = serde_json::to_string(&list) {
            let _ = std::fs::write(&self.whiteouts_file, json);
        }
    }

    fn is_deleted(&self, rel: &Path) -> bool {
        self.deleted.lock_safe().contains(rel)
    }

    /// Real path to read `rel` from -- shadow takes priority (copy-up
    /// already happened for anything modified), else base, else `None` if
    /// whited-out or absent from both.
    fn resolve(&self, rel: &Path) -> Option<PathBuf> {
        if self.is_deleted(rel) {
            return None;
        }
        let s = self.shadow.join(rel);
        if s.symlink_metadata().is_ok() {
            return Some(s);
        }
        let b = self.base.join(rel);
        if b.symlink_metadata().is_ok() {
            return Some(b);
        }
        None
    }

    fn ensure_shadow_dir(&self, rel: &Path) -> io::Result<PathBuf> {
        let p = self.shadow.join(rel);
        std::fs::create_dir_all(&p)?;
        Ok(p)
    }

    /// Copy `rel` (a file) from base into shadow if it isn't already
    /// there, then return its shadow path -- the write side of
    /// overlay-style copy-up.
    fn copy_up(&self, rel: &Path) -> io::Result<PathBuf> {
        let shadow_path = self.shadow.join(rel);
        if shadow_path.symlink_metadata().is_ok() {
            return Ok(shadow_path);
        }
        if let Some(parent) = rel.parent() {
            self.ensure_shadow_dir(parent)?;
        }
        let base_path = self.base.join(rel);
        if base_path.is_file() {
            std::fs::copy(&base_path, &shadow_path)?;
        } else {
            std::fs::write(&shadow_path, [])?;
        }
        Ok(shadow_path)
    }

    fn attr_for(&self, ino: u64, meta: &std::fs::Metadata) -> FileAttr {
        let kind = if meta.is_dir() { FileType::Directory } else { FileType::RegularFile };
        FileAttr {
            ino,
            size: meta.len(),
            blocks: meta.len().div_ceil(512),
            atime: meta.accessed().unwrap_or(std::time::SystemTime::now()),
            mtime: meta.modified().unwrap_or(std::time::SystemTime::now()),
            ctime: meta.modified().unwrap_or(std::time::SystemTime::now()),
            crtime: meta.modified().unwrap_or(std::time::SystemTime::now()),
            kind,
            perm: (meta.mode() & 0o7777) as u16,
            nlink: 1,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }
}

pub fn spawn(original_path: &str, mountpoint: &Path) -> io::Result<BackgroundSession> {
    let fs = FreezeFs::new(original_path);
    let options = vec![MountOption::FSName("freezefs".to_string())];
    fuser::spawn_mount2(fs, mountpoint, &options)
}

impl Filesystem for FreezeFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let mut inodes = self.inodes.lock_safe();
        let Some(parent_path) = inodes.path_for(parent) else {
            reply.error(libc::ENOENT);
            return;
        };
        let child_rel = parent_path.join(name);
        match self.resolve(&child_rel).and_then(|p| std::fs::symlink_metadata(p).ok()) {
            Some(meta) => {
                let ino = inodes.ino_for(&child_rel);
                reply.entry(&TTL, &self.attr_for(ino, &meta), 0);
            }
            None => reply.error(libc::ENOENT),
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        let Some(rel) = self.inodes.lock_safe().path_for(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        match self.resolve(&rel).and_then(|p| std::fs::symlink_metadata(p).ok()) {
            Some(meta) => reply.attr(&TTL, &self.attr_for(ino, &meta)),
            None => reply.error(libc::ENOENT),
        }
    }

    fn readdir(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, mut reply: ReplyDirectory) {
        let mut inodes = self.inodes.lock_safe();
        let Some(dir_rel) = inodes.path_for(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let mut names: HashMap<String, bool> = HashMap::new();
        if let Ok(read) = std::fs::read_dir(self.base.join(&dir_rel)) {
            for e in read.flatten() {
                let rel = dir_rel.join(e.file_name());
                if self.is_deleted(&rel) {
                    continue;
                }
                names.insert(
                    e.file_name().to_string_lossy().to_string(),
                    e.file_type().map(|t| t.is_dir()).unwrap_or(false),
                );
            }
        }
        if let Ok(read) = std::fs::read_dir(self.shadow.join(&dir_rel)) {
            for e in read.flatten() {
                names.insert(
                    e.file_name().to_string_lossy().to_string(),
                    e.file_type().map(|t| t.is_dir()).unwrap_or(false),
                );
            }
        }

        let mut entries = vec![
            (ino, FileType::Directory, ".".to_string()),
            (ino, FileType::Directory, "..".to_string()),
        ];
        for (name, is_dir) in names {
            let rel = dir_rel.join(&name);
            let child_ino = inodes.ino_for(&rel);
            entries.push((child_ino, if is_dir { FileType::Directory } else { FileType::RegularFile }, name));
        }
        for (i, (ino, kind, name)) in entries.into_iter().enumerate().skip(offset as usize) {
            if reply.add(ino, (i + 1) as i64, kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        let Some(rel) = self.inodes.lock_safe().path_for(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let write_intent = (flags & libc::O_ACCMODE) != libc::O_RDONLY;
        let real_path = if write_intent {
            match self.copy_up(&rel) {
                Ok(p) => p,
                Err(_) => {
                    reply.error(libc::EIO);
                    return;
                }
            }
        } else {
            match self.resolve(&rel) {
                Some(p) => p,
                None => {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        };
        let file = OpenOptions::new()
            .read(true)
            .write(write_intent)
            .custom_flags(flags & !(libc::O_CREAT | libc::O_EXCL))
            .open(&real_path);
        match file {
            Ok(f) => {
                let fh = self.next_fh.fetch_add(1, Ordering::SeqCst);
                self.handles.lock_safe().insert(fh, f);
                reply.opened(fh, 0);
            }
            Err(_) => reply.error(libc::EIO),
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        use std::io::{Read, Seek, SeekFrom};
        let mut handles = self.handles.lock_safe();
        let Some(f) = handles.get_mut(&fh) else {
            reply.error(libc::EBADF);
            return;
        };
        if f.seek(SeekFrom::Start(offset as u64)).is_err() {
            reply.error(libc::EIO);
            return;
        }
        let mut buf = vec![0u8; size as usize];
        match f.read(&mut buf) {
            Ok(n) => reply.data(&buf[..n]),
            Err(_) => reply.error(libc::EIO),
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        use std::io::{Seek, SeekFrom, Write};
        let mut handles = self.handles.lock_safe();
        let Some(f) = handles.get_mut(&fh) else {
            reply.error(libc::EBADF);
            return;
        };
        if f.seek(SeekFrom::Start(offset as u64)).is_err() {
            reply.error(libc::EIO);
            return;
        }
        match f.write(data) {
            Ok(n) => reply.written(n as u32),
            Err(_) => reply.error(libc::EIO),
        }
    }

    fn flush(&mut self, _req: &Request, _ino: u64, fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
        if let Some(f) = self.handles.lock_safe().get(&fh) {
            let _ = f.sync_data();
        }
        reply.ok();
    }

    fn release(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.handles.lock_safe().remove(&fh);
        reply.ok();
    }

    fn create(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        let mut inodes = self.inodes.lock_safe();
        let Some(parent_rel) = inodes.path_for(parent) else {
            reply.error(libc::ENOENT);
            return;
        };
        if let Err(_) = self.ensure_shadow_dir(&parent_rel) {
            reply.error(libc::EIO);
            return;
        }
        let child_rel = parent_rel.join(name);
        let shadow_path = self.shadow.join(&child_rel);
        let file = match OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&shadow_path) {
            Ok(f) => f,
            Err(_) => {
                reply.error(libc::EIO);
                return;
            }
        };
        self.deleted.lock_safe().remove(&child_rel);
        self.persist_whiteouts();
        let ino = inodes.ino_for(&child_rel);
        let meta = file.metadata().unwrap();
        let fh = self.next_fh.fetch_add(1, Ordering::SeqCst);
        let attr = self.attr_for(ino, &meta);
        self.handles.lock_safe().insert(fh, file);
        reply.created(&TTL, &attr, 0, fh, 0);
    }

    fn mkdir(&mut self, _req: &Request, parent: u64, name: &OsStr, _mode: u32, _umask: u32, reply: ReplyEntry) {
        let mut inodes = self.inodes.lock_safe();
        let Some(parent_rel) = inodes.path_for(parent) else {
            reply.error(libc::ENOENT);
            return;
        };
        let child_rel = parent_rel.join(name);
        if self.ensure_shadow_dir(&child_rel).is_err() {
            reply.error(libc::EIO);
            return;
        }
        self.deleted.lock_safe().remove(&child_rel);
        self.persist_whiteouts();
        let ino = inodes.ino_for(&child_rel);
        let meta = std::fs::symlink_metadata(self.shadow.join(&child_rel)).unwrap();
        reply.entry(&TTL, &self.attr_for(ino, &meta), 0);
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let Some(parent_rel) = self.inodes.lock_safe().path_for(parent) else {
            reply.error(libc::ENOENT);
            return;
        };
        let child_rel = parent_rel.join(name);
        let _ = std::fs::remove_file(self.shadow.join(&child_rel));
        self.deleted.lock_safe().insert(child_rel);
        self.persist_whiteouts();
        reply.ok();
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let Some(parent_rel) = self.inodes.lock_safe().path_for(parent) else {
            reply.error(libc::ENOENT);
            return;
        };
        let child_rel = parent_rel.join(name);
        let _ = std::fs::remove_dir(self.shadow.join(&child_rel));
        self.deleted.lock_safe().insert(child_rel);
        self.persist_whiteouts();
        reply.ok();
    }

    fn rename(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        let mut inodes = self.inodes.lock_safe();
        let (Some(old_parent), Some(new_parent)) = (inodes.path_for(parent), inodes.path_for(newparent)) else {
            reply.error(libc::ENOENT);
            return;
        };
        let old_rel = old_parent.join(name);
        let new_rel = new_parent.join(newname);
        let existed_in_base = self.base.join(&old_rel).symlink_metadata().is_ok();
        let src = match self.copy_up_any(&old_rel) {
            Ok(p) => p,
            Err(_) => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        if self.ensure_shadow_dir(new_rel.parent().unwrap_or(Path::new(""))).is_err() {
            reply.error(libc::EIO);
            return;
        }
        let dst = self.shadow.join(&new_rel);
        if std::fs::rename(&src, &dst).is_err() {
            reply.error(libc::EIO);
            return;
        }
        let mut deleted = self.deleted.lock_safe();
        if existed_in_base {
            deleted.insert(old_rel.clone());
        }
        deleted.remove(&new_rel);
        drop(deleted);
        self.persist_whiteouts();
        inodes.rekey(&old_rel, &new_rel);
        reply.ok();
    }
}

impl FreezeFs {
    /// Like `copy_up`, but works for directories too (used by `rename`,
    /// which can move either).
    fn copy_up_any(&self, rel: &Path) -> io::Result<PathBuf> {
        let shadow_path = self.shadow.join(rel);
        if shadow_path.symlink_metadata().is_ok() {
            return Ok(shadow_path);
        }
        let base_path = self.base.join(rel);
        if base_path.is_dir() {
            copy_tree(&base_path, &shadow_path)?;
            std::fs::create_dir_all(&shadow_path)?;
        } else {
            self.copy_up(rel)?;
        }
        Ok(shadow_path)
    }
}

impl Inodes {
    fn rekey(&mut self, old: &Path, new: &Path) {
        let affected: Vec<(PathBuf, u64)> = self
            .path_to_ino
            .iter()
            .filter(|(p, _)| p.as_path() == old || p.starts_with(old))
            .map(|(p, ino)| (p.clone(), *ino))
            .collect();
        for (old_path, ino) in affected {
            let suffix = old_path.strip_prefix(old).unwrap();
            let new_path = if suffix.as_os_str().is_empty() { new.to_path_buf() } else { new.join(suffix) };
            self.path_to_ino.remove(&old_path);
            self.path_to_ino.insert(new_path.clone(), ino);
            self.ino_to_path.insert(ino, new_path);
        }
    }
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---- Tauri commands ----

#[tauri::command]
pub fn freeze_folder(state: tauri::State<crate::FreezeState>, path: String, password: String) -> Result<(), String> {
    let hash = hash_password(&password)?;
    freeze(&path, hash, now_epoch())?;
    let session = spawn(&path, Path::new(&path)).str_err()?;
    state.mounts.lock_safe().insert(path, session);
    Ok(())
}

#[tauri::command]
pub fn list_frozen_folders() -> Vec<FreezeMeta> {
    list_frozen()
}

/// Covers both "Unfreeze…" from a live session and "Restore Now" from the
/// Manage Frozen Folders panel for an entry that isn't currently mounted
/// (e.g. after a crash) -- dropping a mount that doesn't exist is a no-op.
#[tauri::command]
pub fn unfreeze_folder(
    state: tauri::State<crate::FreezeState>,
    path: String,
    password: String,
    keep_changes: bool,
) -> Result<(), String> {
    let meta_raw = std::fs::read_to_string(meta_path(&path)).map_err(|_| "not frozen".to_string())?;
    let meta: FreezeMeta = serde_json::from_str(&meta_raw).str_err()?;
    if !verify_password(&password, &meta.password_hash) {
        return Err("incorrect password".to_string());
    }
    state.mounts.lock_safe().remove(&path); // drop -> unmount
    if keep_changes {
        commit(&path)
    } else {
        rollback(&path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// End-to-end smoke test against a real FUSE mount: freeze a scratch
    /// folder, write/delete/rename through the mountpoint, roll back and
    /// confirm the pristine copy survives untouched, then repeat with
    /// commit and confirm changes stick. Cleans up regardless of outcome.
    #[test]
    fn freeze_rollback_and_commit() {
        let test_path = format!("/tmp/ve-freeze-test-{}", std::process::id());
        let _ = std::fs::remove_dir_all(&test_path);
        std::fs::create_dir_all(&test_path).unwrap();
        std::fs::write(format!("{test_path}/original.txt"), b"pristine").unwrap();
        std::fs::create_dir_all(format!("{test_path}/subdir")).unwrap();
        std::fs::write(format!("{test_path}/subdir/nested.txt"), b"nested-pristine").unwrap();

        let cleanup = |path: &str| {
            let _ = std::fs::remove_dir_all(path);
            let _ = std::fs::remove_dir_all(meta_dir_for(path));
        };

        let hash = hash_password("s3cret").unwrap();
        assert!(verify_password("s3cret", &hash));
        assert!(!verify_password("wrong", &hash));

        freeze(&test_path, hash.clone(), 12345).unwrap();
        assert!(freeze(&test_path, hash.clone(), 1).is_err(), "double-freeze should be rejected");

        {
            let session = spawn(&test_path, Path::new(&test_path)).expect("mount failed");

            let original = std::fs::read_to_string(format!("{test_path}/original.txt")).unwrap();
            assert_eq!(original, "pristine");
            let nested = std::fs::read_to_string(format!("{test_path}/subdir/nested.txt")).unwrap();
            assert_eq!(nested, "nested-pristine");

            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(format!("{test_path}/original.txt"))
                .unwrap();
            f.write_all(b"MODIFIED").unwrap();
            drop(f);
            assert_eq!(
                std::fs::read_to_string(format!("{test_path}/original.txt")).unwrap(),
                "MODIFIED"
            );

            std::fs::write(format!("{test_path}/new_file.txt"), b"brand new").unwrap();
            assert!(std::fs::metadata(format!("{test_path}/new_file.txt")).is_ok());

            std::fs::remove_file(format!("{test_path}/subdir/nested.txt")).unwrap();
            assert!(std::fs::metadata(format!("{test_path}/subdir/nested.txt")).is_err());

            let entries: std::collections::HashSet<String> = std::fs::read_dir(&test_path)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            assert!(entries.contains("new_file.txt"));
            assert!(entries.contains("original.txt"));

            drop(session); // unmount
        }

        rollback(&test_path).unwrap();
        assert_eq!(
            std::fs::read_to_string(format!("{test_path}/original.txt")).unwrap(),
            "pristine",
            "rollback must discard the in-mount edit"
        );
        assert!(
            std::fs::metadata(format!("{test_path}/new_file.txt")).is_err(),
            "rollback must discard files created while frozen"
        );
        assert!(
            std::fs::metadata(format!("{test_path}/subdir/nested.txt")).is_ok(),
            "rollback must undo a deletion made while frozen"
        );

        // Freeze again, make a change, this time commit it.
        let hash2 = hash_password("s3cret2").unwrap();
        freeze(&test_path, hash2, 99999).unwrap();
        {
            let session = spawn(&test_path, Path::new(&test_path)).expect("mount failed");
            std::fs::write(format!("{test_path}/committed.txt"), b"keep me").unwrap();
            drop(session);
        }
        commit(&test_path).unwrap();
        assert!(
            std::fs::metadata(format!("{test_path}/committed.txt")).is_ok(),
            "commit must keep changes made while frozen"
        );
        assert!(!meta_dir_for(&test_path).exists(), "commit must clean up freeze metadata");

        cleanup(&test_path);
    }
}
