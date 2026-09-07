//! Sharing a folder as a link anybody can open.
//!
//! Syncing needs HomeCloud on both ends. A link needs nothing: someone opens it
//! in a browser and takes what they want. That is a different job, and it is
//! done by a different process — `hcshare`, which serves the folder over a
//! [zrok](https://zrok.io) public share.
//!
//! Why a tunnel at all: a phone on mobile data has no address of its own, and
//! a home router does not let anything in. zrok inverts that. The helper dials
//! *out* to a zrok frontend and keeps that connection open, and the frontend
//! hands back a public HTTPS address that it forwards back down the same pipe.
//! Nobody ever connects into the device.
//!
//! Two things follow from it being someone else's frontend, and both are said
//! out loud in the interface rather than hidden here: the link only works while
//! the device is on and the process is running, and the password on a link is
//! checked by zrok's frontend, not by this device.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::error::{Error, Result};

/// A folder being served right now.
pub struct Link {
    pub url: String,
    child: Child,
}

/// Owns the helper processes, one per shared folder.
pub struct Links {
    binary: PathBuf,
    /// Where the helper keeps which zrok account this device belongs to. Set to
    /// the app's own directory so nothing is written to the real home.
    home: PathBuf,
    live: Mutex<HashMap<String, Link>>,
}

/// The helper's first line of output: either the share or why there is none.
#[derive(Deserialize)]
struct Announcement {
    url: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct Status {
    enabled: bool,
}

impl Links {
    pub fn new(binary: PathBuf, home: PathBuf) -> Self {
        Links { binary, home, live: Mutex::new(HashMap::new()) }
    }

    /// Whether this device has joined a zrok account. Until it has, there is
    /// nothing to make a link out of.
    pub async fn is_ready(&self) -> bool {
        let Ok(output) = self.run(&["status"]).await else {
            return false;
        };
        serde_json::from_str::<Status>(&output).map(|s| s.enabled).unwrap_or(false)
    }

    /// Joins the zrok account the token belongs to. Done once per device.
    pub async fn join(&self, token: &str) -> Result<()> {
        let token = token.trim();
        if token.is_empty() {
            return Err(Error::Engine("hace falta el token de tu cuenta de zrok".into()));
        }
        self.run(&["enable", token]).await.map(|_| ())
    }

    /// Starts serving `path` and returns the address to hand out.
    ///
    /// `basic_auth` is `usuario:contraseña` or empty for a link with no
    /// password. It cannot reuse the folder's sync password: that one is only
    /// ever stored as a derived key, deliberately, so there is nothing to hand
    /// to a frontend that has to compare it against what a visitor types.
    pub async fn start(&self, folder_id: &str, path: &Path, basic_auth: &str) -> Result<String> {
        if self.url_for(folder_id).is_some() {
            self.stop(folder_id).await?;
        }

        let mut command = Command::new(&self.binary);
        command
            .arg("share")
            .arg(path)
            .env("ZROK_HOME", &self.home)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if !basic_auth.is_empty() {
            command.arg(basic_auth);
        }

        let mut child = command
            .spawn()
            .map_err(|e| Error::Engine(format!("no se pudo lanzar el enlace: {e}")))?;

        // The helper prints the share as one line and then serves until killed,
        // so the URL arrives on stdout rather than as an exit status.
        let stdout = child.stdout.take().ok_or_else(|| Error::Engine("sin salida".into()))?;
        let mut lines = BufReader::new(stdout).lines();
        let first = tokio::time::timeout(std::time::Duration::from_secs(45), lines.next_line())
            .await
            .map_err(|_| Error::Engine("zrok no contestó a tiempo".into()))?
            .map_err(|e| Error::Engine(format!("no se pudo leer la respuesta: {e}")))?;

        let announced: Option<Announcement> =
            first.as_deref().and_then(|line| serde_json::from_str(line).ok());
        let Some(announced) = announced else {
            // Nothing parseable came back, so whatever the helper said on
            // stderr is the only explanation there is.
            return Err(Error::Engine(self.explain(&mut child).await));
        };

        if let Some(problem) = announced.error {
            let _ = child.start_kill();
            return Err(Error::Engine(problem));
        }
        let url = announced.url.filter(|u| !u.is_empty()).ok_or_else(|| {
            Error::Engine("zrok no devolvió ninguna dirección".into())
        })?;

        if let Ok(mut live) = self.live.lock() {
            live.insert(folder_id.to_string(), Link { url: url.clone(), child });
        }
        Ok(url)
    }

