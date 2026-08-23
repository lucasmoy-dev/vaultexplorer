//! Google Drive folder sync that needs no external binary -- the mobile
//! path.
//!
//! Desktop sync (see `rclone.rs` / `sync.rs`) shells out to `rclone`:
//! `rclone authorize` for the OAuth dance, `rclone bisync` for the actual
//! two-way transfer. Android has no rclone to shell out to (and no way to
//! ship one: it's a separate Go binary, and the app can't execute a
//! downloaded ELF from its sandbox), which is why "Cloud & folder sync"
//! read "not available on mobile yet" for so long.
//!
//! So this module is the parts of rclone the phone actually needs, done
//! in-process:
//!
//! * OAuth 2.0 authorization-code + PKCE against the *user's own* Google
//!   OAuth client, with a loopback redirect. A phone can serve
//!   `http://127.0.0.1:<port>` to its own browser exactly like a desktop
//!   can, so the same loopback flow rclone uses works unchanged here --
//!   no custom URL scheme, no manifest intent filter, no app-specific
//!   redirect service in the middle.
//! * A small Drive v3 client (list / download / upload / mkdir / delete).
//! * A two-way sync pass with a per-pair state journal, which is what
//!   makes it a *sync* rather than a mirror: a file gone from one side is
//!   deleted on the other only when the journal says it was there last
//!   time, and a file changed on both sides is kept twice rather than one
//!   copy silently winning.
//!
//! Why the user's own OAuth client rather than a client ID baked in here:
//! a Drive scope on a shared client is exactly what Google is retiring
//! rclone's built-in one over (see `rclone.rs`), an ID shipped inside an
//! APK is not a secret in any meaningful sense, and a token issued to
//! rclone's client can only ever be refreshed with rclone's own secret --
//! which this app doesn't have. One-time setup in the Google Cloud
//! console, and the credentials stay on the device.

use crate::errmap::{LockExt, ToStringErr};
use crate::progress::ProgressReporter;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const API: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3";
/// Full Drive access, matching what the desktop's rclone remote gets --
/// anything narrower (`drive.file`) can only see files this app itself
/// created, so a folder already synced from the desktop would be
/// invisible and the two platforms would not actually share a folder.
const SCOPE: &str = "https://www.googleapis.com/auth/drive";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
/// Everything this app syncs lives under one Drive folder, same as the
/// desktop's `vaultexplorer-drive:VaultExplorer/<name>` remote path -- so
/// a folder paired on the phone and on the desktop is the same folder.
const ROOT_FOLDER: &str = "VaultExplorer";
/// A Google Docs/Sheets/Slides file has no bytes to download (only
/// exports) and no md5 to compare, so it can't take part in a
/// file-for-file sync. Such a file still goes into the listing -- leaving
/// it out would make the sync pass read it as "deleted on Drive" -- but
/// carries neither md5 nor size, which is how the pass recognises it and
/// reports it as skipped instead of touching it.
const GOOGLE_NATIVE_PREFIX: &str = "application/vnd.google-apps";

// ---- stored credentials ----------------------------------------------

#[derive(Clone, Serialize, Deserialize, Default)]
struct Creds {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    #[serde(default)]
    access_token: String,
    /// Unix seconds. Zero means "assume expired".
    #[serde(default)]
    expires_at: u64,
    #[serde(default)]
    account_email: String,
}

fn state_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    // The app's own private data dir, not `$HOME/.config` (which is where
    // the desktop keeps its rclone-based config): on Android HOME isn't
    // set at all, and this directory is both writable and excluded from
    // other apps' reach.
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).str_err()?;
    Ok(dir)
}

fn creds_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(state_dir(app)?.join("drive_auth.json"))
}

fn read_creds(app: &tauri::AppHandle) -> Option<Creds> {
    let raw = std::fs::read_to_string(creds_path(app).ok()?).ok()?;
    let creds: Creds = serde_json::from_str(&raw).ok()?;
    if creds.refresh_token.is_empty() {
        return None;
    }
    Some(creds)
}

fn write_creds(app: &tauri::AppHandle, creds: &Creds) -> Result<(), String> {
    let path = creds_path(app)?;
    std::fs::write(&path, serde_json::to_string_pretty(creds).str_err()?).str_err()?;
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn http() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .str_err()
}

