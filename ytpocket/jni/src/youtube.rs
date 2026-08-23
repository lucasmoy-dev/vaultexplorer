//! Searching YouTube and resolving a video to downloadable streams.
//!
//! Uses `rustypipe` -- the same extractor (and version) the sibling
//! vaultexplorer app already ships on Android, so this is a known-good
//! combination rather than a fresh bet on someone's parser.
//!
//! Two facts about YouTube shape everything below, both established by
//! measurement in that app rather than assumed:
//!
//! 1. **There are no progressive streams any more.** No client returns a
//!    file with video and audio together, so "download the MP4" means
//!    fetching two files and muxing them (on Android, with the platform's
//!    own `MediaMuxer`). Audio alone is a single ready-to-play `.m4a`.
//! 2. **Only the iOS client resolves reliably.** The Android and TV
//!    clients fail signature deobfuscation ("could not extract sig fn
//!    name"), so that client is asked for by name instead of letting the
//!    default order retry through the broken ones on every call.

use rustypipe::client::{ClientType, RustyPipe};
use rustypipe::model::{AudioCodec, VideoCodec, VideoItem};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;

/// Where rustypipe may keep its cache (YouTube's client versions and the
/// deobfuscation code it extracts).
///
/// This has to be set, and it has to be set by the caller: rustypipe
/// defaults to the *current working directory*, which for an Android process
/// is `/` -- not writable. The cache then fails to save on every call, so
/// every search re-fetches and re-extracts YouTube's player JS, which is
/// both slow and the most fragile step in the whole chain. Kotlin passes the
/// app's cache dir in at startup (see `Native.init`).
static STORAGE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Point the cache at a writable directory. Called once at startup; safe to
/// call again (the last one wins).
pub fn set_storage_dir(path: &str) {
    if let Ok(mut guard) = STORAGE_DIR.lock() {
        *guard = Some(PathBuf::from(path));
    }
}

/// A client with the cache pointed somewhere writable.
///
/// Built per call rather than kept in a static: what makes repeat calls
/// cheap is the *file* cache, which this reads on construction, and a
/// long-lived client would have to be `Sync` for a gain the cache already
/// provides.
fn client() -> Result<RustyPipe, String> {
    let mut builder = RustyPipe::builder();
    if let Some(dir) = STORAGE_DIR.lock().ok().and_then(|guard| guard.clone()) {
        builder = builder.storage_dir(dir);
    }
    builder.build().map_err(|e| format!("no se pudo iniciar el cliente de YouTube: {e}"))
}

#[derive(Serialize)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    pub channel: String,
    /// Seconds; absent for a livestream.
    pub duration: Option<u32>,
    pub views: Option<u64>,
    pub published: Option<String>,
    pub thumbnail: String,
}

#[derive(Serialize)]
pub struct Stream {
    pub url: String,
    /// Container extension as YouTube actually serves it -- named honestly
    /// so nothing ends up called `.mp3` when it is AAC.
    pub ext: String,
    /// Codec name, for the caller's benefit: what Android's `MediaMuxer`
    /// and this crate's MP3 encoder can accept is a codec question, not a
    /// container one.
    pub codec: String,
    pub bitrate: u32,
    pub height: u32,
    /// Content length in bytes when YouTube declares it (0 when it does
    /// not), so the download can show a percentage instead of a spinner.
    pub size: u64,
}

#[derive(Serialize)]
pub struct Resolved {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub duration: Option<u32>,
    /// The audio track both downloads use: **AAC**, deliberately, even when
    /// YouTube offers a higher-bitrate Opus one.
    ///
    /// Two reasons, and they both matter more than the few kbps: the MP3
    /// encoder here decodes AAC (symphonia has no Opus decoder at all), and
    /// Android's `MediaMuxer` takes AAC into an MP4 without argument while
    /// Opus-in-MP4 depends on the version. Falls back to whatever audio
    /// exists if a video somehow has no AAC track.
    pub audio: Option<Stream>,
    /// Best video-only stream, preferring **AVC/H.264**: it is the codec
    /// every Android decoder and muxer handles, where VP9/AV1 support
    /// varies by device. That caps quality at 1080p on YouTube, which is
    /// the right trade for a file that has to actually play.
    pub video: Option<Stream>,
}

