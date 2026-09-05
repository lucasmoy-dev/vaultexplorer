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
}

export interface SharedFolder {
  id: string;
  label: string;
  path: string;
  state: FolderState;
  peers: Peer[];
  bytes: number;
  files: number;
  conflicts: number;
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

export interface Settings {
  deviceName: string;
  deviceId: string;
  localNetworkOnly: boolean;
  uploadLimitKbps: number;
  downloadLimitKbps: number;
  keepVersions: number;
  engineVersion: string;
}

export interface CodePreview {
  deviceName: string;
  folderLabel: string;
  suggestedPath: string;
}

/** Which of the two readings of a chosen directory is in force. */
export type Pick = "inside" | "itself";

export interface Destination {
  path: string;
  pick: Pick;
  explanation: string;
}

export interface PairingWindow {
  folderId: string;
  folderLabel: string;
  secondsLeft: number;
}

export const api = {
  readiness: () => invoke<Readiness>("readiness"),
  retryEngine: () => invoke<void>("retry_engine"),
  listFolders: () => invoke<SharedFolder[]>("list_folders"),
  listInvitations: () => invoke<Invitation[]>("list_invitations"),
  shareFolder: (path: string, label: string) => invoke<string>("share_folder", { path, label }),
  codeFor: (folderId: string) => invoke<string>("code_for", { folderId }),
  previewCode: (code: string) => invoke<CodePreview>("preview_code", { code }),
  redeemCode: (code: string, localPath: string) => invoke<void>("redeem_code", { code, localPath }),
  resolveDestination: (chosen: string, label: string, pick?: Pick) =>
    invoke<Destination>("resolve_destination", { chosen, label, pick: pick ?? null }),
  pairingWindow: () => invoke<PairingWindow | null>("pairing_window"),
  closePairingWindow: () => invoke<void>("close_pairing_window"),
  forgetUnusedDevices: () => invoke<string[]>("forget_unused_devices"),
  suggestedPath: (label: string) => invoke<string>("suggested_path", { label }),
  acceptInvitation: (invitation: Invitation, localPath: string | null) =>
    invoke<void>("accept_invitation", { invitation, localPath }),
  declineInvitation: (invitation: Invitation) => invoke<void>("decline_invitation", { invitation }),
  setFolderPaused: (folderId: string, paused: boolean) =>
    invoke<void>("set_folder_paused", { folderId, paused }),
  stopSharing: (folderId: string) => invoke<void>("stop_sharing", { folderId }),
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
