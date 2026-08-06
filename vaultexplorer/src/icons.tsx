import { ReactNode, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Entry, ENCRYPTED_FILE_EXT } from "./api";
// File/folder tile artwork: WhiteSur (macOS Big Sur style) icon theme,
// GPL-3.0 -- see assets/fileicons/NOTICE. Vite resolves each `.svg` import
// to a bundled asset URL, rendered with <img> below.
import folderIcon from "./assets/fileicons/folder.svg";
import imageIcon from "./assets/fileicons/image.svg";
import videoIcon from "./assets/fileicons/video.svg";
import audioIcon from "./assets/fileicons/audio.svg";
import pdfIcon from "./assets/fileicons/pdf.svg";
import archiveIcon from "./assets/fileicons/archive.svg";
import codeIcon from "./assets/fileicons/code.svg";
import textIcon from "./assets/fileicons/text.svg";
import genericIcon from "./assets/fileicons/generic.svg";

// WhiteSur folder-icon variants offered in the "Change Icon" picker
// (colored + semantic folders). import.meta.glob bundles every SVG in the
// dir as an asset URL, so adding a file to assets/foldericons/ is enough.
const folderIconModules = import.meta.glob("./assets/foldericons/*.svg", {
  eager: true,
  query: "?url",
  import: "default",
}) as Record<string, string>;

// key ("folder-documents") -> bundled url
const FOLDER_ICON_URLS: Record<string, string> = Object.fromEntries(
  Object.entries(folderIconModules).map(([path, url]) => [
    path.replace(/.*\/(.+)\.svg$/, "$1"),
    url,
  ])
);

function prettyFolderLabel(key: string): string {
  if (key === "folder") return "Blue";
  const color = key.match(/^(green|grey|orange|pink|purple|red|yellow)-folder$/);
  if (color) return color[1][0].toUpperCase() + color[1].slice(1);
  const m = key.match(/^folder-(.+)$/);
  const base = (m ? m[1] : key).replace(/-/g, " ");
  return base.charAt(0).toUpperCase() + base.slice(1);
}

// A custom icon value is either an emoji (rendered as text) or a WhiteSur
// folder icon, stored as "ws:<key>" so FileIcon knows to draw the asset.
export const CUSTOM_ICON_PREFIX = "ws:";
export const WHITESUR_FOLDER_ICONS: { value: string; url: string; label: string }[] =
  Object.entries(FOLDER_ICON_URLS)
    .map(([key, url]) => ({ value: CUSTOM_ICON_PREFIX + key, url, label: prettyFolderLabel(key) }))
    .sort((a, b) => a.label.localeCompare(b.label));

export function customIconUrl(value: string): string | undefined {
  return value.startsWith(CUSTOM_ICON_PREFIX)
    ? FOLDER_ICON_URLS[value.slice(CUSTOM_ICON_PREFIX.length)]
    : undefined;
}

