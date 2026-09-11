//! Changing a vault's password.
//!
//! A Cryptomator vault's password never encrypts anything but the two
//! master keys: `masterkey.cryptomator` holds them wrapped (AES-KW) under
//! a key scrypt-derived from the password. So a password change is a
//! re-wrap of that one small file -- no file in the vault is touched, and
//! the `vault.cryptomator` JWT stays valid because it is signed with the
//! master keys themselves, which don't change.
//!
//! This is the one piece of the format the engine crate doesn't expose,
//! so it is implemented here against the same file it writes.

use crate::error::{Result, VaultError};
use aes_kw::Kek;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const KEY_LEN: usize = 32;
const SALT_LEN: usize = 8;
const SCRYPT_PARALLELISM: u32 = 1;

/// `masterkey.cryptomator` as Cryptomator writes it: base64 fields, camel
/// case, and a `version` of 999 for vault format 8.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MasterKeyFile {
    version: u32,
    scrypt_salt: String,
    scrypt_cost_param: u64,
    scrypt_block_size: u32,
    primary_master_key: String,
    hmac_master_key: String,
    version_mac: String,
}

fn b64(s: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(s)
        .map_err(|_| VaultError::Crypt("masterkey file is not valid base64".into()))
}

fn kek_from_password(password: &[u8], salt: &[u8], cost: u64, block_size: u32) -> Result<[u8; KEY_LEN]> {
    let log_n = cost
        .checked_ilog2()
        .ok_or_else(|| VaultError::Crypt("bad scrypt cost parameter".into()))? as u8;
    let params = scrypt::Params::new(log_n, block_size, SCRYPT_PARALLELISM, KEY_LEN)
        .map_err(|e| VaultError::Kdf(e.to_string()))?;
    let mut out = [0u8; KEY_LEN];
    scrypt::scrypt(password, salt, &params, &mut out).map_err(|e| VaultError::Kdf(e.to_string()))?;
    Ok(out)
}

/// Re-wrap the vault's master keys under `new_password`. Verifies
/// `old_password` first; a wrong one leaves the file untouched.
pub fn change_password(
    root: impl AsRef<Path>,
    old_password: &[u8],
    new_password: &[u8],
) -> Result<()> {
    let path = root.as_ref().join("masterkey.cryptomator");
    let file: MasterKeyFile = serde_json::from_slice(&fs::read(&path)?)
        .map_err(|e| VaultError::Crypt(format!("masterkey file is unreadable: {e}")))?;

    let old_salt = b64(&file.scrypt_salt)?;
    let old_kek = Kek::from(kek_from_password(
        old_password,
        &old_salt,
        file.scrypt_cost_param,
        file.scrypt_block_size,
    )?);
    let mut enc_key = [0u8; KEY_LEN];
    let mut mac_key = [0u8; KEY_LEN];
    old_kek
        .unwrap(&b64(&file.primary_master_key)?, &mut enc_key)
        .map_err(|_| VaultError::InvalidPassword)?;
    old_kek
        .unwrap(&b64(&file.hmac_master_key)?, &mut mac_key)
        .map_err(|_| VaultError::InvalidPassword)?;

    // A fresh salt with the new password: reusing the old one would let
    // anyone who kept a copy of the file tell "same password" apart from
    // "changed" without trying either.
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    let new_kek = Kek::from(kek_from_password(
        new_password,
        &salt,
        file.scrypt_cost_param,
        file.scrypt_block_size,
    )?);
    let mut wrapped_enc = [0u8; KEY_LEN + SALT_LEN];
    let mut wrapped_mac = [0u8; KEY_LEN + SALT_LEN];
    new_kek
        .wrap(&enc_key, &mut wrapped_enc)
        .map_err(|e| VaultError::Crypt(e.to_string()))?;
    new_kek
        .wrap(&mac_key, &mut wrapped_mac)
        .map_err(|e| VaultError::Crypt(e.to_string()))?;

    let updated = MasterKeyFile {
        version: file.version,
        scrypt_salt: STANDARD.encode(salt),
        scrypt_cost_param: file.scrypt_cost_param,
        scrypt_block_size: file.scrypt_block_size,
        primary_master_key: STANDARD.encode(wrapped_enc),
        hmac_master_key: STANDARD.encode(wrapped_mac),
        // Unchanged: it authenticates the format version under the MAC
        // key, and neither of those moved.
        version_mac: file.version_mac,
    };
    let bytes = serde_json::to_vec(&updated)
        .map_err(|e| VaultError::Crypt(format!("could not write masterkey file: {e}")))?;
    // Write the replacement beside the original and rename it into place,
    // so a crash mid-write can't leave a vault with half a masterkey --
    // which would mean every file in it is unreadable, forever.
    let tmp = path.with_extension("cryptomator.new");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, &path)?;
    let _ = fs::write(root.as_ref().join("masterkey.cryptomator.bak"), &bytes);
    Ok(())
}
