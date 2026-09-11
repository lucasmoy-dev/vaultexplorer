//! vaultcore: the vault engine behind Vault Explorer.
//!
//! A vault here **is a Cryptomator vault** (vault format 8, `SIV_GCM`):
//! folders that Cryptomator's own apps made open here, and folders made
//! here open in Cryptomator. The format work is done by
//! `cryptomator-rs-crypto` (Apache-2.0); this crate wraps it in the shape
//! the app speaks -- plaintext-relative paths, whole-file and ranged
//! reads, a FUSE mount -- and adds two things Cryptomator has no notion
//! of, both stored as ordinary (encrypted) files inside the vault:
//!
//! - files or folders marked *sensitive*, which need the password again
//!   before they decrypt (see [`Vault::set_sensitive`]);
//! - nothing else.
//!
//! Separately, and nothing to do with vaults: [`encrypt_file_with_password`]
//! and friends handle standalone `.vlt` files -- one file, one password.

mod crypto;
mod error;
mod file;
#[cfg(all(unix, not(target_os = "android")))]
pub mod fuse_mount;
mod header;
mod masterkey;
mod vault;
mod vlt;

pub use crypto::Key32;
pub use error::{Result, VaultError};
pub use header::WrappedKey;
pub use masterkey::change_password;
pub use vault::{vault_exists, CompressOptions, DirEntry, Stat, Vault, VaultFileReader};
pub use vlt::{
    decrypt_bytes_with_password, decrypt_file_with_password, encrypt_file_with_password,
};
