//! GitHub-release update flow. The repo is a monorepo, so releases for THIS
//! app are tagged with a prefix (`life-framework-vX.Y.Z`) and the built APK
//! is attached as a release asset.
use serde_json::Value;

// The monorepo's current name. Releases (and their APK assets) live here,
// tagged `life-framework-vX.Y.Z`. If/when the repo is renamed to
// `personal-projects`, GitHub keeps redirecting this path, so the updater
// keeps working without a rebuild.
/// Both names, because the repo is in the middle of being renamed from
/// `vaultexplorer` to `personal-projects`. GitHub redirects a renamed repo's
/// API, but a shipped build that knows only one name depends on that
/// redirect surviving -- and if the old name is later taken by somebody
/// else, it would point at a stranger's releases. Trying both means this
/// build works before the rename and after it.
const REPOS: [&str; 2] = ["lucasmoy-dev/personal-projects", "lucasmoy-dev/vaultexplorer"];

/// The name to show in a "releases page" link: the older one, since that is
/// what exists until the rename happens.
const REPO: &str = REPOS[1];

/// The releases list, from whichever name answers (see [`REPOS`]).
fn fetch_releases(client: &reqwest::blocking::Client) -> Result<Vec<Value>, String> {
    let mut last = String::new();
    for repo in REPOS {
        let attempt = client
            .get(format!("https://api.github.com/repos/{repo}/releases?per_page=30"))
            .header("Accept", "application/vnd.github+json")
            .send()
            .map_err(|e| e.to_string())
            .and_then(|r| r.error_for_status().map_err(|e| e.to_string()))
            .and_then(|r| r.json::<Vec<Value>>().map_err(|e| e.to_string()));
        match attempt {
            Ok(releases) => return Ok(releases),
            Err(error) => last = error,
        }
    }
    Err(last)
}

const TAG_PREFIX: &str = "life-framework-v";

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub notes: String,
    pub apk_url: String,
    pub html_url: String,
    pub has_update: bool,
}

type SemVer = (u64, u64, u64);

fn parse_semver(s: &str) -> SemVer {
    // Take the leading "x.y.z", ignoring any pre-release/build suffix.
    let core = s.split(|c: char| c == '-' || c == '+').next().unwrap_or("");
    let mut it = core.split('.').map(|p| p.trim().parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

#[tauri::command]
pub fn app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

/// True where the one-tap installer path exists (Android only).
#[tauri::command]
pub fn can_auto_install() -> bool {
    cfg!(target_os = "android")
}

#[tauri::command]
pub fn check_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let cur_ver = parse_semver(&current);
    let releases_page = format!("https://github.com/{REPO}/releases");

    let client = reqwest::blocking::Client::builder()
        .user_agent("life-framework-updater")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let releases: Vec<Value> = fetch_releases(&client)?;

    // Highest semver among our-prefixed, non-draft releases.
    let mut best: Option<(SemVer, Value)> = None;
    for r in releases {
        if r.get("draft").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let tag = r.get("tag_name").and_then(Value::as_str).unwrap_or("");
        if !tag.starts_with(TAG_PREFIX) {
            continue;
        }
        let ver = parse_semver(&tag[TAG_PREFIX.len()..]);
        if best.as_ref().map(|(bv, _)| ver > *bv).unwrap_or(true) {
            best = Some((ver, r));
        }
    }

    let Some((ver, rel)) = best else {
        return Ok(UpdateInfo {
            current: current.clone(),
            latest: current,
            notes: "Todavía no hay releases publicadas para esta app.".into(),
            apk_url: String::new(),
            html_url: releases_page,
            has_update: false,
        });
    };

    let apk_url = rel
        .get("assets")
        .and_then(Value::as_array)
        .and_then(|assets| {
            assets.iter().find(|a| {
                a.get("name")
                    .and_then(Value::as_str)
                    .map(|n| n.to_ascii_lowercase().ends_with(".apk"))
                    .unwrap_or(false)
            })
        })
        .and_then(|a| a.get("browser_download_url").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();

    Ok(UpdateInfo {
        current,
        latest: format!("{}.{}.{}", ver.0, ver.1, ver.2),
        notes: rel.get("body").and_then(Value::as_str).unwrap_or("").to_string(),
        html_url: rel
            .get("html_url")
            .and_then(Value::as_str)
            .unwrap_or(&releases_page)
            .to_string(),
        apk_url,
        has_update: ver > cur_ver,
    })
}

/// Download the APK and hand it to the system installer (Android). On other
/// platforms the JS side uses the browser-link fallback instead.
#[tauri::command]
pub fn download_and_install_update(app: tauri::AppHandle, url: String) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        crate::android::install_apk(app, url)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, url);
        Err("La instalación automática solo está disponible en Android; usá el botón de navegador.".into())
    }
}
