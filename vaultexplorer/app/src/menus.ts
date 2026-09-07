import { MenuItem } from "./ContextMenu";

export interface SyncMenuState {
  gitSyncedPaths: Set<string>;
  localSyncedPaths: Set<string>;
  setGitSyncTarget: (p: string | null) => void;
  setLocalSyncTarget: (p: string | null) => void;
  setSyncthingTarget: (p: string | null) => void;
  mobile?: boolean;
  setMobileFolderSyncTarget?: (p: string | null) => void;
}

// Shared "Sync" submenu (Git / Local Folder / Syncthing, each with a
// checkmark if already paired) -- used by both the entry context menu and
// the favorites-sidebar context menu.
export function buildSyncSubmenu(path: string, sync: SyncMenuState): MenuItem {
  if (sync.mobile) {
    // The other options all shell out to a binary Android doesn't have
    // (git, unison, syncthing); listing them here just to fail would be
    // worse than not listing them.
    return {
      type: "submenu",
      label: "Sync",
      items: [
        {
          label: sync.localSyncedPaths.has(path) ? "Local Folder ✓…" : "Local Folder…",
          onClick: () => sync.setMobileFolderSyncTarget?.(path),
        },
      ],
    };
  }
  return {
    type: "submenu",
    label: "Sync",
    items: [
      {
        label: sync.gitSyncedPaths.has(path) ? "Git ✓…" : "Git…",
        onClick: () => sync.setGitSyncTarget(path),
      },
      {
        label: sync.localSyncedPaths.has(path) ? "Local Folder ✓…" : "Local Folder…",
        onClick: () => sync.setLocalSyncTarget(path),
      },
      { label: "Sync P2P…", onClick: () => sync.setSyncthingTarget(path) },
    ],
  };
}
