# LiveSubs for Android

The phone version of [LiveSubs](../app/README.md): live subtitles for what
the microphone hears and for what other apps are playing, drawn on top of
everything else, translated on the device.

Same engine as the desktop app — the VAD, whisper.cpp and the rules for what
counts as a caption live in [`../core`](../core) and are compiled into the
APK through a small JNI shim ([`jni/`](jni)). A fix to speech detection
lands on both platforms at once, because there is only one implementation.

## What it does

- **Two sources, two colours.** The microphone (your voice) and playback
  audio (video, music, browser) are captured as separate streams, each drawn
  in its own colour, exactly like the desktop app.
- **Over every app.** A `TYPE_APPLICATION_OVERLAY` window with
  `FLAG_NOT_TOUCHABLE`: it floats above whatever you are using and every
  touch passes straight through it.
- **Local recognition.** whisper.cpp on the phone's CPU. Nothing is uploaded.
- **Translation on device.** ML Kit, English/Spanish/French in any
  direction, offline after a ~30MB download per language.
- **Automatic language detection**, per utterance, or fixed when you know it.
- **Transcript** appended to a text file you pick, one line per utterance,
  with the time, the source and the language.
- **The notification is the control panel** — pause/resume, settings, stop.
  A phone has no system tray, and an app that keeps recording with its
  window closed *must* be a foreground service with a notification, so that
  notification is where the desktop version's tray menu went.

## The one hard limitation: calls

**The other side of a call cannot be captured.** Not by this app, not by any
app. Android's playback capture API only hears streams whose audio policy
allows it, and every calling app (Meet, Zoom, WhatsApp, Teams, the dialer)
marks its audio `USAGE_VOICE_COMMUNICATION`, which is never capturable. So:

| Source | Captured? |
| --- | --- |
| Your microphone | Yes |
| YouTube, video files, music, browser, podcasts | Yes, unless the app opts out |
| The remote side of a call | **No** |

The workaround is the obvious one: put the call on speaker and let the
microphone hear it. Quality is worse and both sides land in the microphone
colour, but it works, and it is the only thing that does.

## Permissions, and why each one

| Permission | Why | How it's granted |
| --- | --- | --- |
| Microphone | The mic half of the app | Normal dialog |
| Display over other apps | The overlay window | You allow it in Settings; the app opens the right screen |
| Notifications | The service's control panel | Normal dialog (Android 13+) |
| Screen capture consent | Playback capture is gated behind the same permission as screen recording | The system dialog, each time capture starts |

Declining screen capture is not fatal: the microphone half keeps working and
the notification says so.

## Build

Needs the Android SDK, **NDK r26**, Rust with the `aarch64-linux-android`
target, and `cargo-ndk`:

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk
sdkmanager --install "platforms;android-35" "build-tools;35.0.0" "ndk;26.3.11579264"
echo "sdk.dir=$HOME/Android/Sdk" > local.properties

gradle :app:assembleRelease      # or assembleDebug
adb install -r app/build/outputs/apk/release/app-release.apk
```

Gradle builds the Rust library itself (`cargoNdkBuild`), so there is one
command, not two.

**Why NDK r26 and not the newest:** `whisper-rs-sys` hands CMake a
`CMAKE_SYSTEM_PROCESSOR`, and r27 dropped the reverse processor→ABI mapping
(`NDK_PROC_aarch64_ABI`) from its `abis.cmake`. With r27 the build dies on
`Android: Unknown processor 'aarch64'`. r26 still ships it.

**Signing:** release builds read `keystore.properties` (gitignored) and fall
back to the debug key when it is absent. Use a stable key — an APK
sideloaded onto a phone can only be updated in place by an APK signed with
the same one.

## Verifying without a device

The APK cannot be exercised on a laptop, but most of what it does can:

```bash
# The shared engine: VAD thresholds, chunking, text cleaning
cd ../core && cargo test

# The JNI boundary, on real audio, over a real JVM: builds the library for
# this machine, calls it from Java exactly as Kotlin does, prints captions
cd ../android/jni && ./tools/harness/run.sh base

# The Kotlin logic that is pure logic: plate colour, restart rules, caption parsing
cd .. && gradle :app:testDebugUnitTest

# Android API misuse (this is what catches "that constant needs API 30")
gradle :app:lintRelease
```

The harness is the interesting one: it is the only check that the JNI symbol
names, argument marshalling and JSON contract actually line up, and its
failure mode on a phone would otherwise be a silent `UnsatisfiedLinkError`.

## How it fits together

```
AudioRecord (mic) ─────┐                                  ┌─ overlay window
                       ├─ NativeEngine (JNI) ─ ../core ─┬─┤   (click-through)
AudioRecord (playback) ┘   VAD + whisper.cpp            │ └─ transcript (SAF)
   via MediaProjection                                  └─ ML Kit translate
```

- **`CaptionService`** — foreground service: one thread per source running
  capture → VAD → whisper → translation, plus the notification, the overlay
  and the restart rules (only the model, the sources and the spoken language
  require a restart; colours, position and target language apply live).
- **`Capture`** — the two `AudioRecord` configurations, both 16kHz mono
  float, which is what whisper wants, so nothing resamples.
- **`OverlayController`** — the floating window and the caption lines.
- **`Translations`** — ML Kit, one client per direction, models fetched
  before the first caption needs them.
- **`Prefs`** — the same setting names and defaults as the desktop app.
- **`NativeEngine` / `CaptionJson`** — the JNI boundary, and the parsing kept
  out of it so it stays testable without the `.so`.

## Known limits

- **Calls.** See above. This is a platform decision, not a missing feature.
- **Apps that opt out.** Some apps set `ALLOW_CAPTURE_BY_NONE`; their audio
  is silent to this app, with no way to tell from the outside.
- **Overlapping audio.** Playback capture is a mix. Two things playing at
  once reach whisper as one overlapping utterance and come back garbled;
  segments whisper flags as probably-not-speech are dropped, which removes
  most of the invented captions but cannot separate two voices.
- **arm64 only.** Every phone worth running whisper on has been arm64 for
  years; a second ABI doubles the build and the APK for nothing.
- **Battery.** Two whisper streams on a phone CPU is real work. Turning off
  the source you are not using roughly halves it, and pause is one tap away
  in the notification.
- **Not tested on a physical device by its author** (there was none attached
  when it was built). Everything above the platform APIs is verified as
  described in "Verifying without a device"; the first run on a phone is
  still the first run on a phone.
