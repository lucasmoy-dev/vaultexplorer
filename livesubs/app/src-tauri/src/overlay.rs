//! The subtitle window: on top of everything, click-through, and where
//! the user put it.
//!
//! Three properties make it an overlay rather than a window:
//!
//! * `always_on_top` + `skip_taskbar` + no decorations -- it sits over the
//!   video call, and never shows up in the switcher or the dock.
//! * `ignore_cursor_events` -- every click passes straight through to
//!   whatever is underneath. Without it, a transparent 80%-wide strip
//!   across the screen would swallow clicks on the app you're actually
//!   using, which is far worse than having no subtitles.
//! * A transparent background painted by the page, not the window: the
//!   plate behind the text is CSS, so its colour and opacity are settings
//!   rather than a rebuild.
//!
//! The window is declared *resizable* in `tauri.conf.json` even though
//! nobody can grab it (no decorations, and it ignores the pointer): a
//! window GTK considers fixed-size keeps a minimum-size hint derived from
//! its initial geometry, which silently floored the overlay at ~200px tall
//! -- and the window manager then shoved it upwards to keep that height
//! out of the dock's reserved space, so it sat ~30px off its configured
//! margin. Resizable means the app can size it to the exact height the
//! settings ask for.
//!
//! This runs on X11 (checked: Ubuntu 24.04, GNOME 46 on Xorg), which is
//! what lets the app place the window itself. On a Wayland session a
//! client cannot position its own surfaces, and this would need
//! wlr-layer-shell (which Mutter doesn't implement) or a GNOME Shell
//! extension -- see the README.

use crate::settings::{Anchor, Config};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager};

pub const LABEL: &str = "overlay";

/// Size and place the overlay for `config`, then show it. Called at
/// startup and after every settings change -- cheap, and the alternative
/// (recreating the window) would flash.
///
/// At startup this has to be applied more than once (see `apply_settling`):
/// the first request can land while GTK still has the window's initial
/// geometry pending, and the result is an overlay a few dozen pixels off
/// from where the settings say it should be.
pub fn apply(app: &AppHandle, config: &Config) -> Result<(), String> {
    let window = app
        .get_webview_window(LABEL)
        .ok_or("la ventana de subtítulos no existe")?;

    // The monitor the overlay is currently on, falling back to the
    // primary one -- at startup the window hasn't been mapped anywhere
    // yet, so `current_monitor` can legitimately be None.
    let monitor = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .or(window.primary_monitor().map_err(|e| e.to_string())?)
        .ok_or("no se detectó ningún monitor")?;
    let scale = monitor.scale_factor();
    let screen = monitor.size().to_logical::<f64>(scale);
    let origin = monitor.position().to_logical::<f64>(scale);

    let (x, y, width, height) =
        geometry(config, screen.width, screen.height, origin.x, origin.y);
    // `LIVESUBS_DEBUG=1` prints what the app thinks the screen is and
    // where it decided to put the overlay. Worth keeping: every "the
    // subtitles are in the wrong place" report (fractional scaling, a
    // second monitor, a panel reserving space) is answered by these two
    // lines plus `xwininfo`.
    if std::env::var("LIVESUBS_DEBUG").is_ok() {
        eprintln!(
            "[livesubs] monitor {}x{} @{} origin {},{} -> overlay {}x{} at {},{}",
            screen.width, screen.height, scale, origin.x, origin.y, width, height, x, y
        );
    }

    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    window
        .set_position(LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    // Re-asserted on every apply rather than trusted from the manifest:
    // a window manager that drops always-on-top (or a settings change
    // that recreated nothing) shouldn't quietly leave the overlay behind
    // the call it is meant to caption.
    window.set_always_on_top(true).map_err(|e| e.to_string())?;
    // WebKitGTK paints an opaque webview background even in a window
    // declared transparent; clearing it is what makes the corners around
    // the subtitle plate actually see-through.
    let _ = window.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 0)));
    // Order matters: click-through is implemented as an empty input shape
    // on the *realised* GDK window, so asking for it before the window has
    // been shown aborts the process inside GTK (tao unwraps the missing
    // gdk::Window). Show first, then make it transparent to the pointer.
    window.show().map_err(|e| e.to_string())?;
    window.set_ignore_cursor_events(true).map_err(|e| e.to_string())?;
    Ok(())
}