    /// Stops serving a folder. The link dies with the process, which is the
    /// point: there is nothing left running to reach.
    pub async fn stop(&self, folder_id: &str) -> Result<()> {
        let taken = self.live.lock().ok().and_then(|mut live| live.remove(folder_id));
        if let Some(mut link) = taken {
            let _ = link.child.start_kill();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), link.child.wait()).await;
        }
        Ok(())
    }

    /// Takes every link down. Called when the app is closing: a tunnel that
    /// outlives the window is one nobody can see or stop.
    pub async fn stop_all(&self) {
        let ids: Vec<String> = self
            .live
            .lock()
            .map(|live| live.keys().cloned().collect())
            .unwrap_or_default();
        for id in ids {
            let _ = self.stop(&id).await;
        }
    }

    /// The address a folder is being served at, if it is.
    pub fn url_for(&self, folder_id: &str) -> Option<String> {
        let mut live = self.live.lock().ok()?;
        // A helper that died takes its link with it: reporting a URL that
        // answers nothing is worse than reporting none.
        let dead = live
            .get_mut(folder_id)
            .map(|link| matches!(link.child.try_wait(), Ok(Some(_))))
            .unwrap_or(false);
        if dead {
            live.remove(folder_id);
            return None;
        }
        live.get(folder_id).map(|link| link.url.clone())
    }

    /// Runs the helper for one of its short commands and returns its output.
    async fn run(&self, args: &[&str]) -> Result<String> {
        let output = Command::new(&self.binary)
            .args(args)
            .env("ZROK_HOME", &self.home)
            .output()
            .await
            .map_err(|e| Error::Engine(format!("no se pudo ejecutar el enlace: {e}")))?;

        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
        // The helper reports failures as JSON on stderr, which is already
        // phrased for a person; anything else is passed through as it came.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = serde_json::from_str::<Announcement>(stderr.trim())
            .ok()
            .and_then(|a| a.error)
            .unwrap_or_else(|| stderr.trim().to_string());
        Err(Error::Engine(if message.is_empty() {
            "zrok falló sin decir por qué".into()
        } else {
            message
        }))
    }

    /// Whatever the helper managed to say before failing to announce a share.
    async fn explain(&self, child: &mut Child) -> String {
        let _ = child.start_kill();
        let Some(stderr) = child.stderr.take() else {
            return "zrok no devolvió ninguna dirección".into();
        };
        let mut lines = BufReader::new(stderr).lines();
        let said = tokio::time::timeout(std::time::Duration::from_secs(2), lines.next_line())
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
                    "zrok no devolvió ninguna dirección".into()
                } else {
                    said
                }
            })
    }
}

/// The helper binary, beside the sync engine it ships next to.
pub fn link_binary(resource_dir: Option<&Path>) -> Result<PathBuf> {
    let name = if cfg!(target_os = "windows") { "hcshare.exe" } else { "hcshare" };
    let candidate = resource_dir
        .map(|dir| dir.join(name))
        .filter(|path| path.exists())
        .ok_or_else(|| Error::Engine(format!("{name} no viene en esta instalación")))?;
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_helper_is_reported_rather_than_guessed_at() {
        let empty = std::env::temp_dir().join("homecloud-no-helper-here");
        let error = link_binary(Some(&empty)).unwrap_err().to_string();
        assert!(error.contains("hcshare"), "{error}");
    }

    #[test]
    fn a_folder_that_is_not_being_served_has_no_link() {
        let links = Links::new(PathBuf::from("/nonexistent"), std::env::temp_dir());
        assert_eq!(links.url_for("cualquiera"), None);
    }

    /// The announcement is the contract between the helper and this module, so
    /// both shapes it can take are pinned here.
    #[test]
    fn the_helpers_announcement_is_read_either_way() {
        let good: Announcement =
            serde_json::from_str(r#"{"url":"https://x.share.zrok.io","token":"abc"}"#).unwrap();
        assert_eq!(good.url.as_deref(), Some("https://x.share.zrok.io"));
        assert!(good.error.is_none());

        let bad: Announcement = serde_json::from_str(r#"{"error":"no está habilitado"}"#).unwrap();
        assert_eq!(bad.error.as_deref(), Some("no está habilitado"));
        assert!(bad.url.is_none());
    }
}
