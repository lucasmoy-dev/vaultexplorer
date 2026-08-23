//! Post-sync cloud verification. After a bisync pass actually moves data
//! (see `sync.rs`), `rclone check` compares every local file's md5 against
//! the checksum the provider itself reports for its stored copy -- proof
//! the remote bytes match what's on disk, with nothing downloaded. The
//! per-file result feeds the grid's persistent green check: "verified"
//! means "checksummed against the cloud", never merely "a sync pass ran".
//!
//! Everything here operates on ciphertext paths (the real on-disk files a
//! sync pair moves); `sync_verify_states` maps a vault's plaintext entries
//! to their ciphertext files via `Vault::encrypted_path`, so files browsed
//! *inside* an unlocked vault get the same verified badge.

use crate::errmap::LockExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FileCheck {
    Match,
    Differ,
    LocalOnly,
    RemoteOnly,
    Error,
}

#[derive(Serialize, Deserialize)]
struct PairVerify {
    /// Unix seconds when the check ran -- any local mtime newer than this
    /// means the file changed after it was verified, i.e. "pending".
    verified_at: u64,
    /// rclone-relative ciphertext path -> outcome of the last check.
    files: HashMap<String, FileCheck>,
}

fn state_path() -> PathBuf {
    crate::sync::config_dir().join("verify_state.json")
}

fn load_state(path: &Path) -> HashMap<String, PairVerify> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_state(path: &Path, map: &HashMap<String, PairVerify>) {
    if let Ok(json) = serde_json::to_string(map) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, json);
    }
}

/// Seeded from disk so a fresh app launch shows last session's verified
/// badges on the very first status poll, instead of everything sitting
/// blank until the first bisync + `rclone check` round trip (~half a
/// minute) completes. The mtime rule keeps a stale load honest: anything
/// edited while the app was closed is newer than the stored `verified_at`
/// and reads "pending" until re-verified.
fn results() -> &'static Mutex<HashMap<String, PairVerify>> {
    static RESULTS: OnceLock<Mutex<HashMap<String, PairVerify>>> = OnceLock::new();
    RESULTS.get_or_init(|| Mutex::new(load_state(&state_path())))
}

fn verifying_set() -> &'static Mutex<HashSet<String>> {
    static VERIFYING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    VERIFYING.get_or_init(Default::default)
}

pub fn has_result(local_path: &str) -> bool {
    results().lock_safe().contains_key(local_path)
}

/// Every pair mid-`rclone check` right now -- polled by the frontend the
/// same way as the syncing sets, to show a "Verifying …" task.
pub fn verifying_now() -> Vec<String> {
    verifying_set().lock_safe().iter().cloned().collect()
}

/// One line per file: a marker char, a space, the path (`--combined -`).
/// `=` match, `*` differ, `+` source(local)-only, `-` dest(remote)-only,
/// `!` error reading one side.
fn parse_combined(stdout: &str) -> HashMap<String, FileCheck> {
    stdout
        .lines()
        .filter_map(|line| {
            let path = line.get(2..)?.trim_end();
            if path.is_empty() {
                return None;
            }
            let state = match line.as_bytes().first()? {
                b'=' => FileCheck::Match,
                b'*' => FileCheck::Differ,
                b'+' => FileCheck::LocalOnly,
                b'-' => FileCheck::RemoteOnly,
                b'!' => FileCheck::Error,
                _ => return None,
            };
            Some((path.to_string(), state))
        })
        .collect()
}

