//! Hybrid classical + post-quantum key encapsulation, used to wrap a
//! per-file key (FEK) for a *recipient* (sharing scenario), as opposed to
//! wrapping under a password the local vault owner knows.
//!
//! Why hybrid: ML-KEM (FIPS 203 / Kyber) is the NIST-standardized PQ KEM,
//! but it is comparatively young. X25519 is decades-old and well-studied.
//! Combining both means an attacker must break *both* to recover the
//! wrapping key -- if ML-KEM is ever broken, X25519 still holds (against
//! classical attackers), and vice versa against a future quantum one. This
//! is the same construction used in hybrid TLS 1.3 key exchange.

use crate::crypto::{Key32, KEY_LEN};
use crate::error::{Result, VaultError};
use hkdf::Hkdf;
use ml_kem::kem::{Decapsulate, Encapsulate};
use ml_kem::{EncodedSizeUser, KemCore, MlKem768};
use rand_core::OsRng;
use sha2::Sha256;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret as X25519Secret};
use zeroize::{Zeroize, Zeroizing};

type MlKem768DecapKey = <MlKem768 as KemCore>::DecapsulationKey;
type MlKem768EncapKey = <MlKem768 as KemCore>::EncapsulationKey;

/// A recipient's long-term key material. The public half is shared with
/// whoever wants to encrypt a file *for* this recipient; the secret half
/// never leaves the recipient's device.
pub struct RecipientSecretKey {
    x25519_secret: X25519Secret,
    kem_decap: MlKem768DecapKey,
}

#[derive(Clone)]
pub struct RecipientPublicKey {
    pub x25519_public: [u8; 32],
    pub kem_encap_bytes: Vec<u8>,
}

impl RecipientSecretKey {
    pub fn generate() -> (Self, RecipientPublicKey) {
        let x25519_secret = X25519Secret::random_from_rng(OsRng);
        let x25519_public = X25519Public::from(&x25519_secret);

        let (kem_decap, kem_encap) = MlKem768::generate(&mut OsRng);

        let public = RecipientPublicKey {
            x25519_public: *x25519_public.as_bytes(),
            kem_encap_bytes: kem_encap.as_bytes().to_vec(),
        };
        (
            Self {
                x25519_secret,
                kem_decap,
            },
            public,
        )
    }
}

/// Output of wrapping a FEK for a recipient: the two KEM ciphertexts they
/// need to recover the shared secret, plus the AES-GCM-wrapped key itself.
pub struct HybridWrap {
    pub x25519_ephemeral_public: [u8; 32],
    pub kem_ciphertext: Vec<u8>,
    pub nonce: [u8; 12],
    pub wrapped_fek: Vec<u8>,
}

/// Combine an X25519 shared secret and an ML-KEM shared secret into a single
/// wrapping key via HKDF-SHA256. Domain-separated so this key can never
/// collide with a key derived elsewhere in the system.
fn combine_secrets(x25519_ss: &[u8; 32], kem_ss: &[u8]) -> Key32 {
    let mut ikm = [x25519_ss.as_slice(), kem_ss].concat();
    let hk = Hkdf::<Sha256>::new(None, &ikm);
    let mut out = [0u8; KEY_LEN];
    hk.expand(b"vaultcore-hybrid-pq-wrap-v1", &mut out)
        .expect("HKDF expand of 32 bytes never fails");
    ikm.zeroize();
    Key32(out)
}

/// Wrap a FEK so that only the holder of `recipient`'s secret key can
/// recover it -- secure against a future large-scale quantum adversary
/// recording today's ciphertext ("harvest now, decrypt later").
pub fn wrap_for_recipient(recipient: &RecipientPublicKey, fek: &Key32) -> Result<HybridWrap> {
    let eph_secret = X25519Secret::random_from_rng(OsRng);
    let eph_public = X25519Public::from(&eph_secret);
    let recipient_x25519_public = X25519Public::from(recipient.x25519_public);
    let mut x25519_ss = eph_secret.diffie_hellman(&recipient_x25519_public).to_bytes();

    let recipient_encap = MlKem768EncapKey::from_bytes(
        recipient
            .kem_encap_bytes
            .as_slice()
            .try_into()
            .map_err(|_| VaultError::BadHeader)?,
    );
    let (kem_ciphertext, mut kem_ss) = recipient_encap
        .encapsulate(&mut OsRng)
        .map_err(|_| VaultError::Crypto)?;

    let wrap_key = combine_secrets(&x25519_ss, kem_ss.as_slice());
    x25519_ss.zeroize();
    kem_ss.as_mut_slice().zeroize();

    let nonce = crate::crypto::random_nonce();
    let wrapped_fek = crate::crypto::aead_encrypt_with_nonce(
        &wrap_key,
        &nonce,
        b"vaultcore-fek-wrap",
        &fek.0,
    )?;

    Ok(HybridWrap {
        x25519_ephemeral_public: *eph_public.as_bytes(),
        kem_ciphertext: kem_ciphertext.to_vec(),
        nonce,
        wrapped_fek,
    })
}

/// Recover a FEK previously wrapped with [`wrap_for_recipient`].
pub fn unwrap_as_recipient(secret: &RecipientSecretKey, wrap: &HybridWrap) -> Result<Key32> {
    let eph_public = X25519Public::from(wrap.x25519_ephemeral_public);
    let mut x25519_ss = secret.x25519_secret.diffie_hellman(&eph_public).to_bytes();

    let kem_ct = wrap
        .kem_ciphertext
        .as_slice()
        .try_into()
        .map_err(|_| VaultError::BadHeader)?;
    let mut kem_ss = secret
        .kem_decap
        .decapsulate(kem_ct)
        .map_err(|_| VaultError::Crypto)?;

    let wrap_key = combine_secrets(&x25519_ss, kem_ss.as_slice());
    x25519_ss.zeroize();
    kem_ss.as_mut_slice().zeroize();

    let fek_bytes = Zeroizing::new(crate::crypto::aead_decrypt(
        &wrap_key,
        &wrap.nonce,
        b"vaultcore-fek-wrap",
        &wrap.wrapped_fek,
    )?);
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&fek_bytes);
    Ok(Key32(key))
}
