// Mirrors the Rust structs (settings::Config, pipeline::Caption,
// pipeline::Status). Kept hand-written rather than generated: it's one
// small struct, and the field names are the JSON contract in both
// directions -- seeing them spelled out here is the point.

export type Anchor = "bottom" | "center" | "top";

export interface Config {
  capture_mic: boolean;
  capture_system: boolean;
  model: string;
  /** "auto" | "en" | "es" | "fr" */
  source_language: string;
  /** "off" | "en" | "es" | "fr" */
  target_language: string;
  show_original: boolean;
  sensitivity: number;
  anchor: Anchor;
  margin: number;
  width_percent: number;
  font_size: number;
  background_color: string;
  background_opacity: number;
  mic_color: string;
  system_color: string;
  max_lines: number;
  hide_after_ms: number;
  log_enabled: boolean;
  log_path: string;
  paused: boolean;
}

export interface Caption {
  source: "mic" | "system";
  text: string;
  original: string | null;
  language: string;
}

export interface Status {
  kind: "info" | "error" | "progress" | "running" | "idle" | "model-missing" | string;
  message: string;
  progress?: number;
}

export interface UpdateInfo {
  current: string;
  latest: string;
  notes: string;
  page_url: string;
  has_update: boolean;
}

export interface EngineStatus {
  models: [string, boolean][];
  model_ready: boolean;
  translation_ready: boolean;
  audio_ready: boolean;
  parec_ready: boolean;
  capturing: boolean;
}
