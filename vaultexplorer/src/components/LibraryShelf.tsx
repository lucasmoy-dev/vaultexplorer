import { Entry } from "../api";
import { FileIcon } from "../icons";
import { displayEntryName } from "../entryHelpers";

// Deterministic per-book cover tint -- same idea as the old NotesGrid hue
// hash, repurposed here for "different colored book covers on a shelf"
// instead of note colors.
const COVER_HUES = [355, 15, 30, 45, 85, 165, 195, 215, 255, 285, 320];
function coverStyle(name: string): { background: string; width: number } {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) | 0;
  const hue = COVER_HUES[Math.abs(h) % COVER_HUES.length];
  // A few px of width variance per "book" -- some are thicker than others
  // on a real shelf. Bounded tight enough that a title still fits legibly
  // along the spine.
  const width = 30 + (Math.abs(h) % 14);
  return { background: `linear-gradient(100deg, hsl(${hue}, 45%, 32%), hsl(${hue}, 40%, 22%))`, width };
}

function isBook(entry: Entry): boolean {
  if (entry.is_dir) return false;
  const dot = entry.name.lastIndexOf(".");
  if (dot <= 0) return false;
  const ext = entry.name.slice(dot + 1).toLowerCase();
  return ["pdf", "epub", "mobi", "azw", "azw3", "fb2", "djvu", "cbz", "cbr"].includes(ext);
}

// Experimental: a folder of PDFs/ebooks as an actual wooden bookshelf --
// each file a spine standing on a shelf board, title running along it --
// instead of an icon grid. Explicitly a first pass at the idea (a real
// skeuomorphic library, like the original iPhone's Library app), not a
// polished final look: the "shelf" is a repeating CSS background stripe
// timed to match each book's fixed height + row gap (see the CSS), and
// covers are a deterministic gradient tint rather than real artwork --
// there's no cover-image extraction here, just an evocative placeholder.
export function LibraryShelf({
  entries,
  onOpen,
  onMenu,
}: {
  entries: Entry[];
  onOpen: (entry: Entry) => void;
  onMenu: (e: React.MouseEvent, entry: Entry) => void;
}) {
  const books = entries.filter(isBook);
  const others = entries.filter((e) => !isBook(e));

  return (
    <div className="library-view">
      {books.length === 0 ? (
        <p className="notes-empty">No PDFs or ebooks here yet.</p>
      ) : (
        <div className="library-shelf">
          {books.map((entry) => {
            const { background, width } = coverStyle(entry.name);
            const title = displayEntryName(entry, false).replace(/\.\w+$/, "");
            return (
              <div
                key={entry.name}
                className="library-book"
                style={{ background, width }}
                title={entry.name}
                onDoubleClick={() => onOpen(entry)}
                onContextMenu={(e) => onMenu(e, entry)}
              >
                <span className="library-book-title">{title}</span>
              </div>
            );
          })}
        </div>
      )}
      {others.length > 0 && (
        <div className="library-others">
          <div className="notes-section-label">Other files</div>
          <div className="notes-others">
            {others.map((entry) => (
              <button
                key={entry.name}
                className="notes-other-tile"
                onClick={() => onOpen(entry)}
                onContextMenu={(e) => onMenu(e, entry)}
              >
                <FileIcon entry={entry} />
                <span>{displayEntryName(entry, false)}</span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