// Tabler symbol icons (SF-Symbols-style, MIT) offered in the "Change Icon"
// picker as the monochrome alternative to colored folders. Imported as raw
// SVG markup (`?raw`) and inlined -- the SVGs are stroked with
// `currentColor`, so inlined they inherit the element's text color (tint +
// light/dark adaptation) with no CSS-mask/url plumbing to break.
const symbolRawModules = import.meta.glob("./assets/symbolicons/*.svg", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

const SYMBOL_ICON_SVGS: Record<string, string> = Object.fromEntries(
  Object.entries(symbolRawModules).map(([path, svg]) => [
    path.replace(/.*\/(.+)\.svg$/, "$1"),
    svg,
  ])
);

export const SYMBOL_PREFIX = "sym:";
export const SYMBOL_ICONS: { value: string; svg: string; label: string }[] = Object.entries(
  SYMBOL_ICON_SVGS
)
  .map(([key, svg]) => ({
    value: SYMBOL_PREFIX + key,
    svg,
    label: key.replace(/-/g, " ").replace(/\b\w/g, (c) => c.toUpperCase()),
  }))
  .sort((a, b) => a.label.localeCompare(b.label));

export function symbolIconSvg(value: string): string | undefined {
  return value.startsWith(SYMBOL_PREFIX)
    ? SYMBOL_ICON_SVGS[value.slice(SYMBOL_PREFIX.length)]
    : undefined;
}

// The UI chrome glyphs (toolbar/sidebar/breadcrumb) are Lucide
// (https://lucide.dev, ISC License) -- path data copied verbatim and drawn
// with Lucide's defaults (24x24 viewBox, currentColor stroke, 2px
// round-cap/join). The colored file/folder TILE icons use the WhiteSur
// macOS-Big-Sur theme imported above.

export type Kind =
  | "folder"
  | "generic"
  | "image"
  | "video"
  | "audio"
  | "pdf"
  | "archive"
  | "code"
  | "text"
  | "office";

// An encrypted `.vlt` file should look like whatever it *was* before
// encryption, not a generic blank sheet -- strip the suffix before ever
// inspecting the extension for icon/badge purposes.
function displayName(entry: Entry): string {
  const n = entry.name.toLowerCase();
  return n.endsWith(ENCRYPTED_FILE_EXT) ? n.slice(0, -ENCRYPTED_FILE_EXT.length) : n;
}

// Word/Excel/PowerPoint-family extensions -- no bundled artwork for these;
// FileIcon resolves whatever app the desktop has registered to open them
// instead (see useOfficeIcon below), matching Nautilus/Files' own
// behavior for file types they don't ship a fixed icon for.
const OFFICE_EXT_RE = /\.(docx?|odt|xlsx?|ods|pptx?|odp)$/;

export function kindOf(entry: Entry): Kind {
  if (entry.is_dir) return "folder";
  const l = displayName(entry);
  // .ytsearch/.imgsearch/.booksearch (see InternetView) are a saved
  // search's query + filters in JSON, not real media -- but they're meant
  // to read as "a video/image/book file" in the UI, so they borrow the
  // same icon as the kind of result they reopen.
  if (/\.(png|jpe?g|gif|webp|bmp|svg|heic|tiff?|imgsearch)$/.test(l)) return "image";
  if (/\.(mp4|mkv|mov|avi|webm|m4v|ytsearch)$/.test(l)) return "video";
  if (/\.(mp3|wav|flac|ogg|aac|m4a)$/.test(l)) return "audio";
  if (/\.(pdf|booksearch)$/.test(l)) return "pdf";
  if (/\.(zip|tar|gz|7z|rar|bz2|xz)$/.test(l)) return "archive";
  if (/\.(rs|ts|tsx|js|jsx|py|go|c|cpp|h|java|rb|sh|json|toml|yaml|yml|css|html)$/.test(l))
    return "code";
  if (/\.(txt|md|rtf|log)$/.test(l)) return "text";
  if (OFFICE_EXT_RE.test(l)) return "office";
  return "generic";
}

function extOf(entry: Entry): string {
  const l = displayName(entry);
  return l.slice(l.lastIndexOf(".") + 1);
}

const KIND_ICON: Record<Kind, string> = {
  folder: folderIcon,
  image: imageIcon,
  video: videoIcon,
  audio: audioIcon,
  pdf: pdfIcon,
  archive: archiveIcon,
  code: codeIcon,
  text: textIcon,
  generic: genericIcon,
  // Fallback while the real app icon resolves (or if none was found).
  office: genericIcon,
};

// Icon of whichever app the desktop has registered to open `ext`, fetched
// from the Rust `app_icon_for_ext` command (xdg-mime + .desktop + GTK icon
// theme) and cached per-extension for the session -- every tile with the
// same extension shares one lookup instead of one invoke() per file.
const officeIconCache = new Map<string, string | null>();
const officeIconInflight = new Map<string, Promise<string | null>>();

function useOfficeIcon(ext: string | null): string | null {
  const [icon, setIcon] = useState<string | null>(() =>
    ext ? officeIconCache.get(ext) ?? null : null
  );
  useEffect(() => {
    if (!ext) return;
    if (officeIconCache.has(ext)) {
      setIcon(officeIconCache.get(ext) ?? null);
      return;
    }
    let cancelled = false;
    let pending = officeIconInflight.get(ext);
    if (!pending) {
      pending = invoke<string | null>("app_icon_for_ext", { ext }).catch(() => null);
      officeIconInflight.set(ext, pending);
    }
    pending.then((result) => {
      officeIconCache.set(ext, result);
      officeIconInflight.delete(ext);
      if (!cancelled) setIcon(result);
    });
    return () => {
      cancelled = true;
    };
  }, [ext]);
  return icon;
}

// Android's WebView occasionally fails to load a bundled static asset on
// the first request (seen in practice with these bundled SVGs specifically
// -- the browser's own "broken image" glyph in place of the icon, clearing
// up on a manual refresh), the same class of one-off WebView flakiness as
// this app's other Android-only IPC/cache quirks. There's no such thing
// for a data: URI (thumbnails, see useThumbnail.ts) since a cache-busting
// query string would corrupt one -- this is only for real asset/file URLs.
export function RetryImg({
  src,
  className,
  alt = "",
}: {
  src: string;
  className?: string;
  alt?: string;
}) {
  const [attempt, setAttempt] = useState(0);
  const triesRef = useRef(0);
  const resolvedSrc = attempt === 0 ? src : `${src}${src.includes("?") ? "&" : "?"}retry=${attempt}`;
  return (
    <img
      className={className}
      src={resolvedSrc}
      alt={alt}
      draggable={false}
      onError={() => {
        if (triesRef.current >= 3) return;
        triesRef.current += 1;
        setTimeout(() => setAttempt((a) => a + 1), 250 * triesRef.current);
      }}
    />
  );
}

export function FileIcon({
  entry,
  tagHex,
  customIcon,
}: {
  entry: Entry;
  tagHex?: string;
  customIcon?: string;
}) {
  void tagHex; // tag tint isn't applied to the bundled WhiteSur artwork
  const kind = kindOf(entry);
  // Called unconditionally (before the customIcon branch) so hook order stays
  // stable across renders regardless of customIcon presence.
  const officeIcon = useOfficeIcon(kind === "office" ? extOf(entry) : null);

  // The artwork, whatever its source: a user-chosen icon (bundled WhiteSur
  // folder, Tabler symbol, or a bare emoji) or the bundled art for this kind.
  let art;
  if (customIcon) {
    const wsUrl = customIconUrl(customIcon);
    const symSvg = wsUrl ? null : symbolIconSvg(customIcon);
    art = wsUrl ? (
      <RetryImg className="fileicon-img" src={wsUrl} />
    ) : symSvg ? (
      <span className="symbol-icon" dangerouslySetInnerHTML={{ __html: symSvg }} />
    ) : (
      <span className="custom-icon-emoji">{customIcon}</span>
    );
  } else {
    const src = kind === "office" ? officeIcon ?? KIND_ICON.office : KIND_ICON[kind];
    art = <RetryImg className="fileicon-img" src={src} />;
  }

  // Vault folders get a small padlock overlaid on that artwork, rather than
  // shipping a separate locked-folder asset. Applied to a custom icon too:
  // "this folder is an encrypted vault" is a property of the folder, not a
  // style choice, and returning early for custom icons silently dropped the
  // padlock from exactly the vaults a user cared enough about to restyle.
  if (kind === "folder" && entry.is_vault) {
    return (
      <span className="fileicon-wrap">
        {art}
        <span className="fileicon-lock">
          <LockGlyph size={13} />
        </span>
      </span>
    );
  }
  // A saved Internet search (see InternetView) borrows the video/image/pdf
  // icon so it reads as an ordinary file, but plain video.svg alone was
  // easy to mistake for an actual media file (looked like a QuickTime
  // icon) -- this badge is what says "this is a search, not the real thing".
  if (/\.(ytsearch|imgsearch|booksearch)$/i.test(entry.name)) {
    return (
      <span className="fileicon-wrap">
        {art}
        <span className="fileicon-badge" aria-hidden="true">
          🌐
        </span>
      </span>
    );
  }
  return art;
}

// ---- UI glyphs (toolbar / sidebar / breadcrumb), authentic Lucide ----

type GlyphProps = { size?: number };

// Shared Lucide-style wrapper: 24x24 viewBox, stroked, currentColor.
function L({ size = 17, sw = 2, children }: { size?: number; sw?: number; children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={sw}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export function MenuGlyph({ size = 17 }: GlyphProps) {
  return (
    <L size={size} sw={2.2}>
      <path d="M4 5h16" />
      <path d="M4 12h16" />
      <path d="M4 19h16" />
    </L>
  );
}

export function ChevronLeft({ size = 17 }: GlyphProps) {
  return (
    <L size={size} sw={2.2}>
      <path d="m15 18-6-6 6-6" />
    </L>
  );
}

export function ChevronRight({ size = 17 }: GlyphProps) {
  return (
    <L size={size} sw={2.2}>
      <path d="m9 18 6-6-6-6" />
    </L>
  );
}

export function ChevronUp({ size = 17 }: GlyphProps) {
  return (
    <L size={size} sw={2.2}>
      <path d="m18 15-6-6-6 6" />
    </L>
  );
}

export function ChevronDown({ size = 17 }: GlyphProps) {
  return (
    <L size={size} sw={2.2}>
      <path d="m6 9 6 6 6-6" />
    </L>
  );
}

// Lucide `refresh-cw`. Rendered circular via CSS (.refresh-btn), Chrome-style.
export function RefreshGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2.1}>
      <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
      <path d="M21 3v5h-5" />
      <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" />
      <path d="M8 16H3v5" />
    </L>
  );
}

