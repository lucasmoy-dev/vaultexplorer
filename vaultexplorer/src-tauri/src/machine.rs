//! Physical-drive listing for the "My Computer" favorite, and a
//! CPU/RAM/disk summary for its Get Info panel. Same external-tool
//! approach the rest of this codebase already uses instead of pulling in
//! a whole system-info crate: `lsblk` (already used by recovery.rs's disk
//! listing) for the drive tree, `df` for live usage, and `/proc` for
//! CPU/RAM -- all standard on any Linux desktop this app targets.

use crate::errmap::ToStringErr;
use serde::Serialize;
use std::collections::HashMap;
use std::process::Command;

#[derive(Serialize, Clone)]
pub struct Drive {
    pub path: String, // "/dev/sda1"
    pub name: String, // "sda1"
    pub label: Option<String>,
    pub fstype: Option<String>,
    pub mountpoint: Option<String>,
    pub removable: bool,
    pub model: Option<String>,
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

/// Mountpoint device path -> (total, used, free) in bytes, via a single
/// `df` call rather than one `statvfs` per drive.
fn df_usage() -> HashMap<String, (u64, u64, u64)> {
    let mut map = HashMap::new();
    let Ok(output) = Command::new("df").args(["-B1", "--output=source,size,used,avail"]).output()
    else {
        return map;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            continue;
        }
        let (Ok(size), Ok(used), Ok(avail)) =
            (cols[1].parse::<u64>(), cols[2].parse::<u64>(), cols[3].parse::<u64>())
        else {
            continue;
        };
        map.insert(cols[0].to_string(), (size, used, avail));
    }
    map
}

/// List real disks/partitions worth showing in "My Computer": every
/// partition, plus whole disks that carry a filesystem directly with no
/// partition table (common for small USB sticks -- a "superfloppy").
/// Loop devices (snap package mounts) and optical drives are skipped.
pub fn list_drives() -> Result<Vec<Drive>, String> {
    let output = Command::new("lsblk")
        .args(["-J", "-b", "-o", "NAME,PATH,SIZE,FSTYPE,MOUNTPOINT,LABEL,RM,TYPE,MODEL"])
        .output()
        .str_err()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).str_err()?;
    let usage = df_usage();
    let mut out = Vec::new();

    fn walk(
        devices: &[serde_json::Value],
        parent_rm: bool,
        parent_model: Option<&str>,
        usage: &HashMap<String, (u64, u64, u64)>,
        out: &mut Vec<Drive>,
    ) {
        for d in devices {
            let kind = d.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if kind == "loop" || kind == "rom" {
                continue;
            }
            let rm = d.get("rm").and_then(|v| v.as_bool()).unwrap_or(parent_rm);
            let model = d
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| parent_model.map(|s| s.to_string()));
            let children = d.get("children").and_then(|v| v.as_array());
            let has_children = children.map(|a| !a.is_empty()).unwrap_or(false);

            // Surface a leaf: a partition, or a whole disk with no
            // partition table (raw filesystem straight on the disk).
            if kind == "part" || (kind == "disk" && !has_children) {
                let path = d.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = d.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let fstype = d.get("fstype").and_then(|v| v.as_str()).map(|s| s.to_string());
                let mountpoint =
                    d.get("mountpoint").and_then(|v| v.as_str()).map(|s| s.to_string());
                let label = d.get("label").and_then(|v| v.as_str()).map(|s| s.to_string());
                let size = d.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                let (total, used, free) =
                    usage.get(&path).copied().unwrap_or((size, 0, if size > 0 { size } else { 0 }));
                out.push(Drive {
                    path,
                    name,
                    label,
                    fstype,
                    mountpoint,
                    removable: rm,
                    model: model.clone(),
                    total,
                    used,
                    free,
                });
            }

            if let Some(children) = children {
                walk(children, rm, model.as_deref(), usage, out);
            }
        }
    }

    if let Some(devices) = json.get("blockdevices").and_then(|v| v.as_array()) {
        walk(devices, false, None, &usage, &mut out);
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct MachineSummary {
    pub cpu_model: String,
    pub cpu_cores: u32,
    pub ram_total: u64,
    /// MemAvailable, not MemFree: free memory on Linux is mostly cache
    /// that would be handed back on demand, so MemFree reads as "almost
    /// nothing left" on a perfectly healthy machine. Available is what a
    /// person means by free.
    pub ram_available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub uptime_secs: u64,
    pub load1: f32,
    pub os_name: String,
    pub disks: Vec<Drive>,
}

fn read_proc_field(text: &str, prefix: &str) -> Option<String> {
    text.lines()
        .find(|l| l.starts_with(prefix))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
}

pub fn summary() -> MachineSummary {
    let cpu_model = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| read_proc_field(&text, "model name"))
        .unwrap_or_else(|| "Unknown CPU".to_string());
    let cpu_cores = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1);
    let ram_total = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| read_proc_field(&text, "MemTotal"))
        .and_then(|v| v.split_whitespace().next().map(|s| s.to_string()))
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb * 1024)
        .unwrap_or(0);
    let os_name = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines().find(|l| l.starts_with("PRETTY_NAME=")).map(|l| {
                l.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string()
            })
        })
        .unwrap_or_else(|| std::env::consts::OS.to_string());
    // Only the leaves that are actually usable capacity (mounted, or a
    // removable drive worth showing even unmounted) -- not every loose
    // partition lsblk happens to enumerate.
    let disks = list_drives()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.mountpoint.is_some() || d.removable)
        .collect();
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let kb_field = |prefix: &str| -> u64 {
        read_proc_field(&meminfo, prefix)
            .and_then(|v| v.split_whitespace().next().map(|s| s.to_string()))
            .and_then(|kb| kb.parse::<u64>().ok())
            .map(|kb| kb * 1024)
            .unwrap_or(0)
    };
    let ram_available = kb_field("MemAvailable");
    let swap_total = kb_field("SwapTotal");
    let swap_free = kb_field("SwapFree");
    let uptime_secs = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|t| t.split_whitespace().next().and_then(|s| s.parse::<f64>().ok()))
        .map(|s| s as u64)
        .unwrap_or(0);
    let load1 = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|t| t.split_whitespace().next().and_then(|s| s.parse::<f32>().ok()))
        .unwrap_or(0.0);
    MachineSummary {
        cpu_model,
        cpu_cores,
        ram_total,
        ram_available,
        swap_total,
        swap_free,
        uptime_secs,
        load1,
        os_name,
        disks,
    }
}

