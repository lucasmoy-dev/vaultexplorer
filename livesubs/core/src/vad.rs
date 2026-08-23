//! Cutting a continuous stream into utterances.
//!
//! whisper.cpp wants a chunk of speech, not a firehose: feeding it fixed
//! 5-second windows would cut words in half at every boundary and waste
//! most of its work on silence. So this is a small energy-based voice
//! activity detector -- speech starts when the signal rises clearly above
//! the room's own noise floor and ends after a pause -- which is enough
//! for the job here and costs a rolling average per 30ms frame.
//!
//! Two details matter more than the threshold itself:
//!
//! * **Pre-roll.** The first syllable is usually what pushes the level
//!   over the threshold, so by the time speech is detected it is already
//!   partly in the past. A short ring of previous frames is prepended to
//!   every utterance, which is the difference between "hola, qué tal" and
//!   "ola, qué tal".
//! * **An adaptive floor.** A fixed threshold is wrong in both directions
//!   -- deaf in a quiet room, permanently triggered next to a fan. The
//!   floor tracks the quiet parts of the stream and the threshold rides on
//!   top of it.

/// 30ms at 16kHz. Small enough to react, big enough that RMS means
/// something.
pub const FRAME: usize = 480;

const PREROLL_FRAMES: usize = 10; // 300ms
const MIN_SPEECH_FRAMES: usize = 8; // 240ms -- shorter than this is a click, not a word
const SILENCE_FRAMES: usize = 23; // ~700ms of quiet ends an utterance
/// Hard cap, so someone talking without pause still gets subtitles
/// instead of one enormous chunk at the end.
const MAX_SPEECH_FRAMES: usize = 266; // ~8s
/// Absolute floor: below this the "speech" is inaudible anyway, whatever
/// the adaptive threshold says (it protects against a *perfectly* silent
/// stream making the floor 0 and every rounding error look like talking).
const ABSOLUTE_MIN_RMS: f32 = 0.004;

pub struct Vad {
    preroll: Vec<Vec<f32>>,
    speech: Vec<f32>,
    speech_frames: usize,
    silence_run: usize,
    noise_floor: f32,
    /// User-facing sensitivity: >1 needs louder speech, <1 is twitchier.
    threshold_scale: f32,
}

impl Vad {
    pub fn new(sensitivity: f32) -> Vad {
        Vad {
            preroll: Vec::with_capacity(PREROLL_FRAMES),
            speech: Vec::new(),
            speech_frames: 0,
            silence_run: 0,
            noise_floor: 0.0,
            threshold_scale: sensitivity.clamp(0.2, 5.0),
        }
    }

    fn rms(frame: &[f32]) -> f32 {
        if frame.is_empty() {
            return 0.0;
        }
        let sum: f32 = frame.iter().map(|s| s * s).sum();
        (sum / frame.len() as f32).sqrt()
    }

    /// Feed one frame. Returns a complete utterance when one just ended.
    pub fn push(&mut self, frame: &[f32]) -> Option<Vec<f32>> {
        let level = Self::rms(frame);
        let threshold = (self.noise_floor * 3.5).max(ABSOLUTE_MIN_RMS) * self.threshold_scale;
        let is_speech = level > threshold;

        if !is_speech {
            // Track the floor only while quiet, and asymmetrically: rise
            // slowly (a fan starting up is real), fall quickly (once the
            // room goes quiet the old floor is stale and would make the
            // detector deaf).
            self.noise_floor = if level > self.noise_floor {
                self.noise_floor * 0.995 + level * 0.005
            } else {
                self.noise_floor * 0.9 + level * 0.1
            };
        }

        if !self.in_speech() {
            if is_speech {
                // Start of an utterance: take the pre-roll with it.
                for past in self.preroll.drain(..) {
                    self.speech.extend_from_slice(&past);
                }
                self.speech.extend_from_slice(frame);
                self.speech_frames = 1;
                self.silence_run = 0;
                return None;
            }
            self.preroll.push(frame.to_vec());
            if self.preroll.len() > PREROLL_FRAMES {
                self.preroll.remove(0);
            }
            return None;
        }

        // Mid-utterance: silence is kept (a pause inside a sentence is
        // part of it) until it's long enough to mean the end.
        self.speech.extend_from_slice(frame);
        self.speech_frames += 1;
        if is_speech {
            self.silence_run = 0;
        } else {
            self.silence_run += 1;
        }

        let ended = self.silence_run >= SILENCE_FRAMES;
        let too_long = self.speech_frames >= MAX_SPEECH_FRAMES;
        if !ended && !too_long {
            return None;
        }
        let utterance = std::mem::take(&mut self.speech);
        let frames = self.speech_frames;
        self.speech_frames = 0;
        self.silence_run = 0;
        self.preroll.clear();
        // Discard blips: a door closing shouldn't cost a whisper pass.
        if frames < MIN_SPEECH_FRAMES + SILENCE_FRAMES && !too_long {
            return None;
        }
        Some(utterance)
    }

