import { useEffect, useRef, useState } from "react";
import { Entry, api, osOpen, joinPath } from "../api";
import { parseVCard, serializeVCard, cleanPhoneForLink, ParsedVCard } from "../vcard";
import { FileIcon, PhoneGlyph, ChatGlyph } from "../icons";
import { displayEntryName } from "../entryHelpers";
import { useAutoSaveText } from "../hooks/useAutoSaveText";
import { EditableFileName } from "./PreviewColumn";

function isVcf(entry: Entry): boolean {
  return !entry.is_dir && entry.name.toLowerCase().endsWith(".vcf");
}

function ContactRow({
  entry,
  fullPath,
  inVault,
  onEdit,
  onMenu,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  onEdit: () => void;
  onMenu: (e: React.MouseEvent) => void;
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
    <div className="contact-row" onClick={onEdit} onContextMenu={onMenu}>
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

// A folder of .vcf files as a real contacts list (name, primary phone,
// call/WhatsApp actions) instead of a plain file listing -- tapping a row
// opens the same full-screen editor shell Notes view uses (see
// mobileEditorTarget in App.tsx), but with ContactEditForm below in place
// of the plain text editor. Anything that isn't a .vcf still needs to be
// reachable, same reasoning as NotesGrid.
export function ContactsGrid({
  entries,
  curDir,
  inVault,
  onEditContact,
  onActivateOther,
  onMenu,
}: {
  entries: Entry[];
  curDir: string;
  inVault: boolean;
  onEditContact: (entry: Entry, fullPath: string) => void;
  onActivateOther: (entry: Entry) => void;
  onMenu: (e: React.MouseEvent, entry: Entry) => void;
}) {
  // Sorted by filename, not the parsed display name -- each row parses
  // its own vCard asynchronously (see ContactRow), so the *real* name
  // isn't known up front without reading every file before rendering
  // anything. Export already names each .vcf after the contact, so this
  // is the same order in practice for the common case.
  const contacts = entries.filter(isVcf).sort((a, b) => a.name.localeCompare(b.name));
  const others = entries.filter((e) => !isVcf(e));
  return (
    <div className="contacts-view">
      <div className="contacts-list">
        {contacts.map((entry) => (
          <ContactRow
            key={entry.name}
            entry={entry}
            fullPath={joinPath(curDir, entry.name)}
            inVault={inVault}
            onEdit={() => onEditContact(entry, joinPath(curDir, entry.name))}
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
      {entries.length === 0 && <p className="notes-empty">No contacts here yet.</p>}
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
                  <ChatGlyph size={16} />
                  <span>WhatsApp</span>
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
