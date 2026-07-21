//! Filename/directory-name encryption, independent of file content
//! encryption. Each path *component* (one file or directory name) is
//! encrypted on its own with AES-SIV, keyed to a name-encryption key
//! derived from the vault master key.
//!
//! AES-SIV (RFC 5297) is deterministic and nonce-misuse-resistant, so we
//! deliberately use a fixed nonce: the same (key, AAD, plaintext name)
//! always produces the same ciphertext. That determinism is what lets us
//! compute a file's on-disk path directly from its plaintext path (and
//! back), without maintaining a separate name index. Unlike AES-GCM, this
//! is safe specifically because SIV is designed not to catastrophically
//! fail under nonce reuse/omission -- reusing a fixed nonce with GCM would
//! break confidentiality entirely.
//!
//! The AAD is the plaintext path of the *parent* directory. This means
//! identical filenames in different directories still encrypt to different
//! ciphertext (no cross-directory pattern leakage), while directory
//! *structure* itself (the tree shape, nesting, entry count) stays visible
//! on disk -- an accepted tradeoff to keep the on-disk layout a plain
//! mirror of the plaintext tree.

use crate::crypto::Key32;
use crate::error::{Result, VaultError};
use aes_siv::aead::{Aead, KeyInit, Payload};
use aes_siv::{Aes256SivAead, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

const SIV_KEY_LEN: usize = 64; // AES-256-SIV uses two 32-byte subkeys.
const SIV_NONCE_LEN: usize = 16;
const ZERO_NONCE: [u8; SIV_NONCE_LEN] = [0u8; SIV_NONCE_LEN];

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct NameKey([u8; SIV_KEY_LEN]);

pub fn derive_name_key(master_key: &Key32) -> NameKey {
    let hk = Hkdf::<Sha256>::new(None, &master_key.0);
    let mut out = [0u8; SIV_KEY_LEN];
    hk.expand(b"vaultcore-name-enc-v1", &mut out)
        .expect("HKDF expand of 64 bytes never fails");
    NameKey(out)
}

/// Encrypt one path component (`name`), bound to `parent_plain_path` (the
/// plaintext path of the directory it lives in) via AAD. Returns a
/// filesystem-safe (base64url, no padding) token to use as the on-disk
/// name.
pub fn encrypt_name(key: &NameKey, parent_plain_path: &[u8], name: &str) -> Result<String> {
    let cipher = Aes256SivAead::new(key.0.as_slice().into());
    let nonce = Nonce::from_slice(&ZERO_NONCE);
    let ciphertext = cipher
        .encrypt(
            nonce,
            Payload {
                msg: name.as_bytes(),
                aad: parent_plain_path,
            },
        )
        .map_err(|_| VaultError::Crypto)?;
    Ok(URL_SAFE_NO_PAD.encode(ciphertext))
}

/// Inverse of [`encrypt_name`]: recover the plaintext name from its on-disk
/// token, given the same parent plaintext path used to encrypt it.
pub fn decrypt_name(key: &NameKey, parent_plain_path: &[u8], encoded: &str) -> Result<String> {
    let ciphertext = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| VaultError::BadHeader)?;
    let cipher = Aes256SivAead::new(key.0.as_slice().into());
    let nonce = Nonce::from_slice(&ZERO_NONCE);
    let plaintext = cipher
        .decrypt(
            nonce,
            Payload {
                msg: &ciphertext,
                aad: parent_plain_path,
            },
        )
        .map_err(|_| VaultError::Crypto)?;
    String::from_utf8(plaintext).map_err(|_| VaultError::BadHeader)
}