/// A usable access token, refreshing first when the stored one is gone or
/// about to expire. Refreshing needs the client secret, which is why the
/// user's own client credentials are stored alongside the token.
fn access_token(app: &tauri::AppHandle) -> Result<String, String> {
    let mut creds = read_creds(app).ok_or("Google Drive isn't connected on this device yet.")?;
    if !creds.access_token.is_empty() && creds.expires_at > now_secs() + 60 {
        return Ok(creds.access_token);
    }
    let response = http()?
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("client_id", creds.client_id.as_str()),
            ("client_secret", creds.client_secret.as_str()),
            ("refresh_token", creds.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .str_err()?;
    let status = response.status();
    let body: serde_json::Value = response.json().str_err()?;
    if !status.is_success() {
        let detail = body
            .get("error_description")
            .or_else(|| body.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!(
            "Google refused to refresh the sign-in ({detail}). Reconnect Google Drive in Settings."
        ));
    }
    creds.access_token =
        body.get("access_token").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let ttl = body.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(3600);
    creds.expires_at = now_secs() + ttl;
    // A refresh can hand back a *new* refresh token; ignoring it would
    // eventually invalidate the sign-in.
    if let Some(rt) = body.get("refresh_token").and_then(|v| v.as_str()) {
        creds.refresh_token = rt.to_string();
    }
    write_creds(app, &creds)?;
    Ok(creds.access_token)
}

// ---- OAuth (loopback redirect + PKCE) --------------------------------

fn random_urlsafe(bytes: usize) -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

fn pkce_challenge(verifier: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn urlencode(value: &str) -> String {
    // Small hand-rolled encoder rather than another dependency: the only
    // things going through here are a client id, a redirect URI, a scope
    // and base64url randomness.
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Sign in to Google and store the resulting refresh token.
///
/// Serves a one-request loopback endpoint the browser is redirected back
/// to; `url_channel` receives the consent URL the moment it's known, so
/// the UI can show a tappable link even if handing the URL to the system
/// browser fails (which on Android depends on there being a browser that
/// accepts the intent at all).
fn connect(
    app: &tauri::AppHandle,
    client_id: String,
    client_secret: String,
    url_channel: tauri::ipc::Channel<String>,
) -> Result<String, String> {
    let server = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|e| format!("couldn't open the local sign-in port: {e}"))?;
    let port = match server.server_addr() {
        tiny_http::ListenAddr::IP(addr) => addr.port(),
        #[allow(unreachable_patterns)]
        _ => return Err("couldn't read the local sign-in port".to_string()),
    };
    let redirect_uri = format!("http://127.0.0.1:{port}");
    let verifier = random_urlsafe(48);
    let state = random_urlsafe(16);
    let auth_url = format!(
        "{AUTH_ENDPOINT}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}\
         &code_challenge_method=S256&state={}&access_type=offline&prompt=consent",
        urlencode(&client_id),
        urlencode(&redirect_uri),
        urlencode(SCOPE),
        urlencode(&pkce_challenge(&verifier)),
        urlencode(&state),
    );
    let _ = url_channel.send(auth_url.clone());
    // Best effort -- the channel above is the reliable path, this is the
    // convenience one.
    let _ = tauri_plugin_opener::open_url(&auth_url, None::<&str>);

    // One redirect is all this flow ever gets; anything else on this port
    // (a stray favicon request, a probe) is answered and ignored.
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    let code = loop {
        if std::time::Instant::now() > deadline {
            return Err("sign-in timed out -- nothing came back from the browser".to_string());
        }
        let request = match server.recv_timeout(Duration::from_secs(5)) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(e) => return Err(format!("local sign-in server failed: {e}")),
        };
        let url = request.url().to_string();
        let params: HashMap<String, String> = url
            .split_once('?')
            .map(|(_, query)| {
                query
                    .split('&')
                    .filter_map(|kv| kv.split_once('='))
                    .map(|(k, v)| (k.to_string(), percent_decode(v)))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(err) = params.get("error") {
            let _ = request.respond(html_response("Sign-in was refused. You can close this tab."));
            return Err(format!("Google returned \"{err}\""));
        }
        let Some(code) = params.get("code").cloned() else {
            let _ = request.respond(html_response("Waiting for Google…"));
            continue;
        };
        if params.get("state").map(String::as_str) != Some(state.as_str()) {
            let _ = request.respond(html_response("That sign-in didn't match this request."));
            return Err("sign-in state mismatch -- start again".to_string());
        }
        let _ = request.respond(html_response("Signed in. You can close this tab and go back to VaultExplorer."));
        break code;
    };

    let response = http()?
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri.as_str()),
        ])
        .send()
        .str_err()?;
    let status = response.status();
    let body: serde_json::Value = response.json().str_err()?;
    if !status.is_success() {
        let detail = body
            .get("error_description")
            .or_else(|| body.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("Google rejected the sign-in: {detail}"));
    }
    let refresh_token =
        body.get("refresh_token").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    if refresh_token.is_empty() {
        return Err(
            "Google didn't return a refresh token. Remove this app's access at \
             myaccount.google.com/permissions and connect again."
                .to_string(),
        );
    }
    let mut creds = Creds {
        client_id,
        client_secret,
        refresh_token,
        access_token: body
            .get("access_token")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        expires_at: now_secs() + body.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(3600),
        account_email: String::new(),
    };
    // Which account this is, so the UI can show something more useful
    // than "connected".
    if let Ok(about) = http()?
        .get(format!("{API}/about?fields=user(emailAddress)"))
        .bearer_auth(&creds.access_token)
        .send()
    {
        if let Ok(json) = about.json::<serde_json::Value>() {
            creds.account_email = json
                .pointer("/user/emailAddress")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
        }
    }
    write_creds(app, &creds)?;
    Ok(creds.account_email)
}

