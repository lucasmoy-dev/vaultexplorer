//! Device-to-device folder sync via a Syncthing daemon this app manages
//! itself -- peer-to-peer directly between devices, no cloud account, no
//! OAuth, no registering anything with anyone. Runs its own isolated
//! Syncthing instance (its own config/data directory under this app's own
//! config folder) rather than assuming, or interfering with, any
//! system-wide Syncthing the user might separately have running for
//! something else.
//!
//! Talks to Syncthing entirely over its local REST API (127.0.0.1, its
//! own randomly-generated API key, read straight out of the config file
//! it generates itself) -- verified end-to-end against two real,
//! independent Syncthing daemons before writing any of this: device
//! pairing, folder sharing, the pending-device/pending-folder request
//! flow (what shows up on the *other* side after only one side has
//! configured the pairing), and actual file propagation all confirmed
//! working exactly as wrapped here.

use crate::errmap::ToStringErr;
use serde::Serialize;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}
fn syncthing_home() -> PathBuf {
    PathBuf::from(format!("{}/.config/vaultexplorer/syncthing-home", home_dir()))
}
fn config_xml_path() -> PathBuf {
    syncthing_home().join("config.xml")
}

pub fn is_installed() -> bool {
    Command::new("which").arg("syncthing").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Tiny hand-rolled single-tag extraction -- pulling in a real XML parser
/// for a couple of flat, always-present tags in a config file this app's
/// own dedicated Syncthing instance generates for itself isn't worth a
/// new dependency. Only handles a bare `<tag>...</tag>` (no attributes on
/// the opening tag) -- fine for `<address>`/`<apikey>`, not for `<gui ...>`
/// itself (see `gui_section` for that one).
fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}

/// The `<gui ...>...</gui>` block's inner content, as a slice -- config.xml
/// has *another* `<address>` tag before this one (a device's own
/// `<address>dynamic</address>` entry), so extracting `address`/`apikey`
/// from the *whole* file via `extract_xml_tag` would silently grab that
/// one instead (found the hard way: it always returned the literal string
/// "dynamic" as the "GUI address").
fn gui_section(xml: &str) -> Option<&str> {
    let start_tag = xml.find("<gui ")?;
    let content_start = xml[start_tag..].find('>')? + start_tag + 1;
    let end = xml[content_start..].find("</gui>")? + content_start;
    Some(&xml[content_start..end])
}

fn gui_address_and_key() -> Result<(String, String), String> {
    let xml = std::fs::read_to_string(config_xml_path())
        .map_err(|_| "Syncthing hasn't generated its config yet".to_string())?;
    let gui = gui_section(&xml).ok_or("no <gui> section in Syncthing's config")?;
    let address = extract_xml_tag(gui, "address").ok_or("no <address> in Syncthing's <gui> config")?;
    let apikey = extract_xml_tag(gui, "apikey").ok_or("no <apikey> in Syncthing's <gui> config")?;
    Ok((address, apikey))
}

