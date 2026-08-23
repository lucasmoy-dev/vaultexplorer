//! Mounts a [`Vault`] as a real read-write filesystem via FUSE. Any program
//! that opens a file under the mountpoint sees plaintext; the file only
//! ever exists encrypted on the underlying disk. Saving through a normal
//! editor works because `write`/`release` re-encrypt back to the vault,
//! preserving any per-file password / PQ recipient grants already on it.
//!
//! Files are buffered whole in memory while open (simplest correct
//! approach for the editable-document-sized files this targets -- not
//! meant for multi-GB video).
//!
//! [`VaultFs`] is the shared [`fuser::Filesystem`] impl. Callers choose how
//! to run it: [`spawn`] for a non-blocking mount that unmounts on drop
//! (what a GUI app wants), or hand [`VaultFs::new`]'s result straight to
//! [`fuser::mount2`] for a blocking, foreground mount (what a CLI tool
//! wants).

use crate::vault::{Stat, Vault, VaultFileReader};
use fuser::{
    BackgroundSession, FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyCreate,
    ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request, TimeOrNow,
    FUSE_ROOT_ID,
};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

const TTL: Duration = Duration::from_secs(1);

/// Mount `vault` at `mountpoint` in the background. The mount is torn down
/// automatically when the returned session is dropped.
pub fn spawn(vault: Vault, mountpoint: &Path) -> io::Result<BackgroundSession> {
    let fs = VaultFs::new(vault);
    let options = vec![MountOption::FSName("vaultfs".to_string())];
    fuser::spawn_mount2(fs, mountpoint, &options)
}

/// How an open handle backs its plaintext.
///
/// Reads start out [`Streaming`](Content::Streaming): nothing is decrypted
/// at `open` time and each `read` pulls only the 64 KiB chunks it covers.
/// That's what makes opening a multi-GB video in an external player start
/// instantly instead of stalling on a full decrypt of the whole file (and
/// allocating the whole file in RAM) before the first byte is served.
///
/// The first write flips the handle to [`Buffered`](Content::Buffered),
/// since the write path re-encrypts the file whole and does need the entire
/// plaintext in memory.
enum Content {
    Streaming(VaultFileReader),
    Buffered(Vec<u8>),
}

struct OpenFile {
    rel_path: PathBuf,
    content: Content,
    dirty: bool,
}

impl OpenFile {
    /// Materialize the whole plaintext so it can be modified in place.
    /// Returns `None` if the ciphertext can't be read back.
    fn buffer_mut(&mut self) -> Option<&mut Vec<u8>> {
        if let Content::Streaming(reader) = &mut self.content {
            let buf = reader.read_all().ok()?;
            self.content = Content::Buffered(buf);
        }
        match &mut self.content {
            Content::Buffered(b) => Some(b),
            Content::Streaming(_) => None,
        }
    }
}

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
        Self {
            path_to_ino,
            ino_to_path,
            next_ino: FUSE_ROOT_ID + 1,
        }
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

    /// After a rename, every inode whose path was under `old` (including
    /// `old` itself) needs to be re-keyed under `new` -- otherwise a
    /// still-open handle or a cached lookup would keep pointing at a
    /// plaintext path that no longer resolves to anything on disk.
    fn rekey_subtree(&mut self, old: &Path, new: &Path) {
        let affected: Vec<(PathBuf, u64)> = self
            .path_to_ino
            .iter()
            .filter(|(p, _)| p.as_path() == old || p.starts_with(old))
            .map(|(p, ino)| (p.clone(), *ino))
            .collect();
        for (old_path, ino) in affected {
            let suffix = old_path.strip_prefix(old).unwrap();
            let new_path = if suffix.as_os_str().is_empty() {
                new.to_path_buf()
            } else {
                new.join(suffix)
            };
            self.path_to_ino.remove(&old_path);
            self.path_to_ino.insert(new_path.clone(), ino);
            self.ino_to_path.insert(ino, new_path);
        }
    }
}

pub struct VaultFs {
    vault: Vault,
    inodes: Mutex<Inodes>,
    handles: Mutex<HashMap<u64, OpenFile>>,
    next_fh: AtomicU64,
}

impl VaultFs {
    pub fn new(vault: Vault) -> Self {
        Self {
            vault,
            inodes: Mutex::new(Inodes::new()),
            handles: Mutex::new(HashMap::new()),
            next_fh: AtomicU64::new(1),
        }
    }

