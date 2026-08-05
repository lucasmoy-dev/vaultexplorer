//! Backs the "Torrents" category in the desktop-only Internet experiment
//! (see InternetView.tsx). Two things this deliberately does NOT do:
//!
//! - No built-in tracker/piracy-site search index. `search` (see the
//!   `TorrentProvider` list below) only ships one built-in provider,
//!   Internet Archive -- a real, documented, keyless JSON API whose
//!   content is entirely public-domain/openly-licensed, and which (like
//!   Books, see webfind.rs) auto-generates a `.torrent` for nearly every
//!   item. A user can add their own provider (name + a search URL with a
//!   `{query}` placeholder) -- same idea as a browser's custom search
//!   engines, or any real torrent client's "search plugins": this app
//!   isn't the one deciding what that URL points to, same as a browser
//!   isn't responsible for every site its address bar can reach.
//! - No hand-rolled BitTorrent implementation. `librqbit` (a real,
//!   maintained Rust BT engine, also used by rqbit's own CLI/webui) does
//!   the actual protocol work, including its own HTTP range-request
//!   streaming server -- reused here as-is rather than re-implementing
//!   piece-priority + range serving from scratch.
//!
//! A custom provider's results come from scanning its response text for
//! literal `magnet:` URIs (same "no per-site adapter" trick a magnet link
//! affords: it's a self-contained, site-agnostic URI format, usually
//! including a `dn=` display-name param) -- crude, but it works
//! regardless of whether the page is JSON, HTML, or anything else, and
//! needs no site-specific scraping code.

use crate::errmap::ToStringErr;
use crate::progress::{ProgressEvent, ProgressReporter};
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Api, Session};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::OnceCell;

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}
fn config_dir() -> PathBuf {
    std::path::Path::new(&home_dir()).join(".config/vaultexplorer")
}
fn providers_path() -> PathBuf {
    config_dir().join("torrent_providers.json")
}
/// Where `list_only` previews and in-progress streams/downloads keep
/// their working state (rqbit's own resume data etc.) -- separate from
/// wherever the user actually wants a finished file to end up (see
/// `torrent_download`'s `dest_dir`, which copies out of here once done).
fn session_dir() -> PathBuf {
    config_dir().join("torrents")
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TorrentProvider {
    pub id: String,
    pub name: String,
    /// Empty/ignored for the builtin Internet Archive provider. For a
    /// custom one, `{query}` is replaced with the URL-encoded search term.
    pub url_template: String,
    pub builtin: bool,
}

fn default_providers() -> Vec<TorrentProvider> {
    vec![TorrentProvider {
        id: "internet_archive".to_string(),
        name: "Internet Archive".to_string(),
        url_template: String::new(),
        builtin: true,
    }]
}

pub fn list_providers() -> Vec<TorrentProvider> {
    std::fs::read_to_string(providers_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(default_providers)
}

fn save_providers(providers: &[TorrentProvider]) -> Result<(), String> {
    std::fs::create_dir_all(config_dir()).str_err()?;
    let json = serde_json::to_string_pretty(providers).str_err()?;
    std::fs::write(providers_path(), json).str_err()
}

#[tauri::command]
pub fn torrent_providers_list() -> Vec<TorrentProvider> {
    list_providers()
}

/// Short, non-cryptographic hash -- just enough to give a repeat "add"
/// of the same name+URL a stable id instead of piling up duplicates.
fn short_hash(s: &str) -> String {
    let mut h: u32 = 0;
    for b in s.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as u32);
    }
    format!("{h:x}")
}

#[tauri::command]
pub fn torrent_provider_add(name: String, url_template: String) -> Result<Vec<TorrentProvider>, String> {
    if !url_template.contains("{query}") {
        return Err("The search URL needs a {query} placeholder.".to_string());
    }
    let mut providers = list_providers();
    let id = format!("custom-{}", short_hash(&format!("{name}{url_template}")));
    providers.retain(|p| p.id != id);
    providers.push(TorrentProvider { id, name, url_template, builtin: false });
    save_providers(&providers)?;
    Ok(providers)
}

#[tauri::command]
pub fn torrent_provider_remove(id: String) -> Result<Vec<TorrentProvider>, String> {
    let mut providers = list_providers();
    providers.retain(|p| p.builtin || p.id != id);
    save_providers(&providers)?;
    Ok(providers)
}

