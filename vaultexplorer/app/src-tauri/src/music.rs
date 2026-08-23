//! Music: the library the phone actually browses, how often each track was
//! played, and filling in what the tags do not say.
//!
//! Three things live here, and they are here together because they all
//! answer the same question -- "what is this file, really?":
//!
//! 1. [`music_library`] walks a folder for audio and reads each file's own
//!    tags (via `lofty`, already a dependency), so the list can show
//!    "Title -- Artist" instead of `03_track.mp3`, group by folder, and sort
//!    by how often something was played.
//! 2. Play counts, kept in one small JSON file in the app's data directory.
//!    Not in the browser's storage: a music library outlives a WebView's
//!    idea of local storage, and clearing app data is not the same gesture
//!    as forgetting what you listen to.
//! 3. [`update_music_tags`] looks a track up on MusicBrainz, pulls the cover
//!    from the Cover Art Archive, and **writes the result into the file**.
//!    That is the difference from "Reorganize Music" (`musicorg.rs`), which
//!    moves files into `Artist/Year - Album` and leaves their tags alone:
//!    this one leaves the file where it is and fixes what is inside it.
//!
//! Nothing here throws a file away or renames one. The worst case for a
//! wrong guess is a tag the user can overwrite -- which is why a lookup that
//! finds nothing is reported as "left alone" rather than guessed at.

use crate::errmap::ToStringErr;
use crate::progress::{ProgressEvent, ProgressReporter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::ipc::Channel;
use tauri::Manager;

/// Extensions worth walking into. Deliberately the same list
/// `metadata.rs` uses, so "audio file" means one thing across the app.
const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "m4a", "mp4", "ogg", "opus", "wav", "aac", "ape", "wv", "mpc", "aiff", "aif",
];

fn is_audio(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .is_some_and(|ext| AUDIO_EXTS.contains(&ext.as_str()))
}

#[derive(Serialize, Clone)]
pub struct MusicTrack {
    pub path: String,
    /// File name, for when there is no title tag at all.
    pub name: String,
    /// Folder this track sits in, relative to the root that was scanned
    /// ("" for the root itself). This is what the folder switcher lists.
    pub folder: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<i32>,
    pub track_no: Option<u32>,
    pub duration_secs: Option<u64>,
    /// Whether the file carries embedded cover art, so the list can decide
    /// whether asking for it is worth a round trip.
    pub has_art: bool,
    pub plays: u32,
}

#[derive(Serialize, Deserialize, Default, Clone, Copy)]
struct PlayStat {
    plays: u32,
    /// Unix milliseconds, so "recently played" is answerable later without
    /// another migration.
    last_played_ms: u64,
}

type Plays = HashMap<String, PlayStat>;

fn plays_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).str_err()?;
    Ok(dir.join("music_plays.json"))
}

fn read_plays(app: &tauri::AppHandle) -> Plays {
    plays_path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_plays(app: &tauri::AppHandle, plays: &Plays) -> Result<(), String> {
    std::fs::write(plays_path(app)?, serde_json::to_string(plays).str_err()?).str_err()
}

fn collect_audio(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // Hidden folders are somebody else's business (.git, .Trash,
        // .thumbnails), and walking them turns a music scan into a disk scan.
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_audio(&path, out, depth - 1);
        } else if is_audio(&path) {
            out.push(path);
        }
    }
}

/// The year, whichever way the file spells it.
///
/// `lofty` has no year accessor: the value lives as text under `Year` or
/// `RecordingDate`, and a recording date is often a full "2007-03-05".
fn year_of(tag: &lofty::tag::Tag) -> Option<i32> {
    use lofty::prelude::ItemKey;
    tag.get_string(ItemKey::Year)
        .or_else(|| tag.get_string(ItemKey::RecordingDate))
        .and_then(|text| text.get(0..4))
        .and_then(|year| year.parse().ok())
}

