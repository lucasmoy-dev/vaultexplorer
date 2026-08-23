# RecPocket

Record what the phone hears and what it plays — separately or mixed — with
or without the screen, plus screenshots. A floating button starts and stops
it from inside whatever app you are in, and it can arm itself for WhatsApp
calls. Files are named after the date, backwards, so they sort themselves.

Android only, arm64, no account.

## Install

The APK is attached to the latest
[`recpocket-v…` release](https://github.com/lucasmoy-dev/vaultexplorer/releases).
Open it on the phone and allow installing from this source.

After that it updates itself: **Actualizaciones**, at the bottom of the
screen, checks this repo's releases, downloads the APK and hands it to the
system installer, which asks you to confirm. Nothing installs silently.

## What it records

| Setting | What lands in the file |
|---|---|
| Micrófono | Your voice and the room |
| Salida | What the phone is playing (see the limit below) |
| Las dos | Both, mixed into one track |
| + pantalla | The above, plus the screen, in one MP4 |

Quality presets exist because the point is *keeping* recordings, not
admiring them: the defaults are 32 kbps mono audio (about 14MB an hour) and
480p video. Both go up to 160 kbps and 1080p if a particular recording
deserves it.

Files land in `Music/RecPocket` or `Movies/RecPocket` (screenshots in
`Pictures/RecPocket`), named `2026-08-23_17-49-43 llamada.m4a`: the date
backwards, then what it is. That is the one naming convention that sorts
chronologically in every gallery, file manager and `ls` on earth.

## The honest limits

- **The other side of a WhatsApp call does not reach the file** unless it is
  coming out of the speaker. Android lets an app mark its audio as
  un-capturable, and calling apps do exactly that — `AudioPlaybackCapture`
  returns silence for them. No app on an unrooted phone gets around this;
  what works is the speaker plus the microphone, which is what "las dos"
  gives you.
- **Screen capture is a per-grant dialog.** Android will not let an app raise
  it from the background, so automatic recording only works if the permission
  was granted *before* the call — that is what "Activar captura" does, and
  why it stays visible in a notification while it is held.
- **Recording calls is your call.** Consent rules vary by country. The app
  records what its owner tells it to, shows a permanent notification while it
  does, and stores everything on the phone only.

## How the interesting parts work

- **Mixing two microphones' worth of sound.** `MediaRecorder` takes exactly
  one audio source, so "both sides at once" means two `AudioRecord` streams
  added together by hand (`Mixing.kt`). Two traps, both audible: 16-bit
  samples added without saturation wrap from +32767 to −32768 (a violent
  crack, not loudness), and the two streams never read the same number of
  bytes, so the shorter one has to be treated as silence rather than as the
  end of the file. Playback capture also arrives much hotter than a
  microphone, so it is scaled down before adding — otherwise the far end
  buries your own voice.
- **Two encoders, one file.** Audio (AAC) and screen (AVC, from a
  `VirtualDisplay`) feed one `MediaMuxer`, which refuses samples until every
  track has been added — so writing starts only once both formats are known,
  and stopping joins both encoder threads *before* stopping the muxer. Get
  that order wrong and the result is a zero-byte or unplayable file, which is
  what the on-device test below actually checks.
- **The floating button is not in the recording.** Its window carries
  `FLAG_SECURE`, and a secure window is never composited onto a non-secure
  display — which is exactly what a `MediaProjection` virtual display is. So
  the controls are on the screen and absent from the video and the
  screenshots, with no hide/show flicker and no race with the frame being
  captured.
- **Knowing a call started.** There is no API. WhatsApp broadcasts nothing,
  and no app can watch another's telephony state. What is observable is the
  ongoing-call notification, which a `NotificationListenerService` can read:
  `CallSignals.kt` matches its wording (in Spanish, English, Portuguese,
  French, Italian and German, accents flattened) and decides voice or video —
  video gets the screen, voice does not, because a voice call would otherwise
  cost ten times the file for a black rectangle. A missed-call notification
  is explicitly *not* a call: recording it captures silence.
- **Screenshots.** `ImageReader` on the same projection. Captured rows are
  padded to a hardware stride, so the buffer is wider than the screen —
  copying it straight into a bitmap is where the familiar diagonal-smear
  screenshot comes from; the padding is accounted for and cropped.
- **Where files go.** `MediaStore` with `IS_PENDING`: no storage permission
  at all, and nothing appears in the gallery until it is complete.
- **Light and dark.** `MaterialTheme {}` with no colour scheme is *always
  light*, which on a phone set to dark is a white app among dark ones. The
  app follows the system setting (wallpaper-derived colours on Android 12+),
  and the platform theme has a `values-night` variant because
  `Theme.DeviceDefault` has no DayNight flavour -- without it the window
  itself flashes white behind a dark app. Both halves are asserted by a test
  that reads the theme's background under each qualifier.

## Layout

```
app/src/main/java/dev/lucasmoy/recpocket/
  Recorder.kt       AudioRecord(s) -> mix -> AAC + AVC -> MediaMuxer
  Mixing.kt         the PCM arithmetic, with tests
  CaptureService.kt foreground service: holds the projection, records
  Overlay.kt        the floating controls (FLAG_SECURE)
  CallWatcher.kt    notification listener -> start/stop
  CallSignals.kt    "is this a call, and does it have video?", with tests
  Screenshot.kt     one frame, de-padded, as JPEG
  Naming.kt         the reverse-date file names, with tests
  Settings.kt       what to record, how small, when
  Output.kt         MediaStore publishing
  MainActivity.kt   the one screen
  Updater.kt        in-app updates from GitHub releases
```

## Build

```bash
sdkmanager --install "platforms;android-35" "build-tools;35.0.0"
echo "sdk.dir=$HOME/Android/Sdk" > local.properties

gradle :app:assembleRelease
adb install -r app/build/outputs/apk/release/app-release.apk
```

Release builds sign with `keystore.properties` (gitignored), falling back to
the debug key. Keep the key: a sideloaded APK can only be updated in place by
one signed identically.

## Verifying

```bash
# File names, PCM mixing, call-notification matching, settings defaults,
# and that the screen composes at all (Robolectric)
gradle :app:testDebugUnitTest :app:lintRelease

# The pipeline itself, on a running Android: records the microphone for
# three seconds and asks the platform to play the result back. An emulator
# has no microphone, so the audio is silence -- what is under test is the
# container, the timeline and the teardown, and silence exercises all three.
gradle :app:connectedDebugAndroidTest
```

The on-device test is the one that earns its keep: hand-built muxing fails by
producing a file that exists and does not play, which no amount of unit
testing catches.
