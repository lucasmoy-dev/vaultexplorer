import { MenuItem } from "./ContextMenu";

export interface SyncMenuState {
  drivePairsByPath: Map<string, string>;
  gitSyncedPaths: Set<string>;
  localSyncedPaths: Set<string>;
  setDriveTarget: (t: { path: string; provider: string } | null) => void;
  setGitSyncTarget: (p: string | null) => void;
  setLocalSyncTarget: (p: string | null) => void;
  setSyncthingTarget: (p: string | null) => void;
  // Mobile: only Google Drive is reachable, and through an entirely
  // different backend (the in-process Drive client, not rclone) -- so it
  // gets its own target rather than sharing `setDriveTarget`.
  mobile?: boolean;
  setMobileDriveTarget?: (p: string | null) => void;
  setMobileFolderSyncTarget?: (p: string | null) => void;
}

// Shared "Sync" submenu (Google Drive / OneDrive / Dropbox / Git / Local
// Folder / Syncthing, each with a checkmark if already paired) -- used by
// both the entry context menu and the favorites-sidebar context menu.
export function buildSyncSubmenu(path: string, sync: SyncMenuState): MenuItem {
  if (sync.mobile) {
    // The other options all shell out to a binary Android doesn't have
    // (rclone for OneDrive/Dropbox, git, unison, syncthing); listing them
    // here just to fail would be worse than not listing them.
    return {
      type: "submenu",
      label: "Sync",
      items: [
        {
          label: sync.drivePairsByPath.has(path) ? "Google Drive ✓…" : "Google Drive…",
          onClick: () => sync.setMobileDriveTarget?.(path),
        },
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
        label: sync.drivePairsByPath.get(path) === "drive" ? "Google Drive ✓…" : "Google Drive…",
        onClick: () => sync.setDriveTarget({ path, provider: "drive" }),
      },
      {
        label: sync.drivePairsByPath.get(path) === "onedrive" ? "OneDrive ✓…" : "OneDrive…",
        onClick: () => sync.setDriveTarget({ path, provider: "onedrive" }),
      },
      {
        label: sync.drivePairsByPath.get(path) === "dropbox" ? "Dropbox ✓…" : "Dropbox…",
        onClick: () => sync.setDriveTarget({ path, provider: "dropbox" }),
      },
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
