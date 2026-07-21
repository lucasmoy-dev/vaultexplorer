//! Deleted-file recovery (Recuva-style), wrapping `photorec`/`testdisk`
//! rather than building a raw-disk forensic scanner from scratch. Not
//! installed on the machine this was built on, so **the exact `photorec
//! /cmd` scripted-mode invocation below is a best-effort construction
//! from its documented syntax, not verified against a real binary** --
//! it's the one piece of this whole feedback round that still needs a
//! real spike (install `testdisk`, run this, adjust the token list to
//! match). Everything else here (availability check, disk listing,
//! same-disk safety check) is real and doesn't depend on that.
//!
//! This is disk/partition-level, not folder-level -- a structurally
//! different shape from the rest of the app's per-folder actions, which
//! is why it's a Settings/Tools entry rather than a context-menu item.

use crate::errmap::ToStringErr;
use serde::Serialize;
use std::process::Command;

pub fn tool_available() -> bool {
    Command::new("which")
        .arg("photorec")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[derive(Serialize, Clone)]
pub struct DiskInfo {
    pub name: String,       // e.g. "sda", "sda1"
    pub size: String,       // human-readable, straight from lsblk
    pub mountpoint: Option<String>,
    #[serde(rename = "type")]
    pub kind: String, // "disk" | "part" | ...
}

/// List block devices via `lsblk -J` (JSON output, no extra dependency
/// needed -- `serde_json` is already a dependency for other commands).
pub fn list_disks() -> Result<Vec<DiskInfo>, String> {
    let output = Command::new("lsblk")
        .args(["-J", "-o", "NAME,SIZE,MOUNTPOINT,TYPE"])
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).str_err()?;
    let mut out = Vec::new();
    fn walk(node: &serde_json::Value, out: &mut Vec<DiskInfo>) {
        let Some(devices) = node.get("blockdevices").and_then(|v| v.as_array()) else {
            return;
        };
        for d in devices {
            out.push(DiskInfo {
                name: d.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                size: d.get("size").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                mountpoint: d.get("mountpoint").and_then(|v| v.as_str()).map(|s| s.to_string()),
                kind: d.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            });
            if let Some(children) = d.get("children") {
                walk(&serde_json::json!({ "blockdevices": children }), out);
            }
        }
    }
    walk(&json, &mut out);
    Ok(out)
}

/// The top-level disk name (e.g. "sda") backing a given path, via `df` +
/// `lsblk`'s parent-kernel-name -- a best-effort heuristic for the "don't
/// recover onto the disk you're scanning" safety check, not a hard
/// guarantee (LVM/RAID/bind-mount setups can defeat it).
pub fn disk_for_path(path: &str) -> Option<String> {
    let df = Command::new("df").args(["--output=source", path]).output().ok()?;
    let source = String::from_utf8_lossy(&df.stdout)
        .lines()
        .nth(1)?
        .trim()
        .to_string();
    let dev_name = source.trim_start_matches("/dev/").to_string();
    let lsblk = Command::new("lsblk")
        .args(["-no", "PKNAME", &source])
        .output()
        .ok()?;
    let pkname = String::from_utf8_lossy(&lsblk.stdout).trim().to_string();
    Some(if pkname.is_empty() { dev_name } else { pkname })
}

/// Run photorec's scripted mode against `device` (e.g. "/dev/sdb1"),
/// recovering into `dest_dir`. Blocks until photorec exits; there is no
/// reliable percentage-complete signal from scripted mode, so callers
/// show an indeterminate "running" state rather than a progress bar.
pub fn run(device: &str, dest_dir: &str) -> Result<(), String> {
    let output = Command::new("photorec")
        .args([
            "/d",
            dest_dir,
            "/cmd",
            device,
            "partition_none,options,fileopt,everything,enable,search",
        ])
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

// ---- Tauri commands ----

#[tauri::command]
pub fn recovery_tool_available() -> bool {
    tool_available()
}

#[tauri::command]
pub fn recovery_list_disks() -> Result<Vec<DiskInfo>, String> {
    list_disks()
}

/// Best-effort "is the destination on a different disk than the one
/// being scanned" check (see the module docs for its limits).
#[tauri::command]
pub fn recovery_same_disk(device: String, dest_dir: String) -> bool {
    let device_name = device.trim_start_matches("/dev/");
    match disk_for_path(&dest_dir) {
        Some(dest_disk) => device_name.starts_with(&dest_disk) || dest_disk.starts_with(device_name),
        None => false,
    }
}

#[tauri::command]
pub async fn recovery_run(device: String, dest_dir: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || run(&device, &dest_dir))
        .await
        .str_err()?
}
