import { Entry } from "./api";

// A browsing location is either a real OS directory ("fs") or a directory
// inside a currently-unlocked vault ("vault", rel = path relative to root).
export type Loc = { kind: "fs"; path: string } | { kind: "vault"; root: string; rel: string };

export type Clipboard = { paths: string[]; mode: "copy" | "cut"; kind: Loc["kind"] } | null;
export type View = "icon" | "list" | "column" | "listPreview";
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
  | { kind: "password"; entry: Entry }
  | { kind: "gitCommit" }
  | { kind: "freeze"; entry: Entry }
  | { kind: "unlock"; path: string; name: string }
  | { kind: "newVault" };

export interface VaultCreateOptions {
  sensitive: boolean;
  autoLockMinutes: number;
  autoUnlock: boolean;
}