fn percent_decode(value: &str) -> String {
    let bytes = value.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn html_response(message: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let body = format!(
        "<!doctype html><meta name=viewport content=\"width=device-width,initial-scale=1\">\
         <body style=\"font:16px system-ui;padding:2rem;color:#222\">{message}</body>"
    );
    let mut response = tiny_http::Response::from_string(body);
    if let Ok(header) = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
    {
        response.add_header(header);
    }
    response
}

// ---- Drive v3 client -------------------------------------------------

#[derive(Clone, Debug)]
struct RemoteEntry {
    id: String,
    is_dir: bool,
    /// Drive's md5 -- absent for folders and for Google-native files.
    md5: String,
    modified: String,
    size: u64,
    /// A Google Docs/Sheets/Slides file: listed, but not syncable (see
    /// `GOOGLE_NATIVE_PREFIX`).
    native: bool,
}

fn drive_get(app: &tauri::AppHandle, url: &str) -> Result<serde_json::Value, String> {
    let token = access_token(app)?;
    let response = http()?.get(url).bearer_auth(token).send().str_err()?;
    let status = response.status();
    let body: serde_json::Value = response.json().str_err()?;
    if !status.is_success() {
        return Err(drive_error(&body, status));
    }
    Ok(body)
}

fn drive_error(body: &serde_json::Value, status: reqwest::StatusCode) -> String {
    let message = body
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .unwrap_or("no detail")
        .to_string();
    format!("Google Drive said {status}: {message}")
}

/// One page-following listing of a folder's direct children.
fn list_children(app: &tauri::AppHandle, folder_id: &str) -> Result<HashMap<String, RemoteEntry>, String> {
    let mut out = HashMap::new();
    let mut page_token = String::new();
    loop {
        let query = urlencode(&format!("'{folder_id}' in parents and trashed = false"));
        let mut url = format!(
            "{API}/files?q={query}&pageSize=1000&fields=nextPageToken,files(id,name,mimeType,md5Checksum,size,modifiedTime)&supportsAllDrives=true"
        );
        if !page_token.is_empty() {
            url.push_str(&format!("&pageToken={}", urlencode(&page_token)));
        }
        let body = drive_get(app, &url)?;
        for file in body.get("files").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            let name = file.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let mime = file.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
            out.insert(
                name,
                RemoteEntry {
                    id: file.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    is_dir: mime == FOLDER_MIME,
                    md5: file.get("md5Checksum").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    modified: file
                        .get("modifiedTime")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    size: file
                        .get("size")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                    native: mime != FOLDER_MIME && mime.starts_with(GOOGLE_NATIVE_PREFIX),
                },
            );
        }
        match body.get("nextPageToken").and_then(|v| v.as_str()) {
            Some(next) if !next.is_empty() => page_token = next.to_string(),
            _ => break,
        }
    }
    Ok(out)
}

fn find_child(
    app: &tauri::AppHandle,
    parent_id: &str,
    name: &str,
    dirs_only: bool,
) -> Result<Option<RemoteEntry>, String> {
    let escaped = name.replace('\\', "\\\\").replace('\'', "\\'");
    let mut q = format!("'{parent_id}' in parents and name = '{escaped}' and trashed = false");
    if dirs_only {
        q.push_str(&format!(" and mimeType = '{FOLDER_MIME}'"));
    }
    let url = format!(
        "{API}/files?q={}&fields=files(id,name,mimeType,md5Checksum,size,modifiedTime)&pageSize=10",
        urlencode(&q)
    );
    let body = drive_get(app, &url)?;
    let Some(file) = body.get("files").and_then(|v| v.as_array()).and_then(|a| a.first()) else {
        return Ok(None);
    };
    let mime = file.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
    Ok(Some(RemoteEntry {
        id: file.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        is_dir: mime == FOLDER_MIME,
        md5: file.get("md5Checksum").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        modified: file.get("modifiedTime").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        size: file
            .get("size")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        native: mime != FOLDER_MIME && mime.starts_with(GOOGLE_NATIVE_PREFIX),
    }))
}

fn create_folder(app: &tauri::AppHandle, parent_id: &str, name: &str) -> Result<String, String> {
    let token = access_token(app)?;
    let body = serde_json::json!({
        "name": name,
        "mimeType": FOLDER_MIME,
        "parents": [parent_id],
    });
    let response = http()?
        .post(format!("{API}/files?fields=id"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .str_err()?;
    let status = response.status();
    let json: serde_json::Value = response.json().str_err()?;
    if !status.is_success() {
        return Err(drive_error(&json, status));
    }
    Ok(json.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string())
}

fn find_or_create_folder(app: &tauri::AppHandle, parent_id: &str, name: &str) -> Result<String, String> {
    if let Some(existing) = find_child(app, parent_id, name, true)? {
        return Ok(existing.id);
    }
    create_folder(app, parent_id, name)
}

/// The `VaultExplorer/<pair name>` folder, created if missing -- same
/// layout the desktop's rclone remote uses, so the two platforms sync the
/// same folder rather than two folders that happen to share a name.
fn pair_folder_id(app: &tauri::AppHandle, name: &str) -> Result<String, String> {
    let root = find_or_create_folder(app, "root", ROOT_FOLDER)?;
    find_or_create_folder(app, &root, name)
}

fn upload_new(
    app: &tauri::AppHandle,
    parent_id: &str,
    name: &str,
    local: &Path,
) -> Result<RemoteEntry, String> {
    let token = access_token(app)?;
    let metadata = serde_json::json!({ "name": name, "parents": [parent_id] });
    let response = http()?
        .post(format!("{API}/files?fields=id"))
        .bearer_auth(&token)
        .json(&metadata)
        .send()
        .str_err()?;
    let status = response.status();
    let json: serde_json::Value = response.json().str_err()?;
    if !status.is_success() {
        return Err(drive_error(&json, status));
    }
    let id = json.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    upload_media(app, &id, local)
}

/// PUT the file's bytes at an existing Drive file id. Streams straight
/// from disk -- a phone can easily be asked to sync a video it can't hold
/// in RAM twice over.
fn upload_media(app: &tauri::AppHandle, file_id: &str, local: &Path) -> Result<RemoteEntry, String> {
    let token = access_token(app)?;
    let file = std::fs::File::open(local).str_err()?;
    let len = file.metadata().str_err()?.len();
    let response = http()?
        .patch(format!(
            "{UPLOAD_API}/files/{file_id}?uploadType=media&fields=id,md5Checksum,size,modifiedTime"
        ))
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_LENGTH, len)
        .body(reqwest::blocking::Body::sized(file, len))
        .send()
        .str_err()?;
    let status = response.status();
    let json: serde_json::Value = response.json().str_err()?;
    if !status.is_success() {
        return Err(drive_error(&json, status));
    }
    Ok(RemoteEntry {
        id: json.get("id").and_then(|v| v.as_str()).unwrap_or(file_id).to_string(),
        is_dir: false,
        native: false,
        md5: json.get("md5Checksum").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        modified: json.get("modifiedTime").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        size: json
            .get("size")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(len),
    })
}

fn download_file(app: &tauri::AppHandle, file_id: &str, dest: &Path) -> Result<(), String> {
    let token = access_token(app)?;
    let mut response = http()?
        .get(format!("{API}/files/{file_id}?alt=media&supportsAllDrives=true"))
        .bearer_auth(token)
        .send()
        .str_err()?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("Google Drive said {status} downloading a file: {body}"));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).str_err()?;
    }
    // Written to a sibling temp file first, so an interrupted download
    // can't leave a half file that then looks like a real local change
    // and gets uploaded back over the good remote copy.
    let temp = dest.with_extension(format!(
        "{}.vaultexplorer-part",
        dest.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    {
        let mut out = std::fs::File::create(&temp).str_err()?;
        std::io::copy(&mut response, &mut out).str_err()?;
    }
    std::fs::rename(&temp, dest).str_err()?;
    Ok(())
}

fn delete_remote(app: &tauri::AppHandle, file_id: &str) -> Result<(), String> {
    let token = access_token(app)?;
    // Trashed, not purged: an unintended delete propagated from the other
    // side is recoverable from Drive's own trash for 30 days.
    let response = http()?
        .patch(format!("{API}/files/{file_id}"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "trashed": true }))
        .send()
        .str_err()?;
    if !response.status().is_success() {
        let status = response.status();
        let body: serde_json::Value = response.json().unwrap_or_default();
        return Err(drive_error(&body, status));
    }
    Ok(())
}

