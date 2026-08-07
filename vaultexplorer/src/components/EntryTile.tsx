import { useEffect, useRef, useState } from "react";
import { Entry, api, osOpen } from "../api";
import { View } from "../types";
import { formatSize, formatDate } from "../api";
import { FileIcon, GitBranchGlyph, CloudSyncGlyph, LocalSyncGlyph, CheckGlyph, PinGlyph, RefreshGlyph, LockGlyph, PhoneGlyph, ChatGlyph } from "../icons";
import { displayEntryName, kindLabel } from "../entryHelpers";
import { useThumbnail } from "../hooks/useThumbnail";
import { parseVCard, cleanPhoneForLink, ParsedVCard } from "../vcard";

// A contact file isn't a document with an icon -- it's a person. In any
// view, a `.vcf` row shows that person's photo (or their initial) where
// the file icon would go, and offers the two things you actually do with a
// contact: call, and message. Reported as exactly this: the contacts
// "view" was never a different view, just the ordinary list with the right
// affordances for one file type.
function useVCardRow(entry: Entry, fullPath: string, inVault: boolean, ref: React.RefObject<HTMLElement | null>) {
  const isVcf = !entry.is_dir && /\.vcf$/i.test(entry.name);
  const [card, setCard] = useState<ParsedVCard | null>(null);
  useEffect(() => {
    if (!isVcf) return;
    // Gated on visibility for the same reason thumbnails are: a folder of
    // 500 contacts shouldn't read 500 files to draw the first screen.
    const el = ref.current;
    if (!el) return;
    let cancelled = false;
    const load = () => {
      const read = inVault ? api.vaultReadText(fullPath) : api.fsReadText(fullPath);
      read
        .then((text) => {
          if (!cancelled) setCard(parseVCard(text));
        })
        .catch(() => {});
    };
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        io.disconnect();
        load();
      }
    });
    io.observe(el);
    return () => {
      cancelled = true;
      io.disconnect();
    };
  }, [isVcf, fullPath, inVault, ref]);
  return { isVcf, card };
}

