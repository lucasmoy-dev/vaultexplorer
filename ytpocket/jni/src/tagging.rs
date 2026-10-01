//! Who sang it, on which album, and what the cover looks like.
//!
//! A video title is not metadata. "Queen – Bohemian Rhapsody (Official Video
//! Remastered)" by the channel "Queen Official" is what YouTube knows; what a
//! music player wants is artist *Queen*, album *A Night At The Opera*, title
//! *Bohemian Rhapsody*, 1975, and the album cover. This module gets from one
//! to the other and writes the result as ID3 tags, so the file can be named
//! `Artist - Album - Song.mp3` and filed under its artist.
//!
//! Where the facts come from, best first, all free and with no API key:
//!
//! 1. **The video's own description**, when YouTube generated it. Uploads by
//!    "Artist - Topic" channels carry a block that starts "Provided to
//!    YouTube by" and spells out song, artist, album and release date -- the
//!    label's own data, exact for that recording.
//! 2. **The iTunes Search API** (`itunes.apple.com/search`): one request per
//!    song, no key, and it answers with album, release date, genre, track
//!    number and a 600px cover. Chosen over MusicBrainz + Cover Art Archive
//!    because that is two services, two requests and a redirect chase per
//!    song for the same facts, and the cover archive is the slow half.
//!    Apple asks for roughly 20 requests a minute, so requests are spaced
//!    (see [`ITUNES_SPACING`]).
//! 3. **What YouTube said**, cleaned up: "Artist - Song (Official Video)"
//!    split at the dash, the channel with its "VEVO" / " - Topic" stripped,
//!    and the video thumbnail as cover. Never nothing: a file that ends up
//!    as "Channel - Video title" is still better than a failed download.
//!
//! A search result is only believed when the artist *and* the title agree
//! with what was asked for, and the duration is close when it is known --
//! iTunes answers "Despechá" with a lullaby cover of it, and tagging the real
//! song with that would be worse than leaving it untagged.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Written into every MP3 this app makes, as the "encoded by" frame, so the
/// organiser can tell its own files from the rest of a music folder.
pub const MADE_BY: &str = "YT Pocket";

/// The comment an organised file carries: `YT Pocket · itunes`. Its presence
/// means "already looked up" -- a second run then only checks names and
/// folders, with no network at all.
const ORGANISED_PREFIX: &str = "YT Pocket · ";

/// What the caller knows about the audio. For a fresh download that is the
/// whole YouTube answer; for a file being organised it may be nothing, and
/// the file's own tags fill in.
#[derive(Deserialize, Default, Debug, Clone)]
#[serde(default)]
pub struct Hint {
    pub video_id: String,
    pub title: String,
    pub channel: String,
    pub duration: Option<u32>,
    pub description: String,
    /// The current file name without extension: the last resort for a file
    /// with no title tag at all.
    pub name: String,
}

#[derive(Serialize, Debug, Clone, Default, PartialEq)]
pub struct Meta {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub album_artist: String,
    /// `1975-10-31` or `1975`, as precise as the source was.
    pub date: String,
    pub genre: String,
    pub track: Option<u32>,
    pub track_total: Option<u32>,
    #[serde(skip)]
    pub cover_url: Option<String>,
    /// `youtube_music`, `itunes`, `youtube` or `tags` (already organised).
    pub source: String,
}

/// What `tag_file` did, for the caller to name and file the result.
#[derive(Serialize, Debug)]
pub struct Outcome {
    #[serde(flatten)]
    pub meta: Meta,
    pub file_name: String,
    pub folder: String,
    pub cover: bool,
    /// False when the file was already organised and nothing was rewritten.
    pub rewritten: bool,
}

// ---- reading what YouTube says ------------------------------------------

/// Bracketed bits that describe the *upload*, not the song: "(Official
/// Video)", "[Lyrics]", "(Video Oficial)", "(HD)". A group is dropped when
/// any of these appears in it. "Live" and "Remix" are not here on purpose:
/// those name a different recording, and the search should look for that one.
const NOISE: &[&str] = &[
    "official", "oficial", "video", "vídeo", "videoclip", "clip", "audio", "lyric", "lyrics",
    "letra", "visualizer", "visualiser", "hd", "4k", "hq", "remaster", "remastered", "mv",
    "m/v", "color coded", "full song", "explicit", "en exclusiva", "estreno",
];

/// Split a video title into (artist, song), using the channel when the title
/// alone does not say.
pub fn guess(title: &str, channel: &str) -> (String, String) {
    let title = strip_noise(title);
    for separator in [" - ", " – ", " — ", " ~ ", " || "] {
        if let Some((left, right)) = title.split_once(separator) {
            let (artist, song) = (left.trim(), right.trim());
            if !artist.is_empty() && !song.is_empty() {
                return (artist.to_string(), strip_noise(song));
            }
        }
    }
    (clean_channel(channel), title)
}

