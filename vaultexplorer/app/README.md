# Vault Explorer

A file manager with real encrypted vaults built in. Browse your files like any
other explorer; turn any folder into a vault and its contents are AES-GCM
encrypted at rest (filenames included), unlocked with a password (Argon2) for
as long as you're using it.

Runs on Linux desktop and Android from the same codebase (Tauri + Rust +
React).

## Features

- **Vaults**: encrypt a folder in place. Filenames, not just contents, are
  encrypted. Optional "sensitive" sub-folders that re-lock and ask for the
  password again after a timeout, independent of the vault itself.
- **File browsing**: icon/list/column views, thumbnails, tags, favorites,
  search, archive browsing (zip) without extracting first.
- **Cloud & P2P sync**: Google Drive/OneDrive/Dropbox via `rclone`, direct
  device-to-device sync via a self-managed Syncthing instance, plain
  folder-to-folder sync via `unison` (desktop). On Android, Google Drive and
  folder-to-folder sync work too, without any of those binaries — see
  [Mobile scope](#mobile-scope). Per-file "synced" status shown right in the
  file list.
- **Recovery**: exportable recovery key for a vault in case a password is
  lost.
- **Secure delete**: real overwrite-based shredding, not just unlink.
- **Freeze**: mark a folder read-only at the filesystem level.
- **Extras**: inline text/markdown editor, audio transcription, image/video
  tools, git status in the sidebar, terminal integration (desktop).
- **Mobile (Android)**: full vault browsing/encryption, in-app text editor,
  home-screen shortcuts to any folder or vault, two-way Google Drive and
  folder-to-folder sync, YouTube downloads as MP4 or MP3. OneDrive/Dropbox,
  Git and P2P sync stay desktop-only — see [Mobile scope](#mobile-scope)
  below.

## Screenshots

Mobile (Android):

| Browsing | Favorites | Settings |
|---|---|---|
| ![Mobile file browser](./docs/screenshots/mobile-browse.png) | ![Mobile favorites drawer](./docs/screenshots/mobile-sidebar.png) | ![Mobile settings](./docs/screenshots/mobile-settings.png) |

Desktop screenshots aren't in yet -- coming once there's a safe way to grab
them without disturbing whatever's already open on a real desktop session.

## Install

Prebuilt binaries are attached to [GitHub Releases](https://github.com/lucasmoy-dev/vaultexplorer/releases/latest)
(`.apk` for Android, `.deb`/`.rpm`/`.AppImage` for desktop) -- **not**
committed into the repo itself (git isn't a great place for binaries that
change every release; a *debug* Android build alone is 500+MB). The Android
app's own Settings screen has a "Check for updates" button that reads
straight from that same latest release, downloads the APK, and hands it to
the system installer -- no separate update server. Desktop's equivalent just
opens the release page, since replacing a running desktop binary in place is
a different problem this doesn't try to solve yet.

Building from source drops its own installer/APK where noted (below); a
local `releases/` folder (already gitignored) is a convenient place to keep
the latest one handy without it ending up in git history.

## Building from source

Desktop (Linux):

```sh
cd vaultexplorer
npm install
npm run tauri build
```

Produces `.deb`/`.rpm`/`.AppImage` under
`src-tauri/target/release/bundle/`.

Android:

```sh
npm install
npm run tauri android build --debug   # or without --debug for a release build
```

Requires the Android SDK/NDK set up for Tauri mobile (see the
[Tauri mobile prerequisites](https://v2.tauri.app/start/prerequisites/#android)).
Produces an `.apk` under
`src-tauri/gen/android/app/build/outputs/apk/`.

## Mobile scope

Android has no clean way to embed the `rclone`/`syncthing`/`unison` binaries
desktop sync relies on (no shell, Play Store restrictions on bundling
arbitrary executables). So the two backends worth having there are
reimplemented in-process instead of ported:

- **Google Drive** (`src-tauri/src/drive_rest.rs`): OAuth 2.0 + PKCE with a
  loopback redirect the phone serves to its own browser, a small Drive v3
  client, and a journal-based two-way pass (deletions propagate; a file
  changed on both sides is kept twice, never silently overwritten). It syncs
  the same `VaultExplorer/<folder>` Drive folder the desktop's rclone remote
  uses, so a folder paired on both devices is one folder. Setup is one-time:
  your own Google OAuth client, pasted in Settings → Cloud & folder sync —
  deliberately not an ID baked into the APK, which wouldn't be a secret and
  is exactly what Google is retiring rclone's shared Drive client over.
- **Folder-to-folder** (`src-tauri/src/folder_sync.rs`): the same journal
  model between two local folders, which on a phone are genuinely separate
  trees (app storage, shared storage, an SD card).

OneDrive, Dropbox, Git and Syncthing remain desktop-only: each needs its own
API client or its own binary, not a shared one.

MP3 downloads also can't lean on the platform: YouTube serves AAC in an MP4
container, Android ships no MP3 encoder, and there's no ffmpeg to shell out
to — so `src-tauri/src/mp3.rs` decodes with `symphonia` and encodes with LAME
in-process.

Everything else — vaults, browsing, the in-app editor, home-screen shortcuts
— works the same as desktop.

## Project layout

- `app/` — the app (Tauri + React frontend, Rust backend)
- `core/` — the encryption/vault library the app builds on
