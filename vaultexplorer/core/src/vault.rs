//! A vault is a **Cryptomator vault** -- the same on-disk format the
//! Cryptomator apps read and write (vault format 8, `SIV_GCM`).
//!
//! This used to be a format of its own. It isn't any more: a folder that
//! Cryptomator (desktop, Android, iOS) made opens here, and a folder made
//! here opens there. The cryptography itself lives in
//! `cryptomator-rs-crypto` (Apache-2.0) -- scrypt-wrapped masterkey,
//! AES-SIV filenames bound to their parent directory's id, AES-GCM file
//! contents in 32 KiB chunks. What this module adds is the shape the rest
//! of the app already speaks: plaintext-relative paths in, whole files or
//! ranged reads out.
//!
//! Two things are ours rather than Cryptomator's, and both are ordinary
//! files *inside* the vault (so they are encrypted like anything else and
//! a stock Cryptomator can still open the vault):
//!
//! - the sensitive-files manifest (see [`Vault::set_sensitive`]), and
//! - nothing else. Everything else is plain Cryptomator.

use crate::error::{Result, VaultError};
use cryptomator_rs_crypto::{CryptoEntry, CryptoEntryType, Cryptomator, DirId, SeekableRw};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// Where the sensitive-files manifest lives, as a plaintext path inside
/// the vault. Hidden from listings (see [`Vault::list_dir`]) so it reads
/// as vault machinery rather than a file the user left lying around.
const SENSITIVE_MANIFEST: &str = ".vaultexplorer-sensitive";

/// Zip entries above this need the zip64 extension or the writer aborts
/// the archive partway through.
const ZIP64_FILE_THRESHOLD: u64 = 4 * 1024 * 1024 * 1024 - 1;

/// How much plaintext is encrypted per `write_data` call when importing a
/// real file -- big enough to keep the per-call overhead irrelevant, small
/// enough that importing a 4 GB video doesn't need 4 GB of RAM.
const IMPORT_CHUNK: usize = 4 * 1024 * 1024;

fn crypt_err(e: cryptomator_rs_crypto::CryptoError) -> VaultError {
    use cryptomator_rs_crypto::CryptoError as C;
    match e {
        C::IO(e) => VaultError::Io(e),
        // Everything scrypt/AES-KW rejects on unlock arrives as this.
        C::InvalidParameters => VaultError::InvalidPassword,
        other => VaultError::Crypt(other.to_string()),
    }
}

/// Split a plaintext-relative path into its components, dropping `.`,
/// `..` and any leading `/` -- the same normalization the old format did,
/// and what keeps a crafted `../..` from escaping the vault.
fn parts(rel: &Path) -> Vec<String> {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str().map(|s| s.to_string()),
            _ => None,
        })
        .collect()
}

fn norm_rel(rel: &Path) -> String {
    parts(rel).join("/")
}

pub struct Stat {
    pub is_dir: bool,
    pub len: u64,
}

/// Options for [`Vault::compress_paths`]: an optional password (AES-256
/// zip encryption), an optional Deflate level override, and an optional
/// README.txt bundled alongside the compressed entries.
#[derive(Default)]
pub struct CompressOptions {
    pub password: Option<String>,
    pub level: Option<i64>,
    pub readme: Option<String>,
}

/// One entry returned by [`Vault::list_dir`]. `mtime` is the Unix-epoch
/// seconds of the *encrypted* on-disk file (or, for a directory, of the
/// `d/XX/YYY…` folder its children live in), which every write here keeps
/// in step with the plaintext -- so no timestamp has to be stored inside
/// the encrypted format itself.
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    /// Whether this subdirectory is itself a whole nested vault.
    pub is_vault: bool,
    pub size: u64,
    pub mtime: i64,
}

/// The sensitive-files re-auth session state (see [`Vault::unlock_sensitive`]).
#[derive(Clone, Copy)]
enum SensitiveState {
    /// Locked: any sensitive file refuses to decrypt until re-authed.
    Locked,
    /// Unlocked until this instant, then auto-relocks.
    Until(Instant),
    /// Unlocked with no expiry (the "never" timeout option) until the vault
    /// itself is locked / the app exits.
    Forever,
}

/// An unlocked Cryptomator vault. Cloning is cheap (the engine is shared)
/// and every clone observes the same sensitive-files session -- a FUSE
/// mount runs on its own thread with its own clone and must not get a
/// second, independently-unlocked view of the sensitive files.
#[derive(Clone)]
pub struct Vault {
    crypto: Arc<Cryptomator>,
    root: PathBuf,
    sensitive: Arc<Mutex<SensitiveState>>,
}