#[cfg(desktop)]
const PROTECTED_MOUNTPOINTS: &[&str] =
    &["/", "/home", "/boot", "/boot/efi", "/usr", "/var", "/etc", "/opt", "/srv"];
#[cfg(desktop)]
const ALLOWED_FS_TYPES: &[&str] = &["vfat", "exfat", "ntfs", "ext4"];

/// Format `device_path` (e.g. "/dev/sdb1") via UDisks2's D-Bus `Format`
/// method (there's no `udisksctl format` subcommand on the udisks2
/// version this was built against, even though the D-Bus method itself
/// has been there since 2.9 -- see the object's introspection XML).
/// Re-checks removability and the protected-mountpoint list itself,
/// server-side, rather than trusting whatever the caller/UI last saw --
/// this is the one thing in this file that destroys data if it's wrong.
/// Passes `no-block` so the call returns once the format *starts*
/// rather than blocking on the D-Bus default reply timeout for however
/// long a large drive takes to actually finish.
#[cfg(desktop)]
pub async fn format_device(device_path: &str, fs_type: &str, label: &str) -> Result<(), String> {
    if !ALLOWED_FS_TYPES.contains(&fs_type) {
        return Err(format!("Unsupported filesystem type: {fs_type}"));
    }

    let drives = list_drives()?;
    let target = drives
        .iter()
        .find(|d| d.path == device_path)
        .ok_or_else(|| format!("Device {device_path} not found"))?;
    if !target.removable {
        return Err("Refusing to format a non-removable (internal) drive".to_string());
    }
    if let Some(mp) = &target.mountpoint {
        if PROTECTED_MOUNTPOINTS.contains(&mp.as_str()) {
            return Err(format!("Refusing to format {device_path}: mounted at {mp}"));
        }
    }

    use zbus::zvariant::Value;

    let conn = zbus::Connection::system().await.str_err()?;
    let dev_name = device_path.trim_start_matches("/dev/");
    let object_path = format!("/org/freedesktop/UDisks2/block_devices/{dev_name}");

    if target.mountpoint.is_some() {
        if let Ok(fs_proxy) = zbus::Proxy::new(
            &conn,
            "org.freedesktop.UDisks2",
            object_path.as_str(),
            "org.freedesktop.UDisks2.Filesystem",
        )
        .await
        {
            let empty_opts: HashMap<&str, Value> = HashMap::new();
            let _: Result<(), zbus::Error> = fs_proxy.call("Unmount", &empty_opts).await;
        }
    }

    let block_proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.UDisks2",
        object_path.as_str(),
        "org.freedesktop.UDisks2.Block",
    )
    .await
    .str_err()?;

    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("no-block", Value::new(true));
    if !label.is_empty() {
        options.insert("label", Value::new(label));
    }
    block_proxy
        .call::<_, _, ()>("Format", &(fs_type, options))
        .await
        .str_err()
}

