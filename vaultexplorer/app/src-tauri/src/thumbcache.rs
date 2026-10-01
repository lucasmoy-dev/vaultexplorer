//! The shared freedesktop thumbnail cache (`~/.cache/thumbnails`), the
//! one Nautilus, Nemo, Thunar, Dolphin and GTK's file chooser all read and
//! write. Using it instead of a private cache is what makes a photo folder
//! the user has already opened in any of those show its thumbnails
//! *instantly* here on the very first visit -- and what lets those apps
//! reuse the thumbnails generated here.
//!
//! Format, per the Thumbnail Managing Standard:
//! - `<size dir>/<md5 of the file URI>.png`, size dirs `normal` (128),
//!   `large` (256), `x-large` (512), `xx-large` (1024).
//! - PNG `tEXt` chunks `Thumb::URI` and `Thumb::MTime` (whole seconds); a
//!   thumbnail whose MTime no longer matches the file is stale and is
//!   regenerated, never shown.
//! - Written to a temp name in the same directory and renamed, so another
//!   process never reads half a file. Directories 0700, files 0600.
//! - A file that can't be thumbnailed gets a marker under
//!   `fail/vaultexplorer/`, so a corrupt video isn't re-run through ffmpeg
//!   on every visit.
//!
//! A thumbnail that doesn't actually reach its size class (a 160px camera
//! preview standing in for a 256px "large") is kept in this app's own
//! directory instead: putting it in the shared one would leave other file
//! managers showing a blurry upscale until the photo changes.

use md5::{Digest, Md5};
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};

/// (directory, edge in px), smallest first.
pub const BUCKETS: [(&str, u32); 4] = [("normal", 128), ("large", 256), ("x-large", 512), ("xx-large", 1024)];

/// The size class a request for `max_size` px is served from, or `None`
/// when it is bigger than the largest class the standard defines.
pub fn bucket_for(max_size: u32) -> Option<usize> {
    BUCKETS.iter().position(|(_, px)| *px >= max_size)
}

fn cache_home() -> PathBuf {
    let dir = match std::env::var_os("XDG_CACHE_HOME") {
        Some(dir) if Path::new(&dir).is_absolute() => PathBuf::from(dir),
        _ => PathBuf::from(crate::home_dir()).join(".cache"),
    };
    // A relative $HOME would make every cached path relative, and
    // `is_servable` (rightly) refuses anything it can't pin down.
    if dir.is_absolute() {
        dir
    } else {
        std::env::current_dir().map(|cwd| cwd.join(&dir)).unwrap_or(dir)
    }
}

fn shared_root() -> PathBuf {
    cache_home().join("thumbnails")
}

fn own_root() -> PathBuf {
    cache_home().join("vaultexplorer").join("thumbs")
}

const FAIL_DIR: &str = "vaultexplorer";

/// `file://` URI for an absolute path, escaped exactly the way GLib's
/// `g_filename_to_uri` does it -- the md5 of this string *is* the cache
/// file name, so a single differently-escaped character (a space, an
/// accent, a `#`) would silently miss every thumbnail Nautilus made.
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(b, b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b'-' | b'.' | b'/' | b':' | b'=' | b'@' | b'_' | b'~');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn md5_name(uri: &str) -> String {
    let digest = Md5::digest(uri.as_bytes());
    let mut s = String::with_capacity(36);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s.push_str(".png");
    s
}

/// What a thumbnail is checked against.
pub struct Stamp {
    pub uri: String,
    pub mtime: u64,
    pub size: u64,
    name: String,
}

pub fn stamp(path: &Path) -> Option<Stamp> {
    if !path.is_absolute() {
        return None;
    }
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let uri = file_uri(path);
    let name = md5_name(&uri);
    Some(Stamp { uri, mtime, size: meta.len(), name })
}

/// The `tEXt`/`iTXt` key-value pairs in front of a PNG's image data --
/// `read_info` stops at the first IDAT, so this reads a few hundred bytes,
/// not the picture.
fn text_chunks(file: &Path) -> Option<Vec<(String, String)>> {
    let f = File::open(file).ok()?;
    let reader = png::Decoder::new(BufReader::new(f)).read_info().ok()?;
    let info = reader.info();
    let mut out: Vec<(String, String)> = info
        .uncompressed_latin1_text
        .iter()
        .map(|c| (c.keyword.clone(), c.text.clone()))
        .collect();
    for c in &info.utf8_text {
        if let Ok(text) = c.get_text() {
            out.push((c.keyword.clone(), text));
        }
    }
    Some(out)
}

fn is_fresh(file: &Path, st: &Stamp) -> bool {
    let Some(chunks) = text_chunks(file) else { return false };
    let get = |k: &str| chunks.iter().find(|(key, _)| key == k).map(|(_, v)| v.trim().to_string());
    // Thumb::MTime is mandatory; a thumbnail without it can't be trusted.
    let Some(mtime) = get("Thumb::MTime").and_then(|v| v.parse::<u64>().ok()) else {
        return false;
    };
    if mtime != st.mtime {
        return false;
    }
    match get("Thumb::URI") {
        Some(uri) => uri == st.uri,
        None => true,
    }
}

