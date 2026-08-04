//! Backs the desktop-only "Internet" sidebar experiment (see App.tsx's
//! `InternetView`): a fake "Videos" folder full of YouTube search results
//! and a fake "Images" folder full of web image search results, browsed
//! like any other folder even though nothing here is a real file.
//!
//! Neither YouTube nor DuckDuckGo offer this without either an API key
//! (YouTube Data API -- needs a Google Cloud project and billing, not
//! something to provision on someone else's behalf mid-experiment) or a
//! paid plan (Bing/Google Image Search APIs). Both commands below instead
//! read the same public search pages a browser would, unauthenticated --
//! `search_youtube` picks `videoId`/title pairs out of the page's own
//! embedded JSON (the same technique most no-API-key YouTube search tools
//! use), and `search_images` replicates DuckDuckGo's own image tab request
//! (fetch the page for a `vqd` token, then call its `i.js` JSON endpoint
//! with it). Both are unofficial, undocumented, and can break the moment
//! either site changes its markup/endpoint -- acceptable for an
//! experiment explicitly framed as "might not work out", not something
//! to build a real feature's only code path on.

use crate::errmap::ToStringErr;
use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct YoutubeResult {
    pub id: String,
    pub title: String,
    pub thumbnail: String,
    pub duration: Option<String>,
    pub published: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct ImageResult {
    pub title: String,
    pub thumbnail: String,
    pub image: String,
    pub source_url: String,
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        // A default reqwest user-agent gets these unofficial endpoints
        // blocked outright by some CDN-level bot filtering; a plain
        // browser-shaped one is enough to pass.
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
        .build()
        .str_err()
}

/// Reads one JSON string literal starting right after its opening quote
/// (i.e. `s` is positioned just past the `"`), respecting backslash
/// escapes rather than stopping at the first `"` seen -- a title
/// containing an escaped quote would otherwise truncate. Delegates the
/// actual unescaping to `serde_json` rather than hand-rolling `\uXXXX`
/// handling. Returns the decoded string and how many bytes of `s` it
/// consumed (including the closing quote).
fn json_string_at(s: &str) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut escaped = false;
    while i < bytes.len() {
        let c = bytes[i];
        if escaped {
            escaped = false;
        } else if c == b'\\' {
            escaped = true;
        } else if c == b'"' {
            let raw = &s[..i];
            let decoded: String = serde_json::from_str(&format!("\"{raw}\"")).ok()?;
            return Some((decoded, i + 1));
        }
        i += 1;
    }
    None
}

/// YouTube's `sp=` search-page param is a base64'd protobuf (undocumented,
/// reverse-engineered by many scrapers) picking sort order plus a nested
/// "filters" message. Hand-encoding it (instead of a lookup table of known
/// single-filter values) is what lets sort + upload-date + duration
/// combine freely -- verified live against youtube.com/results: combining
/// upload_date=week with duration=long only ever returned results both
/// long *and* from the last few hours.
///   top-level:  field 1 (varint)      = sort_by       (2 = upload date)
///               field 2 (length-delim) = filters, containing:
///                 field 1 (varint) = upload_date (1=hour,2=today,3=week,4=month,5=year)
///                 field 3 (varint) = duration    (1=short<4m,2=long>20m,3=medium 4-20m)
fn youtube_sp_param(sort_by_date: bool, upload_date: Option<u8>, duration: Option<u8>) -> Option<String> {
    let mut filters = Vec::new();
    if let Some(d) = upload_date {
        filters.extend([0x08, d]);
    }
    if let Some(d) = duration {
        filters.extend([0x18, d]);
    }
    let mut out = Vec::new();
    if sort_by_date {
        out.extend([0x08, 0x02]);
    }
    if !filters.is_empty() {
        out.push(0x12);
        out.push(filters.len() as u8);
        out.extend(filters);
    }
    if out.is_empty() {
        return None;
    }
    use base64::Engine;
    Some(base64::engine::general_purpose::URL_SAFE.encode(out))
}

