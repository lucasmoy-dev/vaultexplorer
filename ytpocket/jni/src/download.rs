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
///
/// **Bound to IPv4 on purpose.** A googlevideo URL carries the client's IP
/// in its signed parameters (`ip=…`, listed in `sparams`), so requests from
/// a different address are refused with 403. On a phone that is a live
/// hazard: Android's IPv6 privacy addresses rotate, and a dual-stack device
/// can resolve over IPv6 and then open the next connection over IPv4 — which
/// is exactly the "worked for the first few megabytes, then 403" failure
/// reported from a real device, in every request shape. An IPv4 address on a
/// phone is a carrier NAT address that stays put for the session, so pinning
/// the family makes the URL's IP and our IP agree for the whole download.
///
/// If the network has no IPv4 at all, the bind fails and the unbound client
/// is used instead — being consistent is the goal, not being IPv4.
pub(crate) fn http() -> Result<&'static reqwest::blocking::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::blocking::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            let base = || {
                reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(120))
                    // No redirect chasing: a googlevideo URL is final, and a
                    // redirect here would mean something is wrong (a captive
                    // portal, an ISP interstitial) rather than something to
                    // follow silently.
                    .redirect(reqwest::redirect::Policy::none())
            };
            let ipv4 = base()
                .local_address(Some(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)))
                .build();
            match ipv4 {
                Ok(client) => Ok(client),
                Err(_) => base().build().map_err(|e| e.to_string()),
            }
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// The IP googlevideo signed into a stream URL, if it is in there.
///
/// Worth surfacing rather than keeping as trivia: when this does not match
/// the address the phone is actually using, every request is refused, and
/// saying so ("tu IP cambió durante la descarga") is the difference between
/// a fixable report and "no funciona".
pub fn signed_ip(url: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    let raw = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("ip="))?;
    // Percent-decoding, for the colons in an IPv6 address.
    Some(raw.replace("%3A", ":").replace("%3a", ":"))
}

/// The address this machine is actually reaching the internet from, through
/// the same client the downloads use.
pub fn egress_ip() -> Option<String> {
    let text = http().ok()?.get("https://api.ipify.org").send().ok()?.text().ok()?;
    let text = text.trim().to_string();
    (!text.is_empty() && text.len() < 64).then_some(text)
}

/// How to ask googlevideo for a byte range.
///
/// There is more than one way, and which one is accepted has turned out to
/// depend on the network: a `Range` header is the obvious HTTP thing, while
/// YouTube's own DASH players put the range in the **query string** and add a
/// `cpn` (client playback nonce) that identifies the playback session. When a
/// download is refused mid-transfer on one network and works on another, the
/// shape of the request is the first thing worth varying -- so all three are
/// implemented and can be measured (see the `diag` example) instead of
/// guessed at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RangeMode {
    /// `Range: bytes=a-b`.
    Header,
    /// `&range=a-b` appended to the URL, the way a DASH player does it.
    Query,
    /// `&range=a-b&cpn=…`, which is what a real client sends.
    QueryWithNonce,
}

impl RangeMode {
    pub fn label(self) -> &'static str {
        match self {
            RangeMode::Header => "header",
            RangeMode::Query => "query",
            RangeMode::QueryWithNonce => "query+cpn",
        }
    }
}

/// The mode used by default. Kept in one place so a measurement can change
/// the app's behaviour by changing one line.
pub const DEFAULT_RANGE_MODE: RangeMode = RangeMode::Header;

/// A client playback nonce: 16 characters from the alphabet YouTube's own
/// players use. Generated once per process, like a playback session.
fn playback_nonce() -> &'static str {
    static NONCE: OnceLock<String> = OnceLock::new();
    NONCE.get_or_init(|| {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
        // No rand dependency for 16 characters: the point is that it is
        // unique per session, not that it is unpredictable.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut state = seed as u64 | 1;
        (0..16)
            .map(|_| {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ALPHABET[(state >> 33) as usize % ALPHABET.len()] as char
            })
            .collect()
    })
}

/// Build the request for one range, in the given shape.
fn ranged_request(
    url: &str,
    offset: u64,
    end: u64,
    user_agent: &str,
    mode: RangeMode,
) -> Result<reqwest::blocking::RequestBuilder, String> {
    let client = http()?;
    Ok(match mode {
        RangeMode::Header => client
            .get(url)
            .header(reqwest::header::USER_AGENT, user_agent)
            .header(reqwest::header::RANGE, format!("bytes={offset}-{end}")),
        RangeMode::Query => client
            .get(format!("{url}&range={offset}-{end}"))
            .header(reqwest::header::USER_AGENT, user_agent),
        RangeMode::QueryWithNonce => client
            .get(format!("{url}&range={offset}-{end}&cpn={}", playback_nonce()))
            .header(reqwest::header::USER_AGENT, user_agent),
    })
}

