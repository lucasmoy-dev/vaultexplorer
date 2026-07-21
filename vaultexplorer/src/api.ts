import { Channel, invoke } from "@tauri-apps/api/core";

export interface ProgressEvent {
  done: number;
  total: number;
}

export interface Entry {
  name: string;
  is_dir: boolean;
  is_vault?: boolean; // a nested vault (real fs) or nested vault (vault-internal)
  size: number;
  mtime: number; // unix epoch seconds
  created?: number; // unix epoch seconds, only meaningful for real-filesystem entries
  is_hidden?: boolean; // real-filesystem entries only
}

export const api = {
  // vault lifecycle
  vaultExists: (path: string) => invoke<boolean>("vault_exists", { path }),
  createVault: (path: string, password: string) =>
    invoke<void>("create_vault", { path, password }),
  unlockVault: (path: string, password: string) =>
    invoke<void>("unlock_vault", { path, password }),
  lockVault: (root: string) => invoke<void>("lock_vault", { root }),
  setActiveVault: (root: string) => invoke<void>("set_active_vault", { root }),
  setVaultAutoUnlock: (root: string, password: string) =>
    invoke<void>("set_vault_auto_unlock", { root, password }),
  clearVaultAutoUnlock: (root: string) => invoke<void>("clear_vault_auto_unlock", { root }),
  autoUnlockVaults: (roots: string[]) => invoke<string[]>("auto_unlock_vaults", { roots }),

  // vault-internal (operate on the currently unlocked vault)
  listDir: (relPath: string) => invoke<Entry[]>("list_dir", { relPath }),
  search: (query: string) => invoke<string[]>("search_vault", { query }),
  moveEntry: (src: string, dest: string) => invoke<void>("move_entry", { src, dest }),
  copyEntry: (src: string, dest: string) => invoke<void>("copy_entry", { src, dest }),
  deleteFile: (relPath: string) => invoke<void>("delete_file", { relPath }),
  deleteDir: (relPath: string) => invoke<void>("delete_dir", { relPath }),
  makeDir: (relPath: string) => invoke<void>("make_dir", { relPath }),
  newFile: (relPath: string) => invoke<void>("new_file", { relPath }),
  importFile: (srcPath: string, destRel: string) =>
    invoke<void>("import_file", { srcPath, destRel }),
  exportFile: (relPath: string, destFsPath: string) =>
    invoke<void>("export_file", { relPath, destFsPath }),
  setFilePassword: (relPath: string, filePassword: string) =>
    invoke<void>("set_file_password", { relPath, filePassword }),
  openPath: (relPath: string) => invoke<string>("open_path", { relPath }),
  compressEntries: (
    dir: string,
    names: string[],
    destName: string,
    password: string | null = null,
    level: number | null = null,
    readme: string | null = null
  ) => invoke<void>("compress_entries", { dir, names, destName, password, level, readme }),
  decompressEntry: (zipRelPath: string, destDirRelPath: string, password: string | null = null) =>
    invoke<void>("decompress_entry", { zipRelPath, destDirRelPath, password }),
  dirSize: (relPath: string) => invoke<number>("dir_size", { relPath }),

  // real filesystem
  homeDir: () => invoke<string>("browse_root_dir"),
  isMobilePlatform: () => invoke<boolean>("is_mobile_platform"),
  androidStorageAccessGranted: () => invoke<boolean>("android_storage_access_granted"),
  androidRequestStorageAccess: () => invoke<void>("android_request_storage_access"),
  openTerminal: (path: string, terminal: string) =>
    invoke<void>("open_terminal", { path, terminal }),
  runShellScript: (path: string, terminal: string) =>
    invoke<void>("run_shell_script", { path, terminal }),
  openInEditor: (path: string) => invoke<void>("open_in_editor", { path }),
  fsList: (path: string, showHidden: boolean) =>
    invoke<Entry[]>("fs_list", { path, showHidden }),
  fsSetReadonly: (path: string, readonly: boolean) =>
    invoke<void>("fs_set_readonly", { path, readonly }),
  fsIsReadonly: (path: string) => invoke<boolean>("fs_is_readonly", { path }),
  fsSearch: (root: string, query: string) => invoke<string[]>("fs_search", { root, query }),
  fsMkdir: (path: string) => invoke<void>("fs_mkdir", { path }),
  fsNewFile: (path: string) => invoke<void>("fs_new_file", { path }),
  fsReadText: (path: string) => invoke<string>("fs_read_text", { path }),
  fsWriteText: (path: string, content: string) => invoke<void>("fs_write_text", { path, content }),
  fsSavePastedImage: (dir: string, bytes: number[]) =>
    invoke<string>("fs_save_pasted_image", { dir, bytes }),
  vaultReadText: (relPath: string) => invoke<string>("vault_read_text", { relPath }),
  vaultWriteText: (relPath: string, content: string) =>
    invoke<void>("vault_write_text", { relPath, content }),
  fsShareFile: (path: string) => invoke<string>("fs_share_file", { path }),
  vaultShareFile: (relPath: string) => invoke<string>("vault_share_file", { relPath }),
  fsDelete: (path: string) => invoke<void>("fs_delete", { path }),
  fsSecureDelete: (paths: string[], channel: Channel<ProgressEvent>) =>
    invoke<void>("fs_secure_delete", { paths, channel }),
  fsTrash: (path: string) => invoke<void>("fs_trash", { path }),
  trashDir: () => invoke<string>("trash_dir"),
  emptyTrash: () => invoke<void>("empty_trash"),
  templatesDir: () => invoke<string>("templates_dir"),
  gitRepoRoot: (path: string) => invoke<string | null>("git_repo_root", { path }),
  gitStatus: (root: string) => invoke<GitFileStatus[]>("git_status", { root }),
  gitPull: (root: string) => invoke<string>("git_pull", { root }),
  gitPush: (root: string) => invoke<string>("git_push", { root }),
  gitCommitAll: (root: string, message: string) => invoke<string>("git_commit_all", { root, message }),
  gitStage: (root: string, path: string) => invoke<void>("git_stage", { root, path }),
  gitUnstage: (root: string, path: string) => invoke<void>("git_unstage", { root, path }),
  gitDiscard: (root: string, path: string) => invoke<void>("git_discard", { root, path }),

  // System file-picker portal integration
  portalIsEnabled: () => invoke<boolean>("portal_is_enabled"),
  portalEnable: () => invoke<void>("portal_enable"),
  portalDisable: () => invoke<void>("portal_disable"),
  portalResolve: (requestId: string, uris: string[]) =>
    invoke<void>("portal_resolve", { requestId, uris }),
  portalCancel: (requestId: string) => invoke<void>("portal_cancel", { requestId }),

  // File recovery (photorec wrapper)
  recoveryToolAvailable: () => invoke<boolean>("recovery_tool_available"),
  recoveryListDisks: () => invoke<DiskInfo[]>("recovery_list_disks"),
  machineListDrives: () => invoke<Drive[]>("machine_list_drives"),
  machineSummary: () => invoke<MachineSummary>("machine_summary"),
  machineAdvancedInfo: () => invoke<AdvancedInfo>("machine_advanced_info"),
  machineUpdateDrivers: () => invoke<void>("machine_update_drivers"),
  machineFormatDrive: (device: string, fsType: string, label: string) =>
    invoke<void>("machine_format_drive", { device, fsType, label }),
  recoverySameDisk: (device: string, destDir: string) =>
    invoke<boolean>("recovery_same_disk", { device, destDir }),
  recoveryRun: (device: string, destDir: string) => invoke<void>("recovery_run", { device, destDir }),

  // Freeze Folder
  freezeFolder: (path: string, password: string) => invoke<void>("freeze_folder", { path, password }),
  listFrozenFolders: () => invoke<FreezeMeta[]>("list_frozen_folders"),
  unfreezeFolder: (path: string, password: string, keepChanges: boolean) =>
    invoke<void>("unfreeze_folder", { path, password, keepChanges }),
  fsRename: (src: string, dest: string) => invoke<void>("fs_rename", { src, dest }),
  fsCopy: (src: string, dest: string, channel: Channel<ProgressEvent>) =>
    invoke<void>("fs_copy", { src, dest, channel }),
  fsCompress: (
    dir: string,
    names: string[],
    destName: string,
    channel: Channel<ProgressEvent>,
    password: string | null = null,
    level: number | null = null,
    readme: string | null = null
  ) => invoke<void>("fs_compress", { dir, names, destName, password, level, readme, channel }),
  fsCompressTargz: (dir: string, names: string[], destName: string, channel: Channel<ProgressEvent>) =>
    invoke<void>("fs_compress_targz", { dir, names, destName, channel }),
  fsDecompress: (
    zipPath: string,
    destDir: string,
    channel: Channel<ProgressEvent>,
    password: string | null = null
  ) => invoke<void>("fs_decompress", { zipPath, destDir, password, channel }),
  archiveMount: (path: string, password: string | null = null) =>
    invoke<string>("archive_mount", { path, password }),
  archiveUnmount: (mountpoint: string) => invoke<void>("archive_unmount", { mountpoint }),
  archiveMountsLeftBehind: (newPath: string) =>
    invoke<string[]>("archive_mounts_left_behind", { newPath }),
  archiveAllMounts: () => invoke<string[]>("archive_all_mounts"),
  fsThumbnail: (path: string, maxSize: number) =>
    invoke<string>("fs_thumbnail", { path, maxSize }),
  vaultThumbnail: (relPath: string, maxSize: number) =>
    invoke<string>("vault_thumbnail", { relPath, maxSize }),
  fsCopyImageToClipboard: (path: string) =>
    invoke<void>("fs_copy_image_to_clipboard", { path }),
  vaultCopyImageToClipboard: (relPath: string) =>
    invoke<void>("vault_copy_image_to_clipboard", { relPath }),
  fsClearMetadata: (paths: string[], channel: Channel<ProgressEvent>) =>
    invoke<ClearResult[]>("fs_clear_metadata", { paths, channel }),
  vaultClearMetadata: (relPaths: string[], channel: Channel<ProgressEvent>) =>
    invoke<ClearResult[]>("vault_clear_metadata", { relPaths, channel }),

  fsFileInfo: (path: string) => invoke<[string, string][]>("fs_file_info", { path }),
  vaultFileInfo: (relPath: string) => invoke<[string, string][]>("vault_file_info", { relPath }),
  convertFfmpegAvailable: () => invoke<boolean>("convert_ffmpeg_available"),
  fsConvertImage: (path: string, destPath: string, targetExt: string, quality: number | null = null) =>
    invoke<void>("fs_convert_image", { path, destPath, targetExt, quality }),
  vaultConvertImage: (relPath: string, destRelPath: string, targetExt: string, quality: number | null = null) =>
    invoke<void>("vault_convert_image", { relPath, destRelPath, targetExt, quality }),
  fsResizeImages: (paths: string[], width: number, height: number, channel: Channel<ProgressEvent>) =>
    invoke<void>("fs_resize_images", { paths, width, height, channel }),
  vaultResizeImages: (relPaths: string[], width: number, height: number, channel: Channel<ProgressEvent>) =>
    invoke<void>("vault_resize_images", { relPaths, width, height, channel }),
  convertLibreofficeAvailable: () => invoke<boolean>("convert_libreoffice_available"),
  fsConvertOffice: (path: string, destDir: string, targetExt: string) =>
    invoke<string>("fs_convert_office", { path, destDir, targetExt }),
  fsPdfToImages: (path: string, destDir: string, destStem: string) =>
    invoke<string[]>("fs_pdf_to_images", { path, destDir, destStem }),
  fsImageToPdf: (path: string, destPath: string) => invoke<void>("fs_image_to_pdf", { path, destPath }),
  transcribeModelDownloaded: () => invoke<boolean>("transcribe_model_downloaded"),
  transcribeDownloadModel: (channel: Channel<ProgressEvent>) =>
    invoke<void>("transcribe_download_model", { channel }),
  transcribeRun: (path: string, destTxtPath: string, channel: Channel<ProgressEvent>) =>
    invoke<void>("transcribe_run", { path, destTxtPath, channel }),
  fsConvertMedia: (
    path: string,
    destPath: string,
    targetExt: string,
    quality: "high" | "medium" | "low",
    channel: Channel<ProgressEvent>
  ) => invoke<void>("fs_convert_media", { path, destPath, targetExt, quality, channel }),
  fsBuildMontage: (
    visualPaths: string[],
    audioPath: string | null,
    destPath: string,
    width: number,
    height: number,
    quality: "high" | "medium" | "low",
    includeOriginalAudio: boolean,
    channel: Channel<ProgressEvent>
  ) =>
    invoke<void>("fs_build_montage", {
      visualPaths,
      audioPath,
      destPath,
      width,
      height,
      quality,
      includeOriginalAudio,
      channel,
    }),
  fsCreateShortcut: (target: string, dest: string) =>
    invoke<void>("fs_create_shortcut", { target, dest }),
  fsDirSize: (path: string) => invoke<number>("fs_dir_size", { path }),
  fsGetTags: (dir: string) => invoke<Record<string, string>>("fs_get_tags", { dir }),
  fsSetTag: (dir: string, name: string, color: string | null) =>
    invoke<void>("fs_set_tag", { dir, name, color }),
  fsEncryptFile: (path: string, password: string) =>
    invoke<string>("fs_encrypt_file", { path, password }),
  fsDecryptFile: (path: string, password: string) =>
    invoke<string>("fs_decrypt_file", { path, password }),
  encryptFileInVault: (relPath: string, password: string) =>
    invoke<string>("encrypt_file_in_vault", { relPath, password }),
  decryptFileInVault: (relPath: string, password: string) =>
    invoke<string>("decrypt_file_in_vault", { relPath, password }),

  // Cloud sync (Google Drive, OneDrive, Dropbox, ...), via rclone (see
  // rclone.rs) -- no client ID/secret setup step, rclone's own bundled
  // OAuth client per provider handles that.
  rcloneInstalled: () => invoke<boolean>("rclone_installed"),
  rcloneProviders: () => invoke<[string, string][]>("rclone_providers"),
  rcloneIsConnected: (provider: string) => invoke<boolean>("rclone_is_connected", { provider }),
  rcloneConnect: (provider: string, urlChannel: Channel<string>) =>
    invoke<void>("rclone_connect", { provider, urlChannel }),
  rcloneDisconnect: (provider: string) => invoke<void>("rclone_disconnect", { provider }),
  driveListPairs: () => invoke<SyncPair[]>("drive_list_pairs"),
  driveAddPair: (provider: string, localPath: string) =>
    invoke<SyncPair>("drive_add_pair", { provider, localPath }),
  driveRemovePair: (localPath: string) => invoke<void>("drive_remove_pair", { localPath }),
  driveSyncNow: (localPath: string) => invoke<SyncReport>("drive_sync_now", { localPath }),
  driveSyncingNow: () => invoke<string[]>("drive_syncing_now"),
  gitSyncListPairs: () => invoke<GitSyncPair[]>("git_sync_list_pairs"),
  gitSyncIsActive: (localPath: string) => invoke<boolean>("git_sync_is_active", { localPath }),
  gitSyncSyncingNow: () => invoke<string[]>("git_sync_syncing_now"),
  gitSyncAdd: (localPath: string, remoteUrl: string, repoName: string) =>
    invoke<GitSyncPair>("git_sync_add", { localPath, remoteUrl, repoName }),
  gitSyncRemove: (localPath: string) => invoke<void>("git_sync_remove", { localPath }),
  localSyncAvailable: () => invoke<boolean>("local_sync_available"),
  localSyncListPairs: () => invoke<LocalSyncPair[]>("local_sync_list_pairs"),
  localSyncIsActive: (folderA: string, folderB: string) =>
    invoke<boolean>("local_sync_is_active", { folderA, folderB }),
  localSyncSyncingNow: () => invoke<string[]>("local_sync_syncing_now"),
  localSyncAdd: (folderA: string, folderB: string) =>
    invoke<string[]>("local_sync_add", { folderA, folderB }),
  localSyncRemove: (folderA: string, folderB: string) =>
    invoke<void>("local_sync_remove", { folderA, folderB }),
  localSyncNow: (folderA: string, folderB: string) =>
    invoke<string[]>("local_sync_now", { folderA, folderB }),
  syncthingInstalled: () => invoke<boolean>("syncthing_installed"),
  syncthingSyncingNow: () => invoke<string[]>("syncthing_syncing_now"),
  syncthingMyDeviceId: () => invoke<string>("syncthing_my_device_id"),
  syncthingQrSvg: (data: string) => invoke<string>("syncthing_qr_svg", { data }),
  syncthingListDevices: () => invoke<SyncthingDevice[]>("syncthing_list_devices"),
  syncthingAddDevice: (id: string, name: string) => invoke<void>("syncthing_add_device", { id, name }),
  syncthingRemoveDevice: (id: string) => invoke<void>("syncthing_remove_device", { id }),
  syncthingListFolders: () => invoke<SyncthingFolder[]>("syncthing_list_folders"),
  syncthingShareFolder: (folderId: string, label: string, path: string, deviceIds: string[]) =>
    invoke<void>("syncthing_share_folder", { folderId, label, path, deviceIds }),
  syncthingRemoveFolder: (folderId: string) => invoke<void>("syncthing_remove_folder", { folderId }),
  syncthingPendingDevices: () => invoke<SyncthingPendingDevice[]>("syncthing_pending_devices"),
  syncthingPendingFolders: () => invoke<SyncthingPendingFolder[]>("syncthing_pending_folders"),
  // Native OS-level drag-out (tauri-plugin-drag), so dropping a real file
  // onto an external app (a browser tab, another native app) actually
  // hands it real bytes -- HTML5's own `dataTransfer` never does, no
  // matter what's put in it, since it can't carry real files across
  // process boundaries.
  startFileDrag: (paths: string[], image?: string) =>
    invoke<void>("plugin:drag|start_drag", {
      item: paths,
      // A 1x1 transparent pixel fallback -- callers normally pass a real
      // (synchronously-built, so the drag starts without any extra delay)
      // translucent icon instead; see buildDragImage in App.tsx.
      image:
        image ??
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
      options: { mode: "copy" },
      onEvent: new Channel(),
    }),
};

