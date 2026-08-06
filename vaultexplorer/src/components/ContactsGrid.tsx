import { useEffect, useState } from "react";
import { Entry, api, osOpen, joinPath } from "../api";
import { parseVCard, cleanPhoneForLink, ParsedVCard } from "../vcard";
import { FileIcon, PhoneGlyph, ChatGlyph } from "../icons";
import { displayEntryName } from "../entryHelpers";

function isVcf(entry: Entry): boolean {
  return !entry.is_dir && entry.name.toLowerCase().endsWith(".vcf");
}

export function ContactRow({
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
    <div className="contact-row" onClick={onEdit} onContextMenu={onMenu}>
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

// A folder of .vcf files as a real contacts list (name, primary phone,
// call/WhatsApp actions) instead of a plain file listing -- tapping a row
// opens the same full-screen editor Notes view uses, on the .vcf's raw
// text (this isn't a structured-field contact editor, just the fastest
// way to fix a typo'd number without leaving the app). Anything that
// isn't a .vcf still needs to be reachable, same reasoning as NotesGrid.
export function ContactsGrid({
  entries,
  curDir,
  inVault,
  onEditContact,
  onActivateOther,
  onMenu,
  // Overrides the curDir-based path derivation -- lets a caller feed in
  // entries that don't all live in the same directory (e.g. search
  // results spanning the whole vault/folder tree) while still getting
  // this exact row rendering.
  pathFor,
  emptyMessage,
  header,
}: {
  entries: Entry[];
  curDir: string;
  inVault: boolean;
  onEditContact: (entry: Entry, fullPath: string) => void;
  onActivateOther: (entry: Entry) => void;
  onMenu: (e: React.MouseEvent, entry: Entry) => void;
  pathFor?: (entry: Entry) => string;
  emptyMessage?: string;
  header?: React.ReactNode;
}) {
  // Sorted by filename, not the parsed display name -- each row parses
  // its own vCard asynchronously (see ContactRow), so the *real* name
  // isn't known up front without reading every file before rendering
  // anything. Export already names each .vcf after the contact, so this
  // is the same order in practice for the common case.
  const contacts = entries.filter(isVcf).sort((a, b) => a.name.localeCompare(b.name));
  const others = entries.filter((e) => !isVcf(e));
  const fullPathOf = (entry: Entry) => (pathFor ? pathFor(entry) : joinPath(curDir, entry.name));
  return (
    <div className="contacts-view">
      {header}
      <div className="contacts-list">
        {contacts.map((entry) => (
          <ContactRow
            key={fullPathOf(entry)}
            entry={entry}
            fullPath={fullPathOf(entry)}
            inVault={inVault}
            onEdit={() => onEditContact(entry, fullPathOf(entry))}
            onMenu={(e) => onMenu(e, entry)}
          />
        ))}
      </div>
      {others.length > 0 && (
        <div className="notes-others">
          {others.map((entry) => (
            <button
              key={fullPathOf(entry)}
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
      {entries.length === 0 && <p className="notes-empty">{emptyMessage ?? "No contacts here yet."}</p>}
    </div>
  );
}
