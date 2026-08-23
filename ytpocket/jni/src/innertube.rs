//! A direct Innertube player client, for the clients that need no PO token.
//!
//! Why this exists at all, given rustypipe is already here: **PO tokens**.
//! Since August 2024 YouTube has required a "proof of origin" token for
//! stream URLs from more and more of its clients, and enforcement depends on
//! the network you are on -- which is exactly why downloads worked from a
//! laptop and came back 403 on a phone. yt-dlp's own client table (the most
//! carefully maintained source of truth for this) currently marks the token
//! as required for `web`, `mweb`, `android`, `ios`, `web_safari`,
//! `tv_simply` and friends, and rustypipe only speaks to clients on that
//! list.
//!
//! Four clients are *not* on it: `tv`, `tv_downgraded`, `web_embedded` and
//! `visionos`. Of those, only **`visionos`** also needs no JavaScript player
//! -- i.e. its stream URLs arrive ready to use, with no signature
//! deobfuscation, no QuickJS, and nothing that can rot when YouTube reshuffles
//! its player script. That makes it the right primary client for a phone app,
//! and it is what this module talks to.
//!
//! Generating a PO token is the other way out, and it is not available here:
//! it needs a simulated browser environment (yt-dlp delegates it to plugins,
//! NewPipe runs BotGuard inside an Android WebView). That is a possible
//! future addition, not a dependency this should carry today.
//!
//! Deliberately hand-rolled rather than another crate: it is one POST and
//! one JSON walk, and the whole point is not to depend on somebody else's
//! idea of which clients are safe.

use serde::Serialize;
use serde_json::Value;

/// One Innertube client's identity, as YouTube expects to see it.
///
/// The values come from yt-dlp's `INNERTUBE_CLIENTS`, which is where this
/// information is kept current; they are copied rather than derived because
/// YouTube checks them against real app releases.
#[derive(Clone, Copy)]
pub struct Client {
    pub label: &'static str,
    pub client_name: &'static str,
    pub client_version: &'static str,
    pub client_name_id: u32,
    pub user_agent: &'static str,
    pub device_make: &'static str,
    pub device_model: &'static str,
    pub os_name: &'static str,
    pub os_version: &'static str,
}

/// The Apple Vision Pro app. No PO token, no JS player: the only client with
/// both properties, and therefore the one this app leads with.
pub const VISIONOS: Client = Client {
    label: "visionos",
    client_name: "VISIONOS",
    client_version: "1.02",
    client_name_id: 101,
    user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15",
    device_make: "Apple",
    device_model: "RealityDevice17,1",
    os_name: "visionOS",
    os_version: "26.5.23O471",
};

#[derive(Serialize)]
struct ClientContext<'a> {
    #[serde(rename = "clientName")]
    client_name: &'a str,
    #[serde(rename = "clientVersion")]
    client_version: &'a str,
    #[serde(rename = "deviceMake")]
    device_make: &'a str,
    #[serde(rename = "deviceModel")]
    device_model: &'a str,
    #[serde(rename = "userAgent")]
    user_agent: &'a str,
    #[serde(rename = "osName")]
    os_name: &'a str,
    #[serde(rename = "osVersion")]
    os_version: &'a str,
    hl: &'a str,
    gl: &'a str,
}

#[derive(Serialize)]
struct Context<'a> {
    client: ClientContext<'a>,
}

#[derive(Serialize)]
struct PlayerRequest<'a> {
    context: Context<'a>,
    #[serde(rename = "videoId")]
    video_id: &'a str,
    // Both of these say "yes, I know what I'm asking for": without them
    // YouTube withholds streams for anything it considers sensitive and
    // answers with a playability error instead.
    #[serde(rename = "contentCheckOk")]
    content_check_ok: bool,
    #[serde(rename = "racyCheckOk")]
    racy_check_ok: bool,
}

/// One playable track, straight out of `streamingData.adaptiveFormats`.
pub struct Format {
    pub url: String,
    pub mime: String,
    pub codec: String,
    pub bitrate: u32,
    pub height: u32,
    pub size: u64,
}

pub struct Player {
    pub title: String,
    pub channel: String,
    pub duration: Option<u32>,
    pub is_live: bool,
    pub formats: Vec<Format>,
}