async fn ping(address: &str, key: &str) -> bool {
    reqwest::Client::new()
        .get(format!("http://{address}/rest/system/ping"))
        .header("X-API-Key", key)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Make sure this app's own dedicated Syncthing daemon is up, generating
/// its config/identity on first use and spawning it if it isn't already
/// running (from a previous session, or a moment ago). Deliberately
/// doesn't track or kill the spawned process on app exit -- like any
/// normal Syncthing install, it should keep running and syncing in the
/// background regardless of whether this app's window is open, which is
/// the whole point of a P2P sync daemon. Idempotent, safe to call before
/// every operation.
pub async fn ensure_running() -> Result<(String, String), String> {
    if !is_installed() {
        return Err("syncthing isn't installed".to_string());
    }
    if let Ok((address, key)) = gui_address_and_key() {
        if ping(&address, &key).await {
            return Ok((address, key));
        }
    }
    if !config_xml_path().exists() {
        std::fs::create_dir_all(syncthing_home()).str_err()?;
        let status = Command::new("syncthing")
            .arg("generate")
            .arg("--home")
            .arg(syncthing_home())
            .status()
            .str_err()?;
        if !status.success() {
            return Err("syncthing generate failed".to_string());
        }
    }
    Command::new("syncthing")
        .args(["serve", "--no-browser", "--no-restart"])
        .arg("--home")
        .arg(syncthing_home())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .str_err()?;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Ok((address, key)) = gui_address_and_key() {
            if ping(&address, &key).await {
                return Ok((address, key));
            }
        }
    }
    Err("Syncthing didn't come up in time".to_string())
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

async fn get(address: &str, key: &str, path: &str) -> Result<serde_json::Value, String> {
    let resp = client()
        .get(format!("http://{address}{path}"))
        .header("X-API-Key", key)
        .send()
        .await
        .str_err()?;
    if !resp.status().is_success() {
        return Err(format!("Syncthing API error ({}): {}", resp.status(), resp.text().await.unwrap_or_default()));
    }
    resp.json().await.str_err()
}

async fn put(address: &str, key: &str, path: &str, body: serde_json::Value) -> Result<(), String> {
    let resp = client()
        .put(format!("http://{address}{path}"))
        .header("X-API-Key", key)
        .json(&body)
        .send()
        .await
        .str_err()?;
    if !resp.status().is_success() {
        return Err(format!("Syncthing API error ({}): {}", resp.status(), resp.text().await.unwrap_or_default()));
    }
    Ok(())
}

async fn delete(address: &str, key: &str, path: &str) -> Result<(), String> {
    let resp = client()
        .delete(format!("http://{address}{path}"))
        .header("X-API-Key", key)
        .send()
        .await
        .str_err()?;
    if !resp.status().is_success() {
        return Err(format!("Syncthing API error ({}): {}", resp.status(), resp.text().await.unwrap_or_default()));
    }
    Ok(())
}

pub async fn my_device_id() -> Result<String, String> {
    let (address, key) = ensure_running().await?;
    let status = get(&address, &key, "/rest/system/status").await?;
    status["myID"].as_str().map(str::to_string).ok_or_else(|| "no myID in Syncthing's status".to_string())
}

#[derive(Serialize)]
pub struct DeviceStatus {
    pub id: String,
    pub name: String,
    pub connected: bool,
}

/// Every device this instance knows about (excluding itself), with live
/// connection status cross-referenced from `/rest/system/connections`.
pub async fn list_devices() -> Result<Vec<DeviceStatus>, String> {
    let (address, key) = ensure_running().await?;
    let my_id = my_device_id().await?;
    let devices = get(&address, &key, "/rest/config/devices").await?;
    let connections = get(&address, &key, "/rest/system/connections").await?;
    let empty = serde_json::Map::new();
    let conn_map = connections["connections"].as_object().unwrap_or(&empty);
    let mut out = Vec::new();
    for d in devices.as_array().cloned().unwrap_or_default() {
        let id = d["deviceID"].as_str().unwrap_or_default().to_string();
        if id == my_id {
            continue;
        }
        let name = d["name"].as_str().unwrap_or(&id).to_string();
        let connected = conn_map.get(&id).and_then(|c| c["connected"].as_bool()).unwrap_or(false);
        out.push(DeviceStatus { id, name, connected });
    }
    Ok(out)
}

pub async fn add_device(id: &str, name: &str) -> Result<(), String> {
    let (address, key) = ensure_running().await?;
    put(
        &address,
        &key,
        &format!("/rest/config/devices/{id}"),
        serde_json::json!({ "deviceID": id, "name": name }),
    )
    .await
}

pub async fn remove_device(id: &str) -> Result<(), String> {
    let (address, key) = ensure_running().await?;
    delete(&address, &key, &format!("/rest/config/devices/{id}")).await
}

#[derive(Serialize)]
pub struct FolderStatus {
    pub id: String,
    pub label: String,
    pub path: String,
    pub device_ids: Vec<String>,
}

pub async fn list_folders() -> Result<Vec<FolderStatus>, String> {
    let (address, key) = ensure_running().await?;
    let folders = get(&address, &key, "/rest/config/folders").await?;
    let mut out = Vec::new();
    for f in folders.as_array().cloned().unwrap_or_default() {
        let device_ids = f["devices"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|d| d["deviceID"].as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        out.push(FolderStatus {
            id: f["id"].as_str().unwrap_or_default().to_string(),
            label: f["label"].as_str().unwrap_or_default().to_string(),
            path: f["path"].as_str().unwrap_or_default().to_string(),
            device_ids,
        });
    }
    Ok(out)
}

/// Every shared folder's real local path whose Syncthing-reported state
/// is actively "syncing" right now (as opposed to "idle"/"scanning") --
/// the same live-status convention used for git/local/Drive sync, backed
/// here by Syncthing's own per-folder `/rest/db/status` rather than a
/// flag this app tracks itself, since the daemon already knows.
pub async fn syncing_now() -> Result<Vec<String>, String> {
    let (address, key) = ensure_running().await?;
    let folders = list_folders().await?;
    let mut out = Vec::new();
    for f in folders {
        let status = get(&address, &key, &format!("/rest/db/status?folder={}", f.id)).await;
        if let Ok(status) = status {
            if status["state"].as_str() == Some("syncing") {
                out.push(f.path);
            }
        }
    }
    Ok(out)
}

/// Share `path` (a real local folder) under `folder_id` with every device
/// in `device_ids`. Also used to *accept* an incoming pending-folder
/// request: same PUT, just choosing where this side stores it -- calling
/// this with the `folder_id` a remote device already offered clears it
/// from `pending_folders()` automatically (confirmed).
pub async fn share_folder(folder_id: &str, label: &str, path: &str, device_ids: &[String]) -> Result<(), String> {
    let (address, key) = ensure_running().await?;
    let devices: Vec<serde_json::Value> =
        device_ids.iter().map(|id| serde_json::json!({ "deviceID": id })).collect();
    put(
        &address,
        &key,
        &format!("/rest/config/folders/{folder_id}"),
        serde_json::json!({
            "id": folder_id,
            "label": label,
            "path": path,
            "type": "sendreceive",
            "devices": devices,
        }),
    )
    .await
}

pub async fn remove_folder(folder_id: &str) -> Result<(), String> {
    let (address, key) = ensure_running().await?;
    delete(&address, &key, &format!("/rest/config/folders/{folder_id}")).await
}

#[derive(Serialize)]
pub struct PendingDevice {
    pub id: String,
}

/// Devices that have tried to connect to this one but aren't in its
/// config yet -- the "X wants to connect" case. Accept with `add_device`.
pub async fn pending_devices() -> Result<Vec<PendingDevice>, String> {
    let (address, key) = ensure_running().await?;
    let pending = get(&address, &key, "/rest/cluster/pending/devices").await?;
    Ok(pending
        .as_object()
        .map(|m| m.keys().map(|id| PendingDevice { id: id.clone() }).collect())
        .unwrap_or_default())
}

#[derive(Serialize)]
pub struct PendingFolder {
    pub id: String,
    pub label: String,
    pub offered_by_device_id: String,
}

/// Folders a paired device has shared with this one that it hasn't
/// accepted (chosen a local path for) yet. Accept with `share_folder`
/// using the same `id`.
pub async fn pending_folders() -> Result<Vec<PendingFolder>, String> {
    let (address, key) = ensure_running().await?;
    let pending = get(&address, &key, "/rest/cluster/pending/folders").await?;
    let mut out = Vec::new();
    if let Some(map) = pending.as_object() {
        for (folder_id, info) in map {
            let Some(offered_by) = info["offeredBy"].as_object() else { continue };
            for (device_id, detail) in offered_by {
                out.push(PendingFolder {
                    id: folder_id.clone(),
                    label: detail["label"].as_str().unwrap_or(folder_id).to_string(),
                    offered_by_device_id: device_id.clone(),
                });
            }
        }
    }
    Ok(out)
}

// ---- Tauri commands ----

#[tauri::command]
pub fn syncthing_installed() -> bool {
    is_installed()
}

#[tauri::command]
pub async fn syncthing_syncing_now() -> Result<Vec<String>, String> {
    syncing_now().await
}

/// Renders `data` (a `vaultexplorer://add-device?...` link, in practice)
/// as a scannable QR code, raw SVG markup -- so pairing with another
/// machine is "point a phone camera at this" instead of copy-pasting a
/// 60-some-character device ID by hand. This app only ever generates a
/// QR here, never reads one back via a camera itself (the other side is
/// whatever scanned it, or the optional link shared some other way).
#[tauri::command]
pub fn syncthing_qr_svg(data: String) -> Result<String, String> {
    let code = qrcode::QrCode::new(data.as_bytes()).str_err()?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .build())
}

#[tauri::command]
pub async fn syncthing_my_device_id() -> Result<String, String> {
    my_device_id().await
}

#[tauri::command]
pub async fn syncthing_list_devices() -> Result<Vec<DeviceStatus>, String> {
    list_devices().await
}

#[tauri::command]
pub async fn syncthing_add_device(id: String, name: String) -> Result<(), String> {
    add_device(&id, &name).await
}

#[tauri::command]
pub async fn syncthing_remove_device(id: String) -> Result<(), String> {
    remove_device(&id).await
}

#[tauri::command]
pub async fn syncthing_list_folders() -> Result<Vec<FolderStatus>, String> {
    list_folders().await
}

#[tauri::command]
pub async fn syncthing_share_folder(
    folder_id: String,
    label: String,
    path: String,
    device_ids: Vec<String>,
) -> Result<(), String> {
    share_folder(&folder_id, &label, &path, &device_ids).await
}

#[tauri::command]
pub async fn syncthing_remove_folder(folder_id: String) -> Result<(), String> {
    remove_folder(&folder_id).await
}

#[tauri::command]
pub async fn syncthing_pending_devices() -> Result<Vec<PendingDevice>, String> {
    pending_devices().await
}

#[tauri::command]
pub async fn syncthing_pending_folders() -> Result<Vec<PendingFolder>, String> {
    pending_folders().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// A disposable Syncthing instance with its own home dir and a
    /// randomly-chosen GUI port (so parallel test runs / the real app's
    /// own instance never collide with these).
    struct TestInstance {
        home: PathBuf,
        address: String,
        key: String,
        listen_port: u32,
        child: std::process::Child,
    }
    impl Drop for TestInstance {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    async fn spawn_test_instance() -> TestInstance {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id() % 1000;
        let gui_port = 31000 + pid * 10 + n * 2;
        // `generate` picks its own random `tcp://0.0.0.0:PORT` listen
        // address per instance on this Syncthing build (not the literal
        // placeholder text "default" older docs/versions describe) -- so
        // two disposable instances don't actually collide on port 22000
        // by default here. But this test still needs to know each
        // instance's *real* listen port up front, to give the other side
        // an explicit dial address instead of relying on slow/flaky
        // internet discovery -- so it's overridden to a value derived the
        // same way as the GUI port, deterministic and known before the
        // process even starts.
        let listen_port = gui_port + 1;
        let home = PathBuf::from(format!("/tmp/ve-syncthing-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert!(Command::new("syncthing")
            .arg("generate")
            .arg("--home")
            .arg(&home)
            .status()
            .unwrap()
            .success());
        let config_path = home.join("config.xml");
        let xml = std::fs::read_to_string(&config_path).unwrap();
        // Must replace the *whole* `<address>...</address>` tag, scoped to
        // inside `<gui>`, not just the inner text -- a bare text-only
        // replace of the old address value risks matching unrelated parts
        // of the file that happen to contain the same substring (this bit
        // everyone the first time this test was written: extracting
        // "address" from the *whole* file grabbed a device's own
        // `<address>dynamic</address>` entry instead, and blindly
        // replacing every "dynamic" in the config corrupted it without
        // ever touching the real GUI port).
        let gui = gui_section(&xml).unwrap();
        let old_address_tag = format!("<address>{}</address>", extract_xml_tag(gui, "address").unwrap());
        // Whatever real `tcp://...` listen address `generate` assigned --
        // found by prefix rather than assumed to be the literal word
        // "default", which this build never actually writes to disk.
        let old_listen_tag = xml
            .lines()
            .find(|l| l.trim_start().starts_with("<listenAddress>tcp://"))
            .map(str::trim)
            .expect("generated config has no tcp:// listenAddress line");
        let new_xml = xml
            .replacen(&old_address_tag, &format!("<address>127.0.0.1:{gui_port}</address>"), 1)
            .replacen(
                old_listen_tag,
                &format!("<listenAddress>tcp://127.0.0.1:{listen_port}</listenAddress>"),
                1,
            );
        std::fs::write(&config_path, new_xml).unwrap();
        let key = extract_xml_tag(&std::fs::read_to_string(&config_path).unwrap(), "apikey").unwrap();
        let address = format!("127.0.0.1:{gui_port}");

        let log_path = home.join("stderr.log");
        let child = Command::new("syncthing")
            .args(["serve", "--no-browser", "--no-restart"])
            .arg("--home")
            .arg(&home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log_path).unwrap())
            .spawn()
            .unwrap();
        // Constructed (and so Drop-cleaned-up-on-panic) before the
        // readiness check, not after -- a `Child` on its own does *not*
        // kill its process on drop, so panicking before this point in an
        // earlier version of this test left orphaned `syncthing serve`
        // processes behind on every failed run.
        let instance = TestInstance { home, address, key, listen_port, child };
        let mut up = false;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if ping(&instance.address, &instance.key).await {
                up = true;
                break;
            }
        }
        if !up {
            let log = std::fs::read_to_string(&log_path).unwrap_or_default();
            panic!("test syncthing instance on {} never came up after 15s; stderr:\n{log}", instance.address);
        }
        instance
    }

    async fn my_id_of(inst: &TestInstance) -> String {
        get(&inst.address, &inst.key, "/rest/system/status").await.unwrap()["myID"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// Full real round-trip across two independent, disposable Syncthing
    /// daemons: pair them by device ID, share a folder one-sidedly, see
    /// it show up as pending on the other side, accept it, and confirm a
    /// file placed on either side actually reaches the other.
    #[tokio::test]
    async fn device_and_folder_pairing_syncs_a_real_file() {
        let a = spawn_test_instance().await;
        let b = spawn_test_instance().await;
        let (id_a, id_b) = (my_id_of(&a).await, my_id_of(&b).await);

        // Explicit addresses rather than leaving them "dynamic" -- these
        // are two disposable instances on 127.0.0.1 with made-up ports
        // that no discovery mechanism (local broadcast or global) has any
        // reason to know about, so relying on discovery to find each
        // other would be slow at best and flaky at worst in a test.
        put(
            &a.address,
            &a.key,
            &format!("/rest/config/devices/{id_b}"),
            serde_json::json!({"deviceID": id_b, "name": "b", "addresses": [format!("tcp://127.0.0.1:{}", b.listen_port)]}),
        )
        .await
        .unwrap();
        put(
            &b.address,
            &b.key,
            &format!("/rest/config/devices/{id_a}"),
            serde_json::json!({"deviceID": id_a, "name": "a", "addresses": [format!("tcp://127.0.0.1:{}", a.listen_port)]}),
        )
        .await
        .unwrap();

        let dir_a = format!("{}/data", a.home.display());
        let dir_b = format!("{}/data", b.home.display());
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::write(format!("{dir_a}/hello.txt"), "from a").unwrap();

        // Only A configures the folder -- B should see it as pending.
        put(
            &a.address,
            &a.key,
            "/rest/config/folders/paired",
            serde_json::json!({"id": "paired", "label": "Paired", "path": dir_a, "type": "sendreceive", "devices": [{"deviceID": id_b}]}),
        )
        .await
        .unwrap();

        // Generous timeout -- under load (e.g. the test suite's other
        // tests spawning their own background processes/watchers
        // concurrently) two disposable daemons discovering and
        // connecting to each other can take a few seconds longer than in
        // isolation.
        let mut saw_pending = false;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let Ok(pending) = get(&b.address, &b.key, "/rest/cluster/pending/folders").await else {
                continue;
            };
            if pending.as_object().is_some_and(|m| m.contains_key("paired")) {
                saw_pending = true;
                break;
            }
        }
        assert!(saw_pending, "folder B never saw as pending from A's one-sided share");

        // B accepts, choosing its own local path.
        put(
            &b.address,
            &b.key,
            "/rest/config/folders/paired",
            serde_json::json!({"id": "paired", "label": "Paired", "path": dir_b, "type": "sendreceive", "devices": [{"deviceID": id_a}]}),
        )
        .await
        .unwrap();

        let mut synced = false;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(300)).await;
            if std::fs::read_to_string(format!("{dir_b}/hello.txt")).map(|s| s == "from a").unwrap_or(false) {
                synced = true;
                break;
            }
        }
        assert!(synced, "file never propagated from A to B after accepting the pending folder");
    }

    #[test]
    fn qr_svg_encodes_the_given_link() {
        let svg = syncthing_qr_svg("vaultexplorer://add-device?id=ABC123".to_string()).unwrap();
        assert!(svg.contains("<svg"), "expected real SVG markup, got: {svg}");
        assert!(svg.contains("</svg>"));
    }
}
