// ---------------------------------------------------------------------------
// Thin bridge to the Rust backend. When running in a plain browser (e.g.
// `npm run dev` without the Tauri shell) everything degrades to localStorage
// so the full UI is still clickable for development.
// ---------------------------------------------------------------------------

const LS_KEY = "life-framework-db";

// Tauri injects this global; its absence means we're in a plain browser.
const hasTauri = () =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

export async function loadDb(): Promise<string | null> {
  if (!hasTauri()) return localStorage.getItem(LS_KEY);
  const raw = await invoke<string>("load_db");
  return raw && raw.length ? raw : null;
}

export async function saveDb(json: string): Promise<void> {
  if (!hasTauri()) {
    localStorage.setItem(LS_KEY, json);
    return;
  }
  await invoke("save_db", { json });
}

/** Export to a user-chosen file. Returns the path written, or null if cancelled. */
export async function exportDb(json: string): Promise<string | null> {
  if (!hasTauri()) {
    // Browser: trigger a download.
    const blob = new Blob([json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "life-framework-backup.json";
    a.click();
    URL.revokeObjectURL(url);
    return "life-framework-backup.json";
  }
  const { save } = await import("@tauri-apps/plugin-dialog");
  const dest = await save({
    defaultPath: "life-framework-backup.json",
    filters: [{ name: "JSON", extensions: ["json"] }],
  });
  if (!dest) return null;
  await invoke("export_db", { json, dest });
  return dest;
}

/** Import from a user-chosen file. Returns the JSON string, or null if cancelled. */
export async function importDb(): Promise<string | null> {
  if (!hasTauri()) {
    return new Promise((resolve) => {
      const input = document.createElement("input");
      input.type = "file";
      input.accept = "application/json,.json";
      input.onchange = () => {
        const file = input.files?.[0];
        if (!file) return resolve(null);
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.readAsText(file);
      };
      input.click();
    });
  }
  const { open } = await import("@tauri-apps/plugin-dialog");
  const src = await open({
    multiple: false,
    filters: [{ name: "JSON", extensions: ["json"] }],
  });
  if (!src || typeof src !== "string") return null;
  return invoke<string>("import_db", { src });
}

export interface UpdateInfo {
  current: string;
  latest: string;
  notes: string;
  apkUrl: string;
  htmlUrl: string;
  hasUpdate: boolean;
}

export async function appVersion(): Promise<string> {
  if (!hasTauri()) return "0.1.0-web";
  return invoke<string>("app_version");
}

export async function checkUpdate(): Promise<UpdateInfo> {
  if (!hasTauri()) {
    return {
      current: "0.1.0-web",
      latest: "0.1.0-web",
      notes: "El chequeo de actualizaciones solo funciona en la app instalada.",
      apkUrl: "",
      htmlUrl: "",
      hasUpdate: false,
    };
  }
  return invoke<UpdateInfo>("check_update");
}

/** Android: download the APK and fire the system installer. */
export async function downloadAndInstall(apkUrl: string): Promise<void> {
  await invoke("download_and_install_update", { url: apkUrl });
}

/** True only when the auto-install path is available (Android). */
export async function canAutoInstall(): Promise<boolean> {
  if (!hasTauri()) return false;
  try {
    return await invoke<boolean>("can_auto_install");
  } catch {
    return false;
  }
}

/** Open a URL in the system browser (fallback download path). */
export async function openUrl(url: string): Promise<void> {
  if (!hasTauri()) {
    window.open(url, "_blank");
    return;
  }
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(url);
}
