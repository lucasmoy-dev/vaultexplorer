use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("that does not look like a device ID: {0}")]
    BadDeviceId(String),

    #[error("that pairing code is not valid: {0}")]
    BadPairingCode(String),

    #[error("could not reach the sync engine: {0}")]
    Http(#[from] reqwest::Error),

    #[error("the sync engine rejected the request ({status}): {body}")]
    Api { status: u16, body: String },

    #[error("the sync engine would not start: {0}")]
    Engine(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("could not read the engine's answer: {0}")]
    Json(#[from] serde_json::Error),
}