fn md5_of(path: &Path) -> Result<String, String> {
    // Drive's own checksum is md5, so comparing content means computing
    // md5 locally -- there's no other hash both sides share.
    let mut file = std::fs::File::open(path).str_err()?;
    let mut context = Md5::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let read = file.read(&mut buf).str_err()?;
        if read == 0 {
            break;
        }
        context.update(&buf[..read]);
    }
    Ok(context.finish())
}

// ---- pairs + journal -------------------------------------------------

#[derive(Clone, Serialize, Deserialize)]
pub struct MobilePair {
    pub local_path: String,
    pub remote_folder_name: String,
}

fn pairs_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(state_dir(app)?.join("drive_pairs.json"))
}

pub fn list_pairs(app: &tauri::AppHandle) -> Vec<MobilePair> {
    pairs_path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_pairs(app: &tauri::AppHandle, pairs: &[MobilePair]) -> Result<(), String> {
    std::fs::write(pairs_path(app)?, serde_json::to_string_pretty(pairs).str_err()?).str_err()
}

/// What the last successful pass saw, per relative path. Comparing both
/// sides against this is what tells "changed here" apart from "changed
/// there", and a deletion apart from a file that was never there.
#[derive(Clone, Serialize, Deserialize, Default)]
struct Journal {
    #[serde(default)]
    files: HashMap<String, JournalEntry>,
}

#[derive(Clone, Serialize, Deserialize)]
struct JournalEntry {
    local_size: u64,
    local_mtime_ms: u64,
    remote_id: String,
    remote_md5: String,
    remote_modified: String,
}

fn journal_path(app: &tauri::AppHandle, local_path: &str) -> Result<PathBuf, String> {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(local_path.as_bytes());
    let name = format!("drive_journal_{:x}.json", digest);
    Ok(state_dir(app)?.join(name))
}

fn read_journal(app: &tauri::AppHandle, local_path: &str) -> Journal {
    journal_path(app, local_path)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_journal(app: &tauri::AppHandle, local_path: &str, journal: &Journal) -> Result<(), String> {
    std::fs::write(journal_path(app, local_path)?, serde_json::to_string(journal).str_err()?)
        .str_err()
}

// ---- the sync pass ---------------------------------------------------

#[derive(Default, Serialize)]
pub struct SyncOutcome {
    pub uploaded: usize,
    pub downloaded: usize,
    pub deleted_local: usize,
    pub deleted_remote: usize,
    pub conflicts: Vec<String>,
    pub skipped: Vec<String>,
    /// One human-readable line, the same shape the desktop's rclone
    /// summary takes, so the sheet can show either without special-casing.
    pub summary: String,
}

pub(crate) struct LocalEntry {
    pub size: u64,
    pub mtime_ms: u64,
    pub is_dir: bool,
}

/// Every file and folder under `root`, keyed by path relative to it.
/// Shared with `folder_sync.rs`, which needs exactly the same view of a
/// local tree for its two-folder pass.
pub(crate) fn walk_local(root: &Path) -> Result<HashMap<String, LocalEntry>, String> {
    let mut out = HashMap::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // A folder that disappeared mid-walk (or that this app can't
            // read) shouldn't abort the whole pass.
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".vaultexplorer-part") {
                continue; // our own interrupted download
            }
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_symlink() {
                continue;
            }
            if meta.is_dir() {
                out.insert(rel.clone(), LocalEntry { size: 0, mtime_ms: 0, is_dir: true });
                stack.push((entry.path(), rel));
            } else if meta.is_file() {
                let mtime_ms = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                out.insert(rel, LocalEntry { size: meta.len(), mtime_ms, is_dir: false });
            }
        }
    }
    Ok(out)
}