export interface SyncPair {
  local_path: string;
  provider: string;
  drive_folder_name: string;
  resynced: boolean;
}

export interface GitSyncPair {
  local_path: string;
  remote_url: string;
  repo_name: string;
}

export interface LocalSyncPair {
  folder_a: string;
  folder_b: string;
}

export interface SyncthingDevice {
  id: string;
  name: string;
  connected: boolean;
}

export interface SyncthingFolder {
  id: string;
  label: string;
  path: string;
  device_ids: string[];
}

export interface SyncthingPendingDevice {
  id: string;
}

export interface SyncthingPendingFolder {
  id: string;
  label: string;
  offered_by_device_id: string;
}

export interface SyncReport {
  summary: string;
}

export interface ClearResult {
  name: string;
  cleared: boolean;
  reason: string | null;
}

export interface GitFileStatus {
  path: string;
  status: string;
}

export interface DiskInfo {
  name: string;
  size: string;
  mountpoint: string | null;
  type: string;
}

export interface FreezeMeta {
  original_path: string;
  frozen_at: number;
}

export interface Drive {
  path: string;
  name: string;
  label: string | null;
  fstype: string | null;
  mountpoint: string | null;
  removable: boolean;
  model: string | null;
  total: number;
  used: number;
  free: number;
}

