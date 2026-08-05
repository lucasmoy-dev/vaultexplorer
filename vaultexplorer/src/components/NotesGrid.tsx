import { useEffect, useState } from "react";
import { Entry, api, joinPath, TAG_COLORS } from "../api";
import { FileIcon, PinGlyph, TrashGlyph } from "../icons";
import { renderMarkdownToHtml } from "../markdown";
import { displayEntryName } from "../entryHelpers";

function isNote(entry: Entry): boolean {
  if (entry.is_dir) return false;
  const dot = entry.name.lastIndexOf(".");
  if (dot <= 0) return false;
  const ext = entry.name.slice(dot + 1).toLowerCase();
  return ext === "md" || ext === "markdown" || ext === "txt";
}

// Manual drag-to-reorder, persisted per folder -- entries not yet
// mentioned in a saved order fall back to whatever order they arrived in
// (name/date, from the normal sort), appended after everything that IS
// ordered. Two independent orders (pinned vs. not) since dragging a note
// out of/into the pinned section is a pin change, not a reorder within it.
function useNoteOrder(curDir: string, section: "pinned" | "others") {
  const key = `vaultexplorer:notesOrder:${section}:${curDir}`;
  const [order, setOrder] = useState<string[]>(() => {
    try {
      const raw = localStorage.getItem(key);
      return raw ? JSON.parse(raw) : [];
    } catch {
      return [];
    }
  });
  useEffect(() => {
    try {
      const raw = localStorage.getItem(key);
      setOrder(raw ? JSON.parse(raw) : []);
    } catch {
      setOrder([]);
    }
  }, [key]);
  function persist(next: string[]) {
    setOrder(next);
    localStorage.setItem(key, JSON.stringify(next));
  }
  function sorted<T extends { name: string }>(items: T[]): T[] {
    const byName = new Map(items.map((i) => [i.name, i]));
    const ordered = order.filter((n) => byName.has(n)).map((n) => byName.get(n)!);
    const rest = items.filter((i) => !order.includes(i.name));
    return [...ordered, ...rest];
  }
  function moveTo(name: string, beforeName: string | null, allNames: string[]) {
    const base = order.length ? order.filter((n) => allNames.includes(n)) : allNames.slice();
    const withoutMoved = base.filter((n) => n !== name);
    const insertAt = beforeName ? withoutMoved.indexOf(beforeName) : withoutMoved.length;
    withoutMoved.splice(insertAt < 0 ? withoutMoved.length : insertAt, 0, name);
    persist(withoutMoved);
  }
  return { sorted, moveTo };
}

function NoteCard({
  entry,
  fullPath,
  inVault,
  colorHex,
  pinned,
  onEdit,
  onMenu,
  onDelete,
  onTogglePin,
  onSetColor,
  onDragStart,
  onDragOver,
  onDrop,
  isDropTarget,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  colorHex: string | undefined;
  pinned: boolean;
  onEdit: () => void;
  onMenu: (e: React.MouseEvent) => void;
  onDelete: () => void;
  onTogglePin: () => void;
  onSetColor: (hex: string | null) => void;
  onDragStart: (e: React.DragEvent) => void;
  onDragOver: (e: React.DragEvent) => void;
  onDrop: (e: React.DragEvent) => void;
  isDropTarget: boolean;
}) {
  const [preview, setPreview] = useState<string | null>(null);
  const [pickingColor, setPickingColor] = useState(false);
  useEffect(() => {
    let cancelled = false;
    const read = inVault ? api.vaultReadText(fullPath) : api.fsReadText(fullPath);
    read
      .then((text) => {
        if (!cancelled) setPreview(text.slice(0, 220));
      })
      .catch(() => {
        if (!cancelled) setPreview("");
      });
    return () => {
      cancelled = true;
    };
    // entry.mtime/size change whenever the file's content changes (e.g.
    // editing the note and coming back) -- without them here, this only
    // ever re-fetched on a full remount, so an edited note kept showing
    // its old preview text until you left the folder and came back.
  }, [fullPath, inVault, entry.mtime, entry.size]);

  return (
    <div
      className={`note-card ${isDropTarget ? "drop-target" : ""}`}
      style={{ background: colorHex ? `${colorHex}30` : undefined }}
      draggable
      onClick={onEdit}
      onContextMenu={onMenu}
      onDragStart={onDragStart}
      onDragOver={onDragOver}
      onDrop={onDrop}
    >
      <div className="note-card-title">{displayEntryName(entry, false).replace(/\.(md|markdown|txt)$/i, "")}</div>
      <div
        className="note-card-body"
        dangerouslySetInnerHTML={{ __html: preview === null ? "…" : preview ? renderMarkdownToHtml(preview) : "Empty note" }}
      />
      <div className="note-card-actions" onClick={(e) => e.stopPropagation()}>
        <button
          className={`note-card-action ${pinned ? "active" : ""}`}
          onClick={onTogglePin}
          aria-label={pinned ? "Unpin" : "Pin"}
          title={pinned ? "Unpin" : "Pin"}
        >
          <PinGlyph size={13} />
        </button>
        <span className="note-card-color-picker">
          <button
            className="note-card-action note-card-color-dot"
            style={{ background: colorHex || "transparent", borderColor: colorHex || "currentColor" }}
            onClick={() => setPickingColor((v) => !v)}
            aria-label="Color"
            title="Color"
          />
          {pickingColor && (
            <div className="note-color-palette">
              <button
                className="note-color-swatch none"
                title="Default"
                onClick={() => {
                  onSetColor(null);
                  setPickingColor(false);
                }}
              />
              {TAG_COLORS.map((c) => (
                <button
                  key={c.key}
                  className="note-color-swatch"
                  style={{ background: c.hex }}
                  title={c.label}
                  onClick={() => {
                    onSetColor(c.hex);
                    setPickingColor(false);
                  }}
                />
              ))}
            </div>
          )}
        </span>
        <button className="note-card-action" onClick={onDelete} aria-label="Delete" title="Delete">
          <TrashGlyph size={13} />
        </button>
      </div>
    </div>
  );
}

