import { Entry } from "./api";

// A browsing location is either a real OS directory ("fs") or a directory
// inside a currently-unlocked vault ("vault", rel = path relative to root).
export type Loc = { kind: "fs"; path: string } | { kind: "vault"; root: string; rel: string };

// `root` is only set when `kind === "vault"` -- which specific vault the
// paths were cut/copied from, so pasting into a *different* unlocked vault
// (both are `kind: "vault"`, but a different root) is recognized as a
// cross-vault paste instead of misreading the rel paths against whichever
// vault happens to be active when paste() actually runs.
export type Clipboard =
  | { paths: string[]; mode: "copy" | "cut"; kind: Loc["kind"]; root?: string }
  | null;
export type View = "icon" | "list" | "column" | "listPreview" | "notes" | "contacts";
export type ProgressOp = {
  id: number;
  label: string;
  done: number;
  total: number;
  // The Tauri Channel's own id, used as the cancellable-operation key
  // (see ops.rs / cancel_operation). undefined for ops that can't cancel.
  cancelId?: number;
  status?: "running" | "error" | "cancelled";
};

export type PendingAction =
  | { kind: "delete"; names: string[] }
  | { kind: "secureDelete"; names: string[] }
  | { kind: "gitCommit" }
  | { kind: "freeze"; entry: Entry }
  | { kind: "unlock"; path: string; name: string }
  | { kind: "newVault" };

// Timeout options (seconds) for the sensitive-files re-auth window; "never"
// keeps it open until the vault is locked / the app closes.
// Seconds a sensitive-file session stays open, or "never" for no auto-relock.
// The older values (1200/7200/18000) stay in the union so a setting saved
// before the list below existed still loads and typechecks.
export type SensitiveTimeout = 60 | 300 | 1200 | 3600 | 7200 | 18000 | 86400 | 604800 | "never";

/// The durations offered in the UI, in one place so the Settings dropdown and
/// the unlock sheet's selector can't drift apart -- the sheet preselects
/// whatever Settings holds, which only works if both draw from the same list.
export const SENSITIVE_TIMEOUT_CHOICES: { value: SensitiveTimeout; label: string }[] = [
  { value: 60, label: "1 min" },
  { value: 300, label: "5 min" },
  { value: 3600, label: "1 hour" },
  { value: 86400, label: "1 day" },
  { value: 604800, label: "1 week" },
  { value: "never", label: "Never" },
];

/// Label for a stored value that predates (or falls outside) the list above,
/// so Settings can still show it accurately instead of appearing unset.
export function sensitiveTimeoutLabel(value: SensitiveTimeout): string {
  const known = SENSITIVE_TIMEOUT_CHOICES.find((o) => o.value === value);
  if (known) return known.label;
  if (value === "never") return "Never";
  const mins = Math.round(value / 60);
  return mins >= 60 ? `${Math.round(mins / 60)} hours` : `${mins} min`;
}

/// The choice a selector should start on: the configured value when it's one
/// of the offered ones, otherwise the nearest offered duration -- never
/// silently longer than what was configured would suggest.
export function nearestSensitiveChoice(value: SensitiveTimeout): SensitiveTimeout {
  if (SENSITIVE_TIMEOUT_CHOICES.some((o) => o.value === value)) return value;
  if (value === "never") return "never";
  let best: SensitiveTimeout = 300;
  let bestDiff = Infinity;
  for (const o of SENSITIVE_TIMEOUT_CHOICES) {
    if (o.value === "never") continue;
    const diff = Math.abs(o.value - value);
    if (diff < bestDiff) {
      bestDiff = diff;
      best = o.value;
    }
  }
  return best;
}

export interface VaultCreateOptions {
  sensitive: boolean;
  autoLockMinutes: number;
  autoUnlock: boolean;
}
