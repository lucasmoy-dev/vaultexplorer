import { useEffect, useRef, useState } from "react";
import { Entry, api, osOpen, joinPath } from "../api";
import { parseVCard, cleanPhoneForLink, ParsedVCard } from "../vcard";
import { FileIcon, PhoneGlyph, ChatGlyph } from "../icons";
import { displayEntryName } from "../entryHelpers";
import { useSelection } from "../hooks/useSelection";

function isVcf(entry: Entry): boolean {
  return !entry.is_dir && entry.name.toLowerCase().endsWith(".vcf");
}

function ContactRow({
  entry,
  fullPath,
  inVault,
  selected,
  onEdit,
  onToggle,
  onRangeSelect,
  onMenu,
  onDragStart,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  selected: boolean;
  onEdit: () => void;
  onToggle: () => void;
  onRangeSelect: () => void;
  onMenu: (e: React.MouseEvent) => void;
  onDragStart: (e: React.DragEvent) => void;
}) {
  const [parsed, setParsed] = useState<ParsedVCard | null>(null);
  useEffect(() => {
    let cancelled = false;
    const read = inVault ? api.vaultReadText(fullPath) : api.fsReadText(fullPath);
    read
      .then((text) => {
        if (!cancelled) setParsed(parseVCard(text));
      })
      .catch(() => {
        if (!cancelled) setParsed({ name: displayEntryName(entry, false), phones: [], photoDataUrl: null });
      });
    return () => {
      cancelled = true;
    };
  }, [fullPath, inVault]);

  const phone = parsed?.phones[0];
  // Handed to the OS's own `ACTION_VIEW`-equivalent (same call this app
  // already uses to open any file with its default app) rather than a
  // plain `<a href="tel:...">` -- whether an embedded WebView's default
  // link handling resolves a non-http(s) scheme to a real Intent varies
  // enough by platform/version that this app's own already-proven "open"
  // path is the safer bet.
  const dialLink = phone ? `tel:${cleanPhoneForLink(phone)}` : undefined;
  // wa.me wants a bare international number, no "+"/spaces/dashes -- this
  // is the documented format for a "click to chat" link with no API key.
  const waLink = phone ? `https://wa.me/${cleanPhoneForLink(phone).replace(/^\+/, "")}` : undefined;

  return (
    <div
      className={`contact-row contact-entry ${selected ? "selected" : ""}`}
      data-name={entry.name}
      draggable
      onDragStart={onDragStart}
      onClick={(e) => {
        if (e.metaKey || e.ctrlKey) {
          e.stopPropagation();
          onToggle();
          return;
        }
        if (e.shiftKey) {
          e.stopPropagation();
          onRangeSelect();
          return;
        }
        onEdit();
      }}
      onContextMenu={onMenu}
    >
      <div className="contact-avatar">
        {parsed?.photoDataUrl ? (
          <img src={parsed.photoDataUrl} alt="" className="contact-avatar-photo" />
        ) : (
          (parsed?.name || "?").charAt(0).toUpperCase()
        )}
      </div>
      <div className="contact-info">
        <div className="contact-name">{parsed?.name ?? "…"}</div>
        {phone && <div className="contact-phone">{phone}</div>}
      </div>
      {phone && (
        <div className="contact-actions" onClick={(e) => e.stopPropagation()}>
          <button
            className="contact-action-btn contact-action-call"
            aria-label="Call"
            onClick={() => dialLink && osOpen(dialLink).catch(() => {})}
          >
            <PhoneGlyph size={16} />
          </button>
          <button
            className="contact-action-btn contact-action-whatsapp"
            aria-label="WhatsApp"
            onClick={() => waLink && osOpen(waLink).catch(() => {})}
          >
            <ChatGlyph size={16} />
            <span>WhatsApp</span>
          </button>
        </div>
      )}
    </div>
  );
}

// A folder rendered the same width/height as a contact row -- same avatar
// slot (a folder icon instead of an initial/photo), same name line -- but
// with no phone/call/WhatsApp actions, since it isn't a person. Doubles as
// a drop target: dragging the current contact selection onto it moves
// those .vcf files in, turning subfolders into ad-hoc "categories" for an
// otherwise flat vCard folder.
function FolderRow({
  entry,
  isDropTarget,
  onOpen,
  onMenu,
  onDragOver,
  onDragLeave,
  onDrop,
}: {
  entry: Entry;
  isDropTarget: boolean;
  onOpen: () => void;
  onMenu: (e: React.MouseEvent) => void;
  onDragOver: (e: React.DragEvent) => void;
  onDragLeave: () => void;
  onDrop: (e: React.DragEvent) => void;
}) {
  return (
    <div
      className={`contact-row contact-folder-row ${isDropTarget ? "drop-target" : ""}`}
      onClick={onOpen}
      onContextMenu={onMenu}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <div className="contact-avatar contact-folder-avatar">
        <FileIcon entry={entry} />
      </div>
      <div className="contact-info">
        <div className="contact-name">{displayEntryName(entry, false)}</div>
      </div>
    </div>
  );
}

