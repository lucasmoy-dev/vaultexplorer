import { useEffect, useState, useCallback, useMemo, useRef } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrent as getCurrentDeepLinks, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { onBackButtonPress } from "@tauri-apps/api/app";
import { open as pickPath } from "@tauri-apps/plugin-dialog";
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
  osOpen,
  PlayerItem,
} from "./api";
import { TitleBar, TrafficLights } from "./TitleBar";
import { ContextMenu, MenuState, MenuItem } from "./ContextMenu";
import { MediaViewer, GalleryEntry } from "./components/media/MediaViewer";
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
  SmartphoneGlyph,
  CopyGlyph,
  CheckGlyph,
  TrashGlyph,
  GitBranchGlyph,
  CloudSyncGlyph,
  LocalSyncGlyph,
  SettingsGlyph,
  NewFileGlyph,
  NewFolderGlyph,
  PasteGlyph,
  RetryImg,
  kindOf,
  Kind,
  customIconUrl,
  CUSTOM_ICON_PREFIX,
  symbolIconSvg,
} from "./icons";
import { Loc, Clipboard, View, ProgressOp, PendingAction, VaultCreateOptions, SensitiveTimeout } from "./types";
import { ProgressPanel } from "./components/ProgressPanel";
import { kindLabel, editorExtOf } from "./entryHelpers";
import { EntryTile } from "./components/EntryTile";
import { MyComputerView } from "./components/MyComputerView";
import { InternetView, SavedInternetSearch, InternetDownloadItem } from "./components/InternetView";
import { SavedSearchDigest } from "./components/SavedSearchDigest";
import { SearchResults } from "./components/SearchResults";
import { FilePreviewPane, TextEditorPane } from "./components/TextEditorPane";
import { NotesGrid } from "./components/NotesGrid";
import { LibraryShelf } from "./components/LibraryShelf";
import { ContactsGrid, ContactEditForm } from "./components/ContactsGrid";
import { serializeVCard, emptyVCard } from "./vcard";
import { ColumnView } from "./components/ColumnView";
import { PickerView } from "./components/PickerView";
import { PlayerWindow } from "./components/PlayerWindow";
import { MediaWindow } from "./components/MediaWindow";
import { FreeUpSpaceView } from "./components/FreeUpSpaceView";
import { DeviceView } from "./components/DeviceView";
import { ReorganizeSheet } from "./components/sheets/reorganize-sheet";
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
  VaultSettingsSheet,
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
  SettingsScreen,
} from "./components/sheets/system-sheets";
import { GetInfoSheet, MultiInfoSheet } from "./components/sheets/info-sheets";
import { OpenWithSheet } from "./components/sheets/open-with-sheet";
import { readText as clipboardReadText } from "@tauri-apps/plugin-clipboard-manager";
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

// A home-screen shortcut's pinned Intent can only carry a plain URL, not a
// live `Loc` object -- this is the deep link that round-trips one, both
// directions. Mirrors `vaultexplorer://add-device` (sync-sheets.tsx) but
// for "open this folder", not pairing.
function buildOpenFolderLink(loc: Loc): string {
  const params =
    loc.kind === "vault"
      ? new URLSearchParams({ kind: "vault", root: loc.root, rel: loc.rel })
      : new URLSearchParams({ kind: "fs", path: loc.path });
  return `vaultexplorer://open-folder?${params.toString()}`;
}
function parseOpenFolderLink(url: string): Loc | null {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "vaultexplorer:" || parsed.hostname !== "open-folder") return null;
    const kind = parsed.searchParams.get("kind");
    if (kind === "vault") {
      const root = parsed.searchParams.get("root");
      if (!root) return null;
      return { kind: "vault", root, rel: parsed.searchParams.get("rel") || "" };
    }
    const path = parsed.searchParams.get("path");
    if (!path) return null;
    return { kind: "fs", path };
  } catch {
    return null;
  }
}
// Stable per-folder id for the pinned shortcut (Android scopes shortcut
// IDs per-package): a short, non-cryptographic hash of the deep link is
// enough to make re-adding the same folder update its existing pin
// instead of piling up duplicates, with no server/crypto round-trip.
function hashForShortcutId(s: string): string {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (Math.imul(h, 31) + s.charCodeAt(i)) | 0;
  return `folder-${(h >>> 0).toString(36)}`;
}

