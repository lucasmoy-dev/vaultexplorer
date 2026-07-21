//! Cloud sync via the `rclone` binary rather than a hand-rolled OAuth
//! client per provider. This replaces an earlier `yup-oauth2`-based
//! Google-only implementation that, across several rounds of fixes this
//! session, kept failing in ways that were hard to fully pin down
//! remotely. `rclone` ships its own long-standing, provider-registered
//! OAuth client for every backend below, so there's no "create your own
//! OAuth app in each provider's console" step at all for any of them --
//! `rclone authorize <backend>` does exactly the local-loopback-server
//! browser dance this app's own code was reimplementing, just backed by
//! a much more battle-tested implementation used by a huge number of
//! people daily.
//!
//! Every supported provider maps 1:1 to an rclone backend name and gets
//! its own remote, `vaultexplorer-<provider>`, so several can be
//! connected at once independently. Providers considered but *not*
//! included: Mega (rclone dropped its `mega` backend outright a few
//! releases back over an unmaintained upstream dependency -- there's
//! nothing safe to shell out to anymore) and Proton Drive (never had
//! official rclone support; the unofficial community forks that exist
//! aren't vetted enough to shell out to from here).
//!
//! Actual file transfer (see `sync.rs`) also shells out to `rclone`, via
//! `rclone bisync` -- a real two-way sync with change detection on both
//! sides, which is a strictly better guarantee than the naive
//! last-write-wins push+pull this app's own original Drive-only REST
//! client used to do.

use crate::errmap::ToStringErr;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};

/// `(id, display name)` for every provider this app knows how to connect
/// -- `id` doubles as the literal rclone backend name.
pub const PROVIDERS: &[(&str, &str)] =
    &[("drive", "Google Drive"), ("onedrive", "OneDrive"), ("dropbox", "Dropbox")];

pub fn remote_name(provider: &str) -> String {
    format!("vaultexplorer-{provider}")
}

pub fn is_installed() -> bool {
    Command::new("which").arg("rclone").output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn is_connected(provider: &str) -> bool {
    let Ok(output) = Command::new("rclone").arg("listremotes").output() else {
        return false;
    };
    let target = format!("{}:", remote_name(provider));
    String::from_utf8_lossy(&output.stdout).lines().any(|l| l.trim() == target)
}

pub fn disconnect(provider: &str) -> Result<(), String> {
    let output = Command::new("rclone")
        .args(["config", "delete", &remote_name(provider)])
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

/// Tries each of the usual Linux URL launchers in turn, waiting for its
/// *real* exit status rather than firing-and-forgetting (the failure mode
/// that made `tauri_plugin_opener::open_url` always report success
/// earlier this session, regardless of whether the launcher actually
/// worked). Returns the first success, or every attempt's own error.
fn launch_browser(url: &str) -> Result<(), String> {
    let candidates: &[(&str, &[&str])] = &[("xdg-open", &[]), ("gio", &["open"]), ("gnome-open", &[])];
    let mut failures = Vec::new();
    for (bin, prefix_args) in candidates {
        let mut cmd = Command::new(bin);
        cmd.args(*prefix_args).arg(url);
        match cmd.output() {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                failures.push(format!(
                    "{bin}: exited {} ({})",
                    output.status,
                    if stderr.trim().is_empty() { "no output" } else { stderr.trim() }
                ));
            }
            Err(e) => failures.push(format!("{bin}: {e}")),
        }
    }
    Err(failures.join("; "))
}

/// `rclone authorize` prints its real result (an OAuth token, as JSON)
/// between a documented pair of markers on stdout so it can be piped/
/// copy-pasted independent of its own progress logging (which goes to
/// stderr) -- see https://rclone.org/remote_setup/. Falls back to
/// scanning for a bare `{...}` line, in case a given rclone build's exact
/// banner text ever changes.
fn extract_token(stdout: &str) -> Option<String> {
    if let Some(start) = stdout.find("--->") {
        if let Some(end_rel) = stdout[start..].find("<---") {
            let blob = stdout[start + 4..start + end_rel].trim();
            if !blob.is_empty() {
                return Some(blob.to_string());
            }
        }
    }
    stdout
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with('{') && l.ends_with('}'))
        .map(str::to_string)
}

