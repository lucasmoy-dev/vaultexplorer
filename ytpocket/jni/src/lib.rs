//! The native half of YT Pocket: find videos, resolve their streams, and
//! turn the downloaded audio into an MP3.
//!
//! Kotlin owns the UI, the HTTP downloads, muxing (Android's `MediaMuxer`)
//! and where files land (`MediaStore`). Everything here is the part the
//! platform cannot do: talking to YouTube like a client, and encoding MP3 --
//! Android ships an AAC decoder but has never shipped an MP3 encoder, and
//! there is no ffmpeg to shell out to.
pub mod bridge;
pub mod mp3;
pub mod naming;
pub mod youtube;