    fn attr_for(&self, ino: u64, stat: &Stat) -> FileAttr {
        let now = SystemTime::now();
        let kind = if stat.is_dir {
            FileType::Directory
        } else {
            FileType::RegularFile
        };
        let perm = if stat.is_dir { 0o755 } else { 0o644 };
        FileAttr {
            ino,
            size: stat.len,
            blocks: stat.len.div_ceil(512),
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind,
            perm,
            nlink: 1,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    fn writeback(&self, fh: u64) {
        let mut handles = self.handles.lock().unwrap();
        if let Some(f) = handles.get_mut(&fh) {
            // Only a `Buffered` handle can be dirty (writing is what
            // materializes the buffer), so a read-only handle never
            // re-encrypts anything here.
            if f.dirty {
                if let Content::Buffered(buf) = &f.content {
                    let _ = self.vault.write_file(&f.rel_path, buf);
                }
                f.dirty = false;
            }
        }
    }
}

impl Filesystem for VaultFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let mut inodes = self.inodes.lock().unwrap();
        let Some(parent_path) = inodes.path_for(parent) else {
            reply.error(libc::ENOENT);
            return;
        };
        let child_path = parent_path.join(name);
        match self.vault.stat(&child_path) {
            Ok(stat) => {
                let ino = inodes.ino_for(&child_path);
                reply.entry(&TTL, &self.attr_for(ino, &stat), 0);
            }
            Err(_) => reply.error(libc::ENOENT),
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        let path = match self.inodes.lock().unwrap().path_for(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        match self.vault.stat(&path) {
            Ok(stat) => reply.attr(&TTL, &self.attr_for(ino, &stat)),
            Err(_) => reply.error(libc::ENOENT),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let mut inodes = self.inodes.lock().unwrap();
        let Some(dir_path) = inodes.path_for(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let children = match self.vault.list_dir(&dir_path) {
            Ok(c) => c,
            Err(_) => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let mut entries = vec![
            (ino, FileType::Directory, ".".to_string()),
            (ino, FileType::Directory, "..".to_string()),
        ];
        for child in children {
            let name = child.name;
            let is_dir = child.is_dir;
            let child_path = dir_path.join(&name);
            let child_ino = inodes.ino_for(&child_path);
            let kind = if is_dir {
                FileType::Directory
            } else {
                FileType::RegularFile
            };
            entries.push((child_ino, kind, name));
        }

        for (i, (ino, kind, name)) in entries.into_iter().enumerate().skip(offset as usize) {
            if reply.add(ino, (i + 1) as i64, kind, name) {
                break; // reply buffer full
            }
        }
        reply.ok();
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        let path = match self.inodes.lock().unwrap().path_for(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        // No decryption here on purpose -- `read` pulls chunks as they're
        // asked for, so `open` is O(1) whatever the file's size.
        let reader = match self.vault.open_read(&path) {
            Ok(r) => r,
            Err(_) => {
                reply.error(libc::EIO);
                return;
            }
        };
        let fh = self.next_fh.fetch_add(1, Ordering::SeqCst);
        self.handles.lock().unwrap().insert(
            fh,
            OpenFile {
                rel_path: path,
                content: Content::Streaming(reader),
                dirty: false,
            },
        );
        reply.opened(fh, 0);
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
        let mut handles = self.handles.lock().unwrap();
        match handles.get_mut(&fh) {
            Some(f) => {
                let offset = offset.max(0) as u64;
                match &mut f.content {
                    Content::Streaming(reader) => match reader.read_at(offset, size as usize) {
                        Ok(data) => reply.data(&data),
                        Err(_) => reply.error(libc::EIO),
                    },
                    Content::Buffered(buf) => {
                        let offset = offset as usize;
                        let end = (offset + size as usize).min(buf.len());
                        if offset >= buf.len() {
                            reply.data(&[]);
                        } else {
                            reply.data(&buf[offset..end]);
                        }
                    }
                }
            }
            None => reply.error(libc::EBADF),
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
        let mut handles = self.handles.lock().unwrap();
        match handles.get_mut(&fh) {
            Some(f) => {
                let offset = offset.max(0) as usize;
                let Some(buf) = f.buffer_mut() else {
                    reply.error(libc::EIO);
                    return;
                };
                if buf.len() < offset + data.len() {
                    buf.resize(offset + data.len(), 0);
                }
                buf[offset..offset + data.len()].copy_from_slice(data);
                f.dirty = true;
                reply.written(data.len() as u32);
            }
            None => reply.error(libc::EBADF),
        }
    }

    // Without this, the kernel returns ENOSYS for any truncate / chmod /
    // utimes -- which breaks external editors that save by truncating the
    // file to zero and rewriting it (or opening with O_TRUNC), the classic
    // "Unable to save" from Photoshop/GIMP/etc editing a vault file through
    // the mount. Only `size` (truncate) is materially honored; mode/owner/
    // timestamps are accepted as no-ops (this fs doesn't store them) so the
    // save flow completes instead of erroring.
    #[allow(clippy::too_many_arguments)]
    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        let path = match self.inodes.lock().unwrap().path_for(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        if let Some(sz) = size {
            let sz = sz as usize;
            let mut handles = self.handles.lock().unwrap();
            let open = fh.and_then(|h| handles.get_mut(&h));
            if let Some(f) = open {
                // Truncate the in-memory buffer; writeback re-encrypts on flush.
                let Some(buf) = f.buffer_mut() else {
                    reply.error(libc::EIO);
                    return;
                };
                buf.resize(sz, 0);
                f.dirty = true;
            } else {
                drop(handles);
                let mut buf = self.vault.decrypt_file(&path).unwrap_or_default();
                buf.resize(sz, 0);
                if self.vault.write_file(&path, &buf).is_err() {
                    reply.error(libc::EIO);
                    return;
                }
            }
            reply.attr(&TTL, &self.attr_for(ino, &Stat { is_dir: false, len: sz as u64 }));
            return;
        }
        // No size change: accept (mode/owner/time are not stored) and reply
        // with the current attributes.
        match self.vault.stat(&path) {
            Ok(stat) => reply.attr(&TTL, &self.attr_for(ino, &stat)),
            Err(_) => reply.error(libc::ENOENT),
        }
    }

    fn flush(&mut self, _req: &Request, _ino: u64, fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
        self.writeback(fh);
        reply.ok();
    }

    fn fsync(&mut self, _req: &Request, _ino: u64, fh: u64, _datasync: bool, reply: ReplyEmpty) {
        self.writeback(fh);
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
        self.writeback(fh);
        self.handles.lock().unwrap().remove(&fh);
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
        let mut inodes = self.inodes.lock().unwrap();
        let Some(parent_path) = inodes.path_for(parent) else {
            reply.error(libc::ENOENT);
            return;
        };
        let child_path = parent_path.join(name);
        if self.vault.write_file(&child_path, b"").is_err() {
            reply.error(libc::EIO);
            return;
        }
        let ino = inodes.ino_for(&child_path);
        let stat = Stat {
            is_dir: false,
            len: 0,
        };
        let fh = self.next_fh.fetch_add(1, Ordering::SeqCst);
        self.handles.lock().unwrap().insert(
            fh,
            OpenFile {
                rel_path: child_path,
                content: Content::Buffered(Vec::new()),
                dirty: false,
            },
        );
        reply.created(&TTL, &self.attr_for(ino, &stat), 0, fh, 0);
    }

    fn mkdir(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let mut inodes = self.inodes.lock().unwrap();
        let Some(parent_path) = inodes.path_for(parent) else {
            reply.error(libc::ENOENT);
            return;
        };
        let child_path = parent_path.join(name);
        if self.vault.create_dir(&child_path).is_err() {
            reply.error(libc::EIO);
            return;
        }
        let ino = inodes.ino_for(&child_path);
        let stat = Stat {
            is_dir: true,
            len: 0,
        };
        reply.entry(&TTL, &self.attr_for(ino, &stat), 0);
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let parent_path = match self.inodes.lock().unwrap().path_for(parent) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        match self.vault.remove_file(parent_path.join(name)) {
            Ok(()) => reply.ok(),
            Err(_) => reply.error(libc::EIO),
        }
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let parent_path = match self.inodes.lock().unwrap().path_for(parent) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        match self.vault.remove_dir(parent_path.join(name)) {
            Ok(()) => reply.ok(),
            Err(_) => reply.error(libc::EIO),
        }
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
        let mut inodes = self.inodes.lock().unwrap();
        let (Some(old_parent), Some(new_parent)) =
            (inodes.path_for(parent), inodes.path_for(newparent))
        else {
            reply.error(libc::ENOENT);
            return;
        };
        let old_path = old_parent.join(name);
        let new_path = new_parent.join(newname);
        match self.vault.move_path(&old_path, &new_path) {
            Ok(()) => {
                inodes.rekey_subtree(&old_path, &new_path);
                reply.ok();
            }
            Err(_) => reply.error(libc::EIO),
        }
    }
}
