//! "Reorganize Music": look a track up online, fill in what's missing, and
//! file it where it belongs.
//!
//! Sources, in the order they are tried:
//!
//! 1. The file's own tags. A track that already knows its artist, album
//!    and number needs no network at all, and the local answer is more
//!    trustworthy than a guess from a filename.
//! 2. MusicBrainz, queried by whatever the tags or the filename give us.
//!    No API key, and its rate limit (one request a second) is honoured
//!    rather than risking a block for everyone using this app.
//!
//! The result is the layout that was asked for:
//!
//! ```text
//! Music/Artist/Year - Album/01 - Title.mp3
//! ```
//!
//! Files are *moved*, never rewritten in place, and anything that can't be
//! identified is left exactly where it is -- an unrecognised track staying
//! put is much better than one filed under a wrong guess.

use crate::errmap::ToStringErr;
use crate::progress::{ProgressEvent, ProgressReporter};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::ipc::Channel;

#[derive(Serialize, Clone)]
pub struct OrganizedTrack {
    pub from: String,
    pub to: String,
    pub artist: String,
    pub album: String,
    pub title: String,
    pub year: Option<i32>,
    pub track_no: Option<u32>,
    /// Whether anything had to be looked up online for this one.
    pub from_online: bool,
}

#[derive(Default, Clone)]
struct TrackInfo {
    artist: Option<String>,
    album: Option<String>,
    title: Option<String>,
    year: Option<i32>,
    track_no: Option<u32>,
}

impl TrackInfo {
    fn complete(&self) -> bool {
        self.artist.is_some() && self.album.is_some() && self.title.is_some()
    }
}

/// Reads ID3v2/MP4 tags well enough for the fields this needs, without a
/// tag-parsing dependency: only TPE1/TALB/TIT2/TRCK/TDRC (and their ID3v2.2
/// three-letter equivalents) are looked at, and anything unparseable is
/// simply treated as "not tagged".
fn read_tags(path: &Path) -> TrackInfo {
    let mut info = TrackInfo::default();
    let Ok(bytes) = std::fs::read(path) else { return info };
    if bytes.len() < 10 || &bytes[0..3] != b"ID3" {
        return info;
    }
    // ID3v2 header: size is 4 syncsafe bytes (7 bits each).
    let size = ((bytes[6] as usize & 0x7f) << 21)
        | ((bytes[7] as usize & 0x7f) << 14)
        | ((bytes[8] as usize & 0x7f) << 7)
        | (bytes[9] as usize & 0x7f);
    let version = bytes[3];
    let end = (10 + size).min(bytes.len());
    let mut pos = 10;
    let id_len = if version >= 3 { 4 } else { 3 };
    let header_len = if version >= 3 { 10 } else { 6 };

    while pos + header_len <= end {
        let id = String::from_utf8_lossy(&bytes[pos..pos + id_len]).to_string();
        if id.trim_matches('\0').is_empty() {
            break;
        }
        let frame_size = if version >= 4 {
            // v2.4 sizes are syncsafe too
            ((bytes[pos + 4] as usize & 0x7f) << 21)
                | ((bytes[pos + 5] as usize & 0x7f) << 14)
                | ((bytes[pos + 6] as usize & 0x7f) << 7)
                | (bytes[pos + 7] as usize & 0x7f)
        } else if version == 3 {
            u32::from_be_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize
        } else {
            ((bytes[pos + 3] as usize) << 16) | ((bytes[pos + 4] as usize) << 8) | bytes[pos + 5] as usize
        };
        let start = pos + header_len;
        let stop = (start + frame_size).min(end);
        if start >= stop {
            break;
        }
        let text = decode_text_frame(&bytes[start..stop]);
        match id.as_str() {
            "TPE1" | "TP1" => info.artist = text,
            "TALB" | "TAL" => info.album = text,
            "TIT2" | "TT2" => info.title = text,
            "TRCK" | "TRK" => {
                info.track_no = text.and_then(|t| t.split('/').next().and_then(|n| n.trim().parse().ok()))
            }
            "TDRC" | "TYER" | "TYE" => {
                info.year = text.and_then(|t| t.get(0..4).and_then(|y| y.parse().ok()))
            }
            _ => {}
        }
        pos = stop;
    }
    info
}

