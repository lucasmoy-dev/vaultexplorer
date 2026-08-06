//! Continuous git sync for a favorited folder: the user supplies a name
//! and a remote they've already created (this assumes git/SSH auth is
//! already set up on the machine, same as any other git remote -- no
//! GitHub API/token involved, unlike the Drive sync path). Once linked,
//! a background loop periodically commits+pushes local changes and pulls
//! remote ones.
//!
//! That loop is a plain ~25s poll, not a filesystem watcher: a real
//! inotify-style watch would need a new dependency and its own pile of
//! edge cases (debouncing bursts of writes, ignoring editor swap/lock
//! files, coalescing renames...) for a case where "commit whatever
//! changed in the last 25 seconds" is just as good a guarantee and a lot
//! less code.

use crate::errmap::{LockExt, ToStringErr};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(25);

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}
fn config_dir() -> PathBuf {
    Path::new(&home_dir()).join(".config/vaultexplorer")
}
fn pairs_path() -> PathBuf {
    config_dir().join("git_sync_pairs.json")
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GitSyncPair {
    pub local_path: String,
    pub remote_url: String,
    pub repo_name: String,
}

pub fn list_pairs() -> Vec<GitSyncPair> {
    fs::read_to_string(pairs_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_pairs(pairs: &[GitSyncPair]) -> Result<(), String> {
    fs::create_dir_all(config_dir()).str_err()?;
    let json = serde_json::to_string_pretty(pairs).str_err()?;
    fs::write(pairs_path(), json).str_err()
}

fn run(root: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// `git init` (if needed) + point `origin` at `remote_url` + get the repo
/// pushed and tracking. Idempotent, so it's safe to call again if a
/// previous attempt partially failed (e.g. push failed because the
/// remote had unrelated history).
pub fn init_and_link(local_path: &str, remote_url: &str, repo_name: &str) -> Result<(), String> {
    if !Path::new(local_path).join(".git").exists() {
        run(local_path, &["init"])?;
    }
    if run(local_path, &["remote", "get-url", "origin"]).is_ok() {
        run(local_path, &["remote", "set-url", "origin", remote_url])?;
    } else {
        run(local_path, &["remote", "add", "origin", remote_url])?;
    }
    let status = run(local_path, &["status", "--porcelain"]).unwrap_or_default();
    let has_commit = run(local_path, &["rev-parse", "HEAD"]).is_ok();
    if !has_commit || !status.trim().is_empty() {
        run(local_path, &["add", "-A"])?;
        let msg = format!("Initial sync: {repo_name}");
        if has_commit {
            run(local_path, &["commit", "-m", &msg])?;
        } else {
            // First-ever commit: an empty folder stages nothing, and a
            // plain commit would then fail with "nothing to commit" --
            // `--allow-empty` covers that. Either way this now actually
            // surfaces a real failure (most commonly: git has no
            // user.name/user.email configured on this machine) instead of
            // it being swallowed here and only showing up much later as
            // the far more confusing "src refspec HEAD does not match any
            // ref" from the push below.
            run(local_path, &["commit", "--allow-empty", "-m", &msg])?;
        }
    }
    // Best-effort: the remote may already have commits (e.g. a README
    // added when creating it), so merge those in before pushing rather
    // than letting a non-fast-forward push fail outright.
    let _ = run(
        local_path,
        &["pull", "--no-rebase", "--allow-unrelated-histories", "origin", "HEAD"],
    );
    run(local_path, &["push", "-u", "origin", "HEAD"])?;
    Ok(())
}

/// The most recent `sync_tick` failure for a given root, if the last
/// attempt didn't succeed -- cleared on the next successful tick. Same
/// reasoning as Drive sync's own `last_error_map` (see sync.rs): the loop
/// below is entirely best-effort (`let _ =`), so a root stuck failing
/// every tick (moved/deleted folder, auth gone stale, remote deleted...)
/// used to fail silently forever, with the UI's only visible symptom
/// being a permanent "syncing…" badge and no explanation why it never
/// finishes -- confirmed live as exactly that complaint.
fn last_error_map() -> &'static Mutex<HashMap<String, String>> {
    static LAST_ERROR: std::sync::OnceLock<Mutex<HashMap<String, String>>> = std::sync::OnceLock::new();
    LAST_ERROR.get_or_init(Default::default)
}

/// One poll tick: commit+push if there are local changes, then pull.
/// Every git step is still best-effort (`let _ =`) -- a transient network
/// hiccup on one tick shouldn't kill the loop, it just tries again next
/// tick -- but the *first* one now short-circuits and records a real
/// error if the root doesn't even exist anymore, rather than repeatedly
/// shelling out to `git -C <gone> ...` forever with nothing to show for
/// it beyond a stuck-looking "syncing" indicator.
fn sync_tick(root: &str) {
    if !Path::new(root).is_dir() {
        last_error_map()
            .lock_safe()
            .insert(root.to_string(), "This folder no longer exists.".to_string());
        return;
    }
    let status = run(root, &["status", "--porcelain"]).unwrap_or_default();
    if !status.trim().is_empty() {
        let _ = run(root, &["add", "-A"]);
        let _ = run(root, &["commit", "-m", "Auto-sync"]);
        let _ = run(root, &["push"]);
    }
    match run(root, &["pull", "--no-rebase"]) {
        Ok(_) => {
            last_error_map().lock_safe().remove(root);
        }
        Err(e) => {
            last_error_map().lock_safe().insert(root.to_string(), e);
        }
    }
}

#[derive(Default)]
pub struct GitSyncState {
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Which roots are mid-`sync_tick` *right now* -- separate from
    /// `active` (which just means "being watched at all") so the
    /// frontend can show a real "syncing…" indicator instead of a
    /// permanently-static one, for a loop that runs unattended in the
    /// background rather than only while some sheet with its own status
    /// text happens to be open. An `Arc` of its own (rather than living
    /// directly on `GitSyncState`) so the spawned loop below -- which,
    /// like the `stop` flag, has to be `'static` and can't hold a
    /// borrowed `&GitSyncState` -- can share it directly.
    syncing: Arc<Mutex<HashSet<String>>>,
}

pub fn start_loop(state: &GitSyncState, root: String) {
    let mut active = state.active.lock_safe();
    if active.contains_key(&root) {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    active.insert(root.clone(), stop.clone());
    drop(active);
    let syncing = state.syncing.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let root_clone = root.clone();
            syncing.lock_safe().insert(root.clone());
            let _ = tokio::task::spawn_blocking(move || sync_tick(&root_clone)).await;
            syncing.lock_safe().remove(&root);
        }
    });
}

pub fn stop_loop(state: &GitSyncState, root: &str) {
    if let Some(stop) = state.active.lock_safe().remove(root) {
        stop.store(true, Ordering::Relaxed);
    }
}

pub fn is_active(state: &GitSyncState, root: &str) -> bool {
    state.active.lock_safe().contains_key(root)
}

pub fn syncing_now(state: &GitSyncState) -> Vec<String> {
    state.syncing.lock_safe().iter().cloned().collect()
}

// ---- Tauri commands ----

#[tauri::command]
pub fn git_sync_list_pairs() -> Vec<GitSyncPair> {
    list_pairs()
}

#[tauri::command]
pub fn git_sync_is_active(state: tauri::State<GitSyncState>, local_path: String) -> bool {
    is_active(&state, &local_path)
}

#[tauri::command]
pub fn git_sync_syncing_now(state: tauri::State<GitSyncState>) -> Vec<String> {
    syncing_now(&state)
}

#[tauri::command]
pub fn git_sync_last_error(local_path: String) -> Option<String> {
    last_error_map().lock_safe().get(&local_path).cloned()
}

#[tauri::command]
pub async fn git_sync_add(
    state: tauri::State<'_, GitSyncState>,
    local_path: String,
    remote_url: String,
    repo_name: String,
) -> Result<GitSyncPair, String> {
    let (lp, ru, rn) = (local_path.clone(), remote_url.clone(), repo_name.clone());
    tokio::task::spawn_blocking(move || init_and_link(&lp, &ru, &rn))
        .await
        .str_err()??;

    let mut pairs = list_pairs();
    pairs.retain(|p| p.local_path != local_path);
    let pair = GitSyncPair {
        local_path: local_path.clone(),
        remote_url,
        repo_name,
    };
    pairs.push(pair.clone());
    save_pairs(&pairs)?;
    start_loop(&state, local_path);
    Ok(pair)
}

/// Stops the background sync loop and forgets the pairing -- the local
/// `.git` folder and its history are left untouched (this only removes
/// VaultExplorer's own tracking of it, same "unsync keeps the files"
/// contract as unlinking a Drive pair).
#[tauri::command]
pub fn git_sync_remove(state: tauri::State<GitSyncState>, local_path: String) -> Result<(), String> {
    stop_loop(&state, &local_path);
    let mut pairs = list_pairs();
    pairs.retain(|p| p.local_path != local_path);
    save_pairs(&pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A populated folder should init, commit, and push cleanly to a
    /// disposable local bare "remote" -- confirmed by cloning that remote
    /// back and checking the file actually landed.
    #[test]
    fn init_and_link_populated_folder_pushes_successfully() {
        let pid = std::process::id();
        let local = format!("/tmp/ve-git-sync-test-pop-{pid}");
        let remote_bare = format!("/tmp/ve-git-sync-test-pop-remote-{pid}.git");
        let clone_dir = format!("/tmp/ve-git-sync-test-pop-clone-{pid}");
        let _ = fs::remove_dir_all(&local);
        let _ = fs::remove_dir_all(&remote_bare);
        let _ = fs::remove_dir_all(&clone_dir);
        fs::create_dir_all(&local).unwrap();
        fs::write(format!("{local}/file.txt"), b"hello").unwrap();
        fs::create_dir_all(&remote_bare).unwrap();
        assert!(Command::new("git").args(["init", "--bare", &remote_bare]).status().unwrap().success());

        init_and_link(&local, &remote_bare, "test-repo").expect("init_and_link should succeed");

        assert!(Command::new("git").args(["clone", &remote_bare, &clone_dir]).status().unwrap().success());
        assert_eq!(fs::read_to_string(format!("{clone_dir}/file.txt")).unwrap(), "hello");

        let _ = fs::remove_dir_all(&local);
        let _ = fs::remove_dir_all(&remote_bare);
        let _ = fs::remove_dir_all(&clone_dir);
    }

    /// An empty folder has nothing to `git add`, so a plain commit would
    /// fail with "nothing to commit" -- must fall back to an empty commit
    /// so `push -u origin HEAD` has something to point at (previously this
    /// surfaced as the confusing "src refspec HEAD does not match any
    /// ref" from the push, instead of failing, or succeeding, up front).
    #[test]
    fn init_and_link_empty_folder_pushes_via_allow_empty_commit() {
        let pid = std::process::id();
        let local = format!("/tmp/ve-git-sync-test-empty-{pid}");
        let remote_bare = format!("/tmp/ve-git-sync-test-empty-remote-{pid}.git");
        let _ = fs::remove_dir_all(&local);
        let _ = fs::remove_dir_all(&remote_bare);
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(&remote_bare).unwrap();
        assert!(Command::new("git").args(["init", "--bare", &remote_bare]).status().unwrap().success());

        init_and_link(&local, &remote_bare, "test-repo")
            .expect("init_and_link should succeed even for an empty folder");

        let log = run(&local, &["log", "--oneline"]).unwrap();
        assert!(!log.trim().is_empty(), "an initial commit must exist even for an empty folder");

        let _ = fs::remove_dir_all(&local);
        let _ = fs::remove_dir_all(&remote_bare);
    }

    /// A commit that fails for a real reason (simulated here with a
    /// pre-commit hook that always rejects, standing in for e.g. git
    /// having no user.name/user.email configured) must now surface as an
    /// error from `init_and_link` itself -- previously it was swallowed
    /// (`let _ = ...`), silently leaving zero commits, and only failed
    /// much later at push time with an unrelated-looking refspec error.
    #[test]
    fn init_and_link_surfaces_commit_failure_instead_of_swallowing() {
        let pid = std::process::id();
        let local = format!("/tmp/ve-git-sync-test-fail-{pid}");
        let remote_bare = format!("/tmp/ve-git-sync-test-fail-remote-{pid}.git");
        let _ = fs::remove_dir_all(&local);
        let _ = fs::remove_dir_all(&remote_bare);
        fs::create_dir_all(&local).unwrap();
        fs::write(format!("{local}/file.txt"), b"content").unwrap();
        run(&local, &["init"]).unwrap();

        let hooks_dir = format!("{local}/.git/hooks");
        let hook_path = format!("{hooks_dir}/pre-commit");
        fs::write(&hook_path, "#!/bin/sh\nexit 1\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&hook_path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&hook_path, perms).unwrap();
        }
        fs::create_dir_all(&remote_bare).unwrap();
        assert!(Command::new("git").args(["init", "--bare", &remote_bare]).status().unwrap().success());

        let result = init_and_link(&local, &remote_bare, "test-repo");
        assert!(result.is_err(), "a real commit failure must now surface as an error, not be swallowed");

        let _ = fs::remove_dir_all(&local);
        let _ = fs::remove_dir_all(&remote_bare);
    }
}
