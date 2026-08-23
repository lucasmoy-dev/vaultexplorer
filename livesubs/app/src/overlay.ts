import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Config, Caption } from "./types";

// The overlay is a dumb renderer on purpose: it holds no state the backend
// doesn't own, and every visual choice comes from the config it is handed.
// That way a settings change is a repaint, not a restart, and nothing here
// can drift from what the user actually saved.

const container = document.getElementById("lines") as HTMLDivElement;

let config: Config | null = null;
/// The lines currently on screen, oldest first.
const shown: { element: HTMLDivElement; timer: number }[] = [];

function applyConfig(next: Config) {
  config = next;
  const root = document.documentElement;
  root.style.setProperty("--font-size", `${next.font_size}px`);
  root.style.setProperty("--plate-bg", plateColor(next));
  root.style.setProperty("--mic-color", next.mic_color);
  root.style.setProperty("--system-color", next.system_color);
  // A smaller max_lines has to take effect now, not after the next
  // caption: the window was already resized to fit fewer lines, and
  // anything extra would be clipped.
  while (shown.length > Math.max(1, next.max_lines)) removeOldest();
}

/// The plate colour: a hex background plus a separate opacity slider is
/// far easier to get right than asking anyone to type an 8-digit hex.
function plateColor(next: Config): string {
  const hex = next.background_color.replace("#", "");
  const full = hex.length === 3 ? hex.split("").map((c) => c + c).join("") : hex;
  const value = parseInt(full.slice(0, 6) || "000000", 16);
  const r = (value >> 16) & 0xff;
  const g = (value >> 8) & 0xff;
  const b = value & 0xff;
  const alpha = Math.min(1, Math.max(0, next.background_opacity));
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function removeOldest() {
  const oldest = shown.shift();
  if (!oldest) return;
  window.clearTimeout(oldest.timer);
  fadeOut(oldest.element);
}

function fadeOut(element: HTMLDivElement) {
  element.classList.add("leaving");
  // Matches the CSS transition; removing the node earlier would cut the
  // fade off half way.
  window.setTimeout(() => element.remove(), 450);
}

function addCaption(caption: Caption) {
  if (!config) return;
  const line = document.createElement("div");
  line.className = `line ${caption.source}`;
  line.textContent = caption.text;
  if (caption.original && caption.original !== caption.text) {
    const original = document.createElement("span");
    original.className = "original";
    original.textContent = caption.original;
    line.appendChild(original);
  }
  container.appendChild(line);

  const timer = window.setTimeout(() => {
    const index = shown.findIndex((entry) => entry.element === line);
    if (index >= 0) shown.splice(index, 1);
    fadeOut(line);
  }, Math.max(1000, config.hide_after_ms));
  shown.push({ element: line, timer });

  while (shown.length > Math.max(1, config.max_lines)) removeOldest();
}

async function main() {
  applyConfig(await invoke<Config>("get_config"));
  await listen<Config>("config-changed", (event) => applyConfig(event.payload));
  await listen<Caption>("caption", (event) => addCaption(event.payload));
}

main().catch((error) => {
  // No console anyone will read and nowhere to draw an error without
  // covering the user's screen -- but keep it out of the way rather than
  // failing silently for a developer looking at the webview inspector.
  console.error("overlay failed to start", error);
});