/// Remove the upload's decorations: noise groups, trailing "| Official
/// Video", stray quotes around the song.
fn strip_noise(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(['(', '[']) {
        let close = if rest[start..].starts_with('(') { ')' } else { ']' };
        let Some(len) = rest[start..].find(close) else { break };
        let inner = rest[start + 1..start + len].to_lowercase();
        out.push_str(&rest[..start]);
        if !NOISE.iter().any(|word| contains_word(&inner, word)) {
            out.push_str(&rest[start..=start + len]);
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    // "Song | Official Video" -- whatever follows a bar is the uploader
    // talking, never part of the title.
    let out = out.split(" | ").next().unwrap_or("").to_string();
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    out.trim_matches(|c: char| c == '"' || c == '\'' || c == '“' || c == '”' || c.is_whitespace())
        .to_string()
}

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack
        .split(|c: char| !(c.is_alphanumeric() || c == '/'))
        .any(|piece| piece == word)
        || (word.contains(' ') && haystack.contains(word))
}

/// "QueenVEVO", "Queen Official", "Queen - Topic" -> "Queen".
pub fn clean_channel(channel: &str) -> String {
    let mut name = channel.trim().to_string();
    for suffix in [" - Topic", "VEVO", " Official", " Oficial", " - Official", " Music", " TV"] {
        if name.len() > suffix.len() && name.to_lowercase().ends_with(&suffix.to_lowercase()) {
            name.truncate(name.len() - suffix.len());
            name = name.trim().to_string();
        }
    }
    for prefix in ["Official ", "Oficial "] {
        if name.to_lowercase().starts_with(&prefix.to_lowercase()) && name.len() > prefix.len() {
            name = name[prefix.len()..].trim().to_string();
        }
    }
    name
}

/// The "Provided to YouTube by" block of an auto-generated upload:
///
/// ```text
/// Provided to YouTube by Columbia
///
/// DESPECHÁ · ROSALÍA
///
/// DESPECHÁ
///
/// ℗ 2022 Columbia Records
///
/// Released on: 2022-07-28
/// ```
pub fn from_description(description: &str) -> Option<Meta> {
    if !description.contains("Provided to YouTube by") {
        return None;
    }
    let lines: Vec<&str> = description
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.iter().position(|l| l.starts_with("Provided to YouTube by"))?;
    let credits = lines.get(start + 1)?;
    let mut parts = credits.split(" · ").map(str::trim);
    let title = parts.next()?.to_string();
    let artists: Vec<&str> = parts.collect();
    if title.is_empty() || artists.is_empty() {
        return None;
    }
    let album = lines
        .get(start + 2)
        .filter(|l| !l.starts_with('℗') && !l.starts_with('©') && !l.contains(':'))
        .map(|l| l.to_string())
        .unwrap_or_default();
    let date = lines
        .iter()
        .find_map(|l| l.strip_prefix("Released on:"))
        .map(|d| d.trim().to_string())
        .or_else(|| {
            // No release date: the ℗ line still has the year.
            lines.iter().find_map(|l| {
                l.strip_prefix('℗')
                    .and_then(|rest| rest.split_whitespace().next())
                    .filter(|y| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()))
                    .map(str::to_string)
            })
        })
        .unwrap_or_default();
    Some(Meta {
        artist: artists.join(", "),
        album_artist: artists[0].to_string(),
        title,
        album,
        date,
        source: "youtube_music".to_string(),
        ..Meta::default()
    })
}

// ---- matching search results ----------------------------------------------

/// Lowercase, accents folded, punctuation gone: "ROSALÍA" and "Rosalia"
/// are the same artist.
fn normalise(text: &str) -> String {
    let folded: String = text
        .to_lowercase()
        .replace('&', " and ")
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' | 'ø' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A title without its brackets, its " - Remastered 2011" tail and its
/// "feat." credits: the part two releases of the same song share.
fn core_title(title: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let out = out.split(" - ").next().unwrap_or("").to_string();
    let lower = out.to_lowercase();
    let cut = [" feat.", " feat ", " ft.", " ft ", " featuring "]
        .iter()
        .filter_map(|marker| lower.find(marker))
        .min()
        .unwrap_or(out.len());
    normalise(&out[..cut])
}

/// The first named artist, for comparing "Queen & Adam Lambert" with "Queen".
fn first_artist(artist: &str) -> String {
    let lower = artist.to_lowercase();
    let cut = [" feat", " ft.", " ft ", " & ", ", ", " x ", " with "]
        .iter()
        .filter_map(|marker| lower.find(marker))
        .min()
        .unwrap_or(artist.len());
    normalise(&artist[..cut])
}

fn tokens(text: &str) -> Vec<&str> {
    text.split(' ').filter(|t| !t.is_empty()).collect()
}

fn artist_agrees(asked: &str, found: &str) -> bool {
    let asked_first = first_artist(asked);
    let found_all = normalise(found);
    let found_first = first_artist(found);
    let asked_all = normalise(asked);
    if asked_first.is_empty() || found_first.is_empty() {
        return false;
    }
    let contains = |hay: &str, needle: &str| {
        let hay = tokens(hay);
        tokens(needle).iter().all(|t| hay.contains(t))
    };
    contains(&found_all, &asked_first) || contains(&asked_all, &found_first)
}

fn title_agrees(asked: &str, found: &str) -> bool {
    let (a, f) = (core_title(asked), core_title(found));
    if a.is_empty() || f.is_empty() {
        return false;
    }
    if a == f {
        return true;
    }
    let (ta, tf) = (tokens(&a), tokens(&f));
    let common = ta.iter().filter(|t| tf.contains(t)).count();
    let union = ta.len() + tf.len() - common;
    union > 0 && common as f32 / union as f32 >= 0.75
}

/// Words that make a track a different recording from the one asked for.
const VARIANTS: &[&str] = &[
    "live", "en vivo", "en directo", "remix", "mix", "acoustic", "acustico", "instrumental",
    "karaoke", "cover", "a cappella", "acapella", "edit", "demo", "version", "pianoforte",
    "slowed", "sped up", "reverb", "tabata", "lullaby", "8d", "piano",
];
/// Words in an album name that say "compilation", whose track is the same
/// recording but the wrong album to file it under.
const COMPILATIONS: &[&str] = &[
    "greatest hits", "best of", "the best", "collection", "hits", "anthems", "soundtrack",
    "essentials", "platinum", "ultimate", "kidz bop", "originals", "party", "workout",
    "lullaby", "karaoke", "now that s", "compilation", "exitos", "grandes exitos", "lo mejor",
];
const REISSUES: &[&str] = &["deluxe", "remaster", "remastered", "expanded", "anniversary", "edition"];

fn mentions(text: &str, words: &[&str]) -> bool {
    let text = format!(" {} ", normalise(text));
    words.iter().any(|w| text.contains(&format!(" {} ", normalise(w))))
}

/// "RR - Single" -> "RR": the store's decoration, not the album's name.
fn clean_album(album: &str) -> String {
    let mut name = album.trim().to_string();
    for suffix in [" - Single", " - EP"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            name = stripped.trim().to_string();
        }
    }
    name
}

