# LiveSubs

Live subtitles for everything your machine hears — your microphone *and*
whatever is playing through the speakers — drawn on top of every other
window. Built for Ubuntu (X11 + GNOME), fully local: speech recognition and
translation both run on this machine, so the audio of a meeting never
leaves it.

It has no main window. It lives in the top bar next to the wifi icon; the
menu there is **Configuración…**, **Pausar escucha** and **Salir**.

## What it does

- **Listens to both sides.** The microphone (your voice) and the monitor of
  the default output (everyone else in the call, a video, a podcast) are
  captured as two independent streams.
- **Subtitles over everything.** A click-through, always-on-top overlay,
  centred at the bottom by default: dark translucent plate, large text.
  Clicks pass straight through to the app underneath.
- **Two colours, two voices.** Your microphone and the system audio get
  their own colour, so you can tell who said what at a glance.
- **Translates.** Pick a language and every subtitle appears in it, whatever
  is being spoken — an English meeting reads in Spanish. English, Spanish
  and French, in every direction.
- **Detects the language.** Automatic per utterance, or fixed when you
  already know (faster and more accurate).
- **Saves the transcript.** Optional append-only text file, one line per
  utterance, with a timestamp, the source and the language.

Everything above is configurable from the settings window: position (top /
centre / bottom + margin), width, font size, how many lines stay on screen,
how long they last, plate colour and opacity, both text colours, mic
sensitivity, model size, languages, and the transcript path.

## Requirements

| Thing | Why | Install |
| --- | --- | --- |
| X11 session | The overlay places itself; a Wayland client can't (see below) | Ubuntu's "Ubuntu on Xorg" login option |
| `parec` | Audio capture (PulseAudio/PipeWire) | `sudo apt install pulseaudio-utils` |
| AppIndicator extension | The tray icon on GNOME | Ships enabled on Ubuntu (`ubuntu-appindicators@ubuntu.com`) |
| `python3-venv` | Only for translation | `sudo apt install python3-venv` |

First run downloads a whisper model (`base`, ~148MB) into
`~/.cache/livesubs/whisper/`. Translation is a separate one-time install
from the settings window and is much heavier — ~1.4GB in
`~/.local/share/livesubs/venv`, because argostranslate imports `stanza`,
which pulls in PyTorch. It is installed from PyTorch's CPU-only index; the
default wheel would add ~3.5GB of NVIDIA CUDA libraries that a machine with
an Intel iGPU can never load (the first working install here weighed
4.8GB). Skip the whole thing if you only want subtitles in the language
being spoken.

## Build and run

```bash
npm install
npm run build                     # frontend
cd src-tauri && cargo build --release
./target/release/livesubs
```

A packaged build (`.deb` / AppImage) needs the Tauri CLI:
`npx @tauri-apps/cli build`.

Two environment variables help when something looks wrong:

- `LIVESUBS_DEMO=1` draws one caption of each colour a couple of seconds
  after start — the quick way to check placement and readability without
  waiting for someone to speak.
- `LIVESUBS_DEBUG=1` prints the monitor geometry it detected and where it
  placed the overlay, which is the answer to every "the subtitles are in
  the wrong place" question (fractional scaling, a second monitor, a panel
  reserving space).

The slow, networked checks are `#[ignore]`d tests, run on purpose:

```bash
# real speech -> whisper -> Spanish, using whisper.cpp's own sample clip
cargo test --release --features custom-protocol -- --ignored --nocapture transcribes
# installs the translation venv, then translates in all four directions
cargo test --release --features custom-protocol -- --ignored --nocapture install_then_translate
```

This is the desktop half of the project; there is an
[Android app](../android/README.md) too, sharing the same recognition engine
([`../core`](../core)).

## How it works

```
parec (mic)     ─┐                        ┌─ overlay window (click-through)
                 ├─ VAD ─ whisper.cpp ─ Argos ─┤
parec (monitor) ─┘         (local)      (local) └─ transcript file
```

- **`audio.rs`** — two `parec` streams at 16kHz mono f32, from
  `@DEFAULT_SOURCE@` and `@DEFAULT_MONITOR@`. Those names are resolved by
  the audio server on connect, so plugging in headphones mid-call keeps
  working; a device index wouldn't.
- **`vad.rs`** — energy-based voice activity detection with an adaptive
  noise floor and 300ms of pre-roll, cutting the stream into utterances.
  Whisper wants speech, not a firehose, and the pre-roll is the difference
  between "hola, qué tal" and "ola, qué tal".
- **`../../core`** — the VAD and whisper.cpp (`whisper-rs`), shared with the
  Android app: one loaded model per app, a separate decoder state per
  stream, `tiny`/`base`/`small`/`medium`. `stt.rs` here is only the cache
  directory and the download.
- **`translate.rs`** — Argos Translate in its own venv, as a long-lived
  worker spoken to over JSON lines. Only the four English pairs are
  installed; Argos pivots through English for Spanish↔French. An
  environment built before the CPU-only torch pin is detected by its
  `nvidia` wheels and rebuilt.
- **`overlay.rs`** — sizes and places the overlay window, re-asserting
  always-on-top and click-through on every settings change.
- **`pipeline.rs`** — one thread per source running the whole chain, plus
  the restart rules: only the model and which sources to listen to require
  a restart; colours, position, language and logging are read per utterance.

## Known limits

- **Wayland.** The overlay needs to position itself and stay on top, which
  a plain Wayland client cannot do: Mutter implements no `wlr-layer-shell`,
  so this would need a GNOME Shell extension. On a Wayland session the
  window manager decides where the window goes, which in practice means
  the subtitles land in the wrong place. Log into "Ubuntu on Xorg".
- **Auto-detection on short utterances.** Whisper guesses the language from
  the audio it's given; "sí" or "okay" is not much to go on. If you know
  the language, fixing it is both faster and more accurate.
- **CPU only here.** This machine has an Intel iGPU, so whisper runs on the
  CPU: `base` keeps up comfortably with two streams, `small` is noticeably
  better on accents and roughly 2-3× the cost, `medium` only makes sense
  with one source.
- **Overlapping audio.** The system stream is the *mix* of everything
  playing. Two sources at once (a video plus a call) reach whisper as one
  overlapping utterance, and what comes back is somewhere between wrong and
  invented. Segments whisper itself flags as probably-not-speech are
  dropped, which removes most hallucinated captions over music, but it
  cannot separate two people talking over each other in the same mix.
- **Two streams, not diarisation.** "Your mic" vs "the system" is a real
  distinction the audio gives us for free. Telling two speakers *inside*
  the call apart is a different problem (speaker diarisation) and isn't
  attempted.
