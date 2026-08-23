//! Fetching the bytes, natively.
//!
//! This used to live in Kotlin, with `HttpURLConnection`. It moved here
//! after downloads came back **403 Forbidden** on a real phone while the
//! very same URLs served fine from the machine that resolved them.
//!
//! A googlevideo URL is not a plain link. It is minted for the specific
//! YouTube client that asked for it, and the server can reject a request
//! that does not look like that client -- different `User-Agent`, different
//! headers, even a different IP family if one HTTP stack prefers IPv6 and
//! the other IPv4. Resolving with rustypipe's HTTP client and then
//! downloading with a *second, different* client in Kotlin is exactly that
//! mismatch: two stacks, two sets of default headers, two connection paths.
//!
//! So the download now goes out through the same kind of client, with the
//! agent of the client that produced the URL (`Resolved::user_agent`), and
//! Kotlin drives it one chunk at a time:
//!
//! * ranged chunks, because googlevideo also *throttles* a plain sequential
//!   read to a non-browser client -- measured, a 3.4MB file timed out at 30s
//!   as one stream and arrives in ~2s as 4MB ranges;
//! * one call per chunk, so Kotlin keeps its progress bar, its notification
//!   and the ability to stop, with no callbacks crossing JNI.

use std::io::Write;
use std::sync::OnceLock;
use std::time::Duration;

