//! Dragging real files out of the window, into another app (Chrome's
//! upload box, a chat, a terminal).
//!
//! This used to go through `tauri-plugin-drag`, which on Linux starts the
//! GTK drag with *no triggering event* (`drag_begin_with_coordinates(...,
//! None, -1, -1)`) from an IPC callback, long after the mouse press. GTK
//! then has no device or timestamp to grab the pointer with, and the
//! button press is still held by the webview's own implicit grab. In a
//! test X server the drag "began" but no drop ever reached a plain GTK
//! drop target; on the desktop the release went to the window underneath,
//! which Chrome took as a click. Its URIs were also `file://` + the raw
//! path, so a name with a space ("search google.html") was not a valid URI.
//!
//! Here the webview's last left-button press is kept (a handler that runs
//! before WebKit's own) and handed to GTK as the event that started the
//! drag, and paths become URIs through GLib.

use base64::Engine;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;

thread_local! {
    // Main (GTK) thread only: both the press handler and `begin` run there.
    static LAST_PRESS: RefCell<Option<gdk::Event>> = const { RefCell::new(None) };
    static HANDLERS: RefCell<Vec<(gtk::ApplicationWindow, glib::SignalHandlerId)>> = const { RefCell::new(Vec::new()) };
}

/// Starts remembering this webview's mouse presses. Idempotent: called on
/// every page load of every window.
pub fn hook(webview: &tauri::Webview) {
    let _ = webview.with_webview(|pv| {
        let wv = pv.inner();
        // SAFETY: the key is only ever set to `bool` here.
        unsafe {
            if wv.data::<bool>("vx-dragout-hook").is_some() {
                return;
            }
            wv.set_data("vx-dragout-hook", true);
        }
        wv.connect_button_press_event(|_, ev| {
            if ev.button() == 1 {
                let event: &gdk::Event = ev;
                LAST_PRESS.with(|p| *p.borrow_mut() = Some(event.clone()));
            }
            glib::Propagation::Proceed
        });
    });
}

#[tauri::command]
pub fn start_file_drag(window: tauri::WebviewWindow, paths: Vec<String>, image: Option<String>) -> Result<(), String> {
    let w = window.clone();
    window
        .run_on_main_thread(move || {
            if let Ok(gtk_window) = w.gtk_window() {
                if let Err(e) = begin(&gtk_window, &paths, image.as_deref()) {
                    eprintln!("dragout: {e}");
                }
            }
        })
        .map_err(|e| e.to_string())
}

fn begin(win: &gtk::ApplicationWindow, paths: &[String], image: Option<&str>) -> Result<(), String> {
    let uris: Vec<String> = paths
        .iter()
        .filter_map(|p| glib::filename_to_uri(p, None).ok().map(|u| u.to_string()))
        .collect();
    if uris.is_empty() {
        return Err("nothing to drag".into());
    }
    clear_handlers();
    let targets = gtk::TargetList::new(&[]);
    targets.add_uri_targets(0);
    let data_id = win.connect_drag_data_get(move |_, _, data, _, _| {
        let refs: Vec<&str> = uris.iter().map(|s| s.as_str()).collect();
        data.set_uris(&refs);
    });
    let end_id = win.connect_drag_end(|_, _| {
        // Deferred: disconnecting a handler from inside its own emission.
        glib::idle_add_local_once(clear_handlers);
    });
    HANDLERS.with(|h| {
        let mut h = h.borrow_mut();
        h.push((win.clone(), data_id));
        h.push((win.clone(), end_id));
    });
    let event = LAST_PRESS.with(|p| p.borrow().clone());
    let ctx = win
        .drag_begin_with_coordinates(&targets, gdk::DragAction::COPY, 1, event.as_ref(), -1, -1)
        .ok_or("GTK refused to start the drag")?;
    if let Some(pixbuf) = image.and_then(decode_png) {
        ctx.drag_set_icon_pixbuf(&pixbuf, pixbuf.width() / 2, pixbuf.height() / 2);
    }
    Ok(())
}

fn clear_handlers() {
    HANDLERS.with(|h| {
        for (win, id) in h.borrow_mut().drain(..) {
            win.disconnect(id);
        }
    });
}

fn decode_png(data_url: &str) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    let b64 = data_url.strip_prefix("data:image/png;base64,")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.write(&bytes).ok()?;
    loader.close().ok()?;
    loader.pixbuf()
}
