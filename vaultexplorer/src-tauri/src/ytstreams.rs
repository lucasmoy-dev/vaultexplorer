//! Resolving a YouTube video to downloadable stream URLs, without yt-dlp.
//!
//! `yt-dlp` is Python, so it cannot run on Android at all -- which is why
//! "Download MP4/MP3" was desktop-only. This resolves the same thing in
//! process with rustypipe (the extractor NewPipe's approach is built on),
//! so the phone doesn't need a PC.
//!
//! Two things learned by measuring rather than assuming, both of which
//! shape this:
//!
//! 1. YouTube no longer serves *progressive* streams (video and audio in
//!    one file) to any client -- every client tried returns zero of them.
//!    So a video download is two files that have to be muxed together,
//!    while an audio download is a single ready-to-play `.m4a`.
//! 2. Of rustypipe's clients, only the iOS one currently resolves without
//!    the signature deobfuscation that the Android and TV clients fail on
//!    ("could not extract sig fn name"). It is therefore asked for by
//!    name instead of letting the default client order retry through the
//!    broken ones on every call.

use crate::errmap::ToStringErr;
use rustypipe::client::{ClientType, RustyPipe};
use serde::Serialize;

/// What the frontend needs to download a video: a title to name the file
/// after, and the best stream of each kind. `video_url` is video-only (see
/// the note above), so saving a playable mp4 means muxing it with
/// `audio_url` -- which on Android is the platform's own MediaMuxer.
#[derive(Serialize)]
pub struct YoutubeStreams {
    pub title: String,
    pub video_url: Option<String>,
    pub video_height: u32,
    pub audio_url: Option<String>,
    /// Container of the audio stream ("m4a"/"webm"), so the caller names
    /// the file honestly instead of calling everything an mp3.
    pub audio_ext: String,
}

/// `page_url` accepts either a full watch URL or a bare video id, since
/// the frontend has both shapes depending on where the result came from.
fn video_id_of(page_url: &str) -> Option<String> {
    if !page_url.contains('/') && page_url.len() <= 24 {
        return Some(page_url.to_string());
    }
    // Parsed by hand rather than pulling in a URL crate for two lookups:
    // a watch link carries the id in `v=`, and youtu.be/<id> and
    // /shorts/<id> carry it as the last path segment.
    let (path, query) = match page_url.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (page_url, None),
    };
    if let Some(q) = query {
        for pair in q.split('&') {
            if let Some(v) = pair.strip_prefix("v=") {
                return Some(v.to_string());
            }
        }
    }
    let last = path.trim_end_matches('/').rsplit('/').next()?;
    (!last.is_empty() && last.len() <= 24).then(|| last.to_string())
}

#[tauri::command]
pub async fn youtube_streams(page_url: String) -> Result<YoutubeStreams, String> {
    let id = video_id_of(&page_url).ok_or("Not a YouTube video URL")?;
    let player = RustyPipe::new()
        .query()
        .player_from_client(&id, ClientType::Ios)
        .await
        .str_err()?;

    let video = player.video_only_streams.iter().max_by_key(|s| s.height);
    let audio = player.audio_streams.iter().max_by_key(|s| s.bitrate);
    Ok(YoutubeStreams {
        title: player.details.name.clone().unwrap_or_else(|| id.clone()),
        video_url: video.map(|s| s.url.clone()),
        video_height: video.map(|s| s.height).unwrap_or(0),
        audio_url: audio.map(|s| s.url.clone()),
        audio_ext: audio
            .map(|s| format!("{:?}", s.format).to_lowercase())
            .unwrap_or_else(|| "m4a".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_video_id_out_of_every_url_shape() {
        assert_eq!(video_id_of("dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(
            video_id_of("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=5").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(video_id_of("https://youtu.be/dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(
            video_id_of("https://www.youtube.com/shorts/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
    }
}
