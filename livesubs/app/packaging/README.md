# Packaging bits

`livesubs.desktop` is what makes the app appear in the GNOME app grid and,
copied into `~/.config/autostart/`, what starts it with the session:

```bash
install -Dm644 livesubs.desktop ~/.local/share/applications/livesubs.desktop
install -Dm644 ../src-tauri/icons/icon.png ~/.local/share/icons/hicolor/512x512/apps/livesubs.png
install -Dm755 ../src-tauri/target/release/livesubs ~/.local/bin/livesubs
# and, to have it running after every login:
cp ~/.local/share/applications/livesubs.desktop ~/.config/autostart/
```

`Exec=livesubs` assumes the binary is on `PATH` (`~/.local/bin` is, on
Ubuntu). A `.deb`/AppImage built with the Tauri CLI installs its own
desktop entry and doesn't need any of this.
