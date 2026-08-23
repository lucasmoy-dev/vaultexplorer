//! Git-aware context menu actions. First subprocess-shelling code in this
//! codebase -- every other external interaction (Drive sync) goes through
//! HTTP instead. Real fs folders only; scoped out for vault-internal
//! browsing (a git repo living inside a vault's FUSE view is an edge case
//! not worth the complexity here).

use crate::errmap::ToStringErr;
use serde::Serialize;
use std::process::Command;

#[derive(Serialize, Clone)]
pub struct GitFileStatus {
    pub path: String,
    pub status: String,
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

/// The repo root containing `path`, or `None` if it isn't inside a git
/// working tree (or `git` isn't on PATH).
pub fn repo_root(path: &str) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

pub fn status(root: &str) -> Result<Vec<GitFileStatus>, String> {
    let out = run(root, &["status", "--porcelain=v1"])?;
    Ok(out
        .lines()
        .filter(|l| l.len() > 3)
        .map(|line| GitFileStatus {
            status: line[..2].to_string(),
            path: line[3..].trim().to_string(),
        })
        .collect())
}

pub fn pull(root: &str) -> Result<String, String> {
    run(root, &["pull"])
}

pub fn push(root: &str) -> Result<String, String> {
    run(root, &["push"])
}

pub fn commit_all(root: &str, message: &str) -> Result<String, String> {
    run(root, &["add", "-A"])?;
    run(root, &["commit", "-m", message])
}

pub fn stage(root: &str, path: &str) -> Result<(), String> {
    run(root, &["add", "--", path]).map(|_| ())
}

pub fn unstage(root: &str, path: &str) -> Result<(), String> {
    run(root, &["reset", "--", path]).map(|_| ())
}

pub fn discard(root: &str, path: &str) -> Result<(), String> {
    run(root, &["checkout", "--", path]).map(|_| ())
}

// ---- Tauri commands ----

#[tauri::command]
pub fn git_repo_root(path: String) -> Option<String> {
    repo_root(&path)
}

#[tauri::command]
pub fn git_status(root: String) -> Result<Vec<GitFileStatus>, String> {
    status(&root)
}

#[tauri::command]
pub fn git_pull(root: String) -> Result<String, String> {
    pull(&root)
}

#[tauri::command]
pub fn git_push(root: String) -> Result<String, String> {
    push(&root)
}

#[tauri::command]
pub fn git_commit_all(root: String, message: String) -> Result<String, String> {
    commit_all(&root, &message)
}

#[tauri::command]
pub fn git_stage(root: String, path: String) -> Result<(), String> {
    stage(&root, &path)
}

#[tauri::command]
pub fn git_unstage(root: String, path: String) -> Result<(), String> {
    unstage(&root, &path)
}

#[tauri::command]
pub fn git_discard(root: String, path: String) -> Result<(), String> {
    discard(&root, &path)
}