// Google Keep-style card grid for a folder of markdown/plain-text notes --
// every .md/.markdown/.txt file gets a colored card with a rendered-
// markdown content preview (matching the same renderer the full editor
// uses, see TextEditorPane) instead of just an icon; tap opens it in the
// same full-screen editor a plain mobile file-open already uses (see
// `mobileEditorTarget` in App.tsx). Pinned notes (the app's existing
// pin/favorite mechanism, see useFavorites) get their own section above
// everything else, same as Keep's "Pinned"/"Others" split. Anything that
// isn't a note (subfolders, other files) still needs to be reachable from
// this view, so it gets a plain small tile alongside the note cards rather
// than being hidden -- a folder that's *mostly* notes still has the
// occasional attachment or subfolder mixed in.
export function NotesGrid({
  entries,
  curDir,
  inVault,
  tags,
  pinnedPaths,
  onOpenNote,
  onActivateOther,
  onMenu,
  onDelete,
  onTogglePin,
  onSetColor,
}: {
  entries: Entry[];
  curDir: string;
  inVault: boolean;
  tags: Record<string, string>;
  pinnedPaths: Set<string>;
  onOpenNote: (entry: Entry, fullPath: string) => void;
  onActivateOther: (entry: Entry) => void;
  onMenu: (e: React.MouseEvent, entry: Entry) => void;
  onDelete: (entry: Entry) => void;
  onTogglePin: (fullPath: string) => void;
  onSetColor: (name: string, colorKey: string | null) => void;
}) {
  const notes = entries.filter(isNote);
  const others = entries.filter((e) => !isNote(e));
  const pinned = notes.filter((n) => pinnedPaths.has(joinPath(curDir, n.name)));
  const rest = notes.filter((n) => !pinnedPaths.has(joinPath(curDir, n.name)));
  const pinnedOrder = useNoteOrder(curDir, "pinned");
  const restOrder = useNoteOrder(curDir, "others");
  const [dragging, setDragging] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);

  function renderSection(list: Entry[], order: ReturnType<typeof useNoteOrder>) {
    const sortedList = order.sorted(list);
    const names = sortedList.map((e) => e.name);
    return sortedList.map((entry) => {
      const fullPath = joinPath(curDir, entry.name);
      return (
        <NoteCard
          key={entry.name}
          entry={entry}
          fullPath={fullPath}
          inVault={inVault}
          colorHex={TAG_COLORS.find((c) => c.key === tags[entry.name])?.hex}
          pinned={pinnedPaths.has(fullPath)}
          onEdit={() => onOpenNote(entry, fullPath)}
          onMenu={(e) => onMenu(e, entry)}
          onDelete={() => onDelete(entry)}
          onTogglePin={() => onTogglePin(fullPath)}
          onSetColor={(hex) => {
            const key = hex ? TAG_COLORS.find((c) => c.hex === hex)?.key ?? null : null;
            onSetColor(entry.name, key);
          }}
          onDragStart={() => setDragging(entry.name)}
          onDragOver={(e) => {
            e.preventDefault();
            if (dragging && dragging !== entry.name) setDropTarget(entry.name);
          }}
          onDrop={(e) => {
            e.preventDefault();
            if (dragging && dragging !== entry.name) order.moveTo(dragging, entry.name, names);
            setDragging(null);
            setDropTarget(null);
          }}
          isDropTarget={dropTarget === entry.name}
        />
      );
    });
  }

  return (
    <div className="notes-view">
      {pinned.length > 0 && (
        <>
          <div className="notes-section-label">Pinned</div>
          <div className="notes-grid" onDragEnd={() => { setDragging(null); setDropTarget(null); }}>
            {renderSection(pinned, pinnedOrder)}
          </div>
        </>
      )}
      {rest.length > 0 && (
        <>
          {pinned.length > 0 && <div className="notes-section-label">Others</div>}
          <div className="notes-grid" onDragEnd={() => { setDragging(null); setDropTarget(null); }}>
            {renderSection(rest, restOrder)}
          </div>
        </>
      )}
      {others.length > 0 && (
        <div className="notes-others">
          {others.map((entry) => (
            <button
              key={entry.name}
              className="notes-other-tile"
              onClick={() => onActivateOther(entry)}
              onContextMenu={(e) => onMenu(e, entry)}
            >
              <FileIcon entry={entry} />
              <span>{displayEntryName(entry, false)}</span>
            </button>
          ))}
        </div>
      )}
      {entries.length === 0 && <p className="notes-empty">No notes here yet.</p>}
    </div>
  );
}