/// What the file itself says, with the duration its own header reports.
fn read_track(path: &Path, root: &Path, plays: &Plays) -> MusicTrack {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::prelude::Accessor;

    let mut track = MusicTrack {
        path: path.to_string_lossy().to_string(),
        name: path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        folder: path
            .parent()
            .and_then(|parent| parent.strip_prefix(root).ok())
            .map(|rel| rel.to_string_lossy().to_string())
            .unwrap_or_default(),
        title: None,
        artist: None,
        album: None,
        year: None,
        track_no: None,
        duration_secs: None,
        has_art: false,
        plays: plays.get(&path.to_string_lossy().to_string()).map(|s| s.plays).unwrap_or(0),
    };

    // A file whose tags cannot be read is still a playable track: it just
    // shows its file name. That matters on a phone full of downloads.
    if let Ok(tagged) = lofty::probe::Probe::open(path).and_then(|p| p.read()) {
        track.duration_secs = Some(tagged.properties().duration().as_secs());
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            track.title = tag.title().map(|t| t.to_string()).filter(|t| !t.trim().is_empty());
            track.artist = tag.artist().map(|t| t.to_string()).filter(|t| !t.trim().is_empty());
            track.album = tag.album().map(|t| t.to_string()).filter(|t| !t.trim().is_empty());
            track.year = year_of(tag);
            track.track_no = tag.track();
            track.has_art = !tag.pictures().is_empty();
        }
    }
    track
}

/// Every audio file under `root`, tags read, play counts attached.
#[tauri::command]
pub async fn music_library(app: tauri::AppHandle, root: String) -> Result<Vec<MusicTrack>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root_path = PathBuf::from(&root);
        if !root_path.is_dir() {
            return Err(format!("Not a folder: {root}"));
        }
        let mut files = Vec::new();
        // Deep enough for Artist/Album/Disc, shallow enough that pointing
        // this at a home directory does not read the whole disk.
        collect_audio(&root_path, &mut files, 4);
        files.sort();
        let plays = read_plays(&app);
        Ok(files.iter().map(|f| read_track(f, &root_path, &plays)).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Count a play. Called when a track actually starts, not when it is
/// queued -- "most played" should mean what it says.
#[tauri::command]
pub fn music_played(app: tauri::AppHandle, path: String) -> Result<u32, String> {
    let mut plays = read_plays(&app);
    let entry = plays.entry(path).or_default();
    entry.plays = entry.plays.saturating_add(1);
    entry.last_played_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let count = entry.plays;
    write_plays(&app, &plays)?;
    Ok(count)
}

/// The embedded cover, as a data URL the page can put in an `<img>`.
///
/// A data URL rather than a file the media server hands out: cover art is
/// tens of kilobytes and belongs to the tag, not to a path -- and this way
/// there is nothing to clean up afterwards.
#[tauri::command]
pub async fn music_art(path: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use base64::Engine;
        use lofty::file::TaggedFileExt;

        let file = PathBuf::from(&path);
        let Ok(tagged) = lofty::probe::Probe::open(&file).and_then(|p| p.read()) else {
            return Ok(None);
        };
        let picture = tagged
            .primary_tag()
            .or_else(|| tagged.first_tag())
            .and_then(|tag| tag.pictures().first().cloned());
        let Some(picture) = picture else { return Ok(None) };
        let mime = picture
            .mime_type()
            .map(|m| m.to_string())
            .unwrap_or_else(|| "image/jpeg".to_string());
        let encoded = base64::engine::general_purpose::STANDARD.encode(picture.data());
        Ok(Some(format!("data:{mime};base64,{encoded}")))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- filling in the tags ---------------------------------------------

/// What MusicBrainz knows about a recording.
///
/// `releases` is the reason this is not just `musicorg`'s `TrackInfo`: cover
/// art is keyed on a *release*, and a recording usually belongs to several
/// (the original album, a compilation, a regional pressing). Only some of
/// them have a cover in the archive, so the candidates are all kept and
/// tried in turn.
#[derive(Default, Clone)]
pub(crate) struct Found {
    pub artist: Option<String>,
    pub album: Option<String>,
    pub title: Option<String>,
    pub year: Option<i32>,
    pub track_no: Option<u32>,
    pub releases: Vec<String>,
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        // MusicBrainz blocks the default user-agent outright, and asks
        // projects to identify themselves. The Cover Art Archive is the same
        // service, so one client does for both.
        .user_agent("VaultExplorer/1.0 (https://github.com/lucasmoy-dev/vaultexplorer)")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .str_err()
}

/// A MusicBrainz GET, with its two failure modes told apart.
///
/// The service answers "the web server is currently busy" with a 200 and a
/// JSON `error` field, which read as "no match" until this existed -- a
/// misleading thing to show a user whose files are perfectly identifiable.
/// One retry after a pause, because busy is usually momentary.
fn mb_get(http: &reqwest::blocking::Client, url: &str) -> Result<serde_json::Value, String> {
    let mut last = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(2500));
        }
        match http.get(url).send() {
            Ok(response) => match response.json::<serde_json::Value>() {
                Ok(body) => match body["error"].as_str() {
                    Some(error) => last = error.to_string(),
                    None => return Ok(body),
                },
                Err(error) => last = error.to_string(),
            },
            Err(error) => last = error.to_string(),
        }
    }
    Err(format!("MusicBrainz: {last}"))
}

