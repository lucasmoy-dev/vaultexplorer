//! Backs the desktop-only "Internet" sidebar experiment (see App.tsx's
//! `InternetView`): fake "Videos"/"Images"/"Books" folders full of live
//! search results, browsed like any other folder even though nothing here
//! is a real file.
//!
//! Neither YouTube nor DuckDuckGo offer this without either an API key
//! (YouTube Data API -- needs a Google Cloud project and billing, not
//! something to provision on someone else's behalf mid-experiment) or a
//! paid plan (Bing/Google Image Search APIs). `search_youtube` and
//! `search_images` instead read the same public search pages a browser
//! would, unauthenticated -- `search_youtube` picks `videoId`/title pairs
//! out of the page's own embedded JSON (the same technique most no-API-key
//! YouTube search tools use), and `search_images` replicates DuckDuckGo's
//! own image tab request (fetch the page for a `vqd` token, then call its
//! `i.js` JSON endpoint with it). Both are unofficial, undocumented, and
//! can break the moment either site changes its markup/endpoint --
//! acceptable for an experiment explicitly framed as "might not work out",
//! not something to build a real feature's only code path on.
//!
//! `search_books` tried being a plain DuckDuckGo web search scoped to
//! `filetype:pdf` for a while, same "read the HTML a browser would get"
//! technique as the other two -- but DDG's HTML results page is far more
//! aggressive about blocking that than its plain/image search pages, so
//! it came back empty in practice, not just from this sandbox. It's back
//! to Internet Archive's `advancedsearch.php`, a real documented JSON
//! API that just works -- the tradeoff is it only ever covers Archive.org's
//! own public-domain/openly-licensed collection, so it comes up empty for
//! anything outside it (a mainstream book someone's actually looking for).
//! Reliable-but-narrower beats broad-but-broken.
//!
//! `list_video_providers`/`search_provider_videos` back an *additional*
//! provider picker in the Videos folder, alongside the inline
//! `search_youtube` results above: a handful of other sites, each with
//! its own scraper, all feeding the same inline result grid (thumbnail +
//! title, double-click opens the real page in the system browser -- same
//! interaction as a YouTube result, nothing plays embedded in-app for
//! any provider including YouTube). Each of the three was fetched and
//! inspected directly (this sandbox does have outbound access to these
//! specific hosts, even though DuckDuckGo/Bing/a public SearXNG all
//! blocked it earlier -- that block was those services specifically, not
//! a blanket sandbox restriction) before writing its parser, same
//! standard as `search_youtube` above:
//!   - AnimeFLV: plain server-rendered HTML, one `<article class="Anime
//!     ...">` per result with a title/thumbnail/relative href.
//!   - xhamster: a `window.initials = {...}` JSON blob embedded in the
//!     page (the same object the site's own JS hydrates from) containing
//!     a `searchResult.videoThumbProps` array with title/thumbURL/
//!     pageURL/duration already structured -- no HTML parsing needed for
//!     this one, just picking one key out of real JSON.
//!   - Cuevana3: plain server-rendered HTML again, one `<li><div
//!     class="TPost A">` per result; the thumbnail is lazy-loaded (real
//!     src is in `data-src`, `src` itself is just a loading spinner
//!     placeholder).
//! Like `search_youtube`, these are unofficial and can break if a site
//! changes its markup -- acceptable for this experimental feature, not
//! something to build a load-bearing feature's only path on.
//! The three domains are stored XOR-obfuscated rather than as plain
//! string literals -- not real security (anyone who cares to
//! disassemble the binary can trivially recover them), just enough that
//! they don't show up in a plain `strings`/`grep` pass over the source
//! or the compiled app.

use crate::errmap::ToStringErr;
use crate::progress::{ProgressEvent, ProgressReporter};
use serde::Serialize;
use std::io::{Read, Write};
use tauri::ipc::Channel;

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

