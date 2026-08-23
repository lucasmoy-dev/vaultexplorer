import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { Config, EngineStatus, Status, UpdateInfo } from "./types";

// The settings window builds its controls in code rather than as static
// HTML. There are ~20 of them, every one bound to a field of the same
// config object, and the interesting part is the binding -- writing each
// row twice (markup + wiring) is how those two drift apart.

const app = document.getElementById("app") as HTMLElement;

let config: Config;
let engine: EngineStatus;
let saveTimer: number | undefined;
/// Set while applying a config that came *from* the backend, so echoing it
/// straight back as a "change" can't loop.
let applying = false;
/// Filled once at startup from the backend's own package version, so the
/// number shown is the binary's, not a string duplicated here.
let appVersion = "";

const LANGUAGES: [string, string][] = [
  ["en", "Inglés"],
  ["es", "Español"],
  ["fr", "Francés"],
];

function scheduleSave() {
  if (applying) return;
  window.clearTimeout(saveTimer);
  // Dragging a slider fires continuously; one write per gesture is
  // enough, and every write reaches the disk and both windows.
  saveTimer = window.setTimeout(() => {
    invoke("set_config", { config }).catch((error) => log(String(error), true));
  }, 220);
}

// ---- tiny DOM helpers -------------------------------------------------

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> = {},
  children: (Node | string)[] = []
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  Object.assign(node, props);
  for (const child of children) node.append(child);
  return node;
}

function section(title: string, rows: HTMLElement[]): HTMLElement {
  return el("section", {}, [el("h2", { textContent: title }), ...rows]);
}

function row(label: string, hint: string | null, controls: HTMLElement[]): HTMLElement {
  const name = el("label", { className: "name" }, [label]);
  if (hint) name.append(el("span", { className: "hint", textContent: hint }));
  return el("div", { className: "row" }, [name, ...controls]);
}

function checkbox(get: () => boolean, set: (value: boolean) => void): HTMLInputElement {
  const input = el("input", { type: "checkbox", checked: get() });
  input.addEventListener("change", () => {
    set(input.checked);
    scheduleSave();
  });
  return input;
}

function select(
  options: [string, string][],
  get: () => string,
  set: (value: string) => void
): HTMLSelectElement {
  const node = el("select");
  for (const [value, label] of options) {
    node.append(el("option", { value, textContent: label }));
  }
  node.value = get();
  node.addEventListener("change", () => {
    set(node.value);
    scheduleSave();
  });
  return node;
}

function number(
  get: () => number,
  set: (value: number) => void,
  attrs: { min?: number; max?: number; step?: number } = {}
): HTMLInputElement {
  const input = el("input", { type: "number", value: String(get()) });
  if (attrs.min !== undefined) input.min = String(attrs.min);
  if (attrs.max !== undefined) input.max = String(attrs.max);
  if (attrs.step !== undefined) input.step = String(attrs.step);
  input.addEventListener("input", () => {
    const value = Number(input.value);
    if (Number.isFinite(value)) {
      set(value);
      scheduleSave();
    }
  });
  return input;
}

function range(
  get: () => number,
  set: (value: number) => void,
  attrs: { min: number; max: number; step: number },
  format: (value: number) => string
): HTMLElement {
  const input = el("input", { type: "range", value: String(get()) });
  input.min = String(attrs.min);
  input.max = String(attrs.max);
  input.step = String(attrs.step);
  const readout = el("span", { className: "hint", textContent: format(get()) });
  input.addEventListener("input", () => {
    const value = Number(input.value);
    set(value);
    readout.textContent = format(value);
    scheduleSave();
  });
  return el("div", { className: "swatch-pair" }, [input, readout]);
}

function color(get: () => string, set: (value: string) => void): HTMLInputElement {
  const input = el("input", { type: "color", value: get() });
  input.addEventListener("input", () => {
    set(input.value);
    scheduleSave();
  });
  return input;
}

function button(label: string, onClick: () => void, className = ""): HTMLButtonElement {
  const node = el("button", { textContent: label, className });
  node.addEventListener("click", onClick);
  return node;
}

