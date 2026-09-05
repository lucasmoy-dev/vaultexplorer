//! A typed client for the Syncthing REST API, narrowed to what HomeCloud needs.
//!
//! Every method here answers a question the user interface actually asks. The
//! shapes Syncthing returns are deliberately not re-exported: they are an
//! implementation detail that stops at this module's edge.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::model::{FolderState, Invitation, OfferedFolder, Peer, Settings, SharedFolder, ThisDevice};
use crate::pairing::PairingCode;

pub struct Syncthing {
    base: String,
    api_key: String,
    http: reqwest::Client,
}

/// A conflict scan gives up past this many entries. A folder large enough to
/// hit the cap is one where an exact count is not worth the disk churn on
/// every poll; the badge just says "some".
const CONFLICT_SCAN_CAP: usize = 50_000;

impl Syncthing {
    pub fn new(base: impl Into<String>, api_key: impl Into<String>) -> Self {
        Syncthing {
            base: base.into(),
            api_key: api_key.into(),
            http: reqwest::Client::new(),
        }
    }

    async fn request(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let url = format!("{}{}", self.base, path);
        let mut req = self.http.request(method, &url).header("X-API-Key", &self.api_key);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(Error::Api { status: status.as_u16(), body: text });
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&text)?)
    }

    async fn get(&self, path: &str) -> Result<Value> {
        self.request(reqwest::Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.request(reqwest::Method::POST, path, Some(body)).await
    }

    async fn patch(&self, path: &str, body: Value) -> Result<Value> {
        self.request(reqwest::Method::PATCH, path, Some(body)).await
    }

    async fn delete(&self, path: &str) -> Result<Value> {
        self.request(reqwest::Method::DELETE, path, None).await
    }

    /// Resolves once the engine answers, so callers can wait for a freshly
    /// spawned process without guessing at a sleep.
    pub async fn ping(&self) -> Result<()> {
        self.get("/rest/system/ping").await.map(|_| ())
    }

    pub async fn this_device(&self) -> Result<ThisDevice> {
        let status = self.get("/rest/system/status").await?;
        let id = status["myID"].as_str().unwrap_or_default().to_string();
        let name = self
            .get(&format!("/rest/config/devices/{id}"))
            .await
            .ok()
            .and_then(|d| d["name"].as_str().map(str::to_string))
            .unwrap_or_default();
        Ok(ThisDevice { id, name })
    }

    pub async fn set_this_device_name(&self, name: &str) -> Result<()> {
        let id = self.this_device().await?.id;
        self.patch(&format!("/rest/config/devices/{id}"), json!({ "name": name })).await?;
        Ok(())
    }

    /// Applies the settings that make HomeCloud behave the way it promises,
    /// regardless of what the engine's own defaults happen to be.
    pub async fn apply_house_defaults(&self) -> Result<()> {
        self.patch(
            "/rest/config/options",
            json!({
                // Sending usage reports is not ours to opt into on someone's behalf.
                "urAccepted": -1,
                // The engine must never swap itself out from under the app that
                // ships and signs it.
                "autoUpgradeIntervalH": 0,
            }),
        )
        .await?;
        self.lock_engine_web_ui().await
    }

    /// With no credentials set, the engine's own web interface serves any
    /// request that reaches it from localhost — which is every other program
    /// running as this user. HomeCloud authenticates with an API key instead, so
    /// a password is set here purely to close that door. It is random and
    /// immediately discarded because nothing is ever meant to log in with it.
    ///
    /// Done over the API rather than only at `generate` time so that installs
    /// created before this existed get fixed on their next launch.
    async fn lock_engine_web_ui(&self) -> Result<()> {
        let gui = self.get("/rest/config/gui").await?;
        if !gui["user"].as_str().unwrap_or("").is_empty() {
            return Ok(());
        }
        self.patch(
            "/rest/config/gui",
            json!({ "user": "homecloud", "password": random_secret() }),
        )
        .await?;
        Ok(())
    }

    // ---- folders -------------------------------------------------------

    pub async fn folders(&self) -> Result<Vec<SharedFolder>> {
        let configured: Vec<FolderConfig> = serde_json::from_value(self.get("/rest/config/folders").await?)?;
        let devices: Vec<DeviceConfig> = serde_json::from_value(self.get("/rest/config/devices").await?)?;
        let names: HashMap<&str, &str> =
            devices.iter().map(|d| (d.device_id.as_str(), d.name.as_str())).collect();
        let connected = self.connected_devices().await?;
        let me = self.this_device().await?.id;

        let mut out = Vec::with_capacity(configured.len());
        for folder in configured {
            let status = self.get(&format!("/rest/db/status?folder={}", folder.id)).await?;

            let peers: Vec<Peer> = folder
                .devices
                .iter()
                .filter(|d| d.device_id != me)
                .map(|d| Peer {
                    id: d.device_id.clone(),
                    name: names
                        .get(d.device_id.as_str())
                        .filter(|n| !n.is_empty())
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| short_id(&d.device_id)),
                    connected: connected.contains(&d.device_id),
                })
                .collect();

            out.push(SharedFolder {
                state: folder_state(&folder, &status, &peers),
                conflicts: count_conflicts(Path::new(&folder.path)),
                bytes: status["globalBytes"].as_u64().unwrap_or(0),
                files: status["globalFiles"].as_u64().unwrap_or(0),
                peers,
                id: folder.id,
                label: folder.label,
                path: folder.path,
            });
        }
        Ok(out)
    }

    async fn connected_devices(&self) -> Result<Vec<String>> {
        let value = self.get("/rest/system/connections").await?;
        Ok(value["connections"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter(|(_, v)| v["connected"].as_bool().unwrap_or(false))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Starts sharing a local directory and returns the code that lets another
    /// device join it. Bidirectional and watching for changes, because that is
    /// what "sync this folder" means to a person.
    pub async fn share_folder(&self, path: &str, label: &str) -> Result<PairingCode> {
        let me = self.this_device().await?;
        let folder_id = new_folder_id(label);

        self.post(
            "/rest/config/folders",
            json!({
                "id": folder_id,
                "label": label,
                "path": path,
                "type": "sendreceive",
                "fsWatcherEnabled": true,
                // The engine's default of 10s is what makes Syncthing feel
                // sluggish; a second reads as immediate without thrashing.
                "fsWatcherDelayS": 1,
                "devices": [{ "deviceID": me.id }],
            }),
        )
        .await?;

        Ok(PairingCode {
            device_id: me.id,
            device_name: me.name,
            folder_id,
            folder_label: label.to_string(),
            hints: self.lan_hints().await,
        })
    }

    /// The addresses this device is reachable at on the local network.
    ///
    /// Devices normally find each other by broadcast, and these hints are never
    /// needed. They are carried in the pairing code anyway because when
    /// discovery does fail — a network that blocks broadcast, a guest VLAN, a
    /// phone on a different subnet — the alternative is a pairing that silently
    /// never completes and gives the user nothing to act on. They are added
    /// alongside `dynamic`, never instead of it, so a device that later changes
    /// address is still found the usual way.
    async fn lan_hints(&self) -> Vec<String> {
        let Ok(status) = self.get("/rest/system/status").await else {
            return vec![];
        };
        let Some(listeners) = status["connectionServiceStatus"].as_object() else {
            return vec![];
        };
        let mut hints = Vec::new();
        for (name, detail) in listeners {
            if !name.starts_with("tcp://") {
                continue;
            }
            for address in detail["lanAddresses"].as_array().unwrap_or(&vec![]) {
                let Some(address) = address.as_str() else { continue };
                // The wildcard entry is the listener itself, not somewhere a
                // peer could dial.
                if address.contains("0.0.0.0") || address.contains("127.0.0.1") || address.contains("[::]") {
                    continue;
                }
                if !hints.contains(&address.to_string()) {
                    hints.push(address.to_string());
                }
            }
        }
        hints
    }

    /// The code for an already-shared folder, so it can be handed to a second
    /// or third device later.
    pub async fn code_for(&self, folder_id: &str) -> Result<PairingCode> {
        let me = self.this_device().await?;
        let folders: Vec<FolderConfig> = serde_json::from_value(self.get("/rest/config/folders").await?)?;
        let folder = folders
            .into_iter()
            .find(|f| f.id == folder_id)
            .ok_or_else(|| Error::Engine(format!("no folder called {folder_id}")))?;
        Ok(PairingCode {
            device_id: me.id,
            device_name: me.name,
            folder_id: folder.id,
            folder_label: folder.label,
            hints: self.lan_hints().await,
        })
    }

    pub async fn set_folder_paused(&self, folder_id: &str, paused: bool) -> Result<()> {
        self.patch(&format!("/rest/config/folders/{folder_id}"), json!({ "paused": paused })).await?;
        Ok(())
    }

    /// Stops syncing a folder. The files already on disk are left alone —
    /// deleting someone's photos because they tapped "stop sharing" would be
    /// unforgivable, so that is never implied here.
    pub async fn stop_sharing(&self, folder_id: &str) -> Result<()> {
        self.delete(&format!("/rest/config/folders/{folder_id}")).await?;
        Ok(())
    }

    // ---- settings ------------------------------------------------------

    pub async fn settings(&self) -> Result<Settings> {
        let options = self.get("/rest/config/options").await?;
        let defaults = self.get("/rest/config/defaults/folder").await?;
        let me = self.this_device().await?;
        let version = self
            .get("/rest/system/version")
            .await
            .ok()
            .and_then(|v| v["version"].as_str().map(str::to_string))
            .unwrap_or_default();

        Ok(Settings {
            device_name: me.name,
            device_id: me.id,
            // Local discovery is deliberately not part of this: finding devices
            // on the same network is what the app is for, and switching it off
            // would only ever look like a bug.
            local_network_only: !options["globalAnnounceEnabled"].as_bool().unwrap_or(true)
                && !options["relaysEnabled"].as_bool().unwrap_or(true),
            upload_limit_kbps: options["maxSendKbps"].as_u64().unwrap_or(0) as u32,
            download_limit_kbps: options["maxRecvKbps"].as_u64().unwrap_or(0) as u32,
            keep_versions: keep_from_versioning(&defaults["versioning"]),
            engine_version: version,
        })
    }

    pub async fn save_settings(&self, settings: &Settings) -> Result<()> {
        let name = settings.device_name.trim();
        if name.is_empty() {
            return Err(Error::Engine("this device needs a name".into()));
        }
        self.set_this_device_name(name).await?;

        let reachable = !settings.local_network_only;
        self.patch(
            "/rest/config/options",
            json!({
                "globalAnnounceEnabled": reachable,
                "relaysEnabled": reachable,
                "natEnabled": reachable,
                "localAnnounceEnabled": true,
                "maxSendKbps": settings.upload_limit_kbps,
                "maxRecvKbps": settings.download_limit_kbps,
            }),
        )
        .await?;

        self.set_keep_versions(settings.keep_versions).await
    }

    /// Applies the version-keeping preference to folders that already exist as
    /// well as to the template new ones are cut from, so the setting means the
    /// same thing everywhere.
    async fn set_keep_versions(&self, keep: u32) -> Result<()> {
        let versioning = versioning_for(keep);
        self.patch("/rest/config/defaults/folder", json!({ "versioning": versioning }))
            .await?;

        let folders: Vec<FolderConfig> = serde_json::from_value(self.get("/rest/config/folders").await?)?;
        for folder in folders {
            self.patch(
                &format!("/rest/config/folders/{}", folder.id),
                json!({ "versioning": versioning }),
            )
            .await?;
        }
        Ok(())
    }

    // ---- pairing -------------------------------------------------------

    /// Acts on a pasted or scanned code: trusts the other device and takes it
    /// up on the folder it is offering, storing that folder at `local_path`.
    pub async fn redeem(&self, code: &PairingCode, local_path: &str) -> Result<()> {
        if !self.knows_device(&code.device_id).await? {
            let mut device = json!({
                "deviceID": code.device_id,
                "name": code.device_name,
            });
            if !code.hints.is_empty() {
                // `dynamic` stays first: discovery is the route that keeps
                // working after the other device's address changes.
                let mut addresses = vec!["dynamic".to_string()];
                addresses.extend(code.hints.iter().cloned());
                device["addresses"] = json!(addresses);
            }
            self.post("/rest/config/devices", device).await?;
        }

        self.join_folder(&code.folder_id, &code.folder_label, Some(local_path), &code.device_id)
            .await?;

        // The offer, if one was already sitting in the pending list, is now
        // answered; leaving it there would show the user a stale prompt.
        let _ = self
            .delete(&format!(
                "/rest/cluster/pending/folders?folder={}&device={}",
                code.folder_id, code.device_id
            ))
            .await;
        Ok(())
    }

    /// Takes up an offer of a folder.
    ///
    /// The folder may already exist here — that is what happens whenever a third
    /// device joins something two devices already share. In that case the only
    /// change is adding the newcomer to the folder's device list: recreating the
    /// folder would overwrite this device's own path with the one being offered
    /// and drop every other device already sharing it.
    async fn join_folder(
        &self,
        folder_id: &str,
        label: &str,
        local_path: Option<&str>,
        peer: &str,
    ) -> Result<()> {
        let me = self.this_device().await?.id;
        let folders: Vec<FolderConfig> = serde_json::from_value(self.get("/rest/config/folders").await?)?;

        if let Some(existing) = folders.iter().find(|f| f.id == folder_id) {
            let mut devices: Vec<String> =
                existing.devices.iter().map(|d| d.device_id.clone()).collect();
            if !devices.iter().any(|d| d == peer) {
                devices.push(peer.to_string());
            }
            let devices: Vec<Value> = devices.iter().map(|d| json!({ "deviceID": d })).collect();
            self.patch(
                &format!("/rest/config/folders/{folder_id}"),
                json!({ "devices": devices }),
            )
            .await?;
            return Ok(());
        }

        let path = local_path
            .ok_or_else(|| Error::Engine("accepting a folder needs somewhere to put it".into()))?;
        self.post(
            "/rest/config/folders",
            json!({
                "id": folder_id,
                "label": label,
                "path": path,
                "type": "sendreceive",
                "fsWatcherEnabled": true,
                "fsWatcherDelayS": 1,
                "devices": [{ "deviceID": me }, { "deviceID": peer }],
            }),
        )
        .await?;
        Ok(())
    }

    async fn knows_device(&self, device_id: &str) -> Result<bool> {
        let devices: Vec<DeviceConfig> = serde_json::from_value(self.get("/rest/config/devices").await?)?;
        Ok(devices.iter().any(|d| d.device_id == device_id))
    }

    /// Everything waiting for a yes or no: unknown devices that dialled in, and
    /// folders that known devices have offered.
    pub async fn invitations(&self) -> Result<Vec<Invitation>> {
        let pending_devices = self.get("/rest/cluster/pending/devices").await?;
        let pending_folders = self.get("/rest/cluster/pending/folders").await?;
        let known: Vec<DeviceConfig> = serde_json::from_value(self.get("/rest/config/devices").await?)?;
        let known_names: HashMap<&str, &str> =
            known.iter().map(|d| (d.device_id.as_str(), d.name.as_str())).collect();

        let mut out = Vec::new();

        // A folder offer is the more useful prompt, so it wins when a device
        // appears in both lists.
        let mut offered_by: HashMap<String, OfferedFolder> = HashMap::new();
        if let Some(folders) = pending_folders.as_object() {
            for (folder_id, entry) in folders {
                if let Some(devices) = entry["offeredBy"].as_object() {
                    for (device_id, detail) in devices {
                        offered_by.insert(
                            device_id.clone(),
                            OfferedFolder {
                                id: folder_id.clone(),
                                label: detail["label"].as_str().unwrap_or(folder_id).to_string(),
                            },
                        );
                    }
                }
            }
        }

        if let Some(devices) = pending_devices.as_object() {
            for (device_id, detail) in devices {
                out.push(Invitation {
                    from_device_name: detail["name"]
                        .as_str()
                        .filter(|n| !n.is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| short_id(device_id)),
                    folder: offered_by.remove(device_id),
                    from_device_id: device_id.clone(),
                });
            }
        }

        // Folder offers from devices that are already trusted.
        for (device_id, folder) in offered_by {
            out.push(Invitation {
                from_device_name: known_names
                    .get(device_id.as_str())
                    .filter(|n| !n.is_empty())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| short_id(&device_id)),
                from_device_id: device_id,
                folder: Some(folder),
            });
        }

        Ok(out)
    }

    /// Says yes to an invitation. `local_path` is where the folder should live
    /// on this device, and is only needed when a folder was actually offered.
    pub async fn accept(&self, invitation: &Invitation, local_path: Option<&str>) -> Result<()> {
        if !self.knows_device(&invitation.from_device_id).await? {
            self.post(
                "/rest/config/devices",
                json!({
                    "deviceID": invitation.from_device_id,
                    "name": invitation.from_device_name,
                }),
            )
            .await?;
        }
        let _ = self
            .delete(&format!(
                "/rest/cluster/pending/devices?device={}",
                invitation.from_device_id
            ))
            .await;

        if let Some(folder) = &invitation.folder {
            self.join_folder(
                &folder.id,
                &folder.label,
                local_path,
                &invitation.from_device_id,
            )
            .await?;
            let _ = self
                .delete(&format!(
                    "/rest/cluster/pending/folders?folder={}&device={}",
                    folder.id, invitation.from_device_id
                ))
                .await;
        }
        Ok(())
    }

    /// Says no, and makes sure the same prompt does not come back next poll.
    pub async fn decline(&self, invitation: &Invitation) -> Result<()> {
        if let Some(folder) = &invitation.folder {
            let _ = self
                .delete(&format!(
                    "/rest/cluster/pending/folders?folder={}&device={}",
                    folder.id, invitation.from_device_id
                ))
                .await;
        }
        let _ = self
            .delete(&format!(
                "/rest/cluster/pending/devices?device={}",
                invitation.from_device_id
            ))
            .await;
        Ok(())
    }
}

// ---- Syncthing's shapes, kept private ----------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FolderConfig {
    id: String,
    label: String,
    path: String,
    #[serde(default)]
    paused: bool,
    #[serde(default)]
    devices: Vec<FolderDevice>,
}

