//! Sharing a folder as a link anybody can open.
//!
//! Syncing needs HomeCloud on both ends. A link needs nothing: someone opens it
//! in a browser and takes what they want. That is a different job, and it is
//! done by a different process — `hcshare`, which serves the folder itself and
//! reaches the internet through a Cloudflare Quick Tunnel.
//!
//! Why a tunnel at all: a phone on mobile data has no address of its own, and a
//! home router does not let anything in. A quick tunnel inverts that: the
//! helper dials *out* and keeps the connection open, and Cloudflare hands back
//! a random `https://something.trycloudflare.com` address that it forwards
//! back down the same pipe. Nobody ever connects into the device.
//!
//! No account anywhere, which is also why every link is temporary and says so:
//! a quick tunnel gets a new address every time and Cloudflare promises no
//! uptime, so pretending it is permanent would be a lie the app tells on the
//! user's behalf. A link expires on its own after a bounded time, and the
//! interface is expected to show that countdown rather than hide it.
//!
//! The password — if there is one — is checked by `hcshare` itself, in this
//! process's own HTTP server. Nothing is handed to Cloudflare to compare
//! against what a visitor types, because there is no Cloudflare account behind
//! a quick tunnel for it to check against even if we wanted that.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::error::{Error, Result};

/// How long a link lasts when nothing else is said. Long enough to hand
/// someone a link and have them get to it that evening, short enough that a
/// forgotten link does not sit open for a week.
pub const DEFAULT_LIFETIME: Duration = Duration::from_secs(6 * 3600);

/// What the interface needs to know about a live link. Carries no handle to
/// the process: that stays inside `Links`, so a caller can never accidentally
/// hold or drop the thing that is keeping the link alive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkStatus {
    pub url: String,
    /// Seconds since the epoch. When this passes, `hcshare` stops on its own;
    /// the interface is expected to count down to it rather than hide it.
    pub expires_at: u64,
}

/// A folder being served right now. Never leaves this module.
struct Link {
    status: LinkStatus,
    child: Child,
}

/// Owns the helper processes, one per shared folder.
pub struct Links {
    /// The `hcshare` binary, which serves the folder and drives the tunnel.
    share_binary: PathBuf,
    /// The `cloudflared` binary `hcshare` spawns to reach the internet.
    tunnel_binary: PathBuf,
    live: Mutex<HashMap<String, Link>>,
}

/// The helper's first line of output: either the share or why there is none.
#[derive(Deserialize)]
struct Announcement {
    url: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<u64>,
    error: Option<String>,
}

impl Links {
    pub fn new(share_binary: PathBuf, tunnel_binary: PathBuf) -> Self {
        Links { share_binary, tunnel_binary, live: Mutex::new(HashMap::new()) }
    }

    /// Starts serving `path` and returns the address to hand out, plus when it
    /// stops working on its own.
    ///
    /// `basic_auth` is `usuario:contraseña`, or empty for a link anyone with
    /// the URL can open. It cannot reuse the folder's sync password: that one
    /// is only ever stored as a derived key, deliberately, so there is nothing
    /// to hand to a check that compares it against plain text someone typed.
    pub async fn start(
        &self,
        folder_id: &str,
        path: &Path,
        basic_auth: &str,
        lifetime: Duration,
    ) -> Result<LinkStatus> {
        if self.status_for(folder_id).is_some() {
            self.stop(folder_id).await?;
        }
        if !self.tunnel_binary.exists() {
            return Err(Error::Engine(format!(
                "falta {} en esta instalación",
                self.tunnel_binary.display()
            )));
        }

        let mut command = Command::new(&self.share_binary);
        command
            .arg("share")
            .arg(path)
            .arg(&self.tunnel_binary)
            .arg(basic_auth)
            .arg(format!("--for={}s", lifetime.as_secs().max(60)))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command
            .spawn()
            .map_err(|e| Error::Engine(format!("no se pudo lanzar el enlace: {e}")))?;

        // hcshare prints the share as one line and then serves until it is
        // killed or its time runs out, so the URL arrives on stdout rather
        // than as an exit status.
        let stdout = child.stdout.take().ok_or_else(|| Error::Engine("sin salida".into()))?;
        let mut lines = BufReader::new(stdout).lines();
        let first = tokio::time::timeout(Duration::from_secs(60), lines.next_line())
            .await
            .map_err(|_| Error::Engine("no contestó a tiempo".into()))?
            .map_err(|e| Error::Engine(format!("no se pudo leer la respuesta: {e}")))?;

        let announced =
            first.as_deref().and_then(|line| serde_json::from_str::<Announcement>(line).ok());
        let Some(announced) = announced else {
            return Err(Error::Engine(self.explain(&mut child).await));
        };
        if let Some(problem) = announced.error {
            let _ = child.start_kill();
            return Err(Error::Engine(problem));
        }
        let url = announced
            .url
            .filter(|u| !u.is_empty())
            .ok_or_else(|| Error::Engine("no se devolvió ninguna dirección".into()))?;
        let expires_at = announced.expires_at.unwrap_or_else(|| now_secs() + lifetime.as_secs());

        let status = LinkStatus { url, expires_at };
        if let Ok(mut live) = self.live.lock() {
            live.insert(folder_id.to_string(), Link { status: status.clone(), child });
        }
        Ok(status)
    }

