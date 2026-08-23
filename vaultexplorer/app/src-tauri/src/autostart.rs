//! "Start VaultExplorer at login" -- a plain XDG autostart entry
//! (`~/.config/autostart/*.desktop`), the same mechanism every Linux
//! desktop (GNOME/KDE/etc.) already runs a user's session startup apps
//! through. No new in-app launch behavior is needed on top of this: a
//! normal launch already shows the main window and calls
//! `auto_unlock_vaults` for every vault with "Unlock automatically" set
//! (see `lib.rs`), so registering this is the entire feature -- the vault
//! itself unlocking automatically was already there.

use crate::errmap::ToStringErr;

fn autostart_file_path() -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}/.config/autostart", crate::home_dir()))
        .join("vaultexplorer.desktop")
}

#[tauri::command]
pub fn autostart_enabled() -> bool {
    autostart_file_path().is_file()
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let path = autostart_file_path();
    if !enabled {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let exe = std::env::current_exe().str_err()?;
    std::fs::create_dir_all(path.parent().unwrap()).str_err()?;
    std::fs::write(
        &path,
        format!(
            "[Desktop Entry]\nType=Application\nName=VaultExplorer\nExec={}\nX-GNOME-Autostart-enabled=true\nNoDisplay=false\n",
            exe.to_string_lossy()
        ),
    )
    .str_err()
}
