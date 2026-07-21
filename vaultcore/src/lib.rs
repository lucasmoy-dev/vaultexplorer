//! vaultcore: an encrypted-folder engine, portable to desktop and mobile.
//!
//! - File contents: AES-256-GCM, chunked so large files stream instead of
//!   loading whole into memory to encrypt/decrypt.
//! - File and folder *names*: AES-SIV, deterministic and bound to their
//!   parent's plaintext path so identical names in different folders don't
//!   look alike on disk. The directory *tree shape* (nesting, entry count)
//!   is intentionally left visible -- only names are hidden.
//! - Vault unlock: Argon2id password -> master key, itself AES-256-GCM
//!   wrapped so the master key never touches disk in the clear.
//! - Sharing: hybrid X25519 + ML-KEM768 (FIPS 203) key encapsulation, so a
//!   file can be wrapped for a recipient in a way that resists both a
//!   classical and a future large-scale quantum attacker.
//!
//! This crate only ever reads/writes complete files -- it has no opinion on
//! *how* `root` gets exposed to the rest of the OS. A FUSE/WinFsp/loopback
//! layer built on top of [`Vault::decrypt_file`] / [`Vault::encrypt_file`]
//! is what would let a native app "just open" a file from an encrypted
//! folder without ever writing plaintext to disk.

mod crypto;
mod error;
mod file;
#[cfg(unix)]
#[cfg(all(unix, not(target_os = "android")))]
pub mod fuse_mount;
mod header;
mod name;
mod pq;
mod vault;

pub use crypto::Key32;
pub use error::{Result, VaultError};
pub use header::WrappedKey;
pub use pq::{HybridWrap, RecipientPublicKey, RecipientSecretKey};
pub use vault::{
    decrypt_bytes_with_password, decrypt_file_as_recipient, decrypt_file_with_password,
    encrypt_file_with_password, vault_exists, CompressOptions, DirEntry, Stat, Vault,
};
