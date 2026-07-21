use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("(de)serialization error: {0}")]
    Serde(#[from] bincode::Error),

    #[error("invalid password")]
    InvalidPassword,

    #[error("key derivation failed: {0}")]
    Kdf(String),

    #[error("encryption/decryption failed (bad key, tampered data, or wrong nonce)")]
    Crypto,

    #[error("corrupt or unrecognized file header")]
    BadHeader,

    #[error("no matching key-unlock method found for this file")]
    NoMatchingKey,

    #[error("vault already exists at this path")]
    VaultExists,

    #[error("vault metadata not found at this path")]
    VaultNotFound,

    #[error("no such file or directory in the vault")]
    PathNotFound,

    #[error("archive error: {0}")]
    Zip(#[from] zip::result::ZipError),
}

pub type Result<T> = std::result::Result<T, VaultError>;