/// Ask for the first kilobyte, to find out whether this URL will actually
/// serve bytes to us. Used by `resolve` before it hands a URL over.
pub fn probe(url: &str, user_agent: &str) -> Result<(), String> {
    // A *chunk-sized* range on purpose. Measured: URLs from a client that
    // needs a PO token answer `bytes=0-1023` with 200 and the same URL
    // answers `bytes=0-4194303` with 403 -- so the old 1KB probe passed
    // exactly the streams that then failed on the first real chunk. The
    // response body is dropped without reading it, so this still costs one
    // round trip and not 4MB.
    let response = http()?
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .header(reqwest::header::RANGE, format!("bytes=0-{}", CHUNK_PROBE - 1))
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

/// The range a probe asks for: the same size the app's first real chunk asks
/// for, because that is the request that has to be allowed.
const CHUNK_PROBE: u64 = 4 * 1024 * 1024;

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
    // Try the other request shapes before giving up on this chunk.
    //
    // Which shape googlevideo accepts has proven to be network-dependent, and
    // the cost of being wrong is a failed download rather than a slow one --
    // so a refusal retries the same bytes asked for a different way. All three
    // succeed from every network measured so far; this exists for the network
    // where one of them does not.
    let mut last = String::new();
    for mode in [DEFAULT_RANGE_MODE, RangeMode::Query, RangeMode::QueryWithNonce] {
        match chunk_with(url, path, offset, max_bytes, user_agent, mode) {
            Ok(written) => return Ok(written),
            Err(error) => last = format!("{} ({})", error, mode.label()),
        }
    }
    Err(last)
}

