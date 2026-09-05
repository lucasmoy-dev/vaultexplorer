# HomeCloud

Sync a folder between your own devices. Point one device at another with a code
or a QR, say yes once, and the folder stays the same on both from then on.

Two-way by default. Nothing goes through anyone's server: devices talk to each
other directly, and the files are encrypted in transit.

## How it is built

HomeCloud does not implement synchronisation. It ships
[Syncthing](https://syncthing.net) as a child process and drives it over its
local REST API, which is why the sync itself is a decade-tested protocol rather
than something new. What HomeCloud adds is the part Syncthing leaves to you: one
screen, one status dot per folder, one code to pair, and opinionated defaults so
there is nothing to configure.

Syncthing was chosen over the alternatives because its relay and discovery
network is community-run and free for good, with no company able to switch it
off — and because being protocol-compatible means HomeCloud interoperates with
plain Syncthing installs.

```
core/      Rust. REST client, pairing codec, process supervisor. Shared by every platform.
app/       Tauri v2 desktop app (Ubuntu first; Windows and macOS follow).
android/   Native Kotlin + Compose. `android/jni` bridges the same core.
scripts/   Fetches and builds the engine binaries, which are not committed.
```

The desktop and the phone link against the *same* `core`, so a pairing code
written by one is read by the other by construction rather than by two
implementations agreeing.

iOS cannot run a background daemon or reach files outside its sandbox, so
automatic folder sync is not possible there. If it ever happens it will be
sync-on-open over an app-owned folder, and it will say so plainly.

## Running it

Desktop:

```bash
./scripts/fetch-engine.sh          # once: downloads the engine (~27 MB, not committed)
cd app && npm install
npm run tauri dev
```

Android (needs Go and the Android NDK):

```bash
./scripts/build-android-engine.sh  # once: builds the engine for arm64 and x86_64
cd android
gradle assembleDebug                       # a phone
gradle -PrustAbis=x86_64 assembleDebug     # the emulator
```

Tests:

```bash
cargo test --manifest-path core/Cargo.toml
```

## What has actually been tested

End to end, with real clicks and taps, between an Ubuntu desktop and an Android
emulator running the real engine:

- Pairing by code, and by accepting an invitation, in both directions.
- Files, including binaries, verified byte-identical by checksum after syncing.
- Propagation in about 2–4 seconds each way.
- Deletions propagating.
- Concurrent edits on two disconnected devices: both versions kept, nothing lost.
- A third device joining a folder two devices already shared.
- Sync continuing after the phone's UI is killed, via the foreground service.

## Known gaps

- **The phone cannot scan a QR yet.** It shows and accepts codes as text; the
  camera path needs CameraX and is the obvious next piece of the "just scan it"
  promise.
- **The engine can be orphaned on the desktop.** It is stopped when the window
  closes, but a `SIGKILL` leaves the child process running. It should be adopted
  on next launch instead of a second one being spawned.
- **Copying the code to the clipboard is unverified on the desktop.** The button
  produced no visible result on this machine (X11 + WebKitGTK); the QR and the
  on-screen code both work, and the button now reports failure instead of doing
  nothing silently.
- **Accepting takes two taps when the other device dials in first**: once to
  trust the device, once for the folder it then offers. Pasting a code is a
  single step, so this only affects the other direction.
- **Not yet tested outside a LAN**, where relays and NAT traversal come in.
- **The icons are placeholders** copied from another app in this monorepo.
