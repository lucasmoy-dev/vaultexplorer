import { ReactNode } from "react";
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

type Kind =
  | "folder"
  | "generic"
  | "image"
  | "video"
  | "audio"
  | "pdf"
  | "archive"
  | "code"
  | "text";

// An encrypted `.vlt` file should look like whatever it *was* before
// encryption, not a generic blank sheet -- strip the suffix before ever
// inspecting the extension for icon/badge purposes.
function displayName(entry: Entry): string {
  const n = entry.name.toLowerCase();
  return n.endsWith(ENCRYPTED_FILE_EXT) ? n.slice(0, -ENCRYPTED_FILE_EXT.length) : n;
}

export function kindOf(entry: Entry): Kind {
  if (entry.is_dir) return "folder";
  const l = displayName(entry);
  if (/\.(png|jpe?g|gif|webp|bmp|svg|heic|tiff?)$/.test(l)) return "image";
  if (/\.(mp4|mkv|mov|avi|webm|m4v)$/.test(l)) return "video";
  if (/\.(mp3|wav|flac|ogg|aac|m4a)$/.test(l)) return "audio";
  if (/\.(pdf)$/.test(l)) return "pdf";
  if (/\.(zip|tar|gz|7z|rar|bz2|xz)$/.test(l)) return "archive";
  if (/\.(rs|ts|tsx|js|jsx|py|go|c|cpp|h|java|rb|sh|json|toml|yaml|yml|css|html)$/.test(l))
    return "code";
  if (/\.(txt|md|rtf|log)$/.test(l)) return "text";
  return "generic";
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
};

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
  if (customIcon) {
    const wsUrl = customIconUrl(customIcon);
    if (wsUrl) {
      return <img className="fileicon-img" src={wsUrl} alt="" draggable={false} />;
    }
    const symSvg = symbolIconSvg(customIcon);
    if (symSvg) {
      return <span className="symbol-icon" dangerouslySetInnerHTML={{ __html: symSvg }} />;
    }
    return <span className="custom-icon-emoji">{customIcon}</span>;
  }
  const kind = kindOf(entry);
  const src = KIND_ICON[kind];
  // Vault folders reuse the plain folder art with a small padlock overlaid,
  // rather than shipping a separate locked-folder asset.
  if (kind === "folder" && entry.is_vault) {
    return (
      <span className="fileicon-wrap">
        <img className="fileicon-img" src={src} alt="" draggable={false} />
        <span className="fileicon-lock">
          <LockGlyph size={13} />
        </span>
      </span>
    );
  }
  return <img className="fileicon-img" src={src} alt="" draggable={false} />;
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

// Lucide `check`.
export function CheckGlyph({ size = 13 }: GlyphProps) {
  return (
    <L size={size} sw={2.2}>
      <path d="M20 6 9 17l-5-5" />
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