// Lucide `layout-grid`.
export function IconViewGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <rect width="7" height="7" x="3" y="3" rx="1" />
      <rect width="7" height="7" x="14" y="3" rx="1" />
      <rect width="7" height="7" x="14" y="14" rx="1" />
      <rect width="7" height="7" x="3" y="14" rx="1" />
    </L>
  );
}

// Lucide `list`.
export function ListViewGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="M3 5h.01" />
      <path d="M3 12h.01" />
      <path d="M3 19h.01" />
      <path d="M8 5h13" />
      <path d="M8 12h13" />
      <path d="M8 19h13" />
    </L>
  );
}

// Lucide `panel-right` -- a list with a preview pane on the right.
export function ListPreviewGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <rect width="18" height="18" x="3" y="3" rx="2" />
      <path d="M15 3v18" />
    </L>
  );
}

// Lucide `columns-3`.
export function ColumnViewGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <rect width="18" height="18" x="3" y="3" rx="2" />
      <path d="M9 3v18" />
      <path d="M15 3v18" />
    </L>
  );
}

// Lucide `settings` (gear).
export function SettingsGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915" />
      <circle cx="12" cy="12" r="3" />
    </L>
  );
}

// Lucide `eye`.
export function EyeGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0" />
      <circle cx="12" cy="12" r="3" />
    </L>
  );
}