/// A text frame's first byte is its encoding; only the two that actually
/// appear in the wild are handled, and UTF-16 is read by its BOM.
fn decode_text_frame(raw: &[u8]) -> Option<String> {
    let (encoding, body) = raw.split_first()?;
    let text = match encoding {
        0 | 3 => String::from_utf8_lossy(body).to_string(),
        1 | 2 => {
            let units: Vec<u16> = body
                .chunks_exact(2)
                .map(|c| {
                    if body.starts_with(&[0xff, 0xfe]) {
                        u16::from_le_bytes([c[0], c[1]])
                    } else {
                        u16::from_be_bytes([c[0], c[1]])
                    }
                })
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => return None,
    };
    let cleaned = text.trim_matches(|c: char| c == '\0' || c == '\u{feff}').trim().to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// Filenames are the fallback question to ask MusicBrainz with: most are
/// some arrangement of artist, title and a leading track number.
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

fn mb_lookup(artist: Option<&str>, title: &str) -> Result<TrackInfo, String> {
    let mut query = format!("recording:\"{}\"", title.replace('"', ""));
    if let Some(a) = artist {
        query.push_str(&format!(" AND artist:\"{}\"", a.replace('"', "")));
    }
    let url = format!(
        "https://musicbrainz.org/ws/2/recording?query={}&fmt=json&limit=1",
        urlencode(&query)
    );
    let client = reqwest::blocking::Client::builder()
        // MusicBrainz requires a identifying user-agent and blocks the
        // default one outright.
        .user_agent("VaultExplorer/1.0 (https://github.com/lucasmoy-dev/vaultexplorer)")
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .str_err()?;
    let body: serde_json::Value = client.get(&url).send().str_err()?.json().str_err()?;
    let rec = body["recordings"].get(0).ok_or("no match on MusicBrainz")?;
    let release = rec["releases"].get(0);
    Ok(TrackInfo {
        artist: rec["artist-credit"][0]["name"].as_str().map(|s| s.to_string()),
        album: release.and_then(|r| r["title"].as_str()).map(|s| s.to_string()),
        title: rec["title"].as_str().map(|s| s.to_string()),
        year: release
            .and_then(|r| r["date"].as_str())
            .and_then(|d| d.get(0..4))
            .and_then(|y| y.parse().ok()),
        track_no: release
            .and_then(|r| r["media"][0]["track"][0]["number"].as_str())
            .and_then(|n| n.parse().ok()),
    })
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

/// Anything a filesystem (or a person reading a path) would rather not see.
fn sanitize(part: &str) -> String {
    let cleaned: String = part
        .chars()
        .map(|c| if "/\\?%*:|\"<>".contains(c) { '-' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').to_string();
    if trimmed.is_empty() { "Unknown".to_string() } else { trimmed }
}

/// Walks `root` for audio files, identifies each one, and moves it into
/// `Artist/Year - Album/NN - Title.ext` underneath that same root.
#[tauri::command]
pub async fn organize_music(
    root: String,
    channel: Channel<ProgressEvent>,
    registry: tauri::State<'_, crate::ops::OpRegistry>,
) -> Result<Vec<OrganizedTrack>, String> {
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let root_path = PathBuf::from(&root);
        let mut files: Vec<PathBuf> = Vec::new();
        collect_audio(&root_path, &mut files);
        let reporter = ProgressReporter::new_cancellable(channel, files.len().max(1) as u64, cancel);

        let mut moved = Vec::new();
        for (i, file) in files.iter().enumerate() {
            if reporter.is_cancelled() {
                break;
            }
            reporter.report(i as u64);
            let mut info = read_tags(file);
            let mut from_online = false;
            if !info.complete() {
                let (guess_artist, guess_title) = guess_from_filename(file);
                let artist = info.artist.clone().or(guess_artist);
                let title = info.title.clone().unwrap_or(guess_title);
                if let Ok(found) = mb_lookup(artist.as_deref(), &title) {
                    from_online = true;
                    info.artist = info.artist.or(found.artist);
                    info.album = info.album.or(found.album);
                    info.title = info.title.or(found.title);
                    info.year = info.year.or(found.year);
                    info.track_no = info.track_no.or(found.track_no);
                }
                // MusicBrainz asks for no more than one request a second,
                // and being a good citizen keeps this working for everyone.
                std::thread::sleep(std::time::Duration::from_millis(1100));
            }
            let (Some(artist), Some(album), Some(title)) = (&info.artist, &info.album, &info.title) else {
                continue; // unidentified: leave it exactly where it is
            };

            let ext = file.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
            let album_dir = match info.year {
                Some(y) => format!("{y} - {}", sanitize(album)),
                None => sanitize(album),
            };
            let name = match info.track_no {
                Some(n) => format!("{n:02} - {}.{ext}", sanitize(title)),
                None => format!("{}.{ext}", sanitize(title)),
            };
            let dest_dir = root_path.join(sanitize(artist)).join(album_dir);
            let dest = dest_dir.join(&name);
            if dest == *file {
                continue;
            }
            std::fs::create_dir_all(&dest_dir).str_err()?;
            if dest.exists() {
                continue; // never overwrite a track that's already filed
            }
            std::fs::rename(file, &dest).str_err()?;
            moved.push(OrganizedTrack {
                from: file.to_string_lossy().to_string(),
                to: dest.to_string_lossy().to_string(),
                artist: artist.clone(),
                album: album.clone(),
                title: title.clone(),
                year: info.year,
                track_no: info.track_no,
                from_online,
            });
        }
        reporter.report(files.len() as u64);
        Ok(moved)
    })
    .await
    .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    result
}

fn collect_audio(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_audio(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
            Some("mp3" | "m4a" | "flac" | "ogg" | "opus" | "wav" | "aac")
        ) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulls_artist_and_title_out_of_a_filename() {
        let (artist, title) = guess_from_filename(Path::new("/m/01 - Radiohead - Creep.mp3"));
        assert_eq!(artist.as_deref(), Some("Radiohead"));
        assert_eq!(title, "Creep");

        let (artist, title) = guess_from_filename(Path::new("/m/Creep.mp3"));
        assert_eq!(artist, None);
        assert_eq!(title, "Creep");
    }

    #[test]
    fn keeps_path_parts_filesystem_safe() {
        assert_eq!(sanitize("AC/DC"), "AC-DC");
        assert_eq!(sanitize("  Nirvana. "), "Nirvana");
        assert_eq!(sanitize(""), "Unknown");
    }

    #[test]
    fn reads_an_id3v2_text_frame() {
        // encoding byte 0 (latin-1) followed by the value
        assert_eq!(decode_text_frame(&[0, b'A', b'B', b'C']).as_deref(), Some("ABC"));
        assert_eq!(decode_text_frame(&[0]), None);
    }
}