#[derive(Serialize, Clone)]
pub struct TorrentSearchResult {
    pub title: String,
    /// A magnet URI or an http(s) URL to a `.torrent` file -- either is
    /// valid input to `AddTorrent::from_url`, so nothing downstream needs
    /// to care which kind this is.
    pub source_url: String,
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
        .build()
        .str_err()
}

async fn search_internet_archive(query: &str) -> Result<Vec<TorrentSearchResult>, String> {
    let mut url = reqwest::Url::parse("https://archive.org/advancedsearch.php").str_err()?;
    url.query_pairs_mut()
        .append_pair("q", &format!("{query} AND mediatype:(movies OR audio OR texts OR software)"))
        .append_pair("fl[]", "identifier")
        .append_pair("fl[]", "title")
        .append_pair("rows", "30")
        .append_pair("page", "1")
        .append_pair("output", "json");
    let json: serde_json::Value = http_client()?.get(url).send().await.str_err()?.json().await.str_err()?;
    let docs = json
        .get("response")
        .and_then(|r| r.get("docs"))
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(docs
        .into_iter()
        .filter_map(|d| {
            let identifier = d.get("identifier")?.as_str()?.to_string();
            let title = d.get("title").and_then(|t| t.as_str()).unwrap_or(&identifier).to_string();
            Some(TorrentSearchResult {
                title,
                source_url: format!("https://archive.org/download/{identifier}/{identifier}_archive.torrent"),
            })
        })
        .collect())
}

/// Extracts every distinct `magnet:` URI in `text` (a page from a custom
/// provider, whatever shape it's in), pairing each with its own `dn=`
/// (display name) parameter when present -- magnet URIs are self-
/// contained, so this needs no idea what site the text came from.
fn extract_magnets(text: &str) -> Vec<TorrentSearchResult> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel) = text[cursor..].find("magnet:?") {
        let start = cursor + rel;
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>'))
            .map(|e| start + e)
            .unwrap_or(text.len());
        let magnet = &text[start..end];
        cursor = end;
        if magnet.len() < 20 || !seen.insert(magnet.to_string()) {
            continue;
        }
        let title = magnet
            .split('&')
            .find_map(|p| p.strip_prefix("dn=").or_else(|| p.strip_prefix("xt=").map(|_| "")))
            .filter(|s| !s.is_empty())
            .and_then(|s| urlencoding_decode(s))
            .unwrap_or_else(|| format!("Torrent {}", out.len() + 1));
        out.push(TorrentSearchResult { title, source_url: magnet.to_string() });
        if out.len() >= 40 {
            break;
        }
    }
    out
}

/// Minimal `application/x-www-form-urlencoded`-style decode (`+` and
/// `%XX`) -- enough for a magnet's `dn=` param, without pulling in a full
/// URL-encoding crate just for this one field.
fn urlencoding_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Percent-encodes `s` for safe use inside a URL query -- just the
/// characters that actually need it for a search term (spaces, `&`, `?`,
/// `#`, `%`, non-ASCII), not a full RFC 3986 implementation.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn search_custom_provider(url_template: &str, query: &str) -> Result<Vec<TorrentSearchResult>, String> {
    let url = url_template.replace("{query}", &percent_encode(query));
    let text = http_client()?.get(url).send().await.str_err()?.text().await.str_err()?;
    Ok(extract_magnets(&text))
}

#[tauri::command]
pub async fn torrent_search(provider_id: String, query: String) -> Result<Vec<TorrentSearchResult>, String> {
    let providers = list_providers();
    let provider = providers.iter().find(|p| p.id == provider_id).ok_or("Unknown provider")?;
    if provider.builtin {
        search_internet_archive(&query).await
    } else {
        search_custom_provider(&provider.url_template, &query).await
    }
}

// ---- The BitTorrent engine itself ----

struct Engine {
    session: std::sync::Arc<Session>,
    /// The port rqbit's own HTTP API (incl. its range-request file
    /// streaming endpoint, `GET /torrents/{id}/stream/{file_id}`) is
    /// listening on, bound to 127.0.0.1 only -- not reachable from
    /// outside this machine, same trust boundary as any other loopback
    /// dev server.
    stream_port: u16,
}

