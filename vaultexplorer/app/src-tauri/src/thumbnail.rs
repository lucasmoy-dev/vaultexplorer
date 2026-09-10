//! Image thumbnail generation. Real (plain) files are cached to disk
//! keyed by path+mtime so re-listing a folder doesn't re-decode every
//! image every time. Vault-internal images are **never** disk-cached --
//! caching a decrypted thumbnail would leak vault image content onto
//! plaintext disk outside the vault's own ciphertext, defeating the
//! point. Every call for a vault image re-decrypts and re-thumbnails;
//! that's only ever for entries actually rendered on screen, so it's a
//! fine trade for keeping the security property simple and obviously
//! correct.

use crate::errmap::ToStringErr;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageEncoder};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `$HOME` is unset for this app on Android, so `crate::home_dir()`
/// (unix `$HOME`-or-`/`) silently resolved to `/.cache/...` -- a path this
/// app can't write to -- and every `create_dir_all` below failed, meaning
/// the disk thumbnail cache never actually persisted a single file there;
/// every folder listing re-decoded every image from scratch. `app_cache_dir()`
/// is the platform-correct answer (`Context.getCacheDir()` via Tauri's own
/// resolver) and costs nothing extra on desktop, where it's unused.
#[cfg(mobile)]
fn cache_dir(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    match app.path().app_cache_dir() {
        Ok(dir) => dir.join("thumbnails"),
        Err(_) => PathBuf::from(format!("{}/.cache/vaultexplorer/thumbnails", crate::home_dir())),
    }
}
#[cfg(not(mobile))]
fn cache_dir(_app: &tauri::AppHandle) -> PathBuf {
    PathBuf::from(format!("{}/.cache/vaultexplorer/thumbnails", crate::home_dir()))
}

fn cache_key(path: &str, mtime: i64, max_size: u32) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    mtime.hash(&mut hasher);
    max_size.hash(&mut hasher);
    format!("{:x}.jpg", hasher.finish())
}

/// What a JPEG's marker segments say about it without decoding a single
/// pixel: its real dimensions (from the frame header) and the small
/// preview JPEG a camera stored in its EXIF block, if there is one.
struct JpegPeek {
    width: u32,
    height: u32,
    exif_thumb: Option<Vec<u8>>,
}

fn be_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

/// Walk a JPEG's segment headers (they sit in front of the entropy-coded
/// data, so this reads a few hundred bytes, not the image) collecting the
/// frame size and the EXIF APP1 block.
fn peek_jpeg(bytes: &[u8]) -> Option<JpegPeek> {
    if bytes.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut out = JpegPeek { width: 0, height: 0, exif_thumb: None };
    let mut i = 2usize;
    loop {
        // Segments are 0xFF-prefixed; padding 0xFFs before a marker are legal.
        while *bytes.get(i)? == 0xFF && *bytes.get(i + 1)? == 0xFF {
            i += 1;
        }
        if *bytes.get(i)? != 0xFF {
            return None;
        }
        let marker = *bytes.get(i + 1)?;
        // Standalone markers carry no length word.
        if marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
            i += 2;
            continue;
        }
        let len = be_u16(bytes, i + 2)? as usize;
        if len < 2 {
            return None;
        }
        let payload = bytes.get(i + 4..i + 2 + len)?;
        match marker {
            // Start of frame (baseline/progressive/etc., but not the
            // huffman/arithmetic table markers that share the range).
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                out.height = be_u16(payload, 1)? as u32;
                out.width = be_u16(payload, 3)? as u32;
            }
            0xE1 if payload.starts_with(b"Exif\0\0") => {
                out.exif_thumb = exif_thumb_from_tiff(&payload[6..]);
            }
            // Start of scan: compressed data from here on, nothing left to read.
            0xDA => break,
            _ => {}
        }
        i += 2 + len;
    }
    if out.width == 0 || out.height == 0 {
        return None;
    }
    Some(out)
}

