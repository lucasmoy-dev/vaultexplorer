import { useEffect, useRef, useState } from "react";
import { Entry, api, osOpen, joinPath } from "../api";
import { parseVCard, serializeVCard, cleanPhoneForLink, ParsedVCard } from "../vcard";
import { FileIcon, PhoneGlyph, ChatGlyph } from "../icons";
import { displayEntryName } from "../entryHelpers";
import { useAutoSaveText } from "../hooks/useAutoSaveText";
import { EditableFileName } from "./PreviewColumn";
import { useSelection } from "../hooks/useSelection";

// What this grid needs from whoever owns the selection -- the shape
// useSelection() already returns, so App can hand its own straight in.
export type SelectionApi = ReturnType<typeof useSelection>;

function isVcf(entry: Entry): boolean {
  return !entry.is_dir && entry.name.toLowerCase().endsWith(".vcf");
}

export function ContactRow({
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
        if (!cancelled)
          setParsed({
            name: displayEntryName(entry, false),
            phones: [],
            emails: [],
            org: "",
            note: "",
            photoDataUrl: null,
          });
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
        <div className="contact-name">{parsed === null ? "…" : parsed.name || "(no name)"}</div>
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
  selected,
  editing,
  editValue,
  onEditChange,
  onEditCommit,
  onEditCancel,
  onOpen,
  onMenu,
  onDragOver,
  onDragLeave,
  onDrop,
}: {
  entry: Entry;
  isDropTarget: boolean;
  selected: boolean;
  editing: boolean;
  editValue: string;
  onEditChange: (v: string) => void;
  onEditCommit: () => void;
  onEditCancel: () => void;
  onOpen: () => void;
  onMenu: (e: React.MouseEvent) => void;
  onDragOver: (e: React.DragEvent) => void;
  onDragLeave: () => void;
  onDrop: (e: React.DragEvent) => void;
}) {
  return (
    <div
      // data-name is what App's reveal effect looks for when it scrolls a
      // just-created item into view -- without it, making a folder in this
      // view selected nothing and scrolled nowhere.
      data-name={entry.name}
      className={`contact-row contact-folder-row ${isDropTarget ? "drop-target" : ""} ${selected ? "selected" : ""}`}
      onClick={editing ? undefined : onOpen}
      onContextMenu={onMenu}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <div className="contact-avatar contact-folder-avatar">
        <FileIcon entry={entry} />
      </div>
      <div className="contact-info">
        {editing ? (
          // A new folder opens straight into its own name field, same as
          // the file grid does -- naming it is the next thing you want,
          // and "untitled folder" sitting there needing a separate rename
          // was the reported gap.
          <input
            autoFocus
            className="contact-folder-name-edit"
            value={editValue}
            onChange={(e) => onEditChange(e.target.value)}
            onFocus={(e) => e.currentTarget.select()}
            onClick={(e) => e.stopPropagation()}
            onBlur={onEditCommit}
            onKeyDown={(e) => {
              if (e.key === "Enter") onEditCommit();
              if (e.key === "Escape") onEditCancel();
            }}
          />
        ) : (
          <div className="contact-name">{displayEntryName(entry, false)}</div>
        )}
      </div>
    </div>
  );
}

