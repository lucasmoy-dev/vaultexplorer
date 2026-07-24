import { useEffect, useState, useCallback, useMemo, useRef } from "react";
import { Channel } from "@tauri-apps/api/core";
import { openPath as osOpen } from "@tauri-apps/plugin-opener";
import { getCurrent as getCurrentDeepLinks, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  api,
  Entry,
  ProgressEvent,
  joinPath,
  parentPath,
  baseName,
  formatSize,
  formatDate,
  TAG_COLORS,
  ENCRYPTED_FILE_EXT,
} from "./api";
import { TitleBar, TrafficLights } from "./TitleBar";
import { ContextMenu, MenuState, MenuItem } from "./ContextMenu";
import {
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  ChevronDown,
  MenuGlyph,
  RefreshGlyph,
  IconViewGlyph,
  ListViewGlyph,
  ListPreviewGlyph,
  ColumnViewGlyph,
  SearchGlyph,
  LockOpenGlyph,
  PlaceGlyph,
  DiskGlyph,
  ComputerGlyph,
  CopyGlyph,
  CheckGlyph,
  TrashGlyph,
  GitBranchGlyph,
  CloudSyncGlyph,
  LocalSyncGlyph,
  SettingsGlyph,
  kindOf,
  customIconUrl,
  symbolIconSvg,
} from "./icons";
import { Loc, Clipboard, View, ProgressOp, PendingAction, VaultCreateOptions, SensitiveTimeout } from "./types";
import { ProgressPanel } from "./components/ProgressPanel";
import { kindLabel } from "./entryHelpers";
import { EntryTile } from "./components/EntryTile";
import { MyComputerView } from "./components/MyComputerView";
import { SearchResults } from "./components/SearchResults";
import { FilePreviewPane } from "./components/TextEditorPane";
import { ColumnView } from "./components/ColumnView";
import { PickerView } from "./components/PickerView";
import { buildSyncSubmenu } from "./menus";
import { useSelection } from "./hooks/useSelection";
import { useFavorites } from "./hooks/useFavorites";
import { DEFAULT_START_KEY, PHONE_STORAGE_PATH } from "./constants";
import {
  ActionSheet,
  UnlockSheet,
  SensitiveUnlockSheet,
  ReauthOverlay,
  ZipPasswordSheet,
  EncryptFileSheet,
  NewVaultSheet,
  UnfreezeSheet,
} from "./components/sheets/vault-sheets";
import {
  GitStatusSheet,
  DriveSyncSheet,
  GitSyncSheet,
  LocalSyncSheet,
  SyncthingSheet,
  parseAddDeviceLink,
} from "./components/sheets/sync-sheets";
import {
  ChangeIconSheet,
  ResizeSheet,
  ConvertSheet,
  MontageOptionsSheet,
  CompressOptionsSheet,
} from "./components/sheets/media-sheets";
import {
  ManageTemplatesSheet,
  MachineInfoSheet,
  FormatDriveSheet,
  SettingsSheet,
} from "./components/sheets/system-sheets";
import { GetInfoSheet, MultiInfoSheet } from "./components/sheets/info-sheets";
import "./App.css";

