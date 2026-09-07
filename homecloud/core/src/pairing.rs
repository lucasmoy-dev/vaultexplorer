//! The one thing a user copies to connect two devices.
//!
//! The whole pairing UX rests on this being a single short string that also
//! fits comfortably in a QR code. So the device ID travels as its 32 raw bytes
//! rather than the 63-character display form, and the rest is packed with
//! postcard and base64url'd.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};

use crate::device_id::DeviceId;
use crate::error::{Error, Result};
use crate::lock::FolderKey;

/// Bumped if the payload layout ever changes, so an old app tells the user to
/// update instead of decoding garbage into a wrong device ID.
const PREFIX: &str = "HC2";

/// A code for a folder with a password. The body is encrypted, so nothing about
/// the folder — not even which device wrote it — is readable without it.
const PREFIX_LOCKED: &str = "HC2L";

/// The layout this replaced. Still read, so a device that has not been updated
/// can still pair with one that has — the only thing lost is the size warning.
const PREFIX_V1: &str = "HC1";

#[derive(Debug, Serialize, Deserialize)]
struct Payload {
    device: [u8; 32],
    device_name: String,
    folder_id: String,
    folder_label: String,
    /// Direct addresses to try before falling back to discovery. Empty means
    /// "just use discovery", which is the normal case on a home network.
    hints: Vec<String>,
}

/// Adds the folder's size, so the far device can say "this will not fit" while
/// the person is still deciding, rather than after a download has stalled.
#[derive(Debug, Serialize, Deserialize)]
struct PayloadV2 {
    device: [u8; 32],
    device_name: String,
    folder_id: String,
    folder_label: String,
    hints: Vec<String>,
    bytes: u64,
}

/// An invitation to share one folder, in the form a person copies or scans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    pub device_id: String,
    pub device_name: String,
    pub folder_id: String,
    pub folder_label: String,
    pub hints: Vec<String>,
    /// How much the folder holds on the device that wrote the code. `None`
    /// from an older device, which simply means the size is unknown.
    pub bytes: Option<u64>,
}

/// What came out of reading a code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScannedCode {
    /// Readable as it stands.
    Open(PairingCode),
    /// For a folder with a password. Carries the salt needed to turn a password
    /// into the key that opens it, and nothing else: the label, the device and
    /// the folder are all inside the encrypted part.
    Locked { salt: [u8; 16], sealed: Vec<u8> },
}

