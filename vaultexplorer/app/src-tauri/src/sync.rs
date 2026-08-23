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
/// After a watch event fires, keep draining further events until the
/// folder has been quiet for this long before actually syncing. The
/// debouncer alone isn't enough: a write burst longer than its window
/// (e.g. gocryptfs rewriting ciphertext blocks while a file is being
/// saved inside a mounted vault) still delivers an event mid-burst, and
/// a bisync started then reads files that change under it -- rclone
/// aborts the whole run with "corrupted on transfer: md5 hashes differ".
#[cfg(desktop)]
const QUIESCENCE_WINDOW: Duration = Duration::from_secs(2);

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}

pub(crate) fn config_dir() -> PathBuf {
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

/// The file a transfer line is about: rclone -v logs `... INFO  : <path>:
/// Copied (new)`, so the path sits between the log prefix's " : " and the
/// ": <verb>" marker. Returns the pair-relative (ciphertext) path.
fn transfer_line_path(line: &str) -> Option<String> {
    let marker = [": Copied (", ": Deleted", ": Updated", ": Moved", ": Renamed"]
        .iter()
        .find_map(|m| line.find(m))?;
    let head = &line[..marker];
    let (_, path) = head.rsplit_once(" : ")?;
    if path.is_empty() {
        return None;
    }
    Some(path.to_string())
}

/// What each mid-`bisync` pair is transferring right now: the last file a
/// transfer line mentioned (pair-relative ciphertext path) plus how many
/// files this pass has touched -- feeds the bottom-right task's label.
pub struct SyncActivity {
    pub current: String,
    pub count: u64,
}

fn activity_map() -> &'static Mutex<HashMap<String, SyncActivity>> {
    static ACTIVITY: OnceLock<Mutex<HashMap<String, SyncActivity>>> = OnceLock::new();
    ACTIVITY.get_or_init(Default::default)
}

/// `--create-empty-src-dirs` and `--max-lock` are both rclone >= 1.64
/// bisync flags; older system-installed rclones (seen: 1.60.1) reject them
/// outright with "unknown flag", failing every sync. Detected once via
/// `rclone version` and cached, so this degrades gracefully on an old
/// rclone instead of breaking sync entirely.
fn rclone_supports_modern_bisync_flags() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let Ok(output) = Command::new("rclone").arg("version").output() else {
            return false;
        };
        let Some(first_line) = String::from_utf8_lossy(&output.stdout).lines().next().map(str::to_string) else {
            return false;
        };
        let Some(version) = first_line.split_whitespace().nth(1) else {
            return false;
        };
        let version = version.trim_start_matches('v');
        let mut parts = version.split('.').filter_map(|p| p.parse::<u32>().ok());
        let (Some(major), Some(minor)) = (parts.next(), parts.next()) else {
            return false;
        };
        (major, minor) >= (1, 64)
    })
}

/// On success returns rclone's captured log plus whether this pass did any
/// real transfer work (the same signal that lights the syncing badge) --
/// `sync_now` uses that to decide when re-verifying against the cloud is
/// worth an `rclone check`.
fn run_bisync(local_path: &str, remote: &str, resync: bool) -> Result<(String, bool), String> {
    let mut args = vec!["bisync".to_string(), local_path.to_string(), remote.to_string()];
    if rclone_supports_modern_bisync_flags() {
        // `--create-empty-src-dirs`: rclone otherwise ignores empty
        // directories, so an empty folder (e.g. `.../test`) never propagates.
        args.push("--create-empty-src-dirs".to_string());
        // Bisync writes a lock that, by default, NEVER expires -- so a run
        // killed mid-sync (app quit, machine sleep, a `pkexec dpkg -i`
        // restart) leaves a lock valid for ~200 years that wedges every
        // future run ("prior lock file found", Drive sync failed). --max-lock
        // caps it: bisync auto-renews while alive, and a dead run's lock
        // self-expires after this, so sync recovers on its own. 2m is
        // rclone's documented minimum.
        args.push("--max-lock".to_string());
        args.push("2m".to_string());
    }
    args.push("--stats-one-line".to_string());
    args.push("-v".to_string());
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
            if is_transfer_activity(&line) {
                if !marked {
                    marked = true;
                    syncing_set().lock_safe().insert(local_path.to_string());
                }
                if let Some(path) = transfer_line_path(&line) {
                    let mut activity = activity_map().lock_safe();
                    let entry = activity
                        .entry(local_path.to_string())
                        .or_insert(SyncActivity { current: String::new(), count: 0 });
                    entry.current = path;
                    entry.count += 1;
                }
            }
            captured.push_str(&line);
            captured.push('\n');
        }
    }
    let status = child.wait().str_err()?;
    if marked {
        syncing_set().lock_safe().remove(local_path);
        activity_map().lock_safe().remove(local_path);
    }
    let captured = captured.trim().to_string();
    if !status.success() {
        return Err(captured);
    }
    Ok((captured, marked))
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
    match sync_now_inner(pair, &remote) {
        Ok((report, transferred)) => {
            last_error_map().lock_safe().remove(&pair.local_path);
            // Re-checksum against the cloud only when this pass actually
            // moved something (or nothing was ever verified) -- a no-op
            // safety-net tick shouldn't re-hit the provider every 5 min.
            if transferred || !crate::verify::has_result(&pair.local_path) {
                crate::verify::verify_pair(&pair.local_path, &remote);
            }
            Ok(report)
        }
        Err(e) => {
            last_error_map().lock_safe().insert(pair.local_path.clone(), e.clone());
            Err(e)
        }
    }
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