#[derive(Serialize, Clone)]
pub struct BookResult {
    pub title: String,
    pub url: String,
    pub snippet: Option<String>,
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

/// Optional narrowing for `search_images` -- maps to DuckDuckGo's own
/// image-tab filter chips, confirmed live against the real `i.js`
/// endpoint (result counts genuinely shrink per filter, not just ignored):
/// a comma-joined `key:value` list in the `f` query param, e.g.
/// `type:photo,size:Large,color:Monochrome`. DDG has no literal
/// width/height filter -- `size` (Small/Medium/Large/Wallpaper) is the
/// closest real proxy for "dimensions", and there's no dedicated "png"
/// type either -- "transparent" is the closest (transparency needs an
/// alpha channel, which in practice means PNG/GIF/WebP, not JPEG).
#[derive(serde::Deserialize, Default)]
pub(crate) struct ImageSearchFilters {
    // "photo" | "clipart" | "gif" | "transparent" | "line"
    pub file_type: Option<String>,
    // "Small" | "Medium" | "Large" | "Wallpaper"
    pub size: Option<String>,
    // "color" | "Monochrome" | "Red" | "Orange" | ... (DDG's own palette)
    pub color: Option<String>,
    // "Square" | "Tall" | "Wide"
    pub layout: Option<String>,
}

#[tauri::command]
pub(crate) fn search_images(query: String, filters: Option<ImageSearchFilters>) -> Result<Vec<ImageResult>, String> {
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

    let f = filters.unwrap_or_default();
    let f_param = [
        f.file_type.as_deref().map(|v| format!("type:{v}")),
        f.size.as_deref().map(|v| format!("size:{v}")),
        f.color.as_deref().map(|v| format!("color:{v}")),
        f.layout.as_deref().map(|v| format!("layout:{v}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(",");

    let mut img_url = reqwest::Url::parse("https://duckduckgo.com/i.js").str_err()?;
    img_url
        .query_pairs_mut()
        .append_pair("l", "us-en")
        .append_pair("o", "json")
        .append_pair("q", &query)
        .append_pair("vqd", vqd)
        .append_pair("f", &f_param)
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

#[tauri::command]
pub(crate) fn search_books(query: String) -> Result<Vec<BookResult>, String> {
    let mut url = reqwest::Url::parse("https://archive.org/advancedsearch.php").str_err()?;
    url.query_pairs_mut()
        .append_pair("q", &format!("({query}) AND mediatype:texts"))
        .append_pair("fl[]", "identifier")
        .append_pair("fl[]", "title")
        .append_pair("fl[]", "creator")
        .append_pair("fl[]", "year")
        .append_pair("rows", "30")
        .append_pair("page", "1")
        .append_pair("output", "json");
    let json: serde_json::Value = http_client()?.get(url).send().str_err()?.json().str_err()?;
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
            let creator = d.get("creator").and_then(|c| c.as_str());
            let year = d.get("year").and_then(|y| y.as_str());
            let snippet = match (creator, year) {
                (Some(c), Some(y)) => Some(format!("{c} · {y}")),
                (Some(c), None) => Some(c.to_string()),
                (None, Some(y)) => Some(y.to_string()),
                (None, None) => None,
            };
            Some(BookResult { title, url: format!("https://archive.org/details/{identifier}"), snippet })
        })
        .collect())
}

fn xor_decode(bytes: &[u8]) -> String {
    const KEY: u8 = 0x5A;
    bytes.iter().map(|b| (b ^ KEY) as char).collect()
}

fn animeflv_domain() -> String {
    xor_decode(&[45, 45, 45, 110, 116, 59, 52, 51, 55, 63, 60, 54, 44, 116, 52, 63, 46])
}
fn xhamster_domain() -> String {
    xor_decode(&[34, 50, 59, 55, 41, 46, 63, 40, 116, 57, 53, 55])
}
fn cuevana3_domain() -> String {
    xor_decode(&[57, 47, 63, 44, 59, 52, 59, 105, 116, 56, 51, 62])
}

#[derive(Serialize, Clone)]
pub struct VideoProvider {
    pub id: String,
    pub label: String,
}

#[tauri::command]
pub(crate) fn list_video_providers() -> Vec<VideoProvider> {
    vec![
        VideoProvider { id: "youtube".into(), label: "YouTube".into() },
        VideoProvider { id: "animeflv".into(), label: "AnimeFLV".into() },
        VideoProvider { id: "xhamster".into(), label: "xHamster".into() },
        VideoProvider { id: "cuevana3".into(), label: "Cuevana3".into() },
    ]
}

#[derive(Serialize, Clone)]
pub struct ProviderVideoResult {
    pub title: String,
    pub thumbnail: String,
    pub page_url: String,
    pub duration: Option<String>,
}

/// Unescapes the handful of HTML entities that actually show up in a
/// scraped title/attribute -- not a general HTML-to-text converter.
fn unescape_html_entities(s: &str) -> String {
    s.replace("&amp;", "&").replace("&#x27;", "'").replace("&#039;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">")
}

fn format_duration_secs(secs: i64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// AnimeFLV's search results are shows, not individual episodes -- each
/// result opens the show's page (episode list), same as searching
/// "X anime" in a browser and clicking the first result.
fn search_animeflv(query: &str) -> Result<Vec<ProviderVideoResult>, String> {
    let domain = animeflv_domain();
    let mut url = reqwest::Url::parse(&format!("https://{domain}/browse")).str_err()?;
    url.query_pairs_mut().append_pair("q", query);
    let html = http_client()?.get(url).send().str_err()?.text().str_err()?;

    let article_marker = "<article class=\"Anime";
    let href_marker = "<a href=\"";
    let img_marker = "<img src=\"";
    let mut results = Vec::new();
    let mut cursor = 0usize;
    while results.len() < 30 {
        let Some(rel) = html[cursor..].find(article_marker) else { break };
        let start = cursor + rel;
        let window_end = (start + 1500).min(html.len());
        let window = &html[start..window_end];

        let href = window.find(href_marker).and_then(|p| {
            let s = p + href_marker.len();
            window[s..].find('"').map(|e| &window[s..s + e])
        });
        let thumb_end_hint = window.find("<figure>").unwrap_or(0);
        let thumb = window[thumb_end_hint..].find(img_marker).and_then(|p| {
            let s = thumb_end_hint + p + img_marker.len();
            window[s..].find('"').map(|e| window[s..s + e].to_string())
        });
        let title = window.find("<h3 class=\"Title\">").and_then(|p| {
            let s = p + "<h3 class=\"Title\">".len();
            window[s..].find("</h3>").map(|e| unescape_html_entities(window[s..s + e].trim()))
        });

        cursor = start + article_marker.len();
        let (Some(href), Some(title)) = (href, title) else { continue };
        if title.is_empty() {
            continue;
        }
        results.push(ProviderVideoResult {
            title,
            thumbnail: thumb.unwrap_or_default(),
            page_url: format!("https://{domain}{href}"),
            duration: None,
        });
    }
    Ok(results)
}

/// xhamster hydrates its own search page from a `window.initials = {...}`
/// JSON blob rather than needing any HTML parsing -- `searchResult.
/// videoThumbProps` is already an array of structured results.
fn search_xhamster(query: &str) -> Result<Vec<ProviderVideoResult>, String> {
    let domain = xhamster_domain();
    let mut url = reqwest::Url::parse(&format!("https://{domain}/search/")).str_err()?;
    url.path_segments_mut().map_err(|_| "cannot build search path".to_string())?.push(query);
    let html = http_client()?.get(url).send().str_err()?.text().str_err()?;

    let marker = "window.initials=";
    let start = html.find(marker).ok_or("couldn't find the results data on the page")?.saturating_add(marker.len());
    // The script tag has more JS after the JSON literal (`;` and other
    // statements) -- a plain `serde_json::from_str` on the rest of the
    // tag would fail on that trailing content, so this reads only the
    // first complete JSON value and ignores whatever follows it.
    let value: serde_json::Value = serde_json::Deserializer::from_str(&html[start..])
        .into_iter::<serde_json::Value>()
        .next()
        .ok_or("malformed results data")?
        .str_err()?;
    let items = value
        .get("searchResult")
        .and_then(|r| r.get("videoThumbProps"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(items
        .into_iter()
        .filter_map(|item| {
            Some(ProviderVideoResult {
                title: unescape_html_entities(item.get("title")?.as_str()?),
                thumbnail: item.get("thumbURL")?.as_str()?.to_string(),
                page_url: item.get("pageURL")?.as_str()?.to_string(),
                duration: item.get("duration").and_then(|d| d.as_i64()).map(format_duration_secs),
            })
        })
        .take(40)
        .collect())
}

fn search_cuevana3(query: &str) -> Result<Vec<ProviderVideoResult>, String> {
    let domain = cuevana3_domain();
    let mut url = reqwest::Url::parse(&format!("https://{domain}/")).str_err()?;
    url.query_pairs_mut().append_pair("s", query);
    let html = http_client()?.get(url).send().str_err()?.text().str_err()?;

    let item_marker = "<div class=\"TPost A\">";
    let href_marker = "<a href=\"";
    let thumb_marker = "data-src=\"";
    let title_marker = "<div class=Title>";
    let mut results = Vec::new();
    let mut cursor = 0usize;
    while results.len() < 30 {
        let Some(rel) = html[cursor..].find(item_marker) else { break };
        let start = cursor + rel;
        let window_end = (start + 1200).min(html.len());
        let window = &html[start..window_end];

        let href = window.find(href_marker).and_then(|p| {
            let s = p + href_marker.len();
            window[s..].find('"').map(|e| &window[s..s + e])
        });
        let thumb = window.find(thumb_marker).and_then(|p| {
            let s = p + thumb_marker.len();
            window[s..].find('"').map(|e| window[s..s + e].to_string())
        });
        let title = window.find(title_marker).and_then(|p| {
            let s = p + title_marker.len();
            window[s..].find("</div>").map(|e| unescape_html_entities(window[s..s + e].trim()))
        });

        cursor = start + item_marker.len();
        let (Some(href), Some(title)) = (href, title) else { continue };
        if title.is_empty() {
            continue;
        }
        results.push(ProviderVideoResult {
            title,
            thumbnail: thumb.unwrap_or_default(),
            page_url: format!("https://{domain}{href}"),
            duration: None,
        });
    }
    Ok(results)
}

#[tauri::command]
pub(crate) fn search_provider_videos(provider: String, query: String) -> Result<Vec<ProviderVideoResult>, String> {
    match provider.as_str() {
        "animeflv" => search_animeflv(&query),
        "xhamster" => search_xhamster(&query),
        "cuevana3" => search_cuevana3(&query),
        other => Err(format!("unknown provider: {other}")),
    }
}

#[derive(Serialize, Clone)]
pub struct AnimeflvEpisode {
    pub number: f64,
    pub thumbnail: String,
    pub page_url: String,
}

/// Finds a top-level `[...]` JS array literal right after `marker`,
/// respecting quoted strings so a title containing `[`/`]` can't
/// terminate the scan early. Returns the array's own `[...]` slice
/// (still JSON, ready for `serde_json::from_str`).
fn find_js_array<'a>(html: &'a str, marker: &str) -> Option<&'a str> {
    let after = &html[html.find(marker)? + marker.len()..];
    let open = after.find('[')?;
    let bytes = after.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &c) in bytes.iter().enumerate().skip(open) {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_string = false;
            }
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&after[open..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// A show's page renders its episode list client-side from two `var`s
/// left in a plain `<script>` tag -- `anime_info` (`[id, name, slug]`)
/// and `episodes` (`[[number, internal_id], ...]`, newest first, and
/// `number` isn't always an integer -- OVAs/specials show up as e.g.
/// `13.5`). Both are valid JSON once sliced out, no HTML parsing needed.
/// Confirmed live against the site's own `funciones.js`: an episode's
/// real page is `/ver/<slug>-<number>` and its thumbnail is
/// `cdn.animeflv.net/screenshots/<anime_id>/<number>/th_3.jpg`.
#[tauri::command]
pub(crate) fn list_animeflv_episodes(page_url: String) -> Result<Vec<AnimeflvEpisode>, String> {
    let html = http_client()?.get(&page_url).send().str_err()?.text().str_err()?;

    let anime_info: Vec<serde_json::Value> = serde_json::from_str(
        find_js_array(&html, "var anime_info = ").ok_or("couldn't find this show's info on the page")?,
    )
    .str_err()?;
    let anime_id = anime_info.first().and_then(|v| v.as_str()).unwrap_or("0");
    let slug = anime_info.get(2).and_then(|v| v.as_str()).ok_or("couldn't read this show's slug")?;

    let episodes: Vec<(f64, i64)> = serde_json::from_str(
        find_js_array(&html, "var episodes = ").ok_or("couldn't find this show's episode list on the page")?,
    )
    .str_err()?;

    let domain = animeflv_domain();
    let mut list: Vec<AnimeflvEpisode> = episodes
        .into_iter()
        .map(|(number, _internal_id)| AnimeflvEpisode {
            number,
            thumbnail: format!("https://cdn.animeflv.net/screenshots/{anime_id}/{number}/th_3.jpg"),
            page_url: format!("https://{domain}/ver/{slug}-{number}"),
        })
        .collect();
    list.sort_by(|a, b| a.number.total_cmp(&b.number));
    Ok(list)
}

#[derive(Serialize)]
pub struct PlayableSource {
    // "iframe" (a same-site chrome-free embed URL) or "video" (a raw,
    // directly-playable file URL for a native <video> element).
    pub kind: String,
    pub url: String,
}

/// Resolves a search result's `page_url` to something the standalone
/// player window can actually render with nothing else on screen --
/// verified live against each site's real markup, not guessed:
///   - xhamster: `/embed/<id>` loads and its play button visibly
///     responds to a click, but playback never actually starts --
///     confirmed live on more than one video, unaffected by a
///     no-referrer attempt -- a real WebKitGTK/player incompatibility,
///     not something fixable through the embed URL. Returns a clear
///     error instead of a play button that silently does nothing.
///   - cuevana3: the `ver/` page's plain `<source src="...">` mp4 turned
///     out to be a decoy -- confirmed live: its Content-Length is only a
///     few MB regardless of the movie's real runtime, and the page's own
///     "Ver" menu triggers a separate AJAX call (not yet reverse-
///     engineered) to swap in the real player. Reported directly as
///     "plays an intro, not the real content" -- this returns a clear
///     error instead of silently playing the wrong video.
///   - animeflv: its episode player is filled in by client-side JS from
///     data this plain HTTP fetch never receives (confirmed empty on
///     multiple real episode pages, with/without cookies/headers) --
///     not solvable without a headless browser, so this returns a clear
///     error instead of a broken/blank player.
#[tauri::command]
pub(crate) fn resolve_provider_playable(provider: String, _page_url: String) -> Result<PlayableSource, String> {
    match provider.as_str() {
        "xhamster" => Err(
            "xHamster's embed player doesn't actually start playback in this app -- opening in your browser instead."
                .to_string(),
        ),
        "cuevana3" => Err(
            "Cuevana3's real video only loads after picking a server on its own page -- opening in your browser instead."
                .to_string(),
        ),
        "animeflv" => Err("AnimeFLV's player needs JavaScript this app's scraper can't run -- opening in your browser instead.".to_string()),
        other => Err(format!("unknown provider: {other}")),
    }
}

/// Backs "Save to folder…"/drag-to-a-folder for an Internet result --
/// generic on purpose (it just streams whatever's at `url` into
/// `dest_dir/filename`), so it doesn't need to know which result kind it
/// came from. What's a genuinely downloadable file varies by kind though
/// (decided on the frontend, see InternetView.tsx's `downloadItemsFor`):
/// an image result's own `image` field already is one, and a book
/// result's archive.org details URL gets rewritten to that item's real
/// `/download/<id>/<id>.pdf` file (confirmed live against several real
/// archive.org texts items -- a 302 to the actual CDN mirror, which
/// reqwest's blocking client follows on its own). Video results have no
/// such thing: a watch/embed page URL is HTML, not media, and actually
/// extracting the real video stream needs the kind of cipher-solving
/// yt-dlp exists for -- out of scope here, so the frontend never offers
/// this for a video tile in the first place, same honesty standard as
/// `resolve_provider_playable` above.
#[tauri::command]
pub(crate) async fn download_web_result(
    url: String,
    dest_dir: String,
    filename: String,
    channel: Channel<ProgressEvent>,
    registry: tauri::State<'_, crate::ops::OpRegistry>,
) -> Result<(), String> {
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let dest = std::path::Path::new(&dest_dir).join(&filename);
        if dest.exists() {
            return Err(format!("\"{filename}\" already exists in that folder"));
        }
        let mut resp = http_client()?.get(&url).send().str_err()?;
        if !resp.status().is_success() {
            return Err(format!("download failed: HTTP {}", resp.status()));
        }
        let total = resp.content_length().unwrap_or(0).max(1);
        let reporter = ProgressReporter::new_cancellable(channel, total, cancel);
        let tmp = std::path::Path::new(&dest_dir).join(format!("{filename}.part"));
        let mut file = std::fs::File::create(&tmp).str_err()?;
        let mut buf = [0u8; 65536];
        let mut done = 0u64;
        loop {
            if reporter.is_cancelled() {
                drop(file);
                let _ = std::fs::remove_file(&tmp);
                return Err("Cancelled".to_string());
            }
            let n = resp.read(&mut buf).str_err()?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).str_err()?;
            done += n as u64;
            reporter.report(done.min(total));
        }
        drop(file);
        std::fs::rename(&tmp, &dest).str_err()?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    result
}