/// Ask MusicBrainz about one recording.
pub(crate) fn lookup(artist: Option<&str>, title: &str) -> Result<Found, String> {
    let mut base = format!("recording:\"{}\"", title.replace('"', ""));
    if let Some(a) = artist {
        base.push_str(&format!(" AND artist:\"{}\"", a.replace('"', "")));
    }
    let http = client()?;

    // Two queries, in this order, because a plain title search is a trap:
    // ask MusicBrainz for "Smells Like Teen Spirit" and all ten top hits are
    // bootleg live recordings, every one scoring 100. Filtering to official,
    // non-live, non-compilation releases finds the studio track people
    // actually have -- but it finds *nothing* for a track that was never on
    // an official release, so the unfiltered search stays as the fallback.
    let filtered = format!(
        "{base} AND status:official AND -secondarytype:live AND -secondarytype:compilation"
    );
    let mut chosen: Option<serde_json::Value> = None;
    for query in [filtered, base] {
        let url = format!(
            "https://musicbrainz.org/ws/2/recording?query={}&fmt=json&limit=10",
            urlencode(&query)
        );
        let body = mb_get(&http, &url)?;
        if let Some(best) = body["recordings"]
            .as_array()
            .and_then(|list| list.iter().max_by_key(|rec| rank(rec)))
        {
            chosen = Some(best.clone());
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }
    let rec = &chosen.ok_or("sin coincidencias en MusicBrainz")?;

    // The search result carries an abbreviated release list -- often without
    // dates, which is why the year came back empty. Ask about the recording
    // itself for the full one. Second request, so the same one-per-second
    // courtesy applies.
    let detail = rec["id"].as_str().and_then(|id| {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let url = format!(
            "https://musicbrainz.org/ws/2/recording/{id}?inc=releases+release-groups&fmt=json"
        );
        mb_get(&http, &url).ok()
    });
    let releases = detail
        .as_ref()
        .and_then(|d| d["releases"].as_array().cloned())
        .filter(|list| !list.is_empty())
        .or_else(|| rec["releases"].as_array().cloned())
        .unwrap_or_default();

    // An album is a better answer than a compilation that happens to be
    // listed first, and the earliest date is the year people mean by "when
    // is this song from".
    let album_first = releases
        .iter()
        .find(|r| {
            r["status"].as_str() == Some("Official")
                && r["release-group"]["primary-type"].as_str() == Some("Album")
        })
        .or_else(|| releases.iter().find(|r| r["release-group"]["primary-type"].as_str() == Some("Album")))
        .or_else(|| releases.first());
    // The year people mean is when the song came out, not when the reissue
    // they happen to own was pressed -- a 2021 anniversary edition of
    // Nevermind is still a 1991 song. A release group's first-release-date
    // says exactly that, and the release's own date is the fallback.
    let year = releases
        .iter()
        .filter_map(|r| {
            r["release-group"]["first-release-date"]
                .as_str()
                .or_else(|| r["date"].as_str())
        })
        .filter_map(|date| date.get(0..4))
        .filter_map(|year| year.parse::<i32>().ok())
        .filter(|year| *year > 1900)
        .min();

    Ok(Found {
        artist: rec["artist-credit"][0]["name"].as_str().map(str::to_string),
        album: album_first.and_then(|r| r["title"].as_str()).map(str::to_string),
        title: rec["title"].as_str().map(str::to_string),
        year,
        track_no: album_first
            .and_then(|r| r["media"][0]["track"][0]["number"].as_str())
            .and_then(|n| n.parse().ok()),
        // Official albums first, so the cover that gets embedded is the one
        // the user expects rather than a compilation's or a bootleg's.
        releases: {
            let mut ordered: Vec<&serde_json::Value> = releases.iter().collect();
            ordered.sort_by_key(|r| {
                let official = r["status"].as_str() == Some("Official");
                let album = r["release-group"]["primary-type"].as_str() == Some("Album");
                match (official, album) {
                    (true, true) => 0,
                    (true, false) => 1,
                    (false, true) => 2,
                    (false, false) => 3,
                }
            });
            ordered
                .iter()
                .filter_map(|r| r["id"].as_str().map(str::to_string))
                .take(5)
                .collect()
        },
    })
}

/// The first of these releases that has a front cover.
fn any_cover(releases: &[String]) -> Option<Vec<u8>> {
    releases.iter().find_map(|release| cover_art(release))
}

/// How much a search hit looks like the recording someone actually has.
///
/// MusicBrainz's own `score` is about text similarity, so a bootleg whose
/// title matches exactly outranks the album version. These adjustments say
/// what a music library means by "the right one": released officially, on an
/// album, and not a live recording unless that is what was asked for.
fn rank(rec: &serde_json::Value) -> i64 {
    let score = rec["score"].as_i64().unwrap_or(0);
    let releases = rec["releases"].as_array().cloned().unwrap_or_default();
    let official_album = releases.iter().any(|r| {
        r["status"].as_str() == Some("Official")
            && r["release-group"]["primary-type"].as_str() == Some("Album")
    });
    let any_official = releases.iter().any(|r| r["status"].as_str() == Some("Official"));
    let no_releases = releases.is_empty();

    // MusicBrainz spells out what a recording is in its disambiguation
    // ("live, 1991-11-25: ...", "official music video", "demo"). A library
    // full of studio tracks wants none of those, and an empty
    // disambiguation is the studio version saying nothing about itself.
    let aside = rec["disambiguation"].as_str().unwrap_or("").to_lowercase();
    let variant = ["live", "video", "demo", "remix", "instrumental", "karaoke", "rehearsal"]
        .iter()
        .any(|marker| aside.contains(marker));

    score
        + if official_album { 60 } else { 0 }
        + if any_official { 20 } else { 0 }
        + if aside.is_empty() { 40 } else { 0 }
        - if variant { 90 } else { 0 }
        - if rec["video"].as_bool().unwrap_or(false) { 60 } else { 0 }
        - if no_releases { 50 } else { 0 }
}

/// The front cover for a release, at a size worth embedding (500px is what
/// every phone player shows; the originals are often several megabytes and
/// would bloat every file in an album).
fn cover_art(release_id: &str) -> Option<Vec<u8>> {
    let url = format!("https://coverartarchive.org/release/{release_id}/front-500");
    let response = client().ok()?.get(&url).send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().ok()?.to_vec();
    // Sanity: an HTML error page is not a picture, and embedding one would
    // leave every player showing a broken image.
    (bytes.len() > 1024 && (bytes.starts_with(&[0xff, 0xd8]) || bytes.starts_with(b"\x89PNG")))
        .then_some(bytes)
}

#[derive(Serialize, Clone)]
pub struct TagUpdate {
    pub path: String,
    pub name: String,
    /// What changed, in the words the UI shows: "título, artista, portada".
    pub changed: Vec<String>,
    /// Why nothing changed, when nothing did.
    pub skipped: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<i32>,
}

/// Filenames are the fallback question to ask with: most are some
/// arrangement of artist, title and a leading track number.
fn guess_from_filename(path: &Path) -> (Option<String>, String) {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let without_number = stem
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '-' || c == ' ')
        .to_string();
    match without_number.split_once(" - ") {
        Some((artist, title)) => (Some(artist.trim().to_string()), title.trim().to_string()),
        None => (None, without_number),
    }
}