/// Pick the iTunes result that is this song, or none of them.
///
/// Every candidate must agree on artist and title, and on duration within
/// 20s or 20% when both are known (a music video's intro is not a
/// different song; three minutes is). Among those, the one to file it under
/// is the **original album**: compilations, soundtracks, live takes and
/// remixes lose points, and an album that appears for several of the
/// matching tracks wins points (the deluxe, remastered and plain editions of
/// one album all point at it). Ties go to the earliest release.
pub fn best_match(results: &[Value], artist: &str, title: &str, duration: Option<u32>) -> Option<Meta> {
    struct Candidate<'a> {
        item: &'a Value,
        score: f32,
        date: String,
        group: String,
    }
    let asked_is_variant = mentions(title, VARIANTS);
    let field = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).unwrap_or("").to_string();

    let mut candidates: Vec<Candidate> = results
        .iter()
        .filter(|item| item.get("kind").and_then(Value::as_str).map_or(true, |k| k == "song"))
        .filter_map(|item| {
            let track = field(item, "trackName");
            let found_artist = field(item, "artistName");
            if !artist_agrees(artist, &found_artist) || !title_agrees(title, &track) {
                return None;
            }
            let found_seconds = item.get("trackTimeMillis").and_then(Value::as_u64).map(|ms| (ms / 1000) as f32);
            let mut score = 0.0f32;
            if let (Some(asked), Some(found)) = (duration, found_seconds) {
                let diff = (asked as f32 - found).abs();
                if diff > 20f32.max(asked as f32 * 0.2) {
                    return None;
                }
                score -= diff / 10.0;
            }
            let album = field(item, "collectionName");
            // A radio edit is the same recording, shortened -- usually the
            // very one a music video uses, which the duration check above has
            // already confirmed. Only a mild preference for the album take.
            let radio_edit = mentions(&track, &["radio edit", "single version"]);
            if !asked_is_variant && radio_edit {
                score -= 2.0;
            } else if !asked_is_variant && mentions(&track, VARIANTS) {
                score -= 10.0;
            }
            // A various-artists compilation is never the album to file a song
            // under ("Flaix FM 25 Aniversario"); an artist's own best-of is
            // merely a poor second choice.
            let various = mentions(&field(item, "collectionArtistName"), &["various artists", "varios artistas"]);
            if various {
                return None;
            }
            if mentions(&album, COMPILATIONS) {
                score -= 5.0;
            }
            if mentions(&album, REISSUES) || mentions(&track, REISSUES) {
                score -= 1.0;
            }
            Some(Candidate {
                item,
                score,
                date: field(item, "releaseDate"),
                group: core_title(&clean_album(&album)),
            })
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let groups: Vec<String> = candidates.iter().map(|c| c.group.clone()).collect();
    for candidate in &mut candidates {
        let seen = groups.iter().filter(|g| **g == candidate.group).count();
        candidate.score += 2.0 * (seen.min(4) as f32 - 1.0);
    }
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.date.cmp(&b.date))
    });
    let item = candidates[0].item;
    let found_artist = field(item, "artistName");
    let album_artist = Some(field(item, "collectionArtistName"))
        .filter(|a| !a.is_empty() && !mentions(a, &["various artists", "varios artistas"]))
        .unwrap_or_else(|| found_artist.clone());
    let date = field(item, "releaseDate");
    Some(Meta {
        artist: found_artist,
        title: field(item, "trackName"),
        album: clean_album(&field(item, "collectionName")),
        album_artist,
        date: date.get(..10).unwrap_or(&date).to_string(),
        genre: field(item, "primaryGenreName"),
        track: item.get("trackNumber").and_then(Value::as_u64).map(|n| n as u32),
        track_total: item.get("trackCount").and_then(Value::as_u64).map(|n| n as u32),
        // 100px is what the API lists; the same path serves any size.
        cover_url: Some(field(item, "artworkUrl100").replace("100x100bb", "600x600bb")).filter(|u| !u.is_empty()),
        source: "itunes".to_string(),
    })
}

