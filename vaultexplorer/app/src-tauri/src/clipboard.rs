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

/// The reverse direction: whatever image the *system* clipboard holds
/// (a screenshot tool, a browser's "Copy image", another app), re-encoded
/// as PNG so pasting into a folder can write a real file. The clipboard
/// hands back raw RGBA with no container, so the PNG is produced here
/// rather than passed through.
fn read_clipboard_png(app: &tauri::AppHandle) -> Result<Vec<u8>, String> {
    let img = app.clipboard().read_image().str_err()?;
    let (width, height) = (img.width(), img.height());
    let buf = image::RgbaImage::from_raw(width, height, img.rgba().to_vec())
        .ok_or_else(|| "clipboard image size did not match its pixel data".to_string())?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(buf)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .str_err()?;
    Ok(png)
}

/// Both readers go through `spawn_blocking` on purpose: the plugin
/// documents that reading the clipboard on the main thread can deadlock on
/// Linux when *this* process is the clipboard owner -- which "Copy" on an
/// image above makes the normal case, not an edge case.
///
/// Cheap enough to call on every window focus, and it stays server-side:
/// answering "is there an image to paste?" with a bool avoids shipping a
/// multi-megabyte screenshot over IPC just to grey out a button.
#[tauri::command]
pub async fn clipboard_has_image(app: tauri::AppHandle) -> bool {
    tauri::async_runtime::spawn_blocking(move || app.clipboard().read_image().is_ok())
        .await
        .unwrap_or(false)
}

/// Returned as a raw IPC response (an `ArrayBuffer` on the JS side) rather
/// than a serialised `Vec<u8>`: a pasted screenshot is megabytes, and as
/// JSON that would be one number-per-byte array to build and parse.
#[tauri::command]
pub async fn clipboard_read_image_png(app: tauri::AppHandle) -> Result<tauri::ipc::Response, String> {
    let png = tauri::async_runtime::spawn_blocking(move || read_clipboard_png(&app))
        .await
        .str_err()??;
    Ok(tauri::ipc::Response::new(png))
}
