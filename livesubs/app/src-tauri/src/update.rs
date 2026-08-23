//! "Is there a newer version?", against GitHub Releases.
//!
//! Same convention as the other apps in this monorepo (see
//! `life-framework/app/src-tauri/src/update.rs` and the Android app's
//! `Updater.kt`): the repo holds several projects, so a release belongs to
//! an app by its **tag prefix**, and the built artifacts are attached as
//! assets. This app looks for `livesubs-vX.Y.Z`.
//!
//! Where the two platforms differ: the Android app downloads its own APK
//! and hands it to the system installer, because that is the only way to
//! update a sideloaded app. Here it opens the release page instead --
//! replacing a running binary underneath itself is not something a desktop
//! app should do quietly, and the `.deb`/AppImage is a two-click download.

use serde::Serialize;

/// The monorepo's current name on GitHub. A rename to `personal-projects`
/// keeps working through GitHub's redirect, so this needs no rebuild.
const REPO: &str = "lucasmoy-dev/vaultexplorer";
const TAG_PREFIX: &str = "livesubs-v";

type SemVer = (u64, u64, u64);

#[derive(Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub notes: String,
    pub page_url: String,
    pub has_update: bool,
}

/// The leading `x.y.z`, ignoring any pre-release or build suffix.
fn parse_semver(text: &str) -> SemVer {
    let core = text.split(|c: char| c == '-' || c == '+').next().unwrap_or("");
    let mut parts = core.split('.').map(|p| p.trim().parse::<u64>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

pub fn releases_page() -> String {
    format!("https://github.com/{REPO}/releases")
}

#[tauri::command]
pub fn app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
pub async fn check_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || check(&current))
        .await
        .map_err(|e| e.to_string())?
}

fn check(current: &str) -> Result<UpdateInfo, String> {
    let current_version = parse_semver(current);
    let page = releases_page();
    let client = reqwest::blocking::Client::builder()
        .user_agent("livesubs-updater")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let releases: Vec<serde_json::Value> = client
        .get(format!("https://api.github.com/repos/{REPO}/releases?per_page=30"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| format!("no se pudo consultar GitHub: {e}"))?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    // Highest version among this app's own, published releases.
    let mut best: Option<(SemVer, serde_json::Value)> = None;
    for release in releases {
        if release.get("draft").and_then(serde_json::Value::as_bool).unwrap_or(false) {
            continue;
        }
        let tag = release.get("tag_name").and_then(serde_json::Value::as_str).unwrap_or("");
        if !tag.starts_with(TAG_PREFIX) {
            continue;
        }
        let version = parse_semver(&tag[TAG_PREFIX.len()..]);
        if best.as_ref().map(|(best, _)| version > *best).unwrap_or(true) {
            best = Some((version, release));
        }
    }

    let Some((version, release)) = best else {
        return Ok(UpdateInfo {
            current: current.to_string(),
            latest: current.to_string(),
            notes: "Todavía no hay releases publicadas para esta app.".to_string(),
            page_url: page,
            has_update: false,
        });
    };

    Ok(UpdateInfo {
        current: current.to_string(),
        latest: format!("{}.{}.{}", version.0, version.1, version.2),
        notes: release
            .get("body")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .chars()
            .take(400)
            .collect(),
        page_url: release
            .get("html_url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&page)
            .to_string(),
        has_update: version > current_version,
    })
}

#[tauri::command]
pub fn open_releases_page(url: String) -> Result<(), String> {
    // xdg-open, same as the transcript: the desktop's own association is
    // exactly the behaviour wanted, and it costs no dependency.
    std::process::Command::new("xdg-open")
        .arg(if url.is_empty() { releases_page() } else { url })
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_with_and_without_suffixes() {
        assert_eq!(parse_semver("0.1.0"), (0, 1, 0));
        assert_eq!(parse_semver("1.2.3-beta1"), (1, 2, 3));
        assert_eq!(parse_semver("1.2.3+build9"), (1, 2, 3));
        assert_eq!(parse_semver("2"), (2, 0, 0));
        assert_eq!(parse_semver("nonsense"), (0, 0, 0));
    }

    #[test]
    fn double_digit_components_compare_numerically() {
        // The mistake this guards against: string comparison would put
        // 0.9.0 above 0.10.0 and the update would never be offered.
        assert!(parse_semver("0.10.0") > parse_semver("0.9.0"));
        assert!(parse_semver("1.0.0") > parse_semver("0.99.99"));
        assert!(!(parse_semver("0.1.0") > parse_semver("0.1.0")));
    }
}