// ---- network ----------------------------------------------------------------

/// Apple documents "approximately 20 calls per minute" for the search API and
/// answers 403 past it. Three seconds apart keeps a whole-library run under
/// that without anyone having to think about it.
const ITUNES_SPACING: Duration = Duration::from_millis(3100);
static LAST_ITUNES: Mutex<Option<Instant>> = Mutex::new(None);

/// Its own client: the download client (`download::http`) is pinned to IPv4
/// and refuses redirects, both for googlevideo's sake, and neither rule has
/// anything to do with Apple's servers or YouTube's thumbnails.
fn web() -> Result<&'static reqwest::blocking::Client, String> {
    static CLIENT: std::sync::OnceLock<Result<reqwest::blocking::Client, String>> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(20))
                .user_agent("YTPocket/0.2 (+https://github.com/lucasmoy-dev/vaultexplorer)")
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| e.clone())
}

fn itunes_search(term: &str, country: &str) -> Result<Vec<Value>, String> {
    for attempt in 0..2 {
        {
            let mut last = LAST_ITUNES.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(previous) = *last {
                let elapsed = previous.elapsed();
                if elapsed < ITUNES_SPACING {
                    std::thread::sleep(ITUNES_SPACING - elapsed);
                }
            }
            *last = Some(Instant::now());
        }
        let response = web()?
            .get("https://itunes.apple.com/search")
            .query(&[("term", term), ("entity", "song"), ("limit", "25"), ("country", country)])
            .send()
            .map_err(|e| format!("iTunes: {e}"))?;
        let status = response.status().as_u16();
        if status == 403 || status == 429 {
            // Over the rate limit: one long pause, then one more try.
            if attempt == 0 {
                std::thread::sleep(Duration::from_secs(30));
                continue;
            }
            return Err(format!("iTunes: HTTP {status} (límite de peticiones)"));
        }
        if !response.status().is_success() {
            return Err(format!("iTunes: HTTP {status}"));
        }
        let json: Value = response.json().map_err(|e| format!("iTunes: {e}"))?;
        return Ok(json.get("results").and_then(Value::as_array).cloned().unwrap_or_default());
    }
    Err("iTunes: sin respuesta".to_string())
}

/// An image, or nothing. Only JPEG/PNG with a real body: YouTube answers a
/// missing `maxresdefault` with a 120px grey placeholder, which is worse than
/// no cover.
fn fetch_image(url: &str) -> Option<Vec<u8>> {
    let response = web().ok()?.get(url).send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().ok()?.to_vec();
    let is_image = bytes.starts_with(&[0xFF, 0xD8, 0xFF]) || bytes.starts_with(b"\x89PNG");
    (is_image && bytes.len() > 5_000).then_some(bytes)
}

fn cover_for(meta: &Meta, video_id: &str) -> Option<Vec<u8>> {
    if let Some(bytes) = meta.cover_url.as_deref().and_then(fetch_image) {
        return Some(bytes);
    }
    if video_id.is_empty() {
        return None;
    }
    ["maxresdefault", "hqdefault"]
        .iter()
        .find_map(|size| fetch_image(&format!("https://i.ytimg.com/vi/{video_id}/{size}.jpg")))
}

/// Everything known about this song, from the best source that answers.
/// Never fails: the floor is what YouTube said. The flag says whether every
/// source that was asked actually answered -- false means "try again later".
pub fn lookup(hint: &Hint, country: &str) -> (Meta, bool) {
    let described = from_description(&hint.description);
    let (artist, title) = match &described {
        Some(meta) => (meta.artist.clone(), meta.title.clone()),
        None => guess(&hint.title, &hint.channel),
    };
    // Artist and title first; then the title alone, because the store's
    // search ranks by popularity and buries a big hit under the artist's
    // other songs when both names are in the query (measured: "Bad Bunny
    // Tití Me Preguntó" does not list the song, "Tití Me Preguntó" does).
    let song = core_display(&title);
    let mut terms = vec![format!("{} {song}", first_artist_display(&artist))];
    if song.chars().count() >= 4 {
        terms.push(song);
    }
    let mut found = None;
    let mut answered = true;
    for term in terms {
        match itunes_search(&term, country) {
            Ok(results) => {
                found = best_match(&results, &artist, &title, hint.duration);
                if found.is_some() {
                    break;
                }
            }
            Err(_) => {
                answered = false;
                break;
            }
        }
    }
    let meta = match (described, found) {
        // The label's own data names the recording exactly; the store adds
        // what the description lacks (cover, genre, track number) -- but only
        // when it found the same album, or it would be another album's cover.
        (Some(mut own), Some(store)) => {
            if own.album.is_empty() || normalise(&own.album) == normalise(&store.album) {
                if own.album.is_empty() {
                    own.album = store.album.clone();
                }
                own.genre = store.genre;
                own.track = store.track;
                own.track_total = store.track_total;
                own.cover_url = store.cover_url;
                if own.date.is_empty() {
                    own.date = store.date;
                }
            }
            own
        }
        (Some(own), None) => own,
        (None, Some(store)) => store,
        (None, None) => Meta {
            album_artist: artist.clone(),
            artist,
            title,
            source: "youtube".to_string(),
            ..Meta::default()
        },
    };
    (meta, answered)
}