// A folder's custom icon is either an emoji (plain string) or a bundled
// WhiteSur icon ("ws:<key>", see icons.tsx) -- only the emoji case can
// become an actual bitmap for a pinned shortcut's `Icon.createWithBitmap`
// entirely on the JS side (rendering it to a canvas); a WhiteSur icon is a
// frontend-bundled SVG asset with no plain native filesystem path for the
// JNI side to read, so that case just falls back to the app's own icon
// (see `android_pin_folder_shortcut`) rather than not shipping this at all.
function renderEmojiIconPng(emoji: string): string | undefined {
  const size = 108; // Android's adaptive-icon canvas size
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d");
  if (!ctx) return undefined;
  ctx.font = `${Math.round(size * 0.62)}px sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(emoji, size / 2, size / 2 + size * 0.05);
  const dataUrl = canvas.toDataURL("image/png");
  const comma = dataUrl.indexOf(",");
  return comma < 0 ? undefined : dataUrl.slice(comma + 1);
}

// Mobile's in-app editor covers exactly the two extensions desktop's own
// listPreview always treats as plain text (see `editorExtOf` -- it
// deliberately returns `null`, not a toggleable entry, for these same
// two), not the wider user-configurable `textEditorExts` set -- keeps the
// mobile handoff decision a plain yes/no instead of needing that whole
// per-extension picker UI to exist on a phone too.
function isPlainTextEntry(entry: Entry): boolean {
  if (entry.is_dir) return false;
  const dot = entry.name.lastIndexOf(".");
  if (dot <= 0) return false;
  const ext = entry.name.slice(dot + 1).toLowerCase();
  return ext === "txt" || ext === "md" || ext === "markdown";
}

// Same "N results for query" line SearchResults.tsx shows for the
// generic case, reused as the `header` slot of whichever view-specific
// grid is rendering the results instead.
function searchResultsHeader(query: string, count: number) {
  return (
    <div className="search-header">
      <span>
        {count} {count === 1 ? "result" : "results"} for “{query}”
      </span>
    </div>
  );
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
    let cancelled = false;
    // A cold start can race the IPC bridge (seen in practice on Android,
    // worse on a freshly-booted/cold emulator than a warm one: the very
    // first invoke can keep rejecting for several seconds before the
    // bridge finishes attaching). This result is a compile-time constant
    // (`cfg!(mobile)`), never actually false-negative on retry, so giving
    // up too early meant `mobile` got permanently stuck at its `false`
    // default: the sidebar defaults to the *desktop* shape (My Computer
    // visible, no hamburger, docked layout) on a phone for the rest of
    // that session, with no user-visible error to explain why. Keeps
    // retrying every 500ms for up to ~20s -- comfortably past the worst
    // cold-start delay seen -- before actually giving up; each attempt is
    // a single cheap round-trip with nothing else waiting on it.
    async function detect() {
      for (let i = 0; i < 40 && !cancelled; i++) {
        if (i) await new Promise((r) => setTimeout(r, 500));
        try {
          const v = await api.isMobilePlatform();
          if (cancelled) return;
          setMobile(v);
          document.documentElement.classList.toggle("is-mobile", v);
          return;
        } catch {
          // keep retrying
        }
      }
    }
    detect();
    return () => {
      cancelled = true;
    };
  }, []);
  // Touch has no ⌘/⇧-click, so multi-select needs its own explicit mode:
  // entered via "Select" in the long-press menu (see entryMenu), then a
  // plain tap toggles membership instead of opening/activating. The
  // auto-exit-when-empty effect is below, once `selected` (from
  // useSelection()) is actually in scope.
  const [selectionMode, setSelectionMode] = useState(false);
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
    mobileExternalEditor: boolean;
  }>(() => {
    const defaults = {
      showHiddenFiles: false,
      hideExtensions: false,
      terminalApp: "ghostty",
      newFileNameTemplate: "{datetime}",
      newFolderNameTemplate: "untitled folder",
      theme: "system" as const,
      sensitiveTimeout: 1200 as SensitiveTimeout,
      mobileExternalEditor: false,
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
    const handle = (url: string) => {
      const device = parseAddDeviceLink(url);
      if (device) {
        setIncomingDevice(device);
        return;
      }
      // A tap on a pinned home-screen folder shortcut (see
      // `addFolderShortcut`) arrives the same way -- launch args on
      // Linux/Windows, a live `onOpenUrl` event on Android.
      const folder = parseOpenFolderLink(url);
      if (folder) go(folder);
    };
    getCurrentDeepLinks()
      .then((urls) => {
        for (const url of urls ?? []) handle(url);
      })
      .catch(() => {});
    onOpenUrl((urls) => {
      for (const url of urls) handle(url);
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
  // An Actions row for work that reports no percentage -- a long external
  // run (Reorganize & Clean) that used to be invisible once its sheet was
  // closed. Returns the "it finished" callback.
  function beginIndeterminate(label: string): () => void {
    const id = ++progressIdRef.current;
    setProgressOps((prev) => [...prev, { id, label, done: 0, total: 0, status: "running" }]);
    return () => setProgressOps((prev) => prev.filter((p) => p.id !== id));
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
  // `dir` defaults to the folder on screen; search results pass their own,
  // since a hit can live anywhere under the search root.
  async function shareFile(entry: Entry, dir?: string) {
    const full = joinPath(dir ?? curDir, entry.name);
    setShareStatus({ label: entry.name, state: "working" });
    try {
      const url = inVault ? await api.vaultShareFile(full) : await api.fsShareFile(full);
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
  // "Vault Settings…" (any vault folder's context menu) -- editing the
  // same options NewVaultSheet's Advanced section sets at creation time,
  // for a vault that already exists. `canAutoUnlock` false for a nested
  // vault opened while browsing inside another vault (its FUSE path is
  // per-session, so it can't be keyed for startup auto-unlock).
  const [vaultSettingsTarget, setVaultSettingsTarget] = useState<{
    root: string;
    canAutoUnlock: boolean;
  } | null>(null);
  async function saveVaultSettings(
    root: string,
    opts: VaultCreateOptions,
    password: string | null
  ) {
    if (opts.autoUnlock && password !== null) {
      // Verify before persisting -- storing a mistyped password would just
      // silently fail every future auto-unlock attempt instead of erroring
      // now, while the user can still fix it.
      await api.verifyVaultPassword(root, password);
      await api.setVaultAutoUnlock(root, password);
    } else if (!opts.autoUnlock) {
      await api.clearVaultAutoUnlock(root).catch(() => {});
    }
    setVaultSettings((prev) => ({ ...prev, [root]: opts }));
    setVaultSettingsTarget(null);
  }
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
  const [reorganizeTarget, setReorganizeTarget] = useState<string | null>(null);
  const [freeUpSpaceOpen, setFreeUpSpaceOpen] = useState(false);
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
    let prevVerifying = new Set<string>();
    // Drive's background auto-sync loop is entirely best-effort (same as
    // git/local sync's loops) -- a pair stuck failing every tick (e.g.
    // rclone's own "too many deletes" safety abort) would otherwise fail
    // silently forever with nothing ever telling the user. Tracked per
    // path so the same failure doesn't re-show the banner every poll.
    const shownDriveErrors = new Map<string, string>();
    const shownGitSyncErrors = new Map<string, string>();
    // Background sync/verify activity also shows as rows in the bottom-right
    // progress panel, same as any user-started operation -- synthetic ops
    // (no Channel, no cancel), keyed per path so a row updates in place.
    const taskIds = new Map<string, number>();
    function beginTask(key: string, label: string) {
      if (taskIds.has(key)) {
        // Refresh the label in place -- the sync row narrates the current
        // file as the pass moves through them.
        const id = taskIds.get(key)!;
        setProgressOps((ops) => ops.map((p) => (p.id === id && p.label !== label ? { ...p, label } : p)));
        return;
      }
      const id = ++progressIdRef.current;
      taskIds.set(key, id);
      setProgressOps((ops) => [...ops, { id, label, done: 0, total: 1, status: "running" }]);
    }
    function endTask(key: string) {
      const id = taskIds.get(key);
      if (id == null) return;
      taskIds.delete(key);
      // Show 100% for a beat before dropping the row, beginProgress's rhythm.
      setProgressOps((ops) => ops.map((p) => (p.id === id ? { ...p, done: 1, total: 1 } : p)));
      setTimeout(() => setProgressOps((ops) => ops.filter((p) => p.id !== id)), 1000);
    }
    function poll() {
      Promise.all([
        api.gitSyncSyncingNow().catch(() => []),
        api.localSyncSyncingNow().catch(() => []),
        api.driveSyncingNow().catch(() => []),
        api.syncthingSyncingNow().catch(() => []),
        api.driveVerifyingNow().catch(() => []),
        api.driveSyncActivity().catch(() => ({}) as Record<string, { current: string | null; count: number }>),
      ]).then((results) => {
        if (cancelled) return;
        const verifying = new Set(results[4] as string[]);
        const activity = results[5] as Record<string, { current: string | null; count: number }>;
        const next = new Set((results.slice(0, 4) as string[][]).flat());
        const justFinished = [...prev].filter((p) => !next.has(p));
        for (const p of next) {
          // Narrate the specific file mid-transfer when the backend can name
          // it (decrypted for unlocked vaults); fall back to the folder.
          const act = activity[p];
          const label = act?.current
            ? `Syncing "${baseName(act.current)}"`
            : act?.count
            ? `Syncing "${baseName(p)}" · ${act.count} ${act.count === 1 ? "file" : "files"}`
            : `Syncing "${baseName(p)}"`;
          beginTask(`sync:${p}`, label);
        }
        for (const p of prev) if (!next.has(p)) endTask(`sync:${p}`);
        for (const p of verifying)
          if (!prevVerifying.has(p)) beginTask(`verify:${p}`, `Verifying "${baseName(p)}" in cloud`);
        for (const p of prevVerifying) if (!verifying.has(p)) endTask(`verify:${p}`);
        prevVerifying = verifying;
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
      for (const path of gitSyncedPaths) {
        api
          .gitSyncLastError(path)
          .then((err) => {
            if (cancelled) return;
            if (err && shownGitSyncErrors.get(path) !== err) {
              shownGitSyncErrors.set(path, err);
              setError(`Git sync failed for "${baseName(path)}": ${err}`);
            } else if (!err) {
              shownGitSyncErrors.delete(path);
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
  }, [driveSyncedPaths, gitSyncedPaths]);
  const [sortKey, setSortKey] = useState<"name" | "date" | "size" | "kind" | "created">("name");
  const [sortDir, setSortDir] = useState<1 | -1>(1);
  const selection = useSelection();
  const { selected, setSelected, lastClicked, setLastClicked, selectOnly, toggle, selectRange: selectRangeByNames } =
    selection;
  // See the `selectionMode` declaration above -- exits on its own once
  // nothing is left selected, same convention as Google Files/Photos, so
  // there's no separate "still in select mode with 0 picked" limbo state.
  useEffect(() => {
    if (selectionMode && selected.size === 0) setSelectionMode(false);
  }, [selectionMode, selected]);
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
  // Absolute path whose "Other Application…" picker is open (null = closed).
  const [openWithTarget, setOpenWithTarget] = useState<string | null>(null);

  // Extensions the user has decided to open in the text editor, even though
  // `kindOf` doesn't call them "text". A `.js` is kind "code", so the preview
  // pane showed it as a read-only info panel with no way to edit it -- the
  // "Edit" button there adds its extension here, which switches this file to
  // the editor immediately and every later file of that format too. Undone
  // from Get Info (the per-format switch lives there, since it's a property
  // of the format rather than of the file you happen to have selected).
  // Stored lowercase, without the dot.
  const [textEditorExts, setTextEditorExts] = useState<Set<string>>(() => {
    try {
      const raw = localStorage.getItem("vaultexplorer:text-editor-exts");
      if (raw) return new Set(JSON.parse(raw) as string[]);
    } catch {
      /* ignore */
    }
    return new Set();
  });
  useEffect(() => {
    localStorage.setItem("vaultexplorer:text-editor-exts", JSON.stringify([...textEditorExts]));
  }, [textEditorExts]);
  function setExtOpensInEditor(ext: string, on: boolean) {
    const key = ext.toLowerCase();
    if (!key) return;
    setTextEditorExts((prev) => {
      if (prev.has(key) === on) return prev;
      const next = new Set(prev);
      if (on) next.add(key);
      else next.delete(key);
      return next;
    });
  }
  const [error, setErrorRaw] = useState("");
  // Every one of the ~60 call sites across this file just calls
  // `setError(String(e))` -- wrapping the setter here (instead of editing
  // each one) is what lets a raw OS error like "Permission denied (os
  // error 13)" get a plain-language explanation attached without having to
  // find every place it could surface. Android-specific because on
  // desktop the same OS message usually means a real Unix permission
  // (sudo-only file, etc.), not this sandboxing wall.
  function setError(msg: string) {
    if (mobile && /permission denied/i.test(msg)) {
      setErrorRaw(
        `${msg} -- Android blocks apps from most folders outside your own phone storage, even with "All files access" granted. This isn't something the app can unlock.`
      );
    } else {
      setErrorRaw(msg);
    }
  }
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
  const [mediaViewer, setMediaViewer] = useState<{ gallery: GalleryEntry[]; startIndex: number } | null>(null);
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
  // The in-app text/markdown editor, full-screen rather than desktop's
  // split listPreview pane -- there's no room for a split on a phone.
  // Originally just mobile's plain file-open, now also what a note card
  // in Notes view opens into on either platform (see NotesGrid/`view ===
  // "notes"` below) -- the "mobile" in the name is a holdover, not a
  // remaining restriction on who can reach it.
  const [mobileEditorTarget, setMobileEditorTarget] = useState<{
    entry: Entry;
    fullPath: string;
    inVault: boolean;
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
  // "My Device" is the capacity dashboard (disks, RAM, uptime); "My
  // Computer" stays the drive *browser*. One entry was doing both and
  // neither well.
  const [showDevice, setShowDevice] = useState(false);
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
    setShowInternet(false);
    setShowDevice(false);
    setFreeUpSpaceOpen(false);
    setSearchResults(null);
    refreshDrives();
  }

  // "Internet" (desktop-only experiment): same non-fs-path sidebar-entry
  // pattern as "My Computer" above -- swaps the content area for
  // `InternetView`, which owns its own Videos/Images sub-navigation and
  // search state entirely by itself (see that component).
  const [showInternet, setShowInternet] = useState(false);
  // Set only when a `.ytsearch`/`.imgsearch` file was double-clicked (see
  // `activate()`) -- tells InternetView to skip its root tiles and rerun
  // that exact saved search immediately. Cleared on a plain sidebar open
  // so that always lands on the root tiles instead of replaying whatever
  // was last opened.
  const [internetInitial, setInternetInitial] = useState<SavedInternetSearch | null>(null);
  function openInternet() {
    setShowInternet(true);
    setShowMyComputer(false);
    setFreeUpSpaceOpen(false);
    setShowDevice(false);
    setSearchResults(null);
    setInternetInitial(null);
  }
  function openInternetSearchFile(saved: SavedInternetSearch) {
    setShowInternet(true);
    setShowMyComputer(false);
    setFreeUpSpaceOpen(false);
    setShowDevice(false);
    setSearchResults(null);
    setInternetInitial(saved);
  }
  // Writes a saved search straight into curDir (wherever the user was
  // browsing before opening Internet) rather than handing off to the OS's
  // native save dialog -- the whole point of a saved search being a real
  // file is staying inside the app's own filesystem view; a native picker
  // just for this one write would undercut that. Organizing it into a
  // different folder afterward is the same cut/paste or drag the user
  // already has for any other file.
  async function saveInternetSearch(filename: string, content: string): Promise<string> {
    const name = uniqueName(filename);
    const path = joinPath(curDir, name);
    await api.fsWriteText(path, content);
    await refresh();
    // Drop the Internet overlay back to the folder underneath -- curDir
    // never moved while it was open (see the Back/Forward comment further
    // down) -- so the saved file is immediately visible, selected and
    // scrolled into view, instead of the user having to back out and go
    // find it themselves.
    setShowInternet(false);
    selectAndReveal(name);
    return path;
  }
  // A folder holding nothing but .ytsearch (or nothing but .imgsearch, or
  // nothing but .booksearch) files gets an auto-preview digest instead of
  // the normal file view -- desktop-only, same as the rest of Internet
  // (search_youtube/search_images/search_books aren't registered on
  // Android).
  function savedSearchExtOf(list: Entry[]): "ytsearch" | "imgsearch" | "booksearch" | null {
    if (list.length === 0 || list.some((e) => e.is_dir)) return null;
    if (list.every((e) => e.name.toLowerCase().endsWith(".ytsearch"))) return "ytsearch";
    if (list.every((e) => e.name.toLowerCase().endsWith(".imgsearch"))) return "imgsearch";
    if (list.every((e) => e.name.toLowerCase().endsWith(".booksearch"))) return "booksearch";
    return null;
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
      selectAndReveal(name);
      setRenaming({ name, value: name });
    } catch (e) {
      setError(String(e));
    }
  }

  const [editingPath, setEditingPath] = useState(false);
  const [pathInput, setPathInput] = useState("");
  const [pathCopied, setPathCopied] = useState(false);
  const [errorCopied, setErrorCopied] = useState(false);
  async function copyError() {
    try {
      await navigator.clipboard.writeText(error);
      setErrorCopied(true);
      setTimeout(() => setErrorCopied(false), 1200);
    } catch {
      /* ignore -- the error bar is already visible with the same text */
    }
  }

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
  const breadcrumbRef = useRef<HTMLDivElement>(null);
  // Pull-to-refresh (mobile only): pullDist is how far the touch has
  // dragged down past the top of an already-scrolled-to-top list, purely
  // for the indicator's height/rotation while dragging. pullStartY is
  // null whenever no pull is in progress -- including mid-scroll, so a
  // drag that starts lower in the list and only reaches scrollTop 0
  // later doesn't retroactively count.
  const [pullDist, setPullDist] = useState(0);
  const [pullRefreshing, setPullRefreshing] = useState(false);
  const pullStartY = useRef<number | null>(null);
  const PULL_THRESHOLD = 64;
  const PULL_MAX = 100;
  const dragPaths = useRef<string[]>([]);
  const dragFavIndex = useRef<number | null>(null);
  const [draggingFavIdx, setDraggingFavIdx] = useState<number | null>(null);
  // Pending drag payload from InternetView (see beginDrag/dragPaths above
  // for the real-file equivalent) -- an Internet result isn't a real file
  // yet, so there's nothing to hand a native OS-level drag, just this
  // in-window ref a folder target's onDrop reads back out.
  const dragInternetItems = useRef<InternetDownloadItem[] | null>(null);
  // Set by InternetView while it is mounted (see onRegisterBack): returns
  // true when it handled the press itself.
  const internetBackRef = useRef<(() => boolean) | null>(null);

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
  const [digestDismissed, setDigestDismissed] = useState(false);
  useEffect(() => {
    setDigestDismissed(false);
  }, [curDir]);
  const savedSearchExt = mobile || loc.kind !== "fs" ? null : savedSearchExtOf(entries);
  const showDigest = !!savedSearchExt && !digestDismissed;

  // Per-entry "truly synced" state for the folder on screen: "verified"
  // means the last `rclone check` matched this file's checksum against the
  // cloud copy AND nothing changed locally since -- a strictly stronger
  // claim than "a sync pass ran". Entries inside an unlocked vault are
  // covered too (the backend maps them to their ciphertext files). Polled
  // on the same rhythm as the syncing badges.
  const [verifyStates, setVerifyStates] = useState<Map<string, string>>(new Map());
  useEffect(() => {
    let cancelled = false;
    const treeRoot = loc.kind === "vault" ? loc.root : loc.path;
    function poll() {
      const covered =
        entries.length > 0 &&
        [...driveSyncedPaths].some((p) => treeRoot === p || treeRoot.startsWith(p + "/"));
      if (!covered) {
        setVerifyStates((cur) => (cur.size ? new Map() : cur));
        return;
      }
      api
        .syncVerifyStates(
          loc.kind,
          curDir,
          entries.map((e) => e.name)
        )
        .then((states) => {
          if (cancelled) return;
          setVerifyStates(new Map(entries.map((e, i) => [e.name, states[i]])));
        })
        .catch(() => {});
    }
    poll();
    const interval = setInterval(poll, 2500);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [entries, loc, curDir, driveSyncedPaths]);

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
    const applyReveal = ({ path, select }: { path: string; select: string | null }) => {
      go({ kind: "fs", path });
      pendingRevealSelectRef.current = select ? { dir: path, name: select } : null;
    };
    listen<{ path: string; select: string | null }>("show-in-folder", (event) =>
      applyReveal(event.payload)
    )
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});
    // A request that arrived *before* this listener existed -- i.e. D-Bus
    // activation launching the app for a "Show in folder" (Chrome's Downloads
    // panel with VaultExplorer not already running). The backend's emit had
    // no listeners then, so the app just opened on its default folder; it
    // parks the request instead and this drains it. See filemanager1.rs.
    api
      .takePendingReveal()
      .then((pending) => {
        if (pending) applyReveal(pending);
      })
      .catch(() => {});
    return () => unlisten?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // Set (by `refresh()` or the effect below) once the reveal target has been
  // selected but not yet scrolled to -- the two halves have to happen at
  // different moments: selecting belongs in the same `refresh()` that loads
  // the listing (so its own selection reset can't clobber it), while
  // scrolling can only happen after React has committed those entries and
  // the tile actually exists in the DOM.
  const pendingRevealScrollRef = useRef<string | null>(null);
  useEffect(() => {
    const pending = pendingRevealSelectRef.current;
    // Fallback for a reveal that arrives when `refresh()` isn't the thing
    // that loads the folder (it consumes the ref itself when it is).
    if (pending && curDir === pending.dir && entries.some((e) => e.name === pending.name)) {
      selectOnly(pending.name);
      pendingRevealSelectRef.current = null;
      pendingRevealScrollRef.current = pending.name;
    }
    const name = pendingRevealScrollRef.current;
    if (!name || !entries.some((e) => e.name === name)) return;
    pendingRevealScrollRef.current = null;
    // Scoped to contentRef (not just ".entries") so this also finds
    // ContactsGrid's rows/"others" tiles, tagged with data-name the same
    // way -- column view rows aren't tagged at all, so a reveal there
    // selects without scrolling.
    const find = () =>
      contentRef.current?.querySelector(`[data-name="${CSS.escape(name)}"]`) as HTMLElement | null;
    const inView = (el: HTMLElement) => {
      const box = el.getBoundingClientRect();
      const scroller = contentRef.current?.getBoundingClientRect();
      return !!scroller && box.top >= scroller.top && box.bottom <= scroller.bottom;
    };
    // Retried, not fired once: on a cold start (Chrome's "Show in folder"
    // launching the app) the icon grid's column count is still being measured
    // from the live content width when the first scroll lands, so the tile
    // moves right after -- and a file deep in a big folder ends up off-screen
    // again, which is exactly the "selected but not scrolled to" report. The
    // later passes are no-ops once the row is genuinely in view.
    const timers: ReturnType<typeof setTimeout>[] = [];
    const attempt = () => {
      const el = find();
      if (!el) return;
      if (!inView(el)) el.scrollIntoView({ block: "center" });
    };
    const raf = requestAnimationFrame(attempt);
    timers.push(setTimeout(attempt, 150), setTimeout(attempt, 450));
    return () => {
      cancelAnimationFrame(raf);
      timers.forEach(clearTimeout);
    };
  }, [entries, curDir, selectOnly]);
  // Selects + schedules the scroll-into-view above for something just
  // created in curDir (new folder/file/vault/etc.) -- entries has to
  // already include `name` by the time this runs (i.e. call after
  // `await refresh()`), same precondition as the reveal path.
  function selectAndReveal(name: string) {
    selectOnly(name);
    pendingRevealScrollRef.current = name;
  }

  // Column view: a single click on a file selects it (and shows the info
  // preview column on the right) instead of opening it -- opening still
  // takes a double click, matching every other view.
  const [previewEntry, setPreviewEntry] = useState<{ dir: string; entry: Entry } | null>(null);
  // Bumped after an action from the preview pane's own context menu
  // changes the previewed folder's contents (e.g. Move to Trash) -- the
  // fs watcher only covers curDir, so the pane needs its own reload cue.
  const [previewReloadKey, setPreviewReloadKey] = useState(0);
  const locKey = inVault ? loc.root + "::" + loc.rel : loc.path;
  useEffect(() => {
    setPreviewEntry(null);
  }, [locKey]);

  // View mode + icon zoom: ONE global setting that follows you from folder
  // to folder, plus an explicit per-folder pin as the escape hatch.
  //
  // This used to be implicitly per-folder (every folder you ever touched
  // silently remembered its own view), which is what Finder does and what
  // made navigating feel unstable: walking down a tree flipped between
  // icon/list/columns and between zoom levels at folders you didn't
  // consciously configure, and there was no way to tell why. A view that
  // stays put unless you change it is the more predictable default; the
  // "one folder genuinely wants a different view" case (a photo folder as
  // big thumbnails) is rarer, so it's opt-in via "Always Open in This
  // View" and only that folder is affected -- leaving it drops you back to
  // the global view instead of dragging the pinned one along.
  type ViewPrefs = { view: View; iconScale: number };
  const [defaultViewPrefs, setDefaultViewPrefs] = useState<ViewPrefs>(() => {
    try {
      const raw = localStorage.getItem("vaultexplorer:view-default");
      if (raw) return JSON.parse(raw);
    } catch {
      /* ignore */
    }
    return { view: "icon", iconScale: 1 };
  });
  const [pinnedViewPrefs, setPinnedViewPrefs] = useState<Record<string, ViewPrefs>>(() => {
    // The old implicit per-folder map is deliberately dropped rather than
    // migrated -- it's a record of views the user never chose on purpose,
    // so importing it would just reproduce the jumpiness under a new key.
    localStorage.removeItem("vaultexplorer:folder-view");
    try {
      const raw = localStorage.getItem("vaultexplorer:view-pins");
      if (raw) return JSON.parse(raw);
    } catch {
      /* ignore */
    }
    return {};
  });
  useEffect(() => {
    localStorage.setItem("vaultexplorer:view-default", JSON.stringify(defaultViewPrefs));
  }, [defaultViewPrefs]);
  useEffect(() => {
    localStorage.setItem("vaultexplorer:view-pins", JSON.stringify(pinnedViewPrefs));
  }, [pinnedViewPrefs]);
  const viewPrefsRef = useRef({ def: defaultViewPrefs, pinned: pinnedViewPrefs });
  useEffect(() => {
    viewPrefsRef.current = { def: defaultViewPrefs, pinned: pinnedViewPrefs };
  }, [defaultViewPrefs, pinnedViewPrefs]);
  // Whatever the last navigation *restored*, so the persist effect below can
  // tell a restore apart from the user actually picking something.
  const restoredViewRef = useRef<ViewPrefs | null>(null);
  const liveViewRef = useRef<ViewPrefs>({ view, iconScale });
  useEffect(() => {
    liveViewRef.current = { view, iconScale };
  });
  // Apply the pin (if this folder has one) or the global view on navigation.
  // Keyed only on locKey so it doesn't re-fire when the maps are written to.
  useEffect(() => {
    // Column view owns the whole navigation path -- clicking a folder there
    // extends the columns via this same `loc` update instead of replacing
    // the pane -- so pins are ignored while it's active. Otherwise walking
    // into a pinned subfolder would kick you out of the view you're
    // browsing in, which is exactly the old bug in miniature.
    if (liveViewRef.current.view === "column") return;
    const { def, pinned } = viewPrefsRef.current;
    const target = pinned[locKey] ?? def;
    restoredViewRef.current = target;
    // A pin/default saved from a desktop session (or a settings sync)
    // can still name "column"/"listPreview" -- the view-menu no longer
    // offers switching TO them on mobile, but a stored preference bypasses
    // that menu entirely, so it needs its own fallback here.
    const restoredView =
      mobile && (target.view === "column" || target.view === "listPreview") ? "icon" : target.view;
    setView(restoredView);
    setIconScale(target.iconScale);
  }, [locKey, mobile]);
  // Persist a real change: into this folder's pin if it has one, otherwise
  // into the global view. Keyed only on [view, iconScale] (not locKey) so a
  // navigation that restores identical values is a no-op, and a change fires
  // with the already-committed locKey.
  useEffect(() => {
    const restored = restoredViewRef.current;
    if (restored && restored.view === view && restored.iconScale === iconScale) return;
    restoredViewRef.current = null;
    if (viewPrefsRef.current.pinned[locKey]) {
      setPinnedViewPrefs((prev) => ({ ...prev, [locKey]: { view, iconScale } }));
    } else {
      setDefaultViewPrefs({ view, iconScale });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view, iconScale]);
  const viewPinned = pinnedViewPrefs[locKey] !== undefined;
  function toggleViewPin(): void {
    setPinnedViewPrefs((prev) => {
      const next = { ...prev };
      if (locKey in next) delete next[locKey];
      // Unpinning leaves the current view on screen (snapping back would be
      // its own jolt); it just means later changes go to the global view.
      else next[locKey] = { view, iconScale };
      return next;
    });
  }

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

  // Real `Entry` for each search hit, keyed by path. Search itself only
  // returns paths, and the results list was building a stand-in entry with
  // `is_dir: false` from the filename alone -- which is why a folder in the
  // results showed a blank/wrong icon (`FileIcon` picks by kind, and the
  // stand-in claimed "file with no known extension") and why nothing ever
  // got a thumbnail. Resolved by listing each distinct parent directory
  // once, rather than a new per-path stat command: hits cluster into few
  // folders, and both spaces (fs and vault) already have a listing call.
  const [searchEntries, setSearchEntries] = useState<Record<string, Entry>>({});
  const listDir = useCallback(
    (dir: string, kind: Loc["kind"]) =>
      kind === "vault" ? api.listDir(dir) : api.fsList(dir, appSettings.showHiddenFiles),
    [appSettings.showHiddenFiles]
  );

  useEffect(() => {
    if (searchResults === null || searchResults.length === 0) {
      setSearchEntries({});
      return;
    }
    let cancelled = false;
    (async () => {
      const byDir = new Map<string, string[]>();
      for (const p of searchResults) {
        const dir = parentPath(p);
        const names = byDir.get(dir);
        if (names) names.push(baseName(p));
        else byDir.set(dir, [baseName(p)]);
      }
      const resolved: Record<string, Entry> = {};
      for (const [dir, names] of byDir) {
        try {
          const list = await listDir(dir, loc.kind);
          if (cancelled) return;
          const wanted = new Set(names);
          for (const en of list) {
            if (wanted.has(en.name)) resolved[joinPath(dir, en.name)] = en;
          }
        } catch {
          /* unreadable folder -- those rows keep the fallback icon */
        }
        // Publish as each folder lands so a long result list fills in
        // progressively instead of staying iconless until the last listing.
        if (!cancelled) setSearchEntries({ ...resolved });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [searchResults, loc.kind, listDir]);

  // For view-specific search rendering (Contacts/Library rows instead of
  // the generic file-tile row): the active view's own grid component
  // takes `entries: Entry[]` + a single `curDir`, but search hits span
  // many directories, so each resolved Entry is looked up by object
  // identity to recover its real full path instead.
  const searchPathByEntry = useMemo(() => {
    const m = new Map<Entry, string>();
    if (!searchResults) return m;
    for (const p of searchResults) {
      m.set(searchEntries[p] ?? { name: baseName(p), is_dir: false, size: 0, mtime: 0 }, p);
    }
    return m;
  }, [searchResults, searchEntries]);
  const searchEntryList = useMemo(() => [...searchPathByEntry.keys()], [searchPathByEntry]);

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
        // Hand the scroll off to the effect that watches `entries` -- doing it
        // here would target a DOM that doesn't have the row yet.
        pendingRevealScrollRef.current = reveal.name;
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

  // Pull-to-refresh: only starts tracking a drag when the list is already
  // scrolled to the very top (scrollTop <= 0) -- otherwise this is just a
  // normal scroll gesture and must not fight it. Disabled outside the
  // regular list (My Computer / Internet aren't backed by `refresh()`) and
  // on desktop, where there's no touchscreen for this to apply to.
  const pullEligible = mobile && !showMyComputer && !showInternet && searchResults === null;
  function onContentTouchStart(e: React.TouchEvent) {
    if (!pullEligible || pullRefreshing) return;
    const el = contentRef.current;
    if (el && el.scrollTop <= 0) pullStartY.current = e.touches[0].clientY;
  }
  function onContentTouchMove(e: React.TouchEvent) {
    if (pullStartY.current == null) return;
    const delta = e.touches[0].clientY - pullStartY.current;
    if (delta <= 0) {
      // scrolled back up past the start point -- abandon, let it scroll
      pullStartY.current = null;
      setPullDist(0);
      return;
    }
    setPullDist(Math.min(delta, PULL_MAX));
  }
  async function onContentTouchEnd() {
    if (pullStartY.current == null) return;
    pullStartY.current = null;
    if (pullDist >= PULL_THRESHOLD) {
      setPullRefreshing(true);
      try {
        await refresh();
      } finally {
        setPullRefreshing(false);
        setPullDist(0);
      }
    } else {
      setPullDist(0);
    }
  }

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
    // Real shared storage (as opposed to this app's own sandbox) needs the
    // "All files access" permission -- checked here, not just on the
    // initial sidebar tap, so it also covers double-clicking into a
    // subfolder, the breadcrumb, and back/forward: any of those landing on
    // a shared-storage path with the permission since revoked (or never
    // granted -- e.g. a saved favorite/history entry from before it was)
    // would otherwise 404 with a raw "permission denied" instead of the
    // proper request prompt.
    if (mobile && target.kind === "fs" && target.path.startsWith(PHONE_STORAGE_PATH)) {
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
    setShowMyComputer(false);
    setFreeUpSpaceOpen(false);
    setShowDevice(false);
    setShowInternet(false);
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

  // My Computer/Internet overlay the content area without ever pushing a
  // history entry (opening a saved search file doesn't change `loc`
  // either) -- so Back/Forward's normal history-index math doesn't apply
  // to them at all. Dismissing the overlay is enough: `loc`/`curDir`
  // never moved, so whatever's underneath is still exactly the folder the
  // user was in. Without this, Back either did nothing (disabled at
  // histIdx 0, which is exactly when a saved search opened from the
  // start-page folder) or jumped past the current folder to an earlier
  // one in history.
  function goBack() {
    if (showMyComputer || showInternet) {
      setShowMyComputer(false);
      setFreeUpSpaceOpen(false);
      setShowDevice(false);
      setShowInternet(false);
      setInternetInitial(null);
      return;
    }
    if (histIdx === 0) return;
    go(history[histIdx - 1], false).then(() => setHistIdx(histIdx - 1));
  }
  function goForward() {
    if (showMyComputer || showInternet) {
      setShowMyComputer(false);
      setFreeUpSpaceOpen(false);
      setShowDevice(false);
      setShowInternet(false);
      setInternetInitial(null);
      return;
    }
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
  // Above PHONE_STORAGE_PATH is mostly OS-sandboxed away from the app on
  // Android even with "All files access" granted (that permission only
  // covers shared/media storage, not the general filesystem) -- but
  // /storage and /storage/emulated themselves usually still list fine, and
  // this is meant to be a real file explorer, not one artificially capped
  // at a folder boundary. So "up" is allowed all the way to "/" same as
  // desktop; a folder that's genuinely blocked just surfaces a clear,
  // copyable error instead (see `refresh()`'s catch) rather than silently
  // refusing to try.
  const canGoUp = loc.kind === "vault" || loc.path !== "/";
  // Android's physical back button and the gesture-nav back-swipe are the
  // same OS event, and Tauri's own `app` plugin already exposes it as
  // `onBackButtonPress` for exactly this -- subscribing to it is also what
  // tells that plugin's native side to stop applying its own default
  // (WebView `goBack()`/exit, which this app never wants since it doesn't
  // use real browser history for folder navigation). Closest layer wins: a
  // context menu, then Settings, then the favorites drawer, then an
  // expanded search field, then folder-history "up" -- only once none of
  // those apply does a *second* back within 2s actually exit (the standard
  // double-back-to-exit pattern, via the plugin's own `exit` command), so a
  // single stray back press at the root never loses the app by accident.
  const lastBackAt = useRef(0);
  // `onBackButtonPress`'s callback is a closure captured once, whenever
  // this effect (re-)runs -- and `canGoUp` is `true` almost everywhere on
  // Android (its root is a real sandboxed path, never literally "/", so
  // it doesn't flip false↔true across an ordinary browse the way it does
  // on desktop). That meant this effect's dependency array rarely changed
  // and the listener rarely resubscribed, so a back press kept calling a
  // *stale* `goUp` bound to wherever `loc` was the last time it *did*
  // resubscribe (often app launch) -- confirmed live: navigating into a
  // folder then pressing back landed two levels above the real current
  // folder, at whatever was one-up from the stale one, not one-up from
  // where the user actually was. A ref keeps the callback reading
  // whatever is current at press-time without needing to resubscribe for
  // every value it touches -- so this now subscribes exactly once, when
  // `mobile` first turns true, and never again for the component's life.
  const backStateRef = useRef({
    mediaViewer,
    mobileEditorTarget,
    menu,
    settingsOpen,
    sidebarOpen,
    searchExpanded,
    canGoUp,
    goUp,
    refresh,
  });
  backStateRef.current = {
    mediaViewer,
    mobileEditorTarget,
    menu,
    settingsOpen,
    sidebarOpen,
    searchExpanded,
    canGoUp,
    goUp,
    refresh,
  };
  useEffect(() => {
    if (!mobile) return;
    let listener: { unregister: () => Promise<void> } | undefined;
    onBackButtonPress(() => {
      const s = backStateRef.current;
      // Fullscreen photo/video/audio viewer sits on top of everything else
      // (media-viewer-overlay z-index 2100) -- it has to be the very first
      // thing back closes, or the press falls through to this chain's
      // folder-navigation branches and silently moves the *background*
      // folder view up a level while the still-open viewer hides that from
      // the user entirely (confirmed live: pressing back while playing
      // audio just kept playing with nothing visibly changing).
      if (s.mediaViewer) {
        setMediaViewer(null);
        return;
      }
      // Internet gets first refusal after the media viewer: a playing
      // video should close on back, and a search should step back to the
      // Videos/Images/Books tiles -- pressing back there used to leave the
      // whole section, losing the search. InternetView registers what it
      // can consume; anything it doesn't falls through to this chain.
      if (internetBackRef.current?.()) return;
      if (s.mobileEditorTarget) {
        setMobileEditorTarget(null);
        s.refresh();
        return;
      }
      if (s.menu) {
        setMenu(null);
        return;
      }
      if (s.settingsOpen) {
        setSettingsOpen(false);
        return;
      }
      if (s.sidebarOpen) {
        setSidebarOpen(false);
        return;
      }
      if (s.searchExpanded) {
        setSearchExpanded(false);
        return;
      }
      if (s.canGoUp) {
        s.goUp();
        return;
      }
      const now = Date.now();
      if (now - lastBackAt.current < 2000) {
        invoke("plugin:app|exit").catch(() => {});
        return;
      }
      lastBackAt.current = now;
      setInfoMsg("Toca atrás otra vez para salir");
    })
      .then((l) => {
        listener = l;
      })
      .catch(() => {});
    return () => {
      listener?.unregister();
    };
  }, [mobile]);
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
          ? "Vault Explorer"
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

  // Schedule the sensitive session to auto-relock when the window expires:
  // re-lock in the backend and drop any open preview so a sensitive file
  // stops being visible the moment the timer lapses ("walked away" safety).
  // `secs` is what the unlock sheet's duration picker chose for THIS session
  // (defaulted from Settings), not the configured value -- the two differ
  // whenever the user picks a different window for one file.
  function scheduleSensitiveRelock(secs: number | null) {
    if (sensitiveTimerRef.current) {
      clearTimeout(sensitiveTimerRef.current);
      sensitiveTimerRef.current = null;
    }
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

  async function submitSensitive(password: string, timeout: SensitiveTimeout) {
    if (!sensitivePrompt) return;
    const secs = timeout === "never" ? null : timeout;
    try {
      await api.vaultUnlockSensitive(password, secs);
      const proceed = sensitivePrompt.proceed;
      setSensitivePrompt(null);
      scheduleSensitiveRelock(secs);
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
  // Opens the fullscreen gallery viewer for an image/video/audio file
  // instead of shelling out via osOpen. `resolvedPath`/`resolvedInVault`
  // let a caller substitute an already-resolved real path (e.g. a
  // mobile vault file that's been decrypted to a throwaway temp copy,
  // since there's no FUSE mount on Android for MediaViewer's own
  // openPath-based resolution to use) -- when given, the *opened* entry
  // uses that instead of the vault-relative path, but sibling gallery
  // entries (for swipe navigation) still use their normal vault-relative
  // path + inVault=true, since prefetch-decrypting every sibling just to
  // build a gallery isn't worth it. Practical effect: swiping to a
  // sibling from a mobile-vault-opened image falls back through
  // MediaViewer's normal per-item resolution, which does work for
  // desktop (FUSE-backed) but not mobile vault siblings -- an accepted
  // gap for now, single-file viewing still works everywhere.
  function openMediaViewer(
    dir: string,
    entry: Entry,
    inVault: boolean,
    resolved?: { path: string; inVault: boolean }
  ) {
    const kind = kindOf(entry);
    const wantKinds: Kind[] = kind === "audio" ? ["audio"] : ["image", "video"];
    const siblings = entries.filter((e) => !e.is_dir && wantKinds.includes(kindOf(e)));
    const gallery: GalleryEntry[] = siblings.map((e) => ({
      name: e.name,
      fullPath: joinPath(dir, e.name),
      kind: kindOf(e) as "image" | "video" | "audio",
      inVault,
    }));
    let startIndex = gallery.findIndex((g) => g.name === entry.name);
    if (resolved) {
      // Splice in the already-resolved stand-in for the opened entry
      // itself (siblings keep their normal vault-relative path).
      if (startIndex >= 0) {
        gallery[startIndex] = { ...gallery[startIndex], fullPath: resolved.path, inVault: resolved.inVault };
      } else {
        gallery.push({ name: entry.name, fullPath: resolved.path, kind: kind as "image" | "video" | "audio", inVault: resolved.inVault });
        startIndex = gallery.length - 1;
      }
    }
    if (startIndex < 0) startIndex = 0;
    // Desktop opens media in its own window (traffic lights, fullscreen,
    // its own entry in the window switcher, the grid still usable behind
    // it); mobile has no window manager to put one in, so it keeps the
    // in-app overlay.
    if (mobile) {
      setMediaViewer({ gallery, startIndex });
      return;
    }
    api.openMediaWindow(gallery, startIndex).catch(() => {
      // No window (an unusual WM, or the command missing on this
      // platform) is not a reason to leave a double-click doing nothing.
      setMediaViewer({ gallery, startIndex });
    });
  }

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
        // No FUSE on Android, so a vault nested inside another vault has no
        // real on-disk path to unlock at all -- unlike a single file, a
        // whole live/mutable nested vault can't be satisfied by a one-shot
        // decrypted copy. Needs its own DocumentsProvider-shaped solution.
        if (mobile) {
          setError("Nested vaults aren't supported on mobile yet.");
          return;
        }
        try {
          const resolvedRoot = await api.openPath(joinPath(dir, entry.name));
          go({ kind: "vault", root: resolvedRoot, rel: "" });
        } catch (e) {
          setError(String(e));
        }
        return;
      }
      if (entry.is_dir) {
        // Entering a sensitive folder is gated the same as opening a
        // sensitive file -- even the listing (names/sizes) stays hidden
        // until re-auth, not just file contents.
        const full = joinPath(dir, entry.name);
        withSensitive(full, () => go({ kind: "vault", root: loc.root, rel: full }));
        return;
      }
      if (entry.name.toLowerCase().endsWith(ENCRYPTED_FILE_EXT)) {
        setDecryptPrompt({ entry, error: "", mode: "open" });
        return;
      }
      if (ARCHIVE_EXT_RE.test(entry.name)) {
        // Browsing an archive as a folder needs the same FUSE mount a
        // nested vault would -- not a single decrypted file.
        if (mobile) {
          setError("Browsing archives inside a vault isn't supported on mobile yet.");
          return;
        }
        // Browse a vault-internal archive like a folder, same as on fs.
        const full = joinPath(dir, entry.name);
        withSensitive(full, () => mountArchive(dir, entry));
        return;
      }
      {
        const full = joinPath(dir, entry.name);
        withSensitive(full, async () => {
          if (mobile && !appSettings.mobileExternalEditor && isPlainTextEntry(entry)) {
            setMobileEditorTarget({ entry, fullPath: full, inVault: true });
            return;
          }
          const mediaKind = kindOf(entry);
          if (mediaKind === "image" || mediaKind === "video" || mediaKind === "audio") {
            try {
              // See openMediaViewer's comment: no FUSE on Android, so the
              // opened entry itself gets a decrypted temp stand-in there;
              // desktop just uses the FUSE-backed vault-relative path
              // MediaViewer already knows how to resolve on its own.
              if (mobile) {
                const abs = await api.vaultDecryptToTemp(full);
                openMediaViewer(dir, entry, true, { path: abs, inVault: false });
              } else {
                openMediaViewer(dir, entry, true);
              }
            } catch (e) {
              setError(String(e));
            }
            return;
          }
          try {
            // No FUSE mount on Android -- open a throwaway decrypted copy
            // instead of the in-place virtual-filesystem path desktop uses.
            const abs = mobile ? await api.vaultDecryptToTemp(full) : await api.openPath(full);
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
    // Saved Internet search (see InternetView). Malformed/hand-edited JSON
    // just surfaces as a normal error rather than silently falling through
    // to a text-editor open, which would show raw JSON that looks broken
    // for no reason.
    if (/\.(ytsearch|imgsearch|booksearch)$/i.test(entry.name)) {
      try {
        const saved = JSON.parse(await api.fsReadText(full)) as SavedInternetSearch;
        if (saved.kind !== "videos" && saved.kind !== "images" && saved.kind !== "books") {
          throw new Error("not a saved search");
        }
        openInternetSearchFile(saved);
      } catch (e) {
        setError(String(e));
      }
      return;
    }
    // A contact opens in the same form editor the Contacts view uses, from
    // wherever it was double-clicked: a .vcf is a contact everywhere, not
    // only inside one view (see the contact rows in EntryTile).
    if (/\.vcf$/i.test(entry.name)) {
      withSensitive(full, () => setMobileEditorTarget({ entry, fullPath: full, inVault: false }));
      return;
    }
    // A `.url` shortcut -- what dropping an Internet video result into a
    // folder writes (see downloadInternetItems). Opening it should do what
    // opening the result did, not show the shortcut's own text: a YouTube
    // link goes to the in-app player window, anything else to the browser.
    if (/\.url$/i.test(entry.name)) {
      try {
        const body = await api.fsReadText(full);
        const url = body.match(/^URL=(.+)$/mi)?.[1]?.trim();
        if (!url) throw new Error("This shortcut has no URL in it.");
        const ytId = url.match(/[?&]v=([A-Za-z0-9_-]{5,})/)?.[1];
        if (ytId && !mobile) {
          await api.openPlayerWindow("youtube", [{ key: ytId, title: entry.name.replace(/\.youtube\.url$|\.url$/i, "") }], 0);
        } else {
          await osOpen(url);
        }
      } catch (e) {
        setError(String(e));
      }
      return;
    }
    if (mobile && !appSettings.mobileExternalEditor && isPlainTextEntry(entry)) {
      setMobileEditorTarget({ entry, fullPath: full, inVault: false });
      return;
    }
    const mediaKind = kindOf(entry);
    if (mediaKind === "image" || mediaKind === "video" || mediaKind === "audio") {
      openMediaViewer(dir, entry, false);
      return;
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
    if (paths.length) {
      setClipboard({ paths, mode: "copy", kind: loc.kind, root: inVault ? loc.root : undefined });
    }
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
    if (paths.length) {
      setClipboard({ paths, mode: "cut", kind: loc.kind, root: inVault ? loc.root : undefined });
    }
  }
  async function paste() {
    if (!clipboard) return;
    // Two *different* vaults, both `kind: "vault"` -- move_entry/copy_entry
    // and export_file/delete_file below all operate on whichever vault is
    // currently "active" server-side, which tracks navigation, not
    // clipboard history. Pasting into a different vault than the one the
    // files were copied from needs the dedicated cross-vault commands
    // instead (decrypt under the source vault's key, re-encrypt under the
    // destination's) -- files only, same as the fs<->vault boundary below.
    if (clipboard.kind === "vault" && loc.kind === "vault" && clipboard.root && clipboard.root !== loc.root) {
      const srcRoot = clipboard.root;
      try {
        for (const src of clipboard.paths) {
          const dest = joinPath(curDir, baseName(src));
          if (clipboard.mode === "copy") {
            await api.vaultToVaultCopy(srcRoot, src, loc.root, dest);
          } else {
            await api.vaultToVaultMove(srcRoot, src, loc.root, dest);
          }
        }
        setClipboard(null);
        refresh();
      } catch (e) {
        setError(String(e));
      }
      return;
    }
    // Cut/copy across the vault boundary -- clipboard.kind is whichever
    // space the files were cut/copied *from*, loc.kind is where they're
    // now being pasted. Files only (not folders): importFile/exportFile
    // are both single-file encrypt/decrypt, not recursive.
    if (clipboard.kind !== loc.kind) {
      try {
        if (clipboard.kind === "vault" && clipboard.root) {
          // Same "active vault tracks navigation, not clipboard" issue as
          // above -- export_file/delete_file need the source vault active.
          await api.setActiveVault(clipboard.root);
        }
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
    if (inVault && mobile) {
      setError("Browsing archives inside a vault isn't supported on mobile yet.");
      return;
    }
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
    if (inVault && mobile) {
      setError("Encrypting a folder inside a vault isn't supported on mobile yet.");
      return;
    }
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
      selectAndReveal(name);
      setRenaming({ name, value: name });
    } catch (e) {
      setError(String(e));
    }
  }

  // Android-only "Add to Home Screen": pins a launcher icon whose Intent is
  // this folder's `vaultexplorer://open-folder` deep link, resolved back
  // through the same handler `go()`-navigates a shared add-device link
  // with. Covers a plain fs folder, a vault sitting unopened in fs view
  // (pins straight to its root), and a folder nested inside an open vault.
  async function addFolderShortcut(entry: Entry) {
    const rel = joinPath(curDir, entry.name);
    const target: Loc = inVault
      ? { kind: "vault", root: loc.root, rel }
      : entry.is_vault
        ? { kind: "vault", root: rel, rel: "" }
        : { kind: "fs", path: rel };
    const url = buildOpenFolderLink(target);
    // Only the plain-fs custom-icon map applies here (vaults have their own
    // separate icon system) -- and only an emoji renders to a bitmap on the
    // JS side; a WhiteSur icon just falls back to the app's icon.
    const customIcon = !inVault ? customIcons[rel] : undefined;
    const iconBase64 =
      customIcon && !customIcon.startsWith(CUSTOM_ICON_PREFIX)
        ? renderEmojiIconPng(customIcon)
        : undefined;
    try {
      await api.androidPinFolderShortcut(hashForShortcutId(url), entry.name, url, iconBase64);
    } catch (e) {
      setError(String(e));
    }
  }

  // Export/import every `vaultexplorer:*` localStorage key as one blob --
  // favorites, appSettings, per-vault UI prefs, custom icons, templates,
  // pinned files, all of it -- rather than hand-picking fields one at a
  // time and inevitably missing the next one someone adds. `sourceHome`
  // rides along so import can remap paths for a *different* platform's
  // layout (e.g. a Linux `/home/you/...` favorite pasted on Android
  // becomes a real `/storage/emulated/0/...` one, not a dead path) --
  // exactly the "paste this on my phone" case this was built for.
  //
  // `includeCloud` bundles the live rclone OAuth tokens too, for
  // reconnecting cloud sync from wherever this gets pasted next without
  // re-authenticating -- opt-in and off by default (see the Settings
  // checkbox this is wired to): those tokens are as good as the account
  // password for whatever they're scoped to, and this puts them on the
  // plain OS clipboard, readable by any clipboard manager/sync service
  // that happens to be watching. Desktop-only either way -- there's no
  // `rclone` binary on Android for a receiving mobile device to use them
  // with regardless of whether they're included.
  async function buildConfigExportBlob(includeCloud: boolean): Promise<string> {
    const data: Record<string, string> = {};
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (key?.startsWith("vaultexplorer:")) data[key] = localStorage.getItem(key) ?? "";
    }
    const payload: {
      version: 1;
      sourceHome: string;
      data: Record<string, string>;
      rcloneConf?: string;
    } = { version: 1, sourceHome: home, data };
    if (includeCloud && !mobile) {
      try {
        const conf = await api.rcloneReadConfRaw();
        if (conf) payload.rcloneConf = conf;
      } catch {
        /* rclone not installed -- nothing to include */
      }
    }
    return JSON.stringify(payload);
  }

  async function exportConfigToClipboard(includeCloud: boolean) {
    try {
      const blob = await buildConfigExportBlob(includeCloud);
      await navigator.clipboard.writeText(blob);
      setInfoMsg(
        includeCloud
          ? "Config + cloud credentials copied to clipboard"
          : "Config copied to clipboard"
      );
    } catch (e) {
      setError(String(e));
    }
  }

  async function importConfigFromClipboard() {
    let text: string;
    try {
      // Not `navigator.clipboard.readText()` -- Android's WebView denies
      // the Web Clipboard API's read permission outright (confirmed live:
      // "NotAllowedError: Read permission denied"), with no prompt to
      // grant it either. The clipboard-manager plugin reads through the
      // real native Android clipboard API instead, which has no such
      // restriction.
      text = (await clipboardReadText()) ?? "";
    } catch (e) {
      setError(String(e));
      return;
    }
    let payload: { version?: number; sourceHome?: string; data?: Record<string, string>; rcloneConf?: string };
    try {
      payload = JSON.parse(text);
    } catch {
      setError("Clipboard doesn't contain a Vault Explorer config export");
      return;
    }
    if (!payload?.data || typeof payload.data !== "object") {
      setError("Clipboard doesn't contain a Vault Explorer config export");
      return;
    }
    // On mobile the real target for anything that used to live under the
    // exporting device's home is shared storage, not this app's own
    // sandboxed "home" -- see the favorites fix (`useFavorites.ts`) this
    // mirrors, for the same reason.
    const targetPrefix = mobile ? PHONE_STORAGE_PATH : home;
    const sourcePrefix = payload.sourceHome;
    let remapped = 0;
    for (const [key, rawValue] of Object.entries(payload.data)) {
      let value = rawValue;
      if (sourcePrefix && value.includes(sourcePrefix)) {
        const remappedValue = value.split(sourcePrefix).join(targetPrefix);
        if (remappedValue !== value) remapped++;
        value = remappedValue;
      }
      localStorage.setItem(key, value);
    }
    if (payload.rcloneConf && !mobile) {
      try {
        await api.rcloneMergeConfRaw(payload.rcloneConf);
      } catch (e) {
        setError(String(e));
        return;
      }
    }
    // Every setting above is read once at mount (`useState(() => ...
    // localStorage...)`) -- reapplying all of it live would mean
    // duplicating that same read for every single one of those states
    // instead of the one source of truth localStorage already is. A
    // reload is what actually re-runs them.
    setInfoMsg(
      `Imported${payload.rcloneConf ? " + cloud credentials" : ""}${remapped ? ` (${remapped} path${remapped === 1 ? "" : "s"} remapped)` : ""} — reloading…`
    );
    setTimeout(() => window.location.reload(), 900);
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
  // ---- touch drag-to-folder (mobile) --------------------------------
  // Mobile has no HTML5 drag: `draggable` is off there because it swallows
  // the long-press that opens the context menu. So moving files into a
  // folder had no gesture at all -- reported directly. This adds one that
  // can't collide with the existing gestures: while *selection mode* is
  // active (which is itself entered by long-press), pressing a selected
  // tile and sliding onto a folder moves the selection into it. Outside
  // selection mode nothing here runs.
  const touchDrag = useRef<{ names: string[]; startX: number; startY: number; active: boolean } | null>(null);
  function onEntriesTouchStart(e: React.TouchEvent) {
    if (!mobile || !selectionMode || selected.size === 0) return;
    const t = e.touches[0];
    if (!t) return;
    const tile = (t.target as HTMLElement).closest<HTMLElement>("[data-name]");
    const name = tile?.dataset.name;
    // Only a *selected* tile starts a move -- pressing an unselected one
    // still means "add this to the selection".
    if (!name || !selected.has(name)) return;
    touchDrag.current = { names: [...selected], startX: t.clientX, startY: t.clientY, active: false };
  }
  function onEntriesTouchMove(e: React.TouchEvent) {
    const drag = touchDrag.current;
    const t = e.touches[0];
    if (!drag || !t) return;
    if (!drag.active) {
      // Same threshold idea as the desktop marquee: a tap that drifts a
      // pixel or two is still a tap.
      if (Math.abs(t.clientX - drag.startX) < 12 && Math.abs(t.clientY - drag.startY) < 12) return;
      drag.active = true;
    }
    // Scrolling the list while carrying files would fight the gesture.
    e.preventDefault();
    const over = document.elementFromPoint(t.clientX, t.clientY) as HTMLElement | null;
    const folder = over?.closest<HTMLElement>("[data-name]")?.dataset.name;
    const target = folder && entries.find((en) => en.name === folder && en.is_dir);
    setDropTarget(target ? "entry:" + joinPath(curDir, target.name) : null);
  }
  function onEntriesTouchEnd(e: React.TouchEvent) {
    const drag = touchDrag.current;
    touchDrag.current = null;
    const key = dropTarget;
    setDropTarget(null);
    if (!drag?.active || !key?.startsWith("entry:")) return;
    const destDir = key.slice("entry:".length);
    if (drag.names.some((n) => joinPath(curDir, n) === destDir)) return;
    e.preventDefault();
    dragPaths.current = drag.names.map((n) => joinPath(curDir, n));
    dropInto(destDir);
  }

  // "Move to…" for a multi-selection. Mobile has no drag across folders
  // (and the touch drag added for it only reaches folders visible in the
  // current one), so picking a destination is the only way to move a
  // selection somewhere else -- reported as exactly that gap.
  async function moveSelectionTo() {
    const names = [...selected];
    if (names.length === 0) return;
    const dest = await pickPath({ directory: true, multiple: false, title: "Move to folder" });
    if (!dest || Array.isArray(dest)) return;
    if (dest === curDir) return;
    try {
      for (const name of names) {
        const src = joinPath(curDir, name);
        const target = joinPath(dest, name);
        inVault ? await api.moveEntry(src, target) : await api.fsRename(src, target);
      }
      setSelected(new Set());
      setSelectionMode(false);
      refresh();
    } catch (e) {
      setError(String(e));
    }
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
          // import_file writes into whichever vault is currently "active"
          // server-side -- not necessarily this drop's target vault, if a
          // *different* vault was the last one actually navigated into
          // (browsing fs itself never changes which vault is active). Same
          // fix as the cross-vault paste bug above.
          await api.setActiveVault(unlockedRoot);
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

  // ---- Internet result -> real folder (drag or "Save to Folder…") ----
  // Same per-item sequential-with-progress shape the paste/copy loop above
  // uses (one beginProgress row per item) -- these are real network
  // downloads, not instant fs renames, so each genuinely wants its own
  // progress row.
  async function downloadInternetItems(items: InternetDownloadItem[], destDir: string) {
    try {
      for (const item of items) {
        if (item.linkBody) {
          // A video result has no file on the web to fetch -- dropping one
          // into a folder writes a real `.youtube.url` shortcut instead,
          // so the result becomes a file you own: movable, copyable,
          // renameable, and reopenable like anything else.
          await api.fsWriteText(joinPath(destDir, item.filename), item.linkBody);
        } else {
          await api.downloadWebResult(item.url, destDir, item.filename, beginProgress(`Downloading "${item.filename}"`));
        }
      }
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }
  // Straight to Downloads, no destination prompt (see ytdl.rs) -- each URL
  // gets its own Actions row, so several downloads read as several jobs
  // and each can be cancelled on its own.
  function downloadInternetVideos(pageUrls: string[], audioOnly: boolean) {
    for (const url of pageUrls) {
      const run = mobile ? downloadOnMobile(url, audioOnly) : desktopDownload(url, audioOnly);
      run.then(() => refresh()).catch((e) => setError(String(e)));
    }
  }

  function desktopDownload(url: string, audioOnly: boolean) {
    return api.downloadVideo(url, audioOnly, beginProgress(`Downloading ${audioOnly ? "MP3" : "MP4"}`));
  }

  // Android has no yt-dlp (it's Python) and no ffmpeg, so the same job is
  // done in three steps the phone *can* do: resolve the streams in-process
  // (see ytstreams.rs), fetch them over plain HTTP with the downloader
  // that already reports into Actions, and -- for video, since YouTube
  // stopped serving progressive streams -- join the two tracks with
  // Android's own MediaMuxer.
  async function downloadOnMobile(url: string, audioOnly: boolean) {
    const streams = await api.youtubeStreams(url);
    const safe = streams.title.replace(/[/\\?%*:|"<>]/g, "-").slice(0, 120) || "video";
    const dest = joinPath(home ?? "", "Download");
    if (audioOnly) {
      if (!streams.audio_url) throw new Error("No audio stream available for this video.");
      // `.m4a`, not `.mp3`: this is the real container, and converting
      // would need ffmpeg, which Android doesn't have.
      await api.downloadWebResult(
        streams.audio_url,
        dest,
        `${safe}.${streams.audio_ext || "m4a"}`,
        beginProgress(`Downloading audio — ${safe}`)
      );
      return;
    }
    if (!streams.video_url || !streams.audio_url) throw new Error("No downloadable streams for this video.");
    const videoPart = `${safe}.video.mp4`;
    const audioPart = `${safe}.audio.m4a`;
    await api.downloadWebResult(streams.video_url, dest, videoPart, beginProgress(`Downloading video — ${safe}`));
    await api.downloadWebResult(streams.audio_url, dest, audioPart, beginProgress(`Downloading audio — ${safe}`));
    await api.androidMuxVideo(joinPath(dest, videoPart), joinPath(dest, audioPart), joinPath(dest, `${safe}.mp4`));
    // The halves are an implementation detail; leaving them behind would
    // just look like three copies of the same video.
    await api.fsDelete(joinPath(dest, videoPart)).catch(() => {});
    await api.fsDelete(joinPath(dest, audioPart)).catch(() => {});
  }

  async function saveInternetResultsToFolder(items: InternetDownloadItem[]) {
    const dir = await pickPath({ directory: true, multiple: false, title: "Save to folder" });
    if (!dir || Array.isArray(dir)) return;
    await downloadInternetItems(items, dir);
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
      selectAndReveal(name);
      setRenaming({ name, value: name });
    } catch (e) {
      setError(String(e));
    }
  }
  async function createNewFile() {
    const base = formatNameTemplate(appSettings.newFileNameTemplate || "untitled document");
    const isContacts = view === "contacts";
    // List-with-preview and Notes both exist specifically to write/read
    // markdown in place, so a new file made from either defaults to .md
    // instead of the generic .txt. Contacts wants a real (if empty) vCard,
    // not a blank .txt, since it opens straight into ContactEditForm below.
    const name = nextUntitledName(base, isContacts ? ".vcf" : view === "listPreview" || view === "notes" ? ".md" : ".txt");
    const path = joinPath(curDir, name);
    try {
      if (isContacts) {
        const vcf = serializeVCard(emptyVCard());
        inVault ? await api.vaultWriteText(path, vcf) : await api.fsWriteText(path, vcf);
      } else {
        inVault ? await api.newFile(path) : await api.fsNewFile(path);
      }
      await refresh();
      selectAndReveal(name);
      if (isContacts) {
        setMobileEditorTarget({
          entry: { name, is_dir: false, size: 0, mtime: Date.now() / 1000 },
          fullPath: path,
          inVault,
        });
      } else if (view !== "listPreview") {
        // In listPreview, selecting the new note is enough -- the effect
        // below picks it up and opens it in the preview pane ready to type
        // into, so there's no separate inline-rename step to interrupt that.
        setRenaming({ name, value: name });
      }
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
    // Folders are gated too: a sensitive folder's listing (names, sizes)
    // is content -- it must not show in the preview pane without re-auth.
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

  async function renameMobileEditorEntry(newName: string) {
    if (!mobileEditorTarget) return;
    const { entry, fullPath, inVault } = mobileEditorTarget;
    const dest = joinPath(parentPath(fullPath), newName);
    try {
      inVault ? await api.moveEntry(fullPath, dest) : await api.fsRename(fullPath, dest);
      await refresh();
      setMobileEditorTarget({ entry: { ...entry, name: newName }, fullPath: dest, inVault });
      selectOnly(newName);
    } catch (e) {
      setError(String(e));
    }
  }

  // ---- inline rename ----
  async function commitRename() {
    // Read + clear through the ref, not the closure: committing via Enter
    // unmounts the edit field, whose blur then calls the *previous*
    // render's closure where `renaming` was still set -- without this the
    // rename ran twice and the second pass failed on the now-gone source
    // (PickerView guards the same race with skipBlurCommitRef).
    const r = renamingRef.current;
    if (!r) return;
    renamingRef.current = null;
    setRenaming(null);
    const { name } = r;
    const value = r.value.trim();
    if (value === "" || value === name) return;
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
    const paths = names.map((name) => joinPath(curDir, name));
    try {
      // One call for the whole selection, with an Actions row: this used
      // to be one IPC round-trip per file on the main thread, so deleting
      // a few thousand froze the window with nothing to show for it.
      await api.fsTrashMany(paths, beginProgress(paths.length === 1 ? `Deleting "${names[0]}"` : `Deleting ${paths.length} items`));
      setSelected(new Set());
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  // Same desktop-trash-vs-mobile/vault-confirm split the regular context
  // menu's "Move to Trash"/"Delete" already uses -- the Notes grid's quick
  // trash-icon action just skips having to right-click first.
  function deleteNoteQuick(entry: Entry) {
    if (!inVault && !mobile) trashSelection([entry.name]);
    else setPending({ kind: "delete", names: [entry.name] });
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
    if (loc.kind === "vault" && mobile) {
      setSheetError("Nested vaults aren't supported on mobile yet.");
      return;
    }
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
      await refresh();
      selectAndReveal(name);
    } catch (e) {
      setSheetError(String(e));
    }
  }

  // ---- context menus ----
  // "Open With…" for one file: every app the desktop registered for its real
  // MIME type, fetched lazily (see `loadItems` on `MenuItem`) only once the
  // submenu is actually hovered, so right-clicking never has to wait on
  // `xdg-mime`/icon-theme work it might not need. Always ends in "Other
  // Application…", which opens the searchable full-app picker -- the
  // registered-handlers list is often just one entry, and "the app I want
  // isn't in this list" is otherwise a dead end (Windows' "Choose another
  // app" is the reference here).
  // `p` is a vault-relative path in a vault, an absolute one outside.
  function buildOpenWithItem(p: string): MenuItem {
    const otherItem = {
      label: "Other Application…",
      onClick: async () => {
        try {
          setOpenWithTarget(inVault ? await api.openPath(p) : p);
        } catch (e) {
          setError(String(e));
        }
      },
    };
    return {
      type: "submenu",
      label: "Open With…",
      loadItems: async () => {
        try {
          const abs = inVault ? await api.openPath(p) : p;
          const apps = await api.listAppsForPath(abs);
          const appItems: MenuItem[] =
            apps.length === 0
              ? [{ label: "No registered apps", disabled: true, onClick: () => {} }]
              : apps.map((a) => ({
                  label: a.is_default ? `${a.name} (Default)` : a.name,
                  onClick: () => {
                    api.openWith(abs, a.id).catch((e) => setError(String(e)));
                  },
                }));
          return [...appItems, { type: "separator" }, otherItem];
        } catch (e) {
          return [{ label: String(e), disabled: true, onClick: () => {} }, { type: "separator" }, otherItem];
        }
      },
    };
  }

  // Context menu for an entry identified only by its path -- used by search
  // hits and by the preview pane's folder listing, both of which show
  // entries from directories other than `curDir`. Can't reuse `entryMenu`:
  // that one is built against `curDir` + the `selected` name set + this
  // folder's cached `entries`/`tags`/git status, none of which apply to a
  // file in some other directory. What it offers instead is every action
  // that's purely a function of the path. `onChanged` fires after an action
  // that altered the path's parent listing (currently just Move to Trash).
  function pathMenu(e: React.MouseEvent, p: string, onChanged?: () => void): void {
    e.preventDefault();
    e.stopPropagation();
    const parent = parentPath(p);
    const name = baseName(p);
    const abs = inVault && loc.kind === "vault" ? joinPath(loc.root, p) : p;
    // Search only ever returns files, but resolve the real entry (for Open,
    // Get Info, Share) from its parent listing rather than fabricating one --
    // is_dir/is_vault/size/mtime all matter to those actions.
    const withEntry = (fn: (entry: Entry) => void) => async () => {
      try {
        const list = await listDir(parent, loc.kind);
        const found = list.find((en) => en.name === name);
        if (!found) {
          setError(`"${name}" no longer exists`);
          return;
        }
        fn(found);
      } catch (err) {
        setError(String(err));
      }
    };
    const items: MenuItem[] = [
      { label: "Open", onClick: withEntry((en) => activate(parent, en)) },
      // See the openWithItem comment in entryMenu -- no app-chooser
      // equivalent to enumerate on Android.
      ...(mobile ? [] : [buildOpenWithItem(p)]),
      {
        label: "Show in Folder",
        onClick: () => {
          if (loc.kind === "vault") go({ kind: "vault", root: loc.root, rel: parent });
          else go({ kind: "fs", path: parent });
          setSearchQuery("");
          setSearchResults(null);
          pendingRevealSelectRef.current = { dir: parent, name };
        },
      },
      { type: "separator" },
      {
        label: "Get Info",
        onClick: withEntry((en) =>
          setInfoTarget({
            entry: en,
            fullPath: p,
            kind: loc.kind,
            root: loc.kind === "vault" ? loc.root : undefined,
          })
        ),
      },
      { type: "separator" },
      {
        label: "Copy",
        onClick: () =>
          setClipboard({
            paths: [p],
            mode: "copy",
            kind: loc.kind,
            root: inVault ? loc.root : undefined,
          }),
      },
      {
        label: "Cut",
        onClick: () =>
          setClipboard({
            paths: [p],
            mode: "cut",
            kind: loc.kind,
            root: inVault ? loc.root : undefined,
          }),
      },
      {
        label: "Copy Absolute Path",
        onClick: async () => {
          try {
            await navigator.clipboard.writeText(abs);
          } catch (err) {
            setError(String(err));
          }
        },
      },
      { type: "separator" },
      { label: "Share…", onClick: withEntry((en) => shareFile(en, parent)) },
      {
        label: pinnedPaths.has(p) ? "Unpin" : "Pin",
        onClick: () => togglePin(p),
      },
    ];
    if (!inVault) {
      items.push(
        { type: "separator" },
        {
          label: "Move to Trash",
          danger: true,
          onClick: async () => {
            try {
              await api.fsTrash(p);
              onChanged?.();
            } catch (err) {
              setError(String(err));
            }
          },
        }
      );
    }
    setMenu({ x: e.clientX, y: e.clientY, items });
  }

  function entryMenu(e: React.MouseEvent, entry: Entry): void {
    e.preventDefault();
    e.stopPropagation();
    // On mobile, the only way this fires at all is a long-press (there's
    // no right-click) -- and a long-press on a file grid is, on every
    // comparable app (Google Photos/Files, iOS Files), the entry point
    // into multi-select, landing directly on "this item selected, ready
    // to tap more" rather than a menu you have to read and tap "Select"
    // in first. Reported directly as hard to use, with an explicit ask
    // for the best real UX here, not the least code -- this is that:
    // one gesture to start selecting more than one file, no extra tap.
    // Once already selecting, a long-press instead reaches this same
    // menu as before (below), since that's the only way to get bulk
    // actions for more than one file without a dedicated action bar.
    if (mobile && !selectionMode) {
      setSelectionMode(true);
      selectOnly(entry.name);
      return;
    }
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

    // Everything reached less often than Open/Rename/Duplicate/Cut-Copy/
    // Trash lives in one "More" submenu (Create Shortcut, Use as Template,
    // Pin, Convert To, Resize, Security, Tag Color) instead of flooding
    // the top-level menu with every action this app can do to a file.
    const moreItems: MenuItem[] = [];

    const duplicateItem: MenuItem = {
      label: "Duplicate",
      shortcut: "⌘D",
      disabled: many,
      onClick: () => duplicate(entry),
    };
    // Every registered app for this file's real MIME type, not just the
    // default `activate` opens -- fetched lazily (see `loadItems` on
    // `MenuItem`) only once this submenu is actually hovered, so right-
    // clicking a file never has to wait on `xdg-mime`/icon-theme work it
    // might not even need. Not offered for a directory, an already-vault-
    // encrypted `.vlt` file (nothing but this app can make sense of those
    // bytes), or a nested vault -- same "no stable real path yet" reason
    // Vault Settings skips those.
    // Enumerating registered apps (list_apps_for_path) is a Linux desktop
    // API (xdg-mime + .desktop scanning) with no Android equivalent --
    // Android's answer is an Intent chooser, a different UI shape entirely
    // (not a submenu at all), so this hides rather than show a dead one.
    const openWithItem: MenuItem | null =
      !mobile && !many && !entry.is_dir && !isEncryptedFile && !(entry.is_vault && !inVault)
        ? buildOpenWithItem(path)
        : null;

    // Both actions shell out (a terminal to run it, an editor binary to
    // edit it) -- neither exists on Android, so this offers the plain
    // Open/Get Info/etc menu below instead of two guaranteed-to-fail items.
    const runnableShellScript = !mobile && !many && !inVault && isShellScript;
    const items: MenuItem[] = runnableShellScript
      ? [
          { label: "Run", onClick: () => runScript(path) },
          { label: "Edit", onClick: () => editScript(path) },
          ...(openWithItem ? [openWithItem] : []),
          { type: "separator" },
          {
            label: "Rename",
            disabled: many,
            onClick: () => setRenaming({ name: entry.name, value: entry.name }),
          },
          duplicateItem,
        ]
      : [
          // The most obvious action leads the menu: a zip's is Decompress,
          // same as Run leads for scripts and Open Vault for vaults.
          ...(!many && isZip
            ? [{ label: "Decompress", onClick: () => decompressEntry(entry) } as MenuItem]
            : []),
          {
            label: entry.is_vault ? "Open Vault" : isEncryptedFile ? "Decrypt and Open…" : "Open",
            shortcut: "⌘O",
            disabled: many,
            onClick: () => activate(curDir, entry),
          },
          ...(openWithItem ? [openWithItem] : []),
          { type: "separator" },
          {
            label: "Rename",
            disabled: many,
            onClick: () => setRenaming({ name: entry.name, value: entry.name }),
          },
          duplicateItem,
        ];
    if (!many && !inVault) {
      moreItems.push({ label: "Create Shortcut", onClick: () => createShortcut(entry) });
    }
    if (!many && entry.is_dir && mobile) {
      moreItems.push({ label: "Add to Home Screen", onClick: () => addFolderShortcut(entry) });
    }
    if (!many && !inVault && !entry.is_dir) {
      moreItems.push({ label: "Use as Template", onClick: () => useAsTemplate(entry) });
    }
    // Nested vaults (inVault && entry.is_vault) aren't offered this here --
    // their real root only exists as an ephemeral FUSE path once opened, so
    // there's no stable key to save these settings under before that.
    if (!many && entry.is_vault && !inVault) {
      moreItems.push({
        label: "Vault Settings…",
        onClick: () => setVaultSettingsTarget({ root: path, canAutoUnlock: true }),
      });
    }
    if (!many && entry.is_dir && !entry.is_vault && !mobile) {
      items.push(
        { type: "separator" },
        {
          label: "Open in Terminal",
          onClick: async () => {
            const p = inVault ? await api.openPath(path) : path;
            openTerminalAt(p);
          },
        }
      );
    }
    items.push(
      { type: "separator" },
      { label: "Cut", shortcut: "⌘X", onClick: () => cutSel(entry) },
      { label: "Copy", shortcut: "⌘C", onClick: () => copySel(entry) },
      { label: "Copy Absolute Path", onClick: () => copyEntryPaths(targetNames) },
      { type: "separator" }
    );
    if (!many && !entry.is_dir) {
      items.push({ label: "Share…", onClick: () => shareFile(entry) }, { type: "separator" });
    }
    if (!many) {
      moreItems.push({
        label: pinnedPaths.has(path) ? "Unpin" : "Pin",
        onClick: () => togglePin(path),
      });
    }

    // A single zip gets Decompress as the first menu item instead of a
    // Compress submenu -- re-compressing something already compressed is
    // never the obvious action.
    if (many || !isZip) {
      items.push({
        type: "submenu",
        label: "Compress",
        items: [
          {
            label: many ? `Compress ${targetNames.length} Items` : `Compress "${entry.name}"`,
            onClick: () => compressSelection(targetNames),
          },
          { label: "Compress…", onClick: () => setCompressTarget(targetNames) },
        ],
      });
    }

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
        // ImageMagick (`convert`) doesn't exist on Android; the raster-to-
        // raster items above stay (pure Rust, actually work there).
        if (!mobile) convertItems.push({ label: "PDF", onClick: () => runImageToPdf(entry) });
        moreItems.push({ type: "submenu", label: "Convert To", items: convertItems });
      } else if (kind === "pdf" && !inVault) {
        // poppler's `pdftoppm` doesn't exist on Android -- no availability
        // flag to gate on the way ffmpeg/libreoffice do below since there's
        // no `which` either, so this checks the platform directly.
        const pdfItems: MenuItem[] = mobile
          ? []
          : [{ label: "Images (JPG, one per page)", onClick: () => runPdfToImages(entry) }];
        if (libreofficeAvailable) {
          pdfItems.push({ label: "Word Document (.docx)", onClick: () => runOfficeConvert(entry, "docx") });
        }
        if (pdfItems.length) moreItems.push({ type: "submenu", label: "Convert To", items: pdfItems });
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
      // Every option here shells out to a binary (rclone/git/unison/
      // syncthing) that doesn't exist on Android.
      if (!mobile) {
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
      }
      moreItems.push({ label: "Change Icon…", onClick: () => setIconTarget(path) });
      if (!mobile) {
        moreItems.push(
          frozenPaths.has(path)
            ? { label: "Unfreeze…", onClick: () => setUnfreezeTarget(path) }
            : { label: "Freeze…", onClick: () => setPending({ kind: "freeze", entry }) }
        );
        // Shells out to the `claude` CLI (see reorganize.rs) -- no
        // equivalent on Android.
        moreItems.push({ label: "Reorganize & Clean…", onClick: () => setReorganizeTarget(path) });
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
    // Column and split-preview both divide the width into several panes --
    // fine on a desktop window, but there's no room for that on a phone,
    // and neither has touch-specific layout (column view is explicitly
    // excluded from arrow-key nav, and nothing narrows it below its
    // desktop column widths).
    const options: { key: View; label: string }[] = [
      { key: "icon", label: "Icons" },
      { key: "list", label: "List" },
      ...(mobile ? [] : [{ key: "column" as const, label: "Columns" }, { key: "listPreview" as const, label: "List with Preview" }]),
      // Experiment: markdown files as Keep-style note cards (a content
      // preview instead of just an icon+name), tap opens the same
      // full-screen editor mobile's plain file-open already uses. Not
      // useful for a folder that's mostly non-markdown, but folders that
      // are basically a notes stash don't have anything like this in
      // any other view.
      { key: "notes", label: "Notes" },
      { key: "contacts", label: "Contacts" },
      // Experiment: a folder of PDFs/ebooks as an actual wooden bookshelf
      // (see LibraryShelf) instead of an icon grid -- explicitly a first
      // pass at the idea, not a finished look.
      { key: "library", label: "Library" },
    ];
    const items: MenuItem[] = options.map((o) => ({
      label: view === o.key && !showDigest ? `✓ ${o.label}` : o.label,
      // Picking a view here is the one consistent escape hatch this app
      // already has for "no, show me the real files" -- the auto-digest
      // (see SavedSearchDigest) has its own inline "View as files" link
      // too, but that's only discoverable once you're already looking at
      // it. Dismissing here as well means the toolbar's normal view
      // switcher works as the way out, same as a user would expect from
      // every other view choice in this menu.
      onClick: () => {
        setDigestDismissed(true);
        setView(o.key);
      },
    }));
    // Per-folder pin. Hidden on My Computer (not a folder) and in column
    // view (which ignores pins while browsing -- see the restore effect).
    if (!showMyComputer && view !== "column") {
      items.push({ type: "separator" });
      items.push({
        label: viewPinned ? "✓ Always Open in This View" : "Always Open in This View",
        onClick: toggleViewPin,
      });
    }
    const pinCount = Object.keys(pinnedViewPrefs).length;
    if (pinCount > 0) {
      items.push({
        label: `Reset All Folder Views (${pinCount})`,
        onClick: () => setPinnedViewPrefs({}),
      });
    }
    // On mobile this button lives in the bottom toolbar, right above the
    // tab bar -- anchoring the menu below it (like on desktop) means it
    // opens hard against the bottom edge, gets clamped back upward by
    // ContextMenu's own viewport check, and lands roughly where the button
    // already was: low and off to the right. Opening upward from the
    // button's own top edge (anchorBottom) instead keeps it right where a
    // thumb already is, just clear of the tab bar underneath -- a modest
    // correction, not moving it to the opposite corner of the screen
    // (confirmed too far the other way: "se fue arriba a la izquierda de
    // todo").
    if (mobile) {
      setMenu({ x: Math.max(12, r.left - 20), y: r.top - 4, items, anchorBottom: true });
    } else {
      setMenu({ x: r.left, y: r.bottom + 4, items });
    }
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
    if (gitRoot && !mobile) {
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
    if (!mobile && !inVault) {
      // Same action as the per-folder "More" submenu entry -- here the
      // target is the folder you're standing in (curDir) rather than one
      // of its children, since this fired on empty space, not an entry.
      items.push({ label: "Reorganize & Clean…", onClick: () => setReorganizeTarget(curDir) });
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
  // Collapsing by crumb COUNT alone doesn't guarantee the row fits: 3 long
  // folder names can still overflow the bar's actual pixel width, and with
  // a fixed set of visible crumbs there's no further abbreviation to fall
  // back to. So the bar stays scrollable (not clipped) and this keeps it
  // scrolled to the right end, where the current folder -- the crumb that
  // actually matters -- lives, instead of it silently sitting off-screen.
  useEffect(() => {
    const el = breadcrumbRef.current;
    if (el) el.scrollLeft = el.scrollWidth;
  }, [loc, mobile]);

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
    // Permission check for shared-storage paths now lives in `go()` itself
    // (covers every navigation path, not just this one).
    let isVault = false;
    try {
      isVault = await api.fsIsVault(path);
    } catch {
      /* ignore -- treat as a plain folder */
    }
    go(isVault ? { kind: "vault", root: path, rel: "" } : { kind: "fs", path });
  }
  // Bottom-tab-bar "Vaults" button (mobile): there's no persisted list of
  // "every vault the user has" -- `vaultSettings` is keyed by root path but
  // exists for per-vault UI prefs, not as a registry, so this is a proxy:
  // every currently-unlocked root, plus every root that's ever had its
  // settings touched (the closest thing to "vaults I've used before").
  // Jumping into an entry that's since been locked re-prompts for its
  // password the same way tapping it in a folder listing would.
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

  function syncInfoFor(entry: Entry): {
    badge: "git" | "drive" | "local" | null;
    state: "syncing" | "synced" | "verified" | "pending" | null;
  } {
    const path = joinPath(curDir, entry.name);
    // Checksum-verified state from the poll above. Falls back through the
    // path-based badge logic when absent (git/local pairs, or no check yet).
    const vstate = verifyStates.get(entry.name);
    const hit = (entry.is_dir ? syncRootFor(path) : null) ?? syncRootFor(curDir);
    if (!hit) {
      // Inside a vault the fs-path walk can't match (paths here are
      // vault-relative) -- the verify poll is what knows this entry's
      // ciphertext is under a Drive pair, so it alone drives the badge.
      // "unknown" = under a pair but no check result yet: show the plain
      // static badge right away rather than nothing until the first
      // check lands.
      if (vstate === "verified") return { badge: "drive", state: "verified" };
      if (vstate === "pending") return { badge: "drive", state: "pending" };
      if (vstate === "unknown") return { badge: "drive", state: null };
      return { badge: null, state: null };
    }
    const { badge, root } = hit;
    if (syncingPaths.has(root) || syncingPaths.has(path)) return { badge, state: "syncing" };
    if (justSyncedPaths.has(root) || justSyncedPaths.has(path)) return { badge, state: "synced" };
    if (badge === "drive") {
      if (vstate === "verified") return { badge, state: "verified" };
      if (vstate === "pending") return { badge, state: "pending" };
    }
    return { badge, state: null };
  }

  // "List with Preview" reuses the exact same row rendering as plain list
  // view (entryView forces the "list" CSS/thumbnail-size path for it) --
  // only the click handler and the pane alongside it differ.
  const entryView: View = view === "listPreview" ? "list" : view;
  function renderListBody() {
    return (
      <div className={`entries-wrap ${entryView}`}>
        {/* Sort arrow points the way the values run *down* the list: A→Z (and
            oldest→newest, smallest→largest) is ▼, since reading downward is
            reading forward through the order. It was ▲ for ascending, which
            read as "the list runs upward" -- backwards from what you see. */}
        {(view === "list" || view === "listPreview") && entries.length > 0 && (
          <div className={`list-header ${view === "listPreview" ? "compact" : ""}`}>
            <span className="lh-spacer" />
            <span className={`lh-name ${sortKey === "name" ? "on" : ""}`} onClick={() => toggleSort("name")}>
              Name {sortKey === "name" && (sortDir === 1 ? "▼" : "▲")}
            </span>
            {view !== "listPreview" && (
              <>
                <span className={`lh-date ${sortKey === "date" ? "on" : ""}`} onClick={() => toggleSort("date")}>
                  Date Modified {sortKey === "date" && (sortDir === 1 ? "▼" : "▲")}
                </span>
                <span className={`lh-size ${sortKey === "size" ? "on" : ""}`} onClick={() => toggleSort("size")}>
                  Size {sortKey === "size" && (sortDir === 1 ? "▼" : "▲")}
                </span>
                <span className={`lh-kind ${sortKey === "kind" ? "on" : ""}`} onClick={() => toggleSort("kind")}>
                  Type {sortKey === "kind" && (sortDir === 1 ? "▼" : "▲")}
                </span>
              </>
            )}
          </div>
        )}
        <div
          className={`entries ${entryView} ${view === "listPreview" ? "compact" : ""}`}
          onTouchStart={onEntriesTouchStart}
          onTouchMove={onEntriesTouchMove}
          onTouchEnd={onEntriesTouchEnd}
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
              mobile={mobile}
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
                // In selection mode a tap just toggles membership -- it
                // must NOT also open the file, or there'd be no way to
                // pick a second item without opening the first one too.
                if (mobile && selectionMode) {
                  e.stopPropagation();
                  toggle(entry.name);
                  return;
                }
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
        // Only meaningful on desktop (dragging the window by an empty
        // sidebar area); on mobile it's a touch surface (favorites list,
        // swipe-to-close drawer) that this attribute would otherwise
        // compete with for the same gesture.
        data-tauri-drag-region={mobile ? undefined : true}
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
          {/* Collapsing this section has no real use on a phone-width
              sidebar (there's no icon-only mode worth collapsing into
              there) -- desktop keeps the toggle. */}
          {!mobile && (
            <button
              type="button"
              className="sidebar-section-collapse-btn"
              title={favCollapsed ? "Expand Favorites" : "Collapse Favorites"}
              onClick={() => setFavCollapsed((v) => !v)}
            >
              {favCollapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
            </button>
          )}
        </div>
        {/* Drive enumeration (df/lsblk) is meaningless inside an Android
            app sandbox -- there's no second disk to show, and nothing here
            resolves to a real block device the app could see anyway. So
            mobile gets a one-tap jump straight to the phone's real shared
            storage root instead -- "My Computer"'s actual equivalent there
            isn't a drive list, it's just the one storage volume a phone
            has. */}
        {mobile ? (
          <div
            className={`sidebar-item ${!showMyComputer && !showInternet && loc.kind === "fs" && loc.path === PHONE_STORAGE_PATH ? "active" : ""}`}
            onClick={() => go({ kind: "fs", path: PHONE_STORAGE_PATH })}
          >
            <span className="sidebar-ico place">
              <SmartphoneGlyph size={22} />
            </span>
            My Device
          </div>
        ) : (
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
        )}
        {!mobile && (
          <div
            className={`sidebar-item ${showDevice ? "active" : ""} ${favCollapsed ? "icon-only" : ""}`}
            title={favCollapsed ? "My Device" : undefined}
            onClick={() => {
              setShowMyComputer(false);
              setShowInternet(false);
              setFreeUpSpaceOpen(false);
              setShowDevice(true);
            }}
          >
            <span className="sidebar-ico place">
              <SmartphoneGlyph size={22} />
            </span>
            {!favCollapsed && "My Device"}
          </div>
        )}
        {/* Experimental (see InternetView) -- fake "Videos"/"Images"
            folders backed by live search, not a real path. */}
        <div
          className={`sidebar-item ${showInternet ? "active" : ""} ${favCollapsed ? "icon-only" : ""}`}
          title={favCollapsed ? "Internet" : undefined}
          onClick={openInternet}
        >
          <span className="sidebar-ico place">🌐</span>
          {!favCollapsed && "Internet"}
        </div>
        {favorites.map((f, i) => {
          const active = !showMyComputer && !showInternet && loc.kind === "fs" && loc.path === f.path;
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
                if (dragInternetItems.current) {
                  const items = dragInternetItems.current;
                  dragInternetItems.current = null;
                  downloadInternetItems(items, f.path);
                  return;
                }
                if (loc.kind === "fs") dropInto(f.path);
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                const items: MenuItem[] = [
                  { label: "Change Icon…", onClick: () => setIconTarget(f.path) },
                ];
                if (f.path !== "/" && !inVault && !mobile) {
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
                // A favorite is only known to be a vault here once it's been
                // unlocked or configured at least once -- a never-touched
                // locked vault favorite still gets this via its own entry's
                // context menu (entryMenu, above), which always knows
                // `entry.is_vault`.
                if (unlockedRoots.has(f.path) || vaultSettings[f.path]) {
                  items.push(
                    {
                      label: "Vault Settings…",
                      onClick: () => setVaultSettingsTarget({ root: f.path, canAutoUnlock: true }),
                    },
                    { type: "separator" }
                  );
                }
                if (unlockedRoots.has(f.path)) {
                  items.push(
                    { label: "Lock", danger: true, onClick: () => lockVaultRoot(f.path) },
                    { type: "separator" }
                  );
                }
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
                  <RetryImg className="sidebar-fav-img" src={customIconUrl(f.icon)!} />
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

        <div
          className={`sidebar-item ${favCollapsed ? "icon-only" : ""}`}
          title={favCollapsed ? "Free Up Space" : undefined}
          onClick={() => {
            setShowMyComputer(false);
            setShowInternet(false);
            setShowDevice(false);
            setFreeUpSpaceOpen(true);
          }}
        >
          <span className="sidebar-ico place">🧹</span>
          {!favCollapsed && "Free Up Space"}
        </div>

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
                      items: [
                        {
                          label: "Vault Settings…",
                          onClick: () => setVaultSettingsTarget({ root, canAutoUnlock: true }),
                        },
                        { type: "separator" },
                        { label: "Lock", danger: true, onClick: () => lockVaultRoot(root) },
                      ],
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
        <div className="titlebar toolbar" data-tauri-drag-region={mobile ? undefined : true}>
          {mobile && selectionMode ? (
            <>
              <button
                className="tool-btn wide-btn"
                onClick={() => {
                  setSelectionMode(false);
                  setSelected(new Set());
                }}
              >
                ✕ Cancel
              </button>
              <div className="toolbar-title">{selected.size} selected</div>
              <button
                className="tool-btn wide-btn"
                onClick={() =>
                  setSelected(
                    selected.size === entries.length ? new Set() : new Set(entries.map((en) => en.name))
                  )
                }
              >
                {selected.size === entries.length ? "Deselect All" : "Select All"}
              </button>
              <button className="tool-btn wide-btn" onClick={moveSelectionTo}>
                Move to…
              </button>
              <button
                className="tool-btn wide-btn"
                onClick={(e) => {
                  const anyName = [...selected][0];
                  const anyEntry = entries.find((en) => en.name === anyName);
                  if (anyEntry) entryMenu(e, anyEntry);
                }}
              >
                ⋯
              </button>
            </>
          ) : (
          <>
          <div className="nav-buttons">
            {/* Menu moved to the bottom tab bar on mobile (see .mobile-tabbar). */}
            <button
              className="tool-btn"
              onClick={goBack}
              disabled={!showMyComputer && !showInternet && histIdx === 0}
              aria-label="Back"
            >
              <ChevronLeft />
            </button>
            <button
              className="tool-btn"
              onClick={goForward}
              disabled={!showMyComputer && !showInternet && histIdx >= history.length - 1}
              aria-label="Forward"
            >
              <ChevronRight />
            </button>
            {/* Reported as unused on mobile (a parent breadcrumb segment
                already does the same "go up" job, right above this
                toolbar) -- kept for desktop, where it's a real toolbar
                convention. */}
            {!mobile && (
              <button className="tool-btn" onClick={goUp} disabled={!canGoUp} aria-label="Up">
                <ChevronUp />
              </button>
            )}
            <button className="tool-btn refresh-btn" onClick={() => refresh()} aria-label="Refresh">
              <RefreshGlyph />
            </button>
          </div>
          <div className="toolbar-title">
            {showMyComputer
              ? "My Computer"
              : showInternet
                ? "Internet"
                : crumbs.length
                  ? crumbs[crumbs.length - 1].label
                  : "System"}
          </div>
          <button
            className={`tool-btn cluster-start ${mobile ? "" : "wide-btn"}`}
            onClick={createNewFile}
            aria-label="New File"
            title="New File"
          >
            {mobile ? <NewFileGlyph size={19} /> : "+ File"}
          </button>
          <button
            className={`tool-btn ${mobile ? "" : "wide-btn"}`}
            onClick={createNewFolder}
            aria-label="New Folder"
            title="New Folder"
          >
            {mobile ? <NewFolderGlyph size={19} /> : "+ Folder"}
          </button>
          <button
            className={`tool-btn ${mobile ? "" : "wide-btn"}`}
            onClick={paste}
            disabled={!canPaste}
            aria-label="Paste"
            title="Paste"
          >
            {mobile ? <PasteGlyph size={19} /> : "Paste"}
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
          {/* Preferences moved to the bottom tab bar on mobile. */}
          {!mobile && (
            <button
              className="tool-btn"
              aria-label="Preferences"
              title="Preferences"
              onClick={() => setSettingsOpen(true)}
            >
              <SettingsGlyph />
            </button>
          )}
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
            // Trigger moved to the bottom tab bar on mobile -- the expanded
            // field above still renders there when `searchExpanded` is set
            // from that button, this is just the collapsed toggle icon.
            !mobile && (
              <button
                className="tool-btn search-toggle"
                aria-label="Search"
                title="Search"
                onClick={() => setSearchExpanded(true)}
              >
                <SearchGlyph size={18} />
              </button>
            )
          )}
          </>
          )}
        </div>
        {error && (
          <div className="error-bar" title="Tap to copy" onClick={copyError}>
            <span className="error-text">{error}</span>
            <span className="error-copy" onClick={(e) => { e.stopPropagation(); copyError(); }}>
              {errorCopied ? <CheckGlyph size={13} /> : <CopyGlyph size={13} />}
            </span>
            <span
              className="error-x"
              onClick={(e) => {
                e.stopPropagation();
                setErrorRaw("");
              }}
            >
              ✕
            </span>
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
          onContextMenu={showMyComputer || showInternet ? undefined : backgroundMenu}
          onMouseDown={onContentMouseDown}
          onTouchStart={onContentTouchStart}
          onTouchMove={onContentTouchMove}
          onTouchEnd={onContentTouchEnd}
        >
          {(pullDist > 0 || pullRefreshing) && (
            <div
              className={`pull-refresh ${pullRefreshing ? "spin" : ""}`}
              style={{
                height: pullRefreshing ? PULL_THRESHOLD : pullDist,
                opacity: pullRefreshing ? 1 : Math.min(1, pullDist / PULL_THRESHOLD),
              }}
            >
              <span
                className="pull-refresh-spinner"
                style={
                  pullRefreshing
                    ? undefined
                    : { transform: `rotate(${(pullDist / PULL_THRESHOLD) * 360}deg)` }
                }
              />
            </div>
          )}
          {showDevice ? (
            <DeviceView
              onOpenDrive={(d) => d.mountpoint && go({ kind: "fs", path: d.mountpoint })}
              onFreeUpSpace={() => {
                setShowDevice(false);
                setFreeUpSpaceOpen(true);
              }}
            />
          ) : freeUpSpaceOpen ? (
            <FreeUpSpaceView
              favPaths={favPaths}
              home={home ?? ""}
              onDeleted={() => refresh()}
              beginProgress={beginProgress}
            />
          ) : showMyComputer ? (
            <MyComputerView
              drives={drives}
              error={drivesError}
              onOpenDrive={(d) => d.mountpoint && go({ kind: "fs", path: d.mountpoint })}
              onMenu={driveMenu}
            />
          ) : showInternet ? (
            <InternetView
              initial={internetInitial}
              onSave={saveInternetSearch}
              mobile={mobile}
              onDragResults={(items) => {
                dragInternetItems.current = items;
              }}
              onSaveToFolder={saveInternetResultsToFolder}
              onDownloadVideos={downloadInternetVideos}
              onOpenFolder={(path) => go({ kind: "fs", path })}
              onRegisterBack={(fn) => {
                internetBackRef.current = fn;
              }}
            />
          ) : searchResults !== null && view === "contacts" ? (
            <ContactsGrid
              entries={searchEntryList}
              curDir={curDir}
              inVault={inVault}
              pathFor={(entry) => searchPathByEntry.get(entry) ?? ""}
              header={searchResultsHeader(searchQuery, searchResults.length)}
              emptyMessage={`No contacts found for “${searchQuery}”.`}
              onEditContact={(entry, fullPath) =>
                withSensitive(fullPath, () => setMobileEditorTarget({ entry, fullPath, inVault }))
              }
              onActivateOther={(entry) => activate(parentPath(searchPathByEntry.get(entry) ?? ""), entry)}
              onMenu={(e, entry) => pathMenu(e, searchPathByEntry.get(entry) ?? "", () => runSearch(searchQuery))}
            />
          ) : searchResults !== null && view === "library" ? (
            <LibraryShelf
              entries={searchEntryList}
              curDir={curDir}
              inVault={inVault}
              pathFor={(entry) => searchPathByEntry.get(entry) ?? ""}
              header={searchResultsHeader(searchQuery, searchResults.length)}
              emptyMessage={`No results for “${searchQuery}”.`}
              onOpen={(entry) => activate(parentPath(searchPathByEntry.get(entry) ?? ""), entry)}
              onMenu={(e, entry) => pathMenu(e, searchPathByEntry.get(entry) ?? "", () => runSearch(searchQuery))}
            />
          ) : searchResults !== null ? (
            <SearchResults
              query={searchQuery}
              results={searchResults}
              entries={searchEntries}
              inVault={inVault}
              onOpen={(p) => {
                if (loc.kind === "vault") go({ kind: "vault", root: loc.root, rel: parentPath(p) });
                else go({ kind: "fs", path: parentPath(p) });
              }}
              onMenu={(e, p) => pathMenu(e, p, () => runSearch(searchQuery))}
            />
          ) : showDigest ? (
            <SavedSearchDigest
              dir={curDir}
              entries={entries}
              ext={savedSearchExt as "ytsearch" | "imgsearch" | "booksearch"}
              onDismiss={() => setDigestDismissed(true)}
              onOpenFile={(entry) => activate(curDir, entry)}
            />
          ) : view === "column" ? (
            <ColumnView
              chain={columnChain()}
              list={(dir) => listDir(dir, loc.kind)}
              inVault={inVault}
              root={inVault ? loc.root : undefined}
              onActivate={activate}
              // The full entryMenu is hard-bound to curDir's selection/
              // entries/tags, so it's only correct for the *current*
              // column; other columns' entries get the path-based menu
              // (previously they got entryMenu built against the wrong
              // dir, so e.g. Open/Trash hit a same-named sibling in
              // curDir or errored).
              onMenu={(e, dir, entry) =>
                dir === curDir ? entryMenu(e, entry) : pathMenu(e, joinPath(dir, entry.name))
              }
              previewEntry={previewEntry}
              onSelectFile={(dir, entry) =>
                withSensitive(joinPath(dir, entry.name), () => {
                  setPreviewEntry({ dir, entry });
                  // Keep the REAL selection in step with the highlighted row --
                  // copy/delete/Enter/statusbar act on `selected`, and letting
                  // it lag behind the preview highlight meant they silently
                  // targeted the previously-selected file.
                  if (dir === curDir) selectOnly(entry.name);
                })
              }
              cutPaths={clipboard?.mode === "cut" && clipboard.kind === loc.kind ? clipboard.paths : undefined}
            />
          ) : view === "notes" ? (
            <NotesGrid
              entries={entries}
              curDir={curDir}
              inVault={inVault}
              tags={tags}
              pinnedPaths={pinnedPaths}
              onOpenNote={(entry, fullPath) =>
                withSensitive(fullPath, () => setMobileEditorTarget({ entry, fullPath, inVault }))
              }
              onActivateOther={(entry) => activate(curDir, entry)}
              onMenu={(e, entry) => entryMenu(e, entry)}
              onDelete={deleteNoteQuick}
              onTogglePin={togglePin}
              onSetColor={setTagFor}
            />
          ) : view === "contacts" ? (
            <ContactsGrid
              entries={entries}
              curDir={curDir}
              inVault={inVault}
              selection={selection}
              renaming={renaming}
              onRenameChange={(v) => setRenaming((r) => (r ? { ...r, value: v } : r))}
              onRenameCommit={commitRename}
              onRenameCancel={() => setRenaming(null)}
              onEditContact={(entry, fullPath) =>
                withSensitive(fullPath, () => setMobileEditorTarget({ entry, fullPath, inVault }))
              }
              onActivateOther={(entry) => activate(curDir, entry)}
              onMenu={(e, entry) => entryMenu(e, entry)}
              onFilesChanged={refresh}
            />
          ) : view === "library" ? (
            <LibraryShelf
              entries={entries}
              curDir={curDir}
              inVault={inVault}
              onOpen={(entry) => activate(curDir, entry)}
              onMenu={(e, entry) => entryMenu(e, entry)}
            />
          ) : view === "listPreview" ? (
            <div className="list-preview-split">
              {renderListBody()}
              <FilePreviewPane
                target={previewEntry}
                inVault={inVault}
                root={inVault ? loc.root : undefined}
                onRename={renamePreviewEntry}
                textEditorExts={textEditorExts}
                onOpenInEditor={(ext) => setExtOpensInEditor(ext, true)}
                reloadKey={previewReloadKey}
                onChildActivate={(child) => {
                  if (!previewEntry) return;
                  activate(joinPath(previewEntry.dir, previewEntry.entry.name), child);
                }}
                onChildMenu={(e, child) => {
                  if (!previewEntry) return;
                  const childPath = joinPath(joinPath(previewEntry.dir, previewEntry.entry.name), child.name);
                  pathMenu(e, childPath, () => setPreviewReloadKey((k) => k + 1));
                }}
              />
            </div>
          ) : (
            renderListBody()
          )}
        </div>

        <div
          className="breadcrumb-bar"
          ref={breadcrumbRef}
          onClick={(e) => {
            if (showMyComputer || showInternet) return;
            if (!(e.target as HTMLElement).closest(".crumb, .breadcrumb-copy")) beginEditPath();
          }}
        >
          {showMyComputer ? (
            <div className="breadcrumb-crumbs">
              <span className="crumb-group">
                <span className="crumb">My Computer</span>
              </span>
            </div>
          ) : showInternet ? (
            <div className="breadcrumb-crumbs">
              <span className="crumb-group">
                <span className="crumb">Internet</span>
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
          <ProgressPanel ops={progressOps} onCancel={cancelProgress} mobile={mobile} />
          {inVault && <span className="status-loc">🔒 Encrypted Vault</span>}
        </div>
      </div>

      {mobile && (
        <nav className="mobile-tabbar">
          <button className="mobile-tab" onClick={() => setSidebarOpen((v) => !v)} aria-label="Menu">
            <MenuGlyph size={22} />
            <span>Menu</span>
          </button>
          <button className="mobile-tab" onClick={() => setSearchExpanded(true)} aria-label="Search">
            <SearchGlyph size={22} />
            <span>Search</span>
          </button>
          <button
            className="mobile-tab"
            // "My Computer"'s mobile equivalent -- a phone has one storage
            // volume, not a drive list, so Home jumps straight to its real
            // root instead (same target as the "My Device" sidebar entry).
            onClick={() => go({ kind: "fs", path: PHONE_STORAGE_PATH })}
            aria-label="Home"
          >
            <DiskGlyph size={22} />
            <span>Home</span>
          </button>
          <button className="mobile-tab" onClick={() => setSettingsOpen(true)} aria-label="Settings">
            <SettingsGlyph size={22} />
            <span>Settings</span>
          </button>
        </nav>
      )}

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
          defaultTimeout={appSettings.sensitiveTimeout}
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
      {vaultSettingsTarget && (
        <VaultSettingsSheet
          name={baseName(vaultSettingsTarget.root)}
          initial={
            vaultSettings[vaultSettingsTarget.root] ?? {
              sensitive: false,
              autoLockMinutes: 15,
              autoUnlock: false,
            }
          }
          canAutoUnlock={vaultSettingsTarget.canAutoUnlock}
          onSave={(opts, password) => saveVaultSettings(vaultSettingsTarget.root, opts, password)}
          onCancel={() => setVaultSettingsTarget(null)}
        />
      )}
      {settingsOpen && (
        <SettingsScreen
          settings={appSettings}
          onChange={setAppSettings}
          onClose={() => setSettingsOpen(false)}
          mobile={mobile}
          onExportConfig={exportConfigToClipboard}
          onImportConfig={importConfigFromClipboard}
        />
      )}
      {mobileEditorTarget && (
        <div className="mobile-editor-screen">
          <div className="settings-screen-header">
            <button
              className="settings-back-btn"
              onClick={() => {
                setMobileEditorTarget(null);
                refresh();
              }}
              aria-label="Back"
            >
              <ChevronLeft size={18} />
              Back
            </button>
          </div>
          {mobileEditorTarget.entry.name.toLowerCase().endsWith(".vcf") ? (
            <ContactEditForm
              entry={mobileEditorTarget.entry}
              fullPath={mobileEditorTarget.fullPath}
              inVault={mobileEditorTarget.inVault}
              onRename={renameMobileEditorEntry}
            />
          ) : (
            <TextEditorPane
              entry={mobileEditorTarget.entry}
              fullPath={mobileEditorTarget.fullPath}
              inVault={mobileEditorTarget.inVault}
              onRename={renameMobileEditorEntry}
            />
          )}
        </div>
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
      {openWithTarget && (
        <OpenWithSheet
          path={openWithTarget}
          onClose={() => setOpenWithTarget(null)}
          onError={setError}
        />
      )}
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
      {reorganizeTarget && (
        <ReorganizeSheet
          path={reorganizeTarget}
          onDone={() => refresh()}
          onBackground={beginIndeterminate}
          onClose={() => setReorganizeTarget(null)}
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
          opensInEditor={editorExtOf(infoTarget.entry) === null ? null : textEditorExts.has(editorExtOf(infoTarget.entry) as string)}
          onSetOpensInEditor={(on) => {
            const ext = editorExtOf(infoTarget.entry);
            if (ext) setExtOpensInEditor(ext, on);
          }}
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
      {mediaViewer && (
        <MediaViewer
          gallery={mediaViewer.gallery}
          startIndex={mediaViewer.startIndex}
          onClose={() => setMediaViewer(null)}
          onDeleted={() => refresh()}
          onFileChanged={() => refresh()}
          mobile={mobile}
        />
      )}
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

  if (params.get("media") === "1") {
    let gallery: GalleryEntry[] = [];
    try {
      gallery = JSON.parse(params.get("items") ?? "[]");
    } catch {
      /* malformed items -- MediaWindow renders an empty gallery */
    }
    return <MediaWindow gallery={gallery} startIndex={Number(params.get("index") ?? "0")} />;
  }

  if (params.get("player") === "1") {
    let items: PlayerItem[] = [];
    try {
      items = JSON.parse(params.get("items") ?? "[]");
    } catch {
      /* malformed items -- PlayerWindow renders an empty playlist */
    }
    return (
      <PlayerWindow
        kind={params.get("kind") ?? "youtube"}
        items={items}
        startIndex={Number(params.get("index") ?? "0")}
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
    // Same cold-start IPC race as Explorer's own check -- see the long
    // comment there. Milder consequence here (a stray resize-cursor strip
    // on a touch device, not a broken layout), but the fix is identical
    // and just as cheap.
    let cancelled = false;
    async function detect() {
      for (const delay of [0, 300, 1000, 3000]) {
        if (delay) await new Promise((r) => setTimeout(r, delay));
        try {
          const v = await api.isMobilePlatform();
          if (!cancelled) setMobile(v);
          return;
        } catch {
          // keep retrying
        }
      }
    }
    detect();
    return () => {
      cancelled = true;
    };
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