/// Ask a client for a video's streams.
pub fn player(client: Client, video_id: &str) -> Result<Player, String> {
    let body = PlayerRequest {
        context: Context {
            client: ClientContext {
                client_name: client.client_name,
                client_version: client.client_version,
                device_make: client.device_make,
                device_model: client.device_model,
                user_agent: client.user_agent,
                os_name: client.os_name,
                os_version: client.os_version,
                hl: "en",
                gl: "US",
            },
        },
        video_id,
        content_check_ok: true,
        racy_check_ok: true,
    };

    let response = crate::download::http()?
        // `prettyPrint=false` is what every real client sends; the response
        // is otherwise indented, for no reason and at our expense.
        .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
        .header(reqwest::header::USER_AGENT, client.user_agent)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ORIGIN, "https://www.youtube.com")
        .header("X-YouTube-Client-Name", client.client_name_id.to_string())
        .header("X-YouTube-Client-Version", client.client_version)
        .json(&body)
        .send()
        .map_err(|e| format!("{}: no se pudo conectar: {e}", client.label))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("{}: HTTP {}", client.label, status.as_u16()));
    }
    let json: Value = response
        .json()
        .map_err(|e| format!("{}: respuesta ilegible: {e}", client.label))?;

    // A video can answer 200 and still refuse to play (age gate, region
    // block, removed). Saying which is far more useful than "no streams".
    let playability = json.pointer("/playabilityStatus/status").and_then(Value::as_str).unwrap_or("");
    if !playability.is_empty() && playability != "OK" {
        let reason = json
            .pointer("/playabilityStatus/reason")
            .and_then(Value::as_str)
            .unwrap_or(playability);
        return Err(format!("{}: {reason}", client.label));
    }

    let details = json.pointer("/videoDetails");
    let is_live = details
        .and_then(|d| d.get("isLiveContent"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let duration = details
        .and_then(|d| d.get("lengthSeconds"))
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|seconds| *seconds > 0);

    let mut formats = Vec::new();
    for entry in json
        .pointer("/streamingData/adaptiveFormats")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        // No `url` means the client needs the JS player to decipher it. This
        // module only speaks to clients where that never happens, so an
        // entry without one is skipped rather than fudged.
        let Some(url) = entry.get("url").and_then(Value::as_str) else { continue };
        let mime = entry.get("mimeType").and_then(Value::as_str).unwrap_or("").to_string();
        formats.push(Format {
            url: url.to_string(),
            codec: codec_of(&mime),
            mime,
            bitrate: entry.get("bitrate").and_then(Value::as_u64).unwrap_or(0) as u32,
            height: entry.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
            size: entry
                .get("contentLength")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        });
    }

    Ok(Player {
        title: details
            .and_then(|d| d.get("title"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        channel: details
            .and_then(|d| d.get("author"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        duration,
        is_live,
        formats,
    })
}

/// `video/mp4; codecs="avc1.640028"` -> `avc1`.
fn codec_of(mime: &str) -> String {
    mime.split("codecs=")
        .nth(1)
        .map(|rest| rest.trim_matches('"'))
        .and_then(|codecs| codecs.split(&[',', '.'][..]).next())
        .unwrap_or("")
        .trim()
        .to_lowercase()
}

impl Player {
    /// Best AAC audio track. AAC specifically: it is what the MP3 encoder
    /// decodes and what `MediaMuxer` accepts into an MP4.
    pub fn best_audio(&self) -> Option<&Format> {
        self.formats
            .iter()
            .filter(|f| f.mime.starts_with("audio/") && f.codec.starts_with("mp4a"))
            .max_by_key(|f| f.bitrate)
            .or_else(|| {
                self.formats
                    .iter()
                    .filter(|f| f.mime.starts_with("audio/"))
                    .max_by_key(|f| f.bitrate)
            })
    }

    /// Best AVC/H.264 video track, for the same reason: every Android
    /// decoder and the platform muxer handle it.
    pub fn best_video(&self) -> Option<&Format> {
        self.formats
            .iter()
            .filter(|f| f.mime.starts_with("video/") && f.codec.starts_with("avc"))
            .max_by_key(|f| f.height)
            .or_else(|| {
                self.formats
                    .iter()
                    .filter(|f| f.mime.starts_with("video/"))
                    .max_by_key(|f| f.height)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codecs_are_read_out_of_the_mime_type() {
        assert_eq!(codec_of(r#"video/mp4; codecs="avc1.640028""#), "avc1");
        assert_eq!(codec_of(r#"audio/mp4; codecs="mp4a.40.2""#), "mp4a");
        assert_eq!(codec_of(r#"video/webm; codecs="vp9""#), "vp9");
        assert_eq!(codec_of("audio/mp4"), "");
    }

    /// Live: the whole reason this module exists -- a client that needs no PO
    /// token hands back URLs that actually serve bytes. `#[ignore]`d.
    #[test]
    #[ignore]
    fn visionos_streams_serve_bytes() {
        let player = player(VISIONOS, "dQw4w9WgXcQ").expect("visionos player failed");
        println!("{} — {} ({:?}s), {} formats", player.title, player.channel, player.duration, player.formats.len());
        assert!(!player.title.is_empty());
        assert!(!player.formats.is_empty(), "no direct-URL formats: the client needed a JS player after all");

        let audio = player.best_audio().expect("no audio format");
        println!("audio: {} {} {}kbps {}B", audio.mime, audio.codec, audio.bitrate / 1000, audio.size);
        assert!(audio.codec.starts_with("mp4a"), "expected AAC, got {}", audio.codec);
        crate::download::probe(&audio.url, VISIONOS.user_agent).expect("audio stream refused");

        let video = player.best_video().expect("no video format");
        println!("video: {} {} {}p {}B", video.mime, video.codec, video.height, video.size);
        assert!(video.codec.starts_with("avc"), "expected H.264, got {}", video.codec);
        crate::download::probe(&video.url, VISIONOS.user_agent).expect("video stream refused");
    }
}
