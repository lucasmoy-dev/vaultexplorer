//! A one-page loopback HTTP server that exists purely to give YouTube's
//! embed player an origin it will accept.
//!
//! On Linux/macOS a Tauri window's real origin is `tauri://localhost` --
//! not http(s) -- and the frontend's `referrerPolicy="no-referrer"` meant
//! the embed request arrived with no usable Referer either. YouTube
//! answers that with error 153 ("video player configuration error"),
//! which is exactly the desktop playback failure reported against the
//! previous `?origin=<window.location.origin>` fix: passing an unusable
//! origin is no better than passing none.
//!
//! So the iframe points here instead: a tiny page served from
//! `http://127.0.0.1:<port>`, which then embeds YouTube itself. That
//! inner request carries a real http origin and Referer, which YouTube
//! accepts (loopback is fine -- it's the scheme it cares about). The
//! server is loopback-only, serves this single page and nothing else, and
//! starts lazily the first time a video is played.

use std::sync::OnceLock;

/// Bound once per process and reused -- there's no state to keep beyond
/// the port, and re-binding per video would leak a thread each time.
fn port() -> Result<u16, String> {
    static PORT: OnceLock<Result<u16, String>> = OnceLock::new();
    PORT.get_or_init(start_server).clone()
}

fn start_server() -> Result<u16, String> {
    // Port 0 = let the OS pick a free one; 127.0.0.1 so nothing off this
    // machine can reach the page.
    let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = server.server_addr().to_ip().ok_or("no TCP address")?.port();
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let id = query_param(request.url(), "v").unwrap_or_default();
            let body = if is_video_id(&id) {
                page(&id, port)
            } else {
                // Anything that isn't a plain YouTube id is a request this
                // server has no business answering -- and interpolating it
                // into the page would be an HTML injection into a document
                // that is about to be handed real web content.
                "<!doctype html><title>Not found</title>".to_string()
            };
            let response = tiny_http::Response::from_data(body.into_bytes()).with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                    .unwrap_or_else(|_| unreachable!("static header is valid")),
            );
            let _ = request.respond(response);
        }
    });
    Ok(port)
}

/// YouTube ids are 11 url-safe base64 chars today, but the length has
/// changed before -- bound it loosely and reject anything that isn't in
/// the character set, which is all the escaping guarantee this needs.
fn is_video_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 24 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

fn page(id: &str, port: u16) -> String {
    format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8">
<style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}iframe{{border:0;width:100%;height:100%;display:block}}</style>
</head><body>
<iframe src="https://www.youtube.com/embed/{id}?autoplay=1&enablejsapi=1&origin=http://127.0.0.1:{port}"
  allow="autoplay; fullscreen; encrypted-media; picture-in-picture" allowfullscreen></iframe>
</body></html>"#
    )
}

/// Returns the loopback URL to point the player's iframe at.
#[tauri::command]
pub fn youtube_embed_url(video_id: String) -> Result<String, String> {
    if !is_video_id(&video_id) {
        return Err(format!("Not a YouTube video id: {video_id}"));
    }
    let port = port()?;
    Ok(format!("http://127.0.0.1:{port}/watch?v={video_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_an_embed_page_for_a_real_id() {
        let url = youtube_embed_url("dQw4w9WgXcQ".to_string()).expect("url");
        let body = reqwest::blocking::get(&url).expect("get").text().expect("text");
        assert!(body.contains("https://www.youtube.com/embed/dQw4w9WgXcQ"), "{body}");
        assert!(body.contains("origin=http://127.0.0.1:"), "{body}");
    }

    #[test]
    fn rejects_an_id_that_could_carry_markup() {
        assert!(youtube_embed_url("\"><script>x</script>".to_string()).is_err());
    }
}
