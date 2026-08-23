//! AAC in, MP3 out -- in process, because the platform cannot do it.
//!
//! What YouTube serves as "audio" is AAC in an MP4 container (an `.m4a`).
//! Android has an AAC *decoder* but has never shipped an MP3 *encoder*, and
//! there is no ffmpeg to shell out to, so `MediaCodec` cannot get from one
//! to the other. `symphonia` (pure Rust) demuxes and decodes, and
//! `mp3lame-encoder` (LAME, built from vendored C by `cc`, which
//! cross-compiles with the NDK like any other C dependency) encodes.
//!
//! Lifted from the sibling vaultexplorer app, which does exactly this for
//! its own mobile downloads -- same crates, same bitrate reasoning, with
//! the Tauri plumbing replaced by a plain progress callback and artist
//! tagging added (a music file wants both tags, not just a title).
//!
//! Licensing note: LAME is LGPL-2.1, and this links it statically.

use std::io::Write;
use std::path::Path;

/// Bitrate for the encode. 192 kbps is a deliberate compromise: YouTube's
/// own AAC audio is typically ~128 kbps, so a *lower* MP3 bitrate would
/// audibly lose more on top of an already-lossy source, and a higher one
/// would only make the file bigger without recovering anything.
const BITRATE: mp3lame_encoder::Bitrate = mp3lame_encoder::Bitrate::Kbps192;

/// Decode `src` (anything symphonia can read -- m4a/aac, wav, flac, ogg…)
/// and write an MP3 to `dest`. `title`, when given, is written as the
/// MP3's title tag so music players show the video's name rather than a
/// filename.
/// Decode `src` and write an MP3 to `dest`, tagging it with the video's
/// title and channel. `on_progress` receives decoded frames and returns
/// false to cancel.
pub fn transcode(
    src: &Path,
    dest: &Path,
    title: Option<&str>,
    artist: Option<&str>,
    on_progress: &mut dyn FnMut(u64) -> bool,
) -> Result<(), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error as SymError;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(src).map_err(|e| e.to_string())?;
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

    let mut out = std::io::BufWriter::new(std::fs::File::create(dest).map_err(|e| e.to_string())?);
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
        out.write_all(&mp3_buf).map_err(|e| e.to_string())?;
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
        out.write_all(&mp3_buf).map_err(|e| e.to_string())?;
    } else {
        return Err("no audio found in this file".to_string());
    }
    out.flush().map_err(|e| e.to_string())?;
    drop(out);

    if title.is_some() || artist.is_some() {
        // Best-effort: a file that plays but shows its filename as the
        // title is a far better outcome than failing the whole download
        // because a tag couldn't be written.
        let _ = write_tags(dest, title.unwrap_or_default(), artist.unwrap_or_default());
    }
    Ok(())
}


/// Convert `src` to an MP3 at `dest`. `title` (optional) becomes the
/// file's title tag. Deletes `src` afterwards when `remove_source` is set
/// -- the download flow does, since the intermediate `.m4a` is an
/// implementation detail the user never asked for.

