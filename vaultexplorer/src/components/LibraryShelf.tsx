import { useLayoutEffect, useRef, useState } from "react";
import { Entry, joinPath } from "../api";
import { kindOf, Kind } from "../icons";
import { useThumbnail } from "../hooks/useThumbnail";
import { displayEntryName } from "../entryHelpers";

// Deterministic per-item cover tint, for whatever doesn't get a real
// rendered thumbnail (a book/ebook with no page-1 renderer, an audio
// sleeve, a folder, anything that failed to load).
const COVER_HUES = [355, 15, 30, 45, 85, 165, 195, 215, 255, 285, 320];
function coverHue(name: string): number {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) | 0;
  return COVER_HUES[Math.abs(h) % COVER_HUES.length];
}
function coverGradient(name: string): string {
  const hue = coverHue(name);
  return `linear-gradient(160deg, hsl(${hue}, 42%, 40%), hsl(${hue}, 45%, 24%))`;
}
function folderGradient(name: string): string {
  const hue = coverHue(name);
  return `linear-gradient(160deg, hsl(${hue}, 38%, 78%), hsl(${hue}, 34%, 60%))`;
}

function isBookish(entry: Entry): boolean {
  if (entry.is_dir) return false;
  const dot = entry.name.lastIndexOf(".");
  if (dot <= 0) return false;
  const ext = entry.name.slice(dot + 1).toLowerCase();
  return ["epub", "mobi", "azw", "azw3", "fb2", "djvu", "cbz", "cbr"].includes(ext);
}

function extOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toUpperCase() : "FILE";
}

// Every real shelf item shares this: a fixed-height slot (so every row
// lands on the same shelf pitch no matter what mix of object heights it
// holds -- a short cassette and a tall book both stand on the same
// floor), with its own object bottom-aligned inside, plus a contact
// shadow right at its base -- the biggest single thing that turns a
// flat cutout into something that reads as sitting there.
function ShelfItem({ title, children, width }: { title: string; children: React.ReactNode; width: number }) {
  return (
    <div className="shelf-item" style={{ width }} title={title}>
      {children}
      <div className="shelf-item-shadow" />
    </div>
  );
}