#[cfg(desktop)]
#[tauri::command]
pub(crate) fn search_youtube(
    query: String,
    sort_by_date: bool,
    upload_date: Option<u8>,
    duration: Option<u8>,
) -> Result<Vec<YoutubeResult>, String> {
    let mut url = reqwest::Url::parse("https://www.youtube.com/results").str_err()?;
    url.query_pairs_mut().append_pair("search_query", &query);
    if let Some(sp) = youtube_sp_param(sort_by_date, upload_date, duration) {
        url.query_pairs_mut().append_pair("sp", &sp);
    }
    let html = http_client()?.get(url).send().str_err()?.text().str_err()?;

    let marker = "\"videoId\":\"";
    let title_marker = "\"title\":{\"runs\":[{\"text\":\"";
    let length_marker = "\"simpleText\":\"";
    let published_marker = "\"publishedTimeText\":{\"simpleText\":\"";
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cursor = 0usize;
    while results.len() < 30 {
        let Some(rel) = html[cursor..].find(marker) else { break };
        let start = cursor + rel + marker.len();
        let Some((id, consumed)) = json_string_at(&html[start..]) else { break };
        cursor = start + consumed;
        // Real video IDs are always 11 chars -- filters out unrelated
        // "videoId" fields (ads, mix/playlist placeholders) that don't
        // have a nearby title in this same shape.
        if id.len() != 11 || !seen.insert(id.clone()) {
            continue;
        }
        let window_end = (cursor + 4000).min(html.len());
        let window = &html[cursor..window_end];
        if let Some(tpos) = window.find(title_marker) {
            let tstart = tpos + title_marker.len();
            if let Some((title, _)) = json_string_at(&window[tstart..]) {
                // Both are cosmetic (shown next to the result, not used to
                // re-filter client-side) -- a miss just leaves them blank
                // rather than dropping the result.
                let duration = window
                    .find("\"lengthText\":")
                    .and_then(|p| window[p..].find(length_marker).map(|q| p + q + length_marker.len()))
                    .and_then(|s| json_string_at(&window[s..]))
                    .map(|(s, _)| s);
                let published = window
                    .find(published_marker)
                    .map(|p| p + published_marker.len())
                    .and_then(|s| json_string_at(&window[s..]))
                    .map(|(s, _)| s);
                results.push(YoutubeResult {
                    thumbnail: format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"),
                    id,
                    title,
                    duration,
                    published,
                });
            }
        }
    }
    Ok(results)
}

#[cfg(desktop)]
#[tauri::command]
pub(crate) fn search_images(query: String) -> Result<Vec<ImageResult>, String> {
    let client = http_client()?;

    let mut search_url = reqwest::Url::parse("https://duckduckgo.com/").str_err()?;
    search_url.query_pairs_mut().append_pair("q", &query);
    let html = client.get(search_url).send().str_err()?.text().str_err()?;
    let vqd_pos = html.find("vqd=").ok_or("couldn't find a search token on the results page")?;
    let after = &html[vqd_pos + 4..];
    let quote = after.chars().next().ok_or("malformed search token")?;
    let after_quote = &after[quote.len_utf8()..];
    let end = after_quote.find(quote).ok_or("malformed search token")?;
    let vqd = &after_quote[..end];

    let mut img_url = reqwest::Url::parse("https://duckduckgo.com/i.js").str_err()?;
    img_url
        .query_pairs_mut()
        .append_pair("l", "us-en")
        .append_pair("o", "json")
        .append_pair("q", &query)
        .append_pair("vqd", vqd)
        .append_pair("f", ",,,")
        .append_pair("p", "1");
    let json: serde_json::Value = client.get(img_url).send().str_err()?.json().str_err()?;
    let results = json.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();
    Ok(results
        .into_iter()
        .filter_map(|r| {
            Some(ImageResult {
                title: r.get("title")?.as_str()?.to_string(),
                thumbnail: r.get("thumbnail")?.as_str()?.to_string(),
                image: r.get("image")?.as_str()?.to_string(),
                source_url: r.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string(),
            })
        })
        .take(40)
        .collect())
}