fn sync_now_inner(pair: &SyncPair, remote: &str) -> Result<(SyncReport, bool), String> {
    match run_bisync(&pair.local_path, remote, !pair.resynced) {
        Ok((stderr, transferred)) => {
            if !pair.resynced {
                mark_resynced(&pair.local_path)?;
            }
            Ok((SyncReport { summary: extract_summary(&stderr) }, transferred))
        }
        Err(e) if needs_resync_retry(&e) => {
            // Two attempts, not one: the abort that lands here can be the
            // transient "corrupted on transfer: md5 hashes differ" case (a
            // file -- typically gocryptfs ciphertext under a mounted vault
            // -- changed while rclone was mid-copy). Retrying instantly
            // against a still-moving source just fails the same way, so
            // corruption aborts wait out the write burst first; the plain
            // stale-baseline case keeps its original immediate retry.
            let mut last = e;
            for delay in [Duration::from_secs(5), Duration::from_secs(20)] {
                if is_corruption_error(&last) {
                    std::thread::sleep(delay);
                }
                match run_bisync(&pair.local_path, remote, true) {
                    Ok((stderr, transferred)) => {
                        mark_resynced(&pair.local_path)?;
                        return Ok((SyncReport { summary: extract_summary(&stderr) }, transferred));
                    }
                    Err(e2) if needs_resync_retry(&e2) || is_corruption_error(&e2) => last = e2,
                    Err(e2) => return Err(e2),
                }
            }
            Err(last)
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
            let (stderr, transferred) = run_bisync(&pair.local_path, remote, !pair.resynced)?;
            if !pair.resynced {
                mark_resynced(&pair.local_path)?;
            }
            Ok((SyncReport { summary: extract_summary(&stderr) }, transferred))
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

/// rclone's "corrupted on transfer: md5 hashes differ" abort -- in this
/// app's setup it virtually always means the source file changed while
/// rclone was mid-copy (gocryptfs rewriting ciphertext under a mounted
/// vault), not real corruption, so it's worth retrying after the write
/// burst has settled rather than surfacing straight to the user.
fn is_corruption_error(err: &str) -> bool {
    err.to_lowercase().contains("corrupted on transfer")
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
            let woke_by_event = rx.recv_timeout(SAFETY_NET_INTERVAL).is_ok();
            if stop.load(Ordering::Relaxed) {
                break;
            }
            // A watch event may land mid-write-burst (see QUIESCENCE_WINDOW)
            // -- drain follow-up events until the folder goes quiet so
            // bisync never reads files still being rewritten under it.
            if woke_by_event {
                while rx.recv_timeout(QUIESCENCE_WINDOW).is_ok() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                }
                if stop.load(Ordering::Relaxed) {
                    break;
                }
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

// `state` is unconditional even though only the desktop watch-loop calls
// use it: `#[tauri::command]`'s handler-generation macro doesn't reconcile
// a per-parameter `#[cfg(desktop)]` against a call site that's compiled
// for every platform (the codegen'd wrapper ends up with a fixed arity
// that mismatches the cfg-stripped signature on mobile) -- it needs the
// mobile-target vs desktop-target arg count to actually agree; `DriveSyncState`
// itself is `.manage()`d unconditionally in `lib.rs`, so taking it here on
// mobile is free, just unused (that half of the call stays desktop-only).
#[tauri::command]
pub async fn drive_add_pair(
    state: tauri::State<'_, DriveSyncState>,
    provider: String,
    local_path: String,
) -> Result<SyncPair, String> {
    let pair = tokio::task::spawn_blocking(move || add_pair(provider, local_path)).await.str_err()??;
    #[cfg(desktop)]
    start_loop(&state, pair.clone());
    #[cfg(mobile)]
    let _ = &state;
    Ok(pair)
}

#[tauri::command]
pub fn drive_remove_pair(state: tauri::State<DriveSyncState>, local_path: String) -> Result<(), String> {
    #[cfg(desktop)]
    stop_loop(&state, &local_path);
    #[cfg(mobile)]
    let _ = &state;
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

#[derive(Serialize)]
pub struct SyncActivityDto {
    /// Human-readable name of the file being transferred: decrypted when it
    /// sits inside a currently-unlocked vault, the on-disk name for a plain
    /// file, `None` when it's ciphertext we can't (or shouldn't) name.
    pub current: Option<String>,
    pub count: u64,
}

/// Any ancestor of `cipher_abs` (up to the pair root) being a vault means
/// the on-disk name is ciphertext -- never worth showing raw.
fn inside_vault_ciphertext(pair_root: &Path, cipher_abs: &Path) -> bool {
    let mut dir = cipher_abs.parent();
    while let Some(d) = dir {
        if d.join(".vault.meta").exists() {
            return true;
        }
        if d == pair_root {
            break;
        }
        dir = d.parent();
    }
    false
}

/// Per-pair live transfer detail for the bottom-right task label, keyed by
/// the pair's local path.
#[tauri::command]
pub fn drive_sync_activity(
    state: tauri::State<crate::AppState>,
) -> HashMap<String, SyncActivityDto> {
    let activity = activity_map().lock_safe();
    let vaults = state.vaults.lock_safe();
    activity
        .iter()
        .map(|(pair_path, act)| {
            let cipher_abs = Path::new(pair_path).join(&act.current);
            let decrypted = vaults.values().find_map(|session| {
                let rel = cipher_abs.strip_prefix(session.vault.root()).ok()?;
                session.vault.decrypt_rel_path(rel).ok()
            });
            let current = decrypted
                .map(|p| p.to_string_lossy().to_string())
                .or_else(|| {
                    if act.current.is_empty()
                        || inside_vault_ciphertext(Path::new(pair_path), &cipher_abs)
                    {
                        None
                    } else {
                        Some(act.current.clone())
                    }
                });
            (pair_path.clone(), SyncActivityDto { current, count: act.count })
        })
        .collect()
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

    #[test]
    fn transfer_line_path_extracts_the_file_from_real_rclone_lines() {
        let copied = "2026/08/03 22:10:01 INFO  : personal/aBrMm/jpgyOBwZ: Copied (new)";
        assert_eq!(transfer_line_path(copied), Some("personal/aBrMm/jpgyOBwZ".to_string()));
        let deleted = "2026/08/03 22:10:02 INFO  : old/file.bin: Deleted";
        assert_eq!(transfer_line_path(deleted), Some("old/file.bin".to_string()));
        assert_eq!(transfer_line_path("Transferred: 3 files, 12 KiB"), None);
    }

    /// Real wording from an rclone 1.74 bisync abort when a source file
    /// changed mid-copy (gocryptfs ciphertext being rewritten while the
    /// vault was mounted).
    #[test]
    fn is_corruption_error_matches_rclones_md5_mismatch_abort() {
        let real = "2026/08/03 00:01:29 ERROR : personal/4WSGkWfIWC89: corrupted on transfer: md5 hashes differ src(Local file system at /home/linux/Documents/cloud/google_drive) \"eb01f3f6f5b68984f5a96cf97a9e99f5\" vs dst(Google drive root 'VaultExplorer/google_drive') \"1ae9bd45a03fdbdf6942c7a2e46e9d7a\"";
        assert!(is_corruption_error(real));
        assert!(!is_corruption_error("some other rclone failure"));
    }
}