    fn in_speech(&self) -> bool {
        self.speech_frames > 0
    }

    /// Flush whatever is buffered -- used when capture stops, so the last
    /// thing said still gets transcribed.
    pub fn flush(&mut self) -> Option<Vec<f32>> {
        if self.speech_frames < MIN_SPEECH_FRAMES {
            return None;
        }
        self.speech_frames = 0;
        self.silence_run = 0;
        Some(std::mem::take(&mut self.speech))
    }

    pub fn set_sensitivity(&mut self, sensitivity: f32) {
        self.threshold_scale = sensitivity.clamp(0.2, 5.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize, amplitude: f32) -> Vec<Vec<f32>> {
        (0..frames)
            .map(|f| {
                (0..FRAME)
                    .map(|i| {
                        let t = (f * FRAME + i) as f32 / crate::SAMPLE_RATE as f32;
                        (t * 220.0 * std::f32::consts::TAU).sin() * amplitude
                    })
                    .collect()
            })
            .collect()
    }

    fn quiet(frames: usize) -> Vec<Vec<f32>> {
        tone(frames, 0.0005)
    }

    #[test]
    fn silence_alone_never_produces_an_utterance() {
        let mut vad = Vad::new(1.0);
        for frame in quiet(200) {
            assert!(vad.push(&frame).is_none());
        }
    }

    #[test]
    fn speech_then_pause_yields_one_utterance_with_preroll() {
        let mut vad = Vad::new(1.0);
        for frame in quiet(30) {
            vad.push(&frame);
        }
        let mut got = None;
        for frame in tone(40, 0.2) {
            if let Some(u) = vad.push(&frame) {
                got = Some(u);
            }
        }
        assert!(got.is_none(), "an utterance must not end while speech continues");
        for frame in quiet(SILENCE_FRAMES + 2) {
            if let Some(u) = vad.push(&frame) {
                got = Some(u);
            }
        }
        let utterance = got.expect("expected an utterance once the pause was long enough");
        // 40 speech frames + up to 300ms of pre-roll + the trailing pause.
        assert!(
            utterance.len() > 40 * FRAME,
            "expected pre-roll to be included, got {} samples",
            utterance.len()
        );
    }

    #[test]
    fn continuous_speech_is_cut_at_the_cap() {
        let mut vad = Vad::new(1.0);
        let mut chunks = 0;
        for frame in tone(MAX_SPEECH_FRAMES * 2 + 10, 0.25) {
            if vad.push(&frame).is_some() {
                chunks += 1;
            }
        }
        assert!(chunks >= 2, "expected the 8s cap to split it, got {chunks} chunk(s)");
    }

    #[test]
    fn a_single_click_is_ignored() {
        let mut vad = Vad::new(1.0);
        for frame in quiet(20) {
            vad.push(&frame);
        }
        let mut got = None;
        for frame in tone(2, 0.3) {
            if let Some(u) = vad.push(&frame) {
                got = Some(u);
            }
        }
        for frame in quiet(SILENCE_FRAMES + 5) {
            if let Some(u) = vad.push(&frame) {
                got = Some(u);
            }
        }
        assert!(got.is_none(), "60ms of noise is not an utterance");
    }

    #[test]
    fn a_loud_room_does_not_trigger_forever() {
        // Steady mid-level noise: the floor should climb to meet it, so
        // after the ramp-up nothing counts as speech any more.
        let mut vad = Vad::new(1.0);
        for frame in tone(400, 0.02) {
            vad.push(&frame);
        }
        let mut triggered = false;
        for frame in tone(60, 0.02) {
            if vad.push(&frame).is_some() {
                triggered = true;
            }
        }
        assert!(!triggered, "constant background noise must not read as speech");
    }
}