// Lucide `eye-off`.
export function EyeOffGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49" />
      <path d="M14.084 14.158a3 3 0 0 1-4.242-4.242" />
      <path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143" />
      <path d="m2 2 20 20" />
    </L>
  );
}

// Lucide `search`.
export function SearchGlyph({ size = 14 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="m21 21-4.34-4.34" />
      <circle cx="11" cy="11" r="8" />
    </L>
  );
}

// Lucide `copy`.
export function CopyGlyph({ size = 13 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
      <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
    </L>
  );
}

// Lucide `save`.
export function SaveGlyph({ size = 13 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <path d="M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z" />
      <path d="M17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7" />
      <path d="M7 3v4a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1V3.4" />
    </L>
  );
}

// Lucide `check`.
export function CheckGlyph({ size = 13 }: GlyphProps) {
  return (
    <L size={size} sw={2.2}>
      <path d="M20 6 9 17l-5-5" />
    </L>
  );
}

// Lucide `file-plus` -- mobile toolbar's icon-only "+ File" button.
export function NewFileGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z" />
      <path d="M13 2v6h6" />
      <line x1="9" x2="15" y1="15" y2="15" />
      <line x1="12" x2="12" y1="12" y2="18" />
    </L>
  );
}

// Same `folder` outline as PlaceGlyph below, plus a "+" -- mobile
// toolbar's icon-only "+ Folder" button.
export function NewFolderGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />
      <line x1="9" x2="15" y1="14" y2="14" />
      <line x1="12" x2="12" y1="11" y2="17" />
    </L>
  );
}

