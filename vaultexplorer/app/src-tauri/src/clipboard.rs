//! "Copy" on an image puts real, decoded pixel data on the *system*
//! clipboard (via `tauri-plugin-clipboard-manager`, backed by `arboard`)
//! in addition to VaultExplorer's own internal clipboard state used for
//! in-app paste -- without this, an image "copied" here was only ever
//! pasteable back into this app, never into a browser tab or another
//! native app, which is the whole point of a system clipboard.

use crate::errmap::ToStringErr;
use image::GenericImageView;
use tauri::image::Image;
use tauri_plugin_clipboard_manager::ClipboardExt;

fn write_rgba_to_clipboard(app: &tauri::AppHandle, bytes: &[u8]) -> Result<(), String> {
    let img = image::load_from_memory(bytes).str_err()?;
    let (width, height) = img.dimensions();
    let rgba = img.to_rgba8().into_raw();
    app.clipboard()
        .write_image(&Image::new(&rgba, width, height))
        .str_err()
}

#[tauri::command]
pub fn fs_copy_image_to_clipboard(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let bytes = std::fs::read(&path).str_err()?;
    write_rgba_to_clipboard(&app, &bytes)
}

#[tauri::command]
pub fn vault_copy_image_to_clipboard(
    app: tauri::AppHandle,
    state: tauri::State<crate::AppState>,
    rel_path: String,
) -> Result<(), String> {
    let bytes = crate::with_vault(&state, |v| v.decrypt_file(&rel_path))?;
    write_rgba_to_clipboard(&app, &bytes)
}