/// The remote side of the pair, flattened to the same relative-path keys
/// `walk_local` produces.
fn walk_remote(
    app: &tauri::AppHandle,
    folder_id: &str,
) -> Result<HashMap<String, RemoteEntry>, String> {
    let mut out = HashMap::new();
    let mut stack = vec![(folder_id.to_string(), String::new())];
    while let Some((id, prefix)) = stack.pop() {
        for (name, entry) in list_children(app, &id)? {
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if entry.is_dir {
                let child_id = entry.id.clone();
                out.insert(rel.clone(), entry);
                stack.push((child_id, rel));
            } else {
                out.insert(rel, entry);
            }
        }
    }
    Ok(out)
}

fn conflict_name(rel: &str) -> String {
    let path = Path::new(rel);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str());
    let renamed = match ext {
        Some(ext) => format!("{stem} (from Drive).{ext}"),
        None => format!("{stem} (from Drive)"),
    };
    match path.parent().and_then(|p| p.to_str()).filter(|p| !p.is_empty()) {
        Some(parent) => format!("{parent}/{renamed}"),
        None => renamed,
    }
}

/// One full two-way pass over a pair.
pub fn sync_pair(
    app: &tauri::AppHandle,
    pair: &MobilePair,
    reporter: Option<&ProgressReporter>,
) -> Result<SyncOutcome, String> {
    let root = PathBuf::from(&pair.local_path);
    if !root.is_dir() {
        return Err(format!("\"{}\" isn't a folder on this device anymore", pair.local_path));
    }
    let folder_id = pair_folder_id(app, &pair.remote_folder_name)?;
    let local = walk_local(&root)?;
    let remote = walk_remote(app, &folder_id)?;
    let mut journal = read_journal(app, &pair.local_path);
    let mut outcome = SyncOutcome::default();

    // Remote folder ids, so a nested upload knows what to hang itself off
    // (and can create the missing folders on the way down).
    let mut dir_ids: HashMap<String, String> = HashMap::new();
    dir_ids.insert(String::new(), folder_id.clone());
    for (rel, entry) in &remote {
        if entry.is_dir {
            dir_ids.insert(rel.clone(), entry.id.clone());
        }
    }

    let mut paths: Vec<String> = local.keys().chain(remote.keys()).cloned().collect();
    paths.extend(journal.files.keys().cloned());
    paths.sort();
    paths.dedup();
    let total = paths.len() as u64;
    // The pass's real size is only knowable here, after both sides have
    // been listed -- the caller created the reporter before any of that.
    if let Some(reporter) = reporter {
        reporter.set_total(total.max(1));
    }
    let mut done = 0u64;
    // Folders before their contents on the way in; the deletion pass
    // below walks the other way round.
    let mut deletions: Vec<String> = Vec::new();

    for rel in &paths {
        done += 1;
        if let Some(reporter) = reporter {
            if reporter.is_cancelled() {
                write_journal(app, &pair.local_path, &journal)?;
                return Err("cancelled".to_string());
            }
            reporter.report(done.min(total));
        }
        let local_entry = local.get(rel);
        let remote_entry = remote.get(rel);
        let recorded = journal.files.get(rel).cloned();

        // Folders: only ever created, never compared -- there's no content
        // to diff, and an empty folder existing on one side is not a
        // change worth propagating in either direction.
        if local_entry.is_some_and(|e| e.is_dir) || remote_entry.is_some_and(|e| e.is_dir) {
            if local_entry.is_some() && remote_entry.is_none() {
                let (parent_rel, name) = split_rel(rel);
                let parent_id = ensure_remote_dir(app, &mut dir_ids, parent_rel)?;
                let id = find_or_create_folder(app, &parent_id, name)?;
                dir_ids.insert(rel.clone(), id);
            } else if remote_entry.is_some() && local_entry.is_none() {
                std::fs::create_dir_all(root.join(rel)).str_err()?;
            }
            continue;
        }

        if remote_entry.is_some_and(|e| e.native) {
            // A Google Docs file has no bytes to sync; saying so beats
            // either failing the pass or pretending it synced.
            outcome.skipped.push(rel.clone());
            continue;
        }

        match (local_entry, remote_entry) {
            (Some(l), Some(r)) => {
                let local_changed = recorded
                    .as_ref()
                    .map(|j| j.local_size != l.size || j.local_mtime_ms != l.mtime_ms)
                    .unwrap_or(true);
                let remote_changed = recorded
                    .as_ref()
                    .map(|j| j.remote_md5 != r.md5 || j.remote_modified != r.modified)
                    .unwrap_or(true);
                if !local_changed && !remote_changed {
                    continue;
                }
                // Identical content on both sides isn't a conflict even
                // when both look "changed" (a first pass, a re-download,
                // a copy made on both devices) -- just re-record it.
                if !r.md5.is_empty() && md5_of(&root.join(rel)).ok().as_deref() == Some(r.md5.as_str()) {
                    journal.files.insert(
                        rel.clone(),
                        JournalEntry {
                            local_size: l.size,
                            local_mtime_ms: l.mtime_ms,
                            remote_id: r.id.clone(),
                            remote_md5: r.md5.clone(),
                            remote_modified: r.modified.clone(),
                        },
                    );
                    continue;
                }
                if local_changed && remote_changed {
                    // Both sides moved: keep both, name the incoming copy
                    // for where it came from, and leave the local file
                    // alone. Silently overwriting one of two real edits is
                    // the one outcome a sync must never produce.
                    let side_path = conflict_name(rel);
                    download_file(app, &r.id, &root.join(&side_path))?;
                    outcome.conflicts.push(rel.clone());
                    outcome.downloaded += 1;
                    let uploaded = upload_media(app, &r.id, &root.join(rel))?;
                    outcome.uploaded += 1;
                    let meta = std::fs::metadata(root.join(rel)).ok();
                    journal.files.insert(
                        rel.clone(),
                        JournalEntry {
                            local_size: meta.as_ref().map(|m| m.len()).unwrap_or(l.size),
                            local_mtime_ms: meta
                                .as_ref()
                                .and_then(|m| m.modified().ok())
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(l.mtime_ms),
                            remote_id: uploaded.id,
                            remote_md5: uploaded.md5,
                            remote_modified: uploaded.modified,
                        },
                    );
                } else if local_changed {
                    let uploaded = upload_media(app, &r.id, &root.join(rel))?;
                    outcome.uploaded += 1;
                    journal.files.insert(
                        rel.clone(),
                        JournalEntry {
                            local_size: l.size,
                            local_mtime_ms: l.mtime_ms,
                            remote_id: uploaded.id,
                            remote_md5: uploaded.md5,
                            remote_modified: uploaded.modified,
                        },
                    );
                } else {
                    download_file(app, &r.id, &root.join(rel))?;
                    outcome.downloaded += 1;
                    let meta = std::fs::metadata(root.join(rel)).ok();
                    journal.files.insert(
                        rel.clone(),
                        JournalEntry {
                            local_size: meta.as_ref().map(|m| m.len()).unwrap_or(r.size),
                            local_mtime_ms: meta
                                .as_ref()
                                .and_then(|m| m.modified().ok())
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0),
                            remote_id: r.id.clone(),
                            remote_md5: r.md5.clone(),
                            remote_modified: r.modified.clone(),
                        },
                    );
                }
            }
            (Some(l), None) => {
                if recorded.is_some() {
                    // It was synced before and is gone from Drive now --
                    // that's a remote delete to mirror. Only when the
                    // local copy hasn't changed since; a local edit wins
                    // over a remote delete and gets re-uploaded instead.
                    let unchanged = recorded
                        .as_ref()
                        .map(|j| j.local_size == l.size && j.local_mtime_ms == l.mtime_ms)
                        .unwrap_or(false);
                    if unchanged {
                        deletions.push(rel.clone());
                        continue;
                    }
                }
                let (parent_rel, name) = split_rel(rel);
                let parent_id = ensure_remote_dir(app, &mut dir_ids, parent_rel)?;
                let uploaded = upload_new(app, &parent_id, name, &root.join(rel))?;
                outcome.uploaded += 1;
                journal.files.insert(
                    rel.clone(),
                    JournalEntry {
                        local_size: l.size,
                        local_mtime_ms: l.mtime_ms,
                        remote_id: uploaded.id,
                        remote_md5: uploaded.md5,
                        remote_modified: uploaded.modified,
                    },
                );
            }
            (None, Some(r)) => {
                if let Some(recorded) = &recorded {
                    // Deleted locally since the last pass: mirror that to
                    // Drive, unless the remote copy changed too (in which
                    // case the newer remote file comes back down).
                    if recorded.remote_md5 == r.md5 && recorded.remote_modified == r.modified {
                        delete_remote(app, &r.id)?;
                        outcome.deleted_remote += 1;
                        journal.files.remove(rel);
                        continue;
                    }
                }
                download_file(app, &r.id, &root.join(rel))?;
                outcome.downloaded += 1;
                let meta = std::fs::metadata(root.join(rel)).ok();
                journal.files.insert(
                    rel.clone(),
                    JournalEntry {
                        local_size: meta.as_ref().map(|m| m.len()).unwrap_or(r.size),
                        local_mtime_ms: meta
                            .as_ref()
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0),
                        remote_id: r.id.clone(),
                        remote_md5: r.md5.clone(),
                        remote_modified: r.modified.clone(),
                    },
                );
            }
            (None, None) => {
                // Only in the journal: gone from both sides, nothing to
                // do but forget it.
                journal.files.remove(rel);
            }
        }
    }

    for rel in deletions {
        let path = root.join(&rel);
        if std::fs::remove_file(&path).is_ok() {
            outcome.deleted_local += 1;
        }
        journal.files.remove(&rel);
    }

    write_journal(app, &pair.local_path, &journal)?;
    outcome.summary = summarize(&outcome);
    if let Some(reporter) = reporter {
        reporter.finish();
    }
    Ok(outcome)
}

