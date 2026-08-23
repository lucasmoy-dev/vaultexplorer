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
- **403 Forbidden, the interesting one.** 0.1.1 and 0.1.2 downloaded fine
  from a laptop and were refused on a phone. Causes, in the order they were
  found:
  - **PO tokens.** YouTube requires a "proof of origin" token for stream
    URLs from most of its clients, and *enforces it per network* — which is
    why the same build worked here and failed there. yt-dlp's client table
    (the best-maintained source for this) currently marks the token as
    required for `web`, `mweb`, `web_safari`, `android` **and `ios`**. Only
    four clients need none: `tv`, `tv_downgraded`, `web_embedded` and
    `visionos` — and of those, only **`visionos`** also needs no JavaScript
    player, i.e. its stream URLs arrive ready to use with no signature
    deobfuscation. So this app talks to `visionos` directly (see
    `jni/src/innertube.rs`, a hand-rolled Innertube player request) and keeps
    rustypipe's clients only as probed fallbacks. Generating a token is the
    other way out and is not available on-device without running BotGuard in
    a WebView, which is a possible future addition rather than a dependency
    this carries today.
  - A googlevideo URL is minted **for the client that asked for it**, so
    downloading with a different HTTP stack (Kotlin's `HttpURLConnection`,
    with its own headers, TLS and possibly a different IP family) can be
    refused on its own. The download therefore runs natively now, through
    the same kind of client that resolved it, with that client's own
    `User-Agent`.
  - **The URL is signed for one IP address**, and that was the cause of the
    stubborn one: "worked for 40MB, then 403". A googlevideo URL carries
    `ip=<the address that asked>` inside `sparams`, and `sparams` is the list
    of parameters the signature covers — so the address is not a hint, it is
    part of the contract. A laptop on one address downloads happily; a phone
    hands out a **rotating IPv6 privacy address** and switches between IPv4
    and IPv6 as signal changes, so part way through a transfer the caller is
    somebody else and every subsequent request is refused, in every request
    shape (which is why the phone reported the same error for `header`,
    `query` and `query+cpn` alike). Two fixes, both needed:
    - The HTTP client is **pinned to IPv4** (`local_address`), so resolving
      and downloading cannot land on different families. Measurable: the
      test below asserts the address written into the URL is the address we
      are calling from — it failed before this and passes now.
    - A refusal mid-transfer no longer restarts. It **re-resolves and
      continues from the byte it stopped at** (up to six times), so a genuine
      address change costs a round trip, not the download. `MediaStore` never
      sees the partial file, so a resumed download is still one clean file.
  - **The actual cause, found by measuring instead of reasoning:** a
    token-free client is only token-free if YouTube believes it is a client.
    Ask `visionos` for a *long, popular* video with no **`visitorData`** in
    the request context and the answer is "Sign in to confirm you're not a
    bot" -- so the app fell back to `ios`, whose URLs need a PO token, and
    those refused the download. Measured, same video, one variable:

    | client | no visitor id | with a visitor id |
    |---|---|---|
    | `visionos` | Sign in to confirm you're not a bot | 20 formats, 4MB chunk served |
    | `android_vr` | bot gate | bot gate |
    | `web_safari` | video unavailable | video unavailable |

    The visitor id comes from `youtube.com/sw.js_data`, is cached, and now
    rides along on every Innertube request. Short or obscure videos were never
    gated, which is why every earlier test passed and the phone still failed.
  - **When the gate closes anyway.** It is decided per session as well as per
    address, so a refusal throws the cached visitor id away and tries once
    with a new one. If that fails too the app says so in those words --
    "YouTube pide verificación de humano desde esta red" and suggests changing
    connection -- instead of reporting a 403 the user can do nothing about.
    Measured from a datacenter address, where YouTube refuses *every* client
    and calls it an ip-ban: no app-side change reaches that, and the live
    tests report it as a property of the network rather than failing.
  - **A 1KB probe proved nothing.** `resolve` used to check a candidate with
    a `bytes=0-1023` request; PO-token URLs answer that with 200 and answer
    `bytes=0-4194303` with **403**. So the probe passed exactly the streams
    that then died on their first real chunk. It now asks for a full
    chunk-sized range and drops the body unread -- same one round trip, an
    answer that means something.
  - Belt and braces: requests are tried as a `Range` header, then as
    `&range=` query parameters, then with a playback nonce, because
    googlevideo has served all three shapes over the years.
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
- **A "Diagnóstico" button**, because a 403 depends on the network the phone
  is on and cannot be reproduced from a laptop. It asks every client in turn
  and reports what each one did — `[{"egress_ip":"…"},{"client":"visionos",
  "resolve":"ok","download":"ok","url_signed_for_ip":"…"},{"client":"ios",
  "download":"YouTube rechazó (403)"},…]` — with a share button. The two
  addresses are in there deliberately: if they ever disagree, the 403 is the
  IP binding above and not anything else.

