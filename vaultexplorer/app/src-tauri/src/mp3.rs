//! In-process audio → MP3 transcoding, so a phone can download a real
//! `.mp3` from a YouTube result.
//!
//! Android can download the audio track on its own already (see
//! `ytstreams.rs`), but what YouTube serves is AAC in an MP4 container --
//! an `.m4a`. Turning that into an `.mp3` on the desktop is a one-line
//! `ffmpeg`/`yt-dlp` call; on Android there is no ffmpeg to shell out to,
//! and the platform's own `MediaCodec` has an AAC *decoder* but no MP3
//! *encoder* (Android has never shipped one), so `MediaMuxer` -- which is
//! what the video download uses to join tracks -- can't help either.
//!
//! So the whole pipeline lives here: `symphonia` (pure Rust) demuxes the
//! MP4 and decodes AAC to PCM, and `mp3lame-encoder` (LAME, built from
//! its vendored C source by `cc`, which cross-compiles to Android with
//! the NDK like any other C dependency) encodes that PCM to MP3. Nothing
//! platform-specific, so it works the same on desktop -- which also gives
//! the desktop a fallback path when `yt-dlp`/ffmpeg aren't installed.
//!
//! Licensing note: LAME is LGPL-2.1, and this links it statically.

use crate::errmap::ToStringErr;
use crate::progress::ProgressReporter;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Bitrate for the encode. 192 kbps is a deliberate compromise: YouTube's
/// own AAC audio is typically ~128 kbps, so a *lower* MP3 bitrate would
/// audibly lose more on top of an already-lossy source, and a higher one
/// would only make the file bigger without recovering anything.
const BITRATE: mp3lame_encoder::Bitrate = mp3lame_encoder::Bitrate::Kbps192;

/// Decode `src` (anything symphonia can read -- m4a/aac, wav, flac, ogg…)
/// and write an MP3 to `dest`. `title`, when given, is written as the
/// MP3's title tag so music players show the video's name rather than a
/// filename.
pub fn transcode_to_mp3(
    src: &Path,
    dest: &Path,
    title: Option<&str>,
    reporter: &ProgressReporter,
) -> Result<(), String> {
    transcode(src, dest, title, &mut |frames| {
        if reporter.is_cancelled() {
            return false;
        }
        reporter.report(frames);
        true
    })
    .map(|()| reporter.finish())
}

/// The transcode itself, with progress as a plain callback (returning
/// false to cancel) rather than a `ProgressReporter` -- that keeps this
/// half testable without a live Tauri IPC channel.
pub fn transcode(
    src: &Path,
    dest: &Path,
    title: Option<&str>,
    on_progress: &mut dyn FnMut(u64) -> bool,
) -> Result<(), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error as SymError;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(src).str_err()?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = src.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("can't read this audio file: {e}"))?;
    let mut format = probed.format;
    // Cloned out of the track before `format` is used mutably below --
    // the track itself borrows from it.
    let (track_id, codec_params) = {
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or("no audio track in this file")?;
        (track.id, track.codec_params.clone())
    };
    let total_frames = codec_params.n_frames.unwrap_or(0);
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| format!("unsupported audio codec: {e}"))?;

    let mut out = std::io::BufWriter::new(std::fs::File::create(dest).str_err()?);
    // Both are created once the first decoded packet reveals the real
    // sample rate and channel count -- the container's declared values
    // can be absent, and LAME has to be configured before the first
    // sample goes in.
    let mut encoder: Option<mp3lame_encoder::Encoder> = None;
    let mut sample_buf: Option<SampleBuffer<i16>> = None;
    let mut mp3_buf: Vec<u8> = Vec::new();
    let mut channels = 0usize;
    let mut frames_done: u64 = 0;

    loop {
        if !on_progress(frames_done) {
            drop(out);
            let _ = std::fs::remove_file(dest);
            return Err("cancelled".to_string());
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            // End of stream -- symphonia signals it as a plain EOF read.
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(format!("audio read failed: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A damaged packet mid-stream is worth skipping rather than
            // throwing away the whole download.
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(format!("audio decode failed: {e}")),
        };
        let spec = *decoded.spec();
        if encoder.is_none() {
            channels = spec.channels.count();
            if channels == 0 || channels > 2 {
                return Err(format!("{channels}-channel audio isn't supported for MP3 here"));
            }
            let mut builder = mp3lame_encoder::Builder::new().ok_or("couldn't start the MP3 encoder")?;
            builder.set_num_channels(channels as u8).map_err(|e| format!("MP3 encoder: {e}"))?;
            builder.set_sample_rate(spec.rate).map_err(|e| format!("MP3 encoder: {e}"))?;
            builder.set_brate(BITRATE).map_err(|e| format!("MP3 encoder: {e}"))?;
            // Not `Best`: on a phone the highest setting is several times
            // slower for a difference nobody hears on a 128 kbps source.
            builder
                .set_quality(mp3lame_encoder::Quality::Good)
                .map_err(|e| format!("MP3 encoder: {e}"))?;
            encoder = Some(builder.build().map_err(|e| format!("MP3 encoder: {e}"))?);
        }
        let enc = encoder.as_mut().expect("built above");
        let capacity = decoded.capacity() as u64;
        let buf = sample_buf.get_or_insert_with(|| SampleBuffer::<i16>::new(capacity, spec));
        buf.copy_interleaved_ref(decoded);
        let pcm = buf.samples();
        if pcm.is_empty() {
            continue;
        }
        mp3_buf.clear();
        mp3_buf.reserve(mp3lame_encoder::max_required_buffer_size(pcm.len() / channels));
        if channels == 1 {
            enc.encode_to_vec(mp3lame_encoder::MonoPcm(pcm), &mut mp3_buf)
        } else {
            enc.encode_to_vec(mp3lame_encoder::InterleavedPcm(pcm), &mut mp3_buf)
        }
        .map_err(|e| format!("MP3 encode failed: {e}"))?;
        out.write_all(&mp3_buf).str_err()?;
        frames_done += (pcm.len() / channels) as u64;
        if total_frames > 0 && !on_progress(frames_done.min(total_frames)) {
            drop(out);
            let _ = std::fs::remove_file(dest);
            return Err("cancelled".to_string());
        }
    }

    if let Some(mut enc) = encoder {
        mp3_buf.clear();
        mp3_buf.reserve(mp3lame_encoder::max_required_buffer_size(0) + 7200);
        enc.flush_to_vec::<mp3lame_encoder::FlushNoGap>(&mut mp3_buf)
            .map_err(|e| format!("MP3 flush failed: {e}"))?;
        out.write_all(&mp3_buf).str_err()?;
    } else {
        return Err("no audio found in this file".to_string());
    }
    out.flush().str_err()?;
    drop(out);

    if let Some(title) = title {
        // Best-effort: a file that plays but shows its filename as the
        // title is a far better outcome than failing the whole download
        // because a tag couldn't be written.
        let _ = write_title_tag(dest, title);
    }
    Ok(())
}

fn write_title_tag(path: &Path, title: &str) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::prelude::{Accessor, TagExt};
    let mut tag = lofty::tag::Tag::new(lofty::tag::TagType::Id3v2);
    tag.set_title(title.to_string());
    tag.save_to_path(path, WriteOptions::default()).str_err()
}

