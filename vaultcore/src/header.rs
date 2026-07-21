use serde::{Deserialize, Serialize};

/// One way to recover this file's FEK. A file can carry several at once
/// (vault master key + an extra per-file password + one or more PQ
/// recipients), so unlocking it doesn't require the vault's own key.
#[derive(Serialize, Deserialize, Clone)]
pub enum WrappedKey {
    /// FEK wrapped (AES-256-GCM) under the vault's master key.
    MasterKey {
        nonce: [u8; 12],
        wrapped_fek: Vec<u8>,
    },
    /// FEK wrapped under a key derived (Argon2id) from a standalone
    /// per-file password, independent of the vault password.
    Password {
        salt: [u8; 16],
        nonce: [u8; 12],
        wrapped_fek: Vec<u8>,
    },
    /// FEK wrapped for a specific recipient via hybrid X25519 + ML-KEM768,
    /// so it survives a future large-scale quantum adversary.
    Recipient {
        x25519_ephemeral_public: [u8; 32],
        kem_ciphertext: Vec<u8>,
        nonce: [u8; 12],
        wrapped_fek: Vec<u8>,
    },
}