/// Write what was found into the file, and say what changed.
///
/// Only *missing* fields are filled in by default: a tag the user (or their
/// ripper) already set is better evidence than a search result, and
/// overwriting it is how a library gets quietly wrong.
fn apply(path: &Path, found: &Found, art: Option<Vec<u8>>) -> Result<Vec<String>, String> {
    use lofty::config::{ParseOptions, WriteOptions};
    use lofty::file::{BoundTaggedFile, TaggedFileExt};
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::prelude::{Accessor, ItemKey};
    use lofty::tag::Tag;

    let file = std::fs::OpenOptions::new().read(true).write(true).open(path).str_err()?;
    let mut bound = BoundTaggedFile::read_from(file, ParseOptions::new()).str_err()?;
    let tag_type = bound.primary_tag_type();
    if bound.primary_tag_mut().is_none() {
        bound.insert_tag(Tag::new(tag_type));
    }
    let tag = bound.primary_tag_mut().ok_or("this format takes no tags")?;

    let mut changed = Vec::new();
    if tag.title().is_none_or(|t| t.trim().is_empty()) {
        if let Some(title) = &found.title {
            tag.set_title(title.clone());
            changed.push("título".to_string());
        }
    }
    if tag.artist().is_none_or(|t| t.trim().is_empty()) {
        if let Some(artist) = &found.artist {
            tag.set_artist(artist.clone());
            changed.push("artista".to_string());
        }
    }
    if tag.album().is_none_or(|t| t.trim().is_empty()) {
        if let Some(album) = &found.album {
            tag.set_album(album.clone());
            changed.push("álbum".to_string());
        }
    }
    if year_of(tag).is_none() {
        if let Some(year) = found.year {
            // Two keys on purpose: players are split between reading the
            // year and reading the recording date.
            tag.insert_text(ItemKey::Year, year.to_string());
            tag.insert_text(ItemKey::RecordingDate, year.to_string());
            changed.push("año".to_string());
        }
    }
    if tag.track().is_none() {
        if let Some(no) = found.track_no {
            tag.set_track(no);
            changed.push("nº de pista".to_string());
        }
    }
    if tag.pictures().is_empty() {
        if let Some(bytes) = art {
            let mime = if bytes.starts_with(b"\x89PNG") { MimeType::Png } else { MimeType::Jpeg };
            tag.push_picture(
                Picture::unchecked(bytes)
                    .pic_type(PictureType::CoverFront)
                    .mime_type(mime)
                    .build(),
            );
            changed.push("portada".to_string());
        }
    }

    if changed.is_empty() {
        return Ok(changed);
    }
    bound.save(WriteOptions::default()).str_err()?;
    Ok(changed)
}

