//! Turning a video title into a filename.
//!
//! This is the whole point of the app for anyone who has downloaded music
//! before: files named after the video, not `videoplayback.m4a` or an
//! eleven-character id. Which means the title has to survive intact
//! wherever it lands -- and phone storage is not a friendly place for
//! arbitrary text:
//!
//! * `/` is a path separator, and the shared storage on Android is
//!   ultimately FAT-derived, so `\ : * ? " < > |` are rejected too. All of
//!   them appear in real titles ("AC/DC", "Song (Live?)", `"quoted"`).
//! * A name ending in `.` or a space is silently mangled or refused.
//! * Emoji, accents and CJK are fine and stay -- Android has no problem
//!   with them, and stripping them would butcher most non-English titles.
//! * Titles get long. A cap keeps the whole path under the filesystem's
//!   255-byte-per-component limit even when the characters are multi-byte.

/// Characters no filename may contain on Android's shared storage.
const FORBIDDEN: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// Bytes, not chars: the limit filesystems enforce is on bytes, and a
/// Japanese or emoji-laden title reaches it three times faster than an
/// English one. 120 leaves room for " (1).mp3" and a directory prefix.
const MAX_BYTES: usize = 120;

/// A safe base name (no extension) for `title`.
pub fn file_stem(title: &str) -> String {
    let cleaned = clean(title, MAX_BYTES);
    if cleaned.is_empty() {
        "video".to_string()
    } else {
        cleaned
    }
}