pub enum Lookup {
    Hit(PathBuf),
    /// A previous attempt (by this app) failed for this exact file version.
    Failed,
    Miss,
}

/// The freshest usable thumbnail for `path` at `max_size` px or larger.
pub fn lookup(path: &Path, max_size: u32) -> Lookup {
    let (Some(b), Some(st)) = (bucket_for(max_size), stamp(path)) else {
        return Lookup::Miss;
    };
    lookup_stamped(&st, b)
}

fn lookup_stamped(st: &Stamp, b: usize) -> Lookup {
    let shared = shared_root();
    for (dir, _) in &BUCKETS[b..] {
        let candidate = shared.join(dir).join(&st.name);
        if candidate.is_file() && is_fresh(&candidate, st) {
            return Lookup::Hit(candidate);
        }
    }
    let own = own_root().join(BUCKETS[b].0).join(&st.name);
    if own.is_file() && is_fresh(&own, st) {
        return Lookup::Hit(own);
    }
    let fail = shared.join("fail").join(FAIL_DIR).join(&st.name);
    if fail.is_file() && is_fresh(&fail, st) {
        return Lookup::Failed;
    }
    Lookup::Miss
}

fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

fn encode_png(img: &image::DynamicImage, st: &Stamp) -> Result<Vec<u8>, String> {
    let (w, h) = (img.width(), img.height());
    let (color, data) = if img.color().has_alpha() {
        (png::ColorType::Rgba, img.to_rgba8().into_raw())
    } else {
        (png::ColorType::Rgb, img.to_rgb8().into_raw())
    };
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let text = [
            ("Thumb::URI", st.uri.clone()),
            ("Thumb::MTime", st.mtime.to_string()),
            ("Thumb::Size", st.size.to_string()),
            ("Software", "Vault Explorer".to_string()),
        ];
        for (k, v) in text {
            enc.add_text_chunk(k.to_string(), v).map_err(|e| e.to_string())?;
        }
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&data).map_err(|e| e.to_string())?;
    }
    Ok(buf)
}

fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    ensure_dir(dir).map_err(|e| e.to_string())?;
    let dest = dir.join(name);
    let tmp = dir.join(format!(
        ".{name}.vaultexplorer-{}-{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        std::fs::rename(&tmp, &dest)
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    Ok(dest)
}

/// Store `img` (already scaled to fit the size class) for `st`. `full`
/// says whether it really is as large as that class promises (see the
/// module doc for where an undersized one goes instead).
pub fn store(st: &Stamp, b: usize, img: &image::DynamicImage, full: bool) -> Result<PathBuf, String> {
    let png = encode_png(img, st)?;
    let dir = if full { shared_root().join(BUCKETS[b].0) } else { own_root().join(BUCKETS[b].0) };
    write_atomic(&dir, &st.name, &png)
}

/// Remember that `st` can't be thumbnailed, until the file changes.
pub fn store_failure(st: &Stamp) {
    let img = image::DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1));
    if let Ok(png) = encode_png(&img, st) {
        let _ = write_atomic(&shared_root().join("fail").join(FAIL_DIR), &st.name, &png);
    }
}

/// For `lookup` callers that already hold a stamp (generation re-checks
/// right before doing the work, in case another request just did it).
pub fn lookup_with(st: &Stamp, b: usize) -> Lookup {
    lookup_stamped(st, b)
}

/// Percent-decode a URL path back to raw bytes (a filename may not be
/// UTF-8, so this never goes through `String`).
fn percent_decode_bytes(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Whether `path` is a thumbnail this app may hand the webview: a `.png`
/// directly inside one of the cache's size directories, nothing else. The
/// `vxthumb://` protocol below serves exactly these files and nothing
/// else, so it can't be turned into a way to read arbitrary files.
pub fn is_servable(path: &Path) -> bool {
    use std::path::Component;
    if path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return false;
    }
    if path.extension().and_then(|e| e.to_str()) != Some("png") {
        return false;
    }
    let Some(dir) = path.parent() else { return false };
    let (shared, own) = (shared_root(), own_root());
    BUCKETS.iter().any(|(name, _)| dir == shared.join(name) || dir == own.join(name))
}

