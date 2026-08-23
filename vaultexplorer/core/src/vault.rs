use crate::crypto::{aead_decrypt, aead_encrypt, derive_key_from_password, random_salt, Key32};
use crate::error::{Result, VaultError};
use crate::file;
use crate::header::WrappedKey;
use crate::name::{self, NameKey};
use crate::pq::{HybridWrap, RecipientPublicKey, RecipientSecretKey};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{BufReader, BufWriter, Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const VAULT_META_FILENAME: &str = ".vault.meta";
/// Encrypted manifest (in the vault root) of which plaintext paths are
/// marked "sensitive". Fixed plaintext name like `.vault.meta`; skipped by
/// directory listings. Its contents are AEAD-encrypted under the master key.
const FLAGS_FILENAME: &str = ".vault.flags";
const FEK_WRAP_AAD: &[u8] = b"vaultcore-fek-wrap";
const MASTER_KEY_WRAP_AAD: &[u8] = b"vaultcore-master-key-wrap";
const FLAGS_AAD: &[u8] = b"vaultcore-flags";

/// Reserved on-disk filenames in a vault that are metadata, not encrypted
/// content -- listings and walks skip these.
fn is_reserved_name(name: &std::ffi::OsStr) -> bool {
    name == VAULT_META_FILENAME || name == FLAGS_FILENAME
}

/// Normalize a plaintext relative path to a canonical `a/b/c` string (only
/// the normal components, forward slashes) -- the key form used in the
/// sensitive-flags manifest so lookups and ancestor walks are consistent.
fn norm_rel(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Serialize, Deserialize)]
struct VaultMeta {
    salt: [u8; 16],
    nonce: [u8; 12],
    wrapped_master_key: Vec<u8>,
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
/// seconds of the *encrypted* on-disk file, which vaultcore's own writes
/// keep in sync with the plaintext's last change -- so no timestamp needs
/// to be stored inside the encrypted format itself.
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    /// Whether this (vault-internal) subdirectory is itself a nested
    /// vault, i.e. it contains its own `.vault.meta`.
    pub is_vault: bool,
    pub size: u64,
    pub mtime: i64,
}

/// A folder of AES-256-GCM encrypted files (names encrypted too, via
/// AES-SIV) unlocked by a single vault password. The folder can be a plain
/// directory or a mount point (e.g. a FUSE mount, network share, removable
/// drive) -- vaultcore only ever touches paths under `root` and never
/// assumes anything about what's backing them.
#[derive(Clone)]
pub struct Vault {
    root: PathBuf,
    master_key: Key32,
    name_key: NameKey,
    /// State of the "sensitive files" re-auth session for this vault.
    /// Files (or whole folders) marked sensitive (see `.vault.flags`) can
    /// only be decrypted while this session is active -- the user re-enters
    /// the vault password to open a time-boxed window, after which sensitive
    /// content locks again. `Arc<Mutex<_>>` (not a bare field) since a FUSE
    /// mount clones the whole `Vault` to run on its own thread, and both
    /// must observe the same session state.
    sensitive: Arc<Mutex<SensitiveState>>,
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

/// Whether `path` looks like plain text worth decrypting for a content
/// search -- deliberately conservative (skips images/video/audio/archives/
/// office docs, which either aren't text at all or need a real parser
/// this doesn't have) rather than trying to sniff arbitrary file content.
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

/// Whether `root` already contains vault metadata, i.e. [`Vault::unlock`]
/// is the right call there rather than [`Vault::create`].
pub fn vault_exists(root: impl AsRef<Path>) -> bool {
    root.as_ref().join(VAULT_META_FILENAME).exists()
}

impl Vault {
    /// Initialize a brand-new vault at `root` (created if it doesn't
    /// exist), protected by `password`.
    pub fn create(root: impl AsRef<Path>, password: &[u8]) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        let meta_path = root.join(VAULT_META_FILENAME);
        if meta_path.exists() {
            return Err(VaultError::VaultExists);
        }

        let master_key = Key32::random();
        let salt = random_salt();
        let password_key = derive_key_from_password(password, &salt)?;
        let (nonce, wrapped_master_key) =
            aead_encrypt(&password_key, MASTER_KEY_WRAP_AAD, &master_key.0)?;

        let meta = VaultMeta {
            salt,
            nonce,
            wrapped_master_key,
        };
        fs::write(&meta_path, bincode::serialize(&meta)?)?;

        let name_key = name::derive_name_key(&master_key);
        Ok(Self {
            root,
            master_key,
            name_key,
            sensitive: Arc::new(Mutex::new(SensitiveState::Locked)),
        })
    }

    /// Open an existing vault at `root` with `password`.
    pub fn unlock(root: impl AsRef<Path>, password: &[u8]) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let meta_path = root.join(VAULT_META_FILENAME);
        if !meta_path.exists() {
            return Err(VaultError::VaultNotFound);
        }
        let meta: VaultMeta = bincode::deserialize(&fs::read(meta_path)?)?;
        let password_key = derive_key_from_password(password, &meta.salt)?;
        let master_key_bytes = Zeroizing::new(
            aead_decrypt(
                &password_key,
                &meta.nonce,
                MASTER_KEY_WRAP_AAD,
                &meta.wrapped_master_key,
            )
            .map_err(|_| VaultError::InvalidPassword)?,
        );

