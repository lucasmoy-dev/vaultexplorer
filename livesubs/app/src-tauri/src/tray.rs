//! The app's entire chrome: one icon in the top bar.
//!
//! There is no main window on purpose -- the thing runs all day next to
//! the wifi icon, and its menu is three entries: open settings, pause
//! listening, quit. Pause is here rather than only in settings because
//! "stop listening, now" is the one action you want without opening
//! anything (a private call, a doctor's appointment).
//!
//! On GNOME this needs the AppIndicator extension, which Ubuntu ships and
//! enables by default (`ubuntu-appindicators@ubuntu.com`); without it the
//! icon simply never appears, which is why the README says so out loud.

use crate::settings::SettingsState;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager};

/// The tray icon, kept monochrome so it reads on the dark top bar. Bundled
/// into the binary rather than loaded from a resource path: in a `cargo
/// run` dev session there are no bundled resources to resolve, and an app
/// whose only UI is this icon must never start without it.
const ICON_PNG: &[u8] = include_bytes!("../icons/tray.png");

pub struct TrayState {
    /// Never read, but dropping it removes the icon from the panel -- and
    /// with it the app's only UI. Held for exactly that reason.
    #[allow(dead_code)]
    pub icon: TrayIcon,
    pub pause_item: MenuItem<tauri::Wry>,
}

pub fn build(app: &AppHandle) -> Result<(), String> {
    let paused = app.state::<SettingsState>().get().paused;
    let settings_item = MenuItem::with_id(app, "settings", "Configuración…", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let pause_item = MenuItem::with_id(app, "pause", pause_label(paused), true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let quit_item =
        MenuItem::with_id(app, "quit", "Salir", true, None::<&str>).map_err(|e| e.to_string())?;
    let separator = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let menu = Menu::with_items(app, &[&settings_item, &pause_item, &separator, &quit_item])
        .map_err(|e| e.to_string())?;

    let image = tauri::image::Image::from_bytes(ICON_PNG).map_err(|e| e.to_string())?;
    let icon = TrayIconBuilder::with_id("livesubs")
        .icon(image)
        .icon_as_template(true)
        .tooltip("LiveSubs")
        .menu(&menu)
        // Left-click opens the menu too (GNOME's indicator area does this
        // by itself, but saying so keeps the behaviour identical if this
        // ever runs under a different panel).
        .show_menu_on_left_click(true)
        .on_menu_event(handle_menu_event)
        .build(app)
        .map_err(|e| e.to_string())?;

    app.manage(TrayState { icon, pause_item });
    Ok(())
}

pub fn pause_label(paused: bool) -> &'static str {
    if paused {
        "Reanudar escucha"
    } else {
        "Pausar escucha"
    }
}

fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "settings" => crate::show_settings_window(app),
        "pause" => {
            if let Err(error) = crate::toggle_paused(app) {
                crate::pipeline::emit_status(app, "error", error);
            }
        }
        "quit" => {
            // Stop capture before exiting so the `parec` children go with
            // us rather than being reparented and left recording.
            app.state::<crate::pipeline::Pipeline>().stop();
            app.exit(0);
        }
        _ => {}
    }
}

/// Keep the menu entry's text in step with the actual state, so the tray
/// never offers to "pause" something that is already paused.
pub fn refresh_pause_label(app: &AppHandle, paused: bool) {
    if let Some(tray) = app.try_state::<TrayState>() {
        let _ = tray.pause_item.set_text(pause_label(paused));
    }
}
