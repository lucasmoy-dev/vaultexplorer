//! Real per-format metadata for Get Info: image dimensions + EXIF, audio
//! tags/properties (via `lofty`, already a dependency), video
//! streams/duration (via `ffprobe`), PDF page count/producer (via
//! `pdfinfo`). Real-fs only for video/PDF (both need a real path for
//! their external tool); image/audio work from in-memory bytes too, so
//! they're available for vault-internal files as well.

use std::path::Path;
use std::process::Command;

pub type Fields = Vec<(String, String)>;

const EXIF_TAGS: &[(exif::Tag, &str)] = &[
    (exif::Tag::Make, "Camera Make"),
    (exif::Tag::Model, "Camera Model"),
    (exif::Tag::DateTimeOriginal, "Date Taken"),
    (exif::Tag::ExposureTime, "Exposure"),
    (exif::Tag::FNumber, "Aperture"),
    (exif::Tag::PhotographicSensitivity, "ISO"),
    (exif::Tag::FocalLength, "Focal Length"),
    (exif::Tag::GPSLatitude, "GPS Latitude"),
    (exif::Tag::GPSLongitude, "GPS Longitude"),
];

pub fn image_info(bytes: &[u8]) -> Fields {
    let mut out = Fields::new();
    if let Ok(img) = image::load_from_memory(bytes) {
        out.push(("Dimensions".to_string(), format!("{} × {} px", img.width(), img.height())));
    }
    if let Ok(exif) = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(bytes)) {
        for (tag, label) in EXIF_TAGS {
            if let Some(field) = exif.get_field(*tag, exif::In::PRIMARY) {
                out.push((label.to_string(), field.display_value().with_unit(&exif).to_string()));
            }
        }
    }
    out
}

pub fn audio_info_from_path(path: &str) -> Fields {
    let Ok(tagged) = lofty::probe::Probe::open(path).and_then(|p| p.read()) else {
        return Fields::new();
    };
    audio_info_from_tagged(&tagged)
}

fn audio_info_from_tagged(tagged: &lofty::file::TaggedFile) -> Fields {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::tag::Accessor;
    let mut out = Fields::new();
    let props = tagged.properties();
    let secs = props.duration().as_secs();
    out.push(("Duration".to_string(), format!("{}:{:02}", secs / 60, secs % 60)));
    if let Some(br) = props.audio_bitrate() {
        out.push(("Bitrate".to_string(), format!("{br} kbps")));
    }
    if let Some(sr) = props.sample_rate() {
        out.push(("Sample Rate".to_string(), format!("{sr} Hz")));
    }
    if let Some(tag) = tagged.primary_tag() {
        if let Some(v) = tag.title() {
            out.push(("Title".to_string(), v.to_string()));
        }
        if let Some(v) = tag.artist() {
            out.push(("Artist".to_string(), v.to_string()));
        }
        if let Some(v) = tag.album() {
            out.push(("Album".to_string(), v.to_string()));
        }
        if let Some(v) = tag.genre() {
            out.push(("Genre".to_string(), v.to_string()));
        }
        if let Some(v) = tag.date() {
            out.push(("Year".to_string(), v.year.to_string()));
        }
    }
    out
}

pub fn video_info(path: &str) -> Fields {
    let mut out = Fields::new();
    let Ok(output) = Command::new("ffprobe")
        .args([
            "-v", "error", "-show_entries",
            "format=duration:stream=width,height,codec_name,r_frame_rate",
            "-of", "json", path,
        ])
        .output()
    else {
        return out;
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return out;
    };
    if let Some(dur) = json.get("format").and_then(|f| f.get("duration")).and_then(|d| d.as_str()) {
        if let Ok(secs) = dur.parse::<f64>() {
            let secs = secs.round() as u64;
            out.push(("Duration".to_string(), format!("{}:{:02}", secs / 60, secs % 60)));
        }
    }
    if let Some(stream) = json.get("streams").and_then(|s| s.as_array()).and_then(|a| a.first()) {
        if let (Some(w), Some(h)) = (stream.get("width"), stream.get("height")) {
            out.push(("Resolution".to_string(), format!("{w} × {h}")));
        }
        if let Some(codec) = stream.get("codec_name").and_then(|c| c.as_str()) {
            out.push(("Codec".to_string(), codec.to_uppercase()));
        }
        if let Some(fr) = stream.get("r_frame_rate").and_then(|f| f.as_str()) {
            if let Some((num, den)) = fr.split_once('/') {
                if let (Ok(n), Ok(d)) = (num.parse::<f64>(), den.parse::<f64>()) {
                    if d > 0.0 {
                        out.push(("Frame Rate".to_string(), format!("{:.2} fps", n / d)));
                    }
                }
            }
        }
    }
    out
}

pub fn pdf_info(path: &str) -> Fields {
    let mut out = Fields::new();
    let Ok(output) = Command::new("pdfinfo").arg(path).output() else {
        return out;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let wanted = [("Pages", "Pages"), ("Producer", "Producer"), ("Page size", "Page Size")];
    for line in text.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if let Some((_, label)) = wanted.iter().find(|(k, _)| *k == key) {
                out.push((label.to_string(), value.trim().to_string()));
            }
        }
    }
    out
}

/// Dispatch by extension for a real filesystem file.
pub fn fs_file_info_fields(path: &str) -> Fields {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp" | "webp" | "tiff" | "tif" | "gif") {
        std::fs::read(path).map(|b| image_info(&b)).unwrap_or_default()
    } else if matches!(ext.as_str(), "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac") {
        audio_info_from_path(path)
    } else if matches!(ext.as_str(), "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v") {
        video_info(path)
    } else if ext == "pdf" {
        pdf_info(path)
    } else {
        Fields::new()
    }
}

/// Dispatch for already-decrypted vault bytes -- image/audio only (video
/// ffprobe and PDF pdfinfo both need a real path; see module docs).
pub fn vault_file_info_fields(bytes: &[u8], name: &str) -> Fields {
    let ext = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp" | "webp" | "tiff" | "tif" | "gif") {
        image_info(bytes)
    } else if matches!(ext.as_str(), "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac") {
        let Ok(probe) = lofty::probe::Probe::new(std::io::Cursor::new(bytes)).guess_file_type() else {
            return Fields::new();
        };
        let Ok(tagged) = probe.read() else {
            return Fields::new();
        };
        audio_info_from_tagged(&tagged)
    } else {
        Fields::new()
    }
}

// ---- Tauri commands ----

#[tauri::command]
pub fn fs_file_info(path: String) -> Fields {
    fs_file_info_fields(&path)
}

#[tauri::command]
pub fn vault_file_info(state: tauri::State<crate::AppState>, rel_path: String) -> Fields {
    let Ok(bytes) = crate::with_vault(&state, |v| v.decrypt_file(&rel_path)) else {
        return Vec::new();
    };
    vault_file_info_fields(&bytes, &rel_path)
}
