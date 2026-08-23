# Life Framework

A personal-development dashboard (Android APK, built with Tauri v2 + React).
Sibling project to `vaultexplorer/` in this monorepo.

It answers three questions at a glance:

1. **¿Cómo estoy hoy?** — a filled radar (hexagon+) of your life areas, each
   scored 0–100. The polygon is shaped by whatever areas you add.
2. **¿Dónde quiero estar?** — a transparent **Next Goal** polygon overlaid on
   the same axes.
3. **¿Cuál es el plan medible?** — per-goal projections: required pace to hit a
   deadline, and when your current trend actually gets there.

## Model

- **Área** → one or more **subcategorías** (each with its own weight, e.g.
  *Health* → Salud mental / Ejercicio / Enfermedades at 70%) → weighted list of
  **preguntas** (audit items). Area score = weighted mean of subcategory scores;
  subcategory score = weighted mean of its questions. Each question maps a raw
  value to 0–100%:
  - `numeric`: linear between an *anchor 0%* and *anchor 100%* value. The 100%
    anchor may be **lower** than the 0% anchor for "menos es mejor" metrics
    (peso, deuda). Example: ingresos pasivos `0€ = 0%`, `2500€ = 100%`.
  - `scale`: 1–10 (configurable min/max).
  - `boolean`: Sí/No.
- Each question has a **weight** within its area; each area has a **weight**
  toward the overall score.
- A **check-in** freezes today's values into a dated snapshot → history, trends,
  and trend-based goal projections.

Everything is **offline and local**: the whole state is one JSON document in the
app-data dir (`src-tauri/src/storage.rs`). Export/import JSON from Ajustes.

## Layout

```
life-framework/app/
  src/            React frontend (screens/, components/, scoring.ts, planner.ts)
  src-tauri/      Rust backend (storage.rs, update.rs, android.rs)
```

Pure logic lives in `src/scoring.ts` and `src/planner.ts` and is unit-tested
(`*.test.ts`, run with `npm test`).

## Develop

```bash
cd life-framework/app
npm install
npm run dev          # plain browser (localStorage fallback) — quick UI loop
npm run tauri dev    # native desktop shell
npm test             # scoring + planner unit tests
```

## Build the APK

Requires `ANDROID_HOME` + `NDK_HOME` (already set on this machine) and the Rust
Android targets (`rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android`).

```bash
cd life-framework/app
npm run android:build
# -> src-tauri/gen/android/app/build/outputs/apk/universal/release/
#      app-universal-release-unsigned.apk   (all 4 ABIs, incl. arm64-v8a)
```

The Android project (`src-tauri/gen/android`) is **committed** (only
`gen/schemas` is ignored) because the manifest carries the
`REQUEST_INSTALL_PACKAGES` permission and Tauri's `FileProvider` block that the
in-app updater needs.

## In-app update (Ajustes → Actualizar app)

`check_update` queries `https://api.github.com/repos/lucasmoy-dev/vaultexplorer/releases`
(the repo's current name — GitHub's rename redirect keeps this working if it's
later renamed to `personal-projects`), keeps releases whose tag starts with
**`life-framework-v`**, and compares the highest semver to the running version.
The constant lives in `src-tauri/src/update.rs`.

**v0.1.0 is already published:**
<https://github.com/lucasmoy-dev/vaultexplorer/releases/tag/life-framework-v0.1.0>
(signed universal APK attached).

- **Descargar e instalar** (Android): downloads the release's `.apk` asset and
  fires the system installer via FileProvider (`src-tauri/src/android.rs`).
  Needs "Instalar apps desconocidas" granted — the app sends you to that
  settings screen the first time.
- **Abrir en navegador**: fallback that opens the release page.

### Release convention (so updates are found)

1. Bump `version` in **both** `src-tauri/tauri.conf.json` and
   `src-tauri/Cargo.toml`.
2. Tag the release **`life-framework-vX.Y.Z`**.
3. Attach the **signed** `.apk` as a release asset.

## Signing

Release APKs are signed with a local keystore. `app/build.gradle.kts` reads
`src-tauri/gen/android/keystore.properties`; both the `.properties` and the
`*.jks` are **gitignored** (never committed). The current key:

```
storeFile   = src-tauri/lifeframework-release.jks
keyAlias    = lifeframework
```

Keep this keystore — Android requires the **same** signing key across versions
or an in-app update fails to install over the previous one. To rebuild on
another machine, recreate `keystore.properties` pointing at a copy of the
`.jks`.

## TODO

- **Replace placeholder icons.** `src-tauri/icons/*` are copied from
  vaultexplorer as a placeholder. Run `npx tauri icon path/to/logo.png` to
  regenerate the real set (desktop + Android mipmaps).