export function EntryTile({
  entry,
  fullPath,
  inVault,
  view,
  compact,
  selected,
  cut,
  isDropTarget,
  tagHex,
  customIcon,
  hideExtensions,
  pinned,
  sensitive,
  syncBadge,
  syncState,
  mobile,
  editing,
  editValue,
  onEditChange,
  onEditCommit,
  onEditCancel,
  onClick,
  onOpen,
  onMenu,
  onDragStart,
  onDragOver,
  onDragLeave,
  onDrop,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  view: View;
  compact?: boolean;
  selected: boolean;
  cut?: boolean;
  isDropTarget: boolean;
  tagHex?: string;
  customIcon?: string;
  hideExtensions?: boolean;
  pinned?: boolean;
  sensitive?: boolean;
  syncBadge?: "git" | "drive" | "local" | null;
  syncState?: "syncing" | "synced" | "verified" | "pending" | null;
  mobile?: boolean;
  editing: boolean;
  editValue: string;
  onEditChange: (v: string) => void;
  onEditCommit: () => void;
  onEditCancel: () => void;
  onClick: (e: React.MouseEvent) => void;
  onOpen: () => void;
  onMenu: (e: React.MouseEvent) => void;
  onDragStart: (e: React.DragEvent) => void;
  onDragOver?: (e: React.DragEvent) => void;
  onDragLeave?: () => void;
  onDrop?: (e: React.DragEvent) => void;
}) {
  const tileRef = useRef<HTMLDivElement>(null);
  const editRef = useRef<HTMLTextAreaElement>(null);
  // Grow the rename field to fit the whole wrapped filename instead of
  // scrolling inside a fixed 2-line box. `rows` only counts *logical*
  // lines (a filename is one), so height has to be driven off the actual
  // rendered scrollHeight -- recomputed on every keystroke and on entering
  // rename. Only meaningful in icon view (list rename stays single-line).
  useEffect(() => {
    if (!editing || view !== "icon") return;
    const el = editRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [editing, editValue, view]);
  // Always the same size regardless of `view`: the <img> is CSS-scaled
  // down for list/listPreview's smaller thumbnail box either way, so
  // there's no quality reason to ask for two sizes -- asking for a
  // different one per view was why switching views took a couple of
  // seconds, forcing a full re-decode+resize+re-encode (and, for images
  // inside a vault, a full re-decrypt too -- vault thumbnails are never
  // disk-cached) of every visible thumbnail purely because the view
  // changed, not because the folder did.
  const thumb = useThumbnail(entry, fullPath, inVault, 160, tileRef);
  const { isVcf, card } = useVCardRow(entry, fullPath, inVault, tileRef);
  const contactPhone = card?.phones[0];

  return (
    <div
      ref={tileRef}
      className={`entry ${view} ${compact ? "compact" : ""} ${selected ? "selected" : ""} ${
        isDropTarget ? "drop" : ""
      } ${entry.is_hidden ? "dimmed" : ""} ${cut ? "cut" : ""}`}
      data-name={entry.name}
      title={editing ? undefined : entry.name}
      // HTML5 drag is dead weight on touch (Android WebView never
      // synthesizes drag events from a touch gesture, see beginDrag) and
      // actively harmful: a `draggable` element can swallow the long-press
      // that's supposed to open the context menu instead. Off entirely on
      // mobile rather than just unused.
      draggable={!editing && !mobile}
      onClick={editing ? undefined : onClick}
      onDoubleClick={editing ? undefined : onOpen}
      onContextMenu={editing ? undefined : onMenu}
      onDragStart={onDragStart}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <span className={`entry-icon${isVcf ? " entry-avatar" : ""}`}>
        {isVcf ? (
          card?.photoDataUrl ? (
            <img src={card.photoDataUrl} className="entry-avatar-photo" alt="" draggable={false} />
          ) : (
            <span className="entry-avatar-initial">{(card?.name || entry.name).charAt(0).toUpperCase()}</span>
          )
        ) : thumb ? (
          <img src={thumb} className="entry-thumb" alt="" draggable={false} />
        ) : (
          <FileIcon entry={entry} tagHex={entry.is_dir ? tagHex : undefined} customIcon={customIcon} />
        )}
        {tagHex && !entry.is_dir && <span className="entry-tag-dot" style={{ background: tagHex }} />}
        {syncBadge && (
          <span
            className={`entry-tag-dot entry-sync-badge ${syncState ?? ""}`}
            title={
              syncState === "verified"
                ? "Verified in cloud (checksums match)"
                : syncState === "pending"
                ? "Not yet verified in cloud"
                : syncState === "syncing"
                ? "Syncing…"
                : undefined
            }
          >
            {syncState === "synced" || syncState === "verified" ? (
              <CheckGlyph size={16} />
            ) : syncState === "syncing" ? (
              <RefreshGlyph size={16} />
            ) : syncState === "pending" ? (
              <CloudSyncGlyph size={16} />
            ) : syncBadge === "git" ? (
              <GitBranchGlyph size={18} />
            ) : syncBadge === "drive" ? (
              <CloudSyncGlyph size={18} />
            ) : (
              <LocalSyncGlyph size={18} />
            )}
          </span>
        )}
        {pinned && (
          <span className="entry-pin-badge">
            <PinGlyph size={11} />
          </span>
        )}
        {sensitive && (
          <span className="entry-sensitive-badge" title="Sensitive">
            <LockGlyph size={11} />
          </span>
        )}
      </span>
      {editing ? (
        <textarea
          ref={editRef}
          autoFocus
          rows={1}
          // Width: a textarea sizes itself from `cols`, not from its content,
          // so leaving the default (20) made every icon-view rename box come
          // out at its max width even for a 3-character name. Driving cols
          // off the text (clamped, and only in icon view -- list rename is a
          // fixed-width single line) makes the field hug the name, with the
          // CSS min/max-width as the real bounds.
          cols={view === "icon" ? Math.max(4, Math.min(14, editValue.length)) : undefined}
          className="entry-name-edit"
          value={editValue}
          onClick={(e) => e.stopPropagation()}
          onChange={(e) => onEditChange(e.target.value)}
          onFocus={(e) => {
            const dot = editValue.lastIndexOf(".");
            e.currentTarget.setSelectionRange(0, dot > 0 ? dot : editValue.length);
          }}
          onBlur={onEditCommit}
          onKeyDown={(e) => {
            e.stopPropagation();
            // A plain textarea would otherwise insert a literal newline
            // here instead of committing -- filenames are one logical
            // line even though this field wraps that line visually across
            // several.
            if (e.key === "Enter") {
              e.preventDefault();
              onEditCommit();
            }
            if (e.key === "Escape") onEditCancel();
          }}
        />
      ) : (
        <span className="entry-name">
          {isVcf ? card?.name || displayEntryName(entry, true) : displayEntryName(entry, !!hideExtensions)}
          {isVcf && contactPhone && <span className="entry-subline">{contactPhone}</span>}
        </span>
      )}
      {isVcf && contactPhone && !editing && (
        // Right-hand actions, the way a phone's contact list has them --
        // stopPropagation so tapping one doesn't also open the contact.
        <span className="entry-actions" onClick={(e) => e.stopPropagation()}>
          <button
            className="entry-action-btn"
            aria-label="Call"
            title="Call"
            onClick={() => osOpen(`tel:${cleanPhoneForLink(contactPhone)}`).catch(() => {})}
          >
            <PhoneGlyph size={15} />
          </button>
          <button
            className="entry-action-btn"
            aria-label="WhatsApp"
            title="WhatsApp"
            onClick={() =>
              osOpen(`https://wa.me/${cleanPhoneForLink(contactPhone).replace(/^\+/, "")}`).catch(() => {})
            }
          >
            <ChatGlyph size={15} />
          </button>
        </span>
      )}
      {view === "list" && !compact && (
        <>
          <span className="entry-date">{formatDate(entry.mtime)}</span>
          <span className="entry-size">{entry.is_dir ? "--" : formatSize(entry.size)}</span>
          <span className="entry-kind">{kindLabel(entry)}</span>
        </>
      )}
    </div>
  );
}