/// The artist as typed, minus "feat." credits, for the search term.
fn first_artist_display(artist: &str) -> String {
    let lower = artist.to_lowercase();
    let cut = [" feat", " ft.", " ft "].iter().filter_map(|m| lower.find(m)).min().unwrap_or(artist.len());
    artist[..cut].trim().to_string()
}

/// The title without brackets, for the search term (iTunes matches words,
/// and "(Official Video)" leftovers only dilute it).
fn core_display(title: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    // A trailing "ft. Somebody" goes too: the store lists guests its own
    // way, and the words only dilute the search.
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = out.to_lowercase();
    let cut = [" feat.", " feat ", " ft.", " ft ", " featuring "]
        .iter()
        .filter_map(|marker| lower.find(marker))
        .min()
        .unwrap_or(out.len());
    out[..cut].trim().to_string()
}

// ---- the file ------------------------------------------------------------

/// What an MP3 already says about itself.
#[derive(Serialize, Debug, Default)]
pub struct Existing {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub date: String,
    pub made_by_us: bool,
    pub organised: bool,
    pub video_id: String,
    pub seconds: u32,
    /// The name and folder these tags call for -- only meaningful when
    /// `organised`, and what lets the organiser leave a file alone without
    /// copying it out first.
    pub file_name: String,
    pub folder: String,
}

pub fn read(path: &Path) -> Result<Existing, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    read_from(file)
}

/// `read`, from an open file: what the organiser hands over is a descriptor
/// from `ContentResolver`, and on Android reopening one through
/// `/proc/self/fd/N` is refused (measured on the emulator: "Permission
/// denied", because the descriptor points into MediaProvider's own storage).
pub fn read_from(file: std::fs::File) -> Result<Existing, String> {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::prelude::{Accessor, ItemKey};
    let tagged = lofty::probe::Probe::new(std::io::BufReader::new(file))
        .guess_file_type()
        .map_err(|e| e.to_string())?
        .read()
        .map_err(|e| e.to_string())?;
    let mut existing = Existing {
        seconds: tagged.properties().duration().as_secs() as u32,
        ..Existing::default()
    };
    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        let text = |key: ItemKey| tag.get_string(key).unwrap_or_default().to_string();
        existing.title = tag.title().map(|t| t.to_string()).unwrap_or_default();
        existing.artist = tag.artist().map(|t| t.to_string()).unwrap_or_default();
        existing.album = tag.album().map(|t| t.to_string()).unwrap_or_default();
        existing.album_artist = text(ItemKey::AlbumArtist);
        existing.date = text(ItemKey::RecordingDate);
        existing.made_by_us = text(ItemKey::EncodedBy) == MADE_BY;
        existing.organised = tag
            .get_strings(ItemKey::Comment)
            .any(|c| c.starts_with(ORGANISED_PREFIX));
        existing.video_id = tag
            .get(ItemKey::AudioSourceUrl)
            .and_then(|item| item.value().locator().or_else(|| item.value().text()))
            .and_then(|url| url.rsplit_once("v="))
            .map(|(_, id)| id.to_string())
            .unwrap_or_default();
    }
    existing.file_name =
        crate::naming::track_file_name(&existing.artist, &existing.album, &existing.title, "mp3");
    let folder_artist = if existing.album_artist.is_empty() { &existing.artist } else { &existing.album_artist };
    existing.folder = crate::naming::artist_folder(&first_artist_display(folder_artist));
    Ok(existing)
}

/// Replace the file's tags with `meta`, the cover and the duration.
///
/// `organised` writes the marker comment; it is left off when a source did
/// not answer, so the next run looks the song up again instead of trusting a
/// fallback forever.
pub fn write(path: &Path, meta: &Meta, cover: Option<&[u8]>, video_id: &str, organised: bool) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::file::AudioFile;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::prelude::{Accessor, ItemKey, TagExt};

    let seconds_ms = lofty::probe::Probe::open(path)
        .and_then(|p| p.read())
        .map(|f| f.properties().duration().as_millis())
        .unwrap_or(0);

    let mut tag = lofty::tag::Tag::new(lofty::tag::TagType::Id3v2);
    tag.set_title(meta.title.clone());
    if !meta.artist.is_empty() {
        tag.set_artist(meta.artist.clone());
    }
    if !meta.album.is_empty() {
        tag.set_album(meta.album.clone());
    }
    if !meta.album_artist.is_empty() {
        tag.insert_text(ItemKey::AlbumArtist, meta.album_artist.clone());
    }
    if !meta.date.is_empty() {
        tag.insert_text(ItemKey::RecordingDate, meta.date.clone());
    }
    if !meta.genre.is_empty() {
        tag.set_genre(meta.genre.clone());
    }
    if let Some(track) = meta.track {
        tag.set_track(track);
    }
    if let Some(total) = meta.track_total {
        tag.set_track_total(total);
    }
    if seconds_ms > 0 {
        // TLEN: milliseconds, as a string. Players that do not want to scan
        // the whole file for its length read this instead.
        tag.insert_text(ItemKey::Length, seconds_ms.to_string());
    }
    tag.insert_text(ItemKey::EncodedBy, MADE_BY.to_string());
    if organised {
        tag.insert_text(ItemKey::Comment, format!("{ORGANISED_PREFIX}{}", meta.source));
    }
    if let Some(bytes) = cover {
        let mime = if bytes.starts_with(b"\x89PNG") { MimeType::Png } else { MimeType::Jpeg };
        tag.push_picture(
            Picture::unchecked(bytes.to_vec())
                .pic_type(PictureType::CoverFront)
                .mime_type(mime)
                // Not empty: with no description lofty 0.24 writes an ID3v2.3
                // APIC whose empty UTF-16 description lacks its BOM, and then
                // cannot read its own file back ("invalid byte order mark").
                .description("Cover")
                .build(),
        );
    }
    // The source video, as WOAS ("official audio source webpage"). Added to
    // the ID3v2 tag itself: lofty's generic tag does not carry URL frames
    // through to the file (measured -- the frame silently went missing).
    let mut id3: lofty::id3::v2::Id3v2Tag = tag.into();
    if !video_id.is_empty() {
        use lofty::id3::v2::{Frame, FrameId, UrlLinkFrame};
        id3.insert(Frame::Url(UrlLinkFrame::new(
            FrameId::Valid(std::borrow::Cow::Borrowed("WOAS")),
            format!("https://www.youtube.com/watch?v={video_id}"),
        )));
    }
    // ID3v2.3, not lofty's default 2.4: Android's own metadata reader takes
    // the year from 2.3's TYER and ignores 2.4's TDRC (measured on the
    // emulator -- `METADATA_KEY_YEAR` came back null), and so do plenty of
    // car stereos. lofty splits the date into TYER + TDAT itself.
    id3.save_to_path(path, WriteOptions::default().use_id3v23(true)).map_err(|e| e.to_string())
}

