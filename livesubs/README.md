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

What each platform can and cannot do is in its own README. The headline
difference: on the desktop, "system audio" means everything the machine
plays, including a video call. On Android it means media only — no app can
capture the other side of a call, by design.