// Lucide `clipboard` -- mobile toolbar's icon-only "Paste" button.
export function PasteGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <rect width="8" height="4" x="8" y="2" rx="1" ry="1" />
      <path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" />
    </L>
  );
}

// Breadcrumb crumb icon: a real folder (Lucide `folder`, stroked so it
// inherits the crumb's text color).
export function PlaceGlyph({ size = 15, color }: GlyphProps & { color?: string }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke={color ?? "currentColor"}
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />
    </svg>
  );
}

// Lucide `hard-drive` -- the drive root crumb / device.
export function DiskGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="M10 16h.01" />
      <path d="M2.212 11.577a2 2 0 0 0-.212.896V18a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-5.527a2 2 0 0 0-.212-.896L18.55 5.11A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" />
      <path d="M21.946 12.013H2.054" />
      <path d="M6 16h.01" />
    </L>
  );
}

// Lucide `monitor`.
export function ComputerGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <rect width="20" height="14" x="2" y="3" rx="2" />
      <line x1="8" x2="16" y1="21" y2="21" />
      <line x1="12" x2="12" y1="17" y2="21" />
    </L>
  );
}

// Lucide `smartphone` -- mobile sidebar's "My Device" entry (My Computer's
// drive-list makes no sense on a phone with exactly one storage volume; a
// one-tap jump back to the phone's real root is the equivalent).
export function SmartphoneGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <rect width="14" height="20" x="5" y="2" rx="2" ry="2" />
      <path d="M12 18h.01" />
    </L>
  );
}

// Lucide `share-2` -- media viewer's mobile-only Share button (OS share
// sheet, distinct from the desktop context menu's link-upload "Share…").
export function ShareGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <circle cx="18" cy="5" r="3" />
      <circle cx="6" cy="12" r="3" />
      <circle cx="18" cy="19" r="3" />
      <line x1="8.59" x2="15.42" y1="13.51" y2="17.49" />
      <line x1="15.41" x2="8.59" y1="6.51" y2="10.49" />
    </L>
  );
}

// Lucide `phone` -- ContactsGrid's Call button. Colored green via CSS at
// the call site (a plain dark glyph read as "doesn't actually invite a
// tap", reported directly), not baked in here like the status glyphs
// above -- every other icon in this file inherits its color from
// context, and this one's context is "the call button", not "always
// green everywhere".
export function PhoneGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="M13.832 16.568a1 1 0 0 0 1.213-.303l.355-.465A2 2 0 0 1 17 15h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2A18 18 0 0 1 2 4a2 2 0 0 1 2-2h3a2 2 0 0 1 2 2v3a2 2 0 0 1-.8 1.6l-.468.351a1 1 0 0 0-.292 1.233 14 14 0 0 0 6.392 6.384" />
    </L>
  );
}

// Lucide `message-circle` -- ContactsGrid's WhatsApp button. No real
// brand-mark icon set is vendored here, so this is a generic chat bubble
// (colored WhatsApp green via CSS, labeled "WhatsApp" in text) rather
// than an inaccurate logo.
export function ChatGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="M2.992 16.342a2 2 0 0 1 .094 1.167l-1.065 3.29a1 1 0 0 0 1.236 1.168l3.413-.998a2 2 0 0 1 1.099.092 10 10 0 1 0 -4.777-4.719" />
    </L>
  );
}

// Lucide `usb`.
export function UsbDriveGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <circle cx="10" cy="7" r="1" />
      <circle cx="4" cy="20" r="1" />
      <path d="M4.7 19.3 19 5" />
      <path d="m21 3-3 1 2 2Z" />
      <path d="M9.26 7.68 5 12l2 5" />
      <path d="m10 14 5 2 3.5-3.5" />
      <path d="m18 12 1-1 1 1-1 1Z" />
    </L>
  );
}

