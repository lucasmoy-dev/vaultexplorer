//! "Share" context-menu action: upload a file to a free, no-account file
//! host and hand back a direct link. Picking a host here isn't a settled
//! choice the way it would be for something durable -- these services
//! come and go (0x0.st has uploads disabled entirely as of this writing,
//! transfer.sh's server didn't even resolve), so this tries uguu.se
//! first (direct download links, ~48h retention) and falls back to
//! tmpfiles.org (a landing page rather than a raw link, longer-lived) if
//! that's unavailable, rather than hard-depending on exactly one host.

use crate::errmap::ToStringErr;
use serde::Deserialize;

const MAX_SHARE_BYTES: u64 = 100 * 1024 * 1024; // 100 MB -- generous for a quick share, small enough to not hang on an accidental huge file.

#[derive(Deserialize)]
struct UguuResponse {
    success: bool,
    files: Vec<UguuFile>,
}
#[derive(Deserialize)]
struct UguuFile {
    url: String,
}

#[derive(Deserialize)]
struct TmpfilesResponse {
    data: TmpfilesData,
}
#[derive(Deserialize)]
struct TmpfilesData {
    url: String,
}

async fn upload_to_uguu(bytes: Vec<u8>, filename: String) -> Result<String, String> {
    let part = reqwest::multipart::Part::bytes(bytes).file_name(filename);
    let form = reqwest::multipart::Form::new().part("files[]", part);
    let resp = reqwest::Client::new()
        .post("https://uguu.se/upload")
        .multipart(form)
        .send()
        .await
        .str_err()?;
    if !resp.status().is_success() {
        return Err(format!("uguu.se returned {}", resp.status()));
    }
    let parsed: UguuResponse = resp.json().await.str_err()?;
    if !parsed.success {
        return Err("uguu.se reported failure".to_string());
    }
    parsed
        .files
        .into_iter()
        .next()
        .map(|f| f.url)
        .ok_or_else(|| "uguu.se returned no file URL".to_string())
}

async fn upload_to_tmpfiles(bytes: Vec<u8>, filename: String) -> Result<String, String> {
    let part = reqwest::multipart::Part::bytes(bytes).file_name(filename);
    let form = reqwest::multipart::Form::new().part("file", part);
    let resp = reqwest::Client::new()
        .post("https://tmpfiles.org/api/v1/upload")
        .multipart(form)
        .send()
        .await
        .str_err()?;
    if !resp.status().is_success() {
        return Err(format!("tmpfiles.org returned {}", resp.status()));
    }
    let parsed: TmpfilesResponse = resp.json().await.str_err()?;
    Ok(parsed.data.url)
}

pub async fn share_bytes(bytes: Vec<u8>, filename: String) -> Result<String, String> {
    if bytes.len() as u64 > MAX_SHARE_BYTES {
        return Err(format!(
            "File is too large to share this way ({} MB, max {} MB)",
            bytes.len() / 1024 / 1024,
            MAX_SHARE_BYTES / 1024 / 1024
        ));
    }
    match upload_to_uguu(bytes.clone(), filename.clone()).await {
        Ok(url) => Ok(url),
        Err(_) => upload_to_tmpfiles(bytes, filename).await,
    }
}

pub async fn share_path(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).str_err()?;
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    share_bytes(bytes, filename).await
}

// ---- Tauri commands ----

#[tauri::command]
pub async fn fs_share_file(path: String) -> Result<String, String> {
    share_path(&path).await
}

#[tauri::command]
pub async fn vault_share_file(state: tauri::State<'_, crate::AppState>, rel_path: String) -> Result<String, String> {
    let bytes = crate::with_vault(&state, |v| v.decrypt_file(&rel_path))?;
    let filename = std::path::Path::new(&rel_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    share_bytes(bytes, filename).await
}
