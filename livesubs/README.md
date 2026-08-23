# LiveSubs

Live subtitles for everything a machine hears — the microphone *and* what is
playing — drawn on top of every other window, translated locally.

Two apps, one engine:

| | |
| --- | --- |
| [`app/`](app/README.md) | **Ubuntu desktop** (Tauri + Rust). Tray icon, click-through X11 overlay, `parec` capture, Argos translation. |
| [`android/`](android/README.md) | **Android** (Kotlin + Compose). Foreground service, `TYPE_APPLICATION_OVERLAY`, `AudioRecord` + playback capture, ML Kit translation. |
| [`core/`](core) | The shared engine: voice activity detection, whisper.cpp, and the rules for what counts as a caption. Compiled into the desktop binary directly and into the APK through `android/jni`. |

The split is deliberate. Speech detection thresholds, chunking and text
cleaning are the part that must not drift between platforms, so they exist
once, in Rust, with their own tests. Everything platform-shaped — audio
capture, windows, translation engines, settings storage — is written twice
because on these two systems they have nothing in common: `parec` and a
monitor source against `AudioRecord` and `MediaProjection`; an X11
override-redirect window against a `WindowManager` overlay; Argos in a
virtualenv against ML Kit.

## Get it

**Android:** the APK is attached to the release —
[livesubs-v0.1.0](https://github.com/lucasmoy-dev/vaultexplorer/releases/tag/livesubs-v0.1.0).
Once installed it updates itself from Configuración → Actualizaciones.

**Desktop:** built from source (see [`app/README.md`](app/README.md)); its
settings window checks for new releases and opens the page.

## Publishing a version

Releases live in this monorepo, tagged per app — `livesubs-vX.Y.Z`, the same
convention `life-framework-v…` uses — with the built APK attached as an
asset. That prefix is how both updaters know which releases are theirs.

```bash
# 1. bump the version in the three places it appears
#    android/app/build.gradle.kts   (versionName + versionCode)
#    app/src-tauri/Cargo.toml       (version)
#    app/src-tauri/tauri.conf.json  (version)

# 2. build the APK (signed with the key in android/keystore.properties)
cd android && gradle :app:assembleRelease

# 3. tag and publish
cp app/build/outputs/apk/release/app-release.apk /tmp/livesubs-X.Y.Z-arm64.apk
gh release create livesubs-vX.Y.Z   --repo lucasmoy-dev/vaultexplorer   --title "LiveSubs vX.Y.Z (Android)"   --notes-file notes.md   /tmp/livesubs-X.Y.Z-arm64.apk

# 4. check the updaters can see it
cd ../app/src-tauri && cargo test --release --features custom-protocol -- --ignored release_convention
```

Use the same signing key every time: a sideloaded APK can only be updated
in place by one signed identically. Losing it means every phone has to
uninstall and reinstall, losing its settings.

What each platform can and cannot do is in its own README. The headline
difference: on the desktop, "system audio" means everything the machine
plays, including a video call. On Android it means media only — no app can
capture the other side of a call, by design.
