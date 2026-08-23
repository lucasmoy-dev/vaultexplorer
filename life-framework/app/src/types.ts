// ---------------------------------------------------------------------------
// Life Framework data model.
//
// The whole app state is one JSON document (`DB`) persisted to disk by the
// Rust side (see src-tauri/src/storage.rs). Everything is offline and local.
// ---------------------------------------------------------------------------

export type QuestionKind = "numeric" | "boolean" | "scale";

/** A single audit item inside an area. Maps a raw value -> 0..100%. */
export interface Question {
  id: string;
  text: string;
  /** Relative weight of this question within its area. Default 1. */
  weight: number;
  kind: QuestionKind;

  // --- numeric ---
  /** Unit label, e.g. "€", "kg", "hrs". Numeric only. */
  unit?: string;
  /** Raw value that means 0%. */
  anchorZero?: number;
  /** Raw value that means 100%. May be LOWER than anchorZero for
   *  "lower is better" metrics (weight, debt): interpolation still works. */
  anchorHundred?: number;

  // --- scale ---
  scaleMin?: number; // default 1
  scaleMax?: number; // default 10

  // --- boolean ---
  yesScore?: number; // default 100
  noScore?: number; // default 0

  // --- goal / planner (optional, any kind) ---
  /** Desired mid-term value -> drives the "Next Goal" polygon + planner. */
  target?: number;
  /** ISO date (yyyy-mm-dd) the target should be reached by. */
  deadline?: string;
}

/** A group of questions inside an area, e.g. Health → "Salud mental". */
export interface Subcategory {
  id: string;
  name: string;
  /** Relative weight of this subcategory within its area. Default 1. */
  weight: number;
  questions: Question[];
}

export interface Area {
  id: string;
  name: string;
  /** Hex color for this axis on the radar. */
  color: string;
  /** Single emoji used as the area icon. */
  icon: string;
  /** Relative weight toward the overall score. Default 1. */
  weight: number;
  order: number;
  /** One or more subcategories, each holding its own questions. */
  subs: Subcategory[];
}

/** A dated check-in: a frozen copy of every answered value. */
export interface Snapshot {
  id: string;
  dateISO: string; // yyyy-mm-dd
  answers: Record<string, number>; // questionId -> raw value
  note?: string;
}

export interface Settings {
  /** GitHub repo the update checker queries. */
  updateRepo: string;
  /** Only releases whose tag starts with this prefix are considered. */
  tagPrefix: string;
  /** Suggested cadence between check-ins, in days. */
  reminderDays: number;
  theme: "dark" | "light";
}

export interface DB {
  schemaVersion: number;
  settings: Settings;
  areas: Area[];
  /** Live "today" values: questionId -> raw value. */
  current: Record<string, number>;
  /** History of check-ins, oldest first. */
  snapshots: Snapshot[];
}

export const SCHEMA_VERSION = 1;