export interface MachineSummary {
  cpu_model: string;
  cpu_cores: number;
  ram_total: number;
  os_name: string;
  disks: Drive[];
}

export interface PciDeviceInfo {
  address: string;
  description: string;
  driver: string | null;
  kind: string;
}

export interface DriverRecommendation {
  vendor: string;
  driver: string;
}

export interface AdvancedInfo {
  board_vendor: string;
  board_name: string;
  board_version: string;
  bios_vendor: string;
  bios_version: string;
  product_name: string;
  pci_devices: PciDeviceInfo[];
  driver_recommendations: DriverRecommendation[];
  ubuntu_drivers_available: boolean;
}

export const ENCRYPTED_FILE_EXT = ".vlt";

export const TAG_COLORS: { key: string; label: string; hex: string }[] = [
  { key: "red", label: "Red", hex: "#ff5f56" },
  { key: "orange", label: "Orange", hex: "#ff9f0a" },
  { key: "yellow", label: "Yellow", hex: "#ffd60a" },
  { key: "green", label: "Green", hex: "#32d74b" },
  { key: "blue", label: "Blue", hex: "#0a84ff" },
  { key: "purple", label: "Purple", hex: "#bf5af2" },
  { key: "gray", label: "Gray", hex: "#8e8e93" },
];

export function joinPath(dir: string, name: string): string {
  if (dir === "") return name;
  if (dir === "/") return `/${name}`;
  return `${dir}/${name}`;
}

export function parentPath(path: string): string {
  const idx = path.lastIndexOf("/");
  if (idx === -1) return "";
  if (idx === 0) return "/"; // parent of "/foo" is "/"
  return path.slice(0, idx);
}

export function baseName(path: string): string {
  const trimmed = path.endsWith("/") && path.length > 1 ? path.slice(0, -1) : path;
  const idx = trimmed.lastIndexOf("/");
  return idx === -1 ? trimmed : trimmed.slice(idx + 1);
}

const SIZE_UNITS = ["bytes", "KB", "MB", "GB", "TB"];

export function formatSize(bytes: number): string {
  if (bytes <= 0) return "0 bytes";
  if (bytes < 1000) return `${bytes} bytes`;
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < SIZE_UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${SIZE_UNITS[unit]}`;
}

export function formatDate(epochSeconds: number): string {
  if (!epochSeconds) return "—";
  const d = new Date(epochSeconds * 1000);
  const date = d.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return `${date}, ${time}`;
}