// ---- status log -------------------------------------------------------

const logBox = el("div", { className: "log" });

function log(message: string, isError = false) {
  const line = el("div", { textContent: message, className: isError ? "err" : "" });
  logBox.append(line);
  // Newest at the bottom, and keep it in view: this is the only place the
  // app can tell the user why it went quiet.
  logBox.scrollTop = logBox.scrollHeight;
  while (logBox.childElementCount > 60) logBox.firstElementChild?.remove();
}

function dot(ok: boolean): HTMLElement {
  return el("span", { className: `dot ${ok ? "ok" : "bad"}` });
}

// ---- the form ---------------------------------------------------------

function render() {
  app.replaceChildren();

  // --- estado ---
  const pauseButton = button(config.paused ? "Reanudar escucha" : "Pausar escucha", async () => {
    try {
      config.paused = await invoke<boolean>("toggle_pause");
      await refreshEngine();
      render();
    } catch (error) {
      log(String(error), true);
    }
  }, "primary");

  const statusRows = [
    el("div", { className: "status-line" }, [
      dot(engine.capturing),
      engine.capturing
        ? "Escuchando."
        : config.paused
          ? "En pausa."
          : "Sin escuchar (mira los avisos de abajo).",
    ]),
    el("div", { className: "status-line" }, [
      dot(engine.parec_ready && engine.audio_ready),
      engine.parec_ready && engine.audio_ready
        ? "Audio del sistema disponible."
        : "Falta parec o el servidor de audio (sudo apt install pulseaudio-utils).",
    ]),
    el("div", { className: "status-line" }, [
      dot(engine.model_ready),
      engine.model_ready
        ? `Modelo de voz "${config.model}" listo.`
        : `El modelo "${config.model}" no está descargado.`,
    ]),
    el("div", { className: "status-line" }, [
      dot(engine.translation_ready),
      engine.translation_ready
        ? "Motor de traducción instalado."
        : "Traducción no instalada (solo hace falta si quieres traducir).",
    ]),
    el("div", { className: "row" }, [
      el("label", { className: "name" }, ["Escucha"]),
      pauseButton,
    ]),
    logBox,
  ];
  app.append(section("Estado", statusRows));

  // --- audio ---
  app.append(
    section("Audio que se escucha", [
      row("Micrófono", "Tu propia voz (fuente de entrada por defecto).", [
        checkbox(
          () => config.capture_mic,
          (value) => (config.capture_mic = value)
        ),
      ]),
      row("Sonido del sistema", "Lo que suena en el equipo: la otra parte de la reunión, un vídeo.", [
        checkbox(
          () => config.capture_system,
          (value) => (config.capture_system = value)
        ),
      ]),
      row("Sensibilidad", "Más alto = hace falta hablar más fuerte para que se active.", [
        range(
          () => config.sensitivity,
          (value) => (config.sensitivity = value),
          { min: 0.3, max: 3, step: 0.1 },
          (value) => `${value.toFixed(1)}×`
        ),
      ]),
    ])
  );

  // --- reconocimiento ---
  const modelSelect = select(
    engine.models.map(([name, present]) => [name, present ? `${name} (descargado)` : `${name} (sin descargar)`]),
    () => config.model,
    (value) => (config.model = value)
  );
  const downloadButton = button("Descargar modelo", async () => {
    downloadButton.disabled = true;
    try {
      await invoke("download_model", { model: config.model });
      await refreshEngine();
      render();
    } catch (error) {
      log(String(error), true);
    } finally {
      downloadButton.disabled = false;
    }
  });
  app.append(
    section("Reconocimiento de voz", [
      row("Modelo", "Más grande = más preciso y más lento. base va bien con dos fuentes a la vez.", [
        modelSelect,
        downloadButton,
      ]),
      row("Idioma hablado", "Automático lo detecta en cada frase; fijarlo es más rápido y preciso.", [
        select(
          [["auto", "Detección automática"], ...LANGUAGES],
          () => config.source_language,
          (value) => (config.source_language = value)
        ),
      ]),
    ])
  );

  // --- traducción ---
  const installButton = button(
    engine.translation_ready ? "Reinstalar motor" : "Instalar motor de traducción",
    async () => {
      installButton.disabled = true;
      log("Instalando el motor de traducción; esto puede tardar varios minutos…");
      try {
        await invoke("install_translation");
        await refreshEngine();
        render();
      } catch (error) {
        log(String(error), true);
      } finally {
        installButton.disabled = false;
      }
    }
  );
  const testButton = button("Probar", async () => {
    const from = config.source_language === "auto" ? "en" : config.source_language;
    const to = config.target_language === "off" ? "es" : config.target_language;
    try {
      const result = await invoke<string>("test_translation", {
        text: from === "es" ? "Buenos días, ¿empezamos la reunión?" : "Good morning, shall we start the meeting?",
        from,
        to,
      });
      log(`Prueba (${from} → ${to}): ${result}`);
    } catch (error) {
      log(String(error), true);
    }
  });
  app.append(
    section("Traducción", [
      row("Mostrar siempre en", "Los subtítulos aparecen en este idioma, hable quien hable.", [
        select(
          [["off", "Sin traducir"], ...LANGUAGES],
          () => config.target_language,
          (value) => (config.target_language = value)
        ),
      ]),
      row("Mostrar también el original", "Debajo de la traducción, más pequeño.", [
        checkbox(
          () => config.show_original,
          (value) => (config.show_original = value)
        ),
      ]),
      row(
        "Motor",
        "Argos Translate, local y sin conexión. Instalación única de ~1,4GB (incluye PyTorch en versión CPU).",
        [
        installButton,
        testButton,
      ]),
    ])
  );

  // --- apariencia ---
  app.append(
    section("Aspecto de los subtítulos", [
      row("Posición", null, [
        select(
          [
            ["bottom", "Abajo"],
            ["center", "Centro"],
            ["top", "Arriba"],
          ],
          () => config.anchor,
          (value) => (config.anchor = value as Config["anchor"])
        ),
      ]),
      row("Distancia al borde", "Píxeles desde el borde elegido.", [
        number(
          () => config.margin,
          (value) => (config.margin = Math.round(value)),
          { min: 0, max: 1200, step: 10 }
        ),
      ]),
      row("Ancho", "Porcentaje del ancho de la pantalla.", [
        range(
          () => config.width_percent,
          (value) => (config.width_percent = Math.round(value)),
          { min: 30, max: 100, step: 5 },
          (value) => `${value}%`
        ),
      ]),
      row("Tamaño de letra", null, [
        number(
          () => config.font_size,
          (value) => (config.font_size = Math.round(value)),
          { min: 14, max: 96, step: 2 }
        ),
      ]),
      row("Líneas visibles", "Cuántas frases se quedan en pantalla a la vez.", [
        number(
          () => config.max_lines,
          (value) => (config.max_lines = Math.round(value)),
          { min: 1, max: 5, step: 1 }
        ),
      ]),
      row("Desaparecen tras", "Milisegundos sin voz antes de que se borren.", [
        number(
          () => config.hide_after_ms,
          (value) => (config.hide_after_ms = Math.round(value)),
          { min: 1000, max: 30000, step: 500 }
        ),
      ]),
      row("Fondo", "Color y opacidad de la placa detrás del texto.", [
        el("div", { className: "swatch-pair" }, [
          color(
            () => config.background_color,
            (value) => (config.background_color = value)
          ),
          range(
            () => config.background_opacity,
            (value) => (config.background_opacity = value),
            { min: 0, max: 1, step: 0.02 },
            (value) => `${Math.round(value * 100)}%`
          ),
        ]),
      ]),
      row("Color del micrófono", "Tu voz.", [
        color(
          () => config.mic_color,
          (value) => (config.mic_color = value)
        ),
        button("Ver ejemplo", () => {
          invoke("preview_caption", { source: "mic" }).catch((error) => log(String(error), true));
        }),
      ]),
      row("Color del sistema", "Las demás voces.", [
        color(
          () => config.system_color,
          (value) => (config.system_color = value)
        ),
        button("Ver ejemplo", () => {
          invoke("preview_caption", { source: "system" }).catch((error) => log(String(error), true));
        }),
      ]),
    ])
  );

  // --- actualizaciones ---
  // Same convention as the Android app and the other apps in this repo:
  // releases live in the monorepo's GitHub releases, tagged per app
  // (`livesubs-vX.Y.Z`). The phone installs its own APK; here it opens the
  // release page, because swapping a running binary out from under itself
  // is not something a desktop app should do quietly.
  const versionLabel = el("span", { className: "hint", textContent: appVersion ? `v${appVersion}` : "…" });
  const updateStatus = el("span", { className: "hint" });
  let latestPage = "";
  const openReleaseButton = button("Ver release", () => {
    invoke("open_releases_page", { url: latestPage }).catch((error) => log(String(error), true));
  });
  const checkButton = button(
    "Buscar actualizaciones",
    async () => {
      checkButton.disabled = true;
      updateStatus.textContent = "Comprobando…";
      try {
        const info = await invoke<UpdateInfo>("check_update");
        latestPage = info.page_url;
        updateStatus.textContent = info.has_update
          ? `Hay versión ${info.latest} (tienes ${info.current}).`
          : `Estás al día (${info.current}).`;
        if (info.notes.trim()) log(info.notes.trim());
      } catch (error) {
        updateStatus.textContent = "";
        log(String(error), true);
      } finally {
        checkButton.disabled = false;
      }
    },
    "primary"
  );
  app.append(
    section("Actualizaciones", [
      row("Versión instalada", "Las releases se publican en GitHub, con el APK de Android adjunto.", [
        versionLabel,
      ]),
      row("Comprobar", null, [checkButton, openReleaseButton, updateStatus]),
    ])
  );

  // --- transcripción ---
  const pathInput = el("input", { type: "text", value: config.log_path });
  pathInput.addEventListener("input", () => {
    config.log_path = pathInput.value;
    scheduleSave();
  });
  app.append(
    section("Guardar transcripción", [
      row("Guardar todo lo reconocido", "Una línea por frase, con hora y fuente.", [
        checkbox(
          () => config.log_enabled,
          (value) => (config.log_enabled = value)
        ),
      ]),
      row("Archivo", null, [
        pathInput,
        button("Elegir…", async () => {
          const picked = await saveDialog({
            title: "Guardar transcripción en",
            defaultPath: config.log_path,
            filters: [{ name: "Texto", extensions: ["txt", "md"] }],
          });
          if (typeof picked === "string") {
            config.log_path = picked;
            pathInput.value = picked;
            scheduleSave();
          }
        }),
        button("Abrir", () => {
          invoke("open_log_file", { path: config.log_path }).catch((error) => log(String(error), true));
        }),
      ]),
    ])
  );
}

async function refreshEngine() {
  engine = await invoke<EngineStatus>("engine_status");
}

async function main() {
  config = await invoke<Config>("get_config");
  appVersion = await invoke<string>("app_version").catch(() => "");
  await refreshEngine();
  render();

  await listen<Status>("status", (event) => {
    const { kind, message, progress } = event.payload;
    const suffix = progress !== undefined ? ` (${Math.round(progress * 100)}%)` : "";
    log(`${message}${suffix}`, kind === "error");
    // "Capture started/stopped" changes the dots at the top, so re-read
    // the engine state rather than leaving a stale green light.
    if (kind === "running" || kind === "idle" || kind === "error" || kind === "info") {
      refreshEngine()
        .then(() => {
          const stale = config.paused;
          void stale;
          render();
        })
        .catch(() => {});
    }
  });

  await listen<Config>("config-changed", (event) => {
    applying = true;
    config = event.payload;
    render();
    applying = false;
  });
}

main().catch((error) => {
  app.append(el("p", { textContent: `No se pudo cargar la configuración: ${error}` }));
});