#[derive(Deserialize)]
struct FolderDevice {
    // Syncthing spells it `deviceID`, which camelCase renaming would turn into
    // `deviceId` and silently fail to match.
    #[serde(rename = "deviceID")]
    device_id: String,
}

#[derive(Deserialize)]
struct DeviceConfig {
    #[serde(rename = "deviceID")]
    device_id: String,
    #[serde(default)]
    name: String,
}

fn folder_state(folder: &FolderConfig, status: &Value, peers: &[Peer]) -> FolderState {
    if folder.paused {
        return FolderState::Paused;
    }
    if let Some(error) = status["error"].as_str().filter(|e| !e.is_empty()) {
        return FolderState::Problem { detail: error.to_string() };
    }
    let pull_errors = status["pullErrors"].as_u64().unwrap_or(0);
    if pull_errors > 0 {
        return FolderState::Problem {
            detail: format!("{pull_errors} files could not be written — check permissions on the folder"),
        };
    }

    let need_bytes = status["needBytes"].as_u64().unwrap_or(0);
    let need_files = status["needFiles"].as_u64().unwrap_or(0);
    if need_files > 0 || need_bytes > 0 {
        let global = status["globalBytes"].as_u64().unwrap_or(0);
        let percent = if global == 0 {
            0
        } else {
            (100u64.saturating_sub(need_bytes.saturating_mul(100) / global.max(1))).min(100) as u8
        };
        return FolderState::Syncing { percent };
    }

    // Up to date only means something if there is someone to be up to date
    // with; otherwise the honest answer is that nobody is reachable.
    if !peers.is_empty() && !peers.iter().any(|p| p.connected) {
        return FolderState::Disconnected;
    }
    FolderState::UpToDate
}

