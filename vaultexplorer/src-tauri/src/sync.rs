//! Cloud folder pairing + sync, on top of `rclone` (see `rclone.rs` for
//! the connection/auth side, and the list of supported providers). A
//! "pair" links a local folder to `vaultexplorer-<provider>:VaultExplorer/
//! <name>`; syncing runs `rclone bisync`, a real two-way sync with change
//! detection on both sides -- a strictly better guarantee than this
//! file's original naive last-write-wins push+pull against the raw Drive
//! REST API (this module used to be Drive-only).

use crate::errmap::{LockExt, ToStringErr};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

// Only reached if the watcher itself never fires (or the last real event
// was long ago) -- a safety net, and the only way remote-side changes
// (made from another device, not through this local folder) ever get
// picked up, since nothing local ever touches those files to trigger the
// watcher. Longer than local_sync's 60s: this hits a real cloud API on
// every tick rather than a local `unison` process, so a much chattier
// default would mean needless network/API load for every paired folder,
// every tick, for the whole time the app is open.
#[cfg(desktop)]
const SAFETY_NET_INTERVAL: Duration = Duration::from_secs(300);
#[cfg(desktop)]
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(800);

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

/// A verbose rclone log line that only appears when a real file operation
/// happened this pass -- never on a no-op pass (both sides already match),
/// which logs only listing/summary lines plus a `0 B / 0 B` stats line.
/// Used to light the sync badge ONLY during genuine transfer, so it stops
/// spinning over nothing between real changes.
fn is_transfer_activity(line: &str) -> bool {
    line.contains(": Copied (")
        || line.contains(": Deleted")
        || line.contains(": Updated")
        || line.contains(": Moved")
        || line.contains(": Renamed")
}

fn run_bisync(local_path: &str, remote: &str, resync: bool) -> Result<String, String> {
    // `--create-empty-src-dirs`: rclone otherwise ignores empty directories,
    // so an empty folder (e.g. `.../test`) never propagates. (Requires
    // rclone >= 1.64 for bisync; the app ships/expects a current rclone.)
    let mut args = vec![
        "bisync".to_string(),
        local_path.to_string(),
        remote.to_string(),
        "--create-empty-src-dirs".to_string(),
        // rclone >= 1.64 writes a bisync lock that, by default, NEVER expires
        // -- so a run killed mid-sync (app quit, machine sleep, a `pkexec
        // dpkg -i` restart) leaves a lock valid for ~200 years that wedges
        // every future run ("prior lock file found", Drive sync failed).
        // --max-lock caps it: bisync auto-renews while alive, and a dead
        // run's lock self-expires after this, so sync recovers on its own.
        // 2m is rclone's documented minimum.
        "--max-lock".to_string(),
        "2m".to_string(),
        "--stats-one-line".to_string(),
        "-v".to_string(),
    ];
    if resync {
        args.push("--resync".to_string());
    }
    let mut child = Command::new("rclone")
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .str_err()?;
    // Stream stderr (where rclone -v logs per-file ops) instead of blocking
    // on .output(): mark the pair "syncing" only once an actual transfer
    // line appears, and clear it when the process exits. A pure no-op pass
    // never inserts the path, so the badge never spins for it. Everything is
    // still captured for error reporting on a non-zero exit.
    let mut captured = String::new();
    let mut marked = false;
    if let Some(stderr) = child.stderr.take() {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if !marked && is_transfer_activity(&line) {
                marked = true;
                syncing_set().lock_safe().insert(local_path.to_string());
            }
            captured.push_str(&line);
            captured.push('\n');
        }
    }
    let status = child.wait().str_err()?;
    if marked {
        syncing_set().lock_safe().remove(local_path);
    }
    let captured = captured.trim().to_string();
    if !status.success() {
        return Err(captured);
    }
    Ok(captured)
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
    // NB: the "syncing" badge flag is managed inside run_bisync now, keyed
    // off actual transfer activity -- not blanket-set for the whole pass, so
    // no-op passes don't light it. (Cleared there on process exit too.)
    let result = sync_now_inner(pair, &remote);
    match &result {
        Ok(_) => {
            last_error_map().lock_safe().remove(&pair.local_path);
        }
        Err(e) => {
            last_error_map().lock_safe().insert(pair.local_path.clone(), e.clone());
        }
    }
    result
}

fn syncing_set() -> &'static Mutex<std::collections::HashSet<String>> {
    static SYNCING: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    SYNCING.get_or_init(Default::default)
}

/// The most recent `sync_now` failure for a given pair, if the last
/// attempt didn't succeed -- cleared on the next successful run. Exists
/// because the background loop's own retries are all best-effort (`let _
/// =`, same as git/local sync's loops): without this, a pair stuck
/// failing every tick (e.g. rclone's own "too many deletes" safety abort,
/// which isn't self-healing like the stale-cache case above) would fail
/// silently forever with no way for the UI to ever surface it.
fn last_error_map() -> &'static Mutex<HashMap<String, String>> {
    static LAST_ERROR: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    LAST_ERROR.get_or_init(Default::default)
}

/// Every Drive pair mid-`bisync` right now -- exposed the same way as the
/// other sync types' live status so the grid badge can show the same
/// "syncing…" indicator consistently.
pub fn syncing_now() -> Vec<String> {
    syncing_set().lock_safe().iter().cloned().collect()
}