/// Write the title and artist tags, so a music player shows the video's
/// name and channel instead of a filename.
fn write_tags(path: &Path, title: &str, artist: &str) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::prelude::{Accessor, TagExt};
    let mut tag = lofty::tag::Tag::new(lofty::tag::TagType::Id3v2);
    tag.set_title(title.to_string());
    if !artist.is_empty() {
        tag.set_artist(artist.to_string());
    }
    tag.save_to_path(path, WriteOptions::default()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real pipeline: build an AAC file with ffmpeg (only to *make* the
    /// fixture -- the code under test shells out to nothing), transcode it,
    /// and check the result is a tagged, decodable MP3.
    #[test]
    fn transcodes_aac_to_tagged_mp3() {
        let dir = std::env::temp_dir().join("ytpocket-mp3-test");
        let _ = std::fs::create_dir_all(&dir);
        let m4a = dir.join("tone.m4a");
        let mp3 = dir.join("tone.mp3");
        let _ = std::fs::remove_file(&mp3);
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=3", "-c:a", "aac", "-ar", "44100", "-ac", "2"])
            .arg(&m4a)
            .output();
        match made {
            Ok(out) if out.status.success() => {}
            // No ffmpeg here: nothing to build the input from.
            _ => return,
        }

        let mut frames = 0u64;
        transcode(&m4a, &mp3, Some("Tone Video"), Some("Some Channel"), &mut |done| {
            frames = done;
            true
        })
        .expect("transcode failed");

        let bytes = std::fs::read(&mp3).expect("no output file");
        assert!(bytes.len() > 10_000, "suspiciously small mp3: {} bytes", bytes.len());
        assert!(frames > 100_000, "expected ~3s of frames, got {frames}");
        assert!(&bytes[..3] == b"ID3", "expected an ID3 tag at the start");

        // And the tags a music player will read.
        use lofty::file::TaggedFileExt;
        use lofty::prelude::Accessor;
        let tagged = lofty::probe::Probe::open(&mp3).and_then(|p| p.read()).expect("unreadable mp3");
        let tag = tagged.primary_tag().or_else(|| tagged.first_tag()).expect("no tag");
        assert_eq!(tag.title().as_deref(), Some("Tone Video"));
        assert_eq!(tag.artist().as_deref(), Some("Some Channel"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod live_tests {
    use crate::{mp3, naming, youtube};

    /// The whole promise of the app, end to end, against real YouTube:
    /// resolve a video, download its audio, transcode it, and check the file
    /// is a tagged MP3 *named after the video*.
    ///
    /// `#[ignore]`d (network, a few MB). Run it on purpose:
    ///
    /// ```text
    /// cargo test --release -- --ignored --nocapture download_and_transcode
    /// ```
    #[test]
    #[ignore]
    fn download_and_transcode_a_real_video() {
        // A short, stable, well-known video: enough audio to be a real
        // transcode, small enough not to abuse anyone's bandwidth.
        let resolved = youtube::resolve("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
            .expect("resolve failed");
        let audio = resolved.audio.expect("no audio stream");
        println!("{} — {} ({} {}kbps)", resolved.title, resolved.channel, audio.ext, audio.bitrate / 1000);

        let dir = std::env::temp_dir().join("ytpocket-e2e");
        let _ = std::fs::create_dir_all(&dir);
        let source = dir.join(format!("{}.{}", resolved.id, audio.ext));
        // Ranged chunks, not one open stream: googlevideo trickles a plain
        // sequential read to a non-browser client (measured -- a 3.4MB file
        // timed out at 30s), and the Kotlin downloader fetches the same way
        // for the same reason.
        const CHUNK: u64 = 4 * 1024 * 1024;
        let client = reqwest::blocking::Client::builder()
            .user_agent("Mozilla/5.0 (Linux; Android 13) YTPocket/1.0")
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap();
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            let start = bytes.len() as u64;
            let response = client
                .get(&audio.url)
                .header("Range", format!("bytes={}-{}", start, start + CHUNK - 1))
                .send()
                .expect("download failed")
                .error_for_status()
                .expect("download rejected");
            let chunk = response.bytes().expect("download body");
            let read = chunk.len();
            bytes.extend_from_slice(&chunk);
            if read < CHUNK as usize {
                break;
            }
        }
        std::fs::write(&source, &bytes).unwrap();
        println!("downloaded {} bytes", bytes.len());
        assert!(bytes.len() > 500_000, "suspiciously small audio download");

        // The filename is the point: the video's title, safe for storage.
        let file_name = naming::file_name(&resolved.title, "mp3");
        println!("file name: {file_name}");
        assert!(file_name.ends_with(".mp3"));
        assert!(file_name.contains("Never Gonna Give You Up"), "{file_name}");
        assert!(!file_name.contains('/'), "{file_name}");

        let destination = dir.join(&file_name);
        mp3::transcode(
            &source,
            &destination,
            Some(&resolved.title),
            Some(&resolved.channel),
            &mut |_| true,
        )
        .expect("transcode failed");

        let produced = std::fs::read(&destination).expect("no mp3 written");
        println!("mp3: {} bytes", produced.len());
        assert!(produced.len() > 1_000_000, "mp3 far too small: {}", produced.len());
        assert_eq!(&produced[..3], b"ID3", "no ID3 tag");

        // And it reads back as a real, tagged, roughly-right-length MP3.
        use lofty::file::{AudioFile, TaggedFileExt};
        use lofty::prelude::Accessor;
        let tagged = lofty::probe::Probe::open(&destination).and_then(|p| p.read()).expect("unreadable mp3");
        let seconds = tagged.properties().duration().as_secs();
        println!("duration: {seconds}s (video says {:?}s)", resolved.duration);
        assert!(seconds > 60, "only {seconds}s of audio");
        let tag = tagged.primary_tag().or_else(|| tagged.first_tag()).expect("no tag");
        assert_eq!(tag.title().as_deref(), Some(resolved.title.as_str()));
        assert_eq!(tag.artist().as_deref(), Some(resolved.channel.as_str()));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