/// Whether `root` holds a Cryptomator vault, i.e. [`Vault::unlock`] is the
/// right call there rather than [`Vault::create`]. Both files matter: the
/// JWT names the masterkey file it was signed with, and without that file
/// there is no key to check it against.
pub fn vault_exists(root: impl AsRef<Path>) -> bool {
    let root = root.as_ref();
    root.join("vault.cryptomator").is_file() && root.join("masterkey.cryptomator").is_file()
}

impl Vault {
    /// Create a new, empty vault at `root`.
    pub fn create(root: impl AsRef<Path>, password: &[u8]) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        if vault_exists(&root) {
            return Err(VaultError::VaultExists);
        }
        let crypto = Cryptomator::create_vault(&root, password).map_err(crypt_err)?;
        Ok(Self::wrap(crypto, root))
    }

    /// Unlock the vault at `root` with `password`.
    pub fn unlock(root: impl AsRef<Path>, password: &[u8]) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        if !vault_exists(&root) {
            return Err(VaultError::VaultNotFound);
        }
        let crypto = Cryptomator::open(&root, password).map_err(crypt_err)?;
        Ok(Self::wrap(crypto, root))
    }

    fn wrap(crypto: Cryptomator, root: PathBuf) -> Self {
        Vault {
            crypto: Arc::new(crypto),
            root,
            sensitive: Arc::new(Mutex::new(SensitiveState::Locked)),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    // ---- path resolution ----

    fn root_dir(&self) -> Result<DirId<'_>> {
        DirId::from_str(b"", &self.crypto).map_err(crypt_err)
    }

    /// The directory id for the plaintext-relative directory `rel`.
    fn dir_for(&self, rel: &[String]) -> Result<DirId<'_>> {
        let mut cur = self.root_dir()?;
        for name in rel {
            let entry = cur.lookup(name).map_err(crypt_err)?.ok_or(VaultError::PathNotFound)?;
            match entry.entry_type {
                CryptoEntryType::Directory { dir_id } => {
                    cur = DirId::from_str(&dir_id, &self.crypto).map_err(crypt_err)?;
                }
                _ => return Err(VaultError::PathNotFound),
            }
        }
        Ok(cur)
    }

    /// Split `rel` into (its parent's directory id, its own name).
    fn parent_of(&self, rel: &Path) -> Result<(DirId<'_>, String)> {
        let mut p = parts(rel);
        let name = p.pop().ok_or(VaultError::PathNotFound)?;
        Ok((self.dir_for(&p)?, name))
    }

    fn lookup(&self, rel: &Path) -> Result<CryptoEntry> {
        let (dir, name) = self.parent_of(rel)?;
        dir.lookup(&name).map_err(crypt_err)?.ok_or(VaultError::PathNotFound)
    }

    /// The real on-disk path of a vault file's ciphertext.
    fn cipher_path(&self, entry: &CryptoEntry) -> Result<PathBuf> {
        match &entry.entry_type {
            CryptoEntryType::File { abs_path } => Ok(self.root.join(abs_path)),
            _ => Err(VaultError::PathNotFound),
        }
    }

    // ---- reading ----

    /// Decrypt the whole file at `rel_path` into memory.
    pub fn decrypt_file(&self, rel_path: impl AsRef<Path>) -> Result<Vec<u8>> {
        let rel = rel_path.as_ref();
        self.check_sensitive_readable(rel)?;
        self.read_raw(rel)
    }

    /// The reverse of `encrypt_dir_at`: recursively decrypts everything
    /// under the vault directory `src_rel` onto the real filesystem at
    /// `dest_dir`, mirroring its structure -- what pasting a vault folder
    /// out to a real location needs (`decrypt_file` alone only covers a
    /// single file, the same restriction `export_file` had before this).
    pub fn decrypt_dir(&self, src_rel: impl AsRef<Path>, dest_dir: impl AsRef<Path>) -> Result<()> {
        let src_rel = src_rel.as_ref();
        let dest_dir = dest_dir.as_ref();
        fs::create_dir_all(dest_dir)?;
        let start = parts(src_rel);
        let dir = self.dir_for(&start)?;
        for entry in dir.list_files().map_err(crypt_err)? {
            let name = entry.name.to_string();
            let child_rel = src_rel.join(&name);
            let child_dest = dest_dir.join(&name);
            if let CryptoEntryType::Directory { .. } = entry.entry_type {
                self.decrypt_dir(&child_rel, &child_dest)?;
            } else {
                let bytes = self.decrypt_file(&child_rel)?;
                fs::write(&child_dest, bytes)?;
            }
        }
        Ok(())
    }

    /// Same, without the sensitive-files gate -- for vault machinery
    /// (the manifest itself) that must be readable to decide the gate.
    fn read_raw(&self, rel: &Path) -> Result<Vec<u8>> {
        let entry = self.lookup(rel)?;
        let path = self.cipher_path(&entry)?;
        let mut handle = self
            .crypto
            .file_handle(fs::File::open(&path)?)
            .map_err(crypt_err)?;
        let mut out = Vec::new();
        handle.read_to_end(&mut out)?;
        Ok(out)
    }

    /// A seekable reader over one file's plaintext, so a FUSE read of one
    /// range doesn't have to decrypt the whole file (see `fuse_mount`).
    pub fn open_read(&self, rel_path: impl AsRef<Path>) -> Result<VaultFileReader> {
        let rel = rel_path.as_ref();
        self.check_sensitive_readable(rel)?;
        let entry = self.lookup(rel)?;
        let path = self.cipher_path(&entry)?;
        let len = Cryptomator::encrypted_file_size(&path).map_err(crypt_err)?;
        let handle = self
            .crypto
            .file_handle(fs::File::open(&path)?)
            .map_err(crypt_err)?;
        Ok(VaultFileReader { handle, len })
    }

    pub fn stat(&self, rel_path: impl AsRef<Path>) -> Result<Stat> {
        let rel = rel_path.as_ref();
        if parts(rel).is_empty() {
            return Ok(Stat { is_dir: true, len: 0 });
        }
        let entry = self.lookup(rel)?;
        match &entry.entry_type {
            CryptoEntryType::Directory { .. } => Ok(Stat { is_dir: true, len: 0 }),
            CryptoEntryType::File { .. } => {
                let path = self.cipher_path(&entry)?;
                let len = Cryptomator::encrypted_file_size(&path).map_err(crypt_err)?;
                Ok(Stat { is_dir: false, len })
            }
            CryptoEntryType::Symlink { target } => Ok(Stat {
                is_dir: false,
                len: target.len() as u64,
            }),
        }
    }

    pub fn list_dir(&self, rel_path: impl AsRef<Path>) -> Result<Vec<DirEntry>> {
        let dir = self.dir_for(&parts(rel_path.as_ref()))?;
        let mut out = Vec::new();
        for entry in dir.list_files().map_err(crypt_err)? {
            let name = entry.name.to_string();
            if name == SENSITIVE_MANIFEST {
                continue;
            }
            let (is_dir, is_vault, size, path) = match &entry.entry_type {
                CryptoEntryType::Directory { dir_id } => {
                    let child = DirId::from_str(dir_id, &self.crypto).map_err(crypt_err)?;
                    // A whole Cryptomator vault nested inside this one
                    // looks exactly like it does on a real filesystem: a
                    // folder holding a vault.cryptomator.
                    let nested = child
                        .lookup("vault.cryptomator")
                        .map_err(crypt_err)?
                        .is_some();
                    (true, nested, 0u64, child.path())
                }
                CryptoEntryType::File { abs_path } => {
                    let path = self.root.join(abs_path);
                    let size = Cryptomator::encrypted_file_size(&path).unwrap_or(0);
                    (false, false, size, path)
                }
                CryptoEntryType::Symlink { target } => {
                    (false, false, target.len() as u64, self.root.clone())
                }
            };
            let mtime = fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            out.push(DirEntry {
                name,
                is_dir,
                is_vault,
                size,
                mtime,
            });
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(out)
    }

    /// Every file in the vault, as plaintext-relative paths.
    pub fn list_files(&self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        self.walk(&[], &mut |rel, is_dir| {
            if !is_dir {
                out.push(PathBuf::from(rel));
            }
            Ok(())
        })?;
        Ok(out)
    }

    /// Depth-first walk of the whole tree below `start`, calling `f` with
    /// each entry's plaintext-relative path (directories included, parents
    /// before children).
    fn walk(
        &self,
        start: &[String],
        f: &mut impl FnMut(&str, bool) -> Result<()>,
    ) -> Result<()> {
        let dir = self.dir_for(start)?;
        for entry in dir.list_files().map_err(crypt_err)? {
            let name = entry.name.to_string();
            if start.is_empty() && name == SENSITIVE_MANIFEST {
                continue;
            }
            let mut child = start.to_vec();
            child.push(name);
            let rel = child.join("/");
            match entry.entry_type {
                CryptoEntryType::Directory { .. } => {
                    f(&rel, true)?;
                    self.walk(&child, f)?;
                }
                _ => f(&rel, false)?,
            }
        }
        Ok(())
    }

    /// Case-insensitive search over names, plus the contents of files that
    /// are plausibly text. Returns plaintext-relative paths.
    pub fn search(&self, query: &str) -> Result<Vec<PathBuf>> {
        let mut hits = Vec::new();
        self.search_streaming(query, &mut |rel, _| {
            hits.push(rel.to_path_buf());
            true
        })?;
        Ok(hits)
    }

    /// [`search`](Self::search), reporting each hit as it is found instead
    /// of all at the end. `on_hit(rel, is_dir)` returns `false` to stop the
    /// search right there -- what a new keystroke does to the search the
    /// previous one started. Name matches (the whole tree) come first, then
    /// content matches, since a content match costs a full decrypt.
    pub fn search_streaming(&self, query: &str, on_hit: &mut dyn FnMut(&Path, bool) -> bool) -> Result<()> {
        let needle = query.to_lowercase();
        if needle.is_empty() {
            return Ok(());
        }
        const STOP: &str = "search stopped";
        let mut files = Vec::new();
        let walked = self.walk(&[], &mut |rel, is_dir| {
            // Only the entry's own name, not its whole path: a query that
            // matches a folder name would otherwise "match" every file
            // under it.
            let name = rel.rsplit('/').next().unwrap_or(rel);
            if name.to_lowercase().contains(&needle) {
                if !on_hit(Path::new(rel), is_dir) {
                    return Err(VaultError::Crypt(STOP.into()));
                }
            } else if !is_dir {
                files.push(PathBuf::from(rel));
            }
            Ok(())
        });
        match walked {
            Err(VaultError::Crypt(msg)) if msg == STOP => return Ok(()),
            other => other?,
        }
        for rel in files {
            if !is_searchable_text(&rel) {
                continue;
            }
            // A sensitive file stays out of content search while locked --
            // otherwise a search result is itself a leak of what's in it.
            let Ok(bytes) = self.decrypt_file(&rel) else {
                continue;
            };
            if let Ok(text) = String::from_utf8(bytes) {
                if text.to_lowercase().contains(&needle) && !on_hit(&rel, false) {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    pub fn dir_size(&self, rel_path: impl AsRef<Path>) -> Result<u64> {
        let start = parts(rel_path.as_ref());
        let mut total = 0u64;
        let mut sizes = Vec::new();
        self.walk(&start, &mut |rel, is_dir| {
            if !is_dir {
                sizes.push(PathBuf::from(rel));
            }
            Ok(())
        })?;
        for rel in sizes {
            total += self.stat(&rel).map(|s| s.len).unwrap_or(0);
        }
        Ok(total)
    }

    // ---- writing ----

    /// Write (or overwrite) a whole file. The previous ciphertext is
    /// dropped rather than written over: a shorter new content would
    /// otherwise leave the tail of the old one behind it.
    pub fn write_file(&self, rel_path: impl AsRef<Path>, plaintext: &[u8]) -> Result<()> {
        let path = self.create_empty(rel_path.as_ref())?;
        if plaintext.is_empty() {
            return Ok(());
        }
        let mut handle = self
            .crypto
            .file_handle(SeekableRw::from_path(&path).map_err(crypt_err)?)
            .map_err(crypt_err)?;
        handle.write_data(0, plaintext).map_err(crypt_err)
    }

    /// Encrypt a real on-disk file into the vault at `rel_path`, streaming
    /// it in chunks so an import is never bounded by RAM.
    pub fn encrypt_file(
        &self,
        plaintext_path: impl AsRef<Path>,
        rel_path: impl AsRef<Path>,
    ) -> Result<()> {
        let mut src = fs::File::open(plaintext_path.as_ref())?;
        let dest = self.create_empty(rel_path.as_ref())?;
        let mut handle = self
            .crypto
            .file_handle(SeekableRw::from_path(&dest).map_err(crypt_err)?)
            .map_err(crypt_err)?;
        let mut buf = vec![0u8; IMPORT_CHUNK];
        let mut pos = 0usize;
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 {
                break;
            }
            handle.write_data(pos, &buf[..n]).map_err(crypt_err)?;
            pos += n;
        }
        Ok(())
    }

    /// Make sure `rel` exists as an empty file (creating its parents) and
    /// hand back the real path of its ciphertext.
    fn create_empty(&self, rel: &Path) -> Result<PathBuf> {
        let mut p = parts(rel);
        let name = p.pop().ok_or(VaultError::PathNotFound)?;
        self.create_dir_parts(&p)?;
        let dir = self.dir_for(&p)?;
        if let Some(existing) = dir.lookup(&name).map_err(crypt_err)? {
            match existing.entry_type {
                CryptoEntryType::Directory { .. } => return Err(VaultError::PathNotFound),
                _ => {
                    self.crypto.delete_entry(&dir, &name).map_err(crypt_err)?;
                }
            }
        }
        let entry = self.crypto.create_file(&dir, &name, true).map_err(crypt_err)?;
        self.cipher_path(&entry)
    }

    /// `mkdir -p` for a plaintext-relative path.
    pub fn create_dir(&self, rel_path: impl AsRef<Path>) -> Result<()> {
        self.create_dir_parts(&parts(rel_path.as_ref()))
    }

    fn create_dir_parts(&self, p: &[String]) -> Result<()> {
        let mut walked: Vec<String> = Vec::new();
        for name in p {
            let dir = self.dir_for(&walked)?;
            match dir.lookup(name).map_err(crypt_err)? {
                Some(entry) => match entry.entry_type {
                    CryptoEntryType::Directory { .. } => {}
                    // A file already sits where this directory should go.
                    _ => return Err(VaultError::PathNotFound),
                },
                None => {
                    self.crypto.create_directory(&dir, name).map_err(crypt_err)?;
                }
            }
            walked.push(name.clone());
        }
        Ok(())
    }

    /// Delete one file.
    pub fn remove_file(&self, rel_path: impl AsRef<Path>) -> Result<()> {
        let (dir, name) = self.parent_of(rel_path.as_ref())?;
        self.crypto
            .delete_entry(&dir, &name)
            .map_err(crypt_err)?
            .ok_or(VaultError::PathNotFound)?;
        Ok(())
    }

    /// Delete a directory and everything under it.
    pub fn remove_dir(&self, rel_path: impl AsRef<Path>) -> Result<()> {
        let p = parts(rel_path.as_ref());
        if p.is_empty() {
            return Err(VaultError::PathNotFound);
        }
        self.empty_dir(&p)?;
        let (dir, name) = self.parent_of(rel_path.as_ref())?;
        self.crypto
            .delete_entry(&dir, &name)
            .map_err(crypt_err)?
            .ok_or(VaultError::PathNotFound)?;
        Ok(())
    }

    /// Remove every child of a directory, depth first -- Cryptomator's own
    /// delete refuses a non-empty directory (the way `rmdir(2)` does).
    fn empty_dir(&self, p: &[String]) -> Result<()> {
        let dir = self.dir_for(p)?;
        for entry in dir.list_files().map_err(crypt_err)? {
            let name = entry.name.to_string();
            if let CryptoEntryType::Directory { .. } = entry.entry_type {
                let mut child = p.to_vec();
                child.push(name.clone());
                self.empty_dir(&child)?;
            }
            self.crypto.delete_entry(&dir, &name).map_err(crypt_err)?;
        }
        Ok(())
    }

    /// Move a file or directory within the vault. Nothing is decrypted:
    /// only the entry's *name* is re-derived for its new parent, and a
    /// directory keeps its directory id, so its whole subtree comes along
    /// untouched however big it is.
    pub fn move_path(&self, src_rel: impl AsRef<Path>, dest_rel: impl AsRef<Path>) -> Result<()> {
        let src = src_rel.as_ref();
        let dest = dest_rel.as_ref();
        if norm_rel(src) == norm_rel(dest) {
            return Ok(());
        }
        let (src_dir, src_name) = self.parent_of(src)?;
        let src_entry = src_dir
            .lookup(&src_name)
            .map_err(crypt_err)?
            .ok_or(VaultError::PathNotFound)?;
        let mut dest_parts = parts(dest);
        let dest_name = dest_parts.pop().ok_or(VaultError::PathNotFound)?;
        self.create_dir_parts(&dest_parts)?;
        let dest_dir = self.dir_for(&dest_parts)?;
        // Moving a directory onto an existing file (or the reverse) is a
        // mistake, not an overwrite -- the engine's own rename compares
        // the source against itself here, so the check has to be ours.
        if let Some(existing) = dest_dir.lookup(&dest_name).map_err(crypt_err)? {
            let src_is_dir = matches!(src_entry.entry_type, CryptoEntryType::Directory { .. });
            let dest_is_dir = matches!(existing.entry_type, CryptoEntryType::Directory { .. });
            if src_is_dir != dest_is_dir {
                return Err(VaultError::PathExists);
            }
            if dest_is_dir {
                self.remove_dir(dest)?;
            } else {
                self.remove_file(dest)?;
            }
        }
        drop(dest_dir);
        drop(src_dir);
        drop(src_entry);
        self.move_entry(src, dest)
    }

    /// The actual move, one entry at a time.
    ///
    /// A file (or symlink) is handed to the engine's own rename, which
    /// relocates the ciphertext without decrypting it. A *directory* is
    /// rebuilt at the destination and its children moved into it, rather
    /// than re-pointed wholesale: an encrypted filename is bound to its
    /// parent directory's id, so children of a directory that kept its id
    /// would still carry names derived from the old parent -- and the
    /// engine's own directory rename mis-resolves which entry to unlink
    /// (it shadows the parent it was given with the directory itself),
    /// leaving the source behind. Each step here is a metadata operation;
    /// no file content is decrypted however deep the tree.
    fn move_entry(&self, src: &Path, dest: &Path) -> Result<()> {
        let (src_dir, src_name) = self.parent_of(src)?;
        let entry = src_dir
            .lookup(&src_name)
            .map_err(crypt_err)?
            .ok_or(VaultError::PathNotFound)?;
        if let CryptoEntryType::Directory { .. } = entry.entry_type {
            self.create_dir(dest)?;
            let children: Vec<String> = self
                .dir_for(&parts(src))?
                .list_files()
                .map_err(crypt_err)?
                .into_iter()
                .map(|e| e.name.to_string())
                .collect();
            for name in children {
                self.move_entry(&src.join(&name), &dest.join(&name))?;
            }
            self.crypto
                .delete_entry(&src_dir, &src_name)
                .map_err(crypt_err)?
                .ok_or(VaultError::PathNotFound)?;
            return Ok(());
        }
        let mut dest_parts = parts(dest);
        let dest_name = dest_parts.pop().ok_or(VaultError::PathNotFound)?;
        let dest_dir = self.dir_for(&dest_parts)?;
        self.crypto
            .rename(&src_dir, &src_name, &dest_dir, &dest_name, false)
            .map_err(crypt_err)?
            .ok_or(VaultError::PathNotFound)?;
        Ok(())
    }

    /// Copy a file or directory within the vault; the source is left
    /// untouched. Unlike a move this does re-encrypt: every file in a
    /// vault carries its own content key, and two files must never share
    /// one.
    pub fn copy_path(&self, src_rel: impl AsRef<Path>, dest_rel: impl AsRef<Path>) -> Result<()> {
        let src = src_rel.as_ref();
        let dest = dest_rel.as_ref();
        match self.stat(src)? {
            Stat { is_dir: false, .. } => {
                let bytes = self.decrypt_file(src)?;
                self.write_file(dest, &bytes)
            }
            Stat { is_dir: true, .. } => {
                self.create_dir(dest)?;
                let start = parts(src);
                let dir = self.dir_for(&start)?;
                for entry in dir.list_files().map_err(crypt_err)? {
                    let name = entry.name.to_string();
                    self.copy_path(src.join(&name), dest.join(&name))?;
                }
                Ok(())
            }
        }
    }

    /// How many files (not directories) `copy_path`/`copy_path_with_progress`
    /// would touch for `rel_path` -- the total a progress bar needs before
    /// the copy starts.
    pub fn count_files(&self, rel_path: impl AsRef<Path>) -> Result<u64> {
        let rel_path = rel_path.as_ref();
        if !self.stat(rel_path)?.is_dir {
            return Ok(1);
        }
        let mut n = 0u64;
        self.walk(&parts(rel_path), &mut |_, is_dir| {
            if !is_dir {
                n += 1;
            }
            Ok(())
        })?;
        Ok(n)
    }

    /// Same as `copy_path`, but calls `on_file_done` after each file lands
    /// -- copying a folder full of files with no feedback at all reads as
    /// the app having hung, especially since every file here is a decrypt
    /// + re-encrypt, not a cheap metadata op like `move_path`'s.
    pub fn copy_path_with_progress(
        &self,
        src_rel: impl AsRef<Path>,
        dest_rel: impl AsRef<Path>,
        on_file_done: &dyn Fn(),
    ) -> Result<()> {
        let src = src_rel.as_ref();
        let dest = dest_rel.as_ref();
        match self.stat(src)? {
            Stat { is_dir: false, .. } => {
                let bytes = self.decrypt_file(src)?;
                self.write_file(dest, &bytes)?;
                on_file_done();
                Ok(())
            }
            Stat { is_dir: true, .. } => {
                self.create_dir(dest)?;
                let start = parts(src);
                let dir = self.dir_for(&start)?;
                for entry in dir.list_files().map_err(crypt_err)? {
                    let name = entry.name.to_string();
                    self.copy_path_with_progress(src.join(&name), dest.join(&name), on_file_done)?;
                }
                Ok(())
            }
        }
    }

    /// Absorb an existing plaintext file/dir sitting at `src` (a real
    /// on-disk path) INTO this vault at plaintext `rel`, then remove the
    /// plaintext original. Used by "Convert to Vault" to encrypt a folder's
    /// pre-existing contents in place.
    pub fn absorb(&self, src: &Path, rel: &Path) -> Result<()> {
        if src.is_dir() {
            self.create_dir(rel)?;
            for entry in fs::read_dir(src)? {
                let entry = entry?;
                self.absorb(&entry.path(), &rel.join(entry.file_name()))?;
            }
            fs::remove_dir_all(src)?;
        } else {
            self.encrypt_file(src, rel)?;
            fs::remove_file(src)?;
        }
        Ok(())
    }

    /// Recursively encrypt every file under `plaintext_dir` into the vault,
    /// mirroring its directory structure.
    pub fn encrypt_dir(&self, plaintext_dir: impl AsRef<Path>) -> Result<()> {
        let plaintext_dir = plaintext_dir.as_ref();
        for entry in fs::read_dir(plaintext_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let rel = PathBuf::from(&name);
            if entry.path().is_dir() {
                self.create_dir(&rel)?;
                self.encrypt_subdir(&entry.path(), &rel)?;
            } else {
                self.encrypt_file(entry.path(), &rel)?;
            }
        }
        Ok(())
    }

    /// Like `encrypt_dir`, but the destination doesn't have to be the vault
    /// root -- what pasting a real folder into a vault subdirectory needs
    /// (`encrypt_dir` always lands at the top).
    pub fn encrypt_dir_at(&self, plaintext_dir: impl AsRef<Path>, dest_rel: impl AsRef<Path>) -> Result<()> {
        let dest_rel = dest_rel.as_ref();
        self.create_dir(dest_rel)?;
        self.encrypt_subdir(plaintext_dir.as_ref(), dest_rel)
    }

    fn encrypt_subdir(&self, dir: &Path, rel: &Path) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let child_rel = rel.join(entry.file_name());
            if entry.path().is_dir() {
                self.create_dir(&child_rel)?;
                self.encrypt_subdir(&entry.path(), &child_rel)?;
            } else {
                self.encrypt_file(entry.path(), &child_rel)?;
            }
        }
        Ok(())
    }

    // ---- archives ----

    /// Zip up `names` (files or whole folders) from `dir_rel` into a new
    /// vault file at `dest_rel`. The zip is built in memory and written
    /// back encrypted -- no plaintext archive ever exists on disk.
    pub fn compress_paths(
        &self,
        dir_rel: impl AsRef<Path>,
        names: &[String],
        dest_rel: impl AsRef<Path>,
        opts: &CompressOptions,
    ) -> Result<()> {
        let dir_rel = dir_rel.as_ref();
        let mut leaves: Vec<PathBuf> = Vec::new();
        for name in names {
            let rel = dir_rel.join(name);
            if self.stat(&rel)?.is_dir {
                let start = parts(&rel);
                self.walk(&start, &mut |child, is_dir| {
                    if !is_dir {
                        leaves.push(PathBuf::from(child));
                    }
                    Ok(())
                })?;
            } else {
                leaves.push(rel);
            }
        }

        let mut buf = Cursor::new(Vec::new());
        {
            let mut zw = ZipWriter::new(&mut buf);
            let mut options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            if let Some(level) = opts.level {
                options = options.compression_level(Some(level));
            }
            if let Some(pw) = &opts.password {
                options = options.with_aes_encryption(zip::AesMode::Aes256, pw);
            }
            for leaf in &leaves {
                let entry_name = leaf
                    .strip_prefix(dir_rel)
                    .unwrap_or(leaf)
                    .to_string_lossy()
                    .replace('\\', "/");
                let content = self.decrypt_file(leaf)?;
                let options = if content.len() as u64 > ZIP64_FILE_THRESHOLD {
                    options.large_file(true)
                } else {
                    options
                };
                zw.start_file(entry_name, options)?;
                zw.write_all(&content)?;
            }
            if let Some(readme) = &opts.readme {
                zw.start_file("README.txt", options)?;
                zw.write_all(readme.as_bytes())?;
            }
            zw.finish()?;
        }
        self.write_file(dest_rel, &buf.into_inner())
    }

    /// Decrypt the zip at `zip_rel` and extract it under `dest_dir_rel`,
    /// re-encrypting every entry back into the vault.
    pub fn decompress_zip(
        &self,
        zip_rel: impl AsRef<Path>,
        dest_dir_rel: impl AsRef<Path>,
        password: Option<&str>,
    ) -> Result<()> {
        let dest_dir_rel = dest_dir_rel.as_ref();
        let bytes = self.decrypt_file(zip_rel)?;
        let mut archive = ZipArchive::new(Cursor::new(bytes))?;
        for i in 0..archive.len() {
            let mut entry = match password {
                Some(pw) => archive.by_index_decrypt(i, pw.as_bytes())?,
                None => archive.by_index(i)?,
            };
            let Some(name) = entry.enclosed_name() else {
                continue;
            };
            let dest = dest_dir_rel.join(name);
            if entry.is_dir() {
                self.create_dir(&dest)?;
            } else {
                let mut content = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut content)?;
                self.write_file(&dest, &content)?;
            }
        }
        Ok(())
    }

    // ---- sensitive files ----

    fn sensitive_list(&self) -> Vec<String> {
        let Ok(bytes) = self.read_raw(Path::new(SENSITIVE_MANIFEST)) else {
            return Vec::new();
        };
        String::from_utf8(bytes)
            .map(|s| {
                s.lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every path explicitly marked sensitive, as plaintext-relative paths.
    pub fn list_sensitive(&self) -> Result<Vec<String>> {
        Ok(self.sensitive_list())
    }

    /// Whether `rel_path` is sensitive -- either marked itself, or living
    /// under a folder that is.
    pub fn is_sensitive(&self, rel_path: impl AsRef<Path>) -> bool {
        let rel = norm_rel(rel_path.as_ref());
        self.sensitive_list().iter().any(|marked| {
            rel == *marked || rel.starts_with(&format!("{marked}/"))
        })
    }

    /// Mark (or unmark) a file or folder sensitive.
    pub fn set_sensitive(&self, rel_path: impl AsRef<Path>, sensitive: bool) -> Result<()> {
        let rel = norm_rel(rel_path.as_ref());
        let mut list = self.sensitive_list();
        if sensitive {
            if !list.contains(&rel) {
                list.push(rel);
            }
        } else {
            // Unmarking only works on the entry that carries the mark:
            // a file inside a sensitive folder inherits it and has
            // nothing of its own to remove.
            if !list.contains(&rel) {
                return Err(VaultError::SensitiveInherited);
            }
            list.retain(|m| *m != rel);
        }
        list.sort();
        self.write_file(Path::new(SENSITIVE_MANIFEST), list.join("\n").as_bytes())
    }

    /// Open the sensitive-files window by re-entering the vault password.
    pub fn unlock_sensitive(&self, password: &[u8], timeout: Option<Duration>) -> Result<()> {
        // Re-deriving the masterkey from scratch is the check: it's the
        // same work an unlock does, and it can't be faked without the
        // real password.
        Cryptomator::open(&self.root, password).map_err(crypt_err)?;
        let state = match timeout {
            Some(d) => SensitiveState::Until(Instant::now() + d),
            None => SensitiveState::Forever,
        };
        *self.sensitive.lock().expect("sensitive state poisoned") = state;
        Ok(())
    }

    pub fn sensitive_unlocked(&self) -> bool {
        let mut guard = self.sensitive.lock().expect("sensitive state poisoned");
        match *guard {
            SensitiveState::Locked => false,
            SensitiveState::Forever => true,
            SensitiveState::Until(deadline) => {
                if Instant::now() < deadline {
                    true
                } else {
                    *guard = SensitiveState::Locked;
                    false
                }
            }
        }
    }

    pub fn lock_sensitive(&self) {
        *self.sensitive.lock().expect("sensitive state poisoned") = SensitiveState::Locked;
    }

    fn check_sensitive_readable(&self, rel: &Path) -> Result<()> {
        if self.is_sensitive(rel) && !self.sensitive_unlocked() {
            return Err(VaultError::SensitiveLocked);
        }
        Ok(())
    }
}

/// A seekable plaintext view of one encrypted file: reads decrypt only the
/// 32 KiB chunks a range actually touches.
pub struct VaultFileReader {
    handle: cryptomator_rs_crypto::FileHandle<fs::File>,
    len: u64,
}

impl VaultFileReader {
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        if offset >= self.len {
            return Ok(Vec::new());
        }
        let want = len.min((self.len - offset) as usize);
        self.handle
            .read_data(offset as usize, want)
            .map_err(crypt_err)
    }

    pub fn read_all(&mut self) -> Result<Vec<u8>> {
        self.read_at(0, self.len as usize)
    }
}

/// Whether `path` looks like plain text worth decrypting for a content
/// search -- deliberately conservative rather than sniffing file content.
fn is_searchable_text(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(
        ext.as_str(),
        "txt" | "md"
            | "rtf"
            | "log"
            | "csv"
            | "rs"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "go"
            | "c"
            | "cpp"
            | "h"
            | "java"
            | "rb"
            | "sh"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "css"
            | "html"
            | "xml"
    )
}
