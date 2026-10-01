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
    /// How much of this folder that peer already has, 0-100. `None` when the
    /// engine cannot say — a peer that has never connected, or a folder that
    /// is paused here and therefore has no numbers at all.
    ///
    /// This is the answer to "is the phone finished yet" asked from the
    /// device that handed the folder out, which otherwise only ever sees its
    /// own copy and reports itself up to date while the other end is at 4%.
    pub completion: Option<u8>,
    /// Which way the bytes are travelling while connected. `None` when not
    /// connected right now.
    ///
    /// Shown because "it seems to always go over the internet" is otherwise
    /// a guess from how slow it feels: this is the engine's own answer.
    #[serde(default)]
    pub route: Option<Route>,
}

/// How a connected peer is being reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Route {
    /// Directly, on the same network. The fast path.
    Lan,
    /// Directly, but across the internet.
    Internet,
    /// Through a community relay: works anywhere, and is the slowest.
    Relay,
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
    /// How fast bytes are moving right now, zero when nothing is. Shown under
    /// the percentage, because "syncing 43%" says nothing about whether it is
    /// about to finish or has stalled.
    pub bytes_per_second: u64,
    /// What this copy of the folder does: both ways, only receive, or keep
    /// everything for ever.
    pub mode: FolderMode,
    /// What this copy holds that the other devices no longer have — the
    /// videos deleted off a full phone, still here. Zero unless this is the
    /// copy keeping them.
    pub extra_bytes: u64,
    /// Room left where this folder lives. `None` when the disk cannot be asked.
    pub free_bytes: Option<u64>,
    /// Still to come down. Compared against `free_bytes` this is what says
    /// "this is not going to fit" while there is still time to act.
    pub pending_bytes: u64,
    /// Hold off while the connection is a metered one. Off by default: a
    /// folder that silently refuses to sync is worse than one that costs data,
    /// and only the person paying the bill knows which folders are big.
    pub wifi_only: bool,
    /// Stopped because of `wifi_only`, not because anyone asked. Kept apart so
    /// resuming never un-pauses a folder the user paused on purpose.
    pub paused_by_network: bool,
    /// Codes for this folder are encrypted, and cannot be used without the
    /// password. Off unless someone set one.
    pub has_password: bool,
    /// How long the rest of the download would take at the speed measured
    /// just now, in seconds. `None` when nothing is moving, so there is no
    /// honest estimate to give.
    ///
    /// A percentage on its own does not say whether to wait or walk away;
    /// this and `bytes_per_second` are what turn it into a decision.
    pub eta_seconds: Option<u64>,
}

/// What one device does with a folder it shares.
///
/// The third mode is the one that turns a second device into somewhere it is
/// safe to delete from: a phone that is running out of room can lose its
/// videos without the copy losing them too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderMode {
    /// The same folder on both sides. What most folders are.
    TwoWay,
    /// Takes changes from the other devices but never sends its own.
    ReceiveOnly,
    /// Takes everything and gives nothing back, and never applies a deletion:
    /// what is here stays here even after it is gone everywhere else.
    Archive,
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

/// What happens to a file when another device deletes or replaces it.
///
/// Syncing a deletion is the one change that cannot be undone by syncing
/// again, so the question is never "keep a copy or not" but "where".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeletionPolicy {
    /// Into the desktop's own recycle bin, where the file manager can also
    /// see it and put it back. The default wherever there is one.
    Bin,
    /// Into a hidden folder beside the files. What a phone gets, because
    /// Android has no recycle bin an app may write to on its own.
    Copies,
    /// Gone. Nothing is kept, and a deletion synced by mistake is final.
    Nothing,
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
    /// Where a deleted or replaced file goes.
    pub deletion_policy: DeletionPolicy,
    /// How many superseded copies of a changed file to keep, when they are
    /// kept in the hidden folder. Ignored by the other two policies.
    pub keep_versions: u32,
    /// Version of the bundled engine, for bug reports.
    pub engine_version: String,
    /// The interface language: "es" or "en". Kept with the rest of the settings
    /// so it survives a reinstall along with everything else, rather than in
    /// browser storage the phone does not have.
    pub language: String,
}
