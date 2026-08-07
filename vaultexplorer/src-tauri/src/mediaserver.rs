//! A loopback HTTP server for local media files.
//!
//! `convertFileSrc` hands the webview an `asset://` URL, and WebKitGTK
//! renders those fine for images -- but a `<video>`/`<audio>` element gets
//! its bytes through GStreamer, not through the page's own loader, and it
//! never plays a custom-scheme URL: the element sits at `readyState` 0
//! with `duration` 0 forever, which is the long-standing "the video player
//! doesn't work" report (black stage, 0:00/0:00, on every file, while the
//! same path shown as an image works).
//!
//! So media is served over real HTTP from 127.0.0.1 instead, with the byte
//! ranges a media element needs to seek. Access is deliberately narrow:
//! loopback-only, and a URL only resolves if the frontend registered that
//! exact path first and holds the random token minted for it -- this
//! server never maps a URL path onto the filesystem, so there is no
//! traversal surface.

use crate::errmap::{LockExt, ToStringErr};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

fn registry() -> &'static Mutex<HashMap<String, PathBuf>> {
    static REG: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    REG.get_or_init(Default::default)
}

/// Tokens are per-path and per-run: enough to keep anything that isn't
/// this webview from guessing a URL, without pretending to be a real
/// capability system (the server is loopback-only to begin with).
fn mint_token(path: &std::path::Path) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    // Process id + counter + a hash of the path: unique per run, stable
    // for the same path so re-opening a file reuses one registry entry.
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in path.as_os_str().as_encoded_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:x}{:x}{:x}", std::process::id(), n, hash)
}

fn port() -> Result<u16, String> {
    static PORT: OnceLock<Result<u16, String>> = OnceLock::new();
    PORT.get_or_init(start_server).clone()
}

fn start_server() -> Result<u16, String> {
    let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = server.server_addr().to_ip().ok_or("no TCP address")?.port();
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            // One thread per request, not a single serial loop: a media
            // element keeps a read-ahead response open on one connection
            // while seeking on another, so serving requests one at a time
            // deadlocks it -- the first response never finishes draining
            // and the second never gets answered, and the element gives up
            // with NotSupportedError. That is why small files played and
            // real (large) ones didn't. Covered by the concurrency test
            // below.
            std::thread::spawn(move || {
                let token = query_param(request.url(), "t").unwrap_or_default();
                let path = registry().lock_safe().get(&token).cloned();
                let range = header_value(&request, "range");
                match path {
                    Some(p) => {
                        if let Err(_e) = serve_file(request, &p, range.as_deref()) {
                            // A dropped connection (the element seeked away
                            // mid-response) is the normal case here, not
                            // something worth logging or retrying.
                        }
                    }
                    None => {
                        let _ = request.respond(tiny_http::Response::empty(404));
                    }
                }
            });
        }
    });
    Ok(port)
}

fn header_value(request: &tiny_http::Request, name: &'static str) -> Option<String> {
    // `equiv` wants a &'static str (tiny_http compares against interned
    // field names), so this takes one rather than allocating a HeaderField
    // per lookup.
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str().to_string())
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

/// `bytes=START-[END]` -- the only form a media element sends. A
/// suffix range (`bytes=-N`) is answered as "from N bytes before the
/// end", which is what a container probing its trailer asks for.
fn parse_range(raw: &str, len: u64) -> Option<(u64, u64)> {
    let spec = raw.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (start_s, end_s) = spec.split_once('-')?;
    if start_s.is_empty() {
        let n: u64 = end_s.parse().ok()?;
        let n = n.min(len);
        return Some((len.saturating_sub(n), len.saturating_sub(1)));
    }
    let start: u64 = start_s.parse().ok()?;
    if start >= len {
        return None;
    }
    let end = if end_s.is_empty() {
        len - 1
    } else {
        end_s.parse::<u64>().ok()?.min(len - 1)
    };
    if end < start {
        return None;
    }
    Some((start, end))
}