/// The container from a mime type (`video/mp4; codecs="avc1"` -> `mp4`).
fn container_of(mime: &str, fallback: &str) -> String {
    mime.split('/')
        .nth(1)
        .and_then(|rest| rest.split(';').next())
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

/// The largest thumbnail YouTube lists, or the well-known URL shape as a
/// fallback (it exists for every video, so a result never renders blank).
fn thumbnail_of(item: &VideoItem) -> String {
    item.thumbnail
        .iter()
        .max_by_key(|t| t.width)
        .map(|t| t.url.clone())
        .unwrap_or_else(|| format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", item.id))
}

pub fn search(query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let runtime = runtime()?;
    let pipe = client()?;
    let results = runtime
        .block_on(async { pipe.query().search::<VideoItem, _>(query).await })
        .map_err(|e| format!("la búsqueda falló: {e}"))?;

    Ok(results
        .items
        .items
        .into_iter()
        .take(limit)
        .map(|item| SearchHit {
            thumbnail: thumbnail_of(&item),
            id: item.id,
            title: item.name,
            channel: item.channel.map(|c| c.name).unwrap_or_default(),
            duration: item.duration,
            views: item.view_count,
            published: item.publish_date_txt,
        })
        .collect())
}

/// Accepts a bare video id or any watch/share URL, since a link pasted from
/// another app can be either.
pub fn video_id_of(text: &str) -> Option<String> {
    let text = text.trim();
    if !text.contains('/') && !text.contains('?') && (8..=24).contains(&text.len()) {
        return Some(text.to_string());
    }
    let (path, query) = match text.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (text, None),
    };
    if let Some(query) = query {
        for pair in query.split('&') {
            if let Some(value) = pair.strip_prefix("v=") {
                return Some(value.to_string());
            }
        }
    }
    // youtu.be/<id> and /shorts/<id> carry it as the last path segment.
    let last = path.trim_end_matches('/').rsplit('/').next()?;
    (!last.is_empty() && last.len() <= 24).then(|| last.to_string())
}

/// Clients to try, in order, until one returns usable streams.
///
/// Not a single hard-coded client: which of YouTube's clients resolve
/// changes over time (the sibling app pinned iOS because Android and TV
/// were failing signature deobfuscation at the time), and a download that
/// stops working is this app's entire failure mode. Trying a short list
/// costs one extra request in the bad case and keeps working across the
/// next change.
const CLIENTS: &[ClientType] = &[ClientType::Ios, ClientType::Tv, ClientType::Desktop, ClientType::Android];

pub fn resolve(video: &str) -> Result<Resolved, String> {
    let id = video_id_of(video).ok_or("eso no parece un vídeo de YouTube")?;
    let runtime = runtime()?;
    let pipe = client()?;
    let mut last_error = String::new();
    let mut player = None;
    for client in CLIENTS {
        match runtime.block_on(async {
            pipe.query().player_from_client(&id, *client).await
        }) {
            // A player with no audio at all is not usable for either
            // download, so keep trying rather than returning it.
            Ok(found) if !found.audio_streams.is_empty() => {
                player = Some(found);
                break;
            }
            Ok(found) => {
                last_error = if found.details.is_live {
                    "es un directo: YouTube no sirve archivos descargables para directos".to_string()
                } else {
                    format!("el cliente {client:?} no devolvió audio")
                };
                if found.details.is_live {
                    break;
                }
            }
            Err(error) => last_error = format!("{client:?}: {error}"),
        }
    }
    let Some(player) = player else {
        return Err(format!("no se pudieron obtener los streams ({last_error})"));
    };

    // Preferences first, "anything that exists" second -- see the field
    // comments on `Resolved` for why AAC and AVC specifically.
    let audio_stream = player
        .audio_streams
        .iter()
        .filter(|s| s.codec == AudioCodec::Mp4a)
        .max_by_key(|s| s.bitrate)
        .or_else(|| player.audio_streams.iter().max_by_key(|s| s.bitrate));
    let video_stream = player
        .video_only_streams
        .iter()
        .filter(|s| s.codec == VideoCodec::Avc1 && !s.hdr)
        .max_by_key(|s| (s.height, s.fps))
        .or_else(|| player.video_only_streams.iter().max_by_key(|s| s.height));

    Ok(Resolved {
        title: player.details.name.clone().unwrap_or_else(|| id.clone()),
        channel: player.details.channel_name.clone().unwrap_or_default(),
        duration: Some(player.details.duration).filter(|d| *d > 0),
        audio: audio_stream.map(|s| Stream {
            url: s.url.clone(),
            // "m4a" rather than the mime's "mp4": it is audio, and every
            // player and file manager reads that extension correctly.
            ext: if s.codec == AudioCodec::Mp4a { "m4a".to_string() } else { container_of(&s.mime, "webm") },
            codec: format!("{:?}", s.codec).to_lowercase(),
            bitrate: s.bitrate,
            height: 0,
            size: s.size,
        }),
        video: video_stream.map(|s| Stream {
            url: s.url.clone(),
            ext: container_of(&s.mime, "mp4"),
            codec: format!("{:?}", s.codec).to_lowercase(),
            bitrate: s.bitrate,
            height: s.height,
            size: s.size.unwrap_or(0),
        }),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_found_in_every_link_shape() {
        assert_eq!(video_id_of("dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(
            video_id_of("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(video_id_of("https://youtu.be/dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(
            video_id_of("https://www.youtube.com/shorts/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
    }

    /// Live: hits YouTube. `#[ignore]`d, run on purpose -- it is also the
    /// canary for "YouTube changed something and the extractor needs a
    /// bump", which is the failure mode this whole app has.
    #[test]
    #[ignore]
    fn search_and_resolve_against_youtube() {
        let hits = search("rick astley never gonna give you up", 5).expect("search failed");
        assert!(!hits.is_empty(), "no results");
        for hit in &hits {
            println!("{} | {} | {:?}s | {}", hit.id, hit.title, hit.duration, hit.channel);
            assert_eq!(hit.id.len(), 11, "odd video id: {}", hit.id);
            assert!(!hit.title.is_empty());
        }

        // A livestream has no downloadable file, so pick a real video --
        // which is also the difference the app itself has to respect.
        let target = hits.iter().find(|h| h.duration.is_some()).expect("no non-live result");
        let resolved = resolve(&target.id).expect("resolve failed");
        println!("resolved: {} ({:?}s)", resolved.title, resolved.duration);
        let audio = resolved.audio.expect("no audio stream");
        println!("audio: {} {} {}kbps {}B", audio.ext, audio.codec, audio.bitrate / 1000, audio.size);
        assert!(audio.url.starts_with("https://"));
        // The MP3 encoder decodes AAC; an Opus track here would mean silent
        // failures later, so the preference is asserted, not hoped for.
        assert_eq!(audio.codec, "mp4a", "expected an AAC track, got {}", audio.codec);
        assert_eq!(audio.ext, "m4a");

        let video = resolved.video.expect("no video stream");
        println!("video: {} {} {}p {}B", video.ext, video.codec, video.height, video.size);
        assert!(video.height >= 360, "suspiciously small video: {}p", video.height);
        // AVC, so Android's muxer and every device decoder can handle it.
        assert_eq!(video.codec, "avc1", "expected H.264, got {}", video.codec);
        assert_eq!(video.ext, "mp4");
    }
}