    /// Stops serving a folder. The link dies with the process, which is the
    /// point: there is nothing left running to reach.
    pub async fn stop(&self, folder_id: &str) -> Result<()> {
        let taken = self.live.lock().ok().and_then(|mut live| live.remove(folder_id));
        if let Some(mut link) = taken {
            let _ = link.child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(5), link.child.wait()).await;
        }
        Ok(())
    }

    /// Takes every link down. Called when the app is closing: a tunnel that
    /// outlives the window is one nobody can see or stop.
    pub async fn stop_all(&self) {
        let ids: Vec<String> =
            self.live.lock().map(|live| live.keys().cloned().collect()).unwrap_or_default();
        for id in ids {
            let _ = self.stop(&id).await;
        }
    }

    /// The address a folder is being served at and when it expires, if it is
    /// being served at all.
    pub fn status_for(&self, folder_id: &str) -> Option<LinkStatus> {
        let mut live = self.live.lock().ok()?;
        // A helper that died — its time ran out, or it crashed — takes its
        // link with it: reporting a URL that answers nothing is worse than
        // reporting none.
        let dead = live
            .get_mut(folder_id)
            .map(|link| matches!(link.child.try_wait(), Ok(Some(_))))
            .unwrap_or(false);
        if dead {
            live.remove(folder_id);
            return None;
        }
        live.get(folder_id).map(|link| link.status.clone())
    }

    /// Whatever the helper managed to say before failing to announce a share.
    async fn explain(&self, child: &mut Child) -> String {
        let _ = child.start_kill();
        let Some(stderr) = child.stderr.take() else {
            return "no se devolvió ninguna dirección".into();
        };
        let mut lines = BufReader::new(stderr).lines();
        let said = tokio::time::timeout(Duration::from_secs(2), lines.next_line())
            .await
            .ok()
            .and_then(|line| line.ok())
            .flatten()
            .unwrap_or_default();
        serde_json::from_str::<Announcement>(said.trim())
            .ok()
            .and_then(|a| a.error)
            .unwrap_or_else(|| {
                if said.is_empty() {
                    "no se devolvió ninguna dirección".into()
                } else {
                    said
                }
            })
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The helper binaries, beside the sync engine they ship next to.
pub fn share_binary(resource_dir: Option<&Path>) -> Result<PathBuf> {
    find_binary(resource_dir, "hcshare", "hcshare.exe")
}

pub fn tunnel_binary(resource_dir: Option<&Path>) -> Result<PathBuf> {
    find_binary(resource_dir, "cloudflared", "cloudflared.exe")
}

fn find_binary(resource_dir: Option<&Path>, unix_name: &str, windows_name: &str) -> Result<PathBuf> {
    let name = if cfg!(target_os = "windows") { windows_name } else { unix_name };
    resource_dir
        .map(|dir| dir.join(name))
        .filter(|path| path.exists())
        .ok_or_else(|| Error::Engine(format!("{name} no viene en esta instalación")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_helper_is_reported_rather_than_guessed_at() {
        let empty = std::env::temp_dir().join("homecloud-no-helper-here");
        let error = share_binary(Some(&empty)).unwrap_err().to_string();
        assert!(error.contains("hcshare"), "{error}");
    }

    #[test]
    fn a_folder_that_is_not_being_served_has_no_link() {
        let links = Links::new(PathBuf::from("/nonexistent"), PathBuf::from("/nonexistent"));
        assert!(links.status_for("cualquiera").is_none());
    }

    /// The announcement is the contract between the helper and this module, so
    /// both shapes it can take are pinned here.
    #[test]
    fn the_helpers_announcement_is_read_either_way() {
        let good: Announcement =
            serde_json::from_str(r#"{"url":"https://x.trycloudflare.com","expiresAt":123}"#)
                .unwrap();
        assert_eq!(good.url.as_deref(), Some("https://x.trycloudflare.com"));
        assert_eq!(good.expires_at, Some(123));
        assert!(good.error.is_none());

        let bad: Announcement = serde_json::from_str(r#"{"error":"no es una carpeta"}"#).unwrap();
        assert_eq!(bad.error.as_deref(), Some("no es una carpeta"));
        assert!(bad.url.is_none());
    }

    #[test]
    fn default_lifetime_is_a_handful_of_hours_not_forever() {
        assert_eq!(DEFAULT_LIFETIME, Duration::from_secs(6 * 3600));
    }
}