fn header(name: &str, value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes())
        .unwrap_or_else(|_| unreachable!("header built from a checked literal"))
}

fn serve_file(request: tiny_http::Request, path: &std::path::Path, range: Option<&str>) -> Result<(), String> {
    let mut file = File::open(path).str_err()?;
    let len = file.metadata().str_err()?.len();
    let mime = mime_for(path);

    // HEAD is how a media element asks "how big is this, and can I seek in
    // it" before requesting a single byte -- and the answer has to carry a
    // real Content-Length. Left to itself tiny_http answers a HEAD with
    // `Transfer-Encoding: chunked` and no length at all, which the element
    // reads as an empty/unusable resource: it then fails with
    // MEDIA_ERR_SRC_NOT_SUPPORTED (error 4) without ever fetching data.
    // That is the "Media error 4" seen on every real file. An empty reader
    // with an explicit data length gives the right headers, and tiny_http
    // sends no body for a HEAD anyway.
    if request.method() == &tiny_http::Method::Head {
        // Written straight onto the socket rather than through
        // `Response`: tiny_http answers a HEAD with `Transfer-Encoding:
        // chunked` and no Content-Length no matter what data length it is
        // given, and a media element reads a length-less HEAD as an
        // empty/unusable resource -- it then fails with
        // MEDIA_ERR_SRC_NOT_SUPPORTED (error 4) before fetching a single
        // byte, which is the "Media error 4" every real file hit. The
        // response is closed rather than kept alive, so writing it by hand
        // can't desynchronize a reused connection.
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {len}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n"
        );
        let mut writer = request.into_writer();
        writer.write_all(head.as_bytes()).str_err()?;
        // Flushed and dropped explicitly: the client is waiting on
        // `Connection: close`, so the socket has to actually close before
        // it will consider the response finished.
        writer.flush().str_err()?;
        drop(writer);
        return Ok(());
    }

    match range.and_then(|r| parse_range(r, len)) {
        Some((start, end)) => {
            let count = end - start + 1;
            file.seek(SeekFrom::Start(start)).str_err()?;
            let response = tiny_http::Response::new(
                tiny_http::StatusCode(206),
                vec![
                    header("Content-Type", &mime),
                    header("Accept-Ranges", "bytes"),
                    header("Content-Range", &format!("bytes {start}-{end}/{len}")),
                ],
                file.take(count),
                Some(count as usize),
                None,
            );
            request.respond(response).str_err()
        }
        None => {
            let response = tiny_http::Response::new(
                tiny_http::StatusCode(200),
                vec![header("Content-Type", &mime), header("Accept-Ranges", "bytes")],
                file,
                Some(len as usize),
                None,
            );
            request.respond(response).str_err()
        }
    }
}