/// Look the song up and rewrite the file's tags, unless it was organised
/// already -- then only the name and folder are worked out, from its tags,
/// with no network at all.
pub fn tag_file(path: &Path, hint: &Hint, country: &str) -> Result<Outcome, String> {
    let existing = read(path)?;
    let organised_already = existing.organised && hint.title.is_empty();
    let (meta, rewritten, cover) = if organised_already {
        let meta = Meta {
            artist: existing.artist.clone(),
            title: existing.title.clone(),
            album: existing.album.clone(),
            album_artist: existing.album_artist.clone(),
            date: existing.date.clone(),
            source: "tags".to_string(),
            ..Meta::default()
        };
        (meta, false, false)
    } else {
        let mut hint = hint.clone();
        if hint.title.is_empty() {
            hint.title = if existing.title.is_empty() { hint.name.clone() } else { existing.title.clone() };
        }
        if hint.channel.is_empty() {
            hint.channel = existing.artist.clone();
        }
        if hint.video_id.is_empty() {
            hint.video_id = existing.video_id.clone();
        }
        if hint.duration.is_none() && existing.seconds > 0 {
            hint.duration = Some(existing.seconds);
        }
        let (meta, answered) = lookup(&hint, country);
        let cover = cover_for(&meta, &hint.video_id);
        write(path, &meta, cover.as_deref(), &hint.video_id, answered)?;
        (meta, true, cover.is_some())
    };
    let folder_artist = if meta.album_artist.is_empty() { &meta.artist } else { &meta.album_artist };
    Ok(Outcome {
        file_name: crate::naming::track_file_name(&meta.artist, &meta.album, &meta.title, "mp3"),
        folder: crate::naming::artist_folder(&first_artist_display(folder_artist)),
        meta,
        cover,
        rewritten,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<Value> {
        let path = format!("{}/testdata/{name}", env!("CARGO_MANIFEST_DIR"));
        let json: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        json["results"].as_array().unwrap().clone()
    }

    #[test]
    fn a_video_title_splits_into_artist_and_song() {
        assert_eq!(
            guess("Queen – Bohemian Rhapsody (Official Video Remastered)", "Queen Official"),
            ("Queen".into(), "Bohemian Rhapsody".into())
        );
        assert_eq!(
            guess("Rick Astley - Never Gonna Give You Up (Official Music Video)", "Rick Astley"),
            ("Rick Astley".into(), "Never Gonna Give You Up".into())
        );
        assert_eq!(
            guess("ROSALÍA - DESPECHÁ (Official Video)", "ROSALÍA"),
            ("ROSALÍA".into(), "DESPECHÁ".into())
        );
        // A real version is kept: it is a different recording to look for.
        assert_eq!(
            guess("Nirvana - Come As You Are (Live) [HD]", "NirvanaVEVO"),
            ("Nirvana".into(), "Come As You Are (Live)".into())
        );
        assert_eq!(
            guess("Bad Bunny - Tití Me Preguntó | Un Verano Sin Ti (Video Oficial)", "Bad Bunny"),
            ("Bad Bunny".into(), "Tití Me Preguntó".into())
        );
    }

    #[test]
    fn with_no_dash_the_channel_is_the_artist() {
        assert_eq!(guess("Bohemian Rhapsody (Remastered 2011)", "Queen - Topic"), ("Queen".into(), "Bohemian Rhapsody".into()));
        assert_eq!(guess("Shape of You [Official Video]", "Ed SheeranVEVO"), ("Ed Sheeran".into(), "Shape of You".into()));
        assert_eq!(clean_channel("Official Queen"), "Queen");
        assert_eq!(clean_channel("Rick Astley"), "Rick Astley");
    }

    #[test]
    fn an_auto_generated_description_is_read_as_metadata() {
        let description = "Provided to YouTube by Columbia\n\nDESPECHÁ · ROSALÍA\n\nDESPECHÁ\n\n℗ 2022 Columbia Records, a Division of Sony Music Entertainment\n\nReleased on: 2022-07-28\n\nProducer: Noah Goldstein\n\nAuto-generated by YouTube.";
        let meta = from_description(description).expect("not recognised");
        assert_eq!(meta.artist, "ROSALÍA");
        assert_eq!(meta.title, "DESPECHÁ");
        assert_eq!(meta.album, "DESPECHÁ");
        assert_eq!(meta.date, "2022-07-28");
        assert_eq!(meta.source, "youtube_music");

        let several = "Provided to YouTube by UMG\n\nBESO · ROSALÍA · Rauw Alejandro\n\nRR\n\n℗ 2023 Columbia\n\nAuto-generated by YouTube.";
        let meta = from_description(several).unwrap();
        assert_eq!(meta.artist, "ROSALÍA, Rauw Alejandro");
        assert_eq!(meta.album_artist, "ROSALÍA");
        assert_eq!(meta.album, "RR");
        assert_eq!(meta.date, "2023", "falls back to the ℗ year");

        assert!(from_description("Taken from A Night At The Opera, 1975.").is_none());
    }

    #[test]
    fn the_original_album_wins_over_soundtracks_compilations_and_live_takes() {
        // Real iTunes answer (testdata/itunes-queen.json): the soundtrack is
        // listed *first*, then the deluxe edition, the Platinum Collection,
        // Greatest Hits, and only then the plain album.
        let meta = best_match(&fixture("itunes-queen.json"), "Queen", "Bohemian Rhapsody", Some(359)).expect("no match");
        assert_eq!(meta.artist, "Queen");
        assert_eq!(meta.title, "Bohemian Rhapsody");
        assert_eq!(meta.album, "A Night At The Opera");
        assert_eq!(meta.date, "1975-10-31");
        assert_eq!(meta.genre, "Rock");
        assert!(meta.cover_url.as_deref().unwrap().contains("600x600bb"));
    }

    #[test]
    fn a_remaster_does_not_beat_the_album_it_remasters() {
        let meta = best_match(&fixture("itunes-astley.json"), "Rick Astley", "Never Gonna Give You Up", Some(213)).expect("no match");
        assert_eq!(meta.title, "Never Gonna Give You Up");
        assert_eq!(meta.album, "Whenever You Need Somebody");
    }

    #[test]
    fn covers_and_lullabies_are_not_the_song() {
        // iTunes has no "DESPECHÁ" by ROSALÍA in this store; it does have a
        // lullaby version, a KIDZ BOP version and remixes. None may win.
        assert!(best_match(&fixture("itunes-rosalia.json"), "ROSALÍA", "DESPECHÁ", Some(157)).is_none());
    }

    #[test]
    fn a_very_different_duration_is_a_different_recording() {
        // The album take is 355s; a 9-minute "video" is something else.
        assert!(best_match(&fixture("itunes-queen.json"), "Queen", "Bohemian Rhapsody", Some(540)).is_none());
        // Unknown duration: artist and title alone decide.
        assert!(best_match(&fixture("itunes-queen.json"), "Queen", "Bohemian Rhapsody", None).is_some());
    }

    #[test]
    fn the_wrong_artist_is_never_accepted() {
        assert!(best_match(&fixture("itunes-queen.json"), "Metallica", "Bohemian Rhapsody", Some(359)).is_none());
    }

    #[test]
    fn accents_and_case_do_not_matter_when_comparing() {
        assert!(artist_agrees("Rosalia", "ROSALÍA"));
        assert!(artist_agrees("Queen", "Queen & Adam Lambert"));
        assert!(artist_agrees("Bad Bunny feat. ROSALÍA", "Bad Bunny & ROSALÍA"));
        assert!(!artist_agrees("Queen", "Queens of the Stone Age"));
        assert!(title_agrees("Tití Me Preguntó", "TITÍ ME PREGUNTÓ"));
        assert!(title_agrees("Bohemian Rhapsody", "Bohemian Rhapsody - Remastered 2011"));
        assert!(!title_agrees("Love", "Love Story"));
    }

    /// Tags written, read back by lofty: everything the player shows,
    /// including the cover and the length.
    #[test]
    fn full_tags_and_the_cover_are_written_and_read_back() {
        let dir = std::env::temp_dir().join("ytpocket-tagging-test");
        let _ = std::fs::create_dir_all(&dir);
        let m4a = dir.join("tone.m4a");
        let mp3 = dir.join("tone.mp3");
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-c:a", "aac"])
            .arg(&m4a)
            .output();
        match made {
            Ok(out) if out.status.success() => {}
            _ => return,
        }
        crate::mp3::transcode(&m4a, &mp3, Some("Queen – Bohemian Rhapsody (Official Video)"), Some("Queen Official"), &mut |_| true).unwrap();

        // The transcode alone marks the file as ours, unorganised.
        let before = read(&mp3).unwrap();
        assert!(before.made_by_us);
        assert!(!before.organised);

        // A tiny real JPEG (SOI + padding) is enough for the picture frame.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        jpeg.extend(std::iter::repeat(0u8).take(6000));
        let meta = Meta {
            artist: "Queen".into(),
            title: "Bohemian Rhapsody".into(),
            album: "A Night At The Opera".into(),
            album_artist: "Queen".into(),
            date: "1975-10-31".into(),
            genre: "Rock".into(),
            track: Some(11),
            track_total: Some(12),
            cover_url: None,
            source: "itunes".into(),
        };
        write(&mp3, &meta, Some(&jpeg), "fJ9rUzIMcZQ", true).unwrap();

        use lofty::file::{AudioFile, TaggedFileExt};
        use lofty::prelude::{Accessor, ItemKey};
        let tagged = lofty::probe::Probe::open(&mp3).and_then(|p| p.read()).unwrap();
        let tag = tagged.primary_tag().unwrap();
        assert_eq!(tag.title().as_deref(), Some("Bohemian Rhapsody"));
        assert_eq!(tag.artist().as_deref(), Some("Queen"));
        assert_eq!(tag.album().as_deref(), Some("A Night At The Opera"));
        assert_eq!(tag.genre().as_deref(), Some("Rock"));
        assert_eq!(tag.track(), Some(11));
        assert_eq!(tag.get_string(ItemKey::RecordingDate), Some("1975-10-31"));
        let length: u64 = tag.get_string(ItemKey::Length).unwrap().parse().unwrap();
        assert!((3500..4500).contains(&length), "TLEN {length}ms for a 4s file");
        assert_eq!(tag.pictures().len(), 1);
        assert_eq!(tag.pictures()[0].pic_type(), lofty::picture::PictureType::CoverFront);
        assert_eq!(tag.pictures()[0].data().len(), jpeg.len());
        assert!(tagged.properties().duration().as_secs() >= 3);

        let after = read(&mp3).unwrap();
        assert!(after.organised && after.made_by_us);
        assert_eq!(after.video_id, "fJ9rUzIMcZQ");

        // A second run: no network, no rewrite, same name and folder.
        let outcome = tag_file(&mp3, &Hint::default(), "US").unwrap();
        assert!(!outcome.rewritten);
        assert_eq!(outcome.file_name, "Queen - A Night At The Opera - Bohemian Rhapsody.mp3");
        assert_eq!(outcome.folder, "Queen");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Live: the real iTunes API, the real cover. `#[ignore]`d.
    ///
    /// ```text
    /// cargo test --release -- --ignored --nocapture itunes_lookup
    /// ```
    #[test]
    #[ignore]
    fn itunes_lookup_finds_the_album_and_the_cover() {
        let hint = Hint {
            title: "Queen – Bohemian Rhapsody (Official Video Remastered)".into(),
            channel: "Queen Official".into(),
            duration: Some(359),
            ..Hint::default()
        };
        let (meta, answered) = lookup(&hint, "ES");
        println!("{meta:?}");
        assert!(answered);
        assert_eq!(meta.source, "itunes");
        assert_eq!(meta.album, "A Night At The Opera");
        let cover = cover_for(&meta, "").expect("no cover downloaded");
        println!("cover: {} bytes", cover.len());
        assert!(cover.len() > 20_000);

        // No match in the store: falls back to YouTube's words, with the
        // video thumbnail as cover.
        let hint = Hint {
            video_id: "fJ9rUzIMcZQ".into(),
            title: "Zzqx Nonexistent - Qqzx Song (Official Video)".into(),
            channel: "Nobody".into(),
            duration: Some(200),
            ..Hint::default()
        };
        let (meta, answered) = lookup(&hint, "ES");
        assert!(answered);
        assert_eq!(meta.source, "youtube");
        assert_eq!((meta.artist.as_str(), meta.title.as_str()), ("Zzqx Nonexistent", "Qqzx Song"));
        assert!(cover_for(&meta, "fJ9rUzIMcZQ").is_some(), "no thumbnail fallback");
    }

    /// Live, and the honest measure of the heuristics: real searches, the
    /// top result's real title/channel/description, and what came of each.
    /// Prints a table; asserts only that most of them found an album, since
    /// which videos YouTube ranks first changes by the week.
    ///
    /// ```text
    /// cargo test --release -- --ignored --nocapture real_searches
    /// ```
    #[test]
    #[ignore]
    fn real_searches_mostly_find_their_album() {
        let queries = [
            "queen bohemian rhapsody",
            "rosalia despecha",
            "bad bunny titi me pregunto",
            "daft punk get lucky",
            "shakira bzrp session 53",
            "nirvana smells like teen spirit",
            "coldplay yellow",
            "estopa como camaron",
        ];
        let mut with_album = 0;
        for query in queries {
            let hits = crate::youtube::search(query, 3).expect("search failed");
            let hit = hits.iter().find(|h| h.duration.is_some()).expect("no video");
            let description = crate::youtube::resolve(&hit.id).map(|r| r.description).unwrap_or_default();
            let hint = Hint {
                video_id: hit.id.clone(),
                title: hit.title.clone(),
                channel: hit.channel.clone(),
                duration: hit.duration,
                description,
                ..Hint::default()
            };
            let (meta, answered) = lookup(&hint, "ES");
            assert!(answered, "iTunes did not answer");
            if !meta.album.is_empty() {
                with_album += 1;
            }
            println!(
                "{:<60} | {:<20} -> [{}] {} - {} - {} ({})",
                hit.title.chars().take(60).collect::<String>(),
                hit.channel.chars().take(20).collect::<String>(),
                meta.source, meta.artist, meta.album, meta.title, meta.date
            );
        }
        assert!(with_album * 2 >= queries.len(), "only {with_album} of {} found an album", queries.len());
    }
}