/// Apply, then re-apply a couple of times over the first second.
///
/// Placing a window is a conversation with the window manager, and at
/// startup the answer can arrive after our request: measured on GNOME 46
/// (Xorg), a single apply at `RunEvent::Ready` left the overlay with its
/// manifest height and ~30px off its configured margin, while the very
/// same call a moment later placed it exactly. Re-asserting is idempotent
/// and invisible (the window is transparent), which is a much better
/// trade than a subtitle strip sitting in the wrong place all session.
pub fn apply_settling(app: &AppHandle, config: &Config) {
    let app = app.clone();
    let config = config.clone();
    std::thread::spawn(move || {
        for delay in [0u64, 250, 700] {
            if delay > 0 {
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
            let handle = app.clone();
            let for_closure = app.clone();
            let config = config.clone();
            // Window geometry has to be touched from the main thread.
            let _ = handle.run_on_main_thread(move || {
                if let Err(error) = apply(&for_closure, &config) {
                    crate::pipeline::emit_status(&for_closure, "error", error);
                }
            });
        }
    });
}

/// Geometry for one configuration, as logical pixels: `(x, y, width,
/// height)`. Split out from `apply` so the placement rules are testable
/// without a display server -- and so "where does a bottom-anchored
/// overlay go on the second monitor" has one answer, not two.
///
/// The height leaves room for `max_lines` of text plus the plate's
/// padding, generously: a clipped descender looks broken, and the window
/// is transparent wherever nothing is drawn, so being too tall costs
/// nothing.
fn geometry(
    config: &Config,
    screen_width: f64,
    screen_height: f64,
    origin_x: f64,
    origin_y: f64,
) -> (f64, f64, f64, f64) {
    let width = (screen_width * config.width_percent.clamp(20, 100) as f64 / 100.0).round();
    let line_height = config.font_size as f64 * 1.45;
    let height =
        (line_height * config.max_lines.clamp(1, 5) as f64 + config.font_size as f64 * 1.6).round();
    let x = origin_x + (screen_width - width) / 2.0;
    let margin = config.margin as f64;
    let y = match config.anchor {
        Anchor::Bottom => origin_y + screen_height - height - margin,
        Anchor::Center => origin_y + (screen_height - height) / 2.0,
        Anchor::Top => origin_y + margin,
    };
    (x, y.max(origin_y), width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::default()
    }

    #[test]
    fn bottom_anchored_overlay_is_centred_and_above_the_edge() {
        let cfg = config(); // bottom, 80% wide, 90px margin
        let (x, y, w, h) = geometry(&cfg, 1920.0, 1080.0, 0.0, 0.0);
        assert_eq!(w, 1536.0);
        assert_eq!(x, (1920.0 - 1536.0) / 2.0);
        assert_eq!(y + h + cfg.margin as f64, 1080.0);
    }

    #[test]
    fn a_second_monitor_offset_is_respected() {
        let cfg = config();
        let (x, y, w, _) = geometry(&cfg, 1920.0, 1080.0, 1920.0, 0.0);
        assert_eq!(x, 1920.0 + (1920.0 - w) / 2.0);
        assert!(y > 0.0);
    }

    #[test]
    fn top_and_centre_anchors_land_where_they_say() {
        let mut cfg = config();
        cfg.anchor = Anchor::Top;
        let (_, y_top, _, _) = geometry(&cfg, 1920.0, 1080.0, 0.0, 0.0);
        assert_eq!(y_top, cfg.margin as f64);

        cfg.anchor = Anchor::Center;
        let (_, y_mid, _, h) = geometry(&cfg, 1920.0, 1080.0, 0.0, 0.0);
        assert_eq!(y_mid, (1080.0 - h) / 2.0);
    }

    #[test]
    fn a_big_font_never_pushes_the_window_off_the_top() {
        let mut cfg = config();
        cfg.font_size = 96;
        cfg.max_lines = 5;
        cfg.margin = 600; // absurd on a small screen, on purpose
        let (_, y, _, _) = geometry(&cfg, 1366.0, 768.0, 0.0, 0.0);
        assert!(y >= 0.0, "y was {y}");
    }

    #[test]
    fn width_percentage_is_clamped_to_something_sane() {
        let mut cfg = config();
        cfg.width_percent = 500;
        let (x, _, w, _) = geometry(&cfg, 1920.0, 1080.0, 0.0, 0.0);
        assert_eq!(w, 1920.0);
        assert_eq!(x, 0.0);
    }
}