function BookItem({ entry, fullPath, inVault }: { entry: Entry; fullPath: string; inVault: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const thumb = useThumbnail(entry, fullPath, inVault, 260, ref);
  const title = displayEntryName(entry, false).replace(/\.\w+$/, "");
  return (
    <ShelfItem title={entry.name} width={104}>
      <div ref={ref} className="shelf-book">
        {/* A couple of thin offset "pages" peeking out from behind the
            cover -- what turns a single flat sheet into a bound stack. */}
        <span className="shelf-book-page shelf-book-page-2" />
        <span className="shelf-book-page shelf-book-page-1" />
        {thumb ? (
          <img className="shelf-book-cover" src={thumb} draggable={false} alt="" />
        ) : (
          <div className="shelf-book-cover shelf-book-cover-placeholder" style={{ background: coverGradient(entry.name) }}>
            <span className="shelf-book-title">{title}</span>
          </div>
        )}
      </div>
    </ShelfItem>
  );
}

function PhotoItem({ entry, fullPath, inVault }: { entry: Entry; fullPath: string; inVault: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const thumb = useThumbnail(entry, fullPath, inVault, 220, ref);
  return (
    <ShelfItem title={entry.name} width={118}>
      <div ref={ref} className="shelf-frame">
        <div className="shelf-frame-mat">
          {thumb ? (
            <img className="shelf-frame-photo" src={thumb} draggable={false} alt="" />
          ) : (
            <div className="shelf-frame-photo shelf-frame-photo-placeholder" />
          )}
        </div>
      </div>
    </ShelfItem>
  );
}

function VideoItem({ entry, fullPath, inVault }: { entry: Entry; fullPath: string; inVault: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const thumb = useThumbnail(entry, fullPath, inVault, 220, ref);
  const title = displayEntryName(entry, false).replace(/\.\w+$/, "");
  return (
    <ShelfItem title={entry.name} width={128}>
      <div ref={ref} className="shelf-vhs">
        <div className="shelf-vhs-label" style={!thumb ? { background: coverGradient(entry.name) } : undefined}>
          {thumb ? <img className="shelf-vhs-thumb" src={thumb} draggable={false} alt="" /> : <span className="shelf-vhs-title">{title}</span>}
        </div>
        <div className="shelf-vhs-window" />
      </div>
    </ShelfItem>
  );
}

function AudioItem({ entry }: { entry: Entry }) {
  return (
    <ShelfItem title={entry.name} width={112}>
      <div className="shelf-vinyl">
        <div className="shelf-vinyl-sleeve" style={{ background: coverGradient(entry.name) }}>
          <span className="shelf-vinyl-title">{displayEntryName(entry, false).replace(/\.\w+$/, "")}</span>
        </div>
        <div className="shelf-vinyl-disc" />
      </div>
    </ShelfItem>
  );
}

// A real manila-folder silhouette (back tab peeking above a front flap)
// with the folder's own name printed on it like a written label --
// deliberately not the app's flat conventional folder icon, which reads
// as a file-manager glyph rather than a physical object standing here.
function FolderItem({ entry }: { entry: Entry }) {
  return (
    <ShelfItem title={entry.name} width={100}>
      <div className="shelf-folder">
        <div className="shelf-folder-back" style={{ background: folderGradient(entry.name) }} />
        <div className="shelf-folder-front" style={{ background: folderGradient(entry.name) }}>
          <span className="shelf-folder-label">{entry.name}</span>
        </div>
      </div>
    </ShelfItem>
  );
}

// A plain standing document card -- folded top-right corner, an
// extension badge, the filename printed on the face -- for anything
// that isn't one of the other physical kinds. Same reasoning as
// FolderItem: a generic file-type icon reads as chrome, not an object.
function GenericItem({ entry }: { entry: Entry }) {
  const ext = extOf(entry.name);
  return (
    <ShelfItem title={entry.name} width={82}>
      <div className="shelf-doc">
        <span className="shelf-doc-name">{displayEntryName(entry, false)}</span>
        <span className="shelf-doc-ext" style={{ background: coverGradient(entry.name) }}>
          {ext}
        </span>
      </div>
    </ShelfItem>
  );
}

// Real DOM shelf boards instead of a background-image trick: each board
// is its own element with a lit top lip (a hard specular highlight, the
// kind of thing a repeating CSS gradient band can't fake), a rounded
// face falling into shadow, a drop shadow blurring onto whatever sits
// below it, and a soft cast-shadow bleeding into the top of the next
// cubby. Rendering real elements (rather than timing a repeating
// background to a fixed row pitch) is what makes the shelf read as
// occupying actual depth instead of a flat striped backdrop.
const ROW_HEIGHT = 150;
const ROW_GAP = 28;
const ROW_PITCH = ROW_HEIGHT + ROW_GAP;
const TOP_PADDING = 12;

function ShelfBoards({ rowCount }: { rowCount: number }) {
  return (
    <div className="shelf-boards" aria-hidden>
      {Array.from({ length: rowCount }, (_, i) => {
        const top = TOP_PADDING + i * ROW_PITCH + ROW_HEIGHT;
        return (
          <div className="shelf-board" key={i} style={{ top }}>
            <div className="shelf-board-lip" />
            <div className="shelf-board-face" />
            <div className="shelf-board-shade" />
          </div>
        );
      })}
    </div>
  );
}

function useRowCount(ref: React.RefObject<HTMLDivElement | null>, watch: unknown) {
  const [rows, setRows] = useState(1);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const recompute = () => setRows(Math.max(1, Math.ceil((el.scrollHeight - TOP_PADDING) / ROW_PITCH)));
    recompute();
    const ro = new ResizeObserver(recompute);
    ro.observe(el);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [watch]);
  return rows;
}

// Experimental: a real folder as an actual wooden bookshelf -- every kind
// of file gets its own physical stand-in instead of an icon, matching the
// look of the original iPhone's skeuomorphic Library app: PDFs/ebooks are
// a bound stack of pages, photos are framed, audio is a record sleeve,
// video is a labeled VHS tape, folders are a real folder with their name
// written on it, everything else is a standing document card with its
// name on it -- standing side by side and wrapping onto the next shelf
// board down. A real PDF/image/video gets its actual content rendered as
// the cover/thumbnail (see useThumbnail/thumbnail.rs); anything without a
// renderer here (an ebook format, an audio file -- no cover-art
// extraction) falls back to a plain colored placeholder with the title
// printed on it, same as a real spine/label would show.
export function LibraryShelf({
  entries,
  curDir,
  inVault,
  onOpen,
  onMenu,
}: {
  entries: Entry[];
  curDir: string;
  inVault: boolean;
  onOpen: (entry: Entry) => void;
  onMenu: (e: React.MouseEvent, entry: Entry) => void;
}) {
  const shelfRef = useRef<HTMLDivElement>(null);
  const rowCount = useRowCount(shelfRef, entries.length);

  function itemFor(entry: Entry) {
    const fullPath = joinPath(curDir, entry.name);
    const kind: Kind = entry.is_dir ? "folder" : kindOf(entry);
    const shared = {
      onDoubleClick: () => onOpen(entry),
      onContextMenu: (e: React.MouseEvent) => onMenu(e, entry),
    };
    let inner: React.ReactNode;
    if (kind === "folder") {
      inner = <FolderItem entry={entry} />;
    } else if (kind === "pdf" || isBookish(entry)) {
      inner = <BookItem entry={entry} fullPath={fullPath} inVault={inVault} />;
    } else if (kind === "image") {
      inner = <PhotoItem entry={entry} fullPath={fullPath} inVault={inVault} />;
    } else if (kind === "video") {
      inner = <VideoItem entry={entry} fullPath={fullPath} inVault={inVault} />;
    } else if (kind === "audio") {
      inner = <AudioItem entry={entry} />;
    } else {
      inner = <GenericItem entry={entry} />;
    }
    return (
      <div key={entry.name} onDoubleClick={shared.onDoubleClick} onContextMenu={shared.onContextMenu}>
        {inner}
      </div>
    );
  }

  return (
    <div className="library-view">
      {entries.length === 0 ? (
        <p className="notes-empty">This folder is empty.</p>
      ) : (
        <div className="library-shelf" ref={shelfRef}>
          <ShelfBoards rowCount={rowCount} />
          {entries.map(itemFor)}
        </div>
      )}
    </div>
  );
}
