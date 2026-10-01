//! Bringing a window to the front when *another* app asked for it -- a
//! screenshot notification's "Show in Files", OBS's "Show Recordings", a
//! browser's "Show in folder".
//!
//! `set_focus()` alone doesn't do it: GTK presents the window with this
//! process's own last user-interaction time, which is older than the
//! click the user just made in the other app, so the window manager's
//! focus-stealing prevention (mutter's, KWin's) keeps the window *behind*
//! everything else -- or opens a new one there. From the user's side,
//! clicking "Show in Files" did nothing at all.
//!
//! On X11 the request is therefore also sent the way a taskbar/pager sends
//! it: an EWMH `_NET_ACTIVE_WINDOW` client message with source indication
//! 2 ("pager") and timestamp 0 ("now"), which window managers treat as the
//! user's own request. Wayland has no equivalent without an activation
//! token from the caller, so there it falls back to `present()` plus an
//! urgency hint, which at least flashes the window in the dock.

use tauri::{Manager, Runtime, WebviewWindow};

#[cfg(target_os = "linux")]
fn net_active_window(xid: u32) -> Result<(), Box<dyn std::error::Error>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask};
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    let atom = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW")?.reply()?.atom;
    // data: [source indication = 2 (pager), timestamp = 0 (now), requestor's
    // currently active window = 0, 0, 0]
    let event = ClientMessageEvent::new(32, xid, atom, [2u32, 0, 0, 0, 0]);
    conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    )?;
    conn.flush()?;
    Ok(())
}

/// Show, unminimize and raise `window`, even when this app isn't the one
/// the user is interacting with right now.
pub fn raise<R: Runtime>(window: &WebviewWindow<R>) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    #[cfg(target_os = "linux")]
    {
        let w = window.clone();
        let _ = window.run_on_main_thread(move || {
            use gtk::prelude::*;
            let Ok(gtk_window) = w.gtk_window() else { return };
            gtk_window.present();
            let xid = gtk_window
                .window()
                .and_then(|gdk_window| gdk_window.downcast::<gdkx11::X11Window>().ok())
                .map(|x11| x11.xid() as u32);
            match xid {
                Some(xid) => {
                    if let Err(e) = net_active_window(xid) {
                        eprintln!("raise: _NET_ACTIVE_WINDOW failed: {e}");
                    }
                }
                None => {
                    let _ = w.request_user_attention(Some(tauri::UserAttentionType::Critical));
                }
            }
        });
    }
}

/// `raise` for a window that was only just created: it has to be mapped
/// before the window manager will act on an activation request for it, so
/// this waits a moment (off the main thread) and raises it then.
pub fn raise_when_mapped<R: Runtime>(app: &tauri::AppHandle<R>, label: String) {
    let app = app.clone();
    std::thread::spawn(move || {
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(150));
            if let Some(w) = app.get_webview_window(&label) {
                if w.is_visible().unwrap_or(false) {
                    raise(&w);
                    return;
                }
            }
        }
    });
}