/// Walk `root`, look every track up, and fill in the tags it is missing.
#[tauri::command]
pub async fn update_music_tags(
    root: String,
    channel: Channel<ProgressEvent>,
    registry: tauri::State<'_, crate::ops::OpRegistry>,
) -> Result<Vec<TagUpdate>, String> {
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let root_path = PathBuf::from(&root);
        let mut files = Vec::new();
        collect_audio(&root_path, &mut files, 4);
        files.sort();
        let reporter = ProgressReporter::new_cancellable(channel, files.len().max(1) as u64, cancel);

        // Album art is per release, and an album is a run of consecutive
        // files: fetching it once per album instead of once per track turns
        // twelve downloads into one.
        let mut cached_art: Option<(String, Option<Vec<u8>>)> = None;
        let mut updates = Vec::new();

        for (index, file) in files.iter().enumerate() {
            if reporter.is_cancelled() {
                break;
            }
            reporter.report(index as u64);
            let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let mut update = TagUpdate {
                path: file.to_string_lossy().to_string(),
                name: name.clone(),
                changed: Vec::new(),
                skipped: None,
                title: None,
                artist: None,
                album: None,
                year: None,
            };

            let existing = read_track(file, &root_path, &HashMap::new());
            let (guess_artist, guess_title) = guess_from_filename(file);
            let artist = existing.artist.clone().or(guess_artist);
            let title = existing.title.clone().unwrap_or(guess_title);
            if title.trim().is_empty() {
                update.skipped = Some("no hay nada por donde buscarlo".to_string());
                updates.push(update);
                continue;
            }

            match lookup(artist.as_deref(), &title) {
                Ok(found) => {
                    let art = if existing.has_art || found.releases.is_empty() {
                        // Already has a cover, or nothing to ask about.
                        None
                    } else {
                        let key = found.releases[0].clone();
                        if cached_art.as_ref().map(|(id, _)| *id != key).unwrap_or(true) {
                            cached_art = Some((key, any_cover(&found.releases)));
                        }
                        cached_art.as_ref().and_then(|(_, bytes)| bytes.clone())
                    };
                    update.title = found.title.clone();
                    update.artist = found.artist.clone();
                    update.album = found.album.clone();
                    update.year = found.year;
                    match apply(file, &found, art) {
                        Ok(changed) if changed.is_empty() => {
                            update.skipped = Some("ya estaba completo".to_string());
                        }
                        Ok(changed) => update.changed = changed,
                        Err(error) => update.skipped = Some(error),
                    }
                }
                Err(error) => {
                    let busy = error.starts_with("MusicBrainz:");
                    update.skipped = Some(error);
                    updates.push(update);
                    if busy {
                        // Hammering a service that just said it is busy is
                        // how an app's user-agent gets blocked. Stop, and let
                        // the user see how far it got.
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1100));
                    continue;
                }
            }
            updates.push(update);

            // MusicBrainz asks for at most one request a second, and a
            // library scan is exactly the case that would otherwise get this
            // app's user-agent blocked for everyone.
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }

        reporter.finish();
        updates
    })
    .await
    .map_err(|e| e.to_string());
    registry.finish(op_id);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_audio_files_are_collected() {
        let dir = std::env::temp_dir().join(format!("ve-music-{}", std::process::id()));
        let nested = dir.join("Album");
        std::fs::create_dir_all(&nested).expect("dirs");
        for name in ["a.mp3", "b.txt", "c.FLAC"] {
            std::fs::write(dir.join(name), b"x").expect("write");
        }
        std::fs::write(nested.join("d.m4a"), b"x").expect("write");
        // Hidden folders are skipped: a music scan should not walk .git.
        let hidden = dir.join(".cache");
        std::fs::create_dir_all(&hidden).expect("hidden");
        std::fs::write(hidden.join("e.mp3"), b"x").expect("write");

        let mut found = Vec::new();
        collect_audio(&dir, &mut found, 4);
        let mut names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.mp3", "c.FLAC", "d.m4a"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The folder a track reports is relative to the scanned root, because
    /// that is what the folder switcher shows -- an absolute path would put
    /// the whole of `/home/user/Music` in every chip.
    #[test]
    fn a_track_knows_which_folder_it_is_in() {
        let dir = std::env::temp_dir().join(format!("ve-music-folder-{}", std::process::id()));
        let nested = dir.join("Pink Floyd").join("1973 - Dark Side");
        std::fs::create_dir_all(&nested).expect("dirs");
        let file = nested.join("01 - Speak to Me.mp3");
        std::fs::write(&file, b"not really an mp3").expect("write");

        let track = read_track(&file, &dir, &HashMap::new());
        assert_eq!(track.folder, "Pink Floyd/1973 - Dark Side");
        assert_eq!(track.name, "01 - Speak to Me.mp3");
        // Unreadable tags must not lose the track: it still plays, showing
        // its file name.
        assert_eq!(track.title, None);
        assert_eq!(track.plays, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_filename_is_a_last_resort_question() {
        let (artist, title) = guess_from_filename(Path::new("/m/03 - Radiohead - Creep.mp3"));
        assert_eq!(artist.as_deref(), Some("Radiohead"));
        assert_eq!(title, "Creep");

        let (none, only) = guess_from_filename(Path::new("/m/Creep.flac"));
        assert_eq!(none, None);
        assert_eq!(only, "Creep");
    }

    /// A WAV with a real header, so `lofty` will open it and write to it.
    /// Hand-built rather than encoded: this test is about tags, and a
    /// dependency on ffmpeg (or on an encoder) would make it skip itself on
    /// exactly the machines where it matters.
    fn silent_wav(path: &Path) {
        let samples: Vec<u8> = vec![0; 4000];
        let data_len = samples.len() as u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&44100u32.to_le_bytes());
        wav.extend_from_slice(&88200u32.to_le_bytes()); // byte rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend_from_slice(&samples);
        std::fs::write(path, wav).expect("write wav");
    }

    /// The half that actually touches the user's files: what a lookup found
    /// ends up *inside* the file, cover art included, and reads back.
    #[test]
    fn what_was_found_is_written_into_the_file() {
        use lofty::file::TaggedFileExt;
        use lofty::prelude::Accessor;

        let dir = std::env::temp_dir().join(format!("ve-music-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("track.wav");
        silent_wav(&file);

        // A JPEG as far as anything that checks is concerned.
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe0];
        jpeg.extend(std::iter::repeat(0x42).take(2000));

        let found = Found {
            artist: Some("Nirvana".to_string()),
            album: Some("Nevermind".to_string()),
            title: Some("Smells Like Teen Spirit".to_string()),
            year: Some(1991),
            track_no: Some(1),
            releases: vec!["release-id".to_string()],
        };
        let changed = apply(&file, &found, Some(jpeg.clone())).expect("apply");
        assert!(changed.contains(&"título".to_string()), "{changed:?}");
        assert!(changed.contains(&"portada".to_string()), "{changed:?}");

        let tagged = lofty::probe::Probe::open(&file).and_then(|p| p.read()).expect("read back");
        let tag = tagged.primary_tag().or_else(|| tagged.first_tag()).expect("no tag written");
        assert_eq!(tag.title().as_deref(), Some("Smells Like Teen Spirit"));
        assert_eq!(tag.artist().as_deref(), Some("Nirvana"));
        assert_eq!(tag.album().as_deref(), Some("Nevermind"));
        assert_eq!(year_of(tag), Some(1991));
        assert_eq!(tag.track(), Some(1));
        assert_eq!(tag.pictures().first().map(|p| p.data().len()), Some(jpeg.len()));

        // Run again: nothing is missing now, so nothing is touched. A second
        // pass over a library must not rewrite every file.
        let second = apply(&file, &found, Some(jpeg)).expect("apply again");
        assert!(second.is_empty(), "rewrote a complete file: {second:?}");

        // And what the library view reports is what was just written.
        let track = read_track(&file, &dir, &HashMap::new());
        assert_eq!(track.title.as_deref(), Some("Smells Like Teen Spirit"));
        assert_eq!(track.year, Some(1991));
        assert!(track.has_art);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live (network): MusicBrainz identifies a famous recording, and the
    /// Cover Art Archive has a front cover for its release. `#[ignore]`d.
    #[test]
    #[ignore]
    fn musicbrainz_and_the_cover_art_archive_answer() {
        let found = match lookup(Some("Nirvana"), "Smells Like Teen Spirit") {
            Ok(found) => found,
            Err(busy) if busy.starts_with("MusicBrainz:") => {
                println!("MusicBrainz is refusing requests right now: {busy}");
                return;
            }
            Err(other) => panic!("lookup: {other}"),
        };
        println!(
            "{:?} - {:?} ({:?}), {} candidate releases",
            found.artist, found.title, found.year, found.releases.len()
        );
        assert!(found.artist.is_some() && found.title.is_some());
        assert!(found.year.is_some(), "no year came back");
        assert!(!found.releases.is_empty(), "no releases to ask for a cover");
        let art = any_cover(&found.releases).expect("no cover on any candidate release");
        println!("cover: {} bytes from {} candidates", art.len(), found.releases.len());
        assert!(art.len() > 4096);
    }
}
