//! The vocabulary HomeCloud shows to a person.
//!
//! Syncthing talks about devices, folders, cluster config and pending entries.
//! A person has folders they share and people asking to share one. These types
//! are that smaller vocabulary; the client module maps Syncthing's onto it.

use serde::{Deserialize, Serialize};

/// Another machine this one syncs with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    /// Canonical Syncthing device ID.
    pub id: String,
    /// What the peer calls itself, e.g. "Pixel de Lucas".
    pub name: String,
    pub connected: bool,
}

/// What a folder is doing right now, in the terms the one status dot uses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FolderState {
    /// Everything that should be here is here.
    UpToDate,
    /// Files are moving. `percent` is completion across all peers.
    Syncing { percent: u8 },
    /// Deliberately stopped by the user.
    Paused,
    /// Nothing to sync with: every peer is unreachable.
    Disconnected,
    /// Something needs a human. `detail` is already phrased for one.
    Problem { detail: String },
}

/// A folder this device shares with at least one other.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolder {
    /// Syncthing's folder ID. Stable, shared across devices, never shown.
    pub id: String,
    /// The name a person reads, e.g. "Fotos".
    pub label: String,
    /// Where the folder lives on *this* device. Differs per device by design.
    pub path: String,
    pub state: FolderState,
    pub peers: Vec<Peer>,
    /// Total size of the folder as every device agrees it should be.
    pub bytes: u64,
    pub files: u64,
    /// Conflicting copies Syncthing kept because two devices edited at once.
    /// Non-zero means there is something for the user to look at.
    pub conflicts: u64,
}

/// Someone is asking to share something with this device.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invitation {
    pub from_device_id: String,
    pub from_device_name: String,
    /// Present once the peer is known and has actually offered a folder.
    /// A brand-new device shows up with no folder yet: it is asking to be
    /// trusted first, and the folder offer follows a second later.
    pub folder: Option<OfferedFolder>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferedFolder {
    pub id: String,
    pub label: String,
}

/// This device's own identity, as shown on the pairing screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThisDevice {
    pub id: String,
    pub name: String,
}

/// Everything the settings screen can change. Read and written as a whole:
/// there are few enough knobs that a partial update would only add ways to get
/// the two out of step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// What other devices call this one. The only setting most people touch.
    pub device_name: String,
    /// Shown so it can be read out or compared when a pairing goes wrong.
    /// Never editable: it is derived from this device's certificate.
    pub device_id: String,
    /// Refuse to announce to, or relay through, anything outside the local
    /// network. Sync then works at home and nowhere else, which is exactly what
    /// some people want.
    pub local_network_only: bool,
    /// Kilobytes per second, 0 meaning no limit.
    pub upload_limit_kbps: u32,
    pub download_limit_kbps: u32,
    /// How many superseded copies of a changed file to keep. 0 turns it off.
    /// This is the difference between "synced a deletion" and "lost the file".
    pub keep_versions: u32,
    /// Version of the bundled engine, for bug reports.
    pub engine_version: String,
}