/// Folder IDs are shared between devices and never shown, so they only have to
/// be stable and unlikely to collide with someone else's folder of the same name.
fn new_folder_id(label: &str) -> String {
    use rand::Rng;
    let slug: String = label
        .chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' || c == '-' || c == '_' {
                Some('-')
            } else {
                None
            }
        })
        .take(24)
        .collect();
    let slug = slug.trim_matches('-').to_string();
    let slug = if slug.is_empty() { "carpeta".to_string() } else { slug };
    let suffix: String = (0..6)
        .map(|_| {
            let c = rand::thread_rng().gen_range(0..36);
            char::from_digit(c, 36).unwrap()
        })
        .collect();
    format!("{slug}-{suffix}")
}

/// Syncthing's "simple" versioning keeps N superseded copies in `.stversions`.
/// An empty type means no versioning at all.
fn versioning_for(keep: u32) -> Value {
    if keep == 0 {
        json!({ "type": "", "params": {}, "cleanupIntervalS": 3600, "fsPath": "", "fsType": "basic" })
    } else {
        json!({
            "type": "simple",
            "params": { "keep": keep.to_string() },
            "cleanupIntervalS": 3600,
            "fsPath": "",
            "fsType": "basic"
        })
    }
}

fn keep_from_versioning(versioning: &Value) -> u32 {
    if versioning["type"].as_str().unwrap_or("") != "simple" {
        return 0;
    }
    // Syncthing stores every versioning parameter as a string.
    versioning["params"]["keep"].as_str().and_then(|k| k.parse().ok()).unwrap_or(0)
}

