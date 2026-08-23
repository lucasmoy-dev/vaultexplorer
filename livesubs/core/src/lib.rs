//! The part of LiveSubs that has nothing to do with any one platform:
//! turning a stream of audio samples into recognised text.
//!
//! Both apps sit on top of this crate -- the desktop one (Tauri, `parec`,
//! an X11 overlay) and the Android one (a foreground service, `AudioRecord`,
//! a `TYPE_APPLICATION_OVERLAY` view, through the JNI shim in
//! `android/rust`). Speech detection thresholds, chunking, whisper
//! parameters and the rules for what counts as a caption at all live here
//! exactly once, so a subtitle looks the same on a phone as on a laptop
//! and a fix to either lands in both.
//!
//! What is *not* here: audio capture, windows, settings storage,
//! translation. Those differ so completely between the two platforms that
//! sharing them would mean an abstraction per platform and no real reuse.

pub mod stt;
pub mod vad;

/// Everything in this crate assumes 16kHz mono `f32` samples -- what
/// whisper wants, and what both capture backends are configured to
/// produce so nothing has to resample.
pub const SAMPLE_RATE: u32 = 16_000;