fn now_epoch() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Run `rclone check` for a pair and store the per-file outcome. Blocking
/// (called at the tail of a sync pass, which already runs on its own
/// thread). A run that produced no usable output keeps the previous
/// result: stale-but-honest beats wiping every badge over a network blip
/// -- the mtime rule still flips anything edited since to "pending".
pub fn verify_pair(local_path: &str, remote: &str) {
    verifying_set().lock_safe().insert(local_path.to_string());
    let output = Command::new("rclone")
        .args(["check", local_path, remote, "--combined", "-"])
        .args(crate::rclone::PACING_ARGS)
        .output();
    verifying_set().lock_safe().remove(local_path);
    let Ok(output) = output else { return };
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Exit code 1 just means "differences found" and still prints the full
    // per-file listing; only a run with no listing at all is a real failure.
    if stdout.trim().is_empty() && !output.status.success() {
        return;
    }
    let mut map = results().lock_safe();
    map.insert(
        local_path.to_string(),
        PairVerify { verified_at: now_epoch(), files: parse_combined(&stdout) },
    );
    save_state(&state_path(), &map);
}

fn file_state(pv: &PairVerify, rel: &str, mtime_secs: u64) -> &'static str {
    if mtime_secs > pv.verified_at {
        return "pending";
    }
    match pv.files.get(rel) {
        Some(FileCheck::Match) => "verified",
        // Differ/only-one-side/error, or a file the last check never saw
        // (created since): not proven to be in the cloud yet.
        _ => "pending",
    }
}

/// Whether any file the last check saw under `rel_prefix` isn't a clean
/// match ("" = the whole pair).
fn map_prefix_pending(pv: &PairVerify, rel_prefix: &str) -> bool {
    pv.files.iter().any(|(rel, st)| {
        *st != FileCheck::Match
            && (rel_prefix.is_empty() || rel.strip_prefix(rel_prefix).is_some_and(|r| r.starts_with('/')))
    })
}

/// Cap on how many on-disk entries a per-directory freshness walk will
/// visit before giving up ("unknown") -- keeps the 2.5s status poll cheap
/// even if someone points it at a huge tree.
const WALK_BUDGET: usize = 5000;

/// Any file under `dir` modified after the check ran? `None` = walk budget
/// exhausted, caller should answer "unknown".
fn tree_newer_than(dir: &Path, verified_at: u64, budget: &mut usize) -> Option<bool> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        let meta = entry.metadata().ok()?;
        if meta.is_dir() {
            match tree_newer_than(&entry.path(), verified_at, budget) {
                Some(true) => return Some(true),
                Some(false) => {}
                None => return None,
            }
        } else {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(u64::MAX);
            if mtime > verified_at {
                return Some(true);
            }
        }
    }
    Some(false)
}

fn dir_state(pv: &PairVerify, rel_prefix: &str, dir_abs: &Path) -> &'static str {
    if map_prefix_pending(pv, rel_prefix) {
        return "pending";
    }
    let mut budget = WALK_BUDGET;
    match tree_newer_than(dir_abs, pv.verified_at, &mut budget) {
        Some(true) => "pending",
        Some(false) => "verified",
        None => "unknown",
    }
}

fn rel_to(pair_root: &str, abs: &Path) -> Option<String> {
    abs.strip_prefix(pair_root).ok().map(|p| p.to_string_lossy().to_string())
}

fn state_for(pair_root: &str, pv: &PairVerify, cipher_abs: &Path) -> &'static str {
    let Some(rel) = rel_to(pair_root, cipher_abs) else { return "unknown" };
    let Ok(meta) = std::fs::metadata(cipher_abs) else { return "unknown" };
    if meta.is_dir() {
        dir_state(pv, &rel, cipher_abs)
    } else {
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(u64::MAX);
        file_state(pv, &rel, mtime)
    }
}

// ---- Tauri commands ----

#[tauri::command]
pub fn drive_verifying_now() -> Vec<String> {
    verifying_now()
}