/// `title` made safe for one path component and capped at `max` bytes; empty
/// when nothing usable is left. The rules `file_stem` is built on, exposed so
/// the parts of an organised name can each be capped on their own.
pub fn clean(title: &str, max: usize) -> String {
    let replaced: String = title
        .chars()
        .map(|c| {
            if FORBIDDEN.contains(&c) {
                // A dash, not nothing: "AC/DC" should read "AC-DC", not
                // "ACDC".
                '-'
            } else if c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();

    // Collapse runs of whitespace, so a title with a line break in it does
    // not become a filename with three spaces.
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim().trim_end_matches('.').trim();
    let capped = truncate_bytes(trimmed, max);
    capped.trim().trim_end_matches('.').trim().to_string()
}

/// `Artist - Album - Song.mp3`, the name an organised MP3 gets, or
/// `Artist - Song.mp3` when the album is not known (a name with an empty
/// middle, "Artist -  - Song", helps nobody).
///
/// Each part is capped on its own rather than the whole being cut at the end:
/// a long album name must not be what eats the song title, which is the part
/// that tells two files apart. 50 + 60 + 80 bytes plus separators stays well
/// under the 255-byte component limit.
pub fn track_file_name(artist: &str, album: &str, title: &str, ext: &str) -> String {
    let parts: Vec<String> = [(artist, 50), (album, 60), (title, 80)]
        .iter()
        .map(|(text, max)| clean(text, *max))
        .filter(|part| !part.is_empty())
        .collect();
    let stem = if parts.is_empty() { "audio".to_string() } else { parts.join(" - ") };
    file_name_with(stem, ext)
}

/// The folder an organised MP3 goes in: the artist, made safe, or a fixed
/// name when there is none (a file must land *somewhere*).
pub fn artist_folder(artist: &str) -> String {
    let cleaned = clean(artist, 60);
    if cleaned.is_empty() {
        "Artista desconocido".to_string()
    } else {
        cleaned
    }
}

/// `file_stem` plus an extension.
pub fn file_name(title: &str, ext: &str) -> String {
    let ext = ext.trim().trim_start_matches('.');
    if ext.is_empty() {
        file_stem(title)
    } else {
        format!("{}.{}", file_stem(title), ext)
    }
}

fn file_name_with(stem: String, ext: &str) -> String {
    let ext = ext.trim().trim_start_matches('.');
    if ext.is_empty() { stem } else { format!("{stem}.{ext}") }
}

/// Cut to at most `max` bytes without splitting a character in half.
fn truncate_bytes(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    // Prefer cutting at a word boundary when one is close, so a truncated
    // title ends on a word rather than mid-word.
    let candidate = &text[..end];
    match candidate.rfind(' ') {
        Some(space) if space > max * 2 / 3 => &candidate[..space],
        _ => candidate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_separators_and_forbidden_characters_become_dashes() {
        assert_eq!(file_stem("AC/DC - Back In Black"), "AC-DC - Back In Black");
        assert_eq!(file_stem(r"Rock\Roll"), "Rock-Roll");
        assert_eq!(file_stem("Live? Yes: 1999"), "Live- Yes- 1999");
        assert_eq!(file_stem(r#"He said "hello""#), "He said -hello-");
        assert_eq!(file_stem("a<b>c|d*e"), "a-b-c-d-e");
    }

    #[test]
    fn accents_emoji_and_other_scripts_survive() {
        assert_eq!(file_stem("Canción de cuna 🎵"), "Canción de cuna 🎵");
        assert_eq!(file_stem("日本語のタイトル"), "日本語のタイトル");
        assert_eq!(file_stem("Français — été"), "Français — été");
    }

    #[test]
    fn whitespace_and_trailing_dots_are_cleaned_up() {
        assert_eq!(file_stem("  spaced   out  \n title "), "spaced out title");
        assert_eq!(file_stem("Ends with a dot."), "Ends with a dot");
        assert_eq!(file_stem("Ends with dots..."), "Ends with dots");
        assert_eq!(file_stem("\t\ttabs\tinside\t"), "tabs inside");
    }

    #[test]
    fn an_unusable_title_still_yields_a_filename() {
        assert_eq!(file_stem(""), "video");
        assert_eq!(file_stem("   "), "video");
        assert_eq!(file_stem("..."), "video");
        assert_eq!(file_stem("///"), "---"); // dashes are a real name
    }

    #[test]
    fn long_titles_are_capped_without_splitting_characters() {
        let long = "Официальное видео с очень длинным названием ".repeat(6);
        let stem = file_stem(&long);
        assert!(stem.len() <= MAX_BYTES, "{} bytes", stem.len());
        // Still valid UTF-8 (would have panicked on a bad slice) and does
        // not end mid-word or with a space.
        assert_eq!(stem.trim(), stem);
        assert!(stem.chars().count() > 20, "cut far too aggressively: {stem}");
    }

    #[test]
    fn extensions_are_appended_exactly_once() {
        assert_eq!(file_name("Song", "mp3"), "Song.mp3");
        assert_eq!(file_name("Song", ".mp3"), "Song.mp3");
        assert_eq!(file_name("Song.", "mp4"), "Song.mp4");
        assert_eq!(file_name("Song", ""), "Song");
    }

    #[test]
    fn an_organised_name_is_artist_album_song() {
        assert_eq!(
            track_file_name("Queen", "A Night At The Opera", "Bohemian Rhapsody", "mp3"),
            "Queen - A Night At The Opera - Bohemian Rhapsody.mp3"
        );
        // No album: two parts, not an empty middle.
        assert_eq!(track_file_name("ROSALÍA", "", "DESPECHÁ", "mp3"), "ROSALÍA - DESPECHÁ.mp3");
        assert_eq!(track_file_name("AC/DC", "Back in Black", "Hells Bells", "mp3"), "AC-DC - Back in Black - Hells Bells.mp3");
        assert_eq!(track_file_name("", "", "", "mp3"), "audio.mp3");
    }

    #[test]
    fn a_long_album_cannot_eat_the_song_title() {
        let name = track_file_name("Artist", &"Very Long Album Name ".repeat(10), "The Song", "mp3");
        assert!(name.ends_with(" - The Song.mp3"), "{name}");
        assert!(name.len() < 255, "{} bytes", name.len());
        let worst = track_file_name(&"ä".repeat(200), &"ö".repeat(200), &"ü".repeat(200), "mp3");
        assert!(worst.len() < 255, "{} bytes", worst.len());
    }

    #[test]
    fn the_artist_folder_is_safe_and_never_empty() {
        assert_eq!(artist_folder("AC/DC"), "AC-DC");
        assert_eq!(artist_folder("  "), "Artista desconocido");
    }

    #[test]
    fn a_capped_name_plus_extension_still_fits_a_filesystem_component() {
        let name = file_name(&"x".repeat(400), "mp3");
        assert!(name.len() < 255, "{} bytes", name.len());
    }
}