static ENGINE: OnceCell<Result<Engine, String>> = OnceCell::const_new();

async fn engine() -> Result<&'static Engine, String> {
    let result = ENGINE
        .get_or_init(|| async {
            std::fs::create_dir_all(session_dir()).str_err()?;
            let session = Session::new(session_dir()).await.map_err(|e| e.to_string())?;
            let api = Api::new(session.clone(), None, None);
            let http_api = librqbit::http_api::HttpApi::new(api, None);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.str_err()?;
            let stream_port = listener.local_addr().str_err()?.port();
            tokio::spawn(http_api.make_http_api_and_run(listener, None));
            Ok(Engine { session, stream_port })
        })
        .await;
    result.as_ref().map_err(|e| e.clone())
}

#[derive(Serialize)]
pub struct TorrentFile {
    pub index: usize,
    pub name: String,
    pub length: u64,
}

/// Fetches a torrent's file list without downloading anything -- same
/// "shows as files, nothing fetched until you open one" rule the rest of
/// Internet already follows.
#[tauri::command]
pub async fn torrent_list_files(source_url: String) -> Result<Vec<TorrentFile>, String> {
    let eng = engine().await?;
    let resp = eng
        .session
        .add_torrent(
            AddTorrent::from_url(source_url.as_str()),
            Some(AddTorrentOptions { list_only: true, ..Default::default() }),
        )
        .await
        .map_err(|e| e.to_string())?;
    let AddTorrentResponse::ListOnly(list) = resp else {
        return Err("Unexpected response listing this torrent's files.".to_string());
    };
    let details = list.info.iter_file_details().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (index, d) in details.enumerate() {
        out.push(TorrentFile {
            index,
            name: d.filename.to_string().map_err(|e| e.to_string())?,
            length: d.len,
        });
    }
    Ok(out)
}

/// Starts (or resumes an already-started) download of just `file_index`
/// and returns the local URL to open it at -- rqbit prioritizes pieces in
/// playback order for a file being actively read this way, so an OS media
/// player pointed at this URL can start playing before the file finishes
/// downloading.
#[tauri::command]
pub async fn torrent_stream_url(source_url: String, file_index: usize) -> Result<String, String> {
    let eng = engine().await?;
    let resp = eng
        .session
        .add_torrent(
            AddTorrent::from_url(source_url.as_str()),
            Some(AddTorrentOptions { only_files: Some(vec![file_index]), overwrite: true, ..Default::default() }),
        )
        .await
        .map_err(|e| e.to_string())?;
    let id = match resp {
        AddTorrentResponse::Added(id, _) | AddTorrentResponse::AlreadyManaged(id, _) => id,
        AddTorrentResponse::ListOnly(_) => return Err("bug: list_only wasn't requested".to_string()),
    };
    Ok(format!("http://127.0.0.1:{}/torrents/{id}/stream/{file_index}", eng.stream_port))
}

/// Downloads `file_index` fully into `dest_dir`, reporting progress the
/// same way any other long-running fs op does (see progress.rs) -- shows
/// up in the same Tasks footer as a copy/move/compress.
#[tauri::command]
pub async fn torrent_download(
    source_url: String,
    file_index: usize,
    dest_dir: String,
    channel: tauri::ipc::Channel<ProgressEvent>,
) -> Result<String, String> {
    let eng = engine().await?;
    let resp = eng
        .session
        .add_torrent(
            AddTorrent::from_url(source_url.as_str()),
            Some(AddTorrentOptions {
                only_files: Some(vec![file_index]),
                output_folder: Some(dest_dir.clone()),
                overwrite: true,
                ..Default::default()
            }),
        )
        .await
        .map_err(|e| e.to_string())?;
    let handle = resp.into_handle().ok_or("Unexpected response downloading this file.")?;

    let file_name = handle
        .with_metadata(|m| m.info.iter_file_details().ok().and_then(|mut it| it.nth(file_index)).map(|d| d.filename.to_string().unwrap_or_default()))
        .ok()
        .flatten()
        .unwrap_or_else(|| "file".to_string());

    let stats = handle.stats();
    let reporter = ProgressReporter::new(channel, stats.total_bytes.max(1));
    loop {
        let stats = handle.stats();
        reporter.report(stats.progress_bytes);
        if stats.finished {
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Ok(file_name)
}
