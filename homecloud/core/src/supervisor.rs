//! Owning the Syncthing process.
//!
//! HomeCloud ships the engine rather than asking the user to install it, so the
//! app is responsible for starting it, talking to it privately, and taking it
//! down again. The API key is minted fresh on every launch and passed on the
//! command line, so nothing long-lived sits on disk for another local program
//! to find.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use rand::Rng;
use tokio::process::{Child, Command};

use crate::client::Syncthing;
use crate::error::{Error, Result};

pub struct Engine {
    child: Option<Child>,
    pub client: Syncthing,
    pub base_url: String,
}

/// How long the engine gets to answer before we call the launch failed. A cold
/// start on a slow disk takes a few seconds; anything past this is broken.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

impl Engine {
    /// Starts the engine, using `home` for its config and database.
    pub async fn start(binary: &Path, home: &Path) -> Result<Self> {
        if !binary.exists() {
            return Err(Error::Engine(format!(
                "the sync engine is missing from {}",
                binary.display()
            )));
        }
        std::fs::create_dir_all(home)?;

        // First run only: mint the device certificate and identity.
        if !home.join("config.xml").exists() {
            // Without credentials the engine's own web interface answers any
            // request from localhost, which would hand every other program on
            // this machine control of the user's folders. HomeCloud talks to the
            // engine with an API key instead, so nobody ever needs this password
            // and it is deliberately thrown away.
            let generated = Command::new(binary)
                .arg("generate")
                .arg("--home")
                .arg(home)
                .arg("--gui-user")
                .arg("homecloud")
                .arg("--gui-password")
                .arg(mint_api_key())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .await?;
            if !generated.status.success() {
                return Err(Error::Engine(format!(
                    "could not create this device's identity: {}",
                    String::from_utf8_lossy(&generated.stderr).trim()
                )));
            }
        }

        let port = free_port()?;
        let api_key = mint_api_key();
        let base_url = format!("http://127.0.0.1:{port}");

        let child = Command::new(binary)
            .arg("serve")
            .arg("--home")
            .arg(home)
            .arg("--gui-address")
            .arg(format!("127.0.0.1:{port}"))
            .arg("--gui-apikey")
            .arg(&api_key)
            .arg("--no-browser")
            // The engine must not replace its own binary: this one is bundled
            // with the app and, on Android, signed alongside it.
            .arg("--no-upgrade")
            .arg("--no-restart")
            .arg("--log-level")
            .arg("WARN")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::Engine(format!("could not launch the sync engine: {e}")))?;

        let client = Syncthing::new(&base_url, api_key);
        let engine = Engine { child: Some(child), client, base_url };
        engine.wait_until_ready().await?;
        engine.client.apply_house_defaults().await?;
        Ok(engine)
    }

    async fn wait_until_ready(&self) -> Result<()> {
        let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
        loop {
            if self.client.ping().await.is_ok() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Engine(
                    "the sync engine started but never answered — try restarting HomeCloud".into(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    /// Asks the engine to exit, then makes sure it did.
    pub async fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
        }
        Ok(())
    }
}

/// The name the engine binary is shipped under on this platform.
pub fn engine_file_name() -> &'static str {
    if cfg!(windows) {
        "syncthing.exe"
    } else {
        "syncthing"
    }
}

/// Finds the engine that ships with this build.
///
/// `resource_dir` is whatever the host toolkit reports as the app's resource
/// directory; it is the only reliable answer for an installed package, where
/// the executable and its resources end up far apart (`/usr/bin/homecloud`
/// against `/usr/lib/HomeCloud/resources/`). The paths after it exist so a
/// developer running out of `target/debug` gets the same engine.
///
/// A `syncthing` sitting next to the executable is deliberately NOT accepted:
/// on an installed system that is `/usr/bin/syncthing`, someone else's copy at
/// someone else's version, and silently driving it would be both surprising and
/// unsupportable.
pub fn engine_binary(resource_dir: Option<&Path>) -> Result<PathBuf> {
    let name = engine_file_name();
    let mut tried: Vec<PathBuf> = Vec::new();

    if let Some(dir) = resource_dir {
        tried.push(dir.join("resources").join(name));
        tried.push(dir.join(name));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // target/debug -> target -> src-tauri/resources
            for up in [1, 2, 3] {
                if let Some(ancestor) = dir.ancestors().nth(up) {
                    tried.push(ancestor.join("resources").join(name));
                }
            }
        }
    }

    for candidate in &tried {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }
    Err(Error::Engine(format!(
        "could not find the bundled sync engine; looked in {}",
        tried
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// Binds port zero to have the OS name a free port, then hands it back. There
/// is a race between here and the engine binding it, but the window is tiny and
/// the alternative is a hardcoded port that collides with a real Syncthing.
fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn mint_api_key() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    (0..40).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_keys_are_long_and_never_repeat() {
        let a = mint_api_key();
        let b = mint_api_key();
        assert_eq!(a.len(), 40);
        assert_ne!(a, b);
    }

    #[test]
    fn free_port_is_actually_free() {
        let port = free_port().unwrap();
        // If it were still held, binding again would fail.
        TcpListener::bind(("127.0.0.1", port)).expect("port should have been released");
    }

    #[test]
    fn engine_is_never_taken_from_beside_the_executable() {
        // /usr/bin/syncthing must not be picked up as "our" engine.
        let Err(err) = engine_binary(Some(Path::new("/definitely/not/here"))) else {
            panic!("there is no engine to find in this test");
        };
        let message = err.to_string();
        assert!(
            message.contains("/definitely/not/here"),
            "the error should name where it looked: {message}"
        );
        assert!(
            !message.contains("/usr/bin/syncthing"),
            "a system-wide Syncthing must never be a candidate: {message}"
        );
    }

    #[test]
    fn engine_is_found_in_the_resource_directory() {
        let dir = std::env::temp_dir().join(format!("homecloud-res-{}", std::process::id()));
        let resources = dir.join("resources");
        std::fs::create_dir_all(&resources).unwrap();
        let engine = resources.join(engine_file_name());
        std::fs::write(&engine, b"not really an engine").unwrap();
        assert_eq!(engine_binary(Some(&dir)).unwrap(), engine);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_missing_engine_is_reported_clearly() {
        let Err(err) = Engine::start(Path::new("/definitely/not/here"), Path::new("/tmp")).await else {
            panic!("starting a non-existent engine must fail");
        };
        assert!(err.to_string().contains("missing"), "unhelpful error: {err}");
    }
}