/// [`chunk`], with the request shape spelled out -- for measuring which
/// shape a given network accepts.
pub fn chunk_with(
    url: &str,
    path: &str,
    offset: u64,
    max_bytes: u64,
    user_agent: &str,
    mode: RangeMode,
) -> Result<u64, String> {
    let end = offset + max_bytes - 1;
    let response = ranged_request(url, offset, end, user_agent, mode)?
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
mod address_tests {
    use super::*;

    /// The invariant that a phone kept breaking: googlevideo signs the
    /// caller's IP into the stream URL (it is listed in `sparams`, so it is
    /// verified), which means the address that resolves and the address that
    /// downloads have to be the same one.
    ///
    /// Before the client was pinned to one family, this machine resolved over
    /// IPv6 and could download over IPv4 -- fine here, fatal on a device whose
    /// IPv6 privacy address rotates mid-transfer: 403 on every request shape,
    /// which is exactly what was reported. `#[ignore]`d (network).
    #[test]
    #[ignore]
    fn the_url_is_signed_for_the_address_we_call_from() {
        let resolved = crate::youtube::resolve("dQw4w9WgXcQ").expect("resolve");
        let audio = resolved.audio.expect("audio");
        let signed = signed_ip(&audio.url).expect("no ip in the stream url");
        let egress = egress_ip().expect("could not determine our egress ip");
        println!("signed for {signed}, calling from {egress}");
        assert_eq!(signed, egress, "the URL is bound to an address we are not using");
    }

    /// Recovering from a refusal by resolving again and *continuing* is what
    /// makes a rotating address survivable: the bytes already on disk stay,
    /// and only the URL is replaced. This proves a fresh URL accepts a
    /// request at an arbitrary offset. `#[ignore]`d (network).
    #[test]
    #[ignore]
    fn a_fresh_url_continues_where_the_old_one_stopped() {
        let first_resolve = crate::youtube::resolve("dQw4w9WgXcQ").expect("resolve");
        let audio = first_resolve.audio.expect("audio");
        let path = std::env::temp_dir().join("ytpocket-resume-test.bin");
        let _ = std::fs::remove_file(&path);
        let file = path.to_string_lossy().to_string();

        let first = chunk(&audio.url, &file, 0, 1024 * 1024, &first_resolve.user_agent)
            .expect("first chunk");
        assert!(first > 0);

        let second_resolve = crate::youtube::resolve("dQw4w9WgXcQ").expect("re-resolve");
        let fresh = second_resolve.audio.expect("audio again");
        let second = chunk(&fresh.url, &file, first, 1024 * 1024, &second_resolve.user_agent)
            .expect("resumed chunk");
        assert!(second > 0);

        // One file, both halves, in order.
        assert_eq!(std::fs::metadata(&path).unwrap().len(), first + second);
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

/// YouTube's own words when it will not serve a network at all, whatever
/// client asks. Worth naming: it is the difference between "this build is
/// broken" and "this address is banned".
#[cfg(test)]
fn is_bot_gated(message: &str) -> bool {
    message.contains("not a bot") || message.contains("ip-ban")
}

#[cfg(test)]
mod volume_tests {
    use super::*;
    use super::is_bot_gated;

    /// Every live test so far downloaded a few MB. The device failed at
    /// ~40MB. So: take a long video and pull the whole audio track through
    /// the same chunk loop the app uses, and report where -- if anywhere --
    /// googlevideo starts refusing. `#[ignore]`d (network, slow, ~50MB).
    #[test]
    #[ignore]
    fn a_long_download_is_not_refused_part_way() {
        let results = crate::youtube::search("full album 1 hour", 20).expect("search");
        let pick = results
            .iter()
            // No duration means a livestream, which has no file to download.
            .filter(|r| r.duration.unwrap_or(0) > 20 * 60)
            .max_by_key(|r| r.duration.unwrap_or(0))
            .expect("no long-enough results");
        println!("picked {} ({:?}s) {}", pick.title, pick.duration, pick.id);

        let resolved = match crate::youtube::resolve(&pick.id) {
            Ok(resolved) => resolved,
            Err(refusal) if is_bot_gated(&refusal) => {
                println!("this network is bot-gated for everything: {refusal}");
                return;
            }
            Err(other) => panic!("resolve: {other}"),
        };
        let audio = resolved.audio.as_ref().expect("audio");
        println!("client {}, declared {} bytes", resolved.client, audio.size);

        let path = std::env::temp_dir().join("ytpocket-volume-test.bin");
        let _ = std::fs::remove_file(&path);
        let file = path.to_string_lossy().to_string();

        let total = total_size(&audio.url, &resolved.user_agent).unwrap_or(audio.size);
        let mut done = 0u64;
        let mut chunks = 0;
        while done < total {
            let want = (4 * 1024 * 1024).min(total - done);
            match chunk(&audio.url, &file, done, want, &resolved.user_agent) {
                Ok(0) => panic!("empty reply at {done} of {total} after {chunks} chunks"),
                Ok(n) => {
                    done += n;
                    chunks += 1;
                    if chunks % 5 == 0 {
                        println!("  {done}/{total} bytes ({chunks} chunks)");
                    }
                }
                Err(e) => panic!("refused at {done} of {total} after {chunks} chunks: {e}"),
            }
        }
        println!("finished {done} bytes in {chunks} chunks");
        assert_eq!(done, total);
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod client_matrix {
    use super::*;
    use super::is_bot_gated;
    use crate::innertube;

    /// The two findings behind the 403, both asserted.
    ///
    /// 1. A **token-free client needs a visitor id.** Without one YouTube
    ///    answers "Sign in to confirm you're not a bot" for anything long or
    ///    popular, and the app fell back to a client that needs a PO token.
    /// 2. A **1KB probe proves nothing.** Those fallback URLs answer a small
    ///    range with 200 and a chunk-sized one with 403, which is why every
    ///    download died on its first real chunk while the probe was happy.
    ///
    /// `#[ignore]`d (network).
    #[test]
    #[ignore]
    fn a_visitor_id_is_what_makes_a_token_free_client_serve_bytes() {
        // A long, popular video: the bot gate does not trigger on obscure
        // ones, which is why smaller tests never caught this.
        let results = crate::youtube::search("full album 1 hour", 20).expect("search");
        let pick = results
            .iter()
            .filter(|r| r.duration.unwrap_or(0) > 20 * 60)
            .max_by_key(|r| r.duration.unwrap_or(0))
            .expect("no long-enough results");
        println!("video: {} ({:?}s) {}", pick.title, pick.duration, pick.id);

        let visitor = innertube::visitor().expect("no visitor id");
        let with = match innertube::player_as(innertube::VISIONOS, &pick.id, Some(&visitor)) {
            Ok(player) => player,
            Err(refusal) if is_bot_gated(&refusal) => {
                // Datacenter addresses are refused outright, visitor id or
                // not -- YouTube calls it an ip-ban. Nothing about the app can
                // change that, so this network cannot answer the question.
                println!("this network is bot-gated for everything: {refusal}");
                return;
            }
            Err(other) => panic!("visionos refused a request carrying a visitor id: {other}"),
        };
        let audio = with.best_audio().expect("no audio formats");

        let path = std::env::temp_dir().join("ytpocket-matrix.bin");
        let _ = std::fs::remove_file(&path);
        let file = path.to_string_lossy().to_string();
        let served = chunk(&audio.url, &file, 0, 4 * 1024 * 1024, innertube::VISIONOS.user_agent)
            .expect("a real chunk was refused");
        assert_eq!(served, 4 * 1024 * 1024, "short chunk");
        let _ = std::fs::remove_file(&path);

        // And the same request without one is refused, which is the part that
        // was shipping. If this ever stops being true, YouTube relaxed its bot
        // gate and the visitor id became belt-and-braces rather than load
        // bearing -- worth knowing either way.
        match innertube::player_as(innertube::VISIONOS, &pick.id, None) {
            Err(refusal) => println!("without a visitor id: {refusal}"),
            Ok(player) => println!(
                "without a visitor id: allowed after all ({} formats)",
                player.formats.len()
            ),
        }
    }
}