#[tauri::command]
pub fn drive_sync_last_error(local_path: String) -> Option<String> {
    last_error_map().lock_safe().get(&local_path).cloned()
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
        // A prior run that got killed mid-sync (app force-quit, a `pkexec
        // dpkg -i` upgrade restarting the process, the machine losing
        // power) leaves this lock file behind -- unlike a genuinely
        // concurrent bisync, there's no live process to actually be
        // holding it, so it'd otherwise wedge this pair permanently
        // (every future tick hitting the exact same error forever, since
        // nothing else ever removes the file). Only handled for our own
        // per-pair background loop / manual button, i.e. best-effort
        // same as the resync-retry case above -- a real second `rclone`
        // process racing this one for the same pair is an edge case this
        // doesn't try to guard against.
        Err(e) if stale_lock_path(&e).is_some() => {
            let lock_path = stale_lock_path(&e).unwrap();
            let _ = std::fs::remove_file(&lock_path);
            let stderr = run_bisync(&pair.local_path, remote, !pair.resynced)?;
            if !pair.resynced {
                mark_resynced(&pair.local_path)?;
            }
            Ok(SyncReport { summary: extract_summary(&stderr) })
        }
        Err(e) => Err(e),
    }
}

/// Strip ANSI escape sequences (e.g. `\x1b[93m`) -- rclone can colorize its
/// error output, which would otherwise embed escape codes into a parsed path.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Consume up to and including the terminating letter of the seq.
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Extracts the path out of rclone's "prior lock file found: <path>" error,
/// if that's what this is. rclone >= 1.64 trails extra "Tip:" text after the
/// path and may wrap it in ANSI color, so strip color and take just the
/// first whitespace-delimited token after the marker (the earlier naive
/// `.trim()` grabbed the color codes + tip text and produced a bogus path,
/// so the self-heal silently never deleted the real lock).
fn stale_lock_path(err: &str) -> Option<String> {
    err.lines().find_map(|l| {
        let (_, rest) = l.split_once("prior lock file found: ")?;
        let path = strip_ansi(rest).split_whitespace().next()?.to_string();
        if path.is_empty() {
            None
        } else {
            Some(path)
        }
    })
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

/// Background per-pair watch loops, one per paired local folder -- same
/// role as `local_sync::LocalSyncState`/`git_sync::GitSyncState`. Doesn't
/// need its own "syncing right now" set: `sync_now` (shared by this loop
/// and the manual "Sync Now" button) already maintains that in the
/// module-level `syncing_set()`, so both triggers show up identically.
#[derive(Default)]
pub struct DriveSyncState {
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

/// Watch `pair.local_path` and run a `sync_now` pass on every real
/// filesystem change (debounced), falling back to `SAFETY_NET_INTERVAL`
/// polling underneath -- the only way a change made on the *remote* side
/// (from another device) ever gets pulled in, since nothing local touches
/// those files to trigger the watcher. Re-reads the pair from disk each
/// tick rather than closing over a stale copy, so a `resynced` flag this
/// same loop just set (or the pair being removed entirely) is always
/// picked up next time. Syncs once immediately on start, rather than only
/// after the first interval/event -- otherwise a freshly-added pair (or
/// one resumed at app launch) would sit unsynced for up to five minutes
/// with nothing having changed locally to wake it.
#[cfg(desktop)]
pub fn start_loop(state: &DriveSyncState, pair: SyncPair) {
    let key = pair.local_path.clone();
    let mut active = state.active.lock_safe();
    if active.contains_key(&key) {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    active.insert(key.clone(), stop.clone());
    drop(active);

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        if let Ok(mut debouncer) =
            notify_debouncer_mini::new_debouncer(DEBOUNCE_WINDOW, tx.clone())
        {
            let _ = debouncer
                .watcher()
                .watch(Path::new(&key), notify_debouncer_mini::notify::RecursiveMode::Recursive);
            // Leaked deliberately: this thread runs for the pair's whole
            // lifetime (which can span the whole app session), and the
            // debouncer must outlive it or the watch stops.
            std::mem::forget(debouncer);
        }
        let _ = sync_now(&pair);
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let _ = rx.recv_timeout(SAFETY_NET_INTERVAL);
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let Some(fresh) = list_pairs().into_iter().find(|p| p.local_path == key) else {
                break; // pair was removed from under us
            };
            let _ = sync_now(&fresh);
        }
    });
}

#[cfg(desktop)]
pub fn stop_loop(state: &DriveSyncState, local_path: &str) {
    if let Some(stop) = state.active.lock_safe().remove(local_path) {
        stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(desktop)]
pub fn is_active(state: &DriveSyncState, local_path: &str) -> bool {
    state.active.lock_safe().contains_key(local_path)
}

// ---- Tauri commands (Drive pairing/one-shot sync) ----

#[tauri::command]
pub fn drive_list_pairs() -> Vec<SyncPair> {
    list_pairs()
}

#[tauri::command]
pub async fn drive_add_pair(
    #[cfg(desktop)] state: tauri::State<'_, DriveSyncState>,
    provider: String,
    local_path: String,
) -> Result<SyncPair, String> {
    let pair = tokio::task::spawn_blocking(move || add_pair(provider, local_path)).await.str_err()??;
    #[cfg(desktop)]
    start_loop(&state, pair.clone());
    Ok(pair)
}

#[tauri::command]
pub fn drive_remove_pair(
    #[cfg(desktop)] state: tauri::State<DriveSyncState>,
    local_path: String,
) -> Result<(), String> {
    #[cfg(desktop)]
    stop_loop(&state, &local_path);
    remove_pair(&local_path)
}

#[cfg(desktop)]
#[tauri::command]
pub fn drive_sync_is_active(state: tauri::State<DriveSyncState>, local_path: String) -> bool {
    is_active(&state, &local_path)
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