/// The `vxthumb://` URI scheme: serves cached thumbnails to the webview.
///
/// Not Tauri's built-in asset protocol: its scope globs refuse any path
/// with a dot-directory in it (`~/.cache`) unless that whole protection is
/// switched off for every path, and a refused thumbnail simply never
/// loads. A purpose-built handler that serves only `is_servable` files is
/// both narrower and actually works.
pub fn serve(uri_path: &str) -> tauri::http::Response<Vec<u8>> {
    if std::env::var_os("VX_THUMB_DEBUG").is_some() {
        eprintln!("vxthumb serve {uri_path}");
    }
    use std::os::unix::ffi::OsStringExt;
    let raw = percent_decode_bytes(uri_path.trim_start_matches('/'));
    let path = PathBuf::from(std::ffi::OsString::from_vec(raw));
    let not_found = || {
        tauri::http::Response::builder()
            .status(404)
            .body(Vec::new())
            .expect("static response")
    };
    if !is_servable(&path) {
        return not_found();
    }
    match std::fs::read(&path) {
        Ok(bytes) => tauri::http::Response::builder()
            .status(200)
            .header("Content-Type", "image/png")
            // The URL carries the source file's mtime, so a given URL's
            // bytes never change: let the webview keep it.
            .header("Cache-Control", "max-age=31536000, immutable")
            .header("Access-Control-Allow-Origin", "*")
            .body(bytes)
            .unwrap_or_else(|_| not_found()),
        Err(_) => not_found(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Some tests point XDG_CACHE_HOME elsewhere; the rest read it.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn uri_escaping_matches_glib() {
        // Expected strings produced by GLib.filename_to_uri on this machine.
        assert_eq!(file_uri(Path::new("/home/u/a b.png")), "file:///home/u/a%20b.png");
        assert_eq!(
            file_uri(Path::new("/tmp/x#y?z[1]{2}%;<>\"`|^\\.jpg")),
            "file:///tmp/x%23y%3Fz%5B1%5D%7B2%7D%25%3B%3C%3E%22%60%7C%5E%5C.jpg"
        );
        assert_eq!(
            file_uri(Path::new("/tmp/!$&'()*+,-.:=@_~ok")),
            "file:///tmp/!$&'()*+,-.:=@_~ok"
        );
        assert_eq!(file_uri(Path::new("/tmp/año/ñ.jpg")), "file:///tmp/a%C3%B1o/%C3%B1.jpg");
    }

    #[test]
    fn only_cache_pngs_are_servable() {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let home = shared_root();
        assert!(is_servable(&home.join("large").join("abc.png")));
        assert!(!is_servable(&home.join("large").join("abc.jpg")));
        assert!(!is_servable(&home.join("large/../../../.ssh/id_rsa.png")));
        assert!(!is_servable(&home.join("fail/vaultexplorer/abc.png")));
        assert!(!is_servable(Path::new("/etc/passwd")));
        assert!(!is_servable(Path::new("/home/u/Pictures/x.png")));
        assert_eq!(percent_decode_bytes("%2Fa%20b%C3%B1.png"), "/a bñ.png".as_bytes());
        assert_eq!(percent_decode_bytes("100%"), b"100%");
    }

    #[test]
    fn md5_name_is_the_standard_one() {
        // md5("file:///home/jens/photos/me.png") from the spec's own example.
        assert_eq!(md5_name("file:///home/jens/photos/me.png"), "c6ee772d9e49320e97ec29a7eb5b1697.png");
    }

    #[test]
    fn stores_reads_back_and_goes_stale() {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("vx-thumbcache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // Point the cache at the temp dir for this test only.
        std::env::set_var("XDG_CACHE_HOME", tmp.join("cache"));
        let src = tmp.join("photo one.jpg");
        std::fs::write(&src, b"not really a jpeg").unwrap();
        let st = stamp(&src).unwrap();
        assert!(matches!(lookup(&src, 160), Lookup::Miss));

        let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(256, 170));
        let b = bucket_for(160).unwrap();
        assert_eq!(BUCKETS[b].0, "large");
        let stored = store(&st, b, &img, true).unwrap();
        assert!(stored.starts_with(tmp.join("cache/thumbnails/large")));
        match lookup(&src, 160) {
            Lookup::Hit(p) => assert_eq!(p, stored),
            _ => panic!("expected a hit"),
        }
        // A smaller request is served by the larger class too...
        assert!(matches!(lookup(&src, 100), Lookup::Hit(_)));
        // ...but a larger one isn't served by a smaller class.
        assert!(matches!(lookup(&src, 400), Lookup::Miss));

        // Touch the file: the thumbnail is stale and must not be used.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        File::options().write(true).open(&src).unwrap().set_modified(later).unwrap();
        assert!(matches!(lookup(&src, 160), Lookup::Miss));

        let st2 = stamp(&src).unwrap();
        store_failure(&st2);
        assert!(matches!(lookup(&src, 160), Lookup::Failed));
        std::env::remove_var("XDG_CACHE_HOME");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Against the real cache on this machine: every path listed in
    /// `VX_THUMB_PROBE` (newline-separated) must resolve to a thumbnail
    /// some other program (Nautilus) wrote. Run with `--ignored`.
    #[test]
    #[ignore]
    fn reads_thumbnails_other_apps_wrote() {
        let list = std::env::var("VX_THUMB_PROBE").expect("set VX_THUMB_PROBE");
        let mut n = 0;
        for line in list.lines().filter(|l| !l.is_empty()) {
            match lookup(Path::new(line), 160) {
                Lookup::Hit(p) => {
                    n += 1;
                    eprintln!("hit {line} -> {}", p.display());
                }
                _ => panic!("no hit for {line}"),
            }
        }
        assert!(n > 0);
    }
}
