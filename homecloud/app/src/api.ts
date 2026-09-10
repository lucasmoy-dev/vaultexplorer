import { invoke } from "@tauri-apps/api/core";

export type FolderState =
  | { kind: "upToDate" }
  | { kind: "syncing"; percent: number }
  | { kind: "paused" }
  | { kind: "disconnected" }
  | { kind: "problem"; detail: string };

export interface Peer {
  id: string;
  name: string;
  connected: boolean;
  /**
   * How much of the folder that device already has, 0-100, or null when the
   * engine cannot say. This is the only way the device that handed a folder
   * out can see whether the other end has finished.
   */
  completion: number | null;
}

/** What one device does with a folder it shares. */
export type FolderMode = "twoWay" | "receiveOnly" | "archive";

export interface SharedFolder {
  id: string;
  label: string;
  path: string;
  state: FolderState;
  peers: Peer[];
  bytes: number;
  files: number;
  conflicts: number;
  bytesPerSecond: number;
  mode: FolderMode;
  /** What this copy holds that the others no longer have. */
  extraBytes: number;
  freeBytes: number | null;
  pendingBytes: number;
  wifiOnly: boolean;
  pausedByNetwork: boolean;
  hasPassword: boolean;
  /** Seconds left at the speed measured just now, or null if nothing moves. */
  etaSeconds: number | null;
}

export interface OfferedFolder {
  id: string;
  label: string;
}

export interface Invitation {
  fromDeviceId: string;
  fromDeviceName: string;
  folder: OfferedFolder | null;
}

export interface ThisDevice {
  id: string;
  name: string;
}

export interface Readiness {
  ready: boolean;
  device: ThisDevice | null;
  problem: string | null;
}

/** One file that was deleted and can still be brought back. */
export interface DeletedFile {
  id: string;
  name: string;
  originalPath: string;
  /** Seconds since the epoch. */
  deletedAt: number;
  bytes: number;
  /** In the desktop's own recycle bin, rather than the hidden folder. */
  inSystemBin: boolean;
}

/** Where a file goes when another device deletes or replaces it. */
export type DeletionPolicy = "bin" | "copies" | "nothing";

export interface Settings {
  deviceName: string;
  deviceId: string;
  localNetworkOnly: boolean;
  uploadLimitKbps: number;
  downloadLimitKbps: number;
  deletionPolicy: DeletionPolicy;
  keepVersions: number;
  engineVersion: string;
  language: "es" | "en";
}

export interface LinkStatus {
  url: string;
  /** Seconds since the epoch. Past this, the link has stopped on its own. */
  expiresAt: number;
}

export interface CodePreview {
  deviceName: string;
  folderLabel: string;
  suggestedPath: string;
  bytes: number | null;
}

/** Which of the two readings of a chosen directory is in force. */
export type Pick = "inside" | "itself";

export interface Destination {
  path: string;
  pick: Pick;
  explanation: string;
  freeBytes: number | null;
}



