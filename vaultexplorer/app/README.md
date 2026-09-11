# Vault Explorer

A file manager with Cryptomator vaults built in. Browse your files like any
other explorer; turn any folder into a vault and its contents — filenames
included — are encrypted at rest, unlocked with a password for as long as
you're using it.

The vault format **is** [Cryptomator](https://cryptomator.org)'s (vault
format 8, `SIV_GCM`). A vault made here opens in Cryptomator's own apps, and
a folder Cryptomator made is recognised here as a vault and opens normally.
Nothing about your encrypted data is specific to this app.

Runs on Linux desktop and Android from the same codebase (Tauri + Rust +
React).

## Features

- **Vaults**: create one, or convert an existing folder in place. Optional
  "sensitive" files and folders that re-lock and ask for the password again
  after a timeout, independent of the vault itself.
- **File browsing**: icon/list/column/list-with-preview views, thumbnails,
  tags, favorites, search, archive browsing (zip) without extracting first.
- **On the desktop**, an unlocked vault is also mounted (FUSE) so any other
  program can open a file straight out of it, with the plaintext living only
  in RAM.
- **Secure delete**: real overwrite-based shredding, not just unlink.
- **Extras**: inline text/markdown editor, image/video/PDF conversion tools,
  "Free Up Space", drive and machine info, terminal integration (desktop),
  a standalone "encrypt this one file with a password" (`.vlt`).
- **Mobile (Android)**: the same vaults, browsing, in-app editor, and
  home-screen shortcuts to any folder or vault.

## Scope

This app is a file manager and a vault. It deliberately does **not** do
cloud/P2P/folder sync, git, internet search or downloads, music/notes/
contacts/library views, folder freezing or AI reorganisation — all of which
it used to, and none of which are what it is for. Sync belongs to whatever
already syncs your folders (including Cryptomator's own supported clouds);
a vault is just a folder.

## Install

Prebuilt binaries are attached to
[GitHub Releases](https://github.com/lucasmoy-dev/vaultexplorer/releases/latest)
(`.apk` for Android, `.deb`/`.rpm`/`.AppImage` for desktop) — **not**
committed into the repo itself. The Android app's own Settings screen has a
"Check for updates" button that reads that same latest release, downloads
the APK and hands it to the system installer. Desktop's equivalent opens the
release page.

## Verifying

```bash
# The vault engine: a real Cryptomator vault on disk, round trips, ranged
# reads, moves/copies/deletes, sensitive files, password change, zip
cd core && cargo test

# The app's own backend
cd ../app/src-tauri && cargo test

# Types, the production bundle, and that every var(--token) a stylesheet
# uses is actually defined (and defined for light mode too)
cd .. && npx tsc --noEmit && npx vite build && node scripts/check-theme-tokens.mjs
```

Interop is the property that matters most here, and it was checked against
an implementation that shares no code with this one
([`pycryptomator`](https://pypi.org/project/pycryptomator/)): a vault
created here — nested folders, a 100 KB multi-chunk file, a filename long
enough to take the `.c9s` shortening path — unlocks there and decrypts
byte-for-byte, and a vault created there opens here.

## Building from source

Desktop (Linux):

```sh
cd app
npm install
npm run tauri build
```

Produces `.deb`/`.rpm`/`.AppImage` under `src-tauri/target/release/bundle/`.

Android:

```sh
npm install
npm run android:build
```

Requires the Android SDK/NDK set up for Tauri mobile (see the
[Tauri mobile prerequisites](https://v2.tauri.app/start/prerequisites/#android)).
Produces an `.apk` under `src-tauri/gen/android/app/build/outputs/apk/`.

## Mobile scope

A few desktop integrations have no Android equivalent and are compiled out
there rather than stubbed: the FUSE mount (so opening a vault file in
another app decrypts to a temporary copy instead), the OS trash, the
xdg-desktop-portal file picker, drive/format tooling, and the terminal.
Everything else — vaults, browsing, the editor, shortcuts — is the same
code on both.

## Project layout

- `app/` — the app (Tauri + React frontend, Rust backend)
- `core/` — `vaultcore`: the vault engine. Wraps
  [`cryptomator-rs-crypto`](https://github.com/Dar9586/cryptomator-rs)
  (Apache-2.0) in the shape the app speaks, adds the sensitive-files
  manifest and the FUSE mount, and handles `.vlt` standalone files and
  vault password changes.