impl PairingCode {
    pub fn encode(&self) -> Result<String> {
        let payload = PayloadV2 {
            device: DeviceId::parse(&self.device_id)?.0,
            device_name: self.device_name.clone(),
            folder_id: self.folder_id.clone(),
            folder_label: self.folder_label.clone(),
            hints: self.hints.clone(),
            bytes: self.bytes.unwrap_or(0),
        };
        let bytes = postcard::to_allocvec(&payload)
            .map_err(|e| Error::BadPairingCode(format!("could not be packed: {e}")))?;
        Ok(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes)))
    }

    /// The same code, encrypted with a folder's password.
    ///
    /// Everything goes inside: without the password there is no device id to
    /// connect to and no folder id to ask for, so a code that leaks is a code
    /// that cannot be used.
    pub fn encode_locked(&self, lock: &FolderKey) -> Result<String> {
        let payload = PayloadV2 {
            device: DeviceId::parse(&self.device_id)?.0,
            device_name: self.device_name.clone(),
            folder_id: self.folder_id.clone(),
            folder_label: self.folder_label.clone(),
            hints: self.hints.clone(),
            bytes: self.bytes.unwrap_or(0),
        };
        let bytes = postcard::to_allocvec(&payload)
            .map_err(|e| Error::BadPairingCode(format!("could not be packed: {e}")))?;
        let sealed = lock.seal(&bytes)?;
        Ok(format!("{PREFIX_LOCKED}{}", URL_SAFE_NO_PAD.encode(sealed)))
    }

    /// Reads a code, saying whether a password is still needed.
    pub fn scan(input: &str) -> Result<ScannedCode> {
        let compact = compact(input);
        if let Some(body) = compact.strip_prefix(PREFIX_LOCKED) {
            let sealed = URL_SAFE_NO_PAD
                .decode(body)
                .map_err(|_| Error::BadPairingCode("it looks truncated or altered".into()))?;
            let salt = FolderKey::salt_of(&sealed)?;
            return Ok(ScannedCode::Locked { salt, sealed });
        }
        Ok(ScannedCode::Open(Self::decode(input)?))
    }

    /// Opens a locked code with the key a password produced.
    pub fn unlock(sealed: &[u8], lock: &FolderKey) -> Result<Self> {
        let bytes = lock.open(sealed)?;
        let payload: PayloadV2 = postcard::from_bytes(&bytes)
            .map_err(|_| Error::BadPairingCode("it was not produced by this version".into()))?;
        Ok(PairingCode {
            device_id: DeviceId(payload.device).to_canonical(),
            device_name: payload.device_name,
            folder_id: payload.folder_id,
            folder_label: payload.folder_label,
            hints: payload.hints,
            bytes: Some(payload.bytes),
        })
    }

    /// Tolerates the whitespace and stray newlines that survive a copy-paste,
    /// and a full `homecloud:` link as well as a bare code.
    pub fn decode(input: &str) -> Result<Self> {
        let compact = compact(input);
        if compact.starts_with(PREFIX_LOCKED) {
            return Err(Error::BadPairingCode(
                "esta carpeta tiene contraseña: hace falta para leer el código".into(),
            ));
        }

        let older = compact.starts_with(PREFIX_V1);
        let body = compact
            .strip_prefix(PREFIX)
            .or_else(|| compact.strip_prefix(PREFIX_V1))
            .ok_or_else(|| {
                Error::BadPairingCode("it does not start with HC — is it the whole code?".into())
            })?;

        let raw = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| Error::BadPairingCode("it looks truncated or altered".into()))?;

        if older {
            let payload: Payload = postcard::from_bytes(&raw)
                .map_err(|_| Error::BadPairingCode("it was not produced by this version".into()))?;
            return Ok(PairingCode {
                device_id: DeviceId(payload.device).to_canonical(),
                device_name: payload.device_name,
                folder_id: payload.folder_id,
                folder_label: payload.folder_label,
                hints: payload.hints,
                bytes: None,
            });
        }

        let payload: PayloadV2 = postcard::from_bytes(&raw)
            .map_err(|_| Error::BadPairingCode("it was not produced by this version".into()))?;
        Ok(PairingCode {
            device_id: DeviceId(payload.device).to_canonical(),
            device_name: payload.device_name,
            folder_id: payload.folder_id,
            folder_label: payload.folder_label,
            hints: payload.hints,
            bytes: Some(payload.bytes),
        })
    }
}