        let mut key = [0u8; 32];
        key.copy_from_slice(&master_key_bytes);
        let master_key = Key32(key);
        let name_key = name::derive_name_key(&master_key);
        Ok(Self {
            root,
            master_key,
            name_key,
            sensitive: Arc::new(Mutex::new(SensitiveState::Locked)),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Maps a plaintext-relative path (e.g. `docs/report.pdf`) to where its
    /// ciphertext lives inside the vault. Every path component is encrypted
    /// independently (AES-SIV, keyed to its parent's plaintext path), so
    /// the on-disk name gives no hint of the original -- only the tree
    /// shape (nesting, entry count) stays visible.
    pub fn encrypted_path(&self, rel_path: &Path) -> Result<PathBuf> {
        let mut plain_parent = PathBuf::new();
        let mut out = self.root.clone();
        for component in rel_path.components() {
            let Component::Normal(os_name) = component else {
                return Err(VaultError::BadHeader);
            };
            let name = os_name.to_str().ok_or(VaultError::BadHeader)?;
            let aad = plain_parent.to_string_lossy();
            let encoded = name::encrypt_name(&self.name_key, aad.as_bytes(), name)?;
            out.push(encoded);
            plain_parent.push(name);
        }
        Ok(out)
    }

    /// Inverse of `encrypted_path`: maps a ciphertext-relative path (as it
    /// sits on disk under the vault root) back to the plaintext relative
    /// path. Fails on any component that doesn't decrypt (not written by
    /// this vault, e.g. a sync tool's `.conflict` copy).
    pub fn decrypt_rel_path(&self, cipher_rel: &Path) -> Result<PathBuf> {
        let mut plain = PathBuf::new();
        for component in cipher_rel.components() {
            let Component::Normal(os_name) = component else {
                return Err(VaultError::BadHeader);
            };
            let encoded = os_name.to_str().ok_or(VaultError::BadHeader)?;
            let aad = plain.to_string_lossy().to_string();
            let name = name::decrypt_name(&self.name_key, aad.as_bytes(), encoded)?;
            plain.push(name);
        }
        Ok(plain)
    }

    fn unwrap_fek_with_master_key(&self, wrapped_keys: &[WrappedKey]) -> Result<Key32> {
        for wk in wrapped_keys {
            if let WrappedKey::MasterKey { nonce, wrapped_fek } = wk {
                if let Ok(bytes) = aead_decrypt(&self.master_key, nonce, FEK_WRAP_AAD, wrapped_fek)
                {
                    let bytes = Zeroizing::new(bytes);
                    let mut key = [0u8; 32];
                    key.copy_from_slice(&bytes);
                    return Ok(Key32(key));
                }
            }
        }
        Err(VaultError::NoMatchingKey)
    }

    /// Encrypt a plaintext file from anywhere on disk into the vault at
    /// `rel_path`. The FEK is generated fresh and wrapped only under this
    /// vault's master key; use [`Vault::add_file_password`] or
    /// [`Vault::add_file_recipient`] afterwards to grant alternate access.
    pub fn encrypt_file(&self, plaintext_path: impl AsRef<Path>, rel_path: impl AsRef<Path>) -> Result<()> {
        let dest = self.encrypted_path(rel_path.as_ref())?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        let reader = BufReader::new(fs::File::open(plaintext_path)?);
        let mut writer = BufWriter::new(fs::File::create(&dest)?);

        let fek = Key32::random();
        let (nonce, wrapped_fek) = aead_encrypt(&self.master_key, FEK_WRAP_AAD, &fek.0)?;
        let wrapped_keys = vec![WrappedKey::MasterKey { nonce, wrapped_fek }];

        file::encrypt_stream(reader, &mut writer, &fek, &wrapped_keys)?;
        writer.flush()?;
        Ok(())
    }

    /// Decrypt `rel_path` (unwrapping its FEK with this vault's master
    /// key) fully into memory. This is the primitive a FUSE/loopback layer
    /// would call on read: plaintext only ever exists transiently in RAM,
    /// never written back to disk unencrypted.
    ///
    /// Every content-reading command in the app (preview, thumbnail,
    /// export, share, Get Info, ...) funnels through this one function,
    /// which is what makes the per-file password gate below actually
    /// hold everywhere rather than just wherever a particular UI surface
    /// remembered to check it.
    pub fn decrypt_file(&self, rel_path: impl AsRef<Path>) -> Result<Vec<u8>> {
        let src = self.encrypted_path(rel_path.as_ref())?;
        let mut reader = BufReader::new(fs::File::open(&src)?);
        let meta = file::read_meta(&mut reader)?;
        if self.is_sensitive(rel_path.as_ref()) && !self.sensitive_unlocked() {
            return Err(VaultError::SensitiveLocked);
        }
        let fek = self.unwrap_fek_with_master_key(&meta.wrapped_keys)?;
        let mut out = Vec::with_capacity(meta.plaintext_len as usize);
        file::decrypt_stream(&mut reader, &mut out, &fek, &meta)?;
        Ok(out)
    }

    // ---- Sensitive files (per-file / per-folder re-auth gate) ----

    fn flags_path(&self) -> PathBuf {
        self.root.join(FLAGS_FILENAME)
    }

    /// Load the set of plaintext paths explicitly marked sensitive. Stored
    /// AEAD-encrypted (master key) as `[12-byte nonce][ciphertext]` so the
    /// manifest itself leaks nothing when the vault is locked. Missing or
    /// unreadable file -> empty set.
    fn load_flags(&self) -> HashSet<String> {
        let Ok(data) = fs::read(self.flags_path()) else {
            return HashSet::new();
        };
        if data.len() < 12 {
            return HashSet::new();
        }
        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&data[..12]);
        let Ok(plain) = aead_decrypt(&self.master_key, &nonce, FLAGS_AAD, &data[12..]) else {
            return HashSet::new();
        };
        bincode::deserialize(&plain).unwrap_or_default()
    }

    fn save_flags(&self, set: &HashSet<String>) -> Result<()> {
        let plain = bincode::serialize(set)?;
        let (nonce, ct) = aead_encrypt(&self.master_key, FLAGS_AAD, &plain)?;
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        fs::write(self.flags_path(), out)?;
        Ok(())
    }

    /// Whether `rel_path` is sensitive -- either marked directly, or living
    /// under a folder that's marked (folder sensitivity is inherited by all
    /// descendants, including files created later).
    pub fn is_sensitive(&self, rel_path: impl AsRef<Path>) -> bool {
        let set = self.load_flags();
        if set.is_empty() {
            return false;
        }
        let rel = norm_rel(rel_path.as_ref());
        let mut p = rel.as_str();
        loop {
            if set.contains(p) {
                return true;
            }
            match p.rfind('/') {
                Some(i) => p = &p[..i],
                None => return false,
            }
        }
    }

    /// The set of paths marked sensitive (raw, not expanded to descendants).
    pub fn list_sensitive(&self) -> Result<Vec<String>> {
        let mut v: Vec<String> = self.load_flags().into_iter().collect();
        v.sort();
        Ok(v)
    }

    /// Mark / unmark `rel_path` (a file or folder) sensitive. Unmarking is
    /// refused when a *parent* folder is sensitive -- the child inherits it
    /// and can't opt out while the folder governs it.
    pub fn set_sensitive(&self, rel_path: impl AsRef<Path>, sensitive: bool) -> Result<()> {
        let rel = norm_rel(rel_path.as_ref());
        let mut set = self.load_flags();
        if sensitive {
            set.insert(rel);
        } else {
            // Refuse if any strict ancestor folder is marked.
            if let Some(i) = rel.rfind('/') {
                let mut anc = &rel[..i];
                loop {
                    if set.contains(anc) {
                        return Err(VaultError::SensitiveInherited);
                    }
                    match anc.rfind('/') {
                        Some(j) => anc = &anc[..j],
                        None => break,
                    }
                }
            }
            set.remove(&rel);
        }
        self.save_flags(&set)
    }

    /// Start (or extend) the sensitive re-auth session by verifying the
    /// vault password again. `timeout = None` means no expiry ("never").
    pub fn unlock_sensitive(&self, password: &[u8], timeout: Option<Duration>) -> Result<()> {
        // Re-verify against the on-disk vault password, independent of the
        // fact that this vault is already unlocked in memory.
        let meta: VaultMeta =
            bincode::deserialize(&fs::read(self.root.join(VAULT_META_FILENAME))?)?;
        let password_key = derive_key_from_password(password, &meta.salt)?;
        let master_bytes = aead_decrypt(
            &password_key,
            &meta.nonce,
            MASTER_KEY_WRAP_AAD,
            &meta.wrapped_master_key,
        )
        .map_err(|_| VaultError::InvalidPassword)?;
        if master_bytes != self.master_key.0 {
            return Err(VaultError::InvalidPassword);
        }
        let state = match timeout {
            Some(d) => SensitiveState::Until(Instant::now() + d),
            None => SensitiveState::Forever,
        };
        *self.sensitive.lock().unwrap() = state;
        Ok(())
    }

    /// Whether the sensitive re-auth session is currently active.
    pub fn sensitive_unlocked(&self) -> bool {
        match *self.sensitive.lock().unwrap() {
            SensitiveState::Locked => false,
            SensitiveState::Forever => true,
            SensitiveState::Until(t) => Instant::now() < t,
        }
    }

    /// End the sensitive session immediately (manual re-lock / timeout).
    pub fn lock_sensitive(&self) {
        *self.sensitive.lock().unwrap() = SensitiveState::Locked;
    }

    /// (Re-)encrypt `plaintext` as `rel_path`'s content. If the file
    /// already exists, its FEK and *every* existing wrapped-key entry
    /// (per-file password, PQ recipients) are preserved -- editing a file
    /// through this method (e.g. a FUSE write-back) doesn't revoke access
    /// grants made with [`Vault::add_file_password`] /
    /// [`Vault::add_file_recipient`]. If it doesn't exist yet, a fresh FEK
    /// is generated and wrapped under the vault's master key, same as
    /// [`Vault::encrypt_file`].
    pub fn write_file(&self, rel_path: impl AsRef<Path>, plaintext: &[u8]) -> Result<()> {
        let dest = self.encrypted_path(rel_path.as_ref())?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        let (fek, wrapped_keys) = if dest.exists() {
            let mut reader = BufReader::new(fs::File::open(&dest)?);
            let meta = file::read_meta(&mut reader)?;
            let fek = self.unwrap_fek_with_master_key(&meta.wrapped_keys)?;
            (fek, meta.wrapped_keys)
        } else {
            let fek = Key32::random();
            let (nonce, wrapped_fek) = aead_encrypt(&self.master_key, FEK_WRAP_AAD, &fek.0)?;
            (fek, vec![WrappedKey::MasterKey { nonce, wrapped_fek }])
        };

        let mut tmp_name = dest.as_os_str().to_os_string();
        tmp_name.push(".tmp");
        let tmp_path = PathBuf::from(tmp_name);
        {
            let mut writer = BufWriter::new(fs::File::create(&tmp_path)?);
            file::encrypt_stream(plaintext, &mut writer, &fek, &wrapped_keys)?;
            writer.flush()?;
        }
        fs::rename(&tmp_path, &dest)?;
        Ok(())
    }

    /// Whether `rel_path` is a file or directory, and its plaintext length
    /// if it's a file (read from the header only -- chunks aren't touched).
    pub fn stat(&self, rel_path: impl AsRef<Path>) -> Result<Stat> {
        let path = self.encrypted_path(rel_path.as_ref())?;
        if !path.exists() {
            return Err(VaultError::PathNotFound);
        }
        if path.is_dir() {
            return Ok(Stat {
                is_dir: true,
                len: 0,
            });
        }
        let mut reader = BufReader::new(fs::File::open(&path)?);
        let meta = file::read_meta(&mut reader)?;
        Ok(Stat {
            is_dir: false,
            len: meta.plaintext_len,
        })
    }

    /// List the immediate (decrypted) children of `rel_path`, each with
    /// whether it's a directory, its plaintext size and last-modified time.
    /// Unlike [`Vault::list_files`], this is a single directory level, not a
    /// full recursive walk -- what a FUSE `readdir` call needs.
    pub fn list_dir(&self, rel_path: impl AsRef<Path>) -> Result<Vec<DirEntry>> {
        let rel_path = rel_path.as_ref();
        // A sensitive folder's listing (names, sizes, shape) is content --
        // enforced here, not only in the UI, so every consumer (preview
        // pane, FUSE readdir, search) hits the same wall as decrypt_file.
        if self.is_sensitive(rel_path) && !self.sensitive_unlocked() {
            return Err(VaultError::SensitiveLocked);
        }
        let enc_dir = self.encrypted_path(rel_path)?;
        let aad = rel_path.to_string_lossy();
        let mut out = Vec::new();
        for entry in fs::read_dir(&enc_dir)? {
            let entry = entry?;
            if is_reserved_name(&entry.file_name()) {
                continue;
            }
            let Some(encoded) = entry.file_name().to_str().map(|s| s.to_string()) else {
                continue;
            };
            // A name that doesn't decrypt isn't necessarily vault damage --
            // the common case is a foreign file dropped into the vault dir
            // by something outside VaultExplorer (most often a cloud-sync
            // conflict copy, e.g. rclone bisync's `<name>.conflict1`/
            // `.conflict2` suffix, which breaks the AES-SIV ciphertext+tag).
            // Skip it rather than failing the whole listing -- one stray
            // file shouldn't make an otherwise-healthy vault look corrupt.
            let Ok(plain_name) = name::decrypt_name(&self.name_key, aad.as_bytes(), &encoded)
            else {
                eprintln!("vault: skipping undecryptable entry {encoded:?} in {aad:?}");
                continue;
            };
            let path = entry.path();
            let is_dir = path.is_dir();
            let metadata = entry.metadata()?;
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let size = if is_dir {
                0
            } else {
                let mut reader = BufReader::new(fs::File::open(&path)?);
                file::read_meta(&mut reader)?.plaintext_len
            };
            // A subdir is a nested vault if it contains a `.vault.meta`.
            // Inside a vault every name is encrypted, so the literal filename
            // never appears on disk -- instead derive the encrypted on-disk
            // name of `.vault.meta` for this subdir (AES-SIV is deterministic:
            // same key + AAD + name => same ciphertext, and that's exactly the
            // name the parent wrote it under) and test for that. AAD is the
            // subdir's own plaintext path, matching `encrypted_path`.
            let is_vault = is_dir && {
                let child_plain = if aad.is_empty() {
                    plain_name.clone()
                } else {
                    format!("{}/{}", aad, plain_name)
                };
                name::encrypt_name(&self.name_key, child_plain.as_bytes(), VAULT_META_FILENAME)
                    .map(|enc| path.join(enc).exists())
                    .unwrap_or(false)
            };
            out.push(DirEntry {
                name: plain_name,
                is_dir,
                is_vault,
                size,
                mtime,
            });
        }
        Ok(out)
    }

    /// Create an empty directory at `rel_path`.
    pub fn create_dir(&self, rel_path: impl AsRef<Path>) -> Result<()> {
        let path = self.encrypted_path(rel_path.as_ref())?;
        fs::create_dir_all(path)?;
        Ok(())
    }

    /// Absorb an existing plaintext file/dir sitting at `src` (an absolute
    /// on-disk path) INTO this vault at plaintext `rel`, then remove the
    /// plaintext original. Used by "Convert to Vault" to encrypt a folder's
    /// pre-existing contents in place. The encrypted output has an encrypted
    /// name (see `encrypted_path`), so it never collides with the plaintext
    /// `src` it's reading, even when both live under the vault root.
    pub fn absorb(&self, src: &Path, rel: &Path) -> Result<()> {
        if src.is_dir() {
            self.create_dir(rel)?;
            for entry in fs::read_dir(src)? {
                let entry = entry?;
                self.absorb(&entry.path(), &rel.join(entry.file_name()))?;
            }
            // Children have each been encrypted + their plaintext removed;
            // the plaintext dir is now empty.
            fs::remove_dir_all(src)?;
        } else {
            self.encrypt_file(src, rel)?;
            fs::remove_file(src)?;
        }
        Ok(())
    }

    /// Remove a directory at `rel_path` and everything under it.
    pub fn remove_dir(&self, rel_path: impl AsRef<Path>) -> Result<()> {
        let path = self.encrypted_path(rel_path.as_ref())?;
        fs::remove_dir_all(path)?;
        Ok(())
    }

    /// Delete the file at `rel_path`.
    pub fn remove_file(&self, rel_path: impl AsRef<Path>) -> Result<()> {
        let path = self.encrypted_path(rel_path.as_ref())?;
        fs::remove_file(path)?;
        Ok(())
    }

    /// Grant a hybrid X25519 + ML-KEM768 recipient access to `rel_path`,
    /// so they can decrypt it with their own secret key -- resistant to a
    /// future quantum adversary that recorded the ciphertext today.
    pub fn add_file_recipient(
        &self,
        rel_path: impl AsRef<Path>,
        recipient: &RecipientPublicKey,
    ) -> Result<()> {
        let path = self.encrypted_path(rel_path.as_ref())?;
        let mut reader = BufReader::new(fs::File::open(&path)?);
        let mut meta = file::read_meta(&mut reader)?;
        let fek = self.unwrap_fek_with_master_key(&meta.wrapped_keys)?;

        let wrap = crate::pq::wrap_for_recipient(recipient, &fek)?;
        meta.wrapped_keys.push(WrappedKey::Recipient {
            x25519_ephemeral_public: wrap.x25519_ephemeral_public,
            kem_ciphertext: wrap.kem_ciphertext,
            nonce: wrap.nonce,
            wrapped_fek: wrap.wrapped_fek,
        });

        rewrite_header(&path, &meta, &mut reader)
    }

    /// Recursively encrypt every file under `plaintext_dir` into the vault,
    /// mirroring its directory structure (with each name encrypted).
    pub fn encrypt_dir(&self, plaintext_dir: impl AsRef<Path>) -> Result<()> {
        let plaintext_dir = plaintext_dir.as_ref();
        for entry in walk_files(plaintext_dir)? {
            let rel = entry.strip_prefix(plaintext_dir).expect("walked under plaintext_dir");
            self.encrypt_file(&entry, rel)?;
        }
        Ok(())
    }

    /// List every encrypted file in the vault, as plaintext-relative paths.
    /// Walks the (visible) directory tree top-down, decrypting each name in
    /// turn -- decrypting a child requires already knowing its parent's
    /// plaintext path, since that's the AAD it was encrypted under.
    pub fn list_files(&self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        self.walk_decrypt(&self.root, Path::new(""), &mut out)?;
        Ok(out)
    }

    /// Case-insensitive substring search over plaintext file/folder names,
    /// plus (for files that look like plain text) their decrypted content.
    /// No index -- every text-like file that doesn't already match by name
    /// gets decrypted and scanned on the spot. That's the right tradeoff
    /// for the personal-notes-sized vaults this is built for; a vault with
    /// thousands of text files would want a real index instead, but nobody
    /// asked for that yet.
    pub fn search(&self, query: &str) -> Result<Vec<PathBuf>> {
        let query = query.to_lowercase();
        let files = self.list_files()?;
        let mut out = Vec::new();
        for p in files {
            let name_matches = p.to_string_lossy().to_lowercase().contains(&query);
            let content_matches = !name_matches
                && is_searchable_text(&p)
                && self
                    .decrypt_file(&p)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .is_some_and(|text| text.to_lowercase().contains(&query));
            if name_matches || content_matches {
                out.push(p);
            }
        }
        Ok(out)
    }

    /// Total plaintext size of `rel_path`: its own size if it's a file, or
    /// the sum of every file under it if it's a directory. Used by the
    /// "Get Info" panel, which shows a shallow size instantly and this
    /// recursive total once it resolves.
    pub fn dir_size(&self, rel_path: impl AsRef<Path>) -> Result<u64> {
        let rel_path = rel_path.as_ref();
        let enc = self.encrypted_path(rel_path)?;
        if !enc.is_dir() {
            return Ok(self.stat(rel_path)?.len);
        }
        let mut leaves = Vec::new();
        self.walk_decrypt(&enc, rel_path, &mut leaves)?;
        let mut total = 0u64;
        for leaf in &leaves {
            total += self.stat(leaf)?.len;
        }
        Ok(total)
    }

    /// Compress `names` (files and/or directories, all children of
    /// `dir_rel`) into a single zip archive written (encrypted, like any
    /// other vault file) at `dest_rel`. Directory contents are recursed
    /// into and each entry's path in the archive is relative to `dir_rel`,
    /// mirroring Finder's "Compress" behavior.
    pub fn compress_paths(
        &self,
        dir_rel: impl AsRef<Path>,
        names: &[String],
        dest_rel: impl AsRef<Path>,
        opts: &CompressOptions,
    ) -> Result<()> {
        let dir_rel = dir_rel.as_ref();
        let mut leaves = Vec::new();
        for name in names {
            let rel = dir_rel.join(name);
            let enc = self.encrypted_path(&rel)?;
            if enc.is_dir() {
                self.walk_decrypt(&enc, &rel, &mut leaves)?;
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

    /// Decrypt the zip archive at `zip_rel` and extract its entries under
    /// `dest_dir_rel`, re-encrypting each file back into the vault.
    /// `password` unlocks an AES-encrypted archive (from [`Vault::compress_paths`]
    /// with a password set); pass `None` for a plain archive.
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

    /// Move (rename/relocate) a file or directory within the vault. File
    /// content ciphertext is never decrypted -- it's relocated byte for
    /// byte. Only *names* are re-derived for the new location, since name
    /// encryption is bound to each entry's parent plaintext path.
    ///
    /// Known limitation: an empty subdirectory (no files anywhere under
    /// it) has nothing to re-derive a name from and is silently dropped
    /// rather than carried over -- the vault only ever tracks directories
    /// implicitly, as the parent path of some file.
    pub fn move_path(&self, src_rel: impl AsRef<Path>, dest_rel: impl AsRef<Path>) -> Result<()> {
        self.relocate(src_rel.as_ref(), dest_rel.as_ref(), true)
    }

    /// Copy a file or directory within the vault; the source is left
    /// untouched. Same content-never-decrypted property as [`Vault::move_path`].
    pub fn copy_path(&self, src_rel: impl AsRef<Path>, dest_rel: impl AsRef<Path>) -> Result<()> {
        self.relocate(src_rel.as_ref(), dest_rel.as_ref(), false)
    }

    fn relocate(&self, src_rel: &Path, dest_rel: &Path, remove_src: bool) -> Result<()> {
        let src = self.encrypted_path(src_rel)?;
        if !src.exists() {
            return Err(VaultError::PathNotFound);
        }
        if src.is_dir() {
            let mut leaves = Vec::new();
            let mut dirs = Vec::new();
            self.walk_decrypt_all(&src, src_rel, &mut leaves, &mut dirs)?;
            // The destination directories are created explicitly rather than
            // left to fall out of moving files into them. `relocate_single_file`
            // only creates the parents of a file it actually moves, so a
            // directory with no files under it had nothing to imply it: an
            // EMPTY directory was "moved" by creating nothing at all and then
            // deleting the source below -- i.e. renaming a freshly created
            // folder inside a vault made it vanish. Same for every empty
            // subdirectory of a tree being moved.
            self.create_dir(dest_rel)?;
            for dir in &dirs {
                let rel_under = dir.strip_prefix(src_rel).expect("walked under src_rel");
                self.create_dir(dest_rel.join(rel_under))?;
            }
            for leaf in &leaves {
                let rel_under = leaf.strip_prefix(src_rel).expect("walked under src_rel");
                self.relocate_single_file(leaf, &dest_rel.join(rel_under), remove_src)?;
            }
            if remove_src {
                let _ = fs::remove_dir_all(&src);
            }
            Ok(())
        } else {
            self.relocate_single_file(src_rel, dest_rel, remove_src)
        }
    }

    fn relocate_single_file(&self, src_rel: &Path, dest_rel: &Path, remove_src: bool) -> Result<()> {
        let src = self.encrypted_path(src_rel)?;
        let dest = self.encrypted_path(dest_rel)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        if remove_src {
            fs::rename(&src, &dest)?;
        } else {
            fs::copy(&src, &dest)?;
        }
        Ok(())
    }

    fn walk_decrypt(&self, enc_dir: &Path, plain_prefix: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        let mut dirs = Vec::new();
        self.walk_decrypt_all(enc_dir, plain_prefix, out, &mut dirs)
    }

    /// Like [`Vault::walk_decrypt`], but also reports the directories it
    /// walked through (in plaintext-path form, parents before children) --
    /// what a caller needs to mirror a tree's shape, empty directories
    /// included, instead of only its files.
    fn walk_decrypt_all(
        &self,
        enc_dir: &Path,
        plain_prefix: &Path,
        out: &mut Vec<PathBuf>,
        dirs: &mut Vec<PathBuf>,
    ) -> Result<()> {
        for entry in fs::read_dir(enc_dir)? {
            let entry = entry?;
            if is_reserved_name(&entry.file_name()) {
                continue;
            }
            let Some(encoded) = entry.file_name().to_str().map(|s| s.to_string()) else {
                continue;
            };
            let aad = plain_prefix.to_string_lossy();
            // See the matching skip in `list_dir` -- a foreign/conflict
            // file shouldn't fail a whole recursive walk either.
            let Ok(plain_name) = name::decrypt_name(&self.name_key, aad.as_bytes(), &encoded)
            else {
                eprintln!("vault: skipping undecryptable entry {encoded:?} in {aad:?}");
                continue;
            };
            let plain_path = plain_prefix.join(&plain_name);

            let path = entry.path();
            if path.is_dir() {
                dirs.push(plain_path.clone());
                self.walk_decrypt_all(&path, &plain_path, out, dirs)?;
            } else {
                out.push(plain_path);
            }
        }
        Ok(())
    }
}

/// Replace an encrypted file's header in place (new wrapped_keys list),
/// leaving its chunk ciphertext untouched. `reader` must already be
/// positioned at the start of chunk data (i.e. right after
/// [`file::read_meta`] consumed the old header).
fn rewrite_header<R: std::io::Read>(
    path: &Path,
    meta: &file::FileMeta,
    reader: &mut R,
) -> Result<()> {
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = PathBuf::from(tmp_name);
    {
        let mut writer = BufWriter::new(fs::File::create(&tmp_path)?);
        file::write_header(
            &mut writer,
            &meta.nonce_prefix,
            meta.chunk_size,
            meta.plaintext_len,
            &meta.wrapped_keys,
        )?;
        std::io::copy(reader, &mut writer)?;
        writer.flush()?;
    }
    fs::rename(&tmp_path, path)?;
    Ok(())
}

fn walk_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// Change a vault's password in O(1), without re-encrypting any file.
/// The vault's random master key is preserved -- only the small
/// `.vault.meta` is rewritten, re-wrapping that same master key under a key
/// derived from `new_password` (fresh salt). Every file's FEK stays wrapped
/// under the unchanged master key, so all children remain decryptable with
/// the new password immediately. `old_password` is verified first.
pub fn change_password(
    root: impl AsRef<Path>,
    old_password: &[u8],
    new_password: &[u8],
) -> Result<()> {
    let meta_path = root.as_ref().join(VAULT_META_FILENAME);
    let meta: VaultMeta = bincode::deserialize(&fs::read(&meta_path)?)?;
    let old_key = derive_key_from_password(old_password, &meta.salt)?;
    let master_bytes = Zeroizing::new(
        aead_decrypt(
            &old_key,
            &meta.nonce,
            MASTER_KEY_WRAP_AAD,
            &meta.wrapped_master_key,
        )
        .map_err(|_| VaultError::InvalidPassword)?,
    );
    let salt = random_salt();
    let new_key = derive_key_from_password(new_password, &salt)?;
    let (nonce, wrapped_master_key) = aead_encrypt(&new_key, MASTER_KEY_WRAP_AAD, &master_bytes)?;
    let new_meta = VaultMeta {
        salt,
        nonce,
        wrapped_master_key,
    };
    fs::write(&meta_path, bincode::serialize(&new_meta)?)?;
    Ok(())
}

/// Encrypt `plaintext` into standalone vault-format bytes, unlockable by
/// [`decrypt_file_with_password`] with `password` alone -- no [`Vault`]
/// needed. Used for the one-off "Encrypt file" action on a plain file,
/// independent of any vault.
pub fn encrypt_file_with_password(plaintext: &[u8], password: &[u8]) -> Result<Vec<u8>> {
    let fek = Key32::random();
    let salt = random_salt();
    let password_key = derive_key_from_password(password, &salt)?;
    let (nonce, wrapped_fek) = aead_encrypt(&password_key, FEK_WRAP_AAD, &fek.0)?;
    let wrapped_keys = vec![WrappedKey::Password {
        salt,
        nonce,
        wrapped_fek,
    }];
    let mut buf = Cursor::new(Vec::new());
    file::encrypt_stream(plaintext, &mut buf, &fek, &wrapped_keys)?;
    Ok(buf.into_inner())
}

/// Decrypt standalone vault-format bytes (as produced by
/// [`encrypt_file_with_password`]) using a per-file password, without
/// needing any [`Vault`] or a file on disk -- used both for standalone
/// `.vlt` files and for a `.vlt` layer living *inside* a vault (where the
/// vault's own decrypt already produced these bytes in memory).
pub fn decrypt_bytes_with_password(bytes: &[u8], password: &[u8]) -> Result<Vec<u8>> {
    let mut reader = Cursor::new(bytes);
    let meta = file::read_meta(&mut reader)?;
    for wk in &meta.wrapped_keys {
        if let WrappedKey::Password {
            salt,
            nonce,
            wrapped_fek,
        } = wk
        {
            let key = derive_key_from_password(password, salt)?;
            if let Ok(fek_bytes) = aead_decrypt(&key, nonce, FEK_WRAP_AAD, wrapped_fek) {
                let fek_bytes = Zeroizing::new(fek_bytes);
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&fek_bytes);
                let fek = Key32(arr);
                let mut out = Vec::with_capacity(meta.plaintext_len as usize);
                file::decrypt_stream(&mut reader, &mut out, &fek, &meta)?;
                return Ok(out);
            }
        }
    }
    Err(VaultError::InvalidPassword)
}

/// Decrypt a single vault-format file using a standalone per-file password,
/// without needing any [`Vault`] at all. Mirrors how [`Vault::decrypt_file`]
/// works but for files shared outside their originating vault.
pub fn decrypt_file_with_password(path: impl AsRef<Path>, password: &[u8]) -> Result<Vec<u8>> {
    let bytes = fs::read(path.as_ref())?;
    decrypt_bytes_with_password(&bytes, password)
}

/// Decrypt a single vault-format file as a PQ recipient, without needing
/// any [`Vault`] at all.
pub fn decrypt_file_as_recipient(
    path: impl AsRef<Path>,
    secret: &RecipientSecretKey,
) -> Result<Vec<u8>> {
    let mut reader = BufReader::new(fs::File::open(path.as_ref())?);
    let meta = file::read_meta(&mut reader)?;
    for wk in &meta.wrapped_keys {
        if let WrappedKey::Recipient {
            x25519_ephemeral_public,
            kem_ciphertext,
            nonce,
            wrapped_fek,
        } = wk
        {
            let wrap = HybridWrap {
                x25519_ephemeral_public: *x25519_ephemeral_public,
                kem_ciphertext: kem_ciphertext.clone(),
                nonce: *nonce,
                wrapped_fek: wrapped_fek.clone(),
            };
            if let Ok(fek) = crate::pq::unwrap_as_recipient(secret, &wrap) {
                let mut out = Vec::with_capacity(meta.plaintext_len as usize);
                file::decrypt_stream(&mut reader, &mut out, &fek, &meta)?;
                return Ok(out);
            }
        }
    }
    Err(VaultError::NoMatchingKey)
}

#[cfg(test)]
mod relocate_tests {
    use super::*;

    /// Renaming an EMPTY directory used to delete it: `relocate` mirrored a
    /// directory by moving the files under it, and an empty directory has
    /// none, so nothing was created at the destination before the source was
    /// removed. This is the regression guard for that -- it silently
    /// destroyed user data (a folder created in the app, then renamed).
    #[test]
    fn renaming_an_empty_dir_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::create(dir.path(), b"pw").unwrap();
        vault.create_dir("untitled folder").unwrap();

        vault.move_path("untitled folder", "notes").unwrap();

        let names: Vec<String> = vault.list_dir("").unwrap().into_iter().map(|e| e.name).collect();
        assert!(names.contains(&"notes".to_string()), "renamed dir missing: {names:?}");
        assert!(!names.contains(&"untitled folder".to_string()), "old name left behind: {names:?}");
    }

    /// Same shape one level down: a subdirectory that holds nothing must
    /// survive its parent being moved.
    #[test]
    fn moving_a_tree_keeps_empty_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let plain = tempfile::tempdir().unwrap();
        let file = plain.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();

        let vault = Vault::create(dir.path(), b"pw").unwrap();
        vault.create_dir("src/empty").unwrap();
        vault.encrypt_file(&file, "src/a.txt").unwrap();

        vault.move_path("src", "dst").unwrap();

        let top: Vec<String> = vault.list_dir("dst").unwrap().into_iter().map(|e| e.name).collect();
        assert!(top.contains(&"a.txt".to_string()), "file lost: {top:?}");
        assert!(top.contains(&"empty".to_string()), "empty subdir lost: {top:?}");
    }
}