/// Convert `src` to an MP3 at `dest`. `title` (optional) becomes the
/// file's title tag. Deletes `src` afterwards when `remove_source` is set
/// -- the download flow does, since the intermediate `.m4a` is an
/// implementation detail the user never asked for.
/// `dest`, or the next free "name (2).ext" beside it. Downloading the same
/// song twice shouldn't silently overwrite the copy already there (the
/// download step itself refuses to, so the conversion step shouldn't
/// quietly differ).
fn unique_dest(dest: &Path) -> PathBuf {
    if !dest.exists() {
        return dest.to_path_buf();
    }
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("audio");
    let ext = dest.extension().and_then(|s| s.to_str()).unwrap_or("mp3");
    for n in 2..1000 {
        let candidate = parent.join(format!("{stem} ({n}).{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dest.to_path_buf()
}

#[tauri::command]
pub async fn audio_to_mp3(
    src: String,
    dest: String,
    title: Option<String>,
    remove_source: bool,
    channel: tauri::ipc::Channel<crate::progress::ProgressEvent>,
    registry: tauri::State<'_, crate::ops::OpRegistry>,
) -> Result<String, String> {
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let src_path = std::path::PathBuf::from(&src);
        let dest_path = unique_dest(Path::new(&dest));
        // Total is in decoded frames (samples per channel); the reporter
        // is created with the *file's* frame count once known, so this
        // starts as a plain "running" row and gains a percentage from the
        // first progress report.
        let frames = probe_frame_count(&src_path);
        let reporter = ProgressReporter::new_cancellable(channel, frames.max(1), cancel);
        let out = transcode_to_mp3(&src_path, &dest_path, title.as_deref(), &reporter);
        if out.is_ok() && remove_source {
            let _ = std::fs::remove_file(&src_path);
        }
        out.map(|()| dest_path.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    result
}

/// The decoded length of `path` in frames, for the progress total. Zero
/// when the container doesn't declare it (the row then just spins).
fn probe_frame_count(path: &Path) -> u64 {
    use lofty::file::AudioFile;
    let Ok(tagged) = lofty::probe::Probe::open(path).and_then(|p| p.read()) else {
        return 0;
    };
    let props = tagged.properties();
    let secs = props.duration().as_secs_f64();
    let rate = props.sample_rate().unwrap_or(0) as f64;
    if secs <= 0.0 || rate <= 0.0 {
        return 0;
    }
    (secs * rate) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trips a real AAC-in-MP4 file (built with ffmpeg, which is
    /// only needed to *make* the fixture -- the code under test never
    /// shells out) and checks the result is a plausible MP3.
    #[test]
    fn transcodes_m4a_to_mp3() {
        let dir = std::env::temp_dir().join("vaultexplorer-mp3-test");
        let _ = std::fs::create_dir_all(&dir);
        let m4a = dir.join("tone.m4a");
        let mp3 = dir.join("tone.mp3");
        let _ = std::fs::remove_file(&mp3);
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=3", "-c:a", "aac", "-ar", "44100", "-ac", "2"])
            .arg(&m4a)
            .output();
        match made {
            Ok(o) if o.status.success() => {}
            // No ffmpeg on this machine: nothing to build the input from.
            _ => return,
        }
        let mut frames = 0u64;
        transcode(&m4a, &mp3, Some("Tone"), &mut |done| {
            frames = done;
            true
        })
        .expect("transcode failed");
        let bytes = std::fs::read(&mp3).expect("no output file");
        assert!(bytes.len() > 10_000, "suspiciously small mp3: {} bytes", bytes.len());
        assert!(frames > 100_000, "expected ~3s of frames, got {frames}");
        // An ID3v2 tag (from write_title_tag) or a raw MPEG frame sync --
        // either means something decodable came out.
        assert!(&bytes[..3] == b"ID3" || (bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0));
        let read_back = lofty::probe::Probe::open(&mp3).and_then(|p| p.read()).expect("unreadable mp3");
        use lofty::file::AudioFile;
        assert!(read_back.properties().duration().as_secs() >= 2);
    }
}
