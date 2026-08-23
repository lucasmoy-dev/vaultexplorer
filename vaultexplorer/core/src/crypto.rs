use crate::error::{Result, VaultError};
use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Key, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use rand_core::{OsRng, RngCore};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const KEY_LEN: usize = 32;
pub const GCM_NONCE_LEN: usize = 12;
pub const SALT_LEN: usize = 16;

/// A 256-bit key that zeroes itself on drop. Used for the vault master key,
/// per-file encryption keys, and password-derived wrapping keys alike.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Key32(pub [u8; KEY_LEN]);

impl Key32 {
    pub fn random() -> Self {
        let mut buf = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut buf);
        Key32(buf)
    }
}

/// Argon2id parameters. Tuned for an interactive desktop/mobile unlock
/// (roughly hundreds of ms), not for a low-power embedded target.
fn argon2_params() -> Params {
    // 64 MiB memory, 3 iterations, 4 lanes.
    Params::new(64 * 1024, 3, 4, Some(KEY_LEN)).expect("valid argon2 params")
}

/// Derive a 256-bit key from a password + salt via Argon2id.
pub fn derive_key_from_password(password: &[u8], salt: &[u8; SALT_LEN]) -> Result<Key32> {
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2_params());
    let mut out = [0u8; KEY_LEN];
    argon2
        .hash_password_into(password, salt, &mut out)
        .map_err(|e| VaultError::Kdf(e.to_string()))?;
    Ok(Key32(out))
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    OsRng.fill_bytes(&mut buf);
    buf
}

pub fn random_salt() -> [u8; SALT_LEN] {
    random_bytes::<SALT_LEN>()
}

pub fn random_nonce() -> [u8; GCM_NONCE_LEN] {
    random_bytes::<GCM_NONCE_LEN>()
}

/// Encrypt `plaintext` with AES-256-GCM under `key`, using a fresh random
/// nonce. Returns (nonce, ciphertext||tag). `aad` binds context (e.g. chunk
/// index, purpose) into the tag without being encrypted itself.
pub fn aead_encrypt(key: &Key32, aad: &[u8], plaintext: &[u8]) -> Result<([u8; GCM_NONCE_LEN], Vec<u8>)> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key.0));
    let nonce_bytes = random_nonce();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad })
        .map_err(|_| VaultError::Crypto)?;
    Ok((nonce_bytes, ciphertext))
}

/// Encrypt with an explicit, caller-supplied nonce. Only safe to use when the
/// caller guarantees the (key, nonce) pair is never reused -- e.g. a
/// per-file random prefix combined with a strictly increasing chunk counter.
pub fn aead_encrypt_with_nonce(
    key: &Key32,
    nonce_bytes: &[u8; GCM_NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key.0));
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .encrypt(nonce, Payload { msg: plaintext, aad })
        .map_err(|_| VaultError::Crypto)
}

pub fn aead_decrypt(
    key: &Key32,
    nonce_bytes: &[u8; GCM_NONCE_LEN],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key.0));
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, Payload { msg: ciphertext, aad })
        .map_err(|_| VaultError::Crypto)
}