/// Per-entry verification state for the folder currently on screen:
/// `"verified"` (last cloud check matched and nothing changed since),
/// `"pending"` (changed/new/differing -- not proven in the cloud),
/// `"unknown"` (under a synced pair but no check result for it yet -- the
/// badge should show its plain static form), or `"none"` (not under any
/// synced pair at all -- no badge). `kind`/`dir` mirror the frontend's
/// `Loc`: for `"vault"`, `dir` is the vault-relative dir of the active
/// vault and each name is mapped to its ciphertext file; for `"fs"`,
/// `dir` is the absolute directory.
#[tauri::command]
pub fn sync_verify_states(
    state: tauri::State<crate::AppState>,
    kind: String,
    dir: String,
    names: Vec<String>,
) -> Vec<String> {
    let nones = || vec!["none".to_string(); names.len()];
    let unknowns = || vec!["unknown".to_string(); names.len()];

    // The tree the pair lookup keys on, plus each entry's ciphertext path.
    let (base, cipher_paths): (String, Vec<Option<PathBuf>>) = if kind == "vault" {
        let Some(root) = state.active.lock_safe().clone() else { return nones() };
        let paths = names
            .iter()
            .map(|name| {
                let rel = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
                crate::with_vault(&state, |v| v.encrypted_path(Path::new(&rel))).ok()
            })
            .collect();
        (root, paths)
    } else {
        let paths = names.iter().map(|name| Some(Path::new(&dir).join(name))).collect();
        (dir.clone(), paths)
    };

    let Some(pair) = crate::sync::list_pairs()
        .into_iter()
        .find(|p| Path::new(&base).starts_with(&p.local_path))
    else {
        return nones();
    };

    let map = results().lock_safe();
    let Some(pv) = map.get(&pair.local_path) else { return unknowns() };
    cipher_paths
        .iter()
        .map(|p| match p {
            Some(abs) => state_for(&pair.local_path, pv, abs).to_string(),
            None => "unknown".to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pv(verified_at: u64, files: &[(&str, FileCheck)]) -> PairVerify {
        PairVerify {
            verified_at,
            files: files.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    #[test]
    fn parse_combined_reads_every_marker() {
        let out = "= a/b.txt\n* c.bin\n+ only_local\n- only_remote\n! broken\ngarbage line\n";
        let map = parse_combined(out);
        assert_eq!(map.get("a/b.txt"), Some(&FileCheck::Match));
        assert_eq!(map.get("c.bin"), Some(&FileCheck::Differ));
        assert_eq!(map.get("only_local"), Some(&FileCheck::LocalOnly));
        assert_eq!(map.get("only_remote"), Some(&FileCheck::RemoteOnly));
        assert_eq!(map.get("broken"), Some(&FileCheck::Error));
        assert_eq!(map.len(), 5);
    }

    #[test]
    fn file_state_is_verified_only_for_an_unmodified_match() {
        let pv = pv(1000, &[("f", FileCheck::Match), ("g", FileCheck::Differ)]);
        assert_eq!(file_state(&pv, "f", 900), "verified");
        assert_eq!(file_state(&pv, "f", 1001), "pending", "edited after the check");
        assert_eq!(file_state(&pv, "g", 900), "pending", "cloud copy differs");
        assert_eq!(file_state(&pv, "new", 900), "pending", "never seen by a check");
    }

    #[test]
    fn state_survives_a_save_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("ve-verify-test-{}", std::process::id()));
        let path = dir.join("verify_state.json");
        let mut map = HashMap::new();
        map.insert("/some/pair".to_string(), pv(1234, &[("f", FileCheck::Match)]));
        save_state(&path, &map);
        let loaded = load_state(&path);
        assert_eq!(loaded.get("/some/pair").unwrap().verified_at, 1234);
        assert_eq!(loaded.get("/some/pair").unwrap().files.get("f"), Some(&FileCheck::Match));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_state_is_empty_for_a_missing_or_corrupt_file() {
        assert!(load_state(Path::new("/nonexistent/verify_state.json")).is_empty());
    }

    #[test]
    fn map_prefix_pending_scopes_to_the_directory() {
        let pv = pv(1000, &[("dir/bad", FileCheck::LocalOnly), ("other/ok", FileCheck::Match)]);
        assert!(map_prefix_pending(&pv, "dir"));
        assert!(!map_prefix_pending(&pv, "other"));
        assert!(map_prefix_pending(&pv, ""), "root prefix sees everything");
        assert!(!map_prefix_pending(&pv, "di"), "a name prefix is not a path prefix");
    }
}
