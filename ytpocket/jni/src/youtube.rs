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
    /// Which YouTube client produced these URLs, and the User-Agent that
    /// goes with it.
    ///
    /// This is not diagnostics: a googlevideo URL is minted for a specific
    /// client, and a request that does not look like that client can be
    /// answered with **403**. The download therefore has to go out with this
    /// exact agent -- which is also why downloading is done by the native
    /// side now, through the same HTTP stack that resolved the URL, rather
    /// than by a second client in Kotlin that differs in its headers, its
    /// TLS and even which IP family it connects over.
    pub client: String,
    pub user_agent: String,
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

/// The clients that need no PO token, spoken to directly (see
/// `innertube.rs`). Tried first, because these are the ones that keep
/// working on a network where YouTube enforces the token -- which is exactly
/// where 0.1.1 and 0.1.2 failed with 403.
const DIRECT_CLIENTS: &[crate::innertube::Client] = &[crate::innertube::VISIONOS];

/// rustypipe's clients, as a fallback for when the direct ones fail.
///
/// Every one of these is on yt-dlp's "PO token required for streams" list, so
/// they may resolve and then be refused -- which is why nothing here is
/// returned without probing it first.
///
/// **Desktop, DesktopMusic and Mobile are deliberately absent.** Since
/// August 2024 YouTube requires a PO token for streams from its web-based
/// clients, and without one googlevideo answers the download with **403** --
/// while the resolve step still succeeds, so nothing looks wrong until the
/// transfer fails. Generating PO tokens needs a simulated browser (rustypipe
/// farms it out to a separate CLI binary), which is not something a phone
/// app can carry. Those clients therefore never mint a download URL here.
///
/// That leaves the three that need no token: iOS (which also needs no
/// signature deobfuscation, making it the most reliable), TV and Android.
const CLIENTS: &[ClientType] = &[ClientType::Ios, ClientType::Tv, ClientType::Android];

/// How many times to walk the client list.
///
/// Two, because a failure is often transient (a timeout, a single bad
/// response) and iOS -- the best of them -- deserves a second chance before
/// the app tells the user it cannot download at all.
const CLIENT_ATTEMPTS: usize = 2;

pub fn resolve(video: &str) -> Result<Resolved, String> {
    let id = video_id_of(video).ok_or("eso no parece un vídeo de YouTube")?;
    let mut refusals = Vec::new();

    // The token-free clients first. If one of these works, YouTube's PO
    // token enforcement cannot touch this download at all.
    for client in DIRECT_CLIENTS {
        match resolve_direct(*client, &id) {
            Ok(resolved) => return Ok(resolved),
            Err(error) => refusals.push(error),
        }
    }

    let runtime = runtime()?;
    let pipe = client()?;
    let mut last_error = String::new();
    let mut found_player = None;
    let mut refused = false;
    for client in CLIENTS.iter().cycle().take(CLIENTS.len() * CLIENT_ATTEMPTS) {
        let player = match runtime.block_on(async {
            pipe.query().player_from_client(&id, *client).await
        }) {
            Ok(player) => player,
            Err(error) => {
                last_error = format!("{client:?}: {error}");
                continue;
            }
        };
        if player.details.is_live {
            return Err(
                "es un directo: YouTube no sirve archivos descargables para directos".to_string()
            );
        }
        if player.audio_streams.is_empty() {
            last_error = format!("el cliente {client:?} no devolvió audio");
            continue;
        }
        // Resolving is not the same as being able to download. YouTube will
        // hand back perfectly-formed URLs that googlevideo then answers with
        // 403 -- which is exactly what shipped in 0.1.1 and failed on the
        // phone. So each candidate is probed with a 1KB range request, using
        // this client's own agent, and a client whose URLs do not actually
        // serve bytes is skipped rather than returned to the user.
        let user_agent = pipe.query().user_agent(*client).to_string();
        let audio = player.audio_streams.iter().max_by_key(|s| s.bitrate);
        match audio.map(|s| crate::download::probe(&s.url, &user_agent)) {
            Some(Ok(())) => {
                found_player = Some((player, *client, user_agent));
                break;
            }
            Some(Err(error)) => {
                refused = refused || error.contains("403");
                last_error = format!("{client:?}: {error}");
                refusals.push(last_error.clone());
            }
            None => last_error = format!("el cliente {client:?} no devolvió audio"),
        }
    }
    let Some((player, used_client, user_agent)) = found_player else {
        // Separate messages because the fixes are different: a refusal is
        // YouTube saying no to this network or this app, while everything
        // else is usually a transient failure worth retrying.
        let detail = refusals.join(" | ");
        return Err(if refused {
            format!(
                "YouTube rechazó la descarga desde esta red (403). Pulsa «Diagnóstico» para ver qué cliente falla. [{detail}]"
            )
        } else {
            format!("no se pudieron obtener los streams ({detail})")
        });
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
        client: format!("{used_client:?}").to_lowercase(),
        user_agent,
    })
}


