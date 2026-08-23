# YT Pocket

Search YouTube on the phone and keep the file: **MP3** (audio, tagged) or
**MP4** (video + audio), named after the video and dropped straight into
`Music/YT Pocket` or `Movies/YT Pocket` where the music player and the
gallery find it.

Android only, arm64, no account and no API key.

## Install

The APK is attached to the latest
[`ytpocket-v…` release](https://github.com/lucasmoy-dev/vaultexplorer/releases).
Open it on the phone and allow installing from this source.

After that it updates itself: the **Actualizaciones** card at the bottom of
the screen checks this repo's releases, downloads the APK and hands it to
the system installer, which asks you to confirm. Nothing installs silently.

## How to use it

Type a search, or paste a link (or share a video to YT Pocket from the
YouTube app — it lands in the search box). Then **MP3** or **MP4** on the
result you want. The download runs in a notification, so you can leave the
app; tapping the finished notification opens the file.

## What is actually hard here, and how it's handled

- **YouTube stopped serving progressive streams.** No client gets a file
  with video and audio together any more, so "download the MP4" is two
  downloads plus a mux. Android has no ffmpeg, so the muxing is the
  platform's own `MediaMuxer`, copying both tracks without re-encoding
  (seconds, not minutes, and lossless).
- **Which streams.** AAC audio and AVC/H.264 video, chosen deliberately over
  the higher-bitrate Opus and the 4K VP9/AV1 that YouTube also offers: those
  two are what `MediaMuxer` and every device decoder take without argument,
  and AAC is what the MP3 encoder can decode. That caps video at 1080p,
  which is the right trade for a file that plays everywhere.
- **Android has no MP3 encoder.** It has an AAC *decoder* and never shipped
  an MP3 *encoder*, so `MediaCodec` cannot get from one to the other. The
  transcode is `symphonia` + LAME in Rust (see `jni/src/mp3.rs`), with the
  title and channel written as ID3 tags.
- **googlevideo throttles plain downloads.** Measured: a 3.4MB audio track
  read as one sequential stream timed out at 30s; the same file fetched as
  4MB `Range` chunks arrives in about two seconds. Both the app and the
  end-to-end test fetch in chunks for that reason.
- **403 Forbidden, the interesting one.** 0.1.1 downloaded fine from a
  laptop and was refused on a phone. Two causes, both now handled:
  - Since August 2024 YouTube requires a **PO token** for streams from its
    *web* clients (Desktop, Mobile) — the resolve succeeds and the download
    is then refused with 403. Generating one needs a simulated browser
    (rustypipe delegates it to a separate CLI binary), which a phone app
    cannot carry, so those clients are no longer allowed to mint download
    URLs at all. Only iOS, TV and Android are, and iOS is preferred because
    it needs neither a token nor signature deobfuscation.
  - A googlevideo URL is minted **for the client that asked for it**, so
    downloading with a different HTTP stack (Kotlin's `HttpURLConnection`,
    with its own headers, TLS and possibly a different IP family) can be
    refused on its own. The download therefore runs natively now, through
    the same kind of client that resolved it, with that client's own
    `User-Agent`.
  - Belt and braces: `resolve` **probes** each candidate with a 1KB range
    request before returning it, so a URL that will not serve bytes never
    reaches the UI; and a refusal part way through a transfer re-resolves
    once and retries with fresh URLs.
- **Filenames.** The point of the app, so the rules live in Rust with tests
  (`jni/src/naming.rs`): `/ \ : * ? " < > |` become dashes ("AC/DC" reads
  "AC-DC", not "ACDC"), control characters and repeated whitespace collapse,
  trailing dots go, accents/emoji/CJK stay, and the name is capped at 120
  **bytes** without splitting a character — the limit filesystems enforce is
  on bytes, and a Japanese title hits it three times sooner than an English
  one.
- **Where files land.** `MediaStore` with `IS_PENDING`, which needs no
  storage permission at all and keeps the file invisible until it is
  complete, so no music player indexes a half download.
- **Livestreams** have no file to download; their buttons are disabled and
  the row says so, instead of failing three steps later.

## Layout

```
jni/    Rust: YouTube search + stream resolution (rustypipe), filename rules,
        AAC -> MP3 (symphonia + LAME). One .so, no C++ runtime needed.
app/    Kotlin + Compose: one screen, a foreground service for downloads,
        MediaMuxer, MediaStore, and the in-app updater.
```

`rustypipe` is the same extractor (and version) the sibling
[vaultexplorer](../vaultexplorer) app already ships on Android — a known-good
combination rather than a fresh bet on someone's parser. It also means the
same lesson applies: **YouTube's iOS client is the one that reliably works**,
so `resolve` walks a short list (iOS, TV, Android — no web clients, see the
403 note above), twice, and takes the first whose streams actually serve
bytes, rather than pinning one and breaking on YouTube's next change.

## Build

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk
sdkmanager --install "platforms;android-35" "build-tools;35.0.0" "ndk;26.3.11579264"
echo "sdk.dir=$HOME/Android/Sdk" > local.properties

gradle :app:assembleRelease
adb install -r app/build/outputs/apk/release/app-release.apk
```

Gradle builds the Rust library itself, so that is the whole command. **NDK
r26 on purpose:** r27 dropped the reverse processor→ABI map from its
`abis.cmake`, and any dependency that hands CMake a `CMAKE_SYSTEM_PROCESSOR`
then fails with `Unknown processor 'aarch64'`.

Release builds sign with `keystore.properties` (gitignored), falling back to
the debug key. Keep the key: a sideloaded APK can only be updated in place
by one signed identically.

## Verifying without a device

```bash
# Filenames, link parsing, and a real AAC -> tagged MP3 transcode
cd jni && cargo test --release

# Live: search YouTube, resolve streams, assert AAC + AVC came back
cargo test --release -- --ignored --nocapture search_and_resolve

# Live, the whole promise: resolve -> download -> MP3 named after the video,
# tags and duration checked against what YouTube said
cargo test --release -- --ignored --nocapture download_and_transcode

# Live: the chunked native download, byte-exact against Content-Range
cargo test --release -- --ignored --nocapture chunked_download

# Live: the 403 root cause -- asserts an iOS stream serves and a web-client
# one does not. If this ever fails, YouTube changed its mind about PO tokens
# and the client list can grow again.
cargo test --release -- --ignored --nocapture a_web_client_url_is_refused

# The JNI boundary itself, from a real JVM, against real YouTube
./tools/harness/run.sh "search terms"

# Kotlin's own logic, and Android API misuse
cd .. && gradle :app:testDebugUnitTest :app:lintRelease
```

The last two are the ones that earn their keep: the harness is the only
check that the JNI symbol names and the JSON contract line up (the failure
mode on a phone is a silent `UnsatisfiedLinkError`), and `lint` is what
caught a muxer bug — `MediaExtractor`'s sample flags and
`MediaCodec.BufferInfo`'s buffer flags are two different sets that overlap,
and copying one into the other would have labelled ordinary frames as codec
configuration and produced files that do not play.

## Known limits

- **YouTube changes and this breaks.** That is the nature of an extractor;
  the fix is usually a `rustypipe` bump. The live tests above are the canary.
- **arm64 only**, and Android 10+ (`MediaStore`'s pending-file flow).
- **Not tested on a physical device by its author** — no phone was attached
  when it was built. Everything above the platform APIs is verified as
  described; the first run on a phone is still the first run on a phone.
- **Downloading from YouTube is against their terms of service.** This is a
  personal tool for keeping copies of things you could already play; that
  decision is the user's.
