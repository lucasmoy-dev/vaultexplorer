import { useRef } from "react";
import { Entry, joinPath } from "../api";
import { FileIcon, kindOf, Kind } from "../icons";
import { useThumbnail } from "../hooks/useThumbnail";
import { displayEntryName } from "../entryHelpers";

// Deterministic per-item cover tint, for whatever doesn't get a real
// rendered thumbnail (a book/ebook with no page-1 renderer, an audio
// sleeve, anything that failed to load).
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

function isBookish(entry: Entry): boolean {
  if (entry.is_dir) return false;
  const dot = entry.name.lastIndexOf(".");
  if (dot <= 0) return false;
  const ext = entry.name.slice(dot + 1).toLowerCase();
  return ["epub", "mobi", "azw", "azw3", "fb2", "djvu", "cbz", "cbr"].includes(ext);
}

// Every real shelf item shares this: it stands on the shelf baseline
// (align-items: flex-end on the row does that regardless of each type's
// own height) and casts its own contact shadow onto the shelf surface --
// a blurred dark ellipse right at its base, the single biggest thing that
// was making everything read as flat cutouts pasted on the wood instead
// of objects actually sitting on it.
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
  return (
    <ShelfItem title={entry.name} width={128}>
      <div ref={ref} className="shelf-vhs">
        <div className="shelf-vhs-label">
          {thumb && <img className="shelf-vhs-thumb" src={thumb} draggable={false} alt="" />}
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

function FolderItem({ entry }: { entry: Entry }) {
  return (
    <ShelfItem title={entry.name} width={96}>
      <div className="shelf-folder">
        <FileIcon entry={entry} />
      </div>
      <span className="shelf-item-label">{entry.name}</span>
    </ShelfItem>
  );
}

function GenericItem({ entry }: { entry: Entry }) {
  return (
    <ShelfItem title={entry.name} width={80}>
      <div className="shelf-generic">
        <FileIcon entry={entry} />
      </div>
      <span className="shelf-item-label">{displayEntryName(entry, false)}</span>
    </ShelfItem>
  );
}

// Experimental: a real folder as an actual wooden bookshelf -- every kind
// of file gets its own physical stand-in instead of an icon, matching the
// look of the original iPhone's skeuomorphic Library app: PDFs/ebooks are
// a bound stack of pages, photos are a framed picture, audio is a record
// sleeve, video is a VHS tape, folders are a real folder, standing side by
// side and wrapping onto the next shelf board down. A real PDF/image/
// video gets its actual content rendered as the cover/thumbnail (see
// useThumbnail/thumbnail.rs); anything without a renderer here yet (an
// ebook format, an audio file -- no cover-art extraction) falls back to a
// plain colored placeholder with the title on it.
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
        <div className="library-shelf">{entries.map(itemFor)}</div>
      )}
    </div>
  );
}