/// Resolve through a direct (token-free) client, probing before returning.
fn resolve_direct(client: crate::innertube::Client, id: &str) -> Result<Resolved, String> {
    let player = crate::innertube::player(client, id)?;
    if player.is_live {
        return Err(format!(
            "{}: es un directo, y YouTube no sirve archivos descargables para directos",
            client.label
        ));
    }
    let audio = player
        .best_audio()
        .ok_or_else(|| format!("{}: sin pista de audio", client.label))?;
    // Same rule as everywhere else: a URL that will not serve bytes must
    // never reach the user as though it would.
    crate::download::probe(&audio.url, client.user_agent)
        .map_err(|e| format!("{}: {e}", client.label))?;

    Ok(Resolved {
        id: id.to_string(),
        title: player.title.clone(),
        channel: player.channel.clone(),
        duration: player.duration,
        audio: Some(Stream {
            // Audio in an MP4 container is an .m4a as far as every player and
            // file manager is concerned.
            ext: if audio.codec.starts_with("mp4a") {
                "m4a".to_string()
            } else {
                container_of(&audio.mime, "webm")
            },
            url: audio.url.clone(),
            codec: audio.codec.clone(),
            bitrate: audio.bitrate,
            height: 0,
            size: audio.size,
        }),
        video: player.best_video().map(|video| Stream {
            url: video.url.clone(),
            ext: container_of(&video.mime, "mp4"),
            codec: video.codec.clone(),
            bitrate: video.bitrate,
            height: video.height,
            size: video.size,
        }),
        client: client.label.to_string(),
        user_agent: client.user_agent.to_string(),
    })
}

/// Try every client and report exactly what each one did, as JSON.
///
/// This exists because the failure that mattered most could not be reproduced
/// by its author: downloads worked from one network and were refused with 403
/// on a phone. Guessing across that gap is expensive; a button that answers
/// "visionos: ok, ios: 403" turns it into a fact.
pub fn diagnose(video: &str) -> String {
    let Some(id) = video_id_of(video) else {
        return serde_json::json!([{ "client": "-", "resolve": "no es un vídeo de YouTube" }])
            .to_string();
    };
    let mut report = Vec::new();

    for client in DIRECT_CLIENTS {
        report.push(match crate::innertube::player(*client, &id) {
            Ok(player) => match player.best_audio() {
                Some(audio) => match crate::download::probe(&audio.url, client.user_agent) {
                    Ok(()) => serde_json::json!({
                        "client": client.label, "resolve": "ok", "download": "ok",
                        "codec": audio.codec, "kbps": audio.bitrate / 1000,
                    }),
                    Err(error) => serde_json::json!({
                        "client": client.label, "resolve": "ok", "download": error,
                    }),
                },
                None => serde_json::json!({
                    "client": client.label, "resolve": "ok", "download": "sin audio",
                }),
            },
            Err(error) => serde_json::json!({ "client": client.label, "resolve": error }),
        });
    }

    if let (Ok(runtime), Ok(pipe)) = (runtime(), client()) {
        for client in CLIENTS {
            let label = format!("{client:?}").to_lowercase();
            report.push(
                match runtime.block_on(async { pipe.query().player_from_client(&id, *client).await })
                {
                    Ok(player) => {
                        let user_agent = pipe.query().user_agent(*client).to_string();
                        match player.audio_streams.iter().max_by_key(|s| s.bitrate) {
                            Some(audio) => match crate::download::probe(&audio.url, &user_agent) {
                                Ok(()) => serde_json::json!({
                                    "client": label, "resolve": "ok", "download": "ok",
                                    "kbps": audio.bitrate / 1000,
                                }),
                                Err(error) => serde_json::json!({
                                    "client": label, "resolve": "ok", "download": error,
                                }),
                            },
                            None => serde_json::json!({
                                "client": label, "resolve": "ok", "download": "sin audio",
                            }),
                        }
                    }
                    Err(error) => serde_json::json!({ "client": label, "resolve": error.to_string() }),
                },
            );
        }
    }

    serde_json::Value::Array(report).to_string()
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