fn read_dmi(field: &str) -> String {
    std::fs::read_to_string(format!("/sys/class/dmi/id/{field}"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Unknown".to_string())
}

#[derive(Serialize, Clone)]
pub struct PciDeviceInfo {
    pub address: String,
    pub description: String,
    pub driver: Option<String>,
    pub kind: String, // "wifi" | "ethernet" | "gpu"
}

/// Network + display controllers from `lspci -nnk`, the same "is a driver
/// actually bound" signal `lspci` itself surfaces -- a device block with
/// no "Kernel driver in use" line means nothing claimed the hardware,
/// which is as close to a generic, cross-vendor "no/broken driver" check
/// as this can get without a distro-specific driver database.
fn pci_devices() -> Vec<PciDeviceInfo> {
    let Ok(output) = Command::new("lspci").arg("-nnk").output() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    let mut current: Option<PciDeviceInfo> = None;

    for line in text.lines() {
        if !line.starts_with([' ', '\t']) {
            if let Some(dev) = current.take() {
                out.push(dev);
            }
            let Some((addr, rest)) = line.split_once(' ') else { continue };
            let lower = rest.to_lowercase();
            let kind = if lower.contains("network controller") || lower.contains("wireless") {
                "wifi"
            } else if lower.contains("ethernet controller") {
                "ethernet"
            } else if lower.contains("vga compatible controller")
                || lower.contains("3d controller")
                || lower.contains("display controller")
            {
                "gpu"
            } else {
                "other"
            };
            if kind == "other" {
                continue;
            }
            let description = rest.split_once(':').map(|(_, d)| d.trim()).unwrap_or(rest).to_string();
            current = Some(PciDeviceInfo {
                address: addr.to_string(),
                description,
                driver: None,
                kind: kind.to_string(),
            });
        } else if let Some(dev) = current.as_mut() {
            if let Some(driver) = line.trim().strip_prefix("Kernel driver in use:") {
                dev.driver = Some(driver.trim().to_string());
            }
        }
    }
    if let Some(dev) = current.take() {
        out.push(dev);
    }
    out
}

#[derive(Serialize, Clone)]
pub struct DriverRecommendation {
    pub vendor: String,
    pub driver: String,
}

/// Best-effort: only present on Ubuntu/derivatives that ship
/// `ubuntu-drivers`. Lists hardware that has a recommended (often
/// non-free) driver available beyond whatever's already active --
/// e.g. a GPU that could use the proprietary driver instead of the open
/// one, or (as seen on this machine) an OEM meta-package / fingerprint
/// reader driver.
fn driver_recommendations() -> Vec<DriverRecommendation> {
    let Ok(output) = Command::new("ubuntu-drivers").arg("devices").output() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    let mut vendor = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("vendor").and_then(|s| s.split(':').nth(1)) {
            vendor = v.trim().to_string();
        } else if let Some(d) = line.strip_prefix("driver").and_then(|s| s.split_once(':')) {
            out.push(DriverRecommendation {
                vendor: vendor.clone(),
                driver: d.1.trim().to_string(),
            });
        }
    }
    out
}

#[derive(Serialize)]
pub struct AdvancedInfo {
    pub board_vendor: String,
    pub board_name: String,
    pub board_version: String,
    pub bios_vendor: String,
    pub bios_version: String,
    pub product_name: String,
    pub pci_devices: Vec<PciDeviceInfo>,
    pub driver_recommendations: Vec<DriverRecommendation>,
    pub ubuntu_drivers_available: bool,
}

pub fn advanced_info() -> AdvancedInfo {
    let ubuntu_drivers_available = Command::new("which")
        .arg("ubuntu-drivers")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    AdvancedInfo {
        board_vendor: read_dmi("board_vendor"),
        board_name: read_dmi("board_name"),
        board_version: read_dmi("board_version"),
        bios_vendor: read_dmi("bios_vendor"),
        bios_version: read_dmi("bios_version"),
        product_name: read_dmi("product_name"),
        pci_devices: pci_devices(),
        driver_recommendations: if ubuntu_drivers_available { driver_recommendations() } else { Vec::new() },
        ubuntu_drivers_available,
    }
}

/// Fire-and-forget: launches `pkexec ubuntu-drivers install`, which pops
/// polkit's own graphical auth prompt and, once authorized, runs in the
/// background. Not waited on -- a driver install can take minutes, and
/// there's no clean way to stream apt's progress back through this
/// command without a lot more plumbing (a Channel like the other
/// long-running ops use) for a button the user will use rarely.
pub fn update_drivers() -> Result<(), String> {
    std::process::Command::new("pkexec")
        .args(["ubuntu-drivers", "install"])
        .spawn()
        .map(|_| ())
        .str_err()
}

// ---- Tauri commands ----

#[tauri::command]
pub fn machine_list_drives() -> Result<Vec<Drive>, String> {
    list_drives()
}

#[tauri::command]
pub fn machine_summary() -> MachineSummary {
    summary()
}

#[cfg(desktop)]
#[tauri::command]
pub async fn machine_format_drive(device: String, fs_type: String, label: String) -> Result<(), String> {
    format_device(&device, &fs_type, &label).await
}

#[tauri::command]
pub fn machine_advanced_info() -> AdvancedInfo {
    advanced_info()
}

#[tauri::command]
pub fn machine_update_drivers() -> Result<(), String> {
    update_drivers()
}