fn summarize(outcome: &SyncOutcome) -> String {
    let mut parts = Vec::new();
    if outcome.uploaded > 0 {
        parts.push(format!("{} uploaded", outcome.uploaded));
    }
    if outcome.downloaded > 0 {
        parts.push(format!("{} downloaded", outcome.downloaded));
    }
    if outcome.deleted_local > 0 {
        parts.push(format!("{} deleted here", outcome.deleted_local));
    }
    if outcome.deleted_remote > 0 {
        parts.push(format!("{} deleted on Drive", outcome.deleted_remote));
    }
    if !outcome.conflicts.is_empty() {
        parts.push(format!(
            "{} changed on both sides -- kept both copies ({})",
            outcome.conflicts.len(),
            outcome.conflicts.join(", ")
        ));
    }
    if !outcome.skipped.is_empty() {
        parts.push(format!(
            "{} Google Docs file(s) skipped (no downloadable file)",
            outcome.skipped.len()
        ));
    }
    if parts.is_empty() {
        "Already up to date.".to_string()
    } else {
        parts.join(", ")
    }
}

fn split_rel(rel: &str) -> (&str, &str) {
    match rel.rsplit_once('/') {
        Some((parent, name)) => (parent, name),
        None => ("", rel),
    }
}

/// The Drive folder id for a relative directory path, creating any
/// missing level on the way.
fn ensure_remote_dir(
    app: &tauri::AppHandle,
    dir_ids: &mut HashMap<String, String>,
    rel: &str,
) -> Result<String, String> {
    if let Some(id) = dir_ids.get(rel) {
        return Ok(id.clone());
    }
    let (parent_rel, name) = split_rel(rel);
    let parent_id = ensure_remote_dir(app, dir_ids, parent_rel)?;
    let id = find_or_create_folder(app, &parent_id, name)?;
    dir_ids.insert(rel.to_string(), id.clone());
    Ok(id)
}

