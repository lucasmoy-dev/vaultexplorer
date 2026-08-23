//! Local persistence: the whole app state is one JSON document in the
//! platform app-data dir. Simple, offline, and trivially exportable.
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

fn db_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("db.json"))
}

/// Returns the stored JSON, or an empty string on first run (the JS side
/// treats empty as "seed a fresh document").
#[tauri::command]
pub fn load_db(app: tauri::AppHandle) -> Result<String, String> {
    let p = db_path(&app)?;
    Ok(fs::read_to_string(&p).unwrap_or_default())
}

/// Write-then-rename so a crash mid-write can't corrupt the live file.
#[tauri::command]
pub fn save_db(app: tauri::AppHandle, json: String) -> Result<(), String> {
    let p = db_path(&app)?;
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, json.as_bytes()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &p).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn export_db(json: String, dest: String) -> Result<(), String> {
    fs::write(&dest, json.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_db(src: String) -> Result<String, String> {
    fs::read_to_string(&src).map_err(|e| e.to_string())
}
