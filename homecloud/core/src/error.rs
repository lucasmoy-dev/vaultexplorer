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

    /// Bare passthrough, no prefix: a public link has nothing to do with the
    /// sync engine, and every message here is already a full sentence written
    /// for a person. Wrapping it in "the sync engine would not start: " — as
    /// reusing `Engine` for this would have done — describes a failure that
    /// never happened.
    #[error("{0}")]
    Link(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("could not read the engine's answer: {0}")]
    Json(#[from] serde_json::Error),
}