/// The thumbnail JPEG referenced by IFD1 of an EXIF TIFF block, if it has
/// one. `tiff` starts at the TIFF header (all offsets in here are relative
/// to that, which is why it's passed as its own slice).
fn exif_thumb_from_tiff(tiff: &[u8]) -> Option<Vec<u8>> {
    let big_endian = match tiff.get(..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b = [*tiff.get(at)?, *tiff.get(at + 1)?];
        Some(if big_endian { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b = [*tiff.get(at)?, *tiff.get(at + 1)?, *tiff.get(at + 2)?, *tiff.get(at + 3)?];
        Some(if big_endian { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    // IFD0 describes the full image; the thumbnail lives in IFD1, which
    // IFD0's trailing "next IFD" pointer leads to.
    let ifd0 = u32_at(4)? as usize;
    let ifd0_count = u16_at(ifd0)? as usize;
    let ifd1 = u32_at(ifd0 + 2 + ifd0_count * 12)? as usize;
    if ifd1 == 0 {
        return None;
    }
    let ifd1_count = u16_at(ifd1)? as usize;
    let mut offset = None;
    let mut length = None;
    for n in 0..ifd1_count {
        let entry = ifd1 + 2 + n * 12;
        let tag = u16_at(entry)?;
        if tag != 0x0201 && tag != 0x0202 {
            continue;
        }
        // Both tags are a single SHORT or LONG, so the value is stored
        // inline in the entry's value field rather than pointed at.
        let value = match u16_at(entry + 2)? {
            3 => u16_at(entry + 8)? as u32,
            4 => u32_at(entry + 8)?,
            _ => continue,
        };
        if tag == 0x0201 {
            offset = Some(value as usize);
        } else {
            length = Some(value as usize);
        }
    }
    let (offset, length) = (offset?, length?);
    let thumb = tiff.get(offset..offset.checked_add(length)?)?;
    // Compression 6 (JPEG) is what every camera writes; anything else
    // (uncompressed TIFF strips) isn't worth handling -- fall back to the
    // full decode.
    if !thumb.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    Some(thumb.to_vec())
}

/// Cameras and phones embed a small JPEG preview in every photo's EXIF
/// block. When it's big enough for what was asked for, that's the whole
/// thumbnail already: decoding it costs microseconds, where decoding the
/// photo it came from means rasterizing 12-50 megapixels (~4 bytes each)
/// just to throw all but a 160px square of it away. That full decode --
/// times however many photos a folder holds, times every visit for a
/// vault, which can't cache to disk -- is what made opening a photo folder
/// crawl.
///
/// Returns `None` (i.e. decode the real image) unless the embedded preview
/// is both large enough not to be upscaled and the same shape as the photo
/// -- some cameras letterbox their preview, and a thumbnail with black
/// bars is worse than a slow one.
fn thumbnail_from_exif(bytes: &[u8], max_size: u32) -> Option<Vec<u8>> {
    let peek = peek_jpeg(bytes)?;
    let thumb_bytes = peek.exif_thumb?;
    let thumb = image::load_from_memory(&thumb_bytes).ok()?;
    let (tw, th) = (thumb.width(), thumb.height());
    if tw == 0 || th == 0 || tw.max(th) < max_size {
        return None;
    }
    let full_aspect = peek.width as f32 / peek.height as f32;
    let thumb_aspect = tw as f32 / th as f32;
    if (full_aspect - thumb_aspect).abs() > full_aspect * 0.03 {
        return None;
    }
    encode_thumbnail(&thumb, max_size).ok()
}

/// Shrink to fit within `max_size` x `max_size` (preserving aspect ratio)
/// and re-encode as a JPEG.
fn encode_thumbnail(img: &image::DynamicImage, max_size: u32) -> Result<Vec<u8>, String> {
    let thumb = img.thumbnail(max_size, max_size);
    let rgb = thumb.to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 80)
        .write_image(rgb.as_raw(), rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
        .str_err()?;
    Ok(out)
}

/// Decode `bytes`, shrink to fit within `max_size` x `max_size`
/// (preserving aspect ratio), and re-encode as a JPEG.
fn make_thumbnail(bytes: &[u8], max_size: u32) -> Result<Vec<u8>, String> {
    if let Some(jpeg) = thumbnail_from_exif(bytes, max_size) {
        return Ok(jpeg);
    }
    let img = image::load_from_memory(bytes).str_err()?;
    encode_thumbnail(&img, max_size)
}

fn to_data_uri(jpeg_bytes: &[u8]) -> String {
    format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg_bytes))
}

/// Thumbnail for a real on-disk image file, cached by path+mtime.
pub fn thumbnail_for_path(app: &tauri::AppHandle, path: &str, max_size: u32) -> Result<String, String> {
    let metadata = std::fs::metadata(path).str_err()?;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let dir = cache_dir(app);
    std::fs::create_dir_all(&dir).str_err()?;
    let cache_path = dir.join(cache_key(path, mtime, max_size));

    if let Ok(cached) = std::fs::read(&cache_path) {
        return Ok(to_data_uri(&cached));
    }
    let bytes = std::fs::read(path).str_err()?;
    let jpeg = make_thumbnail(&bytes, max_size)?;
    let _ = std::fs::write(&cache_path, &jpeg);
    Ok(to_data_uri(&jpeg))
}

/// Thumbnail for already-decrypted vault-file bytes. See module docs for
/// why this path is never disk-cached.
pub fn thumbnail_for_bytes(bytes: &[u8], max_size: u32) -> Result<String, String> {
    let jpeg = make_thumbnail(bytes, max_size)?;
    Ok(to_data_uri(&jpeg))
}

/// Thumbnail for a real on-disk video file: grabs a single frame via
/// `ffmpeg` (1s in, or the very first frame for shorter clips) into a temp
/// JPEG, then runs it through the same resize/cache pipeline as a real
/// image. Real-fs only -- there's no vault-internal equivalent, since
/// ffmpeg needs a real path to read and decrypting a vault video to a
/// plaintext temp file for it would break the same invariant that scoped
/// vault-internal audio metadata clearing and media conversion out too.
pub fn thumbnail_for_video(app: &tauri::AppHandle, path: &str, max_size: u32) -> Result<String, String> {
    let metadata = std::fs::metadata(path).str_err()?;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let dir = cache_dir(app);
    std::fs::create_dir_all(&dir).str_err()?;
    let cache_path = dir.join(cache_key(&format!("video:{path}"), mtime, max_size));
    if let Ok(cached) = std::fs::read(&cache_path) {
        return Ok(to_data_uri(&cached));
    }

    // Per-call unique temp name (pid + atomic seq) -- a process-shared name
    // would collide once video thumbnails run concurrently on the blocking
    // threadpool.
    use std::sync::atomic::{AtomicU64, Ordering};
    static VTHUMB_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = VTHUMB_SEQ.fetch_add(1, Ordering::Relaxed);
    let frame_path =
        std::env::temp_dir().join(format!("vaultexplorer-vthumb-{}-{}.jpg", std::process::id(), seq));
    let grab = |seek: &str| -> bool {
        Command::new("ffmpeg")
            .args(["-y", "-ss", seek, "-i", path, "-frames:v", "1", "-vf", "scale=480:-1"])
            .arg(&frame_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
            && frame_path.exists()
    };
    // 1s in avoids all-black opening frames on most clips; fall back to
    // the very first frame for anything shorter than that.
    if !grab("00:00:01") && !grab("00:00:00") {
        return Err("could not extract a video frame".to_string());
    }
    let bytes = std::fs::read(&frame_path).str_err()?;
    let _ = std::fs::remove_file(&frame_path);
    let jpeg = make_thumbnail(&bytes, max_size)?;
    let _ = std::fs::write(&cache_path, &jpeg);
    Ok(to_data_uri(&jpeg))
}

/// Rasterize one page of a real on-disk PDF to a JPEG data URI, cached by
/// path+mtime+page+size like every other thumbnail here. This is both the
/// page-1 cover the grid/Library views show (see `thumbnail_for_pdf`) and
/// the page images the preview pane pages through, since the webview has
/// no built-in PDF renderer to point at the file directly -- `pdftoppm`
/// (poppler-utils, the same tool `convert::pdf_to_images` shells out to)
/// is what draws the page.
///
/// Real-fs only, for the same reason as video: poppler needs a real path,
/// and decrypting a vault PDF to a plaintext temp file for it would break
/// the vault's on-disk invariant.
pub fn pdf_page_image(
    app: &tauri::AppHandle,
    path: &str,
    page: u32,
    max_size: u32,
) -> Result<String, String> {
    let page = page.max(1);
    let metadata = std::fs::metadata(path).str_err()?;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let dir = cache_dir(app);
    std::fs::create_dir_all(&dir).str_err()?;
    let cache_path = dir.join(cache_key(&format!("pdf:{path}:p{page}"), mtime, max_size));
    if let Ok(cached) = std::fs::read(&cache_path) {
        return Ok(to_data_uri(&cached));
    }

    use std::sync::atomic::{AtomicU64, Ordering};
    static PTHUMB_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = PTHUMB_SEQ.fetch_add(1, Ordering::Relaxed);
    let prefix =
        std::env::temp_dir().join(format!("vaultexplorer-pthumb-{}-{}", std::process::id(), seq));
    // Rasterize at roughly the resolution actually asked for rather than a
    // fixed 120dpi: a page rendered for a grid icon and one rendered to be
    // *read* (and zoomed into) in the preview pane want very different
    // amounts of detail, and rendering small then upscaling is exactly the
    // blur the preview is meant to avoid. ~9in is the long edge of a
    // typical page, so max_size/9 is the dpi that fills the request;
    // clamped so a tiny thumbnail still renders legibly and a big one
    // can't ask poppler for an enormous bitmap.
    let dpi = (max_size / 9).clamp(96, 300);
    let page_arg = page.to_string();
    let output = Command::new("pdftoppm")
        .args(["-jpeg", "-f", &page_arg, "-l", &page_arg, "-r", &dpi.to_string(), path])
        .arg(prefix.to_str().unwrap())
        .stdout(Stdio::null())
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    // pdftoppm suffixes the page number, zero-padded to the width of the
    // document's page count ("-1", "-01", "-001"), and older poppler
    // versions drop the suffix entirely for a single-page render -- so the
    // output file is found by prefix rather than by guessing the name.
    let frame_path = find_page_output(&prefix).ok_or("pdftoppm produced no page image")?;
    let bytes = std::fs::read(&frame_path).str_err()?;
    let _ = std::fs::remove_file(&frame_path);
    let jpeg = make_thumbnail(&bytes, max_size)?;
    let _ = std::fs::write(&cache_path, &jpeg);
    Ok(to_data_uri(&jpeg))
}

/// The single `<prefix>*.jpg` file `pdftoppm` just wrote, whatever page
/// suffix it chose (see the call site).
fn find_page_output(prefix: &Path) -> Option<PathBuf> {
    let dir = prefix.parent()?;
    let stem = prefix.file_name()?.to_string_lossy().to_string();
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .map(|n| {
                    let n = n.to_string_lossy();
                    n.starts_with(&stem) && n.ends_with(".jpg")
                })
                .unwrap_or(false)
        })
}

/// A PDF's page count, via `pdfinfo` -- what the preview pane's page
/// stepper needs to know where the document ends.
pub fn pdf_page_count(path: &str) -> Result<u32, String> {
    let output = Command::new("pdfinfo").arg(path).output().str_err()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| line.strip_prefix("Pages:"))
        .and_then(|value| value.trim().parse::<u32>().ok())
        .ok_or_else(|| "could not read the PDF's page count".to_string())
}

/// The page-1 cover every non-preview view shows for a PDF, the same way
/// Finder/GNOME Files do -- this is what gives the Library view (see
/// LibraryShelf.tsx) a real cover instead of a plain color block.
pub fn thumbnail_for_pdf(app: &tauri::AppHandle, path: &str, max_size: u32) -> Result<String, String> {
    pdf_page_image(app, path, 1, max_size)
}

// ---- Tauri commands ----

/// A small base64 JPEG data URI for a real image file, cached on disk by
/// path+mtime. `async` + `spawn_blocking` so the decode/resize/ffmpeg work
/// runs off the webview (main) thread on the blocking threadpool -- many
/// tiles opening at once then decode in parallel instead of freezing the
/// UI one image at a time.
#[tauri::command]
pub async fn fs_thumbnail(app: tauri::AppHandle, path: String, max_size: u32) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let ext = Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if matches!(ext.as_str(), "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v") {
            thumbnail_for_video(&app, &path, max_size)
        } else if ext == "pdf" {
            thumbnail_for_pdf(&app, &path, max_size)
        } else {
            thumbnail_for_path(&app, &path, max_size)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Same, for an image living inside the active vault -- decrypted only
/// in memory, never disk-cached (see the module doc comment). The decrypt
/// needs the vault state so it stays inline; only the CPU-bound
/// decode/resize is offloaded to the blocking pool.
#[tauri::command]
pub async fn vault_thumbnail(
    state: tauri::State<'_, crate::AppState>,
    rel_path: String,
    max_size: u32,
) -> Result<String, String> {
    let bytes = crate::with_vault(&state, |v| v.decrypt_file(&rel_path))?;
    tauri::async_runtime::spawn_blocking(move || thumbnail_for_bytes(&bytes, max_size))
        .await
        .map_err(|e| e.to_string())?
}

/// One rasterized page of a real on-disk PDF, for the preview pane's
/// page-through viewer. Cached and offloaded exactly like `fs_thumbnail`.
#[tauri::command]
pub async fn fs_pdf_page(
    app: tauri::AppHandle,
    path: String,
    page: u32,
    max_size: u32,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || pdf_page_image(&app, &path, page, max_size))
        .await
        .map_err(|e| e.to_string())?
}

/// How many pages that PDF has, so the viewer knows its bounds.
#[tauri::command]
pub async fn fs_pdf_page_count(path: String) -> Result<u32, String> {
    tauri::async_runtime::spawn_blocking(move || pdf_page_count(&path))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real (tiny) JPEG, to stand in for the preview a camera embeds.
    fn small_jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        }));
        encode_thumbnail(&img, w.max(h)).unwrap()
    }

    /// The smallest JPEG shell `peek_jpeg` will read: SOI, an optional
    /// EXIF APP1 carrying `thumb` in IFD1, a frame header declaring
    /// `full`, then SOS (where the parser stops).
    fn jpeg_with_exif_thumb(full: (u16, u16), thumb: Option<&[u8]>) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        if let Some(thumb) = thumb {
            let mut tiff: Vec<u8> = Vec::new();
            tiff.extend(b"II");
            tiff.extend(42u16.to_le_bytes());
            tiff.extend(8u32.to_le_bytes()); // IFD0 starts right after the header
            // IFD0: one throwaway entry, then a pointer to IFD1.
            tiff.extend(1u16.to_le_bytes());
            tiff.extend(0x0100u16.to_le_bytes()); // ImageWidth
            tiff.extend(3u16.to_le_bytes()); // SHORT
            tiff.extend(1u32.to_le_bytes());
            tiff.extend((full.0 as u32).to_le_bytes());
            let ifd1 = 26u32; // 8 (header) + 2 (count) + 12 (entry) + 4 (next)
            tiff.extend(ifd1.to_le_bytes());
            // IFD1: compression + where the thumbnail bytes live.
            let thumb_at = ifd1 + 2 + 3 * 12 + 4;
            tiff.extend(3u16.to_le_bytes());
            for (tag, ty, value) in [
                (0x0103u16, 3u16, 6u32), // Compression = JPEG
                (0x0201, 4, thumb_at),
                (0x0202, 4, thumb.len() as u32),
            ] {
                tiff.extend(tag.to_le_bytes());
                tiff.extend(ty.to_le_bytes());
                tiff.extend(1u32.to_le_bytes());
                if ty == 3 {
                    tiff.extend((value as u16).to_le_bytes());
                    tiff.extend([0, 0]);
                } else {
                    tiff.extend(value.to_le_bytes());
                }
            }
            tiff.extend(0u32.to_le_bytes()); // no IFD2
            assert_eq!(tiff.len(), thumb_at as usize);
            tiff.extend(thumb);

            let mut payload = b"Exif\0\0".to_vec();
            payload.extend(&tiff);
            out.extend([0xFF, 0xE1]);
            out.extend(((payload.len() + 2) as u16).to_be_bytes());
            out.extend(payload);
        }
        // SOF0 (baseline), one component.
        out.extend([0xFF, 0xC0]);
        out.extend(11u16.to_be_bytes());
        out.push(8);
        out.extend(full.1.to_be_bytes());
        out.extend(full.0.to_be_bytes());
        out.extend([1, 1, 0x11, 0]);
        out.extend([0xFF, 0xDA]);
        out.extend(8u16.to_be_bytes());
        out.extend([1, 1, 0, 0, 63, 0]);
        out
    }

    #[test]
    fn reads_the_embedded_preview_instead_of_the_photo() {
        let thumb = small_jpeg(160, 120);
        let photo = jpeg_with_exif_thumb((4000, 3000), Some(&thumb));

        let peek = peek_jpeg(&photo).expect("markers parse");
        assert_eq!((peek.width, peek.height), (4000, 3000));
        assert_eq!(peek.exif_thumb.as_deref(), Some(thumb.as_slice()));

        // Small enough to be served by the 160x120 preview...
        let out = thumbnail_from_exif(&photo, 64).expect("preview used");
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!(decoded.width().max(decoded.height()), 64);
        // ...but a preview that would have to be upscaled is refused, so
        // the caller decodes the real photo.
        assert!(thumbnail_from_exif(&photo, 400).is_none());
    }

    #[test]
    fn refuses_a_preview_that_is_the_wrong_shape() {
        let thumb = small_jpeg(160, 120); // 4:3
        let square = jpeg_with_exif_thumb((3000, 3000), Some(&thumb));
        assert!(thumbnail_from_exif(&square, 64).is_none());
    }

    #[test]
    fn falls_through_when_there_is_no_exif_at_all() {
        let plain = jpeg_with_exif_thumb((4000, 3000), None);
        assert!(peek_jpeg(&plain).unwrap().exif_thumb.is_none());
        assert!(thumbnail_from_exif(&plain, 64).is_none());
        // Not a JPEG at all: the fast path must not claim it.
        assert!(thumbnail_from_exif(b"\x89PNG\r\n\x1a\n and then some", 64).is_none());
    }
}