/// Runs `rclone authorize <provider>` end to end and, once it succeeds,
/// feeds the resulting token straight into a non-interactive `rclone
/// config create` -- no separate credentials step, for any provider.
/// Blocking (real child-process I/O throughout); callers on the Tauri
/// command side run this via `spawn_blocking`.
pub fn connect(provider: &str, on_url: impl Fn(String) + Send + Sync + 'static) -> Result<(), String> {
    let mut child = Command::new("rclone")
        .args(["authorize", provider])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't launch rclone: {e}"))?;

    let stderr = child.stderr.take().ok_or("rclone: no stderr pipe")?;
    let stderr_thread = std::thread::spawn(move || {
        const MARKER: &str = "go to the following link: ";
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(idx) = line.find(MARKER) {
                let url = line[idx + MARKER.len()..].trim().to_string();
                on_url(url.clone());
                if let Err(e) = launch_browser(&url) {
                    on_url(format!("LAUNCH_FAILED:{e}"));
                }
            }
        }
    });

    let mut stdout = String::new();
    child
        .stdout
        .take()
        .ok_or("rclone: no stdout pipe")?
        .read_to_string(&mut stdout)
        .str_err()?;
    let status = child.wait().str_err()?;
    let _ = stderr_thread.join();

    if !status.success() {
        return Err("rclone authorize didn't complete (cancelled, or the sign-in failed)".to_string());
    }
    let token = extract_token(&stdout).ok_or("couldn't find an auth token in rclone's output")?;

    let remote = remote_name(provider);
    let mut args = vec!["config", "create", &remote, provider, "token", &token, "--non-interactive"];
    // Scoped to just files this app itself creates, rather than full
    // Drive access -- Drive-specific, no equivalent narrow scope exists
    // for the other providers' OAuth apps.
    if provider == "drive" {
        args.extend(["scope", "drive.file"]);
    }
    let output = Command::new("rclone").args(&args).output().str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

// ---- Tauri commands ----

#[tauri::command]
pub fn rclone_installed() -> bool {
    is_installed()
}

#[tauri::command]
pub fn rclone_providers() -> Vec<(String, String)> {
    PROVIDERS.iter().map(|(id, name)| (id.to_string(), name.to_string())).collect()
}

#[tauri::command]
pub fn rclone_is_connected(provider: String) -> bool {
    is_connected(&provider)
}

#[tauri::command]
pub async fn rclone_connect(provider: String, url_channel: tauri::ipc::Channel<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        connect(&provider, move |url| {
            let _ = url_channel.send(url);
        })
    })
    .await
    .str_err()?
}

#[tauri::command]
pub fn rclone_disconnect(provider: String) -> Result<(), String> {
    disconnect(&provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_browser_reports_real_success() {
        let result = launch_browser("https://example.com/vaultexplorer-rclone-test");
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn launch_browser_reports_real_failure_for_a_bogus_scheme() {
        let result = launch_browser("bogus-scheme-nobody-handles://nope");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("xdg-open"), "expected xdg-open's failure in: {err}");
    }

    #[test]
    fn extract_token_reads_the_documented_paste_markers() {
        let stdout = "Paste the following into your remote machine --->\n{\"access_token\":\"abc\"}\n<---End paste\n";
        assert_eq!(extract_token(stdout), Some("{\"access_token\":\"abc\"}".to_string()));
    }

    #[test]
    fn extract_token_falls_back_to_a_bare_json_line() {
        let stdout = "some banner text\n{\"access_token\":\"xyz\"}\ntrailing\n";
        assert_eq!(extract_token(stdout), Some("{\"access_token\":\"xyz\"}".to_string()));
    }

    #[test]
    fn extract_token_is_none_without_any_token_shaped_line() {
        assert_eq!(extract_token("just some log lines\nnothing useful here\n"), None);
    }
}
