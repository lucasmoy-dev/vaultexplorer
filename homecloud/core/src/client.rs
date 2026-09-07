//! A typed client for the Syncthing REST API, narrowed to what HomeCloud needs.
//!
//! Every method here answers a question the user interface actually asks. The
//! shapes Syncthing returns are deliberately not re-exported: they are an
//! implementation detail that stops at this module's edge.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::model::{
    FolderState, Invitation, OfferedFolder, Peer, Settings, SharedFolder, ThisDevice,
};
use crate::lock::FolderKey;
use crate::pairing::PairingCode;

pub struct Syncthing {
    base: String,
    api_key: String,
    http: reqwest::Client,
    /// Where a folder accepted without asking anyone is put.
    auto_accept_root: Mutex<Option<PathBuf>>,
    /// Where HomeCloud's own preferences live. The engine has nowhere to keep
    /// anything it does not understand, and the interface language is one of
    /// those things.
    preferences_path: Mutex<Option<PathBuf>>,
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
            auto_accept_root: Mutex::new(None),
            preferences_path: Mutex::new(None),
        }
    }

    // ---- letting other devices in --------------------------------------

    /// Says yes to everything waiting, without asking anyone.
    ///
    /// Pairing is something the user started by carrying a code from one device
    /// to the other. Being asked to confirm it again on the far device is a
    /// question they already answered, and it arrives phrased backwards: the
    /// device that handed out the code is asked whether to trust the one that
    /// took it. So both halves are taken automatically — a device that turns up
    /// is trusted, and a folder it offers is joined.
    ///
    /// This is a deliberate trade, and the reason folder passwords exist. A
    /// device ID is not a secret: it travels in the announcements every device
    /// broadcasts on the network. What stops a stranger is that a code they
    /// cannot read is a code they cannot redeem.
    ///
    /// Returns how many were let in, so the interface can say what happened
    /// rather than let things appear by themselves.
    pub async fn admit_everything(&self) -> Result<usize> {
        let mut admitted = 0;

        let pending_devices = self.get("/rest/cluster/pending/devices").await?;
        if let Some(entries) = pending_devices.as_object() {
            for (device_id, detail) in entries {
                let name = detail["name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| short_id(device_id));
                if !self.knows_device(device_id).await? {
                    self.post(
                        "/rest/config/devices",
                        json!({ "deviceID": device_id, "name": name }),
                    )
                    .await?;
                }
                let _ = self
                    .delete(&format!("/rest/cluster/pending/devices?device={device_id}"))
                    .await;
                admitted += 1;
            }
        }

        // A folder already here is the other half of a pairing this device
        // started, and joining only adds the newcomer to it. One that is new
        // needs somewhere to live, so it lands under the same roof as anything
        // else taken from a code.
        let pending_folders = self.get("/rest/cluster/pending/folders").await?;
        if let Some(folders) = pending_folders.as_object() {
            for (folder_id, entry) in folders {
                let Some(offers) = entry["offeredBy"].as_object() else {
                    continue;
                };
                for (device_id, detail) in offers {
                    let label = detail["label"].as_str().unwrap_or(folder_id).to_string();
                    let destination = self.landing_place(&label);
                    if let Err(e) = self
                        .join_folder(folder_id, &label, destination.as_deref(), device_id)
                        .await
                    {
                        // One folder that cannot be taken must not stop the rest.
                        eprintln!("homecloud: could not accept {label}: {e}");
                        continue;
                    }
                    let _ = self
                        .delete(&format!(
                            "/rest/cluster/pending/folders?folder={folder_id}&device={device_id}"
                        ))
                        .await;
                    admitted += 1;
                }
            }
        }

        Ok(admitted)
    }

    /// Where HomeCloud keeps what the engine cannot.
    pub fn set_preferences_path(&self, path: PathBuf) {
        if let Ok(mut slot) = self.preferences_path.lock() {
            *slot = Some(path);
        }
    }

    fn read_preferences(&self) -> Value {
        let Ok(slot) = self.preferences_path.lock() else {
            return json!({});
        };
        slot.as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_else(|| json!({}))
    }

    fn write_preference(&self, key: &str, value: Value) {
        let mut prefs = self.read_preferences();
        prefs[key] = value;
        if let Ok(slot) = self.preferences_path.lock() {
            if let Some(path) = slot.as_ref() {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(path, prefs.to_string());
            }
        }
    }

    /// Tells the client where folders it accepts on its own should go. Set by
    /// the platform, which is the only part that knows where a user's files
    /// live.
    pub fn set_auto_accept_root(&self, root: PathBuf) {
        if let Ok(mut slot) = self.auto_accept_root.lock() {
            *slot = Some(root);
        }
    }

    /// Where an automatically accepted folder goes when this device does not
    /// have it already. `None` leaves the join to fail rather than invent a
    /// path nobody chose.
    fn landing_place(&self, label: &str) -> Option<String> {
        let root = self.auto_accept_root.lock().ok()?.clone()?;
        Some(root.join(sanitised(label)).to_string_lossy().into_owned())
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

    /// Gives this device a name on first run if it does not have a usable one.
    ///
    /// A device with no name shows up on other people's screens as a raw ID, and
    /// on Android the engine falls back to the system hostname, which is
    /// `localhost` on every phone ever made. Two phones then look identical in
    /// the one place it matters — deciding whether to trust one — so anything
    /// that carries no information is replaced.
    pub async fn ensure_device_name(&self, fallback: &str) -> Result<String> {
        let me = self.this_device().await?;
        if is_a_real_name(&me.name) {
            return Ok(me.name);
        }
        let fallback = fallback.trim();
        let chosen = if fallback.is_empty() { "Mi dispositivo" } else { fallback };
        self.set_this_device_name(chosen).await?;
        Ok(chosen.to_string())
    }

    /// Drops devices that no longer share anything with this one.
    ///
    /// Reinstalling an app mints a new identity, so the old one lingers for ever
    /// in a list where it is indistinguishable from the live one. Nothing is
    /// deleted from disk and no folder changes: these are entries for devices
    /// that already sync nothing here.
    ///
    /// Returns the names dropped, so the interface can say what it did.
    pub async fn forget_unused_devices(&self) -> Result<Vec<String>> {
        let me = self.this_device().await?.id;
        let folders: Vec<FolderConfig> =
            serde_json::from_value(self.get("/rest/config/folders").await?)?;
        let devices: Vec<DeviceConfig> =
            serde_json::from_value(self.get("/rest/config/devices").await?)?;

        let in_use: std::collections::HashSet<&str> = folders
            .iter()
            .flat_map(|f| f.devices.iter().map(|d| d.device_id.as_str()))
            .collect();

        let mut dropped = Vec::new();
        for device in &devices {
            if device.device_id == me || in_use.contains(device.device_id.as_str()) {
                continue;
            }
            self.delete(&format!("/rest/config/devices/{}", device.device_id)).await?;
            dropped.push(if device.name.is_empty() {
                short_id(&device.device_id)
            } else {
                format!("{} ({})", device.name, short_id(&device.device_id))
            });
        }
        Ok(dropped)
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
                // Findable away from home. Without these, two devices can only
                // meet by shouting on the same network, so leaving the house
                // meant "disconnected" with nothing to do about it. What is
                // published is this device's id and address, never a folder or
                // a file name, and a relay carries bytes it cannot read.
                "globalAnnounceEnabled": true,
                "relaysEnabled": true,
                "natEnabled": true,
                // Announcing on the local network stays on regardless: at home
                // it is the fast path, and turning it off would only ever look
                // like a bug.
                "localAnnounceEnabled": true,
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
        // One reading for every folder: the engine reports transfer rates for
        // the device as a whole, not per folder.
        let (down, up) = self.transfer_rates().await;
        // Read once for the whole listing: these live in HomeCloud's own
        // preferences, which the engine knows nothing about.
        let wifi_only = self.folder_ids_in("wifiOnly");
        let paused_by_network = self.folder_ids_in("pausedByNetwork");

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

            // Asked for only when something is actually wrong: it is another
            // round trip, and this runs for every folder every second and a half.
            let trouble = if status["pullErrors"].as_u64().unwrap_or(0) > 0 {
                self.first_pull_error(&folder.id).await
            } else {
                None
            };
            let state = folder_state(&folder, &status, &peers, trouble);
            let syncing = matches!(state, FolderState::Syncing { .. });
            out.push(SharedFolder {
                state,
                conflicts: count_conflicts(Path::new(&folder.path)),
                bytes: status["globalBytes"].as_u64().unwrap_or(0),
                files: status["globalFiles"].as_u64().unwrap_or(0),
                // Only while something is actually moving: a rate left on
                // screen next to a finished folder reads as a rate for it.
                bytes_per_second: if syncing { down.max(up) } else { 0 },
                read_only: folder.folder_type == "receiveonly",
                free_bytes: crate::disk::free_bytes(Path::new(&folder.path)),
                pending_bytes: status["needBytes"].as_u64().unwrap_or(0),
                wifi_only: wifi_only.contains(&folder.id),
                paused_by_network: paused_by_network.contains(&folder.id),
                has_password: self.folder_has_password(&folder.id),
                peers,
                id: folder.id,
                label: folder.label,
                path: folder.path,
            });
        }
        Ok(out)
    }

    /// How much a folder holds, as every device agrees it should. Zero when the
    /// engine cannot say, which reads as "size unknown" rather than "empty".
    async fn folder_bytes(&self, folder_id: &str) -> u64 {
        self.get(&format!("/rest/db/status?folder={folder_id}"))
            .await
            .ok()
            .and_then(|status| status["globalBytes"].as_u64())
            .unwrap_or(0)
    }

    /// What the engine actually said about the first file it could not write.
    ///
    /// The count of failures alone says nothing a person can act on, and the
    /// message that used to be shown guessed: it blamed permissions for every
    /// one of them, including a full disk, which is the commonest cause by far
    /// and the only one where the guess sends you looking in the wrong place.
    async fn first_pull_error(&self, folder_id: &str) -> Option<String> {
        let errors = self
            .get(&format!("/rest/folder/errors?folder={folder_id}"))
            .await
            .ok()?;
        errors["errors"][0]["error"].as_str().map(str::to_string)
    }

    /// Bytes per second in and out, as the engine has measured them.
    ///
    /// Best effort: a rate that cannot be read is reported as zero rather than
    /// failing the whole folder listing, which happens every second and a half.
    async fn transfer_rates(&self) -> (u64, u64) {
        let Ok(status) = self.get("/rest/system/connections").await else {
            return (0, 0);
        };
        let total = &status["total"];
        (
            total["inBytesPerSecond"].as_f64().unwrap_or(0.0).max(0.0) as u64,
            total["outBytesPerSecond"].as_f64().unwrap_or(0.0).max(0.0) as u64,
        )
    }

    // ---- metered connections -------------------------------------------

    /// The folder ids stored under one preference key.
    fn folder_ids_in(&self, key: &str) -> Vec<String> {
        self.read_preferences()[key]
            .as_array()
            .map(|ids| ids.iter().filter_map(|id| id.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }

    /// Marks a folder as one to hold off on while the connection is metered.
    pub async fn set_folder_wifi_only(&self, folder_id: &str, wifi_only: bool) -> Result<()> {
        let mut ids = self.folder_ids_in("wifiOnly");
        ids.retain(|id| id != folder_id);
        if wifi_only {
            ids.push(folder_id.to_string());
        }
        self.write_preference("wifiOnly", json!(ids));
        Ok(())
    }

    /// Pauses or resumes the wifi-only folders as the connection changes.
    ///
    /// Only ever touches folders it paused itself: a folder the user stopped on
    /// purpose must not come back to life because the phone found a network,
    /// which is why the two reasons for being paused are recorded separately.
    ///
    /// Returns how many folders changed, so a caller can log a no-op as a no-op.
    pub async fn apply_metered_policy(&self, metered: bool) -> Result<usize> {
        let wifi_only = self.folder_ids_in("wifiOnly");
        let mut paused_by_us = self.folder_ids_in("pausedByNetwork");
        let mut changed = 0;

        if metered {
            for folder_id in &wifi_only {
                if paused_by_us.contains(folder_id) {
                    continue;
                }
                self.set_folder_paused(folder_id, true).await?;
                paused_by_us.push(folder_id.clone());
                changed += 1;
            }
        } else {
            for folder_id in paused_by_us.clone() {
                self.set_folder_paused(&folder_id, false).await?;
                paused_by_us.retain(|id| *id != folder_id);
                changed += 1;
            }
        }

        self.write_preference("pausedByNetwork", json!(paused_by_us));
        Ok(changed)
    }

    /// Nudges every device into reconnecting.
    ///
    /// Moving between networks leaves sockets that look alive on one side and
    /// are dead on the other, which is exactly the asymmetry behind a desktop
    /// showing "connected" while the phone shows "disconnected". Pausing and
    /// resuming a device tears the connection down and dials again.
    pub async fn reconnect_all(&self) -> Result<()> {
        let devices: Vec<DeviceConfig> =
            serde_json::from_value(self.get("/rest/config/devices").await?)?;
        let me = self.this_device().await?.id;
        for device in devices.iter().filter(|d| d.device_id != me) {
            let path = format!("/rest/system/pause?device={}", device.device_id);
            let _ = self.post(&path, json!({})).await;
        }
        for device in devices.iter().filter(|d| d.device_id != me) {
            let path = format!("/rest/system/resume?device={}", device.device_id);
            let _ = self.post(&path, json!({})).await;
        }
        Ok(())
    }

    // ---- folder passwords ----------------------------------------------

    /// Puts a password on a folder, or replaces the one it had.
    ///
    /// What gets stored is the salt and the derived key — never the password.
    /// That is enough to lock future codes for this folder and enough to
    /// recognise the right password later, and not enough to recover it.
    pub async fn set_folder_password(&self, folder_id: &str, password: &str) -> Result<()> {
        if password.trim().is_empty() {
            return Err(Error::Engine("la contraseña no puede estar vacía".into()));
        }
        let lock = FolderKey::create(password)?;
        self.store_key(folder_id, Some(&lock));
        Ok(())
    }

    /// Takes the password off a folder, which anyone sharing it may do: from
    /// here on its codes are readable without one. It does not reach the other
    /// devices — each keeps its own decision about the codes *it* writes.
    pub async fn clear_folder_password(&self, folder_id: &str) -> Result<()> {
        self.store_key(folder_id, None);
        Ok(())
    }

    /// Whether a password has to be typed to use this folder's codes.
    pub fn folder_has_password(&self, folder_id: &str) -> bool {
        self.stored_key(folder_id).is_some()
    }

    /// Checks a password against the one a folder has.
    pub fn folder_password_matches(&self, folder_id: &str, password: &str) -> bool {
        let Some(stored) = self.stored_key(folder_id) else {
            return false;
        };
        FolderKey::from_password(password, stored.salt)
            .map(|candidate| candidate.matches(&stored))
            .unwrap_or(false)
    }

    fn store_key(&self, folder_id: &str, lock: Option<&FolderKey>) {
        let mut locks = self.read_preferences()["folderKeys"].clone();
        if !locks.is_object() {
            locks = json!({});
        }
        match lock {
            Some(lock) => {
                locks[folder_id] = json!({
                    "salt": URL_SAFE_NO_PAD.encode(lock.salt),
                    "key": URL_SAFE_NO_PAD.encode(lock.key),
                });
            }
            None => {
                if let Some(map) = locks.as_object_mut() {
                    map.remove(folder_id);
                }
            }
        }
        self.write_preference("folderKeys", locks);
    }

    fn stored_key(&self, folder_id: &str) -> Option<FolderKey> {
        let stored = self.read_preferences();
        let entry = stored["folderKeys"].get(folder_id)?;
        let salt = URL_SAFE_NO_PAD.decode(entry["salt"].as_str()?).ok()?;
        let key = URL_SAFE_NO_PAD.decode(entry["key"].as_str()?).ok()?;
        Some(FolderKey {
            salt: salt.try_into().ok()?,
            key: key.try_into().ok()?,
        })
    }

    /// Asks the engine to look at the folder again from scratch.
    ///
    /// The way out of "no connected device has the required version": the index
    /// and the disk have drifted apart, and only a fresh look reconciles them.
    pub async fn rescan(&self, folder_id: &str) -> Result<()> {
        self.post(&format!("/rest/db/scan?folder={folder_id}"), json!({})).await?;
        Ok(())
    }

    /// Turns a folder into one that receives changes but never sends its own,
    /// or back again. Everything is two-way unless someone says otherwise.
    pub async fn set_folder_read_only(&self, folder_id: &str, read_only: bool) -> Result<()> {
        let folder_type = if read_only { "receiveonly" } else { "sendreceive" };
        self.patch(
            &format!("/rest/config/folders/{folder_id}"),
            json!({ "type": folder_type }),
        )
        .await?;
        Ok(())
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
        let folder_id_for_size = folder_id.clone();

        Ok(PairingCode {
            device_id: me.id,
            device_name: me.name,
            folder_id,
            folder_label: label.to_string(),
            hints: self.lan_hints().await,
            bytes: Some(self.folder_bytes(&folder_id_for_size).await),
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
    /// The code a person copies: locked when the folder has a password.
    pub async fn code_text_for(&self, folder_id: &str) -> Result<String> {
        let code = self.code_for(folder_id).await?;
        match self.stored_key(folder_id) {
            Some(lock) => code.encode_locked(&lock),
            None => code.encode(),
        }
    }

    pub async fn code_for(&self, folder_id: &str) -> Result<PairingCode> {
        let me = self.this_device().await?;
        let folders: Vec<FolderConfig> = serde_json::from_value(self.get("/rest/config/folders").await?)?;
        let folder = folders
            .into_iter()
            .find(|f| f.id == folder_id)
            .ok_or_else(|| Error::Engine(format!("no folder called {folder_id}")))?;

        let bytes = self.folder_bytes(&folder.id).await;
        Ok(PairingCode {
            device_id: me.id,
            device_name: me.name,
            folder_id: folder.id,
            folder_label: folder.label,
            hints: self.lan_hints().await,
            bytes: Some(bytes),
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
            language: self
                .read_preferences()["language"]
                .as_str()
                .filter(|l| *l == "es" || *l == "en")
                .unwrap_or("es")
                .to_string(),
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

        if settings.language == "es" || settings.language == "en" {
            self.write_preference("language", json!(settings.language));
        }

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

    /// Reads a code, using `password` when the folder has one.
    ///
    /// The password is checked here, before anything reaches the network: a
    /// wrong one cannot even produce a device to connect to. When the code
    /// opens, its password is kept for this folder so codes written *from* this
    /// device carry it too — which is how a password follows a folder from the
    /// second device to the third.
    pub async fn read_code(&self, text: &str, password: Option<&str>) -> Result<PairingCode> {
        use crate::pairing::ScannedCode;

        match PairingCode::scan(text)? {
            ScannedCode::Open(code) => Ok(code),
            ScannedCode::Locked { salt, sealed } => {
                let password = password.filter(|p| !p.is_empty()).ok_or_else(|| {
                    Error::BadPairingCode("esta carpeta tiene contraseña".into())
                })?;
                let lock = FolderKey::from_password(password, salt)?;
                let code = PairingCode::unlock(&sealed, &lock)?;
                self.store_key(&code.folder_id, Some(&lock));
                Ok(code)
            }
        }
    }

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
        // The device this app just wrote a code for is not a stranger, so it is
        // let in here rather than surfacing as a prompt the user already answered
        // by handing the code over in the first place.
        let _ = self.admit_everything().await;

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
    /// Syncthing's own word for the direction: `sendreceive` both ways,
    /// `receiveonly` for a folder this device never sends changes from.
    #[serde(rename = "type", default)]
    folder_type: String,
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

fn folder_state(
    folder: &FolderConfig,
    status: &Value,
    peers: &[Peer],
    trouble: Option<String>,
) -> FolderState {
    if folder.paused {
        return FolderState::Paused;
    }
    if let Some(error) = status["error"].as_str().filter(|e| !e.is_empty()) {
        return FolderState::Problem { detail: error.to_string() };
    }
    let pull_errors = status["pullErrors"].as_u64().unwrap_or(0);
    if pull_errors > 0 {
        return FolderState::Problem {
            detail: explain_pull_error(pull_errors, trouble.as_deref()),
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

/// Turns the engine's own words about a failed write into a sentence that says
/// what to do about it.
///
/// Only the causes that lead somewhere different are singled out; everything
/// else keeps the engine's message, which is more use than a guess.
fn explain_pull_error(count: u64, engine_said: Option<&str>) -> String {
    let files = if count == 1 { "1 fichero".to_string() } else { format!("{count} ficheros") };
    let Some(said) = engine_said else {
        return format!("{files} no se pudieron guardar.");
    };
    let lower = said.to_lowercase();

    if lower.contains("insufficient space") || lower.contains("no space left") {
        return format!("{files} no caben: no queda espacio en el disco. {}", sizes_from(said));
    }
    if lower.contains("permission denied") {
        return format!("{files} no se pudieron guardar: la carpeta no da permiso de escritura.");
    }
    if lower.contains("read-only file system") {
        return format!("{files} no se pudieron guardar: el disco está montado como solo lectura.");
    }
    if lower.contains("no connected device has the required version") {
        return format!(
            "{files} ya no están en el otro dispositivo, o no está conectado. \
             Se arregla cuando vuelva a revisar su carpeta; si los borró, aquí también desaparecerán."
        );
    }
    if lower.contains("file name too long") || lower.contains("invalid") {
        return format!("{files} no se pudieron guardar: el nombre no vale en este sistema.");
    }
    format!("{files} no se pudieron guardar: {said}")
}

/// Pulls the "current X < required Y" tail out of the engine's message, which
/// is the part that tells you how much room you actually need.
fn sizes_from(said: &str) -> String {
    match said.split_once("current ") {
        Some((_, tail)) => format!("Hace falta {}.", tail.replace(" < required ", " libres, y se necesitan ")),
        None => String::new(),
    }
}

/// A label arriving from another device must never become a path separator.
fn sanitised(label: &str) -> String {
    let cleaned: String = label
        .trim()
        .chars()
        .map(|c| if std::path::is_separator(c) || c == '\0' { '-' } else { c })
        .collect();
    let cleaned = cleaned.trim_matches('.').trim().to_string();
    if cleaned.is_empty() { "Carpeta".to_string() } else { cleaned }
}

fn short_id(device_id: &str) -> String {
    device_id.split('-').next().unwrap_or(device_id).to_string()
}

/// Whether a device name tells a person anything.
///
/// The engine's own fallbacks do not: on a phone it reports the system
/// hostname, and every Android device calls itself `localhost`.
fn is_a_real_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && !name.eq_ignore_ascii_case("localhost")
        && !name.eq_ignore_ascii_case("android")
        && !name.eq_ignore_ascii_case("unknown")
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
    use super::explain_pull_error;

    /// The message this replaced blamed permissions for everything, including
    /// this — the commonest cause, and the one where being sent to look at
    /// permissions wastes the most time.
    #[test]
    fn a_full_disk_is_named_as_a_full_disk() {
        let said = "syncing: insufficient space in folder \"DCIM\" (dcim-17f2tg) \
                    (/home/lucas/dcim): current 5.3 GB < required 11.5 GB";
        let shown = explain_pull_error(2, Some(said));
        assert!(shown.contains("no queda espacio"), "{shown}");
        assert!(shown.contains("5.3 GB"), "the amounts are what make it actionable: {shown}");
        assert!(shown.contains("11.5 GB"), "{shown}");
        assert!(!shown.to_lowercase().contains("permiso"), "must not blame permissions: {shown}");
    }

    #[test]
    fn permissions_are_named_only_when_that_is_what_happened() {
        let shown = explain_pull_error(1, Some("open /x/y: permission denied"));
        assert!(shown.contains("permiso"), "{shown}");
        assert!(shown.starts_with("1 fichero "), "singular reads as singular: {shown}");
    }

    /// An unknown cause keeps the engine's words, which beat a guess.
    /// The second wrong guess this function existed to stop: a stale index is
    /// not a full disk and not a permission problem.
    #[test]
    fn a_file_the_other_device_no_longer_has_says_so() {
        let shown = explain_pull_error(
            2,
            Some("syncing: no connected device has the required version of this file"),
        );
        assert!(shown.contains("otro dispositivo"), "{shown}");
        assert!(!shown.to_lowercase().contains("espacio"), "{shown}");
        assert!(!shown.to_lowercase().contains("permiso"), "{shown}");
    }

    #[test]
    fn anything_else_keeps_what_the_engine_said() {
        let shown = explain_pull_error(3, Some("something nobody has seen before"));
        assert!(shown.contains("something nobody has seen before"), "{shown}");
    }

    #[test]
    fn no_detail_still_says_how_many() {
        let shown = explain_pull_error(4, None);
        assert!(shown.contains("4 ficheros"), "{shown}");
    }

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