// Status/badge glyphs keep their meaningful color (not part of the neutral
// chrome). `pin`, and the three sync families, stay tinted.
export function PinGlyph({ size = 15 }: GlyphProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="#ff9f0a"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M12 17v5" />
      <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" />
    </svg>
  );
}

// Lucide `trash-2`.
export function TrashGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <path d="M10 11v6" />
      <path d="M14 11v6" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
      <path d="M3 6h18" />
      <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
    </L>
  );
}

// Lucide `git-branch`, tinted (VCS sync badge).
export function GitBranchGlyph({ size = 15 }: GlyphProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="#ff9f0a"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M15 6a9 9 0 0 0-9 9V3" />
      <circle cx="18" cy="6" r="3" />
      <circle cx="6" cy="18" r="3" />
    </svg>
  );
}

// Lucide `arrow-right-left`, tinted teal (local/device sync badge).
export function LocalSyncGlyph({ size = 15 }: GlyphProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="#30b0a8"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="m16 3 4 4-4 4" />
      <path d="M20 7H4" />
      <path d="m8 21-4-4 4-4" />
      <path d="M4 17h16" />
    </svg>
  );
}

// Lucide `cloud` (cloud sync badge), tinted blue.
export function CloudSyncGlyph({ size = 15 }: GlyphProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="#0a84ff"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z" />
    </svg>
  );
}

// Lucide `lock`.
export function LockGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
      <path d="M7 11V7a5 5 0 0 1 10 0v4" />
    </L>
  );
}

// Lucide `lock-open` -- an unlocked vault (shackle swung open).
export function LockOpenGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
      <path d="M7 11V7a5 5 0 0 1 9.9-1" />
    </L>
  );
}

// ---- Media viewer glyphs (play/pause/track-skip/volume), same Lucide
// stroke style as the rest of this file. Added for AudioStage/MediaViewer.

// Lucide `play`.
export function PlayGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <polygon points="6 3 20 12 6 21 6 3" />
    </L>
  );
}

// Lucide `pause`.
export function PauseGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <rect width="4" height="16" x="6" y="4" rx="1" />
      <rect width="4" height="16" x="14" y="4" rx="1" />
    </L>
  );
}

// Lucide `skip-back` -- previous track.
export function SkipBackGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <polygon points="19 20 9 12 19 4 19 20" />
      <line x1="5" x2="5" y1="19" y2="5" />
    </L>
  );
}

// Lucide `skip-forward` -- next track.
export function SkipForwardGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <polygon points="5 4 15 12 5 20 5 4" />
      <line x1="19" x2="19" y1="5" y2="19" />
    </L>
  );
}

// Lucide `volume-2` -- unmuted speaker.
export function VolumeGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
      <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
      <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
    </L>
  );
}

// Lucide `volume-x` -- muted speaker.
export function VolumeMuteGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
      <line x1="22" x2="16" y1="9" y2="15" />
      <line x1="16" x2="22" y1="9" y2="15" />
    </L>
  );
}

// Lucide `music` -- decorative glyph for the audio "now playing" card.
export function MusicNoteGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="M9 18V5l12-2v13" />
      <circle cx="6" cy="18" r="3" />
      <circle cx="18" cy="16" r="3" />
    </L>
  );
}

// Lucide `shuffle` -- AudioStage's shuffle toggle.
export function ShuffleGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="M2 18h1.4c1.3 0 2.5-.6 3.3-1.7l6.1-8.6c.7-1.1 2-1.7 3.3-1.7H22" />
      <path d="m18 2 4 4-4 4" />
      <path d="M2 6h1.4c1.3 0 2.5.6 3.3 1.7l.7 1" />
      <path d="M20 18h-5.3c-1.3 0-2.5-.6-3.3-1.7l-.7-1" />
      <path d="m18 22 4-4-4-4" />
    </L>
  );
}

// Lucide `repeat` -- AudioStage's repeat-all mode.
export function RepeatGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="m17 2 4 4-4 4" />
      <path d="M3 11v-1a4 4 0 0 1 4-4h14" />
      <path d="m7 22-4-4 4-4" />
      <path d="M21 13v1a4 4 0 0 1-4 4H3" />
    </L>
  );
}

