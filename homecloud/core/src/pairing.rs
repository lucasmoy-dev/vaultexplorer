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
const PREFIX: &str = "HC2";

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

    /// Tolerates the whitespace and stray newlines that survive a copy-paste,
    /// and a full `homecloud:` link as well as a bare code.
    pub fn decode(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        let trimmed = trimmed
            .strip_prefix("homecloud://")
            .or_else(|| trimmed.strip_prefix("homecloud:"))
            .unwrap_or(trimmed);
        let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();

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
