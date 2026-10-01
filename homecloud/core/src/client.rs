//! A typed client for the Syncthing REST API, narrowed to what HomeCloud needs.
//!
//! Every method here answers a question the user interface actually asks. The
//! shapes Syncthing returns are deliberately not re-exported: they are an
//! implementation detail that stops at this module's edge.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::model::{
    DeletionPolicy, FolderMode, FolderState, Invitation, OfferedFolder, Peer, Route, Settings,
    SharedFolder, ThisDevice,
};
use crate::trash::{self, DeletedFile};
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
    /// How far along each folder's download was last time it was asked, so a
    /// speed and a time left can be worked out from the difference. The engine
    /// stopped publishing transfer rates in 2.x, so this is now the only
    /// source of "how fast is this going".
    progress: Mutex<HashMap<String, Progress>>,
    /// When each device was last seen connected. Read through a grace period,
    /// because a link that blinks is not a link that is down.
    last_seen: Mutex<HashMap<String, Instant>>,
    /// How much of a folder a peer has, and when that was asked. Kept because
    /// it is one request per peer per folder and the poll runs every second
    /// and a half.
    completion: Mutex<HashMap<(String, String), (Instant, u8)>>,
    /// Sizes a folder reported while the engine was still willing to say. A
    /// paused folder reports zeros for everything, and "0 B" is a lie a person
    /// reads as "my files are gone".
    sizes: Mutex<Option<HashMap<String, RememberedSize>>>,
    /// When those sizes last reached the disk, so remembering them does not
    /// mean writing a file every poll.
    sizes_saved: Mutex<Option<Instant>>,
    /// Where the DeviceConnected event stream has been read up to.
    names_cursor: Mutex<i64>,
    /// The command that puts a file in this platform's recycle bin. Set by the
    /// desktop, which is the only place there is one; a phone leaves it unset
    /// and gets the hidden-copies policy instead.
    trash_command: Mutex<Option<String>>,
    /// Conflict counts per folder, worked out off the poll. Walking a folder
    /// is a full directory scan — on a phone, tens of thousands of entries
    /// over Android's slow shared-storage layer — and it used to run inline
    /// on every poll, which is what kept the first screen on "Arrancando…"
    /// for seconds after the engine had already answered.
    conflicts: Arc<Mutex<HashMap<String, ConflictCount>>>,
    /// When the LAN addresses of connected peers were last written down.
    routes_pinned_at: Mutex<Option<Instant>>,
}

/// One folder's conflict count, and whether a fresh one is being worked out.
#[derive(Clone, Copy, Default)]
struct ConflictCount {
    at: Option<Instant>,
    found: u64,
    counting: bool,
}

/// How long a conflict count is trusted before it is redone in the background.
const CONFLICT_RECOUNT_EVERY: Duration = Duration::from_secs(30);

/// How often the LAN addresses of connected peers are re-checked.
const PIN_ROUTES_EVERY: Duration = Duration::from_secs(30);

/// LAN addresses kept per device besides `dynamic`.
const MAX_PINNED: usize = 3;

/// Syncthing's API server closes a keep-alive connection after 15 s idle
/// (`ReadTimeout` in lib/api). Reusing one at that moment fails the request
/// with "error sending request" even though the engine is perfectly fine —
/// reproduced against a real engine at a 14.995 s gap. Pooled connections are
/// therefore dropped well before the server would drop them.
const POOL_IDLE: Duration = Duration::from_secs(5);

/// One reading of a folder's download, and the speed derived from it.
struct Progress {
    at: Instant,
    need: u64,
    /// Smoothed bytes per second. Raw samples jump around enough that a time
    /// left computed from one would be unreadable.
    rate: f64,
}

/// What a folder held the last time the engine would say.
#[derive(Clone, Copy)]
struct RememberedSize {
    bytes: u64,
    files: u64,
    need: u64,
}

/// A device that was connected this recently still counts as connected.
///
/// Syncthing 2 keeps several connections to the same device and swaps between
/// them, so a heavy transfer reports "not connected" for a moment several
/// times a minute. Nothing is actually interrupted, but the folder went dark
/// and said "Sin conexión" every time.
const CONNECTION_GRACE: Duration = Duration::from_secs(25);

/// How long a peer's completion figure is reused before asking again.
const COMPLETION_TTL: Duration = Duration::from_secs(4);

/// Weight of the newest speed sample. Lower is steadier and slower to react;
/// at one sample every 1.5 s this settles in about five seconds.
const RATE_SMOOTHING: f64 = 0.3;

/// How often remembered sizes are written to disk at most.
const SIZE_PERSIST_EVERY: Duration = Duration::from_secs(60);

/// Copies kept in the hidden folder when nobody has said otherwise. Enough to
/// undo a mistake, few enough not to fill a phone.
const DEFAULT_KEPT_COPIES: u32 = 5;

/// How many direct addresses a pairing code carries at most.
const MAX_HINTS: usize = 2;

/// A conflict scan gives up past this many entries. A folder large enough to
/// hit the cap is one where an exact count is not worth the disk churn on
/// every poll; the badge just says "some".
const CONFLICT_SCAN_CAP: usize = 50_000;

/// Pauses between attempts at a request that failed in transport.
const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(100), Duration::from_millis(400)];