// ---- background loop -------------------------------------------------

/// A phone can't hold a filesystem watcher on a folder for the whole
/// session the way the desktop does (the app is suspended the moment it
/// leaves the foreground, and Android will kill the process outright), so
/// this is a plain interval while the app is running, plus one pass right
/// away. `Sync Now` in the sheet is the deliberate, immediate path.
const LOOP_INTERVAL: Duration = Duration::from_secs(600);

#[derive(Default)]
pub struct MobileSyncState {
    loops: Mutex<HashMap<String, Arc<AtomicBool>>>,
    syncing: Arc<Mutex<HashSet<String>>>,
    last_error: Arc<Mutex<HashMap<String, String>>>,
}

pub fn start_loop(app: &tauri::AppHandle, state: &MobileSyncState, pair: MobilePair) {
    let key = pair.local_path.clone();
    let mut loops = state.loops.lock_safe();
    if loops.contains_key(&key) {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    loops.insert(key.clone(), stop.clone());
    drop(loops);
    let syncing = state.syncing.clone();
    let last_error = state.last_error.clone();
    let app = app.clone();
    std::thread::spawn(move || loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        // Re-read the pair every tick: it may have been unlinked, and the
        // folder may have moved.
        let Some(current) = list_pairs(&app).into_iter().find(|p| p.local_path == key) else {
            return;
        };
        syncing.lock_safe().insert(key.clone());
        let result = sync_pair(&app, &current, None);
        syncing.lock_safe().remove(&key);
        match result {
            Ok(_) => {
                last_error.lock_safe().remove(&key);
            }
            Err(e) => {
                last_error.lock_safe().insert(key.clone(), e);
            }
        }
        // Slept in slices so unlinking a pair doesn't leave a thread
        // sitting for another ten minutes.
        for _ in 0..(LOOP_INTERVAL.as_secs() / 5) {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

pub fn stop_loop(state: &MobileSyncState, local_path: &str) {
    if let Some(stop) = state.loops.lock_safe().remove(local_path) {
        stop.store(true, Ordering::Relaxed);
    }
}

// ---- commands --------------------------------------------------------

#[derive(Serialize)]
pub struct DriveConnection {
    pub connected: bool,
    pub account_email: String,
}

#[tauri::command]
pub fn drive_rest_connection(app: tauri::AppHandle) -> DriveConnection {
    match read_creds(&app) {
        Some(creds) => {
            DriveConnection { connected: true, account_email: creds.account_email }
        }
        None => DriveConnection { connected: false, account_email: String::new() },
    }
}

/// Sign in. Blocks until the browser comes back (or the flow times out),
/// with the consent URL pushed down `url_channel` immediately.
#[tauri::command]
pub async fn drive_rest_connect(
    app: tauri::AppHandle,
    client_id: String,
    client_secret: String,
    url_channel: tauri::ipc::Channel<String>,
) -> Result<String, String> {
    let client_id = client_id.trim().to_string();
    let client_secret = client_secret.trim().to_string();
    if client_id.is_empty() || client_secret.is_empty() {
        return Err("Both the client ID and the client secret are needed.".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || connect(&app, client_id, client_secret, url_channel))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn drive_rest_disconnect(
    app: tauri::AppHandle,
    state: tauri::State<'_, MobileSyncState>,
) -> Result<(), String> {
    for pair in list_pairs(&app) {
        stop_loop(&state, &pair.local_path);
    }
    let _ = std::fs::remove_file(creds_path(&app)?);
    Ok(())
}

#[tauri::command]
pub fn drive_rest_list_pairs(app: tauri::AppHandle) -> Vec<MobilePair> {
    list_pairs(&app)
}

#[tauri::command]
pub async fn drive_rest_add_pair(
    app: tauri::AppHandle,
    local_path: String,
    state: tauri::State<'_, MobileSyncState>,
) -> Result<MobilePair, String> {
    let mut pairs = list_pairs(&app);
    if pairs.iter().any(|p| p.local_path == local_path) {
        return Err("this folder is already linked".to_string());
    }
    let name = Path::new(&local_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Synced Folder".to_string());
    // Fails here rather than at the first sync if the account can't be
    // reached at all, so "Link Folder" means what it says.
    let handle = app.clone();
    let folder_name = name.clone();
    tauri::async_runtime::spawn_blocking(move || pair_folder_id(&handle, &folder_name))
        .await
        .map_err(|e| e.to_string())??;
    let pair = MobilePair { local_path, remote_folder_name: name };
    pairs.push(pair.clone());
    save_pairs(&app, &pairs)?;
    start_loop(&app, &state, pair.clone());
    Ok(pair)
}

#[tauri::command]
pub fn drive_rest_remove_pair(
    app: tauri::AppHandle,
    local_path: String,
    state: tauri::State<'_, MobileSyncState>,
) -> Result<(), String> {
    stop_loop(&state, &local_path);
    let mut pairs = list_pairs(&app);
    pairs.retain(|p| p.local_path != local_path);
    save_pairs(&app, &pairs)?;
    // The journal describes a pairing that no longer exists; keeping it
    // would make a re-link mistake "never synced" for "deleted".
    if let Ok(path) = journal_path(&app, &local_path) {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

#[tauri::command]
pub async fn drive_rest_sync_now(
    app: tauri::AppHandle,
    local_path: String,
    channel: tauri::ipc::Channel<crate::progress::ProgressEvent>,
    state: tauri::State<'_, MobileSyncState>,
    registry: tauri::State<'_, crate::ops::OpRegistry>,
) -> Result<SyncOutcome, String> {
    let pair = list_pairs(&app)
        .into_iter()
        .find(|p| p.local_path == local_path)
        .ok_or("that folder isn't linked to Google Drive")?;
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let syncing = state.syncing.clone();
    let last_error = state.last_error.clone();
    let handle = app.clone();
    let key = local_path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        if !syncing.lock_safe().insert(key.clone()) {
            return Err("this folder is already syncing".to_string());
        }
        let reporter = ProgressReporter::new_cancellable(channel, 1, cancel);
        let out = sync_pair(&handle, &pair, Some(&reporter));
        syncing.lock_safe().remove(&key);
        match &out {
            Ok(_) => {
                last_error.lock_safe().remove(&key);
            }
            Err(e) => {
                last_error.lock_safe().insert(key.clone(), e.clone());
            }
        }
        out
    })
    .await
    .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    // A pair synced by hand before the app ever started its loop (right
    // after linking, say) should keep syncing on its own afterwards.
    if let Some(pair) = list_pairs(&app).into_iter().find(|p| p.local_path == local_path) {
        start_loop(&app, &state, pair);
    }
    result
}

#[derive(Serialize)]
pub struct MobileSyncStatus {
    pub syncing: bool,
    pub last_error: String,
}

#[tauri::command]
pub fn drive_rest_status(
    local_path: String,
    state: tauri::State<'_, MobileSyncState>,
) -> MobileSyncStatus {
    MobileSyncStatus {
        syncing: state.syncing.lock_safe().contains(&local_path),
        last_error: state.last_error.lock_safe().get(&local_path).cloned().unwrap_or_default(),
    }
}

/// Whether *any* linked folder is syncing right now -- for the same
/// toolbar activity badge the desktop's rclone sync lights up.
#[tauri::command]
pub fn drive_rest_syncing_now(state: tauri::State<'_, MobileSyncState>) -> bool {
    !state.syncing.lock_safe().is_empty()
}

// ---- md5 -------------------------------------------------------------

/// A small md5 implementation. Everything else this app hashes uses
/// SHA-256 (`sha2`), but Drive only publishes md5 for its files, so
/// comparing content against the remote side needs md5 specifically --
/// and pulling in a crate whose only job is a broken-for-crypto hash,
/// used here purely as a change detector, isn't worth the dependency.
struct Md5 {
    state: [u32; 4],
    buffer: [u8; 64],
    buffered: usize,
    length: u64,
}

const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

impl Md5 {
    fn new() -> Self {
        Md5 {
            state: [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476],
            buffer: [0u8; 64],
            buffered: 0,
            length: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.length = self.length.wrapping_add(data.len() as u64);
        if self.buffered > 0 {
            let take = (64 - self.buffered).min(data.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&data[..take]);
            self.buffered += take;
            data = &data[take..];
            if self.buffered == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffered = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffered = data.len();
        }
    }

    fn finish(mut self) -> String {
        let bit_len = self.length.wrapping_mul(8);
        self.update(&[0x80]);
        // `update` counted the padding into `length`, so the tail length
        // is captured above, before any of it is added.
        while self.buffered != 56 {
            self.update(&[0x00]);
        }
        let mut block = self.buffer;
        block[56..].copy_from_slice(&bit_len.to_le_bytes());
        self.compress(&block);
        let mut out = String::with_capacity(32);
        for word in self.state {
            for byte in word.to_le_bytes() {
                out.push_str(&format!("{byte:02x}"));
            }
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut m = [0u32; 16];
        for (i, chunk) in block.chunks_exact(4).enumerate() {
            m[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        let [mut a, mut b, mut c, mut d] = self.state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let k = ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32;
            let tmp = d;
            d = c;
            c = b;
            let sum = a
                .wrapping_add(f)
                .wrapping_add(k)
                .wrapping_add(m[g]);
            b = b.wrapping_add(sum.rotate_left(MD5_S[i]));
            a = tmp;
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_known_vectors() {
        let mut h = Md5::new();
        h.update(b"");
        assert_eq!(h.finish(), "d41d8cd98f00b204e9800998ecf8427e");
        let mut h = Md5::new();
        h.update(b"abc");
        assert_eq!(h.finish(), "900150983cd24fb0d6963f7d28e17f72");
        let mut h = Md5::new();
        h.update(b"The quick brown fox jumps over the lazy dog");
        assert_eq!(h.finish(), "9e107d9d372bb6826bd81d3542a419d6");
        // Spans several compression blocks, and lands the padding in the
        // "needs a second block" case.
        let mut h = Md5::new();
        h.update(&vec![b'a'; 1000]);
        assert_eq!(h.finish(), "cabe45dcc9ae5b66ba86600cca6b8ba8");
    }

    #[test]
    fn md5_is_the_same_in_pieces() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let mut whole = Md5::new();
        whole.update(&data);
        let whole = whole.finish();
        let mut split = Md5::new();
        for chunk in data.chunks(37) {
            split.update(chunk);
        }
        assert_eq!(whole, split.finish());
    }

    #[test]
    fn conflict_names_keep_the_extension_and_folder() {
        assert_eq!(conflict_name("notes.md"), "notes (from Drive).md");
        assert_eq!(conflict_name("a/b/notes.md"), "a/b/notes (from Drive).md");
        assert_eq!(conflict_name("README"), "README (from Drive)");
    }
}
