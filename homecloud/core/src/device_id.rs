//! Parsing and formatting of Syncthing device IDs.
//!
//! A device ID is the SHA-256 of the device certificate, base32-encoded and
//! sprinkled with Luhn check characters so that a human who mistypes one gets
//! told so instead of silently pairing with nothing. We need both directions:
//! the check digits let us reject a bad paste immediately, and the raw 32
//! bytes let a pairing code stay short enough to read out loud.

use crate::error::{Error, Result};

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// The 32 raw bytes behind a device ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceId(pub [u8; 32]);

impl DeviceId {
    /// Accepts a device ID in any of the forms a user might paste: with or
    /// without dashes, with or without the check characters, any case.
    pub fn parse(input: &str) -> Result<Self> {
        let cleaned: String = input
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_uppercase())
            // Syncthing's own normalisation: these are the characters people
            // reliably confuse when copying by hand.
            .map(|c| match c {
                '0' => 'O',
                '1' => 'I',
                '8' => 'B',
                other => other,
            })
            .collect();

        let base32 = match cleaned.len() {
            56 => strip_check_chars(&cleaned)?,
            52 => cleaned,
            n => return Err(Error::BadDeviceId(format!("expected 52 or 56 characters, got {n}"))),
        };

        let bytes = base32_decode(&base32)?;
        Ok(DeviceId(bytes))
    }

    /// The canonical form Syncthing's own API and UI use.
    pub fn to_canonical(self) -> String {
        let base32 = base32_encode(&self.0);
        let with_checks = add_check_chars(&base32);
        with_checks
            .as_bytes()
            .chunks(7)
            .map(|c| std::str::from_utf8(c).expect("base32 is ascii"))
            .collect::<Vec<_>>()
            .join("-")
    }
}

/// Syncthing's Luhn mod-32 check character over its base32 alphabet.
fn luhn32(chunk: &str) -> Result<char> {
    let mut factor = 1usize;
    let mut sum = 0usize;
    for c in chunk.chars() {
        let codepoint = ALPHABET
            .iter()
            .position(|&a| a == c as u8)
            .ok_or_else(|| Error::BadDeviceId(format!("invalid character {c:?}")))?;
        let addend = factor * codepoint;
        factor = 3 - factor;
        sum += (addend / 32) + (addend % 32);
    }
    Ok(ALPHABET[(32 - sum % 32) % 32] as char)
}

/// Verifies and removes the check character that follows every 13 data characters.
fn strip_check_chars(s: &str) -> Result<String> {
    let mut out = String::with_capacity(52);
    for (i, chunk) in s.as_bytes().chunks(14).enumerate() {
        let chunk = std::str::from_utf8(chunk).map_err(|_| Error::BadDeviceId("not ascii".into()))?;
        let (data, check) = chunk.split_at(13);
        let expected = luhn32(data)?;
        if check.chars().next() != Some(expected) {
            return Err(Error::BadDeviceId(format!(
                "check character {} of the ID is wrong — the code was probably mistyped",
                i + 1
            )));
        }
        out.push_str(data);
    }
    Ok(out)
}

fn add_check_chars(s: &str) -> String {
    let mut out = String::with_capacity(56);
    for chunk in s.as_bytes().chunks(13) {
        let chunk = std::str::from_utf8(chunk).expect("base32 is ascii");
        out.push_str(chunk);
        out.push(luhn32(chunk).expect("base32 is in the alphabet"));
    }
    out
}

fn base32_encode(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(52);
    let mut buffer = 0u16;
    let mut bits = 0u32;
    for &b in bytes {
        buffer = (buffer << 8) | b as u16;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

fn base32_decode(s: &str) -> Result<[u8; 32]> {
    let mut out = [0u8; 32];
    let mut written = 0usize;
    let mut buffer = 0u16;
    let mut bits = 0u32;
    for c in s.chars() {
        let value = ALPHABET
            .iter()
            .position(|&a| a == c as u8)
            .ok_or_else(|| Error::BadDeviceId(format!("invalid character {c:?}")))?;
        buffer = (buffer << 5) | value as u16;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            if written == 32 {
                return Err(Error::BadDeviceId("device ID is too long".into()));
            }
            out[written] = ((buffer >> bits) & 0xff) as u8;
            written += 1;
        }
    }
    if written != 32 {
        return Err(Error::BadDeviceId(format!("device ID decoded to {written} bytes, expected 32")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Both taken from real `syncthing generate` runs, so a regression here
    // means we can no longer talk to a real Syncthing.
    const REAL_A: &str = "LJKPHDM-VNQWCDM-KNGS4YA-ABV5JUV-SZOIQQN-NNVHFJT-NL2OHCV-RZUJJQX";
    const REAL_B: &str = "Q4XJBIZ-VEJ72XY-ZGE2RIJ-V5DH2OA-FYWSOXW-BHI5NGG-BNNW2BY-RJCXYQP";

    #[test]
    fn round_trips_real_device_ids() {
        for id in [REAL_A, REAL_B] {
            let parsed = DeviceId::parse(id).expect("real ID should parse");
            assert_eq!(parsed.to_canonical(), id);
        }
    }

    #[test]
    fn accepts_a_paste_without_dashes_or_case() {
        let sloppy = REAL_A.replace('-', "").to_lowercase();
        assert_eq!(DeviceId::parse(&sloppy).unwrap().to_canonical(), REAL_A);
    }

    #[test]
    fn rejects_a_mistyped_character() {
        // Swap one data character; the Luhn check must catch it.
        let mut chars: Vec<char> = REAL_A.chars().collect();
        chars[1] = if chars[1] == 'J' { 'K' } else { 'J' };
        let typo: String = chars.into_iter().collect();
        assert!(DeviceId::parse(&typo).is_err(), "a single-character typo must be rejected");
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(DeviceId::parse("TOOSHORT").is_err());
    }
}
