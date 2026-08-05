import { useEffect, useState } from "react";
import { Entry, api, osOpen, joinPath } from "../api";
import { parseVCard, cleanPhoneForLink } from "../vcard";
import { FileIcon } from "../icons";
import { displayEntryName } from "../entryHelpers";

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
  const [parsed, setParsed] = useState<{ name: string; phones: string[] } | null>(null);
  useEffect(() => {
    let cancelled = false;
    const read = inVault ? api.vaultReadText(fullPath) : api.fsReadText(fullPath);
    read
      .then((text) => {
        if (!cancelled) setParsed(parseVCard(text));
      })
      .catch(() => {
        if (!cancelled) setParsed({ name: displayEntryName(entry, false), phones: [] });
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
      <div className="contact-avatar">{(parsed?.name || "?").charAt(0).toUpperCase()}</div>
      <div className="contact-info">
        <div className="contact-name">{parsed?.name ?? "…"}</div>
        {phone && <div className="contact-phone">{phone}</div>}
      </div>
      {phone && (
        <div className="contact-actions" onClick={(e) => e.stopPropagation()}>
          <button
            className="contact-action-btn"
            aria-label="WhatsApp"
            onClick={() => waLink && osOpen(waLink).catch(() => {})}
          >
            💬
          </button>
          <button
            className="contact-action-btn"
            aria-label="Call"
            onClick={() => dialLink && osOpen(dialLink).catch(() => {})}
          >
            📞
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
}: {
  entries: Entry[];
  curDir: string;
  inVault: boolean;
  onEditContact: (entry: Entry, fullPath: string) => void;
  onActivateOther: (entry: Entry) => void;
  onMenu: (e: React.MouseEvent, entry: Entry) => void;
}) {
  const contacts = entries.filter(isVcf);
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
