import { useEffect, useRef } from "react";
import { emit, listen } from "@tauri-apps/api/event";

// ---------------------------------------------------------------------------
// Cross-window state.
//
// "Opening a second VaultExplorer" never starts a second process:
// `tauri_plugin_single_instance` intercepts the launch and opens another
// `main-N` window inside the running one (see `open_extra_explorer_window`
// in lib.rs). Every window then mounts its own React tree with its own
// `useState`, and nothing tells one window that another changed something.
//
// That left two distinct bugs:
//
//   * Ephemeral state was simply invisible across windows. Copy a folder
//     in window A, switch to window B, Paste -- B's `clipboard` state is
//     still null, so nothing happens and no error explains why.
//
//   * localStorage-backed state (favorites, pins, settings, templates,
//     custom icons, view prefs) was read once at mount and written on
//     every change. Window B therefore showed a stale copy *and*, on its
//     next write, persisted that stale copy right over what window A had
//     just saved -- a silent last-writer-wins that could drop a favorite
//     the user had added seconds earlier in the other window.
//
// The DOM `storage` event is not an option: WebKitGTK does not deliver it
// between separate webviews, so the broadcast goes over Tauri's own event
// bus, which is process-wide and reaches every window.
// ---------------------------------------------------------------------------

// Identifies this window so it can ignore the echo of its own broadcast.
// A random id rather than the window label because it only ever needs to
// answer "was this mine?", and it stays correct even if a label is reused.
const SELF = Math.random().toString(36).slice(2);

const STORAGE_CHANNEL = "shared-storage";
type StorageMsg = { key: string; value: string; from: string };

// Persist a localStorage key AND tell the other windows about it.
//
// The `getItem` guard is what stops an endless round trip: a window that
// applies a remote change re-runs its own persist effect and calls back in
// here with the value it was just handed. Identical value, nothing to say,
// no echo -- so the two windows settle instead of bouncing the same
// payload between each other forever.
export function writeShared(key: string, value: string): void {
  if (localStorage.getItem(key) === value) return;
  localStorage.setItem(key, value);
  void emit(STORAGE_CHANNEL, { key, value, from: SELF } satisfies StorageMsg);
}

// Re-hydrate one piece of state when another window writes `key`.
//
// `apply` is held in a ref so an inline arrow at the call site doesn't
// tear the listener down and re-register it on every render (which would
// drop events in the gap).
export function useSharedStorage(key: string, apply: (raw: string) => void): void {
  const applyRef = useRef(apply);
  applyRef.current = apply;
  useEffect(() => {
    const un = listen<StorageMsg>(STORAGE_CHANNEL, (e) => {
      if (e.payload.from === SELF || e.payload.key !== key) return;
      // Mirror into this window's own localStorage before handing the
      // value to React: the persist effect that `apply` is about to
      // trigger reads it back through `writeShared`'s guard, and that is
      // what recognises the change as already-applied.
      localStorage.setItem(e.payload.key, e.payload.value);
      applyRef.current(e.payload.value);
    });
    return () => {
      void un.then((f) => f());
    };
  }, [key]);
}

// ---------------------------------------------------------------------------
// Ephemeral (not persisted) cross-window state -- currently just the
// cut/copy clipboard. Deliberately NOT localStorage-backed: a pending
// "cut" is about this session, and restoring one from disk days later
// would arm a move the user has long forgotten about.
// ---------------------------------------------------------------------------

const LIVE_CHANNEL = "shared-live";
type LiveMsg = { key: string; value: unknown; from: string };

// Push a value to every other window. Called from the handful of places
// that actually change the value, never from an effect watching it --
// that is what keeps this one-directional and loop-free.
export function broadcast(key: string, value: unknown): void {
  void emit(LIVE_CHANNEL, { key, value, from: SELF } satisfies LiveMsg);
}

export function useBroadcast<T>(key: string, apply: (value: T) => void): void {
  const applyRef = useRef(apply);
  applyRef.current = apply;
  useEffect(() => {
    const un = listen<LiveMsg>(LIVE_CHANNEL, (e) => {
      if (e.payload.from === SELF || e.payload.key !== key) return;
      applyRef.current(e.payload.value as T);
    });
    return () => {
      void un.then((f) => f());
    };
  }, [key]);
}
