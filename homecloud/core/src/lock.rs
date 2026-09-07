//! Folder passwords.
//!
//! Syncthing has no notion of a password: what authenticates a device is its
//! certificate, and there is nowhere in the protocol to put a shared secret.
//! So a folder password is not something the engine checks — it is what the
//! pairing code is encrypted with.
//!
//! That inversion is what makes it work. A code for a locked folder carries no
//! readable device id and no folder id, so somebody who has the code but not
//! the password has nothing to redeem: there is no request they can even
//! construct. It also means the password is checked before anything touches
//! the network, by whoever is holding the code.
//!
//! The password itself is never stored. What is kept, so this device can write
//! more codes for the same folder later, is the salt and the key derived from
//! it — enough to encrypt, and enough to recognise the right password, but not
//! enough to recover the password itself.

use argon2::Argon2;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

use crate::error::{Error, Result};

/// Argon2id with parameters that stay usable on a phone.
///
/// A pairing code is short-lived and already semi-secret, so the work factor is
/// tuned to be unpleasant for a bulk attacker rather than maximal: 19 MiB and
/// two passes take well under a second on a mid-range phone, and turn a
/// dictionary run into something that costs real hardware.
fn derive(password: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let params = argon2::Params::new(19 * 1024, 2, 1, Some(32))
        .map_err(|e| Error::BadPairingCode(format!("could not set up the password: {e}")))?;
    let argon = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| Error::BadPairingCode(format!("could not use that password: {e}")))?;
    Ok(key)
}

/// What a device keeps so it can lock future codes for the same folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderKey {
    pub salt: [u8; 16],
    pub key: [u8; 32],
}

impl FolderKey {
    /// Sets a password on a folder for the first time.
    pub fn create(password: &str) -> Result<Self> {
        let mut salt = [0u8; 16];
        fill_random(&mut salt);
        Ok(FolderKey { salt, key: derive(password, &salt)? })
    }

    /// Rebuilds the key from a password and a stored salt, which is also how a
    /// password is checked: the wrong one derives a different key.
    pub fn from_password(password: &str, salt: [u8; 16]) -> Result<Self> {
        Ok(FolderKey { salt, key: derive(password, &salt)? })
    }

    pub fn matches(&self, other: &FolderKey) -> bool {
        // Constant time: this is a password check, and an early return on the
        // first differing byte is a timing oracle.
        self.key.iter().zip(other.key.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }

    /// Encrypts a code body. The salt and nonce travel with it, unencrypted,
    /// because the receiver needs both to derive the same key.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&self.key));
        let mut nonce = [0u8; 24];
        fill_random(&mut nonce);
        let sealed = cipher
            .encrypt(XNonce::from_slice(&nonce), plaintext)
            .map_err(|_| Error::BadPairingCode("could not lock the code".into()))?;

        let mut out = Vec::with_capacity(16 + 24 + sealed.len());
        out.extend_from_slice(&self.salt);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// The salt a sealed body was made with, so a password can be turned into
    /// the right key before trying to open it.
    pub fn salt_of(sealed: &[u8]) -> Result<[u8; 16]> {
        if sealed.len() < 16 + 24 {
            return Err(Error::BadPairingCode("it looks truncated or altered".into()));
        }
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&sealed[..16]);
        Ok(salt)
    }

    /// Opens a sealed body. A wrong password fails here, before anything is
    /// asked of the network.
    pub fn open(&self, sealed: &[u8]) -> Result<Vec<u8>> {
        if sealed.len() < 16 + 24 {
            return Err(Error::BadPairingCode("it looks truncated or altered".into()));
        }
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&self.key));
        cipher
            .decrypt(XNonce::from_slice(&sealed[16..40]), &sealed[40..])
            .map_err(|_| Error::BadPairingCode("la contraseña no es correcta".into()))
    }
}

/// Random bytes from the OS.
fn fill_random(buffer: &mut [u8]) {
    use rand::RngCore;
    rand::thread_rng().fill_bytes(buffer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_locked_with_a_password_opens_with_it() {
        let lock = FolderKey::create("las fotos de la abuela").unwrap();
        let sealed = lock.seal(b"el cuerpo del codigo").unwrap();
        let salt = FolderKey::salt_of(&sealed).unwrap();
        let reader = FolderKey::from_password("las fotos de la abuela", salt).unwrap();
        assert_eq!(reader.open(&sealed).unwrap(), b"el cuerpo del codigo");
    }

    /// The whole point: the code is useless without the password.
    #[test]
    fn the_wrong_password_cannot_read_it() {
        let lock = FolderKey::create("correcta").unwrap();
        let sealed = lock.seal(b"secreto").unwrap();
        let salt = FolderKey::salt_of(&sealed).unwrap();
        let wrong = FolderKey::from_password("incorrecta", salt).unwrap();
        assert!(wrong.open(&sealed).is_err());
    }

    #[test]
    fn the_same_password_and_salt_always_give_the_same_key() {
        let first = FolderKey::create("repetible").unwrap();
        let again = FolderKey::from_password("repetible", first.salt).unwrap();
        assert!(first.matches(&again));
    }

    #[test]
    fn two_folders_with_the_same_password_do_not_share_a_key() {
        let one = FolderKey::create("misma").unwrap();
        let other = FolderKey::create("misma").unwrap();
        assert_ne!(one.salt, other.salt, "each folder gets its own salt");
        assert!(!one.matches(&other));
    }

    #[test]
    fn sealing_twice_never_repeats_the_bytes() {
        let lock = FolderKey::create("clave").unwrap();
        // A repeated nonce with the same key is what breaks this construction
        // outright, so it must be fresh every time.
        assert_ne!(lock.seal(b"igual").unwrap(), lock.seal(b"igual").unwrap());
    }

    #[test]
    fn a_tampered_code_is_refused_rather_than_half_read() {
        let lock = FolderKey::create("clave").unwrap();
        let mut sealed = lock.seal(b"contenido").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xff;
        let salt = FolderKey::salt_of(&sealed).unwrap();
        let reader = FolderKey::from_password("clave", salt).unwrap();
        assert!(reader.open(&sealed).is_err());
    }

    #[test]
    fn something_far_too_short_to_be_a_sealed_code_is_refused() {
        assert!(FolderKey::salt_of(b"corto").is_err());
    }
}