// A folder of .vcf files as a real contacts list (name, primary phone,
// call/WhatsApp actions) instead of a plain file listing -- tapping a row
// opens the same full-screen editor Notes view uses, on the .vcf's raw
// text (this isn't a structured-field contact editor, just the fastest
// way to fix a typo'd number without leaving the app). Anything that
// isn't a .vcf still needs to be reachable, same reasoning as NotesGrid.
// Subfolders get their own contact-like row (folder icon, no actions) and
// double as "categories" -- marquee-selecting several contacts and
// dropping the selection on one moves those files into it.
export function ContactsGrid({
  entries,
  curDir,
  inVault,
  onEditContact,
  onActivateOther,
  onMenu,
  onFilesChanged,
}: {
  entries: Entry[];
  curDir: string;
  inVault: boolean;
  onEditContact: (entry: Entry, fullPath: string) => void;
  onActivateOther: (entry: Entry) => void;
  onMenu: (e: React.MouseEvent, entry: Entry) => void;
  // A vault has no OS-level file watcher backing it (see App.tsx's
  // "fs-changed" listen effect) -- unlike a real-fs move, which that
  // watcher picks up on its own, a vault move needs this nudge or the
  // moved row just sits there stale until the next navigation.
  onFilesChanged?: () => void;
}) {
  // Sorted by filename, not the parsed display name -- each row parses
  // its own vCard asynchronously (see ContactRow), so the *real* name
  // isn't known up front without reading every file before rendering
  // anything. Export already names each .vcf after the contact, so this
  // is the same order in practice for the common case.
  const contacts = entries.filter(isVcf).sort((a, b) => a.name.localeCompare(b.name));
  const folders = entries.filter((e) => e.is_dir).sort((a, b) => a.name.localeCompare(b.name));
  const others = entries.filter((e) => !e.is_dir && !isVcf(e));
  const contactNames = contacts.map((c) => c.name);

  const { selected, setSelected, selectOnly, toggle, selectRange } = useSelection();
  const [dragNames, setDragNames] = useState<string[]>([]);
  const [dropTargetName, setDropTargetName] = useState<string | null>(null);
  const [error, setError] = useState("");
  const listRef = useRef<HTMLDivElement>(null);
  const [marquee, setMarquee] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(null);

  // Drag-select rectangle -- same technique as the real file grid's own
  // marquee (App.tsx's onContentMouseDown), a self-contained local copy
  // since this grid's rows (contact-entry) aren't the real file grid's
  // `.entry` elements and live inside the same `.content` container that
  // already runs its own marquee, so this needs to own mousedown itself
  // (see the stopPropagation below) rather than let both run at once.
  function onListMouseDown(e: React.MouseEvent) {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest(".contact-entry, .contact-folder-row")) return;
    e.stopPropagation();
    if (!(e.metaKey || e.ctrlKey || e.shiftKey)) setSelected(new Set());
    setMarquee({ x0: e.clientX, y0: e.clientY, x1: e.clientX, y1: e.clientY });
  }
  useEffect(() => {
    if (!marquee) return;
    const origin = { x: marquee.x0, y: marquee.y0 };
    const rects = Array.from(listRef.current?.querySelectorAll<HTMLElement>(".contact-entry") ?? []).map((el) => ({
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

  function onRowDragStart(e: React.DragEvent, entry: Entry) {
    const names = selected.has(entry.name) && selected.size ? [...selected] : [entry.name];
    if (!selected.has(entry.name)) selectOnly(entry.name);
    setDragNames(names);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", names.join("\n"));
  }

  async function dropOnFolder(folder: Entry) {
    const names = dragNames;
    setDragNames([]);
    setDropTargetName(null);
    if (!names.length) return;
    const destDir = joinPath(curDir, folder.name);
    try {
      for (const name of names) {
        const src = joinPath(curDir, name);
        const dest = joinPath(destDir, name);
        inVault ? await api.moveEntry(src, dest) : await api.fsRename(src, dest);
      }
      setSelected(new Set());
      setError("");
      onFilesChanged?.();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="contacts-view">
      {error && <p className="error">{error}</p>}
      <div className="contacts-list" ref={listRef} onMouseDown={onListMouseDown} onDragEnd={() => setDragNames([])}>
        {folders.map((entry) => (
          <FolderRow
            key={entry.name}
            entry={entry}
            isDropTarget={dropTargetName === entry.name}
            onOpen={() => onActivateOther(entry)}
            onMenu={(e) => onMenu(e, entry)}
            onDragOver={(e) => {
              if (!dragNames.length || (entry.is_vault && inVault)) return;
              e.preventDefault();
              setDropTargetName(entry.name);
            }}
            onDragLeave={() => setDropTargetName((t) => (t === entry.name ? null : t))}
            onDrop={(e) => {
              e.preventDefault();
              dropOnFolder(entry);
            }}
          />
        ))}
        {contacts.map((entry) => (
          <ContactRow
            key={entry.name}
            entry={entry}
            fullPath={joinPath(curDir, entry.name)}
            inVault={inVault}
            selected={selected.has(entry.name)}
            onEdit={() => onEditContact(entry, joinPath(curDir, entry.name))}
            onToggle={() => toggle(entry.name)}
            onRangeSelect={() => selectRange(entry.name, contactNames)}
            onMenu={(e) => onMenu(e, entry)}
            onDragStart={(e) => onRowDragStart(e, entry)}
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
      {entries.length === 0 && <p className="notes-empty">No contacts here yet.</p>}
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
    </div>
  );
}