export const api = {
  readiness: () => invoke<Readiness>("readiness"),
  retryEngine: () => invoke<void>("retry_engine"),
  listFolders: () => invoke<SharedFolder[]>("list_folders"),
  listInvitations: () => invoke<Invitation[]>("list_invitations"),
  shareFolder: (path: string, label: string) => invoke<string>("share_folder", { path, label }),
  codeFor: (folderId: string) => invoke<string>("code_for", { folderId }),
  previewCode: (code: string, password?: string) =>
    invoke<CodePreview>("preview_code", { code, password: password ?? null }),
  redeemCode: (code: string, localPath: string, password?: string) =>
    invoke<void>("redeem_code", { code, localPath, password: password ?? null }),
  setFolderPassword: (folderId: string, password: string) =>
    invoke<void>("set_folder_password", { folderId, password }),
  linkStart: (folderId: string, path: string, password: string) =>
    invoke<LinkStatus>("link_start", { folderId, path, password }),
  linkStop: (folderId: string) => invoke<void>("link_stop", { folderId }),
  linkFor: (folderId: string) => invoke<LinkStatus | null>("link_for", { folderId }),
  resolveDestination: (chosen: string, label: string, pick?: Pick) =>
    invoke<Destination>("resolve_destination", { chosen, label, pick: pick ?? null }),
  setFolderMode: (folderId: string, mode: FolderMode) =>
    invoke<void>("set_folder_mode", { folderId, mode }),
  rescan: (folderId: string) => invoke<void>("rescan", { folderId }),
  setFolderWifiOnly: (folderId: string, wifiOnly: boolean) =>
    invoke<void>("set_folder_wifi_only", { folderId, wifiOnly }),
  reconnectAll: () => invoke<void>("reconnect_all"),
  downloadUpdate: (url: string) => invoke<string>("download_update", { url }),
  autostartEnabled: () => invoke<boolean>("autostart_enabled"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  forgetUnusedDevices: () => invoke<string[]>("forget_unused_devices"),
  reportCameraProblem: (detail: string) => invoke<void>("report_camera_problem", { detail }),
  suggestedPath: (label: string) => invoke<string>("suggested_path", { label }),
  acceptInvitation: (invitation: Invitation, localPath: string | null) =>
    invoke<void>("accept_invitation", { invitation, localPath }),
  declineInvitation: (invitation: Invitation) => invoke<void>("decline_invitation", { invitation }),
  setFolderPaused: (folderId: string, paused: boolean) =>
    invoke<void>("set_folder_paused", { folderId, paused }),
  stopSharing: (folderId: string) => invoke<void>("stop_sharing", { folderId }),
  deletedFiles: (folderId: string) => invoke<DeletedFile[]>("deleted_files", { folderId }),
  restoreDeleted: (folderId: string, id: string) =>
    invoke<string>("restore_deleted", { folderId, id }),
  settings: () => invoke<Settings>("settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
};

export function formatBytes(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  const units = ["kB", "MB", "GB", "TB"];
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0).replace(".", ",")} ${units[unit]}`;
}

/**
 * Kept free so filling a disk does not take the rest of the machine with it.
 * The same figure the core uses, because two different answers to "does it
 * fit" is worse than either of them.
 */
export const DISK_RESERVE = 1_000_000_000;

/** How much more room a folder of `needed` bytes wants. Zero when it fits. */
export function shortfall(needed: number, free: number): number {
  return Math.max(0, needed - Math.max(0, free - DISK_RESERVE));
}

/** A transfer rate a person can read, e.g. "2,4 MB/s". */
export function formatRate(bytesPerSecond: number): string {
  if (bytesPerSecond <= 0) return "";
  const mb = bytesPerSecond / 1_000_000;
  if (mb >= 1) return `${mb.toFixed(1).replace(".", ",")} MB/s`;
  return `${Math.round(bytesPerSecond / 1000)} kB/s`;
}

/**
 * How long is left, in the roundest terms that are still useful: "2 h 15 min",
 * "8 min", "45 s". A percentage alone never answers the question people
 * actually have, which is whether to wait for it.
 */
export function formatEta(seconds: number): string {
  if (seconds >= 36 * 3600) return "más de un día";
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.round((seconds % 3600) / 60);
  if (hours > 0) return minutes > 0 ? `${hours} h ${minutes} min` : `${hours} h`;
  if (seconds >= 60) return `${Math.max(1, Math.round(seconds / 60))} min`;
  return `${Math.max(1, seconds)} s`;
}

/** What is left of a sync, as one line: "faltan 2 h 15 min · 15,0 MB/s". */
export function remaining(folder: {
  etaSeconds: number | null;
  bytesPerSecond: number;
}): string | null {
  const parts: string[] = [];
  if (folder.etaSeconds !== null) parts.push(`faltan ${formatEta(folder.etaSeconds)}`);
  const rate = formatRate(folder.bytesPerSecond);
  if (rate) parts.push(rate);
  return parts.length > 0 ? parts.join(" · ") : null;
}

/** "hace 3 días", "hace 2 h": when something was deleted, in words. */
export function timeAgo(seconds: number): string {
  const elapsed = Math.max(0, Math.floor(Date.now() / 1000 - seconds));
  if (elapsed < 90) return "hace un momento";
  if (elapsed < 3600) return `hace ${Math.round(elapsed / 60)} min`;
  if (elapsed < 86400) return `hace ${Math.round(elapsed / 3600)} h`;
  const days = Math.round(elapsed / 86400);
  return days === 1 ? "ayer" : `hace ${days} días`;
}

/** The tail of a device ID, so two devices with the same name are still telling apart. */
export function shortId(id: string): string {
  return id.split("-")[0] ?? id;
}

/** The one line under a folder's name. Says who it syncs with, not how. */
export function peerSummary(peers: Peer[]): string {
  if (peers.length === 0) return "Sin dispositivos todavía";
  if (peers.length === 1) return peers[0].name;
  const connected = peers.filter((p) => p.connected).length;
  return `${peers.length} dispositivos · ${connected} conectados`;
}
