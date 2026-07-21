//! Cloud folder pairing + sync, on top of `rclone` (see `rclone.rs` for
//! the connection/auth side, and the list of supported providers). A
//! "pair" links a local folder to `vaultexplorer-<provider>:VaultExplorer/
//! <name>`; syncing runs `rclone bisync`, a real two-way sync with change
//! detection on both sides -- a strictly better guarantee than this
//! file's original naive last-write-wins push+pull against the raw Drive
//! REST API (this module used to be Drive-only).

use crate::errmap::{LockExt, ToStringErr};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}

fn config_dir() -> PathBuf {
    Path::new(&home_dir()).join(".config/vaultexplorer")
}

fn pairs_path() -> PathBuf {
    config_dir().join("sync_pairs.json")
}

fn default_provider() -> String {
    "drive".to_string()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SyncPair {
    pub local_path: String,
    /// The rclone backend/provider this pair syncs to ("drive", "onedrive",
    /// "dropbox", ...). Defaults to "drive" when missing so pairs saved by
    /// this app before it supported more than one provider still load.
    #[serde(default = "default_provider")]
    pub provider: String,
    pub drive_folder_name: String,
    /// `bisync` needs an initial `--resync` run to establish its baseline
    /// listings on both sides before it can do incremental two-way runs --
    /// tracked here so `sync_now` only pays that cost once per pair.
    #[serde(default)]
    pub resynced: bool,
}

pub fn list_pairs() -> Vec<SyncPair> {
    std::fs::read_to_string(pairs_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_pairs(pairs: &[SyncPair]) -> Result<(), String> {
    std::fs::create_dir_all(config_dir()).str_err()?;
    let json = serde_json::to_string_pretty(pairs).str_err()?;
    std::fs::write(pairs_path(), json).str_err()
}

fn remote_path(provider: &str, name: &str) -> String {
    format!("{}:VaultExplorer/{name}", crate::rclone::remote_name(provider))
}

/// Link `local_path` to a same-named folder under `provider`'s
/// `VaultExplorer/` root, creating the remote folder if it doesn't exist
/// yet.
pub fn add_pair(provider: String, local_path: String) -> Result<SyncPair, String> {
    let mut pairs = list_pairs();
    if pairs.iter().any(|p| p.local_path == local_path) {
        return Err("this folder is already linked".to_string());
    }
    let name = Path::new(&local_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Synced Folder".to_string());

    let output = Command::new("rclone")
        .args(["mkdir", &remote_path(&provider, &name)])
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let pair = SyncPair { local_path, provider, drive_folder_name: name, resynced: false };
    pairs.push(pair.clone());
    save_pairs(&pairs)?;
    Ok(pair)
}

pub fn remove_pair(local_path: &str) -> Result<(), String> {
    let mut pairs = list_pairs();
    pairs.retain(|p| p.local_path != local_path);
    save_pairs(&pairs)
}

#[derive(Serialize)]
pub struct SyncReport {
    /// `rclone`'s own one-line stats summary (`--stats-one-line`) --
    /// surfaced close to verbatim rather than re-parsed into separate
    /// counters, the same "show the real tool's own output" approach
    /// `local_sync.rs` already takes with Unison's conflict lines.
    pub summary: String,
}

fn run_bisync(local_path: &str, remote: &str, resync: bool) -> Result<String, String> {
    let mut args = vec!["bisync".to_string(), local_path.to_string(), remote.to_string(), "--stats-one-line".to_string(), "-v".to_string()];
    if resync {
        args.push("--resync".to_string());
    }
    let output = Command::new("rclone").args(&args).output().str_err()?;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(stderr);
    }
    Ok(stderr)
}

fn mark_resynced(local_path: &str) -> Result<(), String> {
    let mut pairs = list_pairs();
    if let Some(p) = pairs.iter_mut().find(|p| p.local_path == local_path) {
        p.resynced = true;
    }
    save_pairs(&pairs)
}

/// Run one `bisync` pass for `pair`. First run for a given pair does
/// `--resync` (establishes the baseline listing on both sides; Path1
/// wins any conflicts that round only), every run after that is a normal
/// incremental two-way sync.
///
/// Self-healing: `bisync` keeps its own baseline-listing cache under
/// `~/.cache/rclone/bisync/` independent of this app's own `resynced`
/// bookkeeping in `sync_pairs.json` -- if those two ever disagree (the
/// cache dir gets cleared, a prior `--resync` run didn't actually
/// complete, etc.) `bisync` fails with its own distinctive "empty prior
/// listing ... must run --resync to recover" error instead of silently
/// doing the wrong thing. Recognizing that specific message and
/// transparently retrying with `--resync` means a `sync_pairs.json`/
/// bisync-cache mismatch fixes itself on the next click instead of
/// leaving the pair permanently stuck failing.
pub fn sync_now(pair: &SyncPair) -> Result<SyncReport, String> {
    let remote = remote_path(&pair.provider, &pair.drive_folder_name);
    syncing_set().lock_safe().insert(pair.local_path.clone());
    let result = sync_now_inner(pair, &remote);
    syncing_set().lock_safe().remove(&pair.local_path);
    result
}

fn syncing_set() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static SYNCING: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    SYNCING.get_or_init(Default::default)
}

/// Every Drive pair mid-`bisync` right now -- manual-trigger-only (no
/// background loop, unlike git/local sync), so this is really just "is
/// the in-flight `sync_now` call for this path still running", but
/// exposed the same way as the other sync types' live status so the grid
/// badge can show the same "syncing…" indicator consistently.
pub fn syncing_now() -> Vec<String> {
    syncing_set().lock_safe().iter().cloned().collect()
}

fn sync_now_inner(pair: &SyncPair, remote: &str) -> Result<SyncReport, String> {
    match run_bisync(&pair.local_path, remote, !pair.resynced) {
        Ok(stderr) => {
            if !pair.resynced {
                mark_resynced(&pair.local_path)?;
            }
            Ok(SyncReport { summary: extract_summary(&stderr) })
        }
        Err(e) if needs_resync_retry(&e) => {
            let stderr = run_bisync(&pair.local_path, remote, true)?;
            mark_resynced(&pair.local_path)?;
            Ok(SyncReport { summary: extract_summary(&stderr) })
        }
        Err(e) => Err(e),
    }
}

/// `bisync`'s own critical-error wording for "my cached baseline listing
/// and reality have diverged" ("Bisync aborted. Must run --resync to
/// recover.") -- checked case-insensitively (rclone's actual casing is
/// "Must", capitalized, which a first version of this check missed
/// entirely by comparing against a lowercase-only literal). Deliberately
/// not gated on this app's own `resynced` bookkeeping: that flag can only
/// ever be a best-effort guess about `bisync`'s *own* cache state, so
/// trusting `bisync`'s own error over it is what makes this self-healing
/// regardless of how the two ever fell out of sync.
fn needs_resync_retry(err: &str) -> bool {
    err.to_lowercase().contains("run --resync to recover")
}

/// `rclone`'s own `--stats-one-line` summary -- surfaced close to
/// verbatim rather than re-parsed into separate counters, the same
/// "show the real tool's own output" approach `local_sync.rs` already
/// takes with Unison's conflict lines.
fn extract_summary(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .find(|l| l.contains("Transferred:") || l.contains("Elapsed time"))
        .unwrap_or("Synced.")
        .trim()
        .to_string()
}

// ---- Tauri commands (Drive pairing/one-shot sync) ----

#[tauri::command]
pub fn drive_list_pairs() -> Vec<SyncPair> {
    list_pairs()
}

#[tauri::command]
pub async fn drive_add_pair(provider: String, local_path: String) -> Result<SyncPair, String> {
    tokio::task::spawn_blocking(move || add_pair(provider, local_path)).await.str_err()?
}

#[tauri::command]
pub fn drive_remove_pair(local_path: String) -> Result<(), String> {
    remove_pair(&local_path)
}

#[tauri::command]
pub async fn drive_sync_now(local_path: String) -> Result<SyncReport, String> {
    tokio::task::spawn_blocking(move || {
        let pairs = list_pairs();
        let pair = pairs
            .into_iter()
            .find(|p| p.local_path == local_path)
            .ok_or("this folder isn't linked to Drive")?;
        sync_now(&pair)
    })
    .await
    .str_err()?
}

#[tauri::command]
pub fn drive_syncing_now() -> Vec<String> {
    syncing_now()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_path_nests_under_the_vaultexplorer_drive_root() {
        assert_eq!(
            remote_path("drive", "Photos"),
            format!("{}:VaultExplorer/Photos", crate::rclone::remote_name("drive"))
        );
    }

    #[test]
    fn remote_path_scopes_by_provider() {
        assert_eq!(remote_path("onedrive", "Photos"), "vaultexplorer-onedrive:VaultExplorer/Photos");
        assert_eq!(remote_path("dropbox", "Photos"), "vaultexplorer-dropbox:VaultExplorer/Photos");
    }

    #[test]
    fn extract_summary_finds_the_stats_line_among_log_noise() {
        let stderr = "2026/07/19 INFO: some log noise\nTransferred: 3 files, 12 KiB, Elapsed time: 1.2s\nmore noise\n";
        assert_eq!(extract_summary(stderr), "Transferred: 3 files, 12 KiB, Elapsed time: 1.2s");
    }

    #[test]
    fn extract_summary_falls_back_when_no_stats_line_is_present() {
        assert_eq!(extract_summary("nothing useful here\n"), "Synced.");
    }

    /// Real `rclone` output uses "Must", capitalized -- an earlier version
    /// of this check compared against an all-lowercase literal and so
    /// silently never matched real output at all, defeating the whole
    /// point of the retry.
    #[test]
    fn needs_resync_retry_matches_rclones_real_capitalization() {
        let real = "2026/07/19 14:19:45 ERROR : Bisync aborted. Must run --resync to recover.";
        assert!(needs_resync_retry(real), "must match rclone's actual (capitalized) wording: {real}");
    }

    #[test]
    fn needs_resync_retry_is_false_for_unrelated_errors() {
        assert!(!needs_resync_retry("some other rclone failure"));
    }
}