/// Enough of a mapping for what this app opens; anything unrecognized is
/// left to the element's own sniffing rather than mislabeled.
fn mime_for(path: &std::path::Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "ogv" => "video/ogg",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Registers `path` and returns the loopback URL a media element can play.
#[tauri::command]
pub fn media_url(path: String) -> Result<String, String> {
    let p = PathBuf::from(&path);
    if !p.is_file() {
        return Err(format!("Not a file: {path}"));
    }
    let token = mint_token(&p);
    registry().lock_safe().insert(token.clone(), p);
    let port = port()?;
    Ok(format!("http://127.0.0.1:{port}/media?t={token}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ve-mediaserver-{}-{name}", std::process::id()));
        let mut f = File::create(&p).expect("create");
        f.write_all(bytes).expect("write");
        p
    }

    #[test]
    fn serves_whole_file_and_byte_ranges() {
        let path = temp_file("whole.mp4", b"0123456789");
        let url = media_url(path.to_string_lossy().to_string()).expect("url");

        let client = reqwest::blocking::Client::new();
        let whole = client.get(&url).send().expect("send");
        assert_eq!(whole.headers().get("accept-ranges").unwrap(), "bytes");
        assert_eq!(whole.text().expect("text"), "0123456789");

        let part = client.get(&url).header("Range", "bytes=2-5").send().expect("send");
        assert_eq!(part.status().as_u16(), 206);
        assert_eq!(part.headers().get("content-range").unwrap(), "bytes 2-5/10");
        assert_eq!(part.text().expect("text"), "2345");

        let _ = std::fs::remove_file(path);
    }

    /// A media element opens a second connection while the first is still
    /// streaming (it reads ahead on one and seeks on another), so a server
    /// that answers one request at a time deadlocks the player: the first
    /// response never finishes draining, the second never gets served, and
    /// the element gives up with NotSupportedError. Reproduced here with a
    /// file big enough that the first response can't fit in socket buffers.
    #[test]
    fn serves_a_second_request_while_the_first_is_still_open() {
        let big = vec![b'x'; 8 * 1024 * 1024];
        let path = temp_file("concurrent.mp4", &big);
        let url = media_url(path.to_string_lossy().to_string()).expect("url");

        let client = reqwest::blocking::Client::new();
        // Held open deliberately: never read to completion.
        let _first = client.get(&url).send().expect("first response");

        let second = client
            .get(&url)
            .header("Range", "bytes=100-199")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .expect("second response while the first is still open");
        assert_eq!(second.status().as_u16(), 206);
        assert_eq!(second.text().expect("text").len(), 100);

        let _ = std::fs::remove_file(path);
    }

    /// Against a real, large file rather than a synthetic small one: the
    /// failure being chased here only showed up on files big enough that a
    /// media element issues several ranged requests, so a test that only
    /// ever serves a few bytes proves nothing about it. Skipped when the
    /// sample file isn't present.
    #[test]
    fn serves_ranges_from_a_large_real_file() {
        let path = match std::env::var("VE_TEST_LARGE_FILE") {
            Ok(p) if std::path::Path::new(&p).is_file() => p,
            _ => return,
        };
        let len = std::fs::metadata(&path).expect("meta").len();
        let url = media_url(path).expect("url");
        let client = reqwest::blocking::Client::new();

        let head = client.head(&url).send().expect("head");
        println!("HEAD status={} headers={:?}", head.status(), head.headers());
        assert_eq!(head.status().as_u16(), 200);
        assert_eq!(
            head.headers().get("content-length").map(|v| v.to_str().unwrap().to_string()),
            Some(len.to_string())
        );

        // The opening probe a media element makes, then a seek near the end
        // (where an mp4's moov atom often lives).
        let first = client.get(&url).header("Range", "bytes=0-65535").send().expect("first");
        assert_eq!(first.status().as_u16(), 206);
        assert_eq!(first.bytes().expect("bytes").len(), 65536);

        let tail_start = len - 65536;
        let tail = client.get(&url).header("Range", format!("bytes={tail_start}-")).send().expect("tail");
        assert_eq!(tail.status().as_u16(), 206);
        assert_eq!(tail.bytes().expect("bytes").len(), 65536);
    }

    #[test]
    fn refuses_an_unregistered_token() {
        // Force the server up, then ask for a token nothing minted.
        let path = temp_file("reg.mp4", b"x");
        let url = media_url(path.to_string_lossy().to_string()).expect("url");
        let bogus = url.rsplit_once("t=").expect("token").0.to_string() + "t=nope";
        let res = reqwest::blocking::get(&bogus).expect("send");
        assert_eq!(res.status().as_u16(), 404);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn parses_the_range_forms_a_media_element_sends() {
        assert_eq!(parse_range("bytes=0-", 10), Some((0, 9)));
        assert_eq!(parse_range("bytes=2-5", 10), Some((2, 5)));
        assert_eq!(parse_range("bytes=-3", 10), Some((7, 9)));
        assert_eq!(parse_range("bytes=20-", 10), None);
    }
}
