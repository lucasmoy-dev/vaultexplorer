// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // GTK3/WebKitGTK's default overlay scrollbars fade the thumb in/out on
    // hover/scroll rather than staying put -- reads as "flickering" next to
    // our own always-visible ::-webkit-scrollbar CSS, which this setting
    // otherwise gets overridden by at the toolkit level. Must be set before
    // GTK initializes (i.e. before Tauri's own setup), so it can't just live
    // in `run()`.
    #[cfg(target_os = "linux")]
    std::env::set_var("GTK_OVERLAY_SCROLLING", "0");

    // Known WebKitGTK visual-corruption workaround on Mesa/Intel (this
    // machine logs "experimental Xe KMD" from Mesa at startup): the DMA-BUF
    // renderer path is the one most commonly implicated in resize-time
    // tearing/flash artifacts on that stack. A transparent, undecorated
    // window (both used here for the frosted-glass UI) is inherently more
    // exposed to this class of bug since there's no opaque native
    // decoration to mask a frame the compositor hasn't caught up on yet --
    // this reduces it, but on Linux/WebKitGTK it may not fully disappear.
    #[cfg(target_os = "linux")]
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");

    vaultexplorer_lib::run()
}
