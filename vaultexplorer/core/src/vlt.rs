//! Standalone password-encrypted files (`.vlt`).
//!
//! Not a vault: one file, one password, nothing else on disk. This is what
//! the "Encrypt file…" action produces, and it is deliberately independent
//! of the Cryptomator vault format -- a `.vlt` is meant to be handed to
//! someone with just a password, not to be a vault they have to mount.

use crate::crypto::{aead_decrypt, aead_encrypt, derive_key_from_password, random_salt, Key32};
use crate::error::{Result, VaultError};
use crate::file;
use crate::header::WrappedKey;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use zeroize::Zeroizing;

const FEK_WRAP_AAD: &[u8] = b"vaultcore-fek-wrap";

/// Encrypt `plaintext` into standalone bytes, unlockable by
/// [`decrypt_file_with_password`] with `password` alone.
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

/// Decrypt standalone bytes (as produced by [`encrypt_file_with_password`])
/// -- used both for a `.vlt` on disk and for one living *inside* a vault,
/// where the vault's own decrypt already produced these bytes in memory.
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

/// Decrypt a `.vlt` file on disk.
pub fn decrypt_file_with_password(path: impl AsRef<Path>, password: &[u8]) -> Result<Vec<u8>> {
    let bytes = fs::read(path.as_ref())?;
    decrypt_bytes_with_password(&bytes, password)
}
