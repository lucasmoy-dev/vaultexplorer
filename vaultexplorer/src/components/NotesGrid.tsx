import { useEffect, useState } from "react";
import { Entry, api, joinPath } from "../api";
import { FileIcon } from "../icons";
import { displayEntryName } from "../entryHelpers";

// Deterministic, not random -- the same note keeps the same color across
// re-renders/reopens instead of visibly shuffling every time this mounts.
const NOTE_HUES = [355, 25, 45, 95, 165, 200, 265, 320];
function noteColor(name: string): string {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) | 0;
  const hue = NOTE_HUES[Math.abs(h) % NOTE_HUES.length];
  return `hsl(${hue}, 65%, 92%)`;
}

function isMarkdown(entry: Entry): boolean {
  if (entry.is_dir) return false;
  const dot = entry.name.lastIndexOf(".");
  if (dot <= 0) return false;
  const ext = entry.name.slice(dot + 1).toLowerCase();
  return ext === "md" || ext === "markdown";
}

function NoteCard({
  entry,
  fullPath,
  inVault,
  onOpen,
  onMenu,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  onOpen: () => void;
  onMenu: (e: React.MouseEvent) => void;
}) {
  const [preview, setPreview] = useState<string | null>(null);
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
  }, [fullPath, inVault]);

  return (
    <div
      className="note-card"
      style={{ background: noteColor(entry.name) }}
      onClick={onOpen}
      onContextMenu={onMenu}
    >
      <div className="note-card-title">{displayEntryName(entry, false).replace(/\.(md|markdown)$/i, "")}</div>
      <div className="note-card-body">{preview === null ? "…" : preview || "Empty note"}</div>
    </div>
  );
}

// Google Keep-style card grid for a folder of markdown notes -- every .md
// file gets a colored card with a content preview instead of just an icon;
// tap opens it in the same full-screen editor a plain mobile file-open
// already uses (see `mobileEditorTarget` in App.tsx). Anything that isn't
// markdown (subfolders, other files) still needs to be reachable from this
// view, so it gets a plain small tile alongside the note cards rather than
// being hidden -- a folder that's *mostly* notes still has the occasional
// attachment or subfolder mixed in.
export function NotesGrid({
  entries,
  curDir,
  inVault,
  onOpenNote,
  onActivateOther,
  onMenu,
}: {
  entries: Entry[];
  curDir: string;
  inVault: boolean;
  onOpenNote: (entry: Entry, fullPath: string) => void;
  onActivateOther: (entry: Entry) => void;
  onMenu: (e: React.MouseEvent, entry: Entry) => void;
}) {
  const notes = entries.filter(isMarkdown);
  const others = entries.filter((e) => !isMarkdown(e));
  return (
    <div className="notes-view">
      <div className="notes-grid">
        {notes.map((entry) => (
          <NoteCard
            key={entry.name}
            entry={entry}
            fullPath={joinPath(curDir, entry.name)}
            inVault={inVault}
            onOpen={() => onOpenNote(entry, joinPath(curDir, entry.name))}
            onMenu={(e) => onMenu(e, entry)}
          />
        ))}
      </div>
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