fn random_secret() -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    (0..40).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect()
}

fn short_id(device_id: &str) -> String {
    device_id.split('-').next().unwrap_or(device_id).to_string()
}

/// Counts the copies Syncthing kept when two devices changed the same file.
/// Bounded, because this runs on every poll.
fn count_conflicts(root: &Path) -> u64 {
    fn walk(dir: &Path, seen: &mut usize, found: &mut u64) {
        if *seen >= CONFLICT_SCAN_CAP {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            *seen += 1;
            if *seen >= CONFLICT_SCAN_CAP {
                return;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".stfolder") || name.starts_with(".stversions") {
                continue;
            }
            if name.contains(".sync-conflict-") {
                *found += 1;
                continue;
            }
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk(&entry.path(), seen, found);
            }
        }
    }
    let mut seen = 0;
    let mut found = 0;
    walk(root, &mut seen, &mut found);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_ids_are_slugged_and_unique() {
        let a = new_folder_id("Fotos de Verano 2026!");
        let b = new_folder_id("Fotos de Verano 2026!");
        assert!(a.starts_with("fotos-de-verano-2026-"), "unexpected id: {a}");
        assert_ne!(a, b, "two folders with the same name must not collide");
    }

    #[test]
    fn folder_id_survives_a_label_with_nothing_usable_in_it() {
        assert!(new_folder_id("📁📁📁").starts_with("carpeta-"));
    }

    // Captured verbatim from a running Syncthing v2.1.3. The field is
    // `deviceID`, not `deviceId`: serde's camelCase renaming gets this wrong,
    // and the only symptom is every folder listing failing at runtime.
    const REAL_DEVICES_JSON: &str = r#"[
      {"deviceID":"LJKPHDM-VNQWCDM-KNGS4YA-ABV5JUV-SZOIQQN-NNVHFJT-NL2OHCV-RZUJJQX",
       "name":"Portatil-Lucas","addresses":["dynamic"],"compression":"metadata"}
    ]"#;

    const REAL_FOLDERS_JSON: &str = r#"[
      {"id":"fotos","label":"Fotos","path":"/home/lucas/Fotos","type":"sendreceive","paused":false,
       "devices":[{"deviceID":"LJKPHDM-VNQWCDM-KNGS4YA-ABV5JUV-SZOIQQN-NNVHFJT-NL2OHCV-RZUJJQX",
                   "introducedBy":"","encryptionPassword":""}]}
    ]"#;

    #[test]
    fn parses_what_a_real_engine_actually_returns() {
        let devices: Vec<DeviceConfig> = serde_json::from_str(REAL_DEVICES_JSON).expect("device list must parse");
        assert_eq!(devices[0].name, "Portatil-Lucas");
        assert!(devices[0].device_id.starts_with("LJKPHDM-"));

        let folders: Vec<FolderConfig> = serde_json::from_str(REAL_FOLDERS_JSON).expect("folder list must parse");
        assert_eq!(folders[0].label, "Fotos");
        assert!(folders[0].devices[0].device_id.starts_with("LJKPHDM-"));
    }

    #[test]
    fn versioning_round_trips_through_syncthings_string_params() {
        assert_eq!(keep_from_versioning(&versioning_for(0)), 0);
        assert_eq!(keep_from_versioning(&versioning_for(5)), 5);
        // The parameter really must be a string; a number is silently ignored
        // by the engine.
        assert_eq!(versioning_for(5)["params"]["keep"], json!("5"));
    }

    #[test]
    fn counts_only_real_conflict_copies() {
        let dir = std::env::temp_dir().join(format!("homecloud-test-{}", std::process::id()));
        let nested = dir.join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.join("notas.txt"), "x").unwrap();
        std::fs::write(dir.join("notas.sync-conflict-20260905-212604-LJKPHDM.txt"), "y").unwrap();
        std::fs::write(nested.join("otro.sync-conflict-20260905-212604-Q4XJBIZ.md"), "z").unwrap();
        assert_eq!(count_conflicts(&dir), 2);
        std::fs::remove_dir_all(&dir).ok();
    }
}