// Lucide `repeat-1` -- AudioStage's repeat-one mode.
export function Repeat1Glyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="m17 2 4 4-4 4" />
      <path d="M3 11v-1a4 4 0 0 1 4-4h14" />
      <path d="m7 22-4-4 4-4" />
      <path d="M21 13v1a4 4 0 0 1-4 4H3" />
      <path d="M11 10h1v4" />
    </L>
  );
}

// No direct Lucide equivalent -- a play triangle pointing left, for
// AudioStage's "listen in reverse" toggle.
export function ReverseGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <polygon points="18 3 4 12 18 21 18 3" />
    </L>
  );
}

// ---- Media viewer / image editor glyphs ----

// Lucide `x` -- generic close button (fullscreen viewer, editor overlay).
export function CloseGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2.2}>
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </L>
  );
}

// Lucide `crop` -- ImageEditor "Crop" mode tab.
export function CropGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <path d="M6 2v14a2 2 0 0 0 2 2h14" />
      <path d="M18 22V8a2 2 0 0 0-2-2H2" />
    </L>
  );
}

// Lucide `pencil` -- ImageEditor "Draw" mode tab.
export function PencilGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z" />
      <path d="m15 5 4 4" />
    </L>
  );
}

// Lucide `sliders-horizontal` -- ImageEditor "Adjust" mode tab.
export function SlidersGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <line x1="21" x2="14" y1="4" y2="4" />
      <line x1="10" x2="3" y1="4" y2="4" />
      <line x1="21" x2="12" y1="12" y2="12" />
      <line x1="8" x2="3" y1="12" y2="12" />
      <line x1="21" x2="16" y1="20" y2="20" />
      <line x1="12" x2="3" y1="20" y2="20" />
      <line x1="14" x2="14" y1="2" y2="6" />
      <line x1="8" x2="8" y1="10" y2="14" />
      <line x1="16" x2="16" y1="18" y2="22" />
    </L>
  );
}

// Lucide `flip-horizontal-2` -- ImageEditor "Flip Horizontal" button.
export function FlipHorizontalGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <path d="M9 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h4" />
      <path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4" />
      <path d="M12 20v2" />
      <path d="M12 14v2" />
      <path d="M12 8v2" />
      <path d="M12 2v2" />
    </L>
  );
}

// Lucide `flip-vertical-2` -- ImageEditor "Flip Vertical" button.
export function FlipVerticalGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={1.9}>
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <path d="M21 9V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v4" />
      <path d="M4 12H2" />
      <path d="M10 12H8" />
      <path d="M16 12h-2" />
      <path d="M22 12h-2" />
    </L>
  );
}

// Lucide `undo-2` -- ImageEditor Draw-mode "Undo" button.
export function UndoGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="M9 14 4 9l5-5" />
      <path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5v0a5.5 5.5 0 0 1-5.5 5.5H11" />
    </L>
  );
}

// Lucide `rotate-cw` -- MediaViewer "Rotate" button (single 3/4-turn arrow,
// distinct from RefreshGlyph's two-arrow refresh icon above).
export function RotateGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8" />
      <path d="M21 3v5h-5" />
    </L>
  );
}

// Lucide `maximize` -- VideoStage "enter fullscreen" button.
export function FullscreenGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="M8 3H5a2 2 0 0 0-2 2v3" />
      <path d="M21 8V5a2 2 0 0 0-2-2h-3" />
      <path d="M3 16v3a2 2 0 0 0 2 2h3" />
      <path d="M16 21h3a2 2 0 0 0 2-2v-3" />
    </L>
  );
}

// Lucide `minimize` -- VideoStage "exit fullscreen" button.
export function FullscreenExitGlyph({ size = 15 }: GlyphProps) {
  return (
    <L size={size} sw={2}>
      <path d="M8 3v3a2 2 0 0 1-2 2H3" />
      <path d="M21 8h-3a2 2 0 0 1-2-2V3" />
      <path d="M3 16h3a2 2 0 0 1 2 2v3" />
      <path d="M16 21v-3a2 2 0 0 1 2-2h3" />
    </L>
  );
}