## Layout

```
jni/    Rust: YouTube search (rustypipe), stream resolution (a direct
        Innertube client for the token-free `visionos`, rustypipe as
        fallback), the download itself, filename rules, and AAC -> MP3
        (symphonia + LAME). One .so, no C++ runtime needed.
app/    Kotlin + Compose: one screen, a foreground service for downloads,
        MediaMuxer, MediaStore, and the in-app updater.
```

A note on what the client list is for: **`visionos` (plus a visitor id) is
what works**, and rustypipe's clients are fallbacks that mostly cannot
download. They stay because a client that resolves is still useful for
metadata and because YouTube's enforcement changes month to month -- but the
probe now tells the truth about them.

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

# Live: the bug that three releases failed to fix -- a token-free client
# needs a visitor id, and a chunk-sized range is the only probe worth
# trusting
cargo test --release -- --ignored --nocapture a_visitor_id_is_what_makes

# Live, and slow (~400MB): a *long* video's audio, whole, through the same
# chunk loop the app uses. Short videos fit in one chunk, which is how a
# mid-download refusal went unnoticed.
cargo test --release -- --ignored --nocapture a_long_download

# Live: the address invariant -- the URL is signed for the IP we call from,
# and a fresh URL can continue a download the old one stopped serving. These
# two are the 0.1.5 regression guards.
cargo test --release -- --ignored --nocapture address_tests

# Live: the 403 root cause -- asserts an iOS stream serves and a web-client
# one does not. If this ever fails, YouTube changed its mind about PO tokens
# and the client list can grow again.
cargo test --release -- --ignored --nocapture a_web_client_url_is_refused

# The JNI boundary itself, from a real JVM, against real YouTube
./tools/harness/run.sh "search terms"

# Kotlin's own logic (including the resume loop, with an injected fetcher
# that refuses on cue), and Android API misuse
cd .. && gradle :app:testDebugUnitTest :app:lintRelease
```

And the one that needs a running Android, on an emulator when no phone is
attached — the real native library, the platform muxer, `MediaStore` and the
real network, end to end:

```bash
sdkmanager --install "emulator" "system-images;android-35;google_apis;x86_64"
emulator -avd <your avd> -no-window -no-audio -gpu swiftshader_indirect &

# arm64 is the phone, x86_64 the emulator; the ABI is a build flag so the
# release APK stays arm64-only
gradle :app:connectedDebugAndroidTest -PrustAbis=x86_64
```

That test deliberately picks the **longest, most popular** result it can
find: it downloaded a 6h52m album, transcoded it, and checked the MP3's own
duration against what YouTube reported (24723s vs 24724s). Both halves of
that matter — long, because the failure only appeared past the first chunk,
and popular, because the bot gate only trips on those.

Note that a software-GPU emulator is too slow to draw Compose in time and
throws an ANR dialog when the app is launched by hand; the instrumented test
draws nothing and is unaffected.

The unit tests and lint earn their keep too: the harness is the only
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
- **Not tested on a physical phone by its author** — no phone was attached
  when it was built. It *is* tested on a real Android runtime: an x86_64
  emulator runs the whole path (see below), and the native side is tested
  against real YouTube from two networks. What an emulator cannot reproduce
  is a carrier network, which is where the 403 story began.
- **Downloading from YouTube is against their terms of service.** This is a
  personal tool for keeping copies of things you could already play; that
  decision is the user's.
