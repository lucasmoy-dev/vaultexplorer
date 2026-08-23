//! Optional transcript on disk.
//!
//! Append-only, one line per caption, with a local timestamp and which
//! side said it -- the format you want when the point is to re-read a
//! meeting afterwards, and one `grep` away from being useful. Never
//! rewritten or rotated: this is the user's file, in the place they chose.

use std::io::Write;
use std::path::Path;

/// `YYYY-MM-DD HH:MM:SS` in *local* time. Local rather than UTC because
/// this is read next to a calendar, and via `localtime_r` rather than a
/// date crate because it's the one call that knows about the machine's
/// timezone and DST.
pub fn local_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `now` is a valid time_t and `tm` is a zeroed, correctly
    // sized destination that localtime_r fills in.
    let ok = unsafe { !libc::localtime_r(&now, &mut tm).is_null() };
    if !ok {
        return "????-??-?? ??:??:??".to_string();
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// One caption line. `translated` is `None` when no translation was asked
/// for; when it is set, both are written -- the transcript is the place to
/// keep the original, even if the overlay only shows one of them.
pub fn append(path: &str, source: &str, language: &str, text: &str, translated: Option<&str>) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("no hay ruta de salida configurada".to_string());
    }
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("no se pudo escribir en {path}: {e}"))?;
    let stamp = local_timestamp();
    let language = if language.is_empty() { "??" } else { language };
    let mut line = format!("[{stamp}] [{source}] [{language}] {text}\n");
    if let Some(translated) = translated {
        if translated != text {
            line.push_str(&format!("[{stamp}] [{source}] [->] {translated}\n"));
        }
    }
    file.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_has_the_expected_shape() {
        let stamp = local_timestamp();
        assert_eq!(stamp.len(), 19, "{stamp}");
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[13..14], ":");
        // Sanity: this app did not exist before 2020 and won't emit a
        // timestamp from the epoch.
        let year: i32 = stamp[..4].parse().expect("year");
        assert!(year >= 2020, "{stamp}");
    }

    #[test]
    fn lines_are_appended_not_replaced() {
        let path = std::env::temp_dir().join("livesubs-log-test.txt");
        let _ = std::fs::remove_file(&path);
        let p = path.to_str().unwrap();
        append(p, "mic", "es", "hola", None).unwrap();
        append(p, "system", "en", "hello", Some("hola")).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 3, "{body}");
        assert!(lines[0].contains("[mic] [es] hola"));
        assert!(lines[1].contains("[system] [en] hello"));
        assert!(lines[2].contains("[->] hola"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_identical_translation_is_not_written_twice() {
        let path = std::env::temp_dir().join("livesubs-log-test-2.txt");
        let _ = std::fs::remove_file(&path);
        let p = path.to_str().unwrap();
        append(p, "mic", "es", "hola", Some("hola")).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body.lines().count(), 1, "{body}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_empty_path_is_an_error_rather_than_a_stray_file() {
        assert!(append("", "mic", "es", "hola", None).is_err());
    }
}
