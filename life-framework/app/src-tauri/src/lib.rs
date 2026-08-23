mod android;
mod storage;
mod update;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            storage::load_db,
            storage::save_db,
            storage::export_db,
            storage::import_db,
            update::app_version,
            update::can_auto_install,
            update::check_update,
            update::download_and_install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Life Framework");
}
