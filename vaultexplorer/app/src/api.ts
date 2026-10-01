import { Channel, invoke } from "@tauri-apps/api/core";
import { openPath as pluginOpenPath, openUrl as pluginOpenUrl } from "@tauri-apps/plugin-opener";
import { writeText as pluginWriteText } from "@tauri-apps/plugin-clipboard-manager";

// Resolved once and cached -- every caller awaits the same promise instead
// of re-invoking the command per call. A *failed* attempt is not cached:
// the very first invoke can reject if it races a cold-start IPC bridge
// (same race App.tsx's own mobile-detection retry loop documents) --
// caching that would permanently misroute every osOpen call for the rest
// of the session instead of just retrying next time.
let mobilePlatform: Promise<boolean> | null = null;
export function isMobilePlatformCached(): Promise<boolean> {
  if (!mobilePlatform) {
    mobilePlatform = invoke<boolean>("is_mobile_platform").catch(() => {
      mobilePlatform = null;
      return false;
    });
  }
  return mobilePlatform;
}

// Opens a local file path or a URL with whatever the OS has registered for
// it -- the one thing every caller actually wants, whether that's a real
// file (`osOpen(fullPath)`) or a link (`osOpen("https://...")`).
//
// On Android this is NOT a thin wrapper around `@tauri-apps/plugin-opener`
// for a local path: that plugin's Android side has a bug where `openPath`
// sends its mobile plugin a bare JSON string instead of the `{url: ...}`
// object its own Kotlin `OpenArgs` requires, throwing
// "no String-argument constructor... to deserialize from String value" on
// every single file open there (see `android_open_path` in
// `src-tauri/src/android.rs`, which this routes to instead).
//
// URLs use the plugin's separate `openUrl` command, not `openPath` -- an
// earlier version of this ran every URL through `openPath` too (reasoning
// that only *paths* were affected), but that's the exact same broken
// Android command under a different name, so YouTube/provider links from
// InternetView failed to open on mobile the same way local files used to.
// `openUrl` is a distinct plugin command with its own (working) Android
// implementation.
export async function osOpen(target: string): Promise<void> {
  const isUrl = /^[a-z][a-z0-9+.-]*:/i.test(target);
  if (isUrl) {
    await pluginOpenUrl(target);
    return;
  }
  if (await isMobilePlatformCached()) {
    await invoke<void>("android_open_path", { path: target });
    return;
  }
  await pluginOpenPath(target);
}

export interface ProgressEvent {
  done: number;
  total: number;
}

// Puts text on the system clipboard, and makes sure it got there.
//
// "Copy Absolute Path" used `navigator.clipboard.writeText`, the webview's
// DOM clipboard API, which WebKitGTK and Android's WebView only honour
// while the page holds focus *and* the call is still inside a fresh user
// gesture -- and which, when it doesn't honour it, may leave the clipboard
// exactly as it was. Whatever the user had copied before (a code-review
// comment, in the report) is then what gets pasted, with nothing in the
// app saying the copy never happened.
//
// Now the write goes through the native clipboard (the Tauri plugin:
// arboard on desktop, ClipboardManager on Android, neither of which cares
// about gestures or focus), is read back to confirm it took, falls back to
// the DOM API once if it didn't, and throws -- so the caller shows an
// error -- if the clipboard still doesn't hold the text.
export async function copyText(text: string): Promise<void> {
  let nativeError: unknown = null;
  try {
    await pluginWriteText(text);
  } catch (e) {
    nativeError = e;
  }
  if (await clipboardHolds(text)) return;
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    /* reported below */
  }
  if (await clipboardHolds(text)) return;
  throw new Error(`Couldn't copy to the clipboard${nativeError ? `: ${nativeError}` : ""}`);
}

// Read back through the backend (off the main thread -- reading the
// clipboard there while this process owns it can deadlock GTK). If the
// read itself isn't possible, the write is trusted rather than reported as
// a failure that may not have happened.
async function clipboardHolds(text: string): Promise<boolean> {
  try {
    return (await invoke<string>("clipboard_read_text")) === text;
  } catch {
    return true;
  }
}