// Expands {date}/{time}/{datetime} tokens in a user-configured default
// name template. Periods instead of colons in {time} -- colons aren't
// filename-safe on Windows/some filesystems, and this app runs on more
// than just Linux desktops.
function formatNameTemplate(template: string): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}.${pad(now.getMinutes())}hs`;
  return template
    .replace(/\{datetime\}/g, `${date} ${time}`)
    .replace(/\{date\}/g, date)
    .replace(/\{time\}/g, time);
}

// ---------- main explorer ----------

const ARCHIVE_EXT_RE = /\.(zip|tar\.gz|tgz)$/i;
function startPath(home: string): string {
  return localStorage.getItem(DEFAULT_START_KEY) || home;
}

// A translucent drag-image for the native OS-level drag (see beginDrag) --
// previously a blank 1x1 pixel, so dragging a real file/folder out to
// another app showed nothing at all following the cursor. Built purely
// synchronously (canvas draw + toDataURL, no thumbnail fetch/`Image`
// load) so it can't add any delay before `startFileDrag` actually begins
// the drag -- GTK's native DnD needs to start right on the same gesture
// that triggered it, and any extra async step here risked making the
// already-finicky native drag-out even less reliable.
function buildDragImage(entry: Entry, count: number): string | undefined {
  const size = 72;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d");
  if (!ctx) return undefined;
  ctx.globalAlpha = 0.62;
  ctx.fillStyle = entry.is_dir ? "#5aa9f7" : "#8e8e93";
  if (entry.is_dir) {
    ctx.beginPath();
    ctx.moveTo(6, 20);
    ctx.lineTo(28, 20);
    ctx.lineTo(34, 28);
    ctx.lineTo(66, 28);
    ctx.lineTo(66, 60);
    ctx.lineTo(6, 60);
    ctx.closePath();
    ctx.fill();
  } else {
    ctx.beginPath();
    ctx.roundRect(18, 8, 36, 56, 4);
    ctx.fill();
  }
  if (count > 1) {
    ctx.globalAlpha = 1;
    ctx.fillStyle = "#ff3b30";
    ctx.beginPath();
    ctx.arc(size - 12, 12, 11, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = "#fff";
    ctx.font = "bold 13px sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(String(count), size - 12, 12);
  }
  return canvas.toDataURL("image/png");
}

function Explorer({ home }: { home: string }) {
  // Gates desktop-only surfaces that either can't function on Android/iOS
  // (no OS trash, no terminal, no FUSE-based freeze, no D-Bus drive
  // formatting) or don't make sense there (floating-window chrome).
  const [mobile, setMobile] = useState(false);
  useEffect(() => {
    api.isMobilePlatform().then((v) => {
      setMobile(v);
      document.documentElement.classList.toggle("is-mobile", v);
    }).catch(() => {});
  }, []);
  // The desktop sidebar is a permanently docked column -- on a phone-width
  // screen that eats half the usable width, so on mobile it becomes an
  // off-canvas drawer instead, opened via the hamburger button and closed
  // by tapping an item, the backdrop, or Escape (already wired below).
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [loc, setLoc] = useState<Loc>({ kind: "fs", path: startPath(home) });
  const [unlockedRoots, setUnlockedRoots] = useState<Set<string>>(new Set());
  // Where to navigate back to when a vault is locked: its real fs parent
  // for a top-level vault, or the parent vault's own loc for a nested one.
  const [vaultParents, setVaultParents] = useState<Record<string, Loc>>({});
  // Per-vault-root, not a single flag -- it used to be one shared
  // boolean, which meant unlocking any *other* vault without checking
  // "keep unlocked" reset it for everyone, silently locking a vault the
  // user had explicitly asked to keep open the next time they stepped
  // out of it.
  const [keepUnlockedRoots, setKeepUnlockedRoots] = useState<Set<string>>(new Set());
  const [appSettings, setAppSettings] = useState<{
    showHiddenFiles: boolean;
    hideExtensions: boolean;
    terminalApp: string;
    newFileNameTemplate: string;
    newFolderNameTemplate: string;
    theme: "light" | "dark" | "system";
    sensitiveTimeout: SensitiveTimeout;
  }>(() => {
    const defaults = {
      showHiddenFiles: false,
      hideExtensions: false,
      terminalApp: "ghostty",
      newFileNameTemplate: "{datetime}",
      newFolderNameTemplate: "untitled folder",
      theme: "system" as const,
      sensitiveTimeout: 1200 as SensitiveTimeout,
    };
    try {
      const raw = localStorage.getItem("vaultexplorer:app-settings");
      if (raw) {
        const stored = JSON.parse(raw);
        // One-time upgrade: earlier builds persisted the literal default
        // "untitled document" the moment *any* setting changed (the
        // save-on-change effect below writes the whole object back), so
        // switching the code default alone wouldn't reach anyone who'd
        // already launched the app once. Only touches it if it's still
        // exactly the old placeholder -- an actual custom value is left
        // alone either way.
        if (stored.newFileNameTemplate === "untitled document") {
          stored.newFileNameTemplate = "{datetime}";
        }
        return { ...defaults, ...stored };
      }
    } catch {
      /* ignore */
    }
    return defaults;
  });
  useEffect(() => {
    localStorage.setItem("vaultexplorer:app-settings", JSON.stringify(appSettings));
  }, [appSettings]);
  // P2P device pairing via a shared `vaultexplorer://add-device` link:
  // `getCurrent` covers being *launched* by one (Linux/Windows spawn a
  // fresh instance with the URL as a CLI arg, without the single-instance
  // plugin to forward it to an already-running window instead), and
  // `onOpenUrl` covers the live case wherever the platform supports it.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    getCurrentDeepLinks()
      .then((urls) => {
        for (const url of urls ?? []) {
          const parsed = parseAddDeviceLink(url);
          if (parsed) setIncomingDevice(parsed);
        }
      })
      .catch(() => {});
    onOpenUrl((urls) => {
      for (const url of urls) {
        const parsed = parseAddDeviceLink(url);
        if (parsed) setIncomingDevice(parsed);
      }
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});
    return () => unlisten?.();
  }, []);
  // "System" removes the attribute entirely so the plain
  // @media (prefers-color-scheme) CSS governs -- which already updates
  // live on its own, no listener needed (verified by toggling the GTK
  // theme under a running window). Only Light/Dark need this attribute,
  // to force a choice that may disagree with the OS setting.
  useEffect(() => {
    if (appSettings.theme === "system") delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = appSettings.theme;
  }, [appSettings.theme]);
  const [settingsOpen, setSettingsOpen] = useState(false);

  // ---- progress panel (bottom-right, Finder-style) ----
  const [progressOps, setProgressOps] = useState<ProgressOp[]>([]);
  const progressIdRef = useRef(0);
  // Creates a Channel wired to update/auto-remove a row in `progressOps`.
  // Not a hook -- called imperatively from inside the async operations
  // (paste/compress/decompress) themselves, one channel per invocation.
  function beginProgress(label: string): Channel<ProgressEvent> {
    const id = ++progressIdRef.current;
    const channel = new Channel<ProgressEvent>();
    // The Channel's own numeric id doubles as the backend cancel key
    // (the commands register under channel.id() -- see ops.rs).
    const cancelId = (channel as unknown as { id: number }).id;
    setProgressOps((prev) => [...prev, { id, label, done: 0, total: 1, cancelId, status: "running" }]);
    channel.onmessage = (e) => {
      setProgressOps((prev) => prev.map((p) => (p.id === id ? { ...p, done: e.done, total: e.total } : p)));
      if (e.done >= e.total) {
        setTimeout(() => setProgressOps((prev) => prev.filter((p) => p.id !== id)), 1000);
      }
    };
    return channel;
  }
  // Cancel a running operation from the footer X: tell the backend to abort
  // (kills child processes / trips the loop cancel flag) and drop the row.
  const cancelProgress = useCallback((op: ProgressOp) => {
    if (op.cancelId != null) {
      import("@tauri-apps/api/core").then(({ invoke }) =>
        invoke("cancel_operation", { opId: op.cancelId }).catch(() => {})
      );
    }
    setProgressOps((prev) => prev.map((p) => (p.id === op.id ? { ...p, status: "cancelled" } : p)));
    setTimeout(() => setProgressOps((prev) => prev.filter((p) => p.id !== op.id)), 600);
  }, []);

  // ---- share (upload + copy link) ----
  const [shareStatus, setShareStatus] = useState<{
    label: string;
    state: "working" | "done" | "error";
    message?: string;
  } | null>(null);
  async function shareFile(entry: Entry) {
    setShareStatus({ label: entry.name, state: "working" });
    try {
      const url = inVault
        ? await api.vaultShareFile(joinPath(curDir, entry.name))
        : await api.fsShareFile(joinPath(curDir, entry.name));
      await navigator.clipboard.writeText(url);
      setShareStatus({ label: entry.name, state: "done", message: url });
      setTimeout(() => setShareStatus((s) => (s?.label === entry.name ? null : s)), 5000);
    } catch (e) {
      setShareStatus({ label: entry.name, state: "error", message: String(e) });
    }
  }

  const [vaultSettings, setVaultSettings] = useState<Record<string, VaultCreateOptions>>(() => {
    try {
      const raw = localStorage.getItem("vaultexplorer:vault-settings");
      if (raw) return JSON.parse(raw);
    } catch {
      /* ignore */
    }
    return {};
  });
  useEffect(() => {
    localStorage.setItem("vaultexplorer:vault-settings", JSON.stringify(vaultSettings));
  }, [vaultSettings]);
  const [reauthPrompt, setReauthPrompt] = useState<{ root: string; name: string } | null>(null);
  const [reauthError, setReauthError] = useState("");

  // Try auto-unlocking every vault marked "Unlock automatically", once, at
  // startup -- silently skips any whose stored password no longer matches.
  useEffect(() => {
    const autoRoots = Object.entries(vaultSettings)
      .filter(([, s]) => s.autoUnlock)
      .map(([root]) => root);
    if (autoRoots.length === 0) return;
    api
      .autoUnlockVaults(autoRoots)
      .then((unlocked) => {
        if (unlocked.length === 0) return;
        setUnlockedRoots((prev) => {
          const next = new Set(prev);
          unlocked.forEach((r) => next.add(r));
          return next;
        });
      })
      .catch(() => {});
    // run once at mount only
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Sensitive vaults auto-lock after a period of inactivity.
  useEffect(() => {
    if (loc.kind !== "vault") return;
    const settings = vaultSettings[loc.root];
    if (!settings?.sensitive) return;
    let timer: ReturnType<typeof setTimeout>;
    const reset = () => {
      clearTimeout(timer);
      timer = setTimeout(() => lockCurrentVault(), (settings.autoLockMinutes || 15) * 60_000);
    };
    reset();
    window.addEventListener("mousemove", reset);
    window.addEventListener("keydown", reset);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("mousemove", reset);
      window.removeEventListener("keydown", reset);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loc, vaultSettings]);

  // Sensitive vaults ask to re-authenticate (with a blurred background)
  // whenever the app window regains focus after having lost it.
  const hasBlurredRef = useRef(false);
  useEffect(() => {
    function onBlur() {
      hasBlurredRef.current = true;
    }
    function onFocus() {
      if (hasBlurredRef.current && loc.kind === "vault" && vaultSettings[loc.root]?.sensitive) {
        setReauthError("");
        setReauthPrompt({ root: loc.root, name: baseName(loc.root) });
      }
      hasBlurredRef.current = false;
    }
    window.addEventListener("blur", onBlur);
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("focus", onFocus);
    };
  }, [loc, vaultSettings]);

  async function submitReauth(password: string) {
    if (!reauthPrompt) return;
    try {
      await api.unlockVault(reauthPrompt.root, password);
      setReauthPrompt(null);
    } catch {
      setReauthError("Incorrect password");
    }
  }
  const [entries, setEntries] = useState<Entry[]>([]);
  const [tags, setTags] = useState<Record<string, string>>({});
  // Git root/status for the current real-fs folder, refreshed alongside
  // the directory listing itself -- null gitRoot means "not inside a git
  // repo" (or we're inside a vault, where this is scoped out for v1).
  const [gitRoot, setGitRoot] = useState<string | null>(null);
  const [gitStatus, setGitStatus] = useState<Record<string, string>>({});
  const [ffmpegAvailable, setFfmpegAvailable] = useState(false);
  useEffect(() => {
    api.convertFfmpegAvailable().then(setFfmpegAvailable).catch(() => setFfmpegAvailable(false));
  }, []);
  const [libreofficeAvailable, setLibreofficeAvailable] = useState(false);
  useEffect(() => {
    api.convertLibreofficeAvailable().then(setLibreofficeAvailable).catch(() => setLibreofficeAvailable(false));
  }, []);

  async function runOfficeConvert(entry: Entry, targetExt: string) {
    try {
      await api.fsConvertOffice(joinPath(curDir, entry.name), curDir, targetExt);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function runPdfToImages(entry: Entry) {
    const stem = uniqueName(entry.name.replace(/\.pdf$/i, "")).replace(/\.[^.]*$/, "");
    try {
      await api.fsPdfToImages(joinPath(curDir, entry.name), curDir, stem);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function runImageToPdf(entry: Entry) {
    const destName = uniqueName(`${entry.name.replace(/\.[^.]+$/, "")}.pdf`);
    try {
      await api.fsImageToPdf(joinPath(curDir, entry.name), joinPath(curDir, destName));
      await refresh();
      selectOnly(destName);
    } catch (e) {
      setError(String(e));
    }
  }

  async function runTranscribe(entry: Entry) {
    try {
      if (!(await api.transcribeModelDownloaded())) {
        await api.transcribeDownloadModel(beginProgress("Downloading transcription model (one-time, ~75MB)"));
      }
      const destName = uniqueName(`${entry.name.replace(/\.[^.]+$/, "")}.txt`);
      await api.transcribeRun(
        joinPath(curDir, entry.name),
        joinPath(curDir, destName),
        beginProgress(`Transcribing "${entry.name}"`)
      );
      await refresh();
      selectOnly(destName);
    } catch (e) {
      setError(String(e));
    }
  }
  const [convertTarget, setConvertTarget] = useState<{
    entry: Entry;
    targetExt: string;
    targetLabel: string;
    mode: "imageQuality" | "mediaQuality";
  } | null>(null);
  const [resizeTarget, setResizeTarget] = useState<string[] | null>(null);
  const [montageTarget, setMontageTarget] = useState<{
    visual: string[];
    audio: string | null;
    imageCount: number;
    videoCount: number;
  } | null>(null);

  async function runMontage(
    target: { visual: string[]; audio: string | null },
    opts: { width: number; height: number; quality: "high" | "medium" | "low"; includeOriginalAudio: boolean }
  ) {
    const destName = uniqueName("Montage.mp4");
    try {
      await api.fsBuildMontage(
        target.visual.map((n) => joinPath(curDir, n)),
        target.audio ? joinPath(curDir, target.audio) : null,
        joinPath(curDir, destName),
        opts.width,
        opts.height,
        opts.quality,
        opts.includeOriginalAudio,
        beginProgress(`Building "${destName}"`)
      );
      await refresh();
      selectOnly(destName);
    } catch (e) {
      setError(String(e));
    }
  }

  async function runResize(names: string[], width: number, height: number) {
    const paths = names.map((n) => joinPath(curDir, n));
    try {
      inVault
        ? await api.vaultResizeImages(paths, width, height, beginProgress("Resizing"))
        : await api.fsResizeImages(paths, width, height, beginProgress("Resizing"));
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  function extOf(entry: Entry): string {
    return entry.name
      .toLowerCase()
      .replace(new RegExp(`\\${ENCRYPTED_FILE_EXT}$`), "")
      .split(".")
      .pop()!;
  }

  async function runImageConvert(entry: Entry, targetExt: string, quality: number | null) {
    const destName = uniqueName(`${entry.name.replace(/\.[^.]+$/, "")}.${targetExt}`);
    try {
      inVault
        ? await api.vaultConvertImage(joinPath(curDir, entry.name), joinPath(curDir, destName), targetExt, quality)
        : await api.fsConvertImage(joinPath(curDir, entry.name), joinPath(curDir, destName), targetExt, quality);
      await refresh();
      selectOnly(destName);
    } catch (e) {
      setError(String(e));
    }
  }

  async function runMediaConvert(entry: Entry, targetExt: string, quality: "high" | "medium" | "low") {
    const destName = uniqueName(`${entry.name.replace(/\.[^.]+$/, "")}.${targetExt}`);
    try {
      await api.fsConvertMedia(
        joinPath(curDir, entry.name),
        joinPath(curDir, destName),
        targetExt,
        quality,
        beginProgress(`Converting "${entry.name}"`)
      );
      await refresh();
      selectOnly(destName);
    } catch (e) {
      setError(String(e));
    }
  }

  const IMAGE_CONVERT_TARGETS: { ext: string; label: string; lossy: boolean }[] = [
    { ext: "jpg", label: "JPEG", lossy: true },
    { ext: "png", label: "PNG", lossy: false },
    { ext: "webp", label: "WebP", lossy: false },
    { ext: "bmp", label: "BMP", lossy: false },
    { ext: "tiff", label: "TIFF", lossy: false },
    { ext: "gif", label: "GIF", lossy: false },
  ];
  const VIDEO_CONVERT_TARGETS: { ext: string; label: string }[] = [
    { ext: "mp4", label: "MP4" },
    { ext: "mkv", label: "MKV" },
    { ext: "webm", label: "WebM" },
    { ext: "mov", label: "MOV" },
  ];
  const AUDIO_CONVERT_TARGETS: { ext: string; label: string; lossy: boolean }[] = [
    { ext: "mp3", label: "MP3", lossy: true },
    { ext: "wav", label: "WAV", lossy: false },
    { ext: "flac", label: "FLAC", lossy: false },
    { ext: "ogg", label: "OGG", lossy: true },
    { ext: "m4a", label: "M4A", lossy: true },
  ];

  const [frozenPaths, setFrozenPaths] = useState<Set<string>>(new Set());
  const [unfreezeTarget, setUnfreezeTarget] = useState<string | null>(null);
  const refreshFrozen = useCallback(() => {
    api
      .listFrozenFolders()
      .then((list) => setFrozenPaths(new Set(list.map((m) => m.original_path))))
      .catch(() => setFrozenPaths(new Set()));
  }, []);

  // Which folders currently sync (Drive and/or Git), for the sidebar/grid
  // badge and for showing "Unsync" instead of "Sync…" in the menu.
  const [driveSyncedPaths, setDriveSyncedPaths] = useState<Set<string>>(new Set());
  // path -> provider id ("drive"/"onedrive"/"dropbox"), for the Sync
  // submenu's per-provider checkmark -- `driveSyncedPaths` alone (used
  // for the badge, which looks the same regardless of provider) can't
  // tell which specific one a path is linked to.
  const [drivePairsByPath, setDrivePairsByPath] = useState<Map<string, string>>(new Map());
  const [gitSyncedPaths, setGitSyncedPaths] = useState<Set<string>>(new Set());
  const [localSyncedPaths, setLocalSyncedPaths] = useState<Set<string>>(new Set());
  const refreshSyncStatus = useCallback(() => {
    api
      .driveListPairs()
      .then((list) => {
        setDriveSyncedPaths(new Set(list.map((p) => p.local_path)));
        setDrivePairsByPath(new Map(list.map((p) => [p.local_path, p.provider])));
      })
      .catch(() => {
        setDriveSyncedPaths(new Set());
        setDrivePairsByPath(new Map());
      });
    api
      .gitSyncListPairs()
      .then((list) => setGitSyncedPaths(new Set(list.map((p) => p.local_path))))
      .catch(() => setGitSyncedPaths(new Set()));
    api
      .localSyncListPairs()
      .then((list) => setLocalSyncedPaths(new Set(list.flatMap((p) => [p.folder_a, p.folder_b]))))
      .catch(() => setLocalSyncedPaths(new Set()));
  }, []);
  // Live "is a sync actually happening right now" for whichever paths are
  // sync-managed -- the badge (grid tile, sidebar favorite) swaps to a
  // spinning version of the same icon while true, the same convention
  // Dropbox/OneDrive/Google Drive's own desktop clients use, rather than
  // a permanently-static badge that can't tell "watched" apart from
  // "actually moving data right now".
  const [syncingPaths, setSyncingPaths] = useState<Set<string>>(new Set());
  // Whatever just *finished* syncing gets a brief green "done" checkmark
  // (same convention as Dropbox/OneDrive/Google Drive Desktop: spinner
  // while active, a transient green check right after, then back to the
  // plain idle badge) rather than either state lingering forever.
  const [justSyncedPaths, setJustSyncedPaths] = useState<Set<string>>(new Set());
  useEffect(() => {
    let cancelled = false;
    let prev = new Set<string>();
    // Drive's background auto-sync loop is entirely best-effort (same as
    // git/local sync's loops) -- a pair stuck failing every tick (e.g.
    // rclone's own "too many deletes" safety abort) would otherwise fail
    // silently forever with nothing ever telling the user. Tracked per
    // path so the same failure doesn't re-show the banner every poll.
    const shownDriveErrors = new Map<string, string>();
    function poll() {
      Promise.all([
        api.gitSyncSyncingNow().catch(() => []),
        api.localSyncSyncingNow().catch(() => []),
        api.driveSyncingNow().catch(() => []),
        api.syncthingSyncingNow().catch(() => []),
      ]).then((results) => {
        if (cancelled) return;
        const next = new Set(results.flat());
        const justFinished = [...prev].filter((p) => !next.has(p));
        prev = next;
        setSyncingPaths(next);
        if (justFinished.length) {
          setJustSyncedPaths((cur) => new Set([...cur, ...justFinished]));
          setTimeout(() => {
            if (cancelled) return;
            setJustSyncedPaths((cur) => {
              const after = new Set(cur);
              justFinished.forEach((p) => after.delete(p));
              return after;
            });
          }, 2500);
        }
      });
      for (const path of driveSyncedPaths) {
        api
          .driveSyncLastError(path)
          .then((err) => {
            if (cancelled) return;
            if (err && shownDriveErrors.get(path) !== err) {
              shownDriveErrors.set(path, err);
              setError(`Drive sync failed for "${baseName(path)}": ${err}`);
            } else if (!err) {
              shownDriveErrors.delete(path);
            }
          })
          .catch(() => {});
      }
    }
    poll();
    const interval = setInterval(poll, 2500);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [driveSyncedPaths]);
  const [sortKey, setSortKey] = useState<"name" | "date" | "size" | "kind" | "created">("name");
  const [sortDir, setSortDir] = useState<1 | -1>(1);
  const { selected, setSelected, lastClicked, setLastClicked, selectOnly, toggle, selectRange: selectRangeByNames } =
    useSelection();
  // Shift+Arrow range selection: `arrowAnchorRef` is the fixed end of the
  // range (set once when a shift-arrow sequence begins), `arrowFocusRef`
  // is the end that moves with each press -- same anchor+focus model
  // shift-click already uses, just driven by keys instead of a click.
  // Any plain click resets both, so the next shift-arrow starts fresh.
  const arrowAnchorRef = useRef<string | null>(null);
  const arrowFocusRef = useRef<string | null>(null);
  const [view, setView] = useState<View>("icon");
  // Icon-grid column count, computed from the live content width instead of
  // CSS `auto-fill`. auto-fill floors the column count and leaves any
  // leftover < one-min-track as dead space on the right; this rounds
  // *optimistically* (adds a column once ~60% of another tile fits) and
  // lets the tracks stretch to fill, so a window that "looks like it has
  // room for one more column" actually gets it. 0 = not measured yet.
  const [gridCols, setGridCols] = useState(0);
  const [clipboard, setClipboard] = useState<Clipboard>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchExpanded, setSearchExpanded] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [searchResults, setSearchResults] = useState<string[] | null>(null);
  const [error, setError] = useState("");
  const [infoMsg, setInfoMsg] = useState("");
  useEffect(() => {
    if (!infoMsg) return;
    const t = setTimeout(() => setInfoMsg(""), 2500);
    return () => clearTimeout(t);
  }, [infoMsg]);
  const [pending, setPending] = useState<PendingAction | null>(null);
  const [sheetError, setSheetError] = useState("");
  const [sensitivePrompt, setSensitivePrompt] = useState<{
    path: string;
    proceed: () => void;
    error: string;
  } | null>(null);
  const sensitiveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Raw set of paths explicitly marked sensitive in the current vault (as
  // returned by the backend). Used for badges + the mark/unmark menu state;
  // `isSensitivePath` expands it to inherited descendants.
  const [sensitiveSet, setSensitiveSet] = useState<Set<string>>(new Set());
  const [menu, setMenu] = useState<MenuState>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const [marquee, setMarquee] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(
    null
  );
  const [renaming, setRenaming] = useState<{ name: string; value: string } | null>(null);
  const [compressTarget, setCompressTarget] = useState<string[] | null>(null);
  const [zipPasswordPrompt, setZipPasswordPrompt] = useState<{ entry: Entry; error: string } | null>(
    null
  );
  const [archiveMountPrompt, setArchiveMountPrompt] = useState<{
    dir: string;
    entry: Entry;
    error: string;
  } | null>(null);
  const [encryptTarget, setEncryptTarget] = useState<Entry | null>(null);
  const [driveTarget, setDriveTarget] = useState<{ path: string; provider: string } | null>(null);
  const [gitSyncTarget, setGitSyncTarget] = useState<string | null>(null);
  const [localSyncTarget, setLocalSyncTarget] = useState<string | null>(null);
  const [syncthingTarget, setSyncthingTarget] = useState<string | null>(null);
  const [incomingDevice, setIncomingDevice] = useState<{ id: string; name: string } | null>(null);
  const [iconTarget, setIconTarget] = useState<string | null>(null);
  const [decryptPrompt, setDecryptPrompt] = useState<{
    entry: Entry;
    error: string;
    mode: "open" | "inplace";
  } | null>(null);
  const [multiInfoTarget, setMultiInfoTarget] = useState<string[] | null>(null);
  const [infoTarget, setInfoTarget] = useState<{
    entry: Entry;
    fullPath: string;
    kind: Loc["kind"];
    root?: string;
  } | null>(null);
  const [iconScale, setIconScale] = useState(1);
  const [trashPath, setTrashPath] = useState<string | null>(null);
  useEffect(() => {
    api.trashDir().then(setTrashPath).catch(() => {});
  }, []);

  // "My Computer": a special sidebar entry that isn't a real fs path, so
  // it swaps the whole content area for a drive list instead of calling
  // `go()` -- picking a mounted drive there is what actually navigates.
  const [showMyComputer, setShowMyComputer] = useState(false);
  const [favCollapsed, setFavCollapsed] = useState(false);
  const [drives, setDrives] = useState<import("./api").Drive[]>([]);
  const [drivesError, setDrivesError] = useState("");
  const [machineInfoOpen, setMachineInfoOpen] = useState(false);
  const [formatTarget, setFormatTarget] = useState<import("./api").Drive | null>(null);
  const refreshDrives = useCallback(() => {
    api
      .machineListDrives()
      .then((d) => {
        setDrives(d);
        setDrivesError("");
      })
      .catch((e) => setDrivesError(String(e)));
  }, []);
  function openMyComputer() {
    setShowMyComputer(true);
    setSearchResults(null);
    refreshDrives();
  }

  // Some Linux window managers don't raise/focus an unfocused, undecorated
  // window on click (the WM has no titlebar to hand click-to-focus off to),
  // so a click that lands on this window while another app has focus can
  // land without ever bringing us to front. Ask for focus explicitly.
  useEffect(() => {
    const requestFocus = () => {
      if (!document.hasFocus()) {
        getCurrentWebviewWindow().setFocus().catch(() => {});
      }
    };
    window.addEventListener("mousedown", requestFocus, true);
    return () => window.removeEventListener("mousedown", requestFocus, true);
  }, []);

  const {
    favPaths,
    defaultStartPath,
    setDefaultStartPath,
    pinnedPaths,
    togglePin,
    favTags,
    refreshFavTags,
    addFavorite,
    removeFavorite,
    moveFavorite,
  } = useFavorites(home, mobile);

  // Custom icons keyed by full path -- works for any folder, not just
  // favorites, so it can be set from Get Info too.
  const [customIcons, setCustomIcons] = useState<Record<string, string>>(() => {
    try {
      const raw = localStorage.getItem("vaultexplorer:custom-icons");
      if (raw) return JSON.parse(raw);
    } catch {
      /* ignore */
    }
    return {};
  });
  useEffect(() => {
    localStorage.setItem("vaultexplorer:custom-icons", JSON.stringify(customIcons));
  }, [customIcons]);

  // "Use as Template" stashes a copy of the file under templatesDir() and
  // keeps only this small metadata list client-side -- the stash means
  // renaming/deleting the original later can't break the template.
  const [templates, setTemplates] = useState<{ id: string; label: string; storedName: string }[]>(() => {
    try {
      const raw = localStorage.getItem("vaultexplorer:templates");
      if (raw) return JSON.parse(raw);
    } catch {
      /* ignore */
    }
    return [];
  });
  useEffect(() => {
    localStorage.setItem("vaultexplorer:templates", JSON.stringify(templates));
  }, [templates]);

  const [manageTemplatesOpen, setManageTemplatesOpen] = useState(false);
  const [gitStatusOpen, setGitStatusOpen] = useState(false);

  async function useAsTemplate(entry: Entry) {
    try {
      const dir = await api.templatesDir();
      const storedName = `${Date.now().toString(36)}-${entry.name}`;
      await api.fsCopy(joinPath(curDir, entry.name), joinPath(dir, storedName), beginProgress(`Saving Template`));
      setTemplates((prev) => [...prev, { id: storedName, label: entry.name, storedName }]);
    } catch (e) {
      setError(String(e));
    }
  }

  async function newFromTemplate(t: { label: string; storedName: string }) {
    try {
      const dir = await api.templatesDir();
      const name = uniqueName(t.label);
      await api.fsCopy(joinPath(dir, t.storedName), joinPath(curDir, name), beginProgress(`Creating "${name}"`));
      await refresh();
      selectOnly(name);
      setRenaming({ name, value: name });
    } catch (e) {
      setError(String(e));
    }
  }

  const [editingPath, setEditingPath] = useState(false);
  const [pathInput, setPathInput] = useState("");
  const [pathCopied, setPathCopied] = useState(false);

  const [history, setHistory] = useState<Loc[]>([{ kind: "fs", path: startPath(home) }]);
  const [histIdx, setHistIdx] = useState(0);
  const pendingNav = useRef<{ target: Loc; push: boolean } | null>(null);
  // Dropping files onto a *locked* vault (favorite or folder) stashes them
  // here while the unlock sheet is up, instead of navigating anywhere --
  // see dropInto() and submitUnlock().
  const pendingDropImport = useRef<{ vaultRoot: string; destRel: string; srcPaths: string[] } | null>(
    null
  );

  const contentRef = useRef<HTMLDivElement>(null);
  const dragPaths = useRef<string[]>([]);
  const dragFavIndex = useRef<number | null>(null);
  const [draggingFavIdx, setDraggingFavIdx] = useState<number | null>(null);

  // Ctrl/Cmd + scroll wheel zooms icon-view tile size, like Finder.
  useEffect(() => {
    const el = contentRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      setIconScale((s) => Math.min(2, Math.max(0.6, s - e.deltaY * 0.0015)));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // Recompute the icon-grid column count from the live content width. The
  // +0.55 bias is the "optimism": floor(x + 0.55) adds a column as soon as
  // ~45% of another tile's width is free, rather than requiring a whole
  // extra min-track (which is what left the nagging right-hand gap). The
  // tracks then stretch (1fr) to fill exactly, so there's never dead space.
  useEffect(() => {
    const el = contentRef.current;
    if (!el) return;
    const GAP = 4;
    const H_PAD = 20; // .entries.icon left+right padding
    const compute = () => {
      const avail = el.clientWidth - H_PAD;
      if (avail <= 0) return;
      const target = 104 * iconScale;
      const cols = Math.max(1, Math.floor((avail + GAP) / (target + GAP) + 0.55));
      setGridCols(cols);
    };
    compute();
    const ro = new ResizeObserver(compute);
    ro.observe(el);
    return () => ro.disconnect();
  }, [iconScale, view, showMyComputer, searchResults]);

  const inVault = loc.kind === "vault";
  const curDir = inVault ? loc.rel : loc.path; // dir key in the active space

  // "Show in folder"-type actions from other apps (Chrome's Downloads
  // panel, OBS's "Show Recordings") when VaultExplorer is the system's
  // org.freedesktop.FileManager1 (see filemanager1.rs) -- navigates here,
  // then (once that folder's listing loads) selects the specific file for
  // ShowItems calls. The path/name are handed off via a ref instead of
  // selecting immediately since `go()` navigating to a different folder is
  // async and `entries` for it hasn't loaded yet at the moment this fires.
  const pendingRevealSelectRef = useRef<{ dir: string; name: string } | null>(null);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ path: string; select: string | null }>("show-in-folder", (event) => {
      const { path, select } = event.payload;
      go({ kind: "fs", path });
      pendingRevealSelectRef.current = select ? { dir: path, name: select } : null;
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});
    return () => unlisten?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    const pending = pendingRevealSelectRef.current;
    if (!pending || curDir !== pending.dir) return;
    if (entries.some((e) => e.name === pending.name)) {
      selectOnly(pending.name);
      pendingRevealSelectRef.current = null;
    }
  }, [entries, curDir, selectOnly]);

  // Column view: a single click on a file selects it (and shows the info
  // preview column on the right) instead of opening it -- opening still
  // takes a double click, matching every other view.
  const [previewEntry, setPreviewEntry] = useState<{ dir: string; entry: Entry } | null>(null);
  const locKey = inVault ? loc.root + "::" + loc.rel : loc.path;
  useEffect(() => {
    setPreviewEntry(null);
  }, [locKey]);

  // Remember view mode + icon zoom per folder (Finder-like), so e.g.
  // Pictures can stay a zoomed-in icon grid while Workspaces stays a list.
  const [folderViewPrefs, setFolderViewPrefs] = useState<
    Record<string, { view: View; iconScale: number }>
  >(() => {
    try {
      const raw = localStorage.getItem("vaultexplorer:folder-view");
      if (raw) return JSON.parse(raw);
    } catch {
      /* ignore */
    }
    return {};
  });
  useEffect(() => {
    localStorage.setItem("vaultexplorer:folder-view", JSON.stringify(folderViewPrefs));
  }, [folderViewPrefs]);
  const folderViewPrefsRef = useRef(folderViewPrefs);
  useEffect(() => {
    folderViewPrefsRef.current = folderViewPrefs;
  }, [folderViewPrefs]);
  // Restore the saved view/zoom on navigation -- keyed only on locKey so
  // this doesn't re-fire every time the map below is written to.
  useEffect(() => {
    const pref = folderViewPrefsRef.current[locKey];
    setView(pref?.view ?? "icon");
    setIconScale(pref?.iconScale ?? 1);
  }, [locKey]);
  // Persist whenever the current folder's view/zoom actually changes --
  // keyed only on [view, iconScale] (not locKey) so a navigation that
  // restores identical values for the new folder is a no-op, and one that
  // changes them fires with the already-committed new locKey.
  useEffect(() => {
    setFolderViewPrefs((prev) => {
      const existing = prev[locKey];
      if (existing && existing.view === view && existing.iconScale === iconScale) return prev;
      return { ...prev, [locKey]: { view, iconScale } };
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view, iconScale]);

  // Declared here (rather than down by its render usage) because the
  // Shift+Arrow keydown handler below needs it, and hook dependency
  // arrays are evaluated immediately during render -- referencing a
  // `const` declared later in the same function body would throw
  // (temporal dead zone), unlike the closures inside the handler itself
  // which only run later, after the whole component has finished
  // rendering.
  const sortedEntries = useMemo(() => {
    const cmp: Record<typeof sortKey, (a: Entry, b: Entry) => number> = {
      name: (a, b) => a.name.localeCompare(b.name),
      date: (a, b) => a.mtime - b.mtime,
      created: (a, b) => (a.created ?? 0) - (b.created ?? 0),
      size: (a, b) => a.size - b.size,
      kind: (a, b) => kindLabel(a).localeCompare(kindLabel(b)),
    };
    return [...entries].sort((a, b) => {
      const aPinned = pinnedPaths.has(joinPath(curDir, a.name));
      const bPinned = pinnedPaths.has(joinPath(curDir, b.name));
      if (aPinned !== bPinned) return aPinned ? -1 : 1;
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      return sortDir * cmp[sortKey](a, b);
    });
  }, [entries, sortKey, sortDir, pinnedPaths, curDir]);

  // Enter opens every selected entry, unless a sheet/menu/inline edit is
  // active or the focused element is itself a text input.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable))
        return;
      if (
        pending ||
        renaming ||
        editingPath ||
        compressTarget ||
        zipPasswordPrompt ||
        archiveMountPrompt ||
        encryptTarget ||
        decryptPrompt ||
        infoTarget ||
        menu
      )
        return;

      // Type-ahead: with nothing else focused, just start typing a
      // letter/number jumps straight into search (Finder does the same,
      // scrolling to the first matching name as you type -- this goes
      // straight to the real search box instead, which already exists).
      if (
        !e.ctrlKey &&
        !e.metaKey &&
        !e.altKey &&
        e.key.length === 1 &&
        /[a-zA-Z0-9]/.test(e.key) &&
        !searchExpanded &&
        !searchQuery
      ) {
        e.preventDefault();
        setSearchExpanded(true);
        setSearchQuery(e.key);
        return;
      }

      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
        e.preventDefault();
        setSelected(new Set(entries.map((en) => en.name)));
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "v") {
        e.preventDefault();
        paste();
        return;
      }
      // Plain arrows move the single selection (Finder-style). Works even
      // with nothing selected yet -- the first press selects the anchor
      // (last-clicked, else first current selection, else the first entry).
      if (
        !e.shiftKey &&
        !e.ctrlKey &&
        !e.metaKey &&
        !e.altKey &&
        (e.key === "ArrowUp" || e.key === "ArrowDown" || e.key === "ArrowLeft" || e.key === "ArrowRight") &&
        (view === "icon" || view === "list" || view === "listPreview")
      ) {
        const names = sortedEntries.map((en) => en.name);
        if (names.length === 0) return;
        e.preventDefault();
        const direction =
          e.key === "ArrowUp" ? "up" : e.key === "ArrowDown" ? "down" : e.key === "ArrowLeft" ? "left" : "right";
        const from =
          (lastClicked && names.includes(lastClicked) ? lastClicked : null) ??
          [...selected].find((n) => names.includes(n)) ??
          names[0];
        // Nothing selected yet: first press just lands on the anchor.
        const target = selected.size === 0 ? from : computeArrowTarget(direction, from) ?? from;
        selectOnly(target);
        arrowAnchorRef.current = target;
        arrowFocusRef.current = target;
        return;
      }
      if (
        e.shiftKey &&
        !e.ctrlKey &&
        !e.metaKey &&
        (e.key === "ArrowUp" || e.key === "ArrowDown" || e.key === "ArrowLeft" || e.key === "ArrowRight") &&
        (view === "icon" || view === "list" || view === "listPreview")
      ) {
        const names = sortedEntries.map((en) => en.name);
        if (names.length === 0) return;
        e.preventDefault();
        const direction =
          e.key === "ArrowUp" ? "up" : e.key === "ArrowDown" ? "down" : e.key === "ArrowLeft" ? "left" : "right";
        if (!arrowAnchorRef.current || !names.includes(arrowAnchorRef.current)) {
          arrowAnchorRef.current =
            lastClicked && names.includes(lastClicked)
              ? lastClicked
              : [...selected].find((n) => names.includes(n)) ?? names[0];
        }
        const currentFocus =
          arrowFocusRef.current && names.includes(arrowFocusRef.current) ? arrowFocusRef.current : arrowAnchorRef.current;
        const target = computeArrowTarget(direction, currentFocus);
        if (!target) return;
        arrowFocusRef.current = target;
        const anchorIdx = names.indexOf(arrowAnchorRef.current);
        const targetIdx = names.indexOf(target);
        const [lo, hi] = anchorIdx < targetIdx ? [anchorIdx, targetIdx] : [targetIdx, anchorIdx];
        setSelected(new Set(names.slice(lo, hi + 1)));
        setLastClicked(target);
        return;
      }
      if (selected.size === 0) return;

      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c") {
        e.preventDefault();
        copySel();
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "x") {
        e.preventDefault();
        cutSel();
        return;
      }

      if (e.key === "Enter") {
        e.preventDefault();
        activateSelected();
        return;
      }
      if (e.key === "Delete" || e.key === "Backspace") {
        e.preventDefault();
        const names = [...selected];
        if (e.ctrlKey || e.metaKey) {
          // Permanent delete, skipping the trash -- always confirmed.
          setPending({ kind: "delete", names });
        } else if (inVault) {
          setPending({ kind: "delete", names });
        } else {
          trashSelection(names);
        }
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    selected,
    entries,
    sortedEntries,
    view,
    lastClicked,
    curDir,
    inVault,
    pending,
    renaming,
    editingPath,
    compressTarget,
    zipPasswordPrompt,
    archiveMountPrompt,
    encryptTarget,
    decryptPrompt,
    infoTarget,
    menu,
    clipboard,
    searchExpanded,
    searchQuery,
  ]);

  const listDir = useCallback(
    (dir: string, kind: Loc["kind"]) =>
      kind === "vault" ? api.listDir(dir) : api.fsList(dir, appSettings.showHiddenFiles),
    [appSettings.showHiddenFiles]
  );

  const refresh = useCallback(async () => {
    if (loc.kind === "vault") {
      try {
        await api.setActiveVault(loc.root);
      } catch {
        /* ignore */
      }
    }
    try {
      const list = await listDir(curDir, loc.kind);
      setEntries(list);
      // Honor a pending "reveal + select" (e.g. Chrome's Downloads → "Show
      // in folder") in the SAME load that would otherwise clear selection --
      // applying it here instead of in a follow-up effect means the
      // setSelected(new Set()) below can't clobber it in a race.
      const reveal = pendingRevealSelectRef.current;
      if (reveal && reveal.dir === curDir && list.some((e) => e.name === reveal.name)) {
        setSelected(new Set([reveal.name]));
        setLastClicked(reveal.name);
        pendingRevealSelectRef.current = null;
      } else {
        // Preserve selection across SAME-folder reloads -- the instant
        // "fs-changed" watch (and the 20s poll) call refresh() whenever the
        // open folder changes on disk (e.g. Drive sync writing into it), and
        // blindly clearing here wiped the user's selection out from under a
        // click ("I select an icon and it deselects itself"). Keeping only
        // names that still exist means navigating to a DIFFERENT folder
        // still clears (those names aren't in the new list), while a
        // background reload of the current folder leaves selection intact.
        const names = new Set(list.map((e) => e.name));
        setSelected((prev) => new Set([...prev].filter((n) => names.has(n))));
        setLastClicked((lc) => (lc && names.has(lc) ? lc : null));
      }
    } catch (e) {
      setError(String(e));
      setEntries([]);
    }
    if (loc.kind === "fs") {
      api.fsGetTags(curDir).then(setTags).catch(() => setTags({}));
      api
        .gitRepoRoot(curDir)
        .then((root) => {
          setGitRoot(root);
          if (root) {
            api
              .gitStatus(root)
              .then((list) => setGitStatus(Object.fromEntries(list.map((s) => [s.path, s.status]))))
              .catch(() => setGitStatus({}));
          } else {
            setGitStatus({});
          }
        })
        .catch(() => {
          setGitRoot(null);
          setGitStatus({});
        });
    } else {
      setTags({});
      setGitRoot(null);
      setGitStatus({});
    }
    refreshFrozen();
    refreshSyncStatus();
  }, [curDir, loc.kind, loc.kind === "vault" ? loc.root : null, listDir, refreshFrozen, refreshSyncStatus]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Periodic background refresh while a real-fs folder stays open, so
  // changes from an external source (git auto-sync's own ~25s poll loop,
  // Drive sync, another program) actually show up instead of requiring a
  // manual refresh or a navigate-away-and-back. Skipped for a tick
  // whenever something's selected or mid-rename, via refs rather than
  // effect deps, so the interval itself doesn't get torn down and reset
  // by every selection change (which would mean it never fires at all
  // for someone actively clicking around).
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const renamingRef = useRef(renaming);
  renamingRef.current = renaming;
  useEffect(() => {
    if (loc.kind !== "fs") return;
    const interval = setInterval(() => {
      if (selectedRef.current.size > 0 || renamingRef.current) return;
      refresh();
    }, 20000);
    return () => clearInterval(interval);
  }, [loc.kind, curDir, refresh]);

  // Instant version of the same idea: watch the real-fs folder currently
  // open and refresh the moment something changes it from outside the app
  // (a terminal `rm`, a browser download landing in it, git/Drive sync
  // writing to it) instead of waiting for the 20s poll above -- that poll
  // stays as a safety net for setups where the underlying watch mechanism
  // doesn't work (some network mounts). Only one folder is ever watched
  // (whatever's currently browsed); vault browsing doesn't need this since
  // nothing writes into a vault's encrypted storage except this app.
  const curDirRef = useRef(curDir);
  curDirRef.current = curDir;
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen<string>("fs-changed", (event) => {
      if (event.payload === curDirRef.current) refresh();
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refresh]);
  useEffect(() => {
    api.fsWatchSet(loc.kind === "fs" ? curDir : null).catch(() => {});
  }, [loc.kind, curDir]);

  // ---- navigation (handles crossing the fs/vault boundary) ----
  function commitLoc(target: Loc, push: boolean) {
    if (push) {
      setHistory((h) => {
        const trimmed = h.slice(0, histIdx + 1);
        trimmed.push(target);
        setHistIdx(trimmed.length - 1);
        return trimmed;
      });
    }
    setLoc(target);
    setSearchResults(null);
    setSearchQuery("");
    // Any mounted archive whose scratch directory isn't (an ancestor of)
    // where we just navigated to has been left behind -- repack it back
    // over its original file now rather than leaving it dangling until
    // quit.
    api
      .archiveMountsLeftBehind(target.kind === "fs" ? target.path : "")
      .then((stale) => Promise.all(stale.map((mp) => api.archiveUnmount(mp))))
      .catch(() => {});
  }

  function cascadeRemoveRoots(prev: Set<string>, root: string): Set<string> {
    const prefix = root + "/";
    const next = new Set<string>();
    for (const r of prev) {
      if (r !== root && !r.startsWith(prefix)) next.add(r);
    }
    return next;
  }

  async function go(target: Loc, push = true) {
    setShowMyComputer(false);
    cancelPendingRenameClick();
    if (target.kind === "vault") {
      if (unlockedRoots.has(target.root)) {
        commitLoc(target, push);
      } else {
        // need to unlock first; stash the intended destination
        pendingNav.current = { target, push };
        setSheetError("");
        setVaultParents((prev) => ({ ...prev, [target.root]: loc }));
        setPending({ kind: "unlock", path: target.root, name: baseName(target.root) });
      }
      return;
    }
    // fs target: leaving an unlocked vault locks it (and any vault nested
    // inside it), unless "Keep Unlocked" was checked -- then it stays open
    // until explicitly locked from the sidebar's right-click menu.
    if (loc.kind === "vault" && !keepUnlockedRoots.has(loc.root) && !vaultSettings[loc.root]?.autoUnlock) {
      try {
        await api.lockVault(loc.root);
      } catch {
        /* ignore */
      }
      setUnlockedRoots((prev) => cascadeRemoveRoots(prev, loc.root));
      setKeepUnlockedRoots((prev) => cascadeRemoveRoots(prev, loc.root));
    }
    commitLoc(target, push);
  }

  function goBack() {
    if (histIdx === 0) return;
    go(history[histIdx - 1], false).then(() => setHistIdx(histIdx - 1));
  }
  function goForward() {
    if (histIdx >= history.length - 1) return;
    go(history[histIdx + 1], false).then(() => setHistIdx(histIdx + 1));
  }
  function goUp() {
    if (loc.kind === "vault") {
      if (loc.rel === "") {
        // Already at the vault's own root -- "up" means leaving the
        // vault, back to wherever it was navigated in from.
        go(vaultParents[loc.root] ?? { kind: "fs", path: parentPath(loc.root) });
      } else {
        go({ kind: "vault", root: loc.root, rel: parentPath(loc.rel) });
      }
      return;
    }
    if (loc.path === "/") return;
    go({ kind: "fs", path: parentPath(loc.path) });
  }
  const canGoUp = loc.kind === "vault" || loc.path !== "/";
  // Best-effort: flush every still-mounted archive back to its file when
  // the window closes, so quitting mid-browse doesn't usually strand
  // edits in a scratch directory that's about to be orphaned. Deliberately
  // NOT gating the close on this (no preventDefault, nothing awaited) --
  // an earlier version blocked close on it, which is a far worse bug than
  // an archive edit occasionally not making it back to disk on quit.
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    const unlisten = win.onCloseRequested(() => {
      api
        .archiveAllMounts()
        .then((mounts) => Promise.all(mounts.map((mp) => api.archiveUnmount(mp))))
        .catch(() => {
          /* best-effort */
        });
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // Reflect the current folder in the window title (taskbar/dock/alt-tab
  // label -- there's no OS titlebar to show it in, decorations are off).
  useEffect(() => {
    const name =
      loc.kind === "vault"
        ? baseName(loc.root)
        : loc.path === "/"
          ? "VaultExplorer"
          : baseName(loc.path) || loc.path;
    getCurrentWebviewWindow()
      .setTitle(name)
      .catch(() => {});
  }, [loc]);

  // ---- external (OS) drops ----
  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().onDragDropEvent((event) => {
      if (event.payload.type !== "drop") return;
      // A native drag *we* started (see beginDrag/api.startFileDrag) lands
      // back here too when it's dropped inside our own window, since it's
      // the same OS-level mechanism regardless of source -- `dragPaths`
      // being non-empty is how we tell "our own drag came back" apart
      // from "something external (the OS file manager, etc.) was
      // dropped in". Route it to whatever specific tile/favorite is
      // actually under the cursor, the same way the plain HTML5 path
      // (dropInto, still used for reordering/etc.) already does, instead
      // of only ever landing in the current folder.
      if (dragPaths.current.length > 0) {
        const scale = window.devicePixelRatio || 1;
        const x = event.payload.position.x / scale;
        const y = event.payload.position.y / scale;
        const el = document.elementFromPoint(x, y) as HTMLElement | null;
        const favEl = el?.closest("[data-path]") as HTMLElement | null;
        const entryEl = el?.closest(".entry[data-name]") as HTMLElement | null;
        let destDir = curDir;
        if (favEl?.dataset.path) {
          destDir = favEl.dataset.path;
        } else if (entryEl?.dataset.name) {
          const target = entries.find((en) => en.name === entryEl.dataset.name);
          if (target?.is_dir) destDir = joinPath(curDir, target.name);
        }
        dropInto(destDir);
        return;
      }
      for (const srcPath of event.payload.paths) {
        const name = srcPath.split(/[/\\]/).pop() ?? srcPath;
        const done = () => refresh();
        if (loc.kind === "vault") {
          api.importFile(srcPath, joinPath(loc.rel, name)).then(done).catch((e) => setError(String(e)));
        } else {
          api
            .fsCopy(srcPath, joinPath(loc.path, name), beginProgress(`Copying "${name}"`))
            .then(done)
            .catch((e) => setError(String(e)));
        }
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [loc, refresh, entries, curDir]);

  // ---- selection ----
  function selectRange(to: string) {
    selectRangeByNames(to, sortedEntries.map((e) => e.name));
  }
  // A second, separate click on an entry that's already the only one
  // selected starts a rename -- same convention Finder/Nautilus use. The
  // trick is telling that apart from the first half of a double-click
  // (meant to open, not rename): schedule the rename after a short delay
  // instead of firing immediately, and let the onOpen handler (wired to
  // onDoubleClick) cancel it if a real double-click follows.
  const renameClickTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  function cancelPendingRenameClick() {
    if (renameClickTimer.current) {
      clearTimeout(renameClickTimer.current);
      renameClickTimer.current = null;
    }
  }
  function onEntryClick(e: React.MouseEvent, entry: Entry) {
    e.stopPropagation();
    arrowAnchorRef.current = null;
    arrowFocusRef.current = null;
    if (e.metaKey || e.ctrlKey) {
      cancelPendingRenameClick();
      toggle(entry.name);
      return;
    }
    if (e.shiftKey) {
      cancelPendingRenameClick();
      selectRange(entry.name);
      return;
    }
    const wasSoleSelected = selected.size === 1 && selected.has(entry.name);
    cancelPendingRenameClick();
    selectOnly(entry.name);
    if (wasSoleSelected) {
      renameClickTimer.current = setTimeout(() => {
        renameClickTimer.current = null;
        setRenaming({ name: entry.name, value: entry.name });
      }, 450);
    }
  }

  // Shift+Arrow's notion of "what's above/below/left/right" depends on
  // the view: list/listPreview is a single column (only up/down make
  // sense), icon view is a real 2D grid. Rather than reverse-engineer the
  // grid's column count from CSS (it's a responsive auto-fill track,
  // width-dependent), this groups the actual rendered tiles by their
  // offsetTop -- tiles sharing a row have the same offsetTop, regardless
  // of how many columns actually fit right now.
  function computeArrowTarget(direction: "up" | "down" | "left" | "right", fromName: string): string | null {
    const names = sortedEntries.map((en) => en.name);
    const idx = names.indexOf(fromName);
    if (idx === -1) return names[0] ?? null;

    if (view !== "icon") {
      if (direction === "down") return names[Math.min(idx + 1, names.length - 1)];
      if (direction === "up") return names[Math.max(idx - 1, 0)];
      return fromName;
    }

    const container = contentRef.current;
    if (!container) return fromName;
    const tiles = Array.from(container.querySelectorAll<HTMLElement>(".entry.icon"));
    const rows: string[][] = [];
    let lastTop = -1;
    for (const tile of tiles) {
      const name = tile.dataset.name;
      if (!name) continue;
      const top = tile.offsetTop;
      if (rows.length === 0 || Math.abs(top - lastTop) > 4) {
        rows.push([name]);
        lastTop = top;
      } else {
        rows[rows.length - 1].push(name);
      }
    }
    let r = -1;
    let c = -1;
    for (let i = 0; i < rows.length; i++) {
      const j = rows[i].indexOf(fromName);
      if (j !== -1) {
        r = i;
        c = j;
        break;
      }
    }
    if (r === -1) return names[0] ?? null;
    if (direction === "left") return rows[r][c - 1] ?? fromName;
    if (direction === "right") return rows[r][c + 1] ?? fromName;
    if (direction === "up") return rows[r - 1]?.[Math.min(c, rows[r - 1].length - 1)] ?? fromName;
    return rows[r + 1]?.[Math.min(c, rows[r + 1].length - 1)] ?? fromName;
  }

  // ---- marquee ----
  function onContentMouseDown(e: React.MouseEvent) {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest(".entry")) return;
    // The inline preview/editor pane (listPreview view's split) lives
    // inside this same `.content` container -- without this, starting a
    // text selection drag inside it (e.g. a text file's editor textarea,
    // or selecting a metadata value in the info panel) bubbles up here
    // uncontained and kicks off a marquee file-selection rectangle at the
    // same time, clearing the real file selection mid-edit.
    if ((e.target as HTMLElement).closest(".preview-pane, .preview-column")) return;
    if (view === "column") return;
    if (!(e.metaKey || e.ctrlKey || e.shiftKey)) setSelected(new Set());
    setMarquee({ x0: e.clientX, y0: e.clientY, x1: e.clientX, y1: e.clientY });
  }
  useEffect(() => {
    if (!marquee) return;
    // Snapshot both the drag origin and every entry's rect once, at drag
    // start -- this used to close over the `marquee` state and depend on
    // it, so every mousemove-driven `setMarquee` call re-ran the whole
    // effect (tearing down and re-adding two window listeners per pixel
    // of mouse movement) *and* recomputed getBoundingClientRect for every
    // entry on every one of those events. Entries don't move mid-drag, so
    // both only need to happen once per drag, not once per mousemove.
    const origin = { x: marquee.x0, y: marquee.y0 };
    const rects = Array.from(contentRef.current?.querySelectorAll<HTMLElement>(".entry") ?? []).map((el) => ({
      name: el.dataset.name,
      rect: el.getBoundingClientRect(),
    }));
    const move = (e: MouseEvent) => {
      setMarquee((m) => (m ? { ...m, x1: e.clientX, y1: e.clientY } : m));
      const left = Math.min(origin.x, e.clientX);
      const right = Math.max(origin.x, e.clientX);
      const top = Math.min(origin.y, e.clientY);
      const bottom = Math.max(origin.y, e.clientY);
      const hit = new Set<string>();
      for (const { name, rect: r } of rects) {
        if (name && r.left < right && r.right > left && r.top < bottom && r.bottom > top) hit.add(name);
      }
      setSelected(hit);
    };
    const up = () => setMarquee(null);
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [marquee !== null]);

  const timeoutSecs = (): number | null =>
    appSettings.sensitiveTimeout === "never" ? null : appSettings.sensitiveTimeout;

  // Schedule the sensitive session to auto-relock when the window expires:
  // re-lock in the backend and drop any open preview so a sensitive file
  // stops being visible the moment the timer lapses ("walked away" safety).
  function scheduleSensitiveRelock() {
    if (sensitiveTimerRef.current) {
      clearTimeout(sensitiveTimerRef.current);
      sensitiveTimerRef.current = null;
    }
    const secs = timeoutSecs();
    if (secs === null) return; // "never" -- no auto-relock
    sensitiveTimerRef.current = setTimeout(() => {
      sensitiveTimerRef.current = null;
      api.vaultLockSensitive().catch(() => {});
      setPreviewEntry((p) => {
        if (!p) return p;
        // Only close the preview if what's showing is itself sensitive.
        return isSensitivePath(joinPath(p.dir, p.entry.name)) ? null : p;
      });
      setInfoMsg("Sesión de archivos sensibles cerrada");
    }, secs * 1000);
  }

  // Gate opening/previewing a sensitive file behind a re-auth of the vault
  // password. Runs `proceed` immediately when: not in a vault, the file
  // isn't sensitive, or the sensitive session is already open. Otherwise it
  // prompts for the vault password first.
  async function withSensitive(fullPath: string, proceed: () => void) {
    if (!inVault) {
      proceed();
      return;
    }
    let sensitive = false;
    try {
      sensitive = await api.vaultIsSensitive(fullPath);
    } catch {
      sensitive = false;
    }
    if (!sensitive) {
      proceed();
      return;
    }
    let unlocked = false;
    try {
      unlocked = await api.vaultSensitiveUnlocked();
    } catch {
      unlocked = false;
    }
    if (unlocked) {
      proceed();
      return;
    }
    setSensitivePrompt({ path: fullPath, proceed, error: "" });
  }

  async function submitSensitive(password: string) {
    if (!sensitivePrompt) return;
    try {
      await api.vaultUnlockSensitive(password, timeoutSecs());
      const proceed = sensitivePrompt.proceed;
      setSensitivePrompt(null);
      scheduleSensitiveRelock();
      proceed();
    } catch {
      setSensitivePrompt((s) => (s ? { ...s, error: "Contraseña incorrecta" } : s));
    }
  }

  const relKey = (rel: string) => rel.replace(/^\/+/, "").replace(/\/+$/, "");
  // Sensitive directly on this exact path.
  const isMarkedSensitive = (rel: string) => sensitiveSet.has(relKey(rel));
  // Sensitive via a marked ANCESTOR folder (inherited -- can't be unmarked
  // individually while the folder governs it).
  function hasSensitiveAncestor(rel: string): boolean {
    let p = relKey(rel);
    const i0 = p.lastIndexOf("/");
    if (i0 < 0) return false;
    p = p.slice(0, i0);
    while (true) {
      if (sensitiveSet.has(p)) return true;
      const i = p.lastIndexOf("/");
      if (i < 0) return false;
      p = p.slice(0, i);
    }
  }
  const isSensitivePath = (rel: string) => isMarkedSensitive(rel) || hasSensitiveAncestor(rel);

  async function toggleSensitive(rel: string, sensitive: boolean) {
    try {
      await api.vaultSetSensitive(rel, sensitive);
      const list = await api.vaultListSensitive();
      setSensitiveSet(new Set(list));
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    if (!inVault) {
      setSensitiveSet(new Set());
      return;
    }
    api
      .vaultListSensitive()
      .then((l) => setSensitiveSet(new Set(l)))
      .catch(() => setSensitiveSet(new Set()));
  }, [inVault, curDir, loc.kind === "vault" ? loc.root : null]);

  // ---- open / activate an entry ----
  async function activate(dir: string, entry: Entry) {
    if (loc.kind === "vault") {
      if (entry.is_vault) {
        // A vault nested inside this one: resolve its real on-disk path
        // through the parent's FUSE mount (using the currently-active
        // vault, i.e. this one). Routed through `go()` -- not a bare
        // `setPending({kind:"unlock"})` -- so a vault that's already
        // unlocked (esp. "keep unlocked when I navigate away") opens
        // straight back up instead of re-prompting every time (see
        // task #106: this unconditional-prompt path, not the leave-vault
        // lock logic in `go()`, was the actual bug).
        try {
          const resolvedRoot = await api.openPath(joinPath(dir, entry.name));
          go({ kind: "vault", root: resolvedRoot, rel: "" });
        } catch (e) {
          setError(String(e));
        }
        return;
      }
      if (entry.is_dir) return go({ kind: "vault", root: loc.root, rel: joinPath(dir, entry.name) });
      if (entry.name.toLowerCase().endsWith(ENCRYPTED_FILE_EXT)) {
        setDecryptPrompt({ entry, error: "", mode: "open" });
        return;
      }
      if (ARCHIVE_EXT_RE.test(entry.name)) {
        // Browse a vault-internal archive like a folder, same as on fs.
        const full = joinPath(dir, entry.name);
        withSensitive(full, () => mountArchive(dir, entry));
        return;
      }
      {
        const full = joinPath(dir, entry.name);
        withSensitive(full, async () => {
          try {
            const abs = await api.openPath(full);
            await osOpen(abs);
          } catch (e) {
            setError(String(e));
          }
        });
      }
      return;
    }
    // fs
    const full = joinPath(dir, entry.name);
    if (entry.is_vault) {
      // Same fix as the nested-vault branch above: go() checks
      // unlockedRoots first, so double-clicking a vault that's already
      // unlocked re-enters it directly instead of re-prompting.
      go({ kind: "vault", root: full, rel: "" });
      return;
    }
    if (entry.is_dir) return go({ kind: "fs", path: full });
    if (entry.name.toLowerCase().endsWith(ENCRYPTED_FILE_EXT)) {
      setDecryptPrompt({ entry, error: "", mode: "open" });
      return;
    }
    if (ARCHIVE_EXT_RE.test(entry.name)) {
      return mountArchive(dir, entry);
    }
    try {
      await osOpen(full);
    } catch (e) {
      setError(String(e));
    }
  }

  // Enter opens every selected entry (a single selected folder still
  // navigates into it, matching the existing single-item behavior).
  async function activateSelected() {
    const names = [...selected];
    if (names.length === 0) return;
    if (names.length === 1) {
      const entry = entries.find((en) => en.name === names[0]);
      if (entry) await activate(curDir, entry);
      return;
    }
    for (const name of names) {
      const entry = entries.find((en) => en.name === name);
      if (entry && !entry.is_dir) await activate(curDir, entry);
    }
  }

  async function runSearch(q: string) {
    if (q.trim() === "") {
      setSearchResults(null);
      return;
    }
    try {
      setSearchResults(inVault ? await api.search(q) : await api.fsSearch(loc.path, q));
    } catch (e) {
      setError(String(e));
    }
  }

  // Live search-as-you-type, debounced so we don't fire a query per
  // keystroke.
  useEffect(() => {
    if (searchQuery.trim() === "") return;
    const t = setTimeout(() => runSearch(searchQuery), 250);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchQuery, curDir, inVault]);

  // ---- clipboard ----
  function selectedPaths(fallback?: string): string[] {
    const names = selected.size ? [...selected] : fallback ? [fallback] : [];
    return names.map((n) => joinPath(curDir, n));
  }
  function copySel(entry?: Entry) {
    const names = selected.size ? [...selected] : entry ? [entry.name] : [];
    const paths = names.map((n) => joinPath(curDir, n));
    if (paths.length) setClipboard({ paths, mode: "copy", kind: loc.kind });
    // Also put real image bytes on the *system* clipboard so it's
    // pasteable outside the app (a browser, another native app) --
    // best-effort only, so a decode failure here (e.g. an unsupported
    // format) never disturbs the in-app cut/paste clipboard above.
    if (names.length === 1) {
      const en = entry?.name === names[0] ? entry : entries.find((e) => e.name === names[0]);
      if (en && !en.is_dir && /\.(png|jpe?g|bmp|webp|tiff?|gif)$/i.test(en.name)) {
        const call = inVault
          ? api.vaultCopyImageToClipboard(paths[0])
          : api.fsCopyImageToClipboard(paths[0]);
        call.catch(() => {});
      }
    }
  }
  function cutSel(entry?: Entry) {
    const paths = selectedPaths(entry?.name);
    if (paths.length) setClipboard({ paths, mode: "cut", kind: loc.kind });
  }
  async function paste() {
    if (!clipboard) return;
    // Cut/copy across the vault boundary -- clipboard.kind is whichever
    // space the files were cut/copied *from*, loc.kind is where they're
    // now being pasted. Files only (not folders): importFile/exportFile
    // are both single-file encrypt/decrypt, not recursive.
    if (clipboard.kind !== loc.kind) {
      try {
        for (const src of clipboard.paths) {
          const dest = joinPath(curDir, baseName(src));
          if (clipboard.kind === "fs" && inVault) {
            await api.importFile(src, dest);
            if (clipboard.mode === "cut") await api.fsDelete(src);
          } else if (clipboard.kind === "vault" && !inVault) {
            await api.exportFile(src, dest);
            if (clipboard.mode === "cut") await api.deleteFile(src);
          }
        }
        setClipboard(null);
        refresh();
      } catch (e) {
        setError(String(e));
      }
      return;
    }
    try {
      for (const src of clipboard.paths) {
        const name = baseName(src);
        let dest = joinPath(curDir, name);
        if (src === dest) {
          // Pasting a copy back into the very folder it was copied from --
          // there's nothing to skip to, so make a Finder-style numbered
          // duplicate instead of silently doing nothing. A cut, on the
          // other hand, really has nothing left to do here (it's already
          // in place), so that case still no-ops.
          if (clipboard.mode !== "copy") continue;
          dest = joinPath(curDir, uniqueName(name));
        }
        if (clipboard.mode === "copy") {
          inVault
            ? await api.copyEntry(src, dest)
            : await api.fsCopy(src, dest, beginProgress(`Copying "${name}"`));
        } else {
          inVault ? await api.moveEntry(src, dest) : await api.fsRename(src, dest);
        }
      }
      setClipboard(null);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }
  function uniqueName(base: string): string {
    const used = new Set(entries.map((e) => e.name));
    if (!used.has(base)) return base;
    const dot = base.lastIndexOf(".");
    const stem = dot > 0 ? base.slice(0, dot) : base;
    const ext = dot > 0 ? base.slice(dot) : "";
    let i = 2;
    while (used.has(`${stem} ${i}${ext}`)) i++;
    return `${stem} ${i}${ext}`;
  }

  // ---- git ----
  function relToGitRoot(name: string): string | null {
    if (!gitRoot) return null;
    const full = joinPath(curDir, name);
    if (full === gitRoot) return "";
    if (full.startsWith(gitRoot + "/")) return full.slice(gitRoot.length + 1);
    return null;
  }
  async function gitPullNow() {
    if (!gitRoot) return;
    try {
      await api.gitPull(gitRoot);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }
  async function gitPushNow() {
    if (!gitRoot) return;
    try {
      await api.gitPush(gitRoot);
    } catch (e) {
      setError(String(e));
    }
  }
  async function gitStageEntry(entry: Entry) {
    const rel = relToGitRoot(entry.name);
    if (!gitRoot || rel === null) return;
    try {
      await api.gitStage(gitRoot, rel);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }
  async function gitUnstageEntry(entry: Entry) {
    const rel = relToGitRoot(entry.name);
    if (!gitRoot || rel === null) return;
    try {
      await api.gitUnstage(gitRoot, rel);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }
  async function gitDiscardEntry(entry: Entry) {
    const rel = relToGitRoot(entry.name);
    if (!gitRoot || rel === null) return;
    try {
      await api.gitDiscard(gitRoot, rel);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function clearMetadataSelection(names: string[]) {
    if (names.length === 0) return;
    const paths = names.map((n) => joinPath(curDir, n));
    try {
      const results = inVault
        ? await api.vaultClearMetadata(paths, beginProgress("Clearing Metadata"))
        : await api.fsClearMetadata(paths, beginProgress("Clearing Metadata"));
      const skipped = results.filter((r) => !r.cleared);
      if (skipped.length > 0) {
        setError(
          `Cleared metadata for ${results.length - skipped.length} of ${results.length} — ` +
            `${skipped.length} skipped (${skipped[0].reason ?? "unsupported"})`
        );
      }
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function compressSelection(names: string[]) {
    if (names.length === 0) return;
    const destName = uniqueName(names.length === 1 ? `${names[0]}.zip` : "Archive.zip");
    try {
      inVault
        ? await api.compressEntries(curDir, names, destName)
        : await api.fsCompress(curDir, names, destName, beginProgress(`Compressing "${destName}"`));
      await refresh();
      selectOnly(destName);
    } catch (e) {
      setError(String(e));
    }
  }

  // Real filesystem only -- browsing a vault-sourced archive this way
  // would mean extracting decrypted bytes to a real scratch directory on
  // disk, the same constraint that keeps vault compression zip-only.
  async function mountArchive(dir: string, entry: Entry, password: string | null = null) {
    const rel = joinPath(dir, entry.name);
    try {
      // Inside a vault the archive only exists decrypted through the FUSE
      // mount -- resolve to that real path so it can be opened/browsed like
      // any other archive. Repack-on-leave writes back through FUSE, so it
      // re-encrypts into the vault.
      const full = inVault ? await api.openPath(rel) : rel;
      const mountpoint = await api.archiveMount(full, password);
      setArchiveMountPrompt(null);
      go({ kind: "fs", path: mountpoint });
    } catch (e) {
      const msg = String(e);
      if (/password/i.test(msg)) {
        setArchiveMountPrompt({ dir, entry, error: password !== null ? "Incorrect password" : "" });
      } else {
        setError(msg);
      }
    }
  }

  async function decompressEntry(entry: Entry, password: string | null = null) {
    const zipPath = joinPath(curDir, entry.name);
    const destPath = joinPath(curDir, uniqueName(entry.name.replace(/\.zip$/i, "")));
    try {
      inVault
        ? await api.decompressEntry(zipPath, destPath, password)
        : await api.fsDecompress(zipPath, destPath, beginProgress(`Extracting "${entry.name}"`), password);
      setZipPasswordPrompt(null);
      refresh();
    } catch (e) {
      const msg = String(e);
      if (/password/i.test(msg)) {
        setZipPasswordPrompt({ entry, error: password !== null ? "Incorrect password" : "" });
      } else {
        setError(msg);
      }
    }
  }

  async function compressWithOptions(
    names: string[],
    opts: { destName: string; format: "zip" | "targz"; password: string | null; level: number; readme: string | null }
  ) {
    const destName = uniqueName(opts.destName);
    try {
      if (inVault) {
        await api.compressEntries(curDir, names, destName, opts.password, opts.level, opts.readme);
      } else if (opts.format === "targz") {
        await api.fsCompressTargz(curDir, names, destName, beginProgress(`Compressing "${destName}"`));
      } else {
        await api.fsCompress(
          curDir,
          names,
          destName,
          beginProgress(`Compressing "${destName}"`),
          opts.password,
          opts.level,
          opts.readme
        );
      }
      setCompressTarget(null);
      await refresh();
      selectOnly(destName);
    } catch (e) {
      setError(String(e));
    }
  }

  async function encryptFile(entry: Entry, password: string) {
    const path = joinPath(curDir, entry.name);
    try {
      const newPath = inVault
        ? await api.encryptFileInVault(path, password)
        : await api.fsEncryptFile(path, password);
      setEncryptTarget(null);
      await refresh();
      selectOnly(baseName(newPath));
    } catch (e) {
      setError(String(e));
    }
  }

  // Encrypting a folder (only offered outside a vault today -- see the
  // nested-vaults item for the inside-a-vault case) turns it into a vault
  // in place, reusing the same primitive as "New Vault...".
  async function encryptFolder(entry: Entry, password: string) {
    const relOrAbs = joinPath(curDir, entry.name);
    try {
      // create_vault needs a real fs path; inside a vault that means
      // resolving the target through this vault's own FUSE mount first --
      // the vault's ciphertext transparently re-encrypts whatever the
      // nested vault writes underneath it.
      const realPath = inVault ? await api.openPath(relOrAbs) : relOrAbs;
      // Convert-in-place: encrypt the folder's existing contents into the new
      // vault (not just drop an empty .vault.meta), so a populated folder
      // actually becomes an encrypted vault of those files.
      await api.convertFolderToVault(realPath, password);
      setEncryptTarget(null);
      await refresh();
      selectOnly(entry.name);
    } catch (e) {
      setError(String(e));
    }
  }

  async function decryptAndOpen(entry: Entry, password: string) {
    const path = joinPath(curDir, entry.name);
    try {
      const tempPath = inVault
        ? await api.decryptFileInVault(path, password)
        : await api.fsDecryptFile(path, password);
      setDecryptPrompt(null);
      await osOpen(tempPath);
    } catch {
      setDecryptPrompt({ entry, error: "Incorrect password", mode: "open" });
    }
  }

  // "Decrypt" (as opposed to "Decrypt and Open"): undoes Encrypt in place,
  // restoring the plaintext under its original name and removing the .vlt.
  async function decryptInPlace(entry: Entry, password: string) {
    const path = joinPath(curDir, entry.name);
    const restoredName = uniqueName(entry.name.replace(new RegExp(`\\${ENCRYPTED_FILE_EXT}$`, "i"), ""));
    try {
      if (inVault) {
        const tempPath = await api.decryptFileInVault(path, password);
        await api.importFile(tempPath, joinPath(curDir, restoredName));
        await api.deleteFile(path);
      } else {
        const tempPath = await api.fsDecryptFile(path, password);
        await api.fsCopy(tempPath, joinPath(curDir, restoredName), beginProgress(`Copying "${restoredName}"`));
        await api.fsDelete(path);
      }
      setDecryptPrompt(null);
      await refresh();
      selectOnly(restoredName);
    } catch {
      setDecryptPrompt({ entry, error: "Incorrect password", mode: "inplace" });
    }
  }

  async function setTagFor(name: string, color: string | null) {
    try {
      await api.fsSetTag(curDir, name, color);
      setTags((t) => {
        const next = { ...t };
        if (color) next[name] = color;
        else delete next[name];
        return next;
      });
      refreshFavTags();
    } catch (e) {
      setError(String(e));
    }
  }

  // Same as setTagFor, but for an arbitrary path (used by the Favorites
  // sidebar, which isn't necessarily inside the currently browsed folder).
  async function setTagForPath(fullPath: string, color: string | null) {
    const dir = parentPath(fullPath);
    const name = baseName(fullPath);
    try {
      await api.fsSetTag(dir, name, color);
      if (dir === curDir) {
        setTags((t) => {
          const next = { ...t };
          if (color) next[name] = color;
          else delete next[name];
          return next;
        });
      }
      refreshFavTags();
    } catch (e) {
      setError(String(e));
    }
  }

  async function createShortcut(entry: Entry) {
    const target = joinPath(curDir, entry.name);
    const name = uniqueName(`${entry.name} shortcut`);
    try {
      await api.fsCreateShortcut(target, joinPath(curDir, name));
      await refresh();
      selectOnly(name);
      setRenaming({ name, value: name });
    } catch (e) {
      setError(String(e));
    }
  }

  async function duplicate(entry: Entry) {
    const src = joinPath(curDir, entry.name);
    const dot = entry.name.lastIndexOf(".");
    const copyName =
      dot > 0 && !entry.is_dir
        ? `${entry.name.slice(0, dot)} copy${entry.name.slice(dot)}`
        : `${entry.name} copy`;
    try {
      inVault
        ? await api.copyEntry(src, joinPath(curDir, copyName))
        : await api.fsCopy(src, joinPath(curDir, copyName), beginProgress(`Copying "${entry.name}"`));
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  // ---- internal drag-drop (move) ----
  // Real fs files/folders also start a *native* OS-level drag alongside
  // the usual HTML5 one -- HTML5's own `dataTransfer` never carries real
  // file bytes another process could read, so without this, dropping
  // onto an external app (a browser tab, another native app) silently
  // does nothing no matter what's in `dataTransfer`. Vault entries are
  // deliberately excluded: their plaintext only ever exists decrypted in
  // memory, and there's no decrypted-on-disk file path to hand another
  // process anyway.
  function beginDrag(e: React.DragEvent, entry: Entry) {
    const names = selected.has(entry.name) && selected.size ? [...selected] : [entry.name];
    if (!selected.has(entry.name)) selectOnly(entry.name);
    dragPaths.current = names.map((n) => joinPath(curDir, n));
    if (!inVault) {
      // Cancel WebKitGTK's own HTML5 drag entirely and start a real OS-
      // level one instead (see api.startFileDrag) -- letting *both* run
      // for the same gesture means they compete for the same X11 pointer
      // grab, and WebKitGTK's own (which never carries real file bytes
      // another process could read, no matter what's in `dataTransfer`)
      // was winning that race, silently. Dropping this same drag back
      // onto our own window still works: the `onDragDropEvent` handler
      // below treats a non-empty `dragPaths` as "this is our own drag
      // coming back" and routes it the same way `onDrop` used to.
      e.preventDefault();
      api.startFileDrag(dragPaths.current, buildDragImage(entry, names.length)).catch(() => {});
      return;
    }
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", dragPaths.current.join("\n"));
  }
  async function dropInto(destDir: string) {
    const srcs = dragPaths.current;
    dragPaths.current = [];
    setDropTarget(null);
    if (!srcs.length) return;

    // Dropping real fs files onto a vault (favorite or folder) needs to
    // *encrypt* them in, not rename/move them -- a vault's contents aren't
    // reachable as plain paths the way a normal folder's are. Figure out
    // which of the three situations this drop is before falling through
    // to the plain intra-space move below.
    if (!inVault) {
      const unlockedRoot = [...unlockedRoots].find((r) => destDir === r || destDir.startsWith(r + "/"));
      if (unlockedRoot) {
        const destRel = destDir === unlockedRoot ? "" : destDir.slice(unlockedRoot.length + 1);
        try {
          for (const src of srcs) {
            await api.importFile(src, joinPath(destRel, baseName(src)));
          }
          refresh();
        } catch (e) {
          setError(String(e));
        }
        return;
      }
      try {
        if (await api.vaultExists(destDir)) {
          // Locked -- stash the drop and ask for the password first;
          // submitUnlock() runs the actual import once it succeeds.
          pendingDropImport.current = { vaultRoot: destDir, destRel: "", srcPaths: srcs };
          setSheetError("");
          setVaultParents((prev) => ({ ...prev, [destDir]: loc }));
          setPending({ kind: "unlock", path: destDir, name: baseName(destDir) });
          return;
        }
      } catch {
        /* not a vault path -- fall through to a plain move */
      }
    }

    try {
      for (const src of srcs) {
        if (parentPath(src) === destDir) continue;
        if (destDir === src || destDir.startsWith(src + "/")) continue;
        const dest = joinPath(destDir, baseName(src));
        inVault ? await api.moveEntry(src, dest) : await api.fsRename(src, dest);
      }
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  // ---- inline create (new folder / new file) ----
  function nextUntitledName(base: string, ext: string): string {
    const used = new Set(entries.map((e) => e.name));
    if (!used.has(base + ext)) return base + ext;
    let i = 2;
    while (used.has(`${base} ${i}${ext}`)) i++;
    return `${base} ${i}${ext}`;
  }
  async function createNewFolder() {
    const base = formatNameTemplate(appSettings.newFolderNameTemplate || "untitled folder");
    const name = nextUntitledName(base, "");
    try {
      inVault ? await api.makeDir(joinPath(curDir, name)) : await api.fsMkdir(joinPath(curDir, name));
      await refresh();
      selectOnly(name);
      setRenaming({ name, value: name });
    } catch (e) {
      setError(String(e));
    }
  }
  async function createNewFile() {
    const base = formatNameTemplate(appSettings.newFileNameTemplate || "untitled document");
    // The list-with-preview view exists specifically to write/read
    // markdown in place, so a new file made from there defaults to .md
    // instead of the generic .txt.
    const name = nextUntitledName(base, view === "listPreview" ? ".md" : ".txt");
    try {
      inVault ? await api.newFile(joinPath(curDir, name)) : await api.fsNewFile(joinPath(curDir, name));
      await refresh();
      selectOnly(name);
      // In listPreview, selecting the new note is enough -- the effect
      // below picks it up and opens it in the preview pane ready to type
      // into, so there's no separate inline-rename step to interrupt that.
      if (view !== "listPreview") setRenaming({ name, value: name });
    } catch (e) {
      setError(String(e));
    }
  }

  // listPreview's whole point is writing into a file immediately -- keep
  // its preview pane in sync with whatever single file is selected,
  // whether that's because the user just clicked one, switched into this
  // view while one was already selected, or a new note was just created.
  const previewEntryRef = useRef(previewEntry);
  previewEntryRef.current = previewEntry;
  useEffect(() => {
    if (view !== "listPreview" || selected.size !== 1) return;
    const name = [...selected][0];
    const entry = entries.find((en) => en.name === name);
    if (!entry) return;
    const prev = previewEntryRef.current;
    if (prev?.dir === curDir && prev.entry.name === name) return;
    // Folders preview their listing (no decryption) -- no sensitive gate.
    if (entry.is_dir) {
      setPreviewEntry({ dir: curDir, entry });
      return;
    }
    withSensitive(joinPath(curDir, name), () => setPreviewEntry({ dir: curDir, entry }));
  }, [view, selected, entries, curDir]);

  async function renamePreviewEntry(newName: string) {
    if (!previewEntry) return;
    const { dir, entry } = previewEntry;
    const src = joinPath(dir, entry.name);
    const dest = joinPath(dir, newName);
    try {
      inVault ? await api.moveEntry(src, dest) : await api.fsRename(src, dest);
      await refresh();
      setPreviewEntry({ dir, entry: { ...entry, name: newName } });
      selectOnly(newName);
    } catch (e) {
      setError(String(e));
    }
  }

  // ---- inline rename ----
  async function commitRename() {
    if (!renaming) return;
    const { name, value } = renaming;
    setRenaming(null);
    if (value.trim() === "" || value === name) return;
    const src = joinPath(curDir, name);
    const dest = joinPath(curDir, value);
    try {
      inVault ? await api.moveEntry(src, dest) : await api.fsRename(src, dest);
      await refresh();
      selectOnly(value);
    } catch (e) {
      setError(String(e));
    }
  }

  // Default "Delete": moves to the OS trash, reversible, no confirmation
  // needed -- matches Finder's plain Delete. Only meaningful outside a
  // vault (vault entries have no trash equivalent; those still go through
  // the confirm+permanent flow below).
  async function trashSelection(names: string[]) {
    try {
      for (const name of names) {
        await api.fsTrash(joinPath(curDir, name));
      }
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function emptyTrashNow() {
    try {
      await api.emptyTrash();
      if (loc.kind === "fs" && trashPath && loc.path === trashPath) refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function restoreAllFromTrash() {
    try {
      await api.trashRestoreAll();
      if (loc.kind === "fs" && trashPath && loc.path === trashPath) refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function restoreFromTrash(names: string[]) {
    try {
      await api.trashRestore(names);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function confirmIncomingDevice() {
    if (!incomingDevice) return;
    try {
      await api.syncthingAddDevice(incomingDevice.id, incomingDevice.name);
    } catch (e) {
      setError(String(e));
    }
    setIncomingDevice(null);
  }

  async function openTerminalAt(path: string) {
    try {
      await api.openTerminal(path, appSettings.terminalApp);
    } catch (e) {
      setError(String(e));
    }
  }

  async function runScript(path: string) {
    try {
      await api.runShellScript(path, appSettings.terminalApp);
    } catch (e) {
      setError(String(e));
    }
  }

  async function editScript(path: string) {
    try {
      await api.openInEditor(path);
    } catch (e) {
      setError(String(e));
    }
  }

  // ---- delete / unlock / new-vault ----
  async function submitPending(value: string) {
    if (!pending) return;
    try {
      switch (pending.kind) {
        case "delete":
          if (!inVault && trashPath && curDir === trashPath) {
            await api.trashPurge(pending.names);
          } else {
            for (const name of pending.names) {
              const full = joinPath(curDir, name);
              const entry = entries.find((e) => e.name === name);
              if (inVault) {
                entry?.is_dir ? await api.deleteDir(full) : await api.deleteFile(full);
              } else {
                await api.fsDelete(full);
              }
            }
          }
          break;
        case "secureDelete": {
          const paths = pending.names.map((n) => joinPath(curDir, n));
          await api.fsSecureDelete(paths, beginProgress("Secure Delete"));
          break;
        }
        case "gitCommit":
          if (value.trim() === "" || !gitRoot) return;
          await api.gitCommitAll(gitRoot, value.trim());
          break;
        case "freeze":
          if (value.trim() === "") return;
          await api.freezeFolder(joinPath(curDir, pending.entry.name), value);
          break;
      }
      setPending(null);
      refresh();
    } catch (e) {
      setError(String(e));
      setPending(null);
    }
  }

  async function submitUnlock(password: string, keep: boolean) {
    if (pending?.kind !== "unlock") return;
    try {
      await api.unlockVault(pending.path, password);
      setUnlockedRoots((prev) => new Set(prev).add(pending.path));
      setKeepUnlockedRoots((prev) => {
        const next = new Set(prev);
        if (keep) next.add(pending.path);
        else next.delete(pending.path);
        return next;
      });
      const dropImport = pendingDropImport.current;
      pendingDropImport.current = null;
      if (dropImport && dropImport.vaultRoot === pending.path) {
        try {
          for (const src of dropImport.srcPaths) {
            await api.importFile(src, joinPath(dropImport.destRel, baseName(src)));
          }
          refresh();
        } catch (e) {
          setError(String(e));
        }
      }
      const nav = pendingNav.current;
      pendingNav.current = null;
      setPending(null);
      setSheetError("");
      if (nav) commitLoc(nav.target, nav.push);
    } catch (e) {
      setSheetError("Incorrect password");
    }
  }

  // Locks any unlocked vault by root, not just the one currently being
  // browsed -- needed so a "Keep Unlocked" vault can be locked again from
  // its favorites entry after navigating away (the dedicated vault
  // sidebar section only shows while actually inside it).
  async function lockVaultRoot(root: string) {
    try {
      await api.lockVault(root);
    } catch {
      /* ignore */
    }
    setUnlockedRoots((prev) => cascadeRemoveRoots(prev, root));
    setKeepUnlockedRoots((prev) => cascadeRemoveRoots(prev, root));
    if (loc.kind === "vault" && (loc.root === root || loc.root.startsWith(root + "/"))) {
      const parent = vaultParents[root] ?? { kind: "fs", path: parentPath(root) };
      commitLoc(parent, true);
    }
  }
  async function lockCurrentVault() {
    if (loc.kind !== "vault") return;
    await lockVaultRoot(loc.root);
  }

  async function submitNewVault(name: string, password: string, opts: VaultCreateOptions) {
    try {
      let root: string;
      if (loc.kind === "fs") {
        root = joinPath(loc.path, name);
        await api.createVault(root, password);
      } else {
        // Nested vault: make the folder inside this vault, then create the
        // vault at its real FUSE path (see nested-vault handling).
        const rel = joinPath(curDir, name);
        await api.makeDir(rel);
        root = await api.openPath(rel);
        await api.createVault(root, password);
      }
      setVaultSettings((prev) => ({ ...prev, [root]: opts }));
      // Auto-unlock persists by real fs path; a nested vault's FUSE path is
      // per-session so it can't be keyed for startup auto-unlock.
      if (loc.kind === "fs" && opts.autoUnlock) {
        await api.setVaultAutoUnlock(root, password).catch(() => {});
      }
      setPending(null);
      setSheetError("");
      refresh();
    } catch (e) {
      setSheetError(String(e));
    }
  }

  // ---- context menus ----
  function entryMenu(e: React.MouseEvent, entry: Entry): void {
    e.preventDefault();
    e.stopPropagation();
    if (!selected.has(entry.name)) selectOnly(entry.name);
    const many = selected.size > 1 && selected.has(entry.name);
    const targetNames = many ? [...selected] : [entry.name];
    const isInTrashView = !inVault && trashPath !== null && curDir === trashPath;
    const isEncryptedFile = !entry.is_dir && entry.name.toLowerCase().endsWith(ENCRYPTED_FILE_EXT);
    const isZip = !entry.is_dir && entry.name.toLowerCase().endsWith(".zip");
    const isShellScript = !entry.is_dir && entry.name.toLowerCase().endsWith(".sh");
    const path = joinPath(curDir, entry.name);

    const getInfoItem: MenuItem = {
      label: !many ? "Get Info" : `Get Info (${targetNames.length} items)`,
      shortcut: "⌘I",
      onClick: () =>
        many
          ? setMultiInfoTarget(targetNames)
          : setInfoTarget({
              entry,
              fullPath: path,
              kind: loc.kind,
              root: loc.kind === "vault" ? loc.root : undefined,
            }),
    };

    // Everything reached less often than Open/Rename/Copy/Trash lives in
    // one "More" submenu (Duplicate, Create Shortcut, Use as Template,
    // Cut, Convert To, Resize, Security, Tag Color) instead of flooding
    // the top-level menu with every action this app can do to a file.
    const moreItems: MenuItem[] = [];

    const runnableShellScript = !many && !inVault && isShellScript;
    const items: MenuItem[] = runnableShellScript
      ? [
          { label: "Run", onClick: () => runScript(path) },
          { label: "Edit", onClick: () => editScript(path) },
          { type: "separator" },
          {
            label: "Rename",
            disabled: many,
            onClick: () => setRenaming({ name: entry.name, value: entry.name }),
          },
        ]
      : [
          {
            label: entry.is_vault ? "Open Vault" : isEncryptedFile ? "Decrypt and Open…" : "Open",
            shortcut: "⌘O",
            disabled: many,
            onClick: () => activate(curDir, entry),
          },
          { type: "separator" },
          {
            label: "Rename",
            disabled: many,
            onClick: () => setRenaming({ name: entry.name, value: entry.name }),
          },
        ];
    moreItems.push({ label: "Duplicate", shortcut: "⌘D", disabled: many, onClick: () => duplicate(entry) });
    if (!many && !inVault) {
      moreItems.push({ label: "Create Shortcut", onClick: () => createShortcut(entry) });
    }
    if (!many && !inVault && !entry.is_dir) {
      moreItems.push({ label: "Use as Template", onClick: () => useAsTemplate(entry) });
    }
    if (!many && entry.is_dir && !entry.is_vault && !mobile) {
      items.push({
        label: "Open in Terminal",
        onClick: async () => {
          const p = inVault ? await api.openPath(path) : path;
          openTerminalAt(p);
        },
      });
    }
    items.push(
      { type: "separator" },
      { label: "Copy", shortcut: "⌘C", onClick: () => copySel(entry) },
      { label: "Copy Absolute Path", onClick: () => copyEntryPaths(targetNames) },
      { type: "separator" }
    );
    if (!many && !entry.is_dir) {
      items.push({ label: "Share…", onClick: () => shareFile(entry) }, { type: "separator" });
    }
    moreItems.push({ label: "Cut", shortcut: "⌘X", onClick: () => cutSel(entry) });
    if (!many) {
      moreItems.push({
        label: pinnedPaths.has(path) ? "Unpin" : "Pin",
        onClick: () => togglePin(path),
      });
    }

    const compressItems: MenuItem[] = [];
    if (!many && isZip) {
      compressItems.push({ label: "Decompress", onClick: () => decompressEntry(entry) });
    }
    compressItems.push(
      {
        label: many ? `Compress ${targetNames.length} Items` : `Compress "${entry.name}"`,
        onClick: () => compressSelection(targetNames),
      },
      { label: "Compress…", onClick: () => setCompressTarget(targetNames) }
    );
    items.push({ type: "submenu", label: !many && isZip ? "Decompress" : "Compress", items: compressItems });

    if (!many && !entry.is_dir) {
      const kind = kindOf(entry);
      const ext = extOf(entry);
      if (kind === "image" && !inVault && ["png", "jpg", "jpeg", "bmp", "webp", "tiff", "tif"].includes(ext)) {
        const targets = IMAGE_CONVERT_TARGETS.filter((t) => t.ext !== ext && !(ext === "jpeg" && t.ext === "jpg"));
        const convertItems: MenuItem[] = targets.map((t) => ({
          label: t.label,
          onClick: () =>
            t.lossy
              ? setConvertTarget({ entry, targetExt: t.ext, targetLabel: t.label, mode: "imageQuality" })
              : runImageConvert(entry, t.ext, null),
        }));
        convertItems.push({ label: "PDF", onClick: () => runImageToPdf(entry) });
        moreItems.push({ type: "submenu", label: "Convert To", items: convertItems });
      } else if (kind === "pdf" && !inVault) {
        const pdfItems: MenuItem[] = [{ label: "Images (JPG, one per page)", onClick: () => runPdfToImages(entry) }];
        if (libreofficeAvailable) {
          pdfItems.push({ label: "Word Document (.docx)", onClick: () => runOfficeConvert(entry, "docx") });
        }
        moreItems.push({ type: "submenu", label: "Convert To", items: pdfItems });
      } else if (["doc", "docx", "odt", "rtf"].includes(ext) && !inVault && libreofficeAvailable) {
        moreItems.push({
          type: "submenu",
          label: "Convert To",
          items: [{ label: "PDF", onClick: () => runOfficeConvert(entry, "pdf") }],
        });
      } else if (kind === "video" && !inVault && ffmpegAvailable) {
        moreItems.push({
          type: "submenu",
          label: "Convert To",
          items: [
            ...VIDEO_CONVERT_TARGETS.filter((t) => t.ext !== ext).map((t) => ({
              label: t.label,
              onClick: () => setConvertTarget({ entry, targetExt: t.ext, targetLabel: t.label, mode: "mediaQuality" }),
            })),
            { type: "separator" },
            {
              label: "Extract Audio (MP3)",
              onClick: () => setConvertTarget({ entry, targetExt: "mp3", targetLabel: "MP3", mode: "mediaQuality" }),
            },
            { label: "Transcribe to Text (offline)…", onClick: () => runTranscribe(entry) },
          ],
        });
      } else if (kind === "audio" && !inVault && ffmpegAvailable) {
        const targets = AUDIO_CONVERT_TARGETS.filter((t) => t.ext !== ext);
        moreItems.push({
          type: "submenu",
          label: "Convert To",
          items: [
            ...targets.map((t) => ({
              label: t.label,
              onClick: () =>
                t.lossy
                  ? setConvertTarget({ entry, targetExt: t.ext, targetLabel: t.label, mode: "mediaQuality" })
                  : runMediaConvert(entry, t.ext, "medium"),
            })),
            { type: "separator" as const },
            { label: "Transcribe to Text (offline)…", onClick: () => runTranscribe(entry) },
          ],
        });
      }
    }

    const resizeTargets = entries
      .filter((en) => targetNames.includes(en.name) && !en.is_dir && kindOf(en) === "image")
      .map((en) => en.name);
    if (resizeTargets.length > 0) {
      moreItems.push({ label: resizeTargets.length > 1 ? `Resize ${resizeTargets.length} Images…` : "Resize…", onClick: () => setResizeTarget(resizeTargets) });
    }

    // A mixed selection of images/videos (optionally plus one audio
    // track) can be turned into a single montage -- offered here rather
    // than under the single-file Convert To above since it only makes
    // sense for a multi-item selection.
    if (many && !inVault && ffmpegAvailable) {
      const selected = entries.filter((en) => targetNames.includes(en.name) && !en.is_dir);
      const visualEntries = selected.filter((en) => kindOf(en) === "image" || kindOf(en) === "video");
      const visual = visualEntries.map((en) => en.name);
      const imageCount = visualEntries.filter((en) => kindOf(en) === "image").length;
      const videoCount = visualEntries.filter((en) => kindOf(en) === "video").length;
      const audioEntry = selected.find((en) => kindOf(en) === "audio");
      if (visual.length >= 2) {
        moreItems.push({
          type: "submenu",
          label: "Convert To",
          items: [
            {
              label: "MP4 (Montage)…",
              onClick: () => setMontageTarget({ visual, audio: audioEntry?.name ?? null, imageCount, videoCount }),
            },
          ],
        });
      }
    }

    // Everything that's about protecting or scrubbing this file's
    // content lives under one "Security" submenu (encrypt, strip
    // metadata, shred) rather than three separate top-level entries.
    const securityItems: MenuItem[] = [];
    if (!many && (!entry.is_dir || !entry.is_vault)) {
      if (!entry.is_dir) {
        securityItems.push(
          isEncryptedFile
            ? { label: "Decrypt…", onClick: () => setDecryptPrompt({ entry, error: "", mode: "inplace" }) }
            : { label: "Encrypt…", onClick: () => setEncryptTarget(entry) }
        );
      } else {
        securityItems.push({ label: "Convert to Vault…", onClick: () => setEncryptTarget(entry) });
      }
    }
    const clearMetaTargets = entries
      .filter((en) => targetNames.includes(en.name) && !en.is_dir)
      .map((en) => en.name);
    if (clearMetaTargets.length > 0) {
      securityItems.push({
        label:
          clearMetaTargets.length > 1 ? `Clear Metadata (${clearMetaTargets.length} Items)` : "Clear Metadata",
        onClick: () => clearMetadataSelection(clearMetaTargets),
      });
    }
    if (!inVault) {
      securityItems.push({
        label: "Secure Delete…",
        danger: true,
        onClick: () => setPending({ kind: "secureDelete", names: targetNames }),
      });
    }
    if (securityItems.length > 0) {
      moreItems.push({ type: "submenu", label: "Security", items: securityItems });
    }

    if (!many && !inVault && gitRoot) {
      const rel = relToGitRoot(entry.name);
      const code = rel !== null ? gitStatus[rel] : undefined;
      if (code) {
        const staged = code[0] !== " " && code[0] !== "?";
        items.push({
          type: "submenu",
          label: "Git",
          items: [
            staged
              ? { label: "Unstage", onClick: () => gitUnstageEntry(entry) }
              : { label: "Stage", onClick: () => gitStageEntry(entry) },
            { label: "Discard Changes…", danger: true, onClick: () => gitDiscardEntry(entry) },
          ],
        });
      }
    }

    if (!many && !inVault) {
      moreItems.push({
        type: "submenu",
        label: "Color",
        items: [
          ...TAG_COLORS.map((c) => ({
            label: `${c.label}${tags[entry.name] === c.key ? " ✓" : ""}`,
            swatch: c.hex,
            onClick: () => setTagFor(entry.name, c.key),
          })),
          ...(tags[entry.name]
            ? [{ label: "None", onClick: () => setTagFor(entry.name, null) }]
            : []),
        ],
      });
    }
    if (!many && entry.is_dir && !inVault) {
      moreItems.push(
        favPaths.includes(path)
          ? { label: "Remove from Favorites", onClick: () => removeFavorite(path) }
          : { label: "Add to Favorites", onClick: () => addFavorite(path) }
      );
      moreItems.push(
        buildSyncSubmenu(path, {
          drivePairsByPath,
          gitSyncedPaths,
          localSyncedPaths,
          setDriveTarget,
          setGitSyncTarget,
          setLocalSyncTarget,
          setSyncthingTarget,
        })
      );
      moreItems.push({ label: "Change Icon…", onClick: () => setIconTarget(path) });
      if (!mobile) {
        moreItems.push(
          frozenPaths.has(path)
            ? { label: "Unfreeze…", onClick: () => setUnfreezeTarget(path) }
            : { label: "Freeze…", onClick: () => setPending({ kind: "freeze", entry }) }
        );
      }
    }
    items.push({ type: "submenu", label: "More", items: moreItems });
    if (inVault && !many) {
      const fullRel = joinPath(curDir, entry.name);
      const inherited = hasSensitiveAncestor(fullRel);
      const marked = isMarkedSensitive(fullRel);
      items.push(
        { type: "separator" },
        inherited
          ? {
              label: "Sensitive (inherited from folder)",
              disabled: true,
              onClick: () => {},
            }
          : marked
            ? { label: "Remove sensitive", onClick: () => toggleSensitive(fullRel, false) }
            : {
                label: entry.is_dir ? "Mark folder sensitive" : "Mark as sensitive",
                onClick: () => toggleSensitive(fullRel, true),
              }
      );
    }
    items.push({ type: "separator" }, getInfoItem, { type: "separator" });
    if (isInTrashView) {
      items.push(
        {
          label: many ? `Restore ${targetNames.length} Items` : "Restore",
          onClick: () => restoreFromTrash(targetNames),
        },
        {
          label: many ? `Delete ${targetNames.length} Items Permanently` : "Delete Permanently",
          danger: true,
          onClick: () => setPending({ kind: "delete", names: targetNames }),
        }
      );
    } else if (!inVault && !mobile) {
      items.push({
        label: many ? `Move ${targetNames.length} Items to Trash` : "Move to Trash",
        shortcut: "⌫ / ⌃⌫ permanently",
        danger: true,
        onClick: () => trashSelection(targetNames),
      });
    } else if (!inVault && mobile) {
      items.push({
        label: many ? `Delete ${targetNames.length} Items` : "Delete",
        danger: true,
        onClick: () => setPending({ kind: "delete", names: targetNames }),
      });
    } else {
      items.push({
        label: many ? `Delete ${targetNames.length} Items` : "Delete",
        shortcut: "⌘⌫",
        danger: true,
        onClick: () => setPending({ kind: "delete", names: targetNames }),
      });
    }
    setMenu({ x: e.clientX, y: e.clientY, items });
  }

  // Finder-style "View Options" dropdown -- one button showing the current
  // view's icon instead of a 4-wide segmented control, matching macOS's
  // toolbar convention (and freeing up the width that control took).
  function openViewMenu(e: React.MouseEvent): void {
    e.stopPropagation();
    const r = e.currentTarget.getBoundingClientRect();
    const options: { key: View; label: string }[] = [
      { key: "icon", label: "Icons" },
      { key: "list", label: "List" },
      { key: "column", label: "Columns" },
      { key: "listPreview", label: "List with Preview" },
    ];
    setMenu({
      x: r.left,
      y: r.bottom + 4,
      items: options.map((o) => ({
        label: view === o.key ? `✓ ${o.label}` : o.label,
        onClick: () => setView(o.key),
      })),
    });
  }

  function driveMenu(e: React.MouseEvent, d: import("./api").Drive): void {
    e.preventDefault();
    const items: MenuItem[] = [];
    if (d.mountpoint) {
      items.push({ label: "Open", onClick: () => go({ kind: "fs", path: d.mountpoint as string }) });
      items.push({ type: "separator" });
    }
    items.push({ label: "Get Info", onClick: () => setMachineInfoOpen(true) });
    if (d.removable && !mobile) {
      items.push({ type: "separator" });
      items.push({ label: "Format…", danger: true, onClick: () => setFormatTarget(d) });
    }
    setMenu({ x: e.clientX, y: e.clientY, items });
  }

  function backgroundMenu(e: React.MouseEvent): void {
    e.preventDefault();
    setSelected(new Set());
    const items: MenuItem[] = [
      { label: "New Folder", shortcut: "⇧⌘N", onClick: createNewFolder },
      { label: "New File", shortcut: "⌘N", onClick: createNewFile },
    ];
    if (!inVault && templates.length > 0) {
      items.push({
        type: "submenu",
        label: "New From Template",
        items: [
          ...templates.map((t) => ({ label: t.label, onClick: () => newFromTemplate(t) })),
          { type: "separator" as const },
          { label: "Manage Templates…", onClick: () => setManageTemplatesOpen(true) },
        ],
      });
    }
    items.push({ label: "New Vault…", onClick: () => { setSheetError(""); setPending({ kind: "newVault" }); } });
    if (!mobile) {
      items.push({
        label: "Open in Terminal",
        onClick: async () => {
          // In a vault, open the terminal at the decrypted FUSE mount path.
          const p = inVault ? await api.openPath(curDir) : curDir;
          openTerminalAt(p);
        },
      });
    }
    if (gitRoot) {
      items.push({
        type: "submenu",
        label: "Git",
        items: [
          { label: "Status…", onClick: () => setGitStatusOpen(true) },
          { label: "Pull", onClick: gitPullNow },
          { label: "Push", onClick: gitPushNow },
          { type: "separator" },
          { label: "Commit All Changes…", onClick: () => setPending({ kind: "gitCommit" }) },
        ],
      });
    }
    items.push(
      { type: "separator" },
      { label: "Paste", shortcut: "⌘V", disabled: !clipboard || clipboard.kind !== loc.kind, onClick: paste }
    );
    {
      items.push({ type: "separator" });
      const sortOptions: { key: typeof sortKey; label: string }[] = [
        { key: "name", label: "Name" },
        { key: "date", label: "Date Modified" },
        ...(!inVault ? [{ key: "created" as const, label: "Date Created" }] : []),
        { key: "size", label: "Size" },
        { key: "kind", label: "Type" },
      ];
      items.push({
        type: "submenu",
        label: "Sort By",
        items: sortOptions.map((opt) => ({
          label: `${opt.label}${sortKey === opt.key ? " ✓" : ""}`,
          onClick: () => toggleSort(opt.key),
        })),
      });
    }
    setMenu({ x: e.clientX, y: e.clientY, items });
  }

  // ---- breadcrumb items ----
  type Crumb = { key: string; label: string; loc: Loc; dropDir: string; lock?: boolean };
  // Recursive so a vault nested inside another vault shows the parent
  // vault's own breadcrumb (name + lock icon) instead of the parent's
  // internal FUSE mountpoint path leaking into the breadcrumb as if it
  // were a real filesystem location.
  function crumbsFor(l: Loc): Crumb[] {
    const out: Crumb[] = [];
    if (l.kind === "fs") {
      const parts = l.path.split("/").filter(Boolean);
      out.push({ key: "/", label: "System", loc: { kind: "fs", path: "/" }, dropDir: "/" });
      let acc = "";
      for (const p of parts) {
        acc = acc + "/" + p;
        out.push({ key: acc, label: p, loc: { kind: "fs", path: acc }, dropDir: acc });
      }
      return out;
    }

    const parent = vaultParents[l.root];
    if (parent) {
      out.push(...crumbsFor(parent));
    } else {
      // top-level vault with no recorded parent (e.g. reopened via
      // history): fall back to its real fs ancestors.
      const parentParts = parentPath(l.root).split("/").filter(Boolean);
      out.push({ key: "/", label: "System", loc: { kind: "fs", path: "/" }, dropDir: "/" });
      let acc = "";
      for (const p of parentParts) {
        acc = acc + "/" + p;
        out.push({ key: acc, label: p, loc: { kind: "fs", path: acc }, dropDir: acc });
      }
    }
    // the vault root itself
    out.push({
      key: l.root,
      label: baseName(l.root),
      loc: { kind: "vault", root: l.root, rel: "" },
      dropDir: "",
      lock: true,
    });
    // rel segments inside the vault
    const relParts = l.rel === "" ? [] : l.rel.split("/");
    let racc = "";
    for (const p of relParts) {
      racc = racc === "" ? p : racc + "/" + p;
      out.push({
        key: l.root + "::" + racc,
        label: p,
        loc: { kind: "vault", root: l.root, rel: racc },
        dropDir: racc,
      });
    }
    return out;
  }
  const crumbs = crumbsFor(loc);
  // A deep path can't fit the breadcrumb bar -- collapse everything but
  // the last few segments (the ones actually useful to click back to)
  // behind a leading "…" that jumps to the nearest hidden ancestor, same
  // idea as Finder's path bar truncation. Phone width fits noticeably
  // fewer than desktop.
  const MAX_CRUMBS = mobile ? 3 : 5;
  const hiddenCrumbs = crumbs.length > MAX_CRUMBS ? crumbs.slice(0, crumbs.length - MAX_CRUMBS) : [];
  const visibleCrumbs = hiddenCrumbs.length ? crumbs.slice(crumbs.length - MAX_CRUMBS) : crumbs;

  // ---- column view chain ----
  // Capped at 3 columns: without this, a chain from the root/vault-root
  // down to a deeply-nested current folder just keeps adding columns,
  // which both gets overwhelming to look at and reflows the whole view
  // drastically on every navigation. Keeping only the last 3 ancestors
  // (i.e. dropping the oldest/leftmost ones once there are more) means
  // the view is always exactly as wide, however deep the current folder
  // actually is.
  const MAX_COLUMNS = 3;
  function columnChain(): { dirs: string[]; sel: string[] } {
    let dirs: string[];
    let parts: string[];
    if (loc.kind === "vault") {
      parts = loc.rel === "" ? [] : loc.rel.split("/");
      dirs = [""];
      let acc = "";
      for (const p of parts) {
        acc = acc === "" ? p : acc + "/" + p;
        dirs.push(acc);
      }
    } else {
      parts = loc.path.split("/").filter(Boolean);
      dirs = ["/"];
      let acc = "";
      for (const p of parts) {
        acc = acc + "/" + p;
        dirs.push(acc);
      }
    }
    if (dirs.length > MAX_COLUMNS) {
      const dropped = dirs.length - MAX_COLUMNS;
      return { dirs: dirs.slice(dropped), sel: parts.slice(dropped) };
    }
    return { dirs, sel: parts };
  }

  // Escape closes whatever sheet/overlay is currently on top. Checked in
  // roughly innermost-first order so only the topmost one closes if
  // somehow more than one could be open. The context menu handles its
  // own Escape already (ContextMenu.tsx); the re-auth challenge is
  // deliberately excluded -- it shouldn't be dismissable that easily.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key !== "Escape") return;
      if (incomingDevice) return setIncomingDevice(null);
      if (multiInfoTarget) return setMultiInfoTarget(null);
      if (infoTarget) return setInfoTarget(null);
      if (iconTarget) return setIconTarget(null);
      if (decryptPrompt) return setDecryptPrompt(null);
      if (encryptTarget) return setEncryptTarget(null);
      if (archiveMountPrompt) return setArchiveMountPrompt(null);
      if (zipPasswordPrompt) return setZipPasswordPrompt(null);
      if (compressTarget) return setCompressTarget(null);
      if (montageTarget) return setMontageTarget(null);
      if (resizeTarget) return setResizeTarget(null);
      if (convertTarget) return setConvertTarget(null);
      if (formatTarget) return setFormatTarget(null);
      if (driveTarget) return setDriveTarget(null);
      if (gitSyncTarget) return setGitSyncTarget(null);
      if (localSyncTarget) return setLocalSyncTarget(null);
      if (syncthingTarget) return setSyncthingTarget(null);
      if (unfreezeTarget) return setUnfreezeTarget(null);
      if (gitStatusOpen) return setGitStatusOpen(false);
      if (manageTemplatesOpen) return setManageTemplatesOpen(false);
      if (machineInfoOpen) return setMachineInfoOpen(false);
      if (settingsOpen) return setSettingsOpen(false);
      if (pending) return setPending(null);
      if (sidebarOpen) return setSidebarOpen(false);
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    sidebarOpen,
    incomingDevice,
    multiInfoTarget,
    infoTarget,
    iconTarget,
    decryptPrompt,
    encryptTarget,
    archiveMountPrompt,
    zipPasswordPrompt,
    compressTarget,
    montageTarget,
    resizeTarget,
    convertTarget,
    formatTarget,
    driveTarget,
    gitSyncTarget,
    localSyncTarget,
    syncthingTarget,
    unfreezeTarget,
    gitStatusOpen,
    manageTemplatesOpen,
    machineInfoOpen,
    settingsOpen,
    pending,
  ]);

  // favorites (real OS locations)
  function favLabel(path: string): string {
    if (path === "/") return "System";
    if (path === home) return "All My Files";
    if (path === "/usr/share/applications") return "Applications";
    if (path === PHONE_STORAGE_PATH) return "Phone Storage";
    return baseName(path) || path;
  }
  // "Phone Storage" needs Android's "All files access" special permission
  // -- it can't be requested via a normal runtime dialog, so tapping it
  // sends the user to the system settings screen that grants it instead
  // of just navigating into a folder that would 403.
  async function openFavorite(path: string) {
    if (mobile && path === PHONE_STORAGE_PATH) {
      try {
        const granted = await api.androidStorageAccessGranted();
        if (!granted) {
          await api.androidRequestStorageAccess();
          return;
        }
      } catch {
        /* command not available (non-Android mobile) -- fall through */
      }
    }
    let isVault = false;
    try {
      isVault = await api.fsIsVault(path);
    } catch {
      /* ignore -- treat as a plain folder */
    }
    go(isVault ? { kind: "vault", root: path, rel: "" } : { kind: "fs", path });
  }
  const favorites = favPaths.map((path) => ({
    label: favLabel(path),
    path,
    icon: customIcons[path],
  }));

  // A folder that's itself a distinct sync root gets its own specific
  // badge; a file (or subfolder) merely *inside* a currently synced
  // folder inherits that same badge (files don't get their own separate
  // sync pairing) so the "this whole tree is under sync" fact is visible
  // on more than just the one root folder, matching the same convention
  // Dropbox/OneDrive/Google Drive Desktop use.
  // Walks up from `path` (through every parent directory, not just an
  // exact match) looking for a synced root -- a file several folders deep
  // inside a paired tree is still "inside a synced folder" and should
  // still show its badge, not just direct children of the paired root.
  function syncRootFor(path: string): { badge: "git" | "drive" | "local"; root: string } | null {
    let p = path;
    while (p) {
      if (gitSyncedPaths.has(p)) return { badge: "git", root: p };
      if (driveSyncedPaths.has(p)) return { badge: "drive", root: p };
      if (localSyncedPaths.has(p)) return { badge: "local", root: p };
      const parent = parentPath(p);
      if (parent === p) return null;
      p = parent;
    }
    return null;
  }

  function syncInfoFor(entry: Entry): { badge: "git" | "drive" | "local" | null; state: "syncing" | "synced" | null } {
    const path = joinPath(curDir, entry.name);
    const hit = (entry.is_dir ? syncRootFor(path) : null) ?? syncRootFor(curDir);
    if (!hit) return { badge: null, state: null };
    const { badge, root } = hit;
    if (syncingPaths.has(root) || syncingPaths.has(path)) return { badge, state: "syncing" };
    if (justSyncedPaths.has(root) || justSyncedPaths.has(path)) return { badge, state: "synced" };
    return { badge, state: null };
  }

  // "List with Preview" reuses the exact same row rendering as plain list
  // view (entryView forces the "list" CSS/thumbnail-size path for it) --
  // only the click handler and the pane alongside it differ.
  const entryView: View = view === "listPreview" ? "list" : view;
  function renderListBody() {
    return (
      <div className={`entries-wrap ${entryView}`}>
        {(view === "list" || view === "listPreview") && entries.length > 0 && (
          <div className={`list-header ${view === "listPreview" ? "compact" : ""}`}>
            <span className="lh-spacer" />
            <span className={`lh-name ${sortKey === "name" ? "on" : ""}`} onClick={() => toggleSort("name")}>
              Name {sortKey === "name" && (sortDir === 1 ? "▲" : "▼")}
            </span>
            {view !== "listPreview" && (
              <>
                <span className={`lh-date ${sortKey === "date" ? "on" : ""}`} onClick={() => toggleSort("date")}>
                  Date Modified {sortKey === "date" && (sortDir === 1 ? "▲" : "▼")}
                </span>
                <span className={`lh-size ${sortKey === "size" ? "on" : ""}`} onClick={() => toggleSort("size")}>
                  Size {sortKey === "size" && (sortDir === 1 ? "▲" : "▼")}
                </span>
                <span className={`lh-kind ${sortKey === "kind" ? "on" : ""}`} onClick={() => toggleSort("kind")}>
                  Type {sortKey === "kind" && (sortDir === 1 ? "▲" : "▼")}
                </span>
              </>
            )}
          </div>
        )}
        <div
          className={`entries ${entryView} ${view === "listPreview" ? "compact" : ""}`}
          style={
            view === "icon"
              ? ({
                  "--icon-scale": iconScale,
                  ...(gridCols ? { gridTemplateColumns: `repeat(${gridCols}, minmax(0, 1fr))` } : {}),
                } as React.CSSProperties)
              : undefined
          }
        >
          {sortedEntries.map((entry) => {
            const syncInfo = syncInfoFor(entry);
            return (
            <EntryTile
              key={entry.name}
              entry={entry}
              fullPath={joinPath(curDir, entry.name)}
              inVault={inVault}
              view={entryView}
              compact={view === "listPreview"}
              selected={selected.has(entry.name)}
              cut={
                clipboard?.mode === "cut" &&
                clipboard.kind === loc.kind &&
                clipboard.paths.includes(joinPath(curDir, entry.name))
              }
              isDropTarget={dropTarget === "entry:" + joinPath(curDir, entry.name)}
              tagHex={TAG_COLORS.find((c) => c.key === tags[entry.name])?.hex}
              customIcon={!inVault ? customIcons[joinPath(curDir, entry.name)] : undefined}
              hideExtensions={appSettings.hideExtensions}
              pinned={pinnedPaths.has(joinPath(curDir, entry.name))}
              sensitive={inVault && isSensitivePath(joinPath(curDir, entry.name))}
              syncBadge={syncInfo.badge ?? undefined}
              syncState={syncInfo.state ?? undefined}
              editing={renaming?.name === entry.name}
              editValue={renaming?.name === entry.name ? renaming.value : ""}
              onEditChange={(v) => setRenaming((r) => (r ? { ...r, value: v } : r))}
              onEditCommit={commitRename}
              onEditCancel={() => setRenaming(null)}
              onClick={(e) => {
                onEntryClick(e, entry);
                if (view === "listPreview" && !entry.is_dir) {
                  withSensitive(joinPath(curDir, entry.name), () => setPreviewEntry({ dir: curDir, entry }));
                }
                // Touch has no double-click -- a single tap both selects and
                // opens, same as every mobile file browser; long-press still
                // does the desktop's right-click (context menu) job.
                if (mobile && !renaming) {
                  cancelPendingRenameClick();
                  activate(curDir, entry);
                }
              }}
              onOpen={() => {
                cancelPendingRenameClick();
                activate(curDir, entry);
              }}
              onMenu={(e) => entryMenu(e, entry)}
              onDragStart={(e) => beginDrag(e, entry)}
              onDragOver={
                entry.is_dir && (!entry.is_vault || !inVault)
                  ? (e) => {
                      e.preventDefault();
                      setDropTarget("entry:" + joinPath(curDir, entry.name));
                    }
                  : undefined
              }
              onDragLeave={
                entry.is_dir
                  ? () => {
                      const key = "entry:" + joinPath(curDir, entry.name);
                      setDropTarget((t) => (t === key ? null : t));
                    }
                  : undefined
              }
              onDrop={
                entry.is_dir && (!entry.is_vault || !inVault)
                  ? (e) => {
                      e.preventDefault();
                      dropInto(joinPath(curDir, entry.name));
                    }
                  : undefined
              }
            />
            );
          })}
          {entries.length === 0 && (
            <p className="empty-hint">
              {inVault ? "Empty folder — drag files here to encrypt them." : "Empty folder."}
            </p>
          )}
        </div>
      </div>
    );
  }

  function toggleSort(key: typeof sortKey) {
    if (sortKey === key) setSortDir((d) => (d === 1 ? -1 : 1));
    else {
      setSortKey(key);
      setSortDir(1);
    }
  }

  const fullPath = inVault ? joinPath(loc.root, loc.rel) : loc.path;

  function beginEditPath() {
    setPathInput(fullPath);
    setEditingPath(true);
  }
  function submitEditPath() {
    const value = pathInput.trim();
    setEditingPath(false);
    if (value === "" || value === fullPath) return;
    if (inVault && (value === loc.root || value.startsWith(loc.root + "/"))) {
      go({ kind: "vault", root: loc.root, rel: value.slice(loc.root.length + 1) || "" });
    } else {
      go({ kind: "fs", path: value });
    }
  }
  async function copyPath() {
    try {
      await navigator.clipboard.writeText(fullPath);
      setPathCopied(true);
      setTimeout(() => setPathCopied(false), 1200);
    } catch (e) {
      setError(String(e));
    }
  }

  async function copyEntryPaths(names: string[]) {
    const paths = names.map((n) => {
      const rel = joinPath(curDir, n);
      return inVault && loc.kind === "vault" ? joinPath(loc.root, rel) : rel;
    });
    try {
      await navigator.clipboard.writeText(paths.join("\n"));
    } catch (e) {
      setError(String(e));
    }
  }

  const canPaste = !!clipboard;

  return (
    <>
      <div className="explorer">
      {mobile && sidebarOpen && (
        <div className="sidebar-backdrop" onClick={() => setSidebarOpen(false)} />
      )}
      <aside
        className={`sidebar ${mobile && sidebarOpen ? "open" : ""} ${favCollapsed ? "sidebar-compact" : ""}`}
        data-tauri-drag-region
        onClickCapture={() => {
          if (mobile) setSidebarOpen(false);
        }}
      >
        {!mobile && (
          <div className="sidebar-top" data-tauri-drag-region>
            <TrafficLights />
          </div>
        )}
        <div
          className={`sidebar-section sidebar-section-collapsible ${dropTarget === "fav-add" ? "drop" : ""}`}
          onDragOver={(e) => {
            e.preventDefault();
            setDropTarget("fav-add");
          }}
          onDragLeave={() => setDropTarget((t) => (t === "fav-add" ? null : t))}
          onDrop={(e) => {
            e.preventDefault();
            setDropTarget(null);
            const paths = dragPaths.current;
            dragPaths.current = [];
            for (const p of paths) {
              const dragged = entries.find((en) => joinPath(curDir, en.name) === p);
              if (dragged?.is_dir) addFavorite(p);
            }
          }}
        >
          {!favCollapsed && "Favorites"}
          <button
            type="button"
            className="sidebar-section-collapse-btn"
            title={favCollapsed ? "Expand Favorites" : "Collapse Favorites"}
            onClick={() => setFavCollapsed((v) => !v)}
          >
            {favCollapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
          </button>
        </div>
        <div
          className={`sidebar-item ${showMyComputer ? "active" : ""} ${favCollapsed ? "icon-only" : ""}`}
          title={favCollapsed ? "My Computer" : undefined}
          onClick={openMyComputer}
          onContextMenu={(e) => {
            e.preventDefault();
            setMenu({
              x: e.clientX,
              y: e.clientY,
              items: [{ label: "Get Info", onClick: () => setMachineInfoOpen(true) }],
            });
          }}
        >
          <span className="sidebar-ico place">
            <ComputerGlyph size={27} />
          </span>
          {!favCollapsed && "My Computer"}
        </div>
        {favorites.map((f, i) => {
          const active = !showMyComputer && loc.kind === "fs" && loc.path === f.path;
          return (
            <div
              key={f.path}
              data-path={f.path}
              className={`sidebar-item ${active ? "active" : ""} ${
                dropTarget === "fav:" + f.path ? "drop" : ""
              } ${draggingFavIdx === i ? "dragging" : ""} ${favCollapsed ? "icon-only" : ""}`}
              title={favCollapsed ? `${f.label}\n${f.path}` : f.path}
              onClick={() => openFavorite(f.path)}
              draggable
              onDragStart={(e) => {
                dragFavIndex.current = i;
                setDraggingFavIdx(i);
                e.dataTransfer.effectAllowed = "move";
                e.dataTransfer.setData("text/plain", f.path);
              }}
              onDragEnd={() => {
                dragFavIndex.current = null;
                setDraggingFavIdx(null);
              }}
              onDragOver={(e) => {
                e.preventDefault();
                setDropTarget("fav:" + f.path);
              }}
              onDragLeave={() => setDropTarget((t) => (t === "fav:" + f.path ? null : t))}
              onDrop={(e) => {
                e.preventDefault();
                setDropTarget(null);
                if (dragFavIndex.current !== null) {
                  const from = dragFavIndex.current;
                  dragFavIndex.current = null;
                  setDraggingFavIdx(null);
                  moveFavorite(from, i);
                  return;
                }
                if (loc.kind === "fs") dropInto(f.path);
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                const items: MenuItem[] = [
                  { label: "Change Icon…", onClick: () => setIconTarget(f.path) },
                ];
                if (f.path !== "/" && !inVault) {
                  items.push(
                    buildSyncSubmenu(f.path, {
                      drivePairsByPath,
                      gitSyncedPaths,
                      localSyncedPaths,
                      setDriveTarget,
                      setGitSyncTarget,
                      setLocalSyncTarget,
                      setSyncthingTarget,
                    })
                  );
                }
                if (f.path !== "/") {
                  items.push({
                    type: "submenu",
                    label: "Color",
                    items: [
                      ...TAG_COLORS.map((c) => ({
                        label: `${c.label}${favTags[f.path] === c.key ? " ✓" : ""}`,
                        swatch: c.hex,
                        onClick: () => setTagForPath(f.path, c.key),
                      })),
                      ...(favTags[f.path]
                        ? [{ label: "None", onClick: () => setTagForPath(f.path, null) }]
                        : []),
                    ],
                  });
                }
                items.push(
                  { type: "separator" },
                  defaultStartPath === f.path
                    ? { label: "Unset as Default ✓", onClick: () => setDefaultStartPath(null) }
                    : { label: "Set as Default", onClick: () => setDefaultStartPath(f.path) },
                  { type: "separator" }
                );
                if (unlockedRoots.has(f.path)) {
                  items.push(
                    { label: "Lock", danger: true, onClick: () => lockVaultRoot(f.path) },
                    { type: "separator" }
                  );
                }
                items.push({
                  label: "Remove from Favorites",
                  danger: true,
                  onClick: () => removeFavorite(f.path),
                });
                setMenu({ x: e.clientX, y: e.clientY, items });
              }}
            >
              <span className="sidebar-ico place">
                {f.icon && customIconUrl(f.icon) ? (
                  <img className="sidebar-fav-img" src={customIconUrl(f.icon)} alt="" draggable={false} />
                ) : f.icon && symbolIconSvg(f.icon) ? (
                  <span
                    className="symbol-icon sidebar-sym"
                    dangerouslySetInnerHTML={{ __html: symbolIconSvg(f.icon)! }}
                  />
                ) : f.icon ? (
                  <span className="sidebar-emoji">{f.icon}</span>
                ) : f.label === "System" ? (
                  <DiskGlyph size={27} />
                ) : (
                  <PlaceGlyph
                    size={27}
                    color={TAG_COLORS.find((c) => c.key === favTags[f.path])?.hex}
                  />
                )}
                {(gitSyncedPaths.has(f.path) || driveSyncedPaths.has(f.path) || localSyncedPaths.has(f.path)) && (
                  <span
                    className={`sync-badge ${syncingPaths.has(f.path) ? "syncing" : ""} ${
                      justSyncedPaths.has(f.path) ? "synced" : ""
                    }`}
                  >
                    {justSyncedPaths.has(f.path) ? (
                      <CheckGlyph size={11} />
                    ) : syncingPaths.has(f.path) ? (
                      <RefreshGlyph size={11} />
                    ) : gitSyncedPaths.has(f.path) ? (
                      <GitBranchGlyph size={11} />
                    ) : driveSyncedPaths.has(f.path) ? (
                      <CloudSyncGlyph size={11} />
                    ) : (
                      <LocalSyncGlyph size={11} />
                    )}
                  </span>
                )}
              </span>
              {!favCollapsed && f.label}
            </div>
          );
        })}

        {trashPath && (
          <div
            className={`sidebar-item ${!showMyComputer && loc.kind === "fs" && loc.path === trashPath ? "active" : ""} ${favCollapsed ? "icon-only" : ""}`}
            title={favCollapsed ? "Trash" : undefined}
            onClick={() => go({ kind: "fs", path: trashPath })}
            onContextMenu={(e) => {
              e.preventDefault();
              setMenu({
                x: e.clientX,
                y: e.clientY,
                items: [
                  { label: "Restore All", onClick: restoreAllFromTrash },
                  { type: "separator" },
                  { label: "Empty Trash", danger: true, onClick: emptyTrashNow },
                ],
              });
            }}
          >
            <span className="sidebar-ico place">
              <TrashGlyph size={22} />
            </span>
            {!favCollapsed && "Trash"}
          </div>
        )}

        {/* Every currently-unlocked vault stays listed here -- including one
            left with "Keep Unlocked" after navigating out of it, so it's
            never silently unlocked-and-forgotten. Click to jump back in,
            right-click to lock. */}
        {unlockedRoots.size > 0 && (
          <>
            {!favCollapsed && <div className="sidebar-section">Unlocked</div>}
            {[...unlockedRoots].map((root) => {
              const active = loc.kind === "vault" && loc.root === root;
              const kept = keepUnlockedRoots.has(root);
              return (
                <div
                  key={root}
                  className={`sidebar-item vaultrow ${active ? "active" : ""} ${favCollapsed ? "icon-only" : ""}`}
                  title={favCollapsed ? baseName(root) : undefined}
                  onClick={() => go({ kind: "vault", root, rel: "" })}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setMenu({
                      x: e.clientX,
                      y: e.clientY,
                      items: [{ label: "Lock", danger: true, onClick: () => lockVaultRoot(root) }],
                    });
                  }}
                >
                  <span className="sidebar-ico vault-open">
                    <LockOpenGlyph size={19} />
                  </span>
                  {!favCollapsed && baseName(root)}
                  {kept && <span className="vault-kept" title="Kept unlocked" />}
                </div>
              );
            })}
          </>
        )}

        <div className="sidebar-spacer" />
      </aside>

      <div className="main">
        <div className="titlebar toolbar" data-tauri-drag-region>
          <div className="nav-buttons">
            {mobile && (
              <button
                className="tool-btn"
                onClick={() => setSidebarOpen((v) => !v)}
                aria-label="Menu"
              >
                <MenuGlyph />
              </button>
            )}
            <button className="tool-btn" onClick={goBack} disabled={histIdx === 0} aria-label="Back">
              <ChevronLeft />
            </button>
            <button
              className="tool-btn"
              onClick={goForward}
              disabled={histIdx >= history.length - 1}
              aria-label="Forward"
            >
              <ChevronRight />
            </button>
            <button className="tool-btn" onClick={goUp} disabled={!canGoUp} aria-label="Up">
              <ChevronUp />
            </button>
            <button className="tool-btn refresh-btn" onClick={() => refresh()} aria-label="Refresh">
              <RefreshGlyph />
            </button>
          </div>
          <div className="toolbar-title">
            {showMyComputer ? "My Computer" : crumbs.length ? crumbs[crumbs.length - 1].label : "System"}
          </div>
          <button className="tool-btn wide-btn cluster-start" onClick={createNewFile}>
            + File
          </button>
          <button className="tool-btn wide-btn" onClick={createNewFolder}>
            + Folder
          </button>
          <button className="tool-btn wide-btn" onClick={paste} disabled={!canPaste}>
            Paste
          </button>
          <button className="tool-btn view-menu-btn" aria-label="View options" title="View options" onClick={openViewMenu}>
            {view === "icon" ? (
              <IconViewGlyph />
            ) : view === "list" ? (
              <ListViewGlyph />
            ) : view === "column" ? (
              <ColumnViewGlyph />
            ) : (
              <ListPreviewGlyph />
            )}
            <ChevronDown size={12} />
          </button>
          <button
            className="tool-btn"
            aria-label="Preferences"
            title="Preferences"
            onClick={() => setSettingsOpen(true)}
          >
            <SettingsGlyph />
          </button>
          {searchExpanded || searchQuery ? (
            <div className="search-field">
              <SearchGlyph />
              <input
                ref={searchInputRef}
                autoFocus
                placeholder={inVault ? "Search in vault" : "Search this folder"}
                value={searchQuery}
                onChange={(e) => {
                  setSearchQuery(e.target.value);
                  if (e.target.value.trim() === "") setSearchResults(null);
                }}
                onKeyDown={(e) => e.key === "Enter" && runSearch(searchQuery)}
                onBlur={() => {
                  if (searchQuery.trim() === "") setSearchExpanded(false);
                }}
              />
              {searchQuery && (
                <button
                  className="search-clear"
                  aria-label="Clear search"
                  onClick={() => {
                    setSearchQuery("");
                    setSearchResults(null);
                    setSearchExpanded(false);
                  }}
                >
                  ✕
                </button>
              )}
            </div>
          ) : (
            <button
              className="tool-btn search-toggle"
              aria-label="Search"
              title="Search"
              onClick={() => setSearchExpanded(true)}
            >
              <SearchGlyph size={18} />
            </button>
          )}
        </div>
        {error && (
          <div className="error-bar" onClick={() => setError("")}>
            {error} <span className="error-x">✕</span>
          </div>
        )}
        {infoMsg && (
          <div className="info-bar" onClick={() => setInfoMsg("")}>
            {infoMsg} <span className="error-x">✕</span>
          </div>
        )}

        <div
          className="content"
          ref={contentRef}
          onContextMenu={showMyComputer ? undefined : backgroundMenu}
          onMouseDown={onContentMouseDown}
        >
          {showMyComputer ? (
            <MyComputerView
              drives={drives}
              error={drivesError}
              onOpenDrive={(d) => d.mountpoint && go({ kind: "fs", path: d.mountpoint })}
              onMenu={driveMenu}
            />
          ) : searchResults !== null ? (
            <SearchResults
              query={searchQuery}
              results={searchResults}
              onOpen={(p) => {
                if (loc.kind === "vault") go({ kind: "vault", root: loc.root, rel: parentPath(p) });
                else go({ kind: "fs", path: parentPath(p) });
              }}
            />
          ) : view === "column" ? (
            <ColumnView
              chain={columnChain()}
              list={(dir) => listDir(dir, loc.kind)}
              inVault={inVault}
              root={inVault ? loc.root : undefined}
              onActivate={activate}
              onMenu={entryMenu}
              previewEntry={previewEntry}
              onSelectFile={(dir, entry) => withSensitive(joinPath(dir, entry.name), () => setPreviewEntry({ dir, entry }))}
              cutPaths={clipboard?.mode === "cut" && clipboard.kind === loc.kind ? clipboard.paths : undefined}
            />
          ) : view === "listPreview" ? (
            <div className="list-preview-split">
              {renderListBody()}
              <FilePreviewPane
                target={previewEntry}
                inVault={inVault}
                root={inVault ? loc.root : undefined}
                onRename={renamePreviewEntry}
              />
            </div>
          ) : (
            renderListBody()
          )}
        </div>

        <div
          className="breadcrumb-bar"
          onClick={(e) => {
            if (showMyComputer) return;
            if (!(e.target as HTMLElement).closest(".crumb, .breadcrumb-copy")) beginEditPath();
          }}
        >
          {showMyComputer ? (
            <div className="breadcrumb-crumbs">
              <span className="crumb-group">
                <span className="crumb">My Computer</span>
              </span>
            </div>
          ) : editingPath ? (
            <input
              autoFocus
              className="breadcrumb-path-input"
              value={pathInput}
              onChange={(e) => setPathInput(e.target.value)}
              onFocus={(e) => e.currentTarget.select()}
              onBlur={() => setEditingPath(false)}
              onKeyDown={(e) => {
                if (e.key === "Enter") submitEditPath();
                if (e.key === "Escape") setEditingPath(false);
              }}
            />
          ) : (
            <div className="breadcrumb-crumbs">
              {hiddenCrumbs.length > 0 && (
                <span className="crumb-group">
                  <span
                    className="crumb crumb-ellipsis"
                    onClick={() => go(hiddenCrumbs[hiddenCrumbs.length - 1].loc)}
                  >
                    …
                  </span>
                  <span className="crumb-sep">›</span>
                </span>
              )}
              {visibleCrumbs.map((c, i) => (
                <span key={c.key} className="crumb-group">
                  {i > 0 && <span className="crumb-sep">›</span>}
                  <span
                    className={`crumb ${dropTarget === "crumb:" + c.key ? "drop" : ""}`}
                    onClick={() => go(c.loc)}
                    onDragOver={(e) => {
                      e.preventDefault();
                      setDropTarget("crumb:" + c.key);
                    }}
                    onDragLeave={() => setDropTarget((t) => (t === "crumb:" + c.key ? null : t))}
                    onDrop={(e) => {
                      e.preventDefault();
                      // only allow drop when the crumb is in the current space
                      if ((loc.kind === "vault") === (c.loc.kind === "vault")) dropInto(c.dropDir);
                    }}
                  >
                    <span className="crumb-icon">
                      {c.lock ? (
                        <LockOpenGlyph size={13} />
                      ) : i === 0 && hiddenCrumbs.length === 0 ? (
                        <DiskGlyph size={13} />
                      ) : (
                        <PlaceGlyph size={13} />
                      )}
                    </span>
                    {c.label}
                  </span>
                </span>
              ))}
            </div>
          )}
          {!showMyComputer && (
            <span
              className={`breadcrumb-copy ${pathCopied ? "copied" : ""}`}
              title="Copy absolute path"
              onClick={(e) => {
                e.stopPropagation();
                copyPath();
              }}
            >
              {pathCopied ? <CheckGlyph size={12} /> : <CopyGlyph size={12} />}
            </span>
          )}
        </div>

        <div className="statusbar">
          <span className="status-count">
            {selected.size > 0
              ? (() => {
                  const size = [...selected].reduce((sum, name) => {
                    const en = entries.find((e) => e.name === name);
                    return sum + (en && !en.is_dir ? en.size : 0);
                  }, 0);
                  const base = `${selected.size} of ${entries.length} selected — ${formatSize(size)}`;
                  if (selected.size !== 1) return base;
                  const only = entries.find((e) => e.name === [...selected][0]);
                  return only && !only.is_dir ? `${base} — ${formatDate(only.mtime)}` : base;
                })()
              : `${entries.length} ${entries.length === 1 ? "item" : "items"}`}
          </span>
          <ProgressPanel ops={progressOps} onCancel={cancelProgress} />
          <span className="status-loc">{inVault ? "🔒 Encrypted Vault" : "File System"}</span>
        </div>
      </div>

      {marquee && (
        <div
          className="marquee"
          style={{
            left: Math.min(marquee.x0, marquee.x1),
            top: Math.min(marquee.y0, marquee.y1),
            width: Math.abs(marquee.x1 - marquee.x0),
            height: Math.abs(marquee.y1 - marquee.y0),
          }}
        />
      )}

      {pending && pending.kind === "unlock" && (
        <UnlockSheet
          name={pending.name}
          error={sheetError}
          onCancel={() => {
            pendingNav.current = null;
            setPending(null);
            setSheetError("");
          }}
          onSubmit={submitUnlock}
        />
      )}
      {sensitivePrompt && (
        <SensitiveUnlockSheet
          name={baseName(sensitivePrompt.path)}
          error={sensitivePrompt.error}
          onCancel={() => setSensitivePrompt(null)}
          onSubmit={submitSensitive}
        />
      )}
      {pending && pending.kind === "newVault" && (
        <NewVaultSheet
          error={sheetError}
          onCancel={() => {
            setPending(null);
            setSheetError("");
          }}
          onSubmit={submitNewVault}
        />
      )}
      {settingsOpen && (
        <SettingsSheet
          settings={appSettings}
          onChange={setAppSettings}
          onClose={() => setSettingsOpen(false)}
          mobile={mobile}
        />
      )}
      {reauthPrompt && (
        <ReauthOverlay
          name={reauthPrompt.name}
          error={reauthError}
          onSubmit={submitReauth}
          onLockInstead={() => {
            setReauthPrompt(null);
            lockCurrentVault();
          }}
        />
      )}
      {pending && pending.kind !== "unlock" && pending.kind !== "newVault" && (
        <ActionSheet action={pending} onCancel={() => setPending(null)} onSubmit={submitPending} />
      )}
      {compressTarget && (
        <CompressOptionsSheet
          defaultName={
            compressTarget.length === 1 ? `${compressTarget[0]}.zip` : "Archive.zip"
          }
          allowTargz={!inVault}
          onCancel={() => setCompressTarget(null)}
          onSubmit={(opts) => compressWithOptions(compressTarget, opts)}
        />
      )}
      {zipPasswordPrompt && (
        <ZipPasswordSheet
          name={zipPasswordPrompt.entry.name}
          error={zipPasswordPrompt.error}
          onCancel={() => setZipPasswordPrompt(null)}
          onSubmit={(pw) => decompressEntry(zipPasswordPrompt.entry, pw)}
        />
      )}
      {archiveMountPrompt && (
        <ZipPasswordSheet
          name={archiveMountPrompt.entry.name}
          error={archiveMountPrompt.error}
          onCancel={() => setArchiveMountPrompt(null)}
          onSubmit={(pw) => mountArchive(archiveMountPrompt.dir, archiveMountPrompt.entry, pw)}
        />
      )}
      {encryptTarget && (
        <EncryptFileSheet
          name={encryptTarget.name}
          isFolder={encryptTarget.is_dir}
          onCancel={() => setEncryptTarget(null)}
          onSubmit={(pw) =>
            encryptTarget.is_dir ? encryptFolder(encryptTarget, pw) : encryptFile(encryptTarget, pw)
          }
        />
      )}
      {decryptPrompt && (
        <ZipPasswordSheet
          name={decryptPrompt.entry.name}
          error={decryptPrompt.error}
          onCancel={() => setDecryptPrompt(null)}
          onSubmit={(pw) =>
            decryptPrompt.mode === "inplace"
              ? decryptInPlace(decryptPrompt.entry, pw)
              : decryptAndOpen(decryptPrompt.entry, pw)
          }
        />
      )}
      {iconTarget && (
        <ChangeIconSheet
          name={favLabel(iconTarget)}
          current={customIcons[iconTarget]}
          onCancel={() => setIconTarget(null)}
          onSubmit={(icon) => {
            setCustomIcons((prev) => {
              const next = { ...prev };
              if (icon) next[iconTarget] = icon;
              else delete next[iconTarget];
              return next;
            });
            setIconTarget(null);
          }}
        />
      )}
      {driveTarget && (
        <DriveSyncSheet
          localPath={driveTarget.path}
          provider={driveTarget.provider}
          onClose={() => {
            setDriveTarget(null);
            refreshSyncStatus();
          }}
        />
      )}
      {gitSyncTarget && (
        <GitSyncSheet
          localPath={gitSyncTarget}
          onClose={() => {
            setGitSyncTarget(null);
            refreshSyncStatus();
          }}
        />
      )}
      {localSyncTarget && (
        <LocalSyncSheet
          folderA={localSyncTarget}
          onClose={() => {
            setLocalSyncTarget(null);
            refreshSyncStatus();
          }}
        />
      )}
      {syncthingTarget && (
        <SyncthingSheet folderA={syncthingTarget} onClose={() => setSyncthingTarget(null)} />
      )}
      {incomingDevice && (
        <div className="sheet-overlay" onMouseDown={() => setIncomingDevice(null)}>
          <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
            <h3>Pair with “{incomingDevice.name}”?</h3>
            <p className="hint">
              This link came from another device for P2P sync. Its ID:
            </p>
            <div className="info-row">
              <span className="info-path" title={incomingDevice.id}>
                {incomingDevice.id}
              </span>
            </div>
            <div className="sheet-actions">
              <button className="btn-plain" onClick={() => setIncomingDevice(null)}>
                Cancel
              </button>
              <button className="btn-primary" onClick={confirmIncomingDevice}>
                Pair
              </button>
            </div>
          </div>
        </div>
      )}
      {machineInfoOpen && <MachineInfoSheet onClose={() => setMachineInfoOpen(false)} />}
      {formatTarget && (
        <FormatDriveSheet
          drive={formatTarget}
          onClose={() => setFormatTarget(null)}
          onFormatted={() => {
            setFormatTarget(null);
            refreshDrives();
          }}
        />
      )}
      {manageTemplatesOpen && (
        <ManageTemplatesSheet
          templates={templates}
          onRemove={(id) => setTemplates((prev) => prev.filter((t) => t.id !== id))}
          onClose={() => setManageTemplatesOpen(false)}
        />
      )}
      {convertTarget && (
        <ConvertSheet
          name={convertTarget.entry.name}
          targetLabel={convertTarget.targetLabel}
          mode={convertTarget.mode}
          onCancel={() => setConvertTarget(null)}
          onSubmit={(value) => {
            const { entry, targetExt, mode } = convertTarget;
            setConvertTarget(null);
            if (mode === "imageQuality") {
              runImageConvert(entry, targetExt, Number(value));
            } else {
              runMediaConvert(entry, targetExt, value as "high" | "medium" | "low");
            }
          }}
        />
      )}
      {resizeTarget && (
        <ResizeSheet
          count={resizeTarget.length}
          onCancel={() => setResizeTarget(null)}
          onSubmit={(w, h) => {
            const names = resizeTarget;
            setResizeTarget(null);
            runResize(names, w, h);
          }}
        />
      )}
      {montageTarget && (
        <MontageOptionsSheet
          imageCount={montageTarget.imageCount}
          videoCount={montageTarget.videoCount}
          hasAudioTrack={!!montageTarget.audio}
          onCancel={() => setMontageTarget(null)}
          onSubmit={(opts) => {
            const target = montageTarget;
            setMontageTarget(null);
            runMontage(target, opts);
          }}
        />
      )}
      {gitStatusOpen && gitRoot && (
        <GitStatusSheet root={gitRoot} status={gitStatus} onClose={() => setGitStatusOpen(false)} />
      )}
      {unfreezeTarget && (
        <UnfreezeSheet
          path={unfreezeTarget}
          onDone={() => {
            setUnfreezeTarget(null);
            refreshFrozen();
            refresh();
          }}
          onClose={() => setUnfreezeTarget(null)}
        />
      )}
      {infoTarget && (
        <GetInfoSheet
          entry={infoTarget.entry}
          fullPath={infoTarget.fullPath}
          root={infoTarget.root}
          kind={infoTarget.kind}
          customIcon={infoTarget.kind === "fs" ? customIcons[infoTarget.fullPath] : undefined}
          onChangeIcon={() => {
            setInfoTarget(null);
            setIconTarget(infoTarget.fullPath);
          }}
          onClose={() => setInfoTarget(null)}
        />
      )}
      {multiInfoTarget && (
        <MultiInfoSheet
          names={multiInfoTarget}
          entries={entries}
          curDir={curDir}
          kind={loc.kind}
          onClose={() => setMultiInfoTarget(null)}
        />
      )}
      <ContextMenu state={menu} onClose={() => setMenu(null)} />
      {shareStatus && (
        <div className={`share-toast ${shareStatus.state}`}>
          {shareStatus.state === "working" && (
            <>
              <span className="share-toast-spinner" />
              Sharing “{shareStatus.label}”…
            </>
          )}
          {shareStatus.state === "done" && (
            <>
              <span className="share-toast-check">✓</span>
              <span>
                <strong>Copied to clipboard</strong> — ready to paste and send
                {shareStatus.message && <span className="share-toast-url"> · {shareStatus.message}</span>}
              </span>
            </>
          )}
          {shareStatus.state === "error" && (
            <>
              Share failed: {shareStatus.message}
              <button className="share-toast-dismiss" onClick={() => setShareStatus(null)}>
                ✕
              </button>
            </>
          )}
        </div>
      )}
      </div>
    </>
  );
}

export default function App() {
  const [home, setHome] = useState<string | null>(null);
  useEffect(() => {
    api.homeDir().then(setHome).catch(() => setHome("/"));
  }, []);

  const params = new URLSearchParams(window.location.search);
  const pickerMode = params.get("picker");
  if (pickerMode === "open" || pickerMode === "save") {
    const rawFilters = params.get("filters");
    let initialFilters: { name: string; patterns: string[] }[] = [];
    if (rawFilters) {
      try {
        initialFilters = JSON.parse(rawFilters);
      } catch {
        /* malformed filters option -- fall back to no format picker */
      }
    }
    return (
      <PickerView
        mode={pickerMode}
        reqId={params.get("reqid") ?? ""}
        multiple={params.get("multiple") === "true"}
        initialName={params.get("name")}
        initialFilters={initialFilters}
        initialFolder={params.get("folder")}
        directory={params.get("directory") === "true"}
      />
    );
  }

  return (
    <div className="app-window">
      {home === null ? (
        <>
          <TitleBar />
          <div className="boot" />
        </>
      ) : (
        <Explorer home={home} />
      )}
      <ResizeHandles />
    </div>
  );
}

// `decorations: false` means no OS-drawn border for the window manager to
// hit-test resize drags against, and the webview's own `cursor: default`
// otherwise wins right up to the true edge -- these thin strips are what
// make hovering the window's border actually show a resize cursor and
// grab-able at all, kicking off a real native resize via
// `startResizeDragging` on mousedown.
function ResizeHandles() {
  // No floating, user-resizable window on Android/iOS.
  const [mobile, setMobile] = useState(false);
  useEffect(() => {
    api.isMobilePlatform().then(setMobile).catch(() => {});
  }, []);
  const win = getCurrentWebviewWindow();
  if (mobile) return null;
  function handle(
    dir: "East" | "North" | "NorthEast" | "NorthWest" | "South" | "SouthEast" | "SouthWest" | "West",
    className: string
  ) {
    return (
      <div
        className={`resize-handle ${className}`}
        onMouseDown={(e) => {
          if (e.button !== 0) return;
          win.startResizeDragging(dir).catch(() => {});
        }}
      />
    );
  }
  return (
    <>
      {handle("North", "n")}
      {handle("South", "s")}
      {handle("East", "e")}
      {handle("West", "w")}
      {handle("NorthEast", "ne")}
      {handle("NorthWest", "nw")}
      {handle("SouthEast", "se")}
      {handle("SouthWest", "sw")}
    </>
  );
}