// A folder of .vcf files as a real contacts list (name, primary phone,
// call/WhatsApp actions) instead of a plain file listing -- tapping a row
// opens the same full-screen editor shell Notes view uses (see
// mobileEditorTarget in App.tsx), but with ContactEditForm below in place
// of the plain text editor. Anything that isn't a .vcf still needs to be
// reachable, same reasoning as NotesGrid.
// Subfolders get their own contact-like row (folder icon, no actions) and
// double as "categories" -- marquee-selecting several contacts and
// dropping the selection on one moves those files into it.
export function ContactsGrid({
  entries,
  curDir,
  inVault,
  selection,
  onEditContact,
  onActivateOther,
  onMenu,
  onFilesChanged,
  // Overrides the curDir-based path derivation -- lets a caller feed in
  // entries that don't all live in the same directory (e.g. search
  // results spanning the whole vault/folder tree) while still getting
  // this exact row rendering.
  pathFor,
  emptyMessage,
  header,
  renaming,
  onRenameChange,
  onRenameCommit,
  onRenameCancel,
}: {
  entries: Entry[];
  curDir: string;
  inVault: boolean;
  // The *app's* selection, not a private one. This grid used to run its
  // own useSelection, which meant the mobile selection toolbar ("N
  // selected" / "Deselect All") was reading a different set than the rows
  // were drawing: it always saw 0 selected here, so its button said
  // "Select All" and cancelling a selection selected everything instead
  // -- exactly the reported behaviour. One selection, owned by App.
  // Omitted by the search-results call-site, which is read-only.
  selection?: SelectionApi;
  onEditContact: (entry: Entry, fullPath: string) => void;
  onActivateOther: (entry: Entry) => void;
  onMenu: (e: React.MouseEvent, entry: Entry) => void;
  // A vault has no OS-level file watcher backing it (see App.tsx's
  // "fs-changed" listen effect) -- unlike a real-fs move, which that
  // watcher picks up on its own, a vault move needs this nudge or the
  // moved row just sits there stale until the next navigation.
  onFilesChanged?: () => void;
  pathFor?: (entry: Entry) => string;
  emptyMessage?: string;
  header?: React.ReactNode;
  // Inline rename of a folder row, driven by App's own renaming state so a
  // freshly created folder can open straight into its name field.
  renaming?: { name: string; value: string } | null;
  onRenameChange?: (v: string) => void;
  onRenameCommit?: () => void;
  onRenameCancel?: () => void;
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
  const fullPathOf = (entry: Entry) => (pathFor ? pathFor(entry) : joinPath(curDir, entry.name));

  // A read-only grid (search results) gets a no-op selection rather than
  // a second source of truth.
  const fallback = useSelection();
  const { selected, setSelected, selectOnly, toggle, selectRange } = selection ?? fallback;
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
    const destDir = fullPathOf(folder);
    try {
      for (const name of names) {
        // Selection is keyed by name, but pathFor resolves by entry
        // identity -- go back through the entry itself so a search-results
        // list (whose rows don't share curDir) moves the right file.
        const entry = contacts.find((c) => c.name === name);
        const src = entry ? fullPathOf(entry) : joinPath(curDir, name);
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
      {header}
      {error && <p className="error">{error}</p>}
      <div className="contacts-list" ref={listRef} onMouseDown={onListMouseDown} onDragEnd={() => setDragNames([])}>
        {folders.map((entry) => (
          <FolderRow
            key={entry.name}
            entry={entry}
            isDropTarget={dropTargetName === entry.name}
            selected={selected.has(entry.name)}
            editing={renaming?.name === entry.name}
            editValue={renaming?.name === entry.name ? renaming.value : ""}
            onEditChange={(v) => onRenameChange?.(v)}
            onEditCommit={() => onRenameCommit?.()}
            onEditCancel={() => onRenameCancel?.()}
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
            key={fullPathOf(entry)}
            entry={entry}
            fullPath={fullPathOf(entry)}
            inVault={inVault}
            selected={selected.has(entry.name)}
            onEdit={() => onEditContact(entry, fullPathOf(entry))}
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
              key={fullPathOf(entry)}
              className={`notes-other-tile ${selected.has(entry.name) ? "selected" : ""}`}
              data-name={entry.name}
              onClick={() => onActivateOther(entry)}
              onContextMenu={(e) => onMenu(e, entry)}
            >
              <FileIcon entry={entry} />
              <span>{displayEntryName(entry, false)}</span>
            </button>
          ))}
        </div>
      )}
      {entries.length === 0 && <p className="notes-empty">{emptyMessage ?? "No contacts here yet."}</p>}
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

// The real field-by-field contact editor a tapped/clicked ContactRow opens
// into (see mobileEditorTarget in App.tsx) -- labeled inputs for the vCard
// fields this app actually understands, auto-saved the same debounced way
// TextEditorPane saves raw text, just serialized through vCard.ts instead
// of written verbatim. Anything the original .vcf had that parseVCard
// doesn't model (PHOTO aside) is dropped on first save -- acceptable for a
// personal contacts file, not for round-tripping an arbitrary import.
export function ContactEditForm({
  entry,
  fullPath,
  inVault,
  onRename,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  onRename?: (newName: string) => void;
}) {
  const { content, error, saving, setContent, externalRevision } = useAutoSaveText(fullPath, inVault);
  const [fields, setFields] = useState<ParsedVCard | null>(null);
  // Keyed on fullPath+externalRevision, not just fullPath -- this component
  // doesn't get remounted when the edited file changes (App.tsx keeps a
  // single overlay instance), and externalRevision is useAutoSaveText's own
  // signal for "the file changed from outside this pane's typing" (a fresh
  // read on window focus). Re-parsing on every `content` change instead
  // would stomp the very keystroke that produced it.
  const loadedKeyRef = useRef<string>("");
  useEffect(() => {
    const key = `${fullPath}:${externalRevision}`;
    if (content === null || loadedKeyRef.current === key) return;
    loadedKeyRef.current = key;
    setFields(parseVCard(content));
  }, [content, fullPath, externalRevision]);

  function update(next: Partial<ParsedVCard>) {
    if (!fields) return;
    const merged = { ...fields, ...next };
    setFields(merged);
    setContent(serializeVCard(merged));
  }
  function updatePhone(i: number, value: string) {
    if (!fields) return;
    update({ phones: fields.phones.map((p, j) => (j === i ? value : p)) });
  }
  function updateEmail(i: number, value: string) {
    if (!fields) return;
    update({ emails: fields.emails.map((em, j) => (j === i ? value : em)) });
  }

  const primaryPhone = fields?.phones.find((p) => p.trim());

  return (
    <div className="preview-pane text-editor-pane contact-edit-pane">
      <div className="preview-name-row">
        <EditableFileName name={entry.name} onRename={onRename} />
        {saving && <span className="saving-hint"> — saving…</span>}
      </div>
      {error && <p className="error">{error}</p>}
      {fields && (
        <div className="contact-edit-form">
          <div className="contact-edit-top-row">
            <div className="contact-avatar contact-edit-avatar">
              {fields.photoDataUrl ? (
                <img src={fields.photoDataUrl} alt="" className="contact-avatar-photo" />
              ) : (
                (fields.name || "?").charAt(0).toUpperCase()
              )}
            </div>
            {primaryPhone && (
              <div className="contact-actions">
                <button
                  className="contact-action-btn contact-action-call"
                  aria-label="Call"
                  onClick={() => osOpen(`tel:${cleanPhoneForLink(primaryPhone)}`).catch(() => {})}
                >
                  <PhoneGlyph size={16} />
                </button>
                <button
                  className="contact-action-btn contact-action-whatsapp"
                  aria-label="Share via WhatsApp"
                  onClick={() =>
                    osOpen(`https://wa.me/${cleanPhoneForLink(primaryPhone).replace(/^\+/, "")}`).catch(() => {})
                  }
                >
                  {/* Icon only, same as the contact rows: the glyph plus
                      its aria-label already say what it is, and the word
                      made this button twice the size of the call button
                      sitting next to it. */}
                  <ChatGlyph size={16} />
                </button>
              </div>
            )}
          </div>

          <label className="contact-edit-field">
            <span className="field-label">Name</span>
            <input value={fields.name} onChange={(e) => update({ name: e.target.value })} placeholder="Full name" />
          </label>

          <div className="contact-edit-list">
            <span className="field-label">Phone</span>
            {fields.phones.map((p, i) => (
              <div className="contact-edit-list-row" key={i}>
                <input value={p} onChange={(e) => updatePhone(i, e.target.value)} placeholder="Phone number" />
                <button
                  type="button"
                  className="contact-edit-remove-btn"
                  aria-label="Remove phone"
                  onClick={() => update({ phones: fields.phones.filter((_, j) => j !== i) })}
                >
                  ✕
                </button>
              </div>
            ))}
            <button type="button" className="contact-edit-add-btn" onClick={() => update({ phones: [...fields.phones, ""] })}>
              + Add phone
            </button>
          </div>

          <div className="contact-edit-list">
            <span className="field-label">Email</span>
            {fields.emails.map((em, i) => (
              <div className="contact-edit-list-row" key={i}>
                <input value={em} onChange={(e) => updateEmail(i, e.target.value)} placeholder="Email address" />
                <button
                  type="button"
                  className="contact-edit-remove-btn"
                  aria-label="Remove email"
                  onClick={() => update({ emails: fields.emails.filter((_, j) => j !== i) })}
                >
                  ✕
                </button>
              </div>
            ))}
            <button type="button" className="contact-edit-add-btn" onClick={() => update({ emails: [...fields.emails, ""] })}>
              + Add email
            </button>
          </div>

          <label className="contact-edit-field">
            <span className="field-label">Organization</span>
            <input value={fields.org} onChange={(e) => update({ org: e.target.value })} placeholder="Company" />
          </label>

          <label className="contact-edit-field">
            <span className="field-label">Note</span>
            <textarea className="contact-edit-note" value={fields.note} onChange={(e) => update({ note: e.target.value })} />
          </label>
        </div>
      )}
    </div>
  );
}
