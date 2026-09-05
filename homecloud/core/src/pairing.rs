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

/// Bumped if the payload layout ever changes, so an old app tells the user to
/// update instead of decoding garbage into a wrong device ID.
const PREFIX: &str = "HC1";

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

/// An invitation to share one folder, in the form a person copies or scans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    pub device_id: String,
    pub device_name: String,
    pub folder_id: String,
    pub folder_label: String,
    pub hints: Vec<String>,
}

impl PairingCode {
    pub fn encode(&self) -> Result<String> {
        let payload = Payload {
            device: DeviceId::parse(&self.device_id)?.0,
            device_name: self.device_name.clone(),
            folder_id: self.folder_id.clone(),
            folder_label: self.folder_label.clone(),
            hints: self.hints.clone(),
        };
        let bytes = postcard::to_allocvec(&payload)
            .map_err(|e| Error::BadPairingCode(format!("could not be packed: {e}")))?;
        Ok(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes)))
    }

    /// Tolerates the whitespace and stray newlines that survive a copy-paste,
    /// and a full `homecloud:` link as well as a bare code.
    pub fn decode(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        let trimmed = trimmed
            .strip_prefix("homecloud://")
            .or_else(|| trimmed.strip_prefix("homecloud:"))
            .unwrap_or(trimmed);
        let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();

        let body = compact.strip_prefix(PREFIX).ok_or_else(|| {
            Error::BadPairingCode("it does not start with HC1 — is it the whole code?".into())
        })?;

        let bytes = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| Error::BadPairingCode("it looks truncated or altered".into()))?;
        let payload: Payload = postcard::from_bytes(&bytes)
            .map_err(|_| Error::BadPairingCode("it was not produced by this version".into()))?;

        Ok(PairingCode {
            device_id: DeviceId(payload.device).to_canonical(),
            device_name: payload.device_name,
            folder_id: payload.folder_id,
            folder_label: payload.folder_label,
            hints: payload.hints,
        })
    }
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
