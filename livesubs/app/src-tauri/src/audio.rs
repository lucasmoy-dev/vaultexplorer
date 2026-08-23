//! Capturing both sides of the machine's audio.
//!
//! Two streams, from the same server: the microphone (`@DEFAULT_SOURCE@`)
//! and everything being played (`@DEFAULT_MONITOR@`, the monitor of the
//! default sink -- which is what makes "the other people in the call"
//! audible to this app at all).
//!
//! It shells out to `parec` rather than binding libpulse or going through
//! `cpal`. That is a deliberate trade: `parec` is part of
//! `pulseaudio-utils`, present on every Ubuntu with PipeWire's Pulse
//! shim, it accepts exactly the format whisper wants (`--format=float32le
//! --rate=16000 --channels=1`, so no resampling or channel mixing here),
//! and -- the part that matters most -- `@DEFAULT_SOURCE@` /
//! `@DEFAULT_MONITOR@` are resolved by the *server* on connect. Plugging
//! in headphones mid-call changes the default sink, and a stream that
//! named a device by index would keep recording silence from the old one.

use std::io::Read;
use std::process::{Child, Command, Stdio};

/// Which of the two halves a caption came from -- and, downstream, which
/// colour it gets drawn in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Mic,
    System,
}

impl Source {
    /// PulseAudio's own "whatever is default right now" names.
    pub fn device(self) -> &'static str {
        match self {
            Source::Mic => "@DEFAULT_SOURCE@",
            Source::System => "@DEFAULT_MONITOR@",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Source::Mic => "mic",
            Source::System => "system",
        }
    }
}

/// 16kHz mono, because that is what whisper wants -- see
/// `livesubs_core::SAMPLE_RATE`, which both apps' capture paths agree on.
pub use livesubs_core::SAMPLE_RATE;

/// A running `parec`, killed on drop -- the pipeline stops by dropping
/// this, and a leaked recorder would keep a capture stream (and its little
/// red recording indicator) alive for the rest of the session.
pub struct Recorder {
    child: Child,
}

impl Recorder {
    pub fn start(source: Source) -> Result<Recorder, String> {
        let child = Command::new("parec")
            .args([
                "--device",
                source.device(),
                "--format=float32le",
                &format!("--rate={SAMPLE_RATE}"),
                "--channels=1",
                // Small enough that a caption isn't waiting on the buffer,
                // large enough not to wake this thread constantly.
                "--latency-msec=100",
                "--client-name=LiveSubs",
                // Without this, PipeWire's Pulse shim can hand back a
                // stream that is corked (silent) when the sink is idle.
                "--stream-name=LiveSubs capture",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!(
                    "couldn't start parec ({e}). Install it with: sudo apt install pulseaudio-utils"
                )
            })?;
        Ok(Recorder { child })
    }

    /// Fill `out` with exactly `out.len()` samples, or return false when
    /// the stream ended (parec exited -- server restart, device gone).
    pub fn read_exact_samples(&mut self, out: &mut [f32]) -> bool {
        let Some(stdout) = self.child.stdout.as_mut() else {
            return false;
        };
        // f32le on the wire; read into bytes and reinterpret, rather than
        // one read per sample.
        let mut bytes = vec![0u8; out.len() * 4];
        if stdout.read_exact(&mut bytes).is_err() {
            return false;
        }
        for (i, chunk) in bytes.chunks_exact(4).enumerate() {
            out[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        true
    }

    /// The recorder's pid, so the pipeline can SIGTERM it to unblock a
    /// read that would otherwise sit waiting for audio that never comes
    /// (a monitor source on a silent machine).
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Whatever parec complained about on the way out, for the error the
    /// UI actually shows.
    pub fn stderr_tail(&mut self) -> String {
        let Some(stderr) = self.child.stderr.as_mut() else {
            return String::new();
        };
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text.trim().to_string()
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Whether the audio server can be reached at all, for the settings
/// window's status line. Cheap enough to call on every settings open.
pub fn audio_server_available() -> bool {
    Command::new("pactl")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn parec_available() -> bool {
    Command::new("parec")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