export interface SearchHit {
  path: string;
  name: string;
  is_dir: boolean;
  is_vault: boolean;
  size: number;
  mtime: number;
  has_meta: boolean;
}
export interface SearchBatch {
  id: number;
  hits: SearchHit[];
  done: boolean;
  truncated: boolean;
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
  convertFolderToVault: (path: string, password: string) =>
    invoke<void>("convert_folder_to_vault", { path, password }),
  unlockVault: (path: string, password: string) =>
    invoke<void>("unlock_vault", { path, password }),
  verifyVaultPassword: (path: string, password: string) =>
    invoke<void>("verify_vault_password", { path, password }),
  lockVault: (root: string) => invoke<void>("lock_vault", { root }),
  setActiveVault: (root: string) => invoke<void>("set_active_vault", { root }),
  setVaultAutoUnlock: (root: string, password: string) =>
    invoke<void>("set_vault_auto_unlock", { root, password }),
  clearVaultAutoUnlock: (root: string) => invoke<void>("clear_vault_auto_unlock", { root }),
  autoUnlockVaults: (roots: string[]) => invoke<string[]>("auto_unlock_vaults", { roots }),

  // vault-internal (operate on the currently unlocked vault)
  listDir: (relPath: string) => invoke<Entry[]>("list_dir", { relPath }),
  vaultListDirAt: (root: string, relPath: string) =>
    invoke<Entry[]>("vault_list_dir_at", { root, relPath }),
  search: (query: string) => invoke<string[]>("search_vault", { query }),
  // Streamed, cancellable search (search.rs): resolves with the search's
  // id at once; hits arrive in batches on `channel`. Starting a new search
  // stops the previous one.
  searchStart: (kind: "fs" | "vault", root: string, query: string, channel: Channel<SearchBatch>) =>
    invoke<number>("search_start", { kind, root, query, channel }),
  searchCancel: () => invoke<void>("search_cancel"),
  moveEntry: (src: string, dest: string) => invoke<void>("move_entry", { src, dest }),
  copyEntry: (src: string, dest: string, channel: Channel<ProgressEvent>) =>
    invoke<void>("copy_entry", { src, dest, channel }),
  vaultToVaultCopy: (srcRoot: string, srcRel: string, destRoot: string, destRel: string) =>
    invoke<void>("vault_to_vault_copy", { srcRoot, srcRel, destRoot, destRel }),
  vaultToVaultMove: (srcRoot: string, srcRel: string, destRoot: string, destRel: string) =>
    invoke<void>("vault_to_vault_move", { srcRoot, srcRel, destRoot, destRel }),
  deleteFile: (relPath: string) => invoke<void>("delete_file", { relPath }),
  deleteDir: (relPath: string) => invoke<void>("delete_dir", { relPath }),
  makeDir: (relPath: string) => invoke<void>("make_dir", { relPath }),
  newFile: (relPath: string) => invoke<void>("new_file", { relPath }),
  importFile: (srcPath: string, destRel: string) =>
    invoke<void>("import_file", { srcPath, destRel }),
  exportFile: (relPath: string, destFsPath: string) =>
    invoke<void>("export_file", { relPath, destFsPath }),
  // sensitive files (per-file / per-folder re-auth gate)
  vaultSetSensitive: (relPath: string, sensitive: boolean) =>
    invoke<void>("vault_set_sensitive", { relPath, sensitive }),
  vaultIsSensitive: (relPath: string) => invoke<boolean>("vault_is_sensitive", { relPath }),
  vaultListSensitive: () => invoke<string[]>("vault_list_sensitive"),
  vaultUnlockSensitive: (password: string, timeoutSecs: number | null) =>
    invoke<void>("vault_unlock_sensitive", { password, timeoutSecs }),
  vaultSensitiveUnlocked: () => invoke<boolean>("vault_sensitive_unlocked"),
  vaultLockSensitive: () => invoke<void>("vault_lock_sensitive"),
  changeVaultPassword: (root: string, oldPassword: string, newPassword: string) =>
    invoke<void>("change_vault_password", { root, oldPassword, newPassword }),
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
  androidPinFolderShortcut: (id: string, label: string, url: string, iconBase64?: string) =>
    invoke<void>("android_pin_folder_shortcut", { id, label, url, iconBase64 }),
  androidDownloadAndInstallApk: (url: string) => invoke<void>("android_download_and_install_apk", { url }),
  androidCanInstallPackages: () => invoke<boolean>("android_can_install_packages"),
  androidRequestInstallPackagesAccess: () => invoke<void>("android_request_install_packages_access"),
  mediaUrl: (path: string) => invoke<string>("media_url", { path }),
  audioToMp3: (
    src: string,
    dest: string,
    title: string | null,
    removeSource: boolean,
    channel: Channel<ProgressEvent>
    // Returns the path actually written -- "name (2).mp3" when something
    // was already sitting at `dest`.
  ) => invoke<string>("audio_to_mp3", { src, dest, title, removeSource, channel }),
  fsTrashMany: (paths: string[], channel: Channel<ProgressEvent>) =>
    invoke<void>("fs_trash_many", { paths, channel }),
  openTerminal: (path: string, terminal: string) =>
    invoke<void>("open_terminal", { path, terminal }),
  runShellScript: (path: string, terminal: string) =>
    invoke<void>("run_shell_script", { path, terminal }),
  openInEditor: (path: string) => invoke<void>("open_in_editor", { path }),
  fsList: (path: string, showHidden: boolean) =>
    invoke<Entry[]>("fs_list", { path, showHidden }),
  fsIsVault: (path: string) => invoke<boolean>("fs_is_vault", { path }),
  fsSetReadonly: (path: string, readonly: boolean) =>
    invoke<void>("fs_set_readonly", { path, readonly }),
  fsIsReadonly: (path: string) => invoke<boolean>("fs_is_readonly", { path }),
  fsSearch: (root: string, query: string) => invoke<string[]>("fs_search", { root, query }),
  fsMkdir: (path: string) => invoke<void>("fs_mkdir", { path }),
  fsNewFile: (path: string) => invoke<void>("fs_new_file", { path }),
  fsReadText: (path: string) => invoke<string>("fs_read_text", { path }),
  fsWriteText: (path: string, content: string) => invoke<void>("fs_write_text", { path, content }),
  fsWriteBytes: (path: string, bytes: Uint8Array) => invoke<void>("fs_write_bytes", { path, bytes }),
  fsSavePastedImage: (dir: string, bytes: number[]) =>
    invoke<string>("fs_save_pasted_image", { dir, bytes }),
  vaultReadText: (relPath: string) => invoke<string>("vault_read_text", { relPath }),
  vaultWriteText: (relPath: string, content: string) =>
    invoke<void>("vault_write_text", { relPath, content }),
  vaultWriteBytes: (relPath: string, bytes: Uint8Array) =>
    invoke<void>("vault_write_bytes", { relPath, bytes }),
  fsShareFile: (path: string) => invoke<string>("fs_share_file", { path }),
  vaultShareFile: (relPath: string) => invoke<string>("vault_share_file", { relPath }),
  // OS share sheet (WhatsApp/etc) for a real file already on disk --
  // Android only, distinct from fsShareFile/vaultShareFile above (those
  // upload to a public host and return a link instead).
  androidSharePath: (path: string) => invoke<void>("android_share_path", { path }),
  fsDelete: (path: string) => invoke<void>("fs_delete", { path }),
  fsSecureDelete: (paths: string[], channel: Channel<ProgressEvent>) =>
    invoke<void>("fs_secure_delete", { paths, channel }),
  fsTrash: (path: string) => invoke<void>("fs_trash", { path }),
  scanLargeFiles: (roots: string[], channel: Channel<LargeFilesEvent>) =>
    invoke<void>("scan_large_files", { roots, channel }),
  trashDir: () => invoke<string>("trash_dir"),
  emptyTrash: () => invoke<void>("empty_trash"),
  trashRestoreAll: () => invoke<void>("trash_restore_all"),
  trashRestore: (names: string[]) => invoke<void>("trash_restore", { names }),
  trashPurge: (names: string[]) => invoke<void>("trash_purge", { names }),
  templatesDir: () => invoke<string>("templates_dir"),
  portalIsEnabled: () => invoke<boolean>("portal_is_enabled"),
  portalEnable: () => invoke<void>("portal_enable"),
  portalDisable: () => invoke<void>("portal_disable"),
  // Owning the `inode/directory` MIME default (so folders opened from any
  // other app land here) -- separate from the portal toggle above, which
  // is only about Open/Save dialogs. See defaultapp.rs.
  defaultFileManagerEnabled: () => invoke<boolean>("default_file_manager_enabled"),
  setDefaultFileManager: (enabled: boolean) =>
    invoke<void>("set_default_file_manager", { enabled }),
  autostartEnabled: () => invoke<boolean>("autostart_enabled"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  listAppsForPath: (path: string) =>
    invoke<{ id: string; name: string; icon: string | null; is_default: boolean }[]>(
      "list_apps_for_path",
      { path }
    ),
  // A "Show in folder" request that arrived before the UI existed (D-Bus
  // activation starting the app); null when the app was already running and
  // the `show-in-folder` event handled it. See filemanager1.rs.
  reportWindowLocation: (path: string | null) => invoke<void>("report_window_location", { path }),
  takePendingReveal: () =>
    invoke<{ path: string; select: string | null } | null>("take_pending_reveal"),
  // Every installed app (not just this file's registered handlers), for the
  // "Other Application…" picker. Icons come back as theme names and are
  // resolved in batches by `appIcons` for the rows actually on screen.
  listAllApps: () =>
    invoke<{ id: string; name: string; comment: string | null; icon_name: string | null }[]>(
      "list_all_apps"
    ),
  appIcons: (icons: string[]) => invoke<(string | null)[]>("app_icons", { icons }),
  openWith: (path: string, desktopId: string) =>
    invoke<void>("open_with", { path, desktopId }),
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
  // Renders into the shared freedesktop cache and returns the PNG's path
  // (see thumbcache.rs); `fsThumbnail` above is the data-URI path kept for
  // sizes past the cache's largest class.
  fsThumb: (path: string, maxSize: number) => invoke<string>("fs_thumb", { path, maxSize }),
  vaultThumbnail: (relPath: string, maxSize: number) =>
    invoke<string>("vault_thumbnail", { relPath, maxSize }),
  fsCopyImageToClipboard: (path: string) =>
    invoke<void>("fs_copy_image_to_clipboard", { path }),
  vaultCopyImageToClipboard: (relPath: string) =>
    invoke<void>("vault_copy_image_to_clipboard", { relPath }),
  // "Is there an image on the system clipboard?" -- kept as a bool so
  // greying out the Paste button doesn't drag a whole screenshot across
  // IPC. The bytes come over as an ArrayBuffer only when actually pasting.
  clipboardHasImage: () => invoke<boolean>("clipboard_has_image"),
  clipboardReadImagePng: () => invoke<ArrayBuffer>("clipboard_read_image_png"),
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
  // One rasterized PDF page as a data URI, plus the document's page count:
  // the preview pane's PDF viewer (the webview has no PDF renderer of its
  // own, so pages are drawn by poppler on the Rust side).
  fsPdfPage: (path: string, page: number, maxSize: number) =>
    invoke<string>("fs_pdf_page", { path, page, maxSize }),
  fsPdfPageCount: (path: string) => invoke<number>("fs_pdf_page_count", { path }),
  fsConvertMedia: (
    path: string,
    destPath: string,
    targetExt: string,
    quality: "high" | "medium" | "low",
    channel: Channel<ProgressEvent>
  ) => invoke<void>("fs_convert_media", { path, destPath, targetExt, quality, channel }),
  // Re-encode a video into a much smaller file at the same resolution and
  // duration (HEVC/H.264 at constant quality) -- "this video doesn't need
  // to be 4GB", as opposed to fsConvertMedia's format change.
  fsShrinkVideo: (
    path: string,
    destPath: string,
    level: "light" | "balanced" | "small",
    channel: Channel<ProgressEvent>
  ) => invoke<void>("fs_shrink_video", { path, destPath, level, channel }),
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
  // Mobile-only alternative to openPath: no FUSE mount exists on Android, so
  // handing a vault file to another app means decrypting it to a throwaway
  // copy in the app's cache dir (shareable via FileProvider) instead.
  vaultDecryptToTemp: (relPath: string) => invoke<string>("vault_decrypt_to_temp", { relPath }),

  fsWatchSet: (path: string | null) => invoke<void>("fs_watch_set", { path }),
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









export interface ClearResult {
  name: string;
  cleared: boolean;
  reason: string | null;
}

export interface LargeFile {
  path: string;
  name: string;
  size: number;
}

// A running top-N snapshot streamed from `scan_large_files` while it's
// still walking -- `files` is already sorted largest-first, `scanned` is
// a live "how far in" counter, and `done` marks the final message (the
// walk has finished, `files` is its last word).
export interface LargeFilesEvent {
  files: LargeFile[];
  scanned: number;
  done: boolean;
}

/** One playable file, as the Music view sees it. */

/** What "Update song data" did to one file. */





export interface DiskInfo {
  name: string;
  size: string;
  mountpoint: string | null;
  type: string;
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
  // MemAvailable, not MemFree -- see the note on the Rust side.
  ram_available: number;
  swap_total: number;
  swap_free: number;
  uptime_secs: number;
  load1: number;
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