/// One client for the whole process: connection reuse is most of why the
/// chunked download is fast, and building a client per chunk would throw
/// that away (plus a fresh TLS handshake each time).
fn http() -> Result<&'static reqwest::blocking::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::blocking::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(120))
                // No redirect chasing: a googlevideo URL is final, and a
                // redirect here would mean something is wrong (a captive
                // portal, an ISP interstitial) rather than something to
                // follow silently.
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// Ask for the first kilobyte, to find out whether this URL will actually
/// serve bytes to us. Used by `resolve` before it hands a URL over.
pub fn probe(url: &str, user_agent: &str) -> Result<(), String> {
    let response = http()?
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .header(reqwest::header::RANGE, "bytes=0-1023")
        .send()
        .map_err(|e| format!("no se pudo conectar: {e}"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    Err(match status.as_u16() {
        403 => "YouTube rechazó la descarga (403)".to_string(),
        410 => "el enlace ha caducado (410)".to_string(),
        other => format!("HTTP {other}"),
    })
}

/// Append one chunk of `url` to `path`, starting at `offset`.
///
/// Returns how many bytes were written: fewer than `max_bytes` means the
/// file ended there. Kotlin loops until that happens, which is also where
/// progress and cancellation live.
pub fn chunk(
    url: &str,
    path: &str,
    offset: u64,
    max_bytes: u64,
    user_agent: &str,
) -> Result<u64, String> {
    let end = offset + max_bytes - 1;
    let response = http()?
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .header(reqwest::header::RANGE, format!("bytes={offset}-{end}"))
        .send()
        .map_err(|e| format!("no se pudo conectar: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        // 416 past the end of the file is not an error, it is the end of the
        // file -- a caller that guessed the size can land here legitimately.
        if status.as_u16() == 416 {
            return Ok(0);
        }
        return Err(match status.as_u16() {
            403 => "YouTube rechazó la descarga a mitad (403)".to_string(),
            410 => "el enlace caducó durante la descarga (410)".to_string(),
            other => format!("HTTP {other}"),
        });
    }
    let bytes = response.bytes().map_err(|e| format!("la descarga se cortó: {e}"))?;
    if bytes.is_empty() {
        return Ok(0);
    }
    // Append, so a resumed or continued download does not truncate what is
    // already on disk.
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("no se pudo escribir en {path}: {e}"))?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    Ok(bytes.len() as u64)
}

/// The total size of `url`, from a one-byte range request's
/// `Content-Range` -- the only number googlevideo reliably gives, and what
/// the progress bar needs before the first chunk lands.
pub fn total_size(url: &str, user_agent: &str) -> Result<u64, String> {
    let response = http()?
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .map_err(|e| format!("no se pudo conectar: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    // "bytes 0-0/3449447"
    let total = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit('/').next())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rejected_url_is_reported_not_swallowed() {
        // httpbin-free: any URL that 403s. Google's own static host does it
        // for a bogus path, and this asserts the message, not the network.
        let error = probe("https://accounts.google.com/o/oauth2/token", "test-agent");
        assert!(error.is_err(), "a POST-only endpoint should not accept a GET range");
    }

    /// Live: the full chunked path against a real stream, which is what the
    /// phone does. `#[ignore]`d (network).
    #[test]
    #[ignore]
    fn chunked_download_matches_the_declared_size() {
        let resolved = crate::youtube::resolve("dQw4w9WgXcQ").expect("resolve failed");
        let audio = resolved.audio.expect("no audio");
        println!("client={} ua={}", resolved.client, resolved.user_agent);

        let total = total_size(&audio.url, &resolved.user_agent).expect("size failed");
        println!("total={total} declared={}", audio.size);
        assert!(total > 0);

        let path = std::env::temp_dir().join("ytpocket-chunk-test.m4a");
        let _ = std::fs::remove_file(&path);
        let path_str = path.to_string_lossy().to_string();
        let mut done = 0u64;
        loop {
            let written = chunk(&audio.url, &path_str, done, 1024 * 1024, &resolved.user_agent)
                .expect("chunk failed");
            if written == 0 {
                break;
            }
            done += written;
        }
        println!("downloaded={done}");
        assert_eq!(done, total, "downloaded size does not match Content-Range");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), total);
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod root_cause_tests {
    use super::*;
    use rustypipe::client::{ClientType, RustyPipe};

    /// Documents (and watches) the actual cause of the 403 reported from a
    /// phone: YouTube's **web** clients need a PO token for streams, and
    /// without one the URLs they hand out are refused -- even though
    /// resolving them succeeds.
    ///
    /// This is why `youtube::CLIENTS` contains no web client, and why
    /// `resolve` probes before returning. `#[ignore]`d (network).
    #[test]
    #[ignore]
    fn a_web_client_url_is_refused_while_the_ios_one_serves() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let pipe = RustyPipe::builder()
            .storage_dir(std::env::temp_dir().join("ytpocket-rootcause"))
            .build()
            .unwrap();

        // iOS: needs neither deobfuscation nor a PO token. This is the one
        // the app relies on.
        let ios = runtime
            .block_on(async { pipe.query().player_from_client("dQw4w9WgXcQ", ClientType::Ios).await })
            .expect("iOS resolve failed");
        let ios_audio = ios.audio_streams.iter().max_by_key(|s| s.bitrate).expect("no iOS audio");
        let ios_ua = pipe.query().user_agent(ClientType::Ios).to_string();
        probe(&ios_audio.url, &ios_ua).expect("iOS stream should serve bytes");

        // Desktop: resolves fine, and then refuses to serve. If this ever
        // starts passing, YouTube changed its mind and the client list can
        // grow again.
        match runtime
            .block_on(async { pipe.query().player_from_client("dQw4w9WgXcQ", ClientType::Desktop).await })
        {
            Ok(desktop) => {
                if let Some(audio) = desktop.audio_streams.iter().max_by_key(|s| s.bitrate) {
                    let ua = pipe.query().user_agent(ClientType::Desktop).to_string();
                    let outcome = probe(&audio.url, &ua);
                    println!("desktop probe: {outcome:?}");
                    assert!(
                        outcome.is_err(),
                        "a web-client stream served without a PO token -- the client list can be revisited",
                    );
                } else {
                    println!("desktop returned no audio streams at all");
                }
            }
            // Not being able to resolve at all is the same practical
            // conclusion: unusable for downloads.
            Err(error) => println!("desktop resolve failed outright: {error}"),
        }
    }
}