/// Tolerates the whitespace and stray newlines that survive a copy-paste, and
/// a full `homecloud:` link as well as a bare code.
fn compact(input: &str) -> String {
    let trimmed = input.trim();
    let trimmed = trimmed
        .strip_prefix("homecloud://")
        .or_else(|| trimmed.strip_prefix("homecloud:"))
        .unwrap_or(trimmed);
    trimmed.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PairingCode {
        PairingCode {
            device_id: "LJKPHDM-VNQWCDM-KNGS4YA-ABV5JUV-SZOIQQN-NNVHFJT-NL2OHCV-RZUJJQX".into(),
            device_name: "Portátil de Lucas".into(),
            folder_id: "fotos-a1b2c3".into(),
            folder_label: "Fotos".into(),
            hints: vec![],
            bytes: Some(11_500_000_000),
        }
    }

    #[test]
    fn round_trips() {
        let code = sample().encode().unwrap();
        assert_eq!(PairingCode::decode(&code).unwrap(), sample());
    }

    #[test]
    fn stays_short_enough_to_paste_and_to_scan() {
        let code = sample().encode().unwrap();
        // Well inside the ~300 characters a phone camera reads reliably from a
        // QR on another screen, and short enough to survive a chat message.
        assert!(code.len() < 120, "pairing code grew to {} characters: {code}", code.len());
    }

    #[test]
    fn survives_a_messy_paste() {
        let code = sample().encode().unwrap();
        let messy = format!("  {}\n", code);
        assert_eq!(PairingCode::decode(&messy).unwrap(), sample());
        let linked = format!("homecloud://{}", code);
        assert_eq!(PairingCode::decode(&linked).unwrap(), sample());
    }

    /// A device that has not been updated still pairs; it just cannot warn
    /// about the size.
    #[test]
    fn a_code_from_the_previous_version_still_reads() {
        // Built the way the old version built it: the v1 payload under HC1.
        #[derive(serde::Serialize)]
        struct OldPayload {
            device: [u8; 32],
            device_name: String,
            folder_id: String,
            folder_label: String,
            hints: Vec<String>,
        }
        let old = OldPayload {
            device: DeviceId::parse(&sample().device_id).unwrap().0,
            device_name: sample().device_name,
            folder_id: sample().folder_id,
            folder_label: sample().folder_label,
            hints: vec![],
        };
        let encoded = format!(
            "HC1{}",
            URL_SAFE_NO_PAD.encode(postcard::to_allocvec(&old).unwrap())
        );
        let read = PairingCode::decode(&encoded).unwrap();
        assert_eq!(read.folder_label, sample().folder_label);
        assert_eq!(read.bytes, None, "an old code carries no size, and must not invent one");
    }

    #[test]
    fn a_locked_code_needs_the_password_and_then_reads_the_same() {
        let lock = FolderKey::create("las fotos de la abuela").unwrap();
        let code = sample().encode_locked(&lock).unwrap();

        // Nothing about the folder is readable from the code alone.
        assert!(!code.contains("Fotos"));
        assert!(PairingCode::decode(&code).is_err());

        match PairingCode::scan(&code).unwrap() {
            ScannedCode::Locked { salt, sealed } => {
                let key = FolderKey::from_password("las fotos de la abuela", salt).unwrap();
                assert_eq!(PairingCode::unlock(&sealed, &key).unwrap(), sample());
            }
            ScannedCode::Open(_) => panic!("a locked code must not read as an open one"),
        }
    }

    #[test]
    fn a_locked_code_refuses_the_wrong_password() {
        let lock = FolderKey::create("correcta").unwrap();
        let code = sample().encode_locked(&lock).unwrap();
        let ScannedCode::Locked { salt, sealed } = PairingCode::scan(&code).unwrap() else {
            panic!("expected a locked code");
        };
        let wrong = FolderKey::from_password("incorrecta", salt).unwrap();
        assert!(PairingCode::unlock(&sealed, &wrong).is_err());
    }

    #[test]
    fn a_folder_without_a_password_still_reads_with_no_extra_step() {
        let code = sample().encode().unwrap();
        match PairingCode::scan(&code).unwrap() {
            ScannedCode::Open(read) => assert_eq!(read, sample()),
            ScannedCode::Locked { .. } => panic!("an open code must not ask for a password"),
        }
    }

    /// It still has to survive a QR read off another screen.
    #[test]
    fn a_locked_code_stays_short_enough_to_scan() {
        let lock = FolderKey::create("clave").unwrap();
        let code = sample().encode_locked(&lock).unwrap();
        assert!(code.len() < 300, "locked code grew to {} characters", code.len());
    }

    #[test]
    fn rejects_something_that_is_not_a_code() {
        assert!(PairingCode::decode("hola que tal").is_err());
    }

    #[test]
    fn rejects_a_truncated_code() {
        let code = sample().encode().unwrap();
        let cut = &code[..code.len() - 8];
        assert!(PairingCode::decode(cut).is_err(), "a cut-off code must not decode");
    }
}