impl Syncthing {
    pub fn new(base: impl Into<String>, api_key: impl Into<String>) -> Self {
        Syncthing {
            base: base.into(),
            api_key: api_key.into(),
            http: reqwest::Client::builder()
                .pool_idle_timeout(POOL_IDLE)
                // It is on this machine: anything slower than this to connect
                // is an engine that is not there, not one that is busy.
                .connect_timeout(Duration::from_secs(3))
                // A wedged engine must surface as an error, not as a poll that
                // never returns and freezes whatever screen awaits it.
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            auto_accept_root: Mutex::new(None),
            preferences_path: Mutex::new(None),
            progress: Mutex::new(HashMap::new()),
            last_seen: Mutex::new(HashMap::new()),
            completion: Mutex::new(HashMap::new()),
            sizes: Mutex::new(None),
            sizes_saved: Mutex::new(None),
            names_cursor: Mutex::new(0),
            trash_command: Mutex::new(None),
            conflicts: Arc::new(Mutex::new(HashMap::new())),
            routes_pinned_at: Mutex::new(None),
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
        // Transport failures to an engine on this same machine are transient
        // by nature — a keep-alive connection closed under us, the engine
        // still binding its port — so they get a short second and third try
        // before anyone is told. Only where repeating cannot do something
        // twice: reads and PATCHes always, anything else only when the
        // request never reached the engine at all.
        let idempotent = method == reqwest::Method::GET || method == reqwest::Method::PATCH;
        let mut attempt = 0;
        loop {
            match self.request_once(method.clone(), path, body.as_ref()).await {
                Err(Error::Http(e)) if attempt < RETRY_DELAYS.len() && (idempotent || e.is_connect()) => {
                    tokio::time::sleep(RETRY_DELAYS[attempt]).await;
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    async fn request_once(&self, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<Value> {
        let url = format!("{}{}", self.base, path);
        let mut req = self.http.request(method, &url).header("X-API-Key", &self.api_key);
        if let Some(body) = body {
            req = req.json(body);
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
    ///
    /// One attempt, no retries: the caller is the one polling, and a retry
    /// ladder here would only add latency to noticing the engine is up.
    pub async fn ping(&self) -> Result<()> {
        self.request_once(reqwest::Method::GET, "/rest/system/ping", None)
            .await
            .map(|_| ())
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

    /// Renames this device, and makes the other devices hear about it.
    ///
    /// The name is not part of how devices find or trust each other — that is
    /// the device ID, which never changes — so renaming must not disturb
    /// anything. It does not: the engine keeps its connections and its folders
    /// exactly as they were.
    ///
    /// The catch is that the name only travels in the handshake, so peers keep
    /// showing the old one until they next reconnect, which could be days. A
    /// single re-handshake per peer fixes that in a few seconds; transfers pick
    /// up where they left off, because Syncthing resumes by block. It is done
    /// only when the name really changed, so nothing happens when the settings
    /// screen is saved with the same name in it.
    pub async fn set_this_device_name(&self, name: &str) -> Result<()> {
        let me = self.this_device().await?;
        if me.name == name {
            return Ok(());
        }
        self.patch(&format!("/rest/config/devices/{}", me.id), json!({ "name": name }))
            .await?;
        self.reannounce_to_peers(&me.id).await;
        Ok(())
    }

    /// Drops and re-dials each peer so it hears this device's name again.
    /// Best effort throughout: a peer that will not come back was already
    /// unreachable, and the rename itself has been saved either way.
    async fn reannounce_to_peers(&self, me: &str) {
        let Ok(devices) = self.get("/rest/config/devices").await else { return };
        let Ok(devices) = serde_json::from_value::<Vec<DeviceConfig>>(devices) else { return };
        let peers: Vec<String> = devices
            .into_iter()
            .map(|d| d.device_id)
            .filter(|id| id != me)
            .collect();

        for id in &peers {
            let _ = self.post(&format!("/rest/system/pause?device={id}"), json!({})).await;
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
        for id in &peers {
            let _ = self.post(&format!("/rest/system/resume?device={id}"), json!({})).await;
        }
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
        // Cheap, and this is the one call every screen makes on a loop: it is
        // where a peer that renamed itself gets noticed.
        let _ = self.adopt_announced_names().await;

        let configured: Vec<FolderConfig> = serde_json::from_value(self.get("/rest/config/folders").await?)?;
        let devices: Vec<DeviceConfig> = serde_json::from_value(self.get("/rest/config/devices").await?)?;
        let names: HashMap<&str, &str> =
            devices.iter().map(|d| (d.device_id.as_str(), d.name.as_str())).collect();
        let (connected, routes) = self.connected_devices().await?;
        let me = self.this_device().await?.id;
        // Read once for the whole listing: these live in HomeCloud's own
        // preferences, which the engine knows nothing about.
        let wifi_only = self.folder_ids_in("wifiOnly");
        let paused_by_network = self.folder_ids_in("pausedByNetwork");

        let mut out = Vec::with_capacity(configured.len());
        for folder in configured {
            // The folder can vanish between the config listing above and this
            // per-folder lookup (a concurrent "stop sharing" racing this poll):
            // drop it from the result instead of failing the whole listing.
            let status = match self.get(&format!("/rest/db/status?folder={}", folder.id)).await {
                Ok(v) => v,
                Err(Error::Api { status: 404, .. }) => continue,
                Err(e) => return Err(e),
            };

            let mut peers: Vec<Peer> = folder
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
                    completion: None,
                    route: routes.get(&d.device_id).copied(),
                })
                .collect();
            // What the other end still has to fetch. Asked for from here
            // because the folder's own status only ever describes this copy:
            // the device that handed a folder out reads "Al día" while the
            // phone it gave it to is still at four per cent.
            for peer in peers.iter_mut() {
                peer.completion = self.peer_completion(&folder.id, &peer.id).await;
            }

            // Asked for only when something is actually wrong: it is another
            // round trip, and this runs for every folder every second and a half.
            let trouble = if status["pullErrors"].as_u64().unwrap_or(0) > 0 {
                self.first_pull_error(&folder.id).await
            } else {
                None
            };
            let state = folder_state(&folder, &status, &peers, trouble);
            let syncing = matches!(state, FolderState::Syncing { .. });

            // A paused folder answers zero to everything it is asked, so what
            // it held a moment ago is what gets shown instead.
            let engine = RememberedSize {
                bytes: status["globalBytes"].as_u64().unwrap_or(0),
                files: status["globalFiles"].as_u64().unwrap_or(0),
                need: status["needBytes"].as_u64().unwrap_or(0),
            };
            let size = self.size_of(&folder.id, engine, folder.paused);

            let rate = if syncing { self.folder_rate(&folder.id, size.need) } else {
                self.forget_rate(&folder.id);
                0
            };
            out.push(SharedFolder {
                state,
                conflicts: self.conflicts_in(&folder.id, &folder.path),
                bytes: size.bytes,
                files: size.files,
                // Only while something is actually moving: a rate left on
                // screen next to a finished folder reads as a rate for it.
                bytes_per_second: rate,
                eta_seconds: eta(size.need, rate),
                mode: mode_of(&folder),
                // What this copy has that the others no longer do. On the
                // device keeping the deleted videos this is the whole point;
                // anywhere else it is zero.
                extra_bytes: status["localBytes"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_sub(status["globalBytes"].as_u64().unwrap_or(0)),
                free_bytes: crate::disk::free_bytes(Path::new(&folder.path)),
                pending_bytes: size.need,
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

    /// The last conflict count for a folder, recounted in the background when
    /// it is stale. Never blocks the poll on a directory walk; a folder seen
    /// for the first time reads 0 until its first count lands a moment later.
    fn conflicts_in(&self, folder_id: &str, path: &str) -> u64 {
        let Ok(mut counts) = self.conflicts.lock() else { return 0 };
        let entry = counts.entry(folder_id.to_string()).or_default();
        let stale = entry.at.is_none_or(|at| at.elapsed() >= CONFLICT_RECOUNT_EVERY);
        if stale && !entry.counting {
            entry.counting = true;
            let counts = Arc::clone(&self.conflicts);
            let (id, root) = (folder_id.to_string(), PathBuf::from(path));
            let job = move || {
                let found = count_conflicts(&root);
                if let Ok(mut counts) = counts.lock() {
                    counts.insert(id, ConflictCount { at: Some(Instant::now()), found, counting: false });
                }
            };
            // Off the async threads: this is blocking file-system work.
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => drop(handle.spawn_blocking(job)),
                Err(_) => drop(std::thread::spawn(job)),
            }
        }
        entry.found
    }

    /// How much of `folder_id` the device `peer` already has, 0-100.
    ///
    /// Cached for a few seconds: this is one request per peer per folder, on a
    /// loop that runs every second and a half.
    async fn peer_completion(&self, folder_id: &str, peer: &str) -> Option<u8> {
        let key = (folder_id.to_string(), peer.to_string());
        if let Ok(cache) = self.completion.lock() {
            if let Some((at, value)) = cache.get(&key) {
                if at.elapsed() < COMPLETION_TTL {
                    return Some(*value);
                }
            }
        }
        let answer = self
            .get(&format!("/rest/db/completion?folder={folder_id}&device={peer}"))
            .await
            .ok()?;
        // A device that has every byte but has not applied a deletion is not
        // behind: it is the copy deliberately keeping what the others threw
        // away, and reporting it at 95% for ever reads as a sync that never
        // finishes.
        let has_everything = answer["needBytes"].as_u64().unwrap_or(0) == 0
            && answer["needItems"].as_u64().unwrap_or(0) == 0;
        let percent = if has_everything {
            100
        } else {
            answer["completion"].as_f64()?.clamp(0.0, 100.0).round() as u8
        };
        if let Ok(mut cache) = self.completion.lock() {
            cache.insert(key, (Instant::now(), percent));
        }
        Some(percent)
    }

    /// Bytes per second for one folder, from how much less it needs than it
    /// did a moment ago.
    ///
    /// Syncthing 2 dropped the per-device transfer rates the app used to read,
    /// which is why no speed was ever shown. `needBytes` falls block by block
    /// rather than file by file, so the difference between two polls is a real
    /// measurement even inside one large file.
    fn folder_rate(&self, folder_id: &str, need: u64) -> u64 {
        let now = Instant::now();
        let Ok(mut seen) = self.progress.lock() else { return 0 };
        let entry = seen
            .entry(folder_id.to_string())
            .or_insert(Progress { at: now, need, rate: 0.0 });
        let elapsed = now.duration_since(entry.at).as_secs_f64();
        // Two readings taken together measure nothing but the clock.
        if elapsed < 0.4 {
            return entry.rate.max(0.0) as u64;
        }
        // Needing *more* than before means new files turned up, not a negative
        // speed.
        let moved = entry.need.saturating_sub(need) as f64;
        let sample = moved / elapsed;
        entry.rate = if entry.rate <= 0.0 {
            sample
        } else {
            entry.rate * (1.0 - RATE_SMOOTHING) + sample * RATE_SMOOTHING
        };
        entry.at = now;
        entry.need = need;
        entry.rate.max(0.0) as u64
    }

    /// Drops a folder's speed history once it stops syncing, so resuming later
    /// does not measure against a reading from an hour ago.
    fn forget_rate(&self, folder_id: &str) {
        if let Ok(mut seen) = self.progress.lock() {
            seen.remove(folder_id);
        }
    }

    /// The sizes to show for a folder: what the engine just said, or what it
    /// said last time if it has stopped answering because the folder is paused.
    fn size_of(&self, folder_id: &str, engine: RememberedSize, paused: bool) -> RememberedSize {
        let blank = engine.bytes == 0 && engine.files == 0;
        if paused && blank {
            if let Some(remembered) = self.remembered_size(folder_id) {
                return remembered;
            }
        }
        if !blank {
            self.remember_size(folder_id, engine);
        }
        engine
    }

    fn remembered_size(&self, folder_id: &str) -> Option<RememberedSize> {
        self.load_sizes();
        let sizes = self.sizes.lock().ok()?;
        sizes.as_ref()?.get(folder_id).copied()
    }

    /// Keeps the last real reading, on disk as well as in memory: a folder
    /// paused for the night is one the app will be restarted in front of.
    fn remember_size(&self, folder_id: &str, size: RememberedSize) {
        self.load_sizes();
        let changed = {
            let Ok(mut sizes) = self.sizes.lock() else { return };
            let map = sizes.get_or_insert_with(HashMap::new);
            let before = map.get(folder_id).copied();
            map.insert(folder_id.to_string(), size);
            !matches!(before, Some(old) if old.bytes == size.bytes && old.files == size.files && old.need == size.need)
        };
        if !changed {
            return;
        }
        // Writing the file on every poll would be a disk write every second
        // and a half for a number nobody reads until a restart.
        {
            let Ok(mut saved) = self.sizes_saved.lock() else { return };
            if let Some(at) = *saved {
                if at.elapsed() < SIZE_PERSIST_EVERY {
                    return;
                }
            }
            *saved = Some(Instant::now());
        }
        self.persist_sizes();
    }

    fn load_sizes(&self) {
        let Ok(mut sizes) = self.sizes.lock() else { return };
        if sizes.is_some() {
            return;
        }
        let mut loaded = HashMap::new();
        if let Some(stored) = self.read_preferences()["folderSizes"].as_object() {
            for (id, value) in stored {
                loaded.insert(
                    id.clone(),
                    RememberedSize {
                        bytes: value["bytes"].as_u64().unwrap_or(0),
                        files: value["files"].as_u64().unwrap_or(0),
                        need: value["need"].as_u64().unwrap_or(0),
                    },
                );
            }
        }
        *sizes = Some(loaded);
    }

    fn persist_sizes(&self) {
        let Ok(sizes) = self.sizes.lock() else { return };
        let Some(map) = sizes.as_ref() else { return };
        let stored: serde_json::Map<String, Value> = map
            .iter()
            .map(|(id, size)| {
                (
                    id.clone(),
                    json!({ "bytes": size.bytes, "files": size.files, "need": size.need }),
                )
            })
            .collect();
        drop(sizes);
        self.write_preference("folderSizes", Value::Object(stored));
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
    /// Sets what this device does with a folder.
    ///
    /// Archive is the interesting one: `receiveonly` so this copy never sends
    /// anything back, and `ignoreDelete` so a deletion arriving from the other
    /// side is not carried out. Together they make somewhere a phone can free
    /// space against — the video leaves the phone and stays here — and the two
    /// have to be set together, because either one alone gives something else.
    pub async fn set_folder_mode(&self, folder_id: &str, mode: FolderMode) -> Result<()> {
        let (folder_type, ignore_delete) = match mode {
            FolderMode::TwoWay => ("sendreceive", false),
            FolderMode::ReceiveOnly => ("receiveonly", false),
            FolderMode::Archive => ("receiveonly", true),
        };
        self.patch(
            &format!("/rest/config/folders/{folder_id}"),
            json!({ "type": folder_type, "ignoreDelete": ignore_delete }),
        )
        .await?;
        Ok(())
    }

    /// The devices this one can currently reach.
    ///
    /// A device counts as reachable for [`CONNECTION_GRACE`] after it was last
    /// seen, which is not fudging: Syncthing 2 holds several connections to the
    /// same device and swaps between them, so a folder halfway through a large
    /// transfer reported "not connected" for one poll several times a minute.
    /// Nothing stopped moving; only the screen said otherwise.
    /// Where this platform's recycle bin is, as a command the engine can run.
    ///
    /// Only the platform knows: on the desktop it is this very binary invoked
    /// with a flag, and on a phone there is nothing to point at.
    pub fn set_trash_command(&self, command: String) {
        if let Ok(mut slot) = self.trash_command.lock() {
            *slot = Some(command);
        }
    }

    fn trash_command(&self) -> Option<String> {
        self.trash_command.lock().ok().and_then(|c| c.clone())
    }

    /// Puts the recycle-bin policy in place the first time this version runs.
    ///
    /// Installs made before it existed keep copies in a hidden folder nobody
    /// ever found, so they are moved over once — and only once, so that
    /// choosing something else later is not undone at the next launch.
    pub async fn ensure_deletion_policy(&self) -> Result<()> {
        let command = self.trash_command();
        let versioning = self
            .get("/rest/config/defaults/folder")
            .await
            .map(|defaults| defaults["versioning"].clone())
            .unwrap_or(Value::Null);
        let chosen_already = self.read_preferences()["deletionPolicySet"].as_bool() == Some(true);
        if chosen_already && !points_somewhere_else(&versioning, command.as_deref()) {
            return Ok(());
        }
        self.apply_deletion_policy(DeletionPolicy::Bin, DEFAULT_KEPT_COPIES)
            .await?;
        self.write_preference("deletionPolicySet", json!(true));
        Ok(())
    }

    /// Everything deleted out of a folder that can still be brought back.
    ///
    /// Reads both places a copy can be: the desktop's recycle bin, and the
    /// hidden folder the engine fills where there is no bin. The interface
    /// shows one list either way — where the file physically sits is not a
    /// question anyone should have to answer.
    pub async fn deleted_files(&self, folder_id: &str) -> Result<Vec<DeletedFile>> {
        let folder = self.folder_config(folder_id).await?;
        Ok(trash::deleted_in(Path::new(&folder.path)))
    }

    /// Puts one back, and asks the engine to look at the folder straight away
    /// so the other devices get it back too rather than in a minute's time.
    pub async fn restore_deleted(&self, folder_id: &str, id: &str) -> Result<String> {
        let folder = self.folder_config(folder_id).await?;
        // Checked before anything moves: an id is a path, and one from
        // somewhere else would otherwise put a file wherever it pleased.
        if !trash::destination_of(id)?.starts_with(&folder.path) {
            return Err(Error::Engine("esa copia no es de esta carpeta".into()));
        }
        let restored = trash::restore(id)?;
        let _ = self.rescan(folder_id).await;
        Ok(restored.to_string_lossy().into_owned())
    }

    async fn folder_config(&self, folder_id: &str) -> Result<FolderConfig> {
        let folders: Vec<FolderConfig> =
            serde_json::from_value(self.get("/rest/config/folders").await?)?;
        folders
            .into_iter()
            .find(|f| f.id == folder_id)
            .ok_or_else(|| Error::Engine("esa carpeta ya no está".into()))
    }

    /// Who is connected (through the grace period), and by which route.
    async fn connected_devices(&self) -> Result<(Vec<String>, HashMap<String, Route>)> {
        let value = self.get("/rest/system/connections").await?;
        let mut routes = HashMap::new();
        let now: Vec<String> = value["connections"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter(|(_, v)| v["connected"].as_bool().unwrap_or(false))
                    .map(|(k, v)| {
                        if let Some(route) = route_of(v) {
                            routes.insert(k.clone(), route);
                        }
                        k.clone()
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Rides along with the poll, a couple of times a minute at most.
        let due = self
            .routes_pinned_at
            .lock()
            .map(|mut at| {
                let due = at.is_none_or(|t| t.elapsed() >= PIN_ROUTES_EVERY);
                if due {
                    *at = Some(Instant::now());
                }
                due
            })
            .unwrap_or(false);
        if due {
            let _ = self.pin_lan_routes(&value).await;
        }

        let Ok(mut seen) = self.last_seen.lock() else { return Ok((now, routes)) };
        let at = Instant::now();
        for device in &now {
            seen.insert(device.clone(), at);
        }
        seen.retain(|_, when| when.elapsed() < CONNECTION_GRACE);
        Ok((seen.keys().cloned().collect(), routes))
    }

    /// Writes down the LAN address each connected peer was reached at, next
    /// to `dynamic` in its device entry.
    ///
    /// Finding a peer on the local network otherwise depends on hearing its
    /// broadcasts, or on the internet discovery servers echoing its LAN
    /// address back. A phone hears no broadcasts unless it holds a multicast
    /// lock, and the discovery servers are neither instant nor always
    /// reachable — so after a dropped connection the first route found again
    /// was often a relay, and a relay is where the sync then stayed for tens
    /// of minutes (seen in the desktop's own connection history). A pinned
    /// address is dialled straight away on every reconnect, whatever
    /// discovery knows; `dynamic` stays first so a peer that moved is still
    /// found the usual way.
    async fn pin_lan_routes(&self, connections: &Value) -> Result<()> {
        let Some(peers) = connections["connections"].as_object() else { return Ok(()) };
        let discovered = self.get("/rest/system/discovery").await.unwrap_or(Value::Null);
        for (device_id, entry) in peers {
            if !entry["connected"].as_bool().unwrap_or(false) {
                continue;
            }
            let heard: Vec<String> = discovered[device_id.as_str()]["addresses"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            let fresh = lan_addresses_of(entry, &heard);
            if fresh.is_empty() {
                continue;
            }
            let Ok(config) = self.get(&format!("/rest/config/devices/{device_id}")).await else {
                continue;
            };
            let current: Vec<String> = config["addresses"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if let Some(wanted) = with_pinned(&current, &fresh) {
                let _ = self
                    .patch(&format!("/rest/config/devices/{device_id}"), json!({ "addresses": wanted }))
                    .await;
            }
        }
        Ok(())
    }

    /// Adopts the name a peer announced when it last connected.
    ///
    /// A device's name travels in the handshake, and the engine writes it down
    /// only the first time, when it has nothing else. So renaming a phone
    /// changed nothing on the laptop: it went on showing the name from the day
    /// they were paired. Reading it back from the connection events is what
    /// makes a rename show up on every device instead of only its own.
    ///
    /// Best effort, and never fatal: this rides along with the folder poll.
    async fn adopt_announced_names(&self) -> Result<()> {
        let since = self.names_cursor.lock().map(|c| *c).unwrap_or(0);
        let events = self
            .get(&format!(
                "/rest/events?since={since}&timeout=0&events=DeviceConnected"
            ))
            .await?;
        let Some(events) = events.as_array() else { return Ok(()) };

        let mut latest = since;
        let mut announced: HashMap<String, String> = HashMap::new();
        for event in events {
            latest = latest.max(event["id"].as_i64().unwrap_or(0));
            let (Some(id), Some(name)) = (
                event["data"]["id"].as_str(),
                event["data"]["deviceName"].as_str(),
            ) else {
                continue;
            };
            if is_a_real_name(name) {
                announced.insert(id.to_string(), name.to_string());
            }
        }
        if let Ok(mut cursor) = self.names_cursor.lock() {
            *cursor = latest;
        }

        for (id, name) in announced {
            let known = self
                .get(&format!("/rest/config/devices/{id}"))
                .await
                .ok()
                .and_then(|d| d["name"].as_str().map(str::to_string))
                .unwrap_or_default();
            if known != name {
                let _ = self
                    .patch(&format!("/rest/config/devices/{id}"), json!({ "name": name }))
                    .await;
            }
        }
        Ok(())
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
        best_hints(hints)
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
            deletion_policy: policy_from_versioning(&defaults["versioning"]),
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

        self.apply_deletion_policy(settings.deletion_policy, settings.keep_versions)
            .await
    }

    /// Tells the engine where deleted files go, on the folders that exist as
    /// well as on the template new ones are cut from, so the setting means the
    /// same thing everywhere.
    pub async fn apply_deletion_policy(&self, policy: DeletionPolicy, keep: u32) -> Result<()> {
        let versioning = versioning_for(policy, keep, self.trash_command().as_deref());
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
    /// Syncthing's switch for "never apply a deletion that arrives from
    /// somewhere else". Together with `receiveonly` it is what makes a copy
    /// safe to delete from on the other side.
    #[serde(default)]
    ignore_delete: bool,
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

/// How long the rest of a download takes at the speed it is going.
///
/// `None` rather than a very large number when nothing is moving: "faltan
/// 2 h" that never counts down is worse than saying nothing, and a stalled
/// folder is exactly when a made-up estimate does the most damage.
fn eta(need: u64, bytes_per_second: u64) -> Option<u64> {
    if need == 0 || bytes_per_second == 0 {
        return None;
    }
    Some(need / bytes_per_second)
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

/// How a connected peer is reached, from its `/rest/system/connections`
/// entry: the best of its connections, since v2 keeps several at once.
fn route_of(entry: &Value) -> Option<Route> {
    let mut all = vec![&entry["primary"]];
    if let Some(more) = entry["secondary"].as_array() {
        all.extend(more.iter());
    }
    // Older engines have no primary block, only the top-level fields.
    if entry["primary"].is_null() {
        all = vec![entry];
    }
    let mut best: Option<Route> = None;
    for conn in all {
        let kind = conn["type"].as_str().unwrap_or("");
        if kind.is_empty() {
            continue;
        }
        let route = if kind.starts_with("relay") {
            Route::Relay
        } else if conn["isLocal"].as_bool().unwrap_or(false) {
            Route::Lan
        } else {
            Route::Internet
        };
        let rank = |r: Route| match r {
            Route::Lan => 0,
            Route::Internet => 1,
            Route::Relay => 2,
        };
        if best.is_none_or(|b| rank(route) < rank(b)) {
            best = Some(route);
        }
    }
    best
}

/// The dialable LAN addresses of a connected peer.
///
/// A connection we dialled carries the peer's listening address; one it
/// dialled in carries a throwaway source port, so for those the listening
/// address is taken from what discovery heard at the same IP. IPv4 private
/// ranges only: IPv6 addresses on a home network rotate daily, and
/// link-local ones need a zone that differs per machine.
fn lan_addresses_of(entry: &Value, discovered: &[String]) -> Vec<String> {
    let mut conns = vec![&entry["primary"]];
    if let Some(more) = entry["secondary"].as_array() {
        conns.extend(more.iter());
    }
    if entry["primary"].is_null() {
        conns = vec![entry];
    }
    let mut out: Vec<String> = Vec::new();
    let mut push = |a: String| {
        if !out.contains(&a) {
            out.push(a);
        }
    };
    for conn in conns {
        let kind = conn["type"].as_str().unwrap_or("");
        if kind.starts_with("relay") || !conn["isLocal"].as_bool().unwrap_or(false) {
            continue;
        }
        let Some(address) = conn["address"].as_str() else { continue };
        let Ok(socket) = address.parse::<std::net::SocketAddr>() else { continue };
        let std::net::IpAddr::V4(ip) = socket.ip() else { continue };
        if !ip.is_private() {
            continue;
        }
        let proto = kind.split('-').next().unwrap_or("tcp");
        if kind.ends_with("-client") {
            push(format!("{proto}://{socket}"));
        } else {
            for heard in discovered {
                let host = heard.split("://").nth(1).and_then(|r| r.rsplit_once(':')).map(|(h, _)| h);
                if host == Some(&ip.to_string()) && (heard.starts_with("tcp://") || heard.starts_with("quic://")) {
                    push(heard.clone());
                }
            }
        }
    }
    // TCP first: it is the one Syncthing ranks best on a LAN.
    out.sort_by_key(|a| if a.starts_with("tcp://") { 0 } else { 1 });
    out
}

/// A device's address list with `fresh` LAN addresses pinned after
/// `dynamic`, or `None` when nothing would change. Leaves alone a device
/// someone configured without `dynamic`: that list was chosen by hand.
fn with_pinned(current: &[String], fresh: &[String]) -> Option<Vec<String>> {
    if !current.iter().any(|a| a == "dynamic") {
        return None;
    }
    let mut explicit: Vec<String> = fresh.to_vec();
    for old in current.iter().filter(|a| *a != "dynamic") {
        if !explicit.contains(old) {
            explicit.push(old.clone());
        }
    }
    explicit.truncate(MAX_PINNED);
    let mut wanted = vec!["dynamic".to_string()];
    wanted.extend(explicit);
    (wanted != current).then_some(wanted)
}

/// The few addresses worth putting in a pairing code.
///
/// Every address makes the QR denser, and a dense QR read off one screen by
/// another phone's camera is the difference between pairing in two seconds and
/// giving up and typing the code by hand. A machine running containers offers
/// half a dozen addresses no other device can reach; those go last and, past
/// the cap, not at all. Discovery finds anything left out — the hints only
/// make the first connection quicker.
fn best_hints(mut hints: Vec<String>) -> Vec<String> {
    hints.sort_by_key(|address| if is_a_home_address(address) { 0 } else { 1 });
    hints.truncate(MAX_HINTS);
    hints
}

/// A home network's own numbering, as opposed to the ranges Docker and the
/// like hand themselves.
fn is_a_home_address(address: &str) -> bool {
    address.contains("://192.168.") || address.contains("://10.")
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
/// How the engine is told to treat a file it is about to delete or replace.
///
/// `external` hands each one to a command instead of destroying it, which is
/// how a deletion arriving from another device ends up in this desktop's own
/// recycle bin. Without such a command — a phone — the engine keeps the copy
/// itself, in the hidden folder beside the files.
fn versioning_for(policy: DeletionPolicy, keep: u32, trash_command: Option<&str>) -> Value {
    let blank = json!({ "type": "", "params": {}, "cleanupIntervalS": 3600, "fsPath": "", "fsType": "basic" });
    match policy {
        DeletionPolicy::Nothing => blank,
        DeletionPolicy::Bin => match trash_command {
            Some(command) => json!({
                "type": "external",
                "params": { "command": command },
                "cleanupIntervalS": 3600,
                "fsPath": "",
                "fsType": "basic"
            }),
            // Asked for a recycle bin on something that has none. The nearest
            // honest thing is keeping the copies, not throwing them away.
            None => versioning_for(DeletionPolicy::Copies, keep.max(1), None),
        },
        DeletionPolicy::Copies => {
            if keep == 0 {
                return blank;
            }
            json!({
                "type": "simple",
                "params": { "keep": keep.to_string() },
                "cleanupIntervalS": 3600,
                "fsPath": "",
                "fsType": "basic"
            })
        }
    }
}

/// Whether the engine is set to call a binary that is no longer the one
/// running.
///
/// The command is this executable's own path, so installing the .deb over a
/// build from source — or the other way round — leaves the engine calling
/// something that is not there. A versioner that fails is a deletion that
/// never completes and a folder stuck on an error, so the path is checked on
/// every launch and put right when it has moved.
fn points_somewhere_else(versioning: &Value, command: Option<&str>) -> bool {
    let Some(command) = command else { return false };
    versioning["type"] == "external" && versioning["params"]["command"] != command
}

fn mode_of(folder: &FolderConfig) -> FolderMode {
    match (folder.folder_type.as_str(), folder.ignore_delete) {
        ("receiveonly", true) => FolderMode::Archive,
        ("receiveonly", false) => FolderMode::ReceiveOnly,
        // A folder that sends its changes and ignores deletions is not a mode
        // this app offers; it would delete on one side and not the other with
        // nothing on screen saying so. Read as the plain two-way folder it
        // mostly is, and set back to that the next time the mode is chosen.
        _ => FolderMode::TwoWay,
    }
}

fn policy_from_versioning(versioning: &Value) -> DeletionPolicy {
    match versioning["type"].as_str().unwrap_or("") {
        "external" => DeletionPolicy::Bin,
        "simple" | "trashcan" | "staggered" => DeletionPolicy::Copies,
        _ => DeletionPolicy::Nothing,
    }
}

fn keep_from_versioning(versioning: &Value) -> u32 {
    if versioning["type"].as_str().unwrap_or("") != "simple" {
        return DEFAULT_KEPT_COPIES;
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
    /// Shaped like the desktop's real `/rest/system/connections` entry for
    /// the phone on 2026-10-01: one connection we dialled, one it dialled in.
    fn phone_on_the_lan() -> Value {
        json!({
            "connected": true, "type": "tcp-client", "address": "192.168.1.147:22000", "isLocal": true,
            "primary": { "type": "tcp-client", "address": "192.168.1.147:22000", "isLocal": true },
            "secondary": [
                { "type": "tcp-server", "address": "192.168.1.138:48128", "isLocal": true },
                { "type": "tcp-client", "address": "[2a0c:5a85:9102:b500::1]:22000", "isLocal": true }
            ]
        })
    }

    #[test]
    fn the_best_of_several_connections_is_the_route_shown() {
        assert_eq!(route_of(&phone_on_the_lan()), Some(Route::Lan));
        let relayed = json!({ "connected": true,
            "primary": { "type": "relay-client", "address": "93.176.169.53:22067", "isLocal": false },
            "secondary": [] });
        assert_eq!(route_of(&relayed), Some(Route::Relay));
        let mixed = json!({ "connected": true,
            "primary": { "type": "relay-client", "address": "93.176.169.53:22067", "isLocal": false },
            "secondary": [{ "type": "tcp-client", "address": "79.117.213.199:22000", "isLocal": false }] });
        assert_eq!(route_of(&mixed), Some(Route::Internet));
        assert_eq!(route_of(&json!({ "connected": false, "primary": { "type": "" } })), None);
    }

    #[test]
    fn lan_addresses_come_from_the_connections_and_discovery_fills_the_port() {
        let heard = vec![
            "tcp://192.168.1.138:22000".to_string(),
            "quic://192.168.1.138:22000".to_string(),
            "tcp://10.56.123.150:22000".to_string(),
            "relay://103.214.6.110:22067/?id=X".to_string(),
        ];
        let got = lan_addresses_of(&phone_on_the_lan(), &heard);
        assert_eq!(
            got,
            vec!["tcp://192.168.1.147:22000", "tcp://192.168.1.138:22000", "quic://192.168.1.138:22000"],
            "the dialled address as is, the dialled-in one by its discovered port, never IPv6 or the ephemeral port"
        );
        let relayed = json!({ "primary": { "type": "relay-client", "address": "192.168.1.5:22067", "isLocal": true } });
        assert!(lan_addresses_of(&relayed, &heard).is_empty(), "a relay is never a LAN route");
    }

    #[test]
    fn pinned_addresses_go_after_dynamic_and_only_change_what_needs_to() {
        let fresh = vec!["tcp://192.168.1.147:22000".to_string()];
        assert_eq!(
            with_pinned(&["dynamic".into()], &fresh),
            Some(vec!["dynamic".to_string(), "tcp://192.168.1.147:22000".to_string()])
        );
        // Already there: no write, so no ConfigSaved every half minute.
        assert_eq!(with_pinned(&["dynamic".into(), "tcp://192.168.1.147:22000".into()], &fresh), None);
        // The newest goes first and the list stays short.
        let old: Vec<String> = ["dynamic", "tcp://192.168.1.2:22000", "tcp://192.168.1.3:22000", "tcp://192.168.1.4:22000"]
            .iter().map(|s| s.to_string()).collect();
        let next = with_pinned(&old, &fresh).unwrap();
        assert_eq!(next.len(), 1 + MAX_PINNED);
        assert_eq!(next[1], "tcp://192.168.1.147:22000");
        // A hand-written address list is not ours to rewrite.
        assert_eq!(with_pinned(&["tcp://nas.local:22000".into()], &fresh), None);
    }

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
    fn a_time_left_needs_something_actually_moving() {
        assert_eq!(eta(1_000, 100), Some(10));
        // A stalled folder gets no estimate rather than an infinite one.
        assert_eq!(eta(1_000, 0), None);
        assert_eq!(eta(0, 100), None);
    }

    #[test]
    fn a_pairing_code_carries_the_addresses_a_phone_can_reach() {
        let hints = best_hints(vec![
            "tcp://172.20.0.1:22000".into(),
            "tcp://192.168.1.151:22000".into(),
            "tcp://172.23.0.1:22000".into(),
            "tcp://10.0.0.4:22000".into(),
        ]);
        assert_eq!(
            hints,
            vec!["tcp://192.168.1.151:22000", "tcp://10.0.0.4:22000"],
            "container addresses must not push a real one out of the code"
        );
    }

    #[test]
    fn a_folder_speed_is_how_much_less_it_needs_than_before() {
        let client = Syncthing::new("http://127.0.0.1:1", "k");
        // The first reading has nothing to compare against.
        assert_eq!(client.folder_rate("f", 1_000_000), 0);
        std::thread::sleep(std::time::Duration::from_millis(500));
        let rate = client.folder_rate("f", 500_000);
        assert!(
            (700_000..=1_300_000).contains(&rate),
            "half a megabyte in half a second is about a megabyte a second, got {rate}"
        );
        // A folder that grows mid-sync is not going backwards.
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(client.folder_rate("f", 900_000) > 0);
    }

    #[test]
    fn a_paused_folder_keeps_the_size_it_had() {
        let client = Syncthing::new("http://127.0.0.1:1", "k");
        let real = RememberedSize { bytes: 8_000_000, files: 12, need: 3_000_000 };
        let blank = RememberedSize { bytes: 0, files: 0, need: 0 };

        assert_eq!(client.size_of("f", real, false).bytes, 8_000_000);
        // Paused, the engine answers zero to everything. Showing that reads as
        // "your files are gone".
        let shown = client.size_of("f", blank, true);
        assert_eq!(shown.bytes, 8_000_000);
        assert_eq!(shown.files, 12);
        assert_eq!(shown.need, 3_000_000);
        // A folder that is genuinely empty and not paused stays empty.
        assert_eq!(client.size_of("g", blank, false).bytes, 0);
    }

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
        let copies = |keep| versioning_for(DeletionPolicy::Copies, keep, None);
        assert_eq!(keep_from_versioning(&copies(5)), 5);
        // The parameter really must be a string; a number is silently ignored
        // by the engine.
        assert_eq!(copies(5)["params"]["keep"], json!("5"));
    }

    #[test]
    fn deletions_go_to_the_recycle_bin_when_there_is_one() {
        let bin = versioning_for(DeletionPolicy::Bin, 5, Some("/usr/bin/homecloud --trash"));
        assert_eq!(bin["type"], json!("external"));
        assert_eq!(bin["params"]["command"], json!("/usr/bin/homecloud --trash"));
        assert_eq!(policy_from_versioning(&bin), DeletionPolicy::Bin);
    }

    #[test]
    fn a_phone_asked_for_a_recycle_bin_keeps_copies_instead_of_nothing() {
        // Android has no bin an app may write to, and "no bin" must never
        // quietly become "deletions are final".
        let fallback = versioning_for(DeletionPolicy::Bin, 0, None);
        assert_eq!(fallback["type"], json!("simple"));
        assert_eq!(policy_from_versioning(&fallback), DeletionPolicy::Copies);
    }

    #[test]
    fn a_copy_that_keeps_everything_is_read_back_as_one() {
        let archive = FolderConfig {
            id: "fotos".into(),
            label: "Fotos".into(),
            path: "/srv/fotos".into(),
            folder_type: "receiveonly".into(),
            ignore_delete: true,
            paused: false,
            devices: vec![],
        };
        assert_eq!(mode_of(&archive), FolderMode::Archive);

        let receive_only = FolderConfig { ignore_delete: false, ..archive };
        assert_eq!(mode_of(&receive_only), FolderMode::ReceiveOnly);

        let two_way = FolderConfig { folder_type: "sendreceive".into(), ..receive_only };
        assert_eq!(mode_of(&two_way), FolderMode::TwoWay);

        // Sending changes while ignoring deletions is not a mode this app
        // offers, and must not be mistaken for the one that is.
        let neither = FolderConfig { ignore_delete: true, ..two_way };
        assert_eq!(mode_of(&neither), FolderMode::TwoWay);
    }

    #[test]
    fn a_binary_that_moved_is_noticed_and_a_deliberate_choice_is_not() {
        let bin = versioning_for(DeletionPolicy::Bin, 5, Some("/usr/bin/homecloud --trash"));
        assert!(!points_somewhere_else(&bin, Some("/usr/bin/homecloud --trash")));
        assert!(points_somewhere_else(&bin, Some("/opt/homecloud --trash")));

        // Someone who asked for hidden copies, or for nothing, must not have
        // the recycle bin put back under them at the next launch.
        let copies = versioning_for(DeletionPolicy::Copies, 5, None);
        assert!(!points_somewhere_else(&copies, Some("/opt/homecloud --trash")));
        let nothing = versioning_for(DeletionPolicy::Nothing, 5, None);
        assert!(!points_somewhere_else(&nothing, Some("/opt/homecloud --trash")));
    }

    #[test]
    fn keeping_nothing_is_only_ever_what_was_asked_for() {
        let nothing = versioning_for(DeletionPolicy::Nothing, 5, Some("cmd"));
        assert_eq!(nothing["type"], json!(""));
        assert_eq!(policy_from_versioning(&nothing), DeletionPolicy::Nothing);
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
