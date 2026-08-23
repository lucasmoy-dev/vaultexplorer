import { useRef } from "react";
import { Entry, baseName, parentPath } from "../api";
import { FileIcon } from "../icons";
import { useThumbnail } from "../hooks/useThumbnail";

// One hit. Split out as its own component so each row can run the thumbnail
// hook (and its own intersection-observer gate) -- results lists get long,
// and only what's on screen should ask the backend for a thumbnail.
function SearchRow({
  path,
  entry,
  inVault,
  mobile,
  selected,
  onSelect,
  onOpen,
  onMenu,
}: {
  path: string;
  entry: Entry;
  inVault: boolean;
  mobile?: boolean;
  selected: boolean;
  onSelect: (p: string) => void;
  onOpen: (p: string) => void;
  onMenu: (e: React.MouseEvent, p: string) => void;
}) {
  const rowRef = useRef<HTMLLIElement>(null);
  const thumb = useThumbnail(entry, path, inVault, 32, rowRef);
  return (
    <li
      ref={rowRef}
      className={selected ? "selected" : undefined}
      // Same click contract as an ordinary listing row: click selects,
      // double-click opens on desktop; on touch a single tap does both
      // (see the EntryTile handler in App.tsx). A hit used to navigate to
      // its *parent folder* on a single click, which threw the search away
      // and never opened the thing that was clicked.
      onClick={() => {
        onSelect(path);
        if (mobile) onOpen(path);
      }}
      onDoubleClick={() => onOpen(path)}
      onContextMenu={(e) => onMenu(e, path)}
    >
      <span className="result-icon">
        {thumb ? (
          <img src={thumb} className="entry-thumb" alt="" draggable={false} />
        ) : (
          <FileIcon entry={entry} />
        )}
      </span>
      <span className="result-name">{entry.name}</span>
      <span className="result-path">{parentPath(path) || "Vault"}</span>
    </li>
  );
}

export function SearchResults({
  query,
  results,
  entries,
  inVault,
  mobile,
  selected,
  onSelect,
  onOpen,
  onMenu,
}: {
  query: string;
  results: string[];
  // Real entries by path, filled in asynchronously by the caller. A row
  // whose entry hasn't landed yet falls back to a name-only stand-in, which
  // is right for the common case (a file) and settles the moment the
  // listing resolves.
  entries: Record<string, Entry>;
  inVault: boolean;
  mobile?: boolean;
  selected: string | null;
  onSelect: (p: string) => void;
  onOpen: (p: string) => void;
  onMenu: (e: React.MouseEvent, p: string) => void;
}) {
  return (
    <div className="search-results">
      <div className="search-header">
        <span>
          {results.length} {results.length === 1 ? "result" : "results"} for “{query}”
        </span>
      </div>
      <ul>
        {results.map((p) => (
          <SearchRow
            key={p}
            path={p}
            entry={entries[p] ?? { name: baseName(p), is_dir: false, size: 0, mtime: 0 }}
            inVault={inVault}
            mobile={mobile}
            selected={selected === p}
            onSelect={onSelect}
            onOpen={onOpen}
            onMenu={onMenu}
          />
        ))}
      </ul>
    </div>
  );
}
